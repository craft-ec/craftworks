// KEYS, a capability: the account's MLS GROUP on this node — its members are the account's nodes, and its epoch secret
// is where every table key comes from. The protocol runs here, in the page's core; the identity delegate keeps this
// node's member state and each epoch's secret, and gives table keys to granted sites.
//
// THE COMMITS keep ONE order (MLS epochs admit no forks), through the `ordering` capability: the commit that moves the
// group FROM epoch e is written in EPOCH e's LOG — its own tail, under a key from e's secret, so only the nodes in the
// group at e can write it (a removed node writes no later epoch's). A commit's row carries the group info after it and
// the next epoch's secret in ESCROW (sealed to the account's encryption key: the recovery words alone reopen every
// epoch). The node that moves the group into an epoch makes that epoch's log, so every reached epoch's log exists.
//
// THE CHANNEL (the space's shared tail, sealed with a words-derived key: read before any epoch) keeps the POINTER to
// the first epoch — its escrow (`e/<epoch>`) and group info (`info`). A node joining with the words starts there and walks: each log's commit, its escrowed next secret,
// the next log — to the present.
//
// - After a registration or a words login (`auth.onJoined`, while the words are in hand): the group is made (no group
//   info yet) or joined by an external commit — no other node needs to be online.
// - After a PIN login, and whenever the page opens: the kept state is loaded and every newer commit applied.
// Only the account's home site runs it (the delegate keeps the state for the home site only).
//
//   const keys = await ctx.require("keys");
//   await keys.ready()        // { epoch, members, me } once current; null if not the home site or no group yet
export async function start(ctx) {
  const auth = await ctx.require("auth");
  const storage = await ctx.require("storage");
  // The space whose group this is (the account): its channel's name.
  const space = await ctx.require("space");
  const CHANNEL = space.tables.channel;
  // The group's commits in one agreed order (the `ordering` capability's `tail`): each in its epoch's own log.
  const ordering = await ctx.require("ordering");
  const { core, glue } = await ctx.require("node");
  const idlogCode = await ctx.require("idlog-wasm");
  // MLS is its own wasm package, loaded here only: no other page pays for it.
  const mlsGlue = await ctx.require("mls-glue");
  await mlsGlue.default({ module_or_path: await ctx.require("mls-wasm") });
  const mls = new mlsGlue.Mls();
  // The account's key log, as the core read it (mls verifies it again against the DID).
  const keyLog = async did => {
    if (!(await auth.identity.readKeyLog(did))) throw new Error("the account's key log is not on the network");
    return core.idlog_state(idlogCode, did);
  };
  const hexOf = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const escrowKey = e => `e/${String(e).padStart(12, "0")}`;
  const parse = e => JSON.parse(e);

  let status = null; // the group as this page holds it
  const watchers = []; // told the group's status whenever it changes (`membership` gossips its members from it)
  const told = st => {
    for (const f of watchers) Promise.resolve(f(st)).catch(e => ctx.log("account keys", { what: e?.message ?? String(e) }));
  };
  let busy = Promise.resolve();

  // Keep the state and the epoch's secret (the delegate); make this epoch's log (its first row: the group info), so
  // every node reading it finds it; a new group's first epoch is pointed to from the channel.
  async function keep(channel, { made = false } = {}) {
    readyShared = null; // the group moved here: the next read checks it again
    const st = mls.status();
    const r = await auth.identity.mlsSave(st.state, st.epoch, st.secret);
    if (!r.mlsSaved) throw new Error(`the identity would not keep the account's keys: ${r.refused ?? JSON.stringify(r)}`);
    if (st.epoch === 0 && !channel.rows().some(x => x.key === escrowKey(0))) {
      await channel.put(escrowKey(0), hexOf(st.escrow));
      await channel.put("info", hexOf(st.info));
    }
    await ensureLog(st, made);
    status = { epoch: st.epoch, me: st.me, members: st.members, removed: st.removed };
    told(status);
    return status;
  }

  // A GROUP's LOGS — the account's, or a space's; one set of rules for both. `g`: `mls()` (the group here), `space`
  // (its id bytes; undefined: the account), `channel` (its logs' table), `seal(epoch)` (a space's logs are sealed with that
  // epoch's own key; the account's with its channel's: undefined), `label` (for the trace).
  function logsOf(g) {
    const open = async (e, secret, known = null) =>
      ordering.open({ type: "tail", table: g.channel, prefix: "c/", owner: glue.epoch_log_public(secret), known, sealWith: await g.seal(e), space: g.space });
    // The commit that moved the group FROM epoch e (`fresh`: the log read again from the network, not as last seen).
    async function commitFrom(e, secret, fresh = false) {
      const log = await open(e, secret);
      if (fresh) await log.reread?.().catch(() => {});
      return log.from(e)[0]?.entry ?? null;
    }
    // COMMIT from epoch e: into e's log, with the group info after it (and, for the account, the next secret in escrow).
    async function commitAt(e, secret, commit) {
      const st = g.mls().status();
      const entry = JSON.stringify({ commit: hexOf(commit), info: hexOf(st.info), ...(st.escrow ? { next: hexOf(st.escrow) } : {}) });
      return { ...(await (await open(e, secret)).append(e, entry)), entry };
    }
    // This epoch's LOG exists: made now by the node that made the epoch (`made`: known new, nothing to ask), else found —
    // or, where that node never made it, made the first time it is looked for. `prev`: the epoch before's secret
    // (a space's HISTORY: whoever holds this epoch opens every earlier one, walking back — the account has escrow).
    async function ensure(st, made, prev = null) {
      const log = await storage.log(g.channel, glue.epoch_log_public(st.secret), { known: made ? false : null, sealWith: await g.seal(st.epoch), space: g.space });
      await log.answer?.();
      if (log.absent) await log.put("open", JSON.stringify({ info: hexOf(st.info), ...(prev ? { prev } : {}) }));
      return log;
    }
    // Every EARLIER epoch this group's log hands on, from `st` back: each secret kept here, so rows sealed before this
    // node joined open too. How many were kept.
    // Walked once per device: where the identity holds the epoch before this one and the first (a walk that finished
    // left them all), nothing is read again — each step is a log read and a delegate call.
    const held = async e => !!(await auth.identity.tableKeyAt(g.channel, e, g.space).catch(() => null))?.tableKey;
    // `whole`: walked to the start whatever is held — a node that joined another BRANCH (a repair) holds its old
    // branch's keys for those epochs, not this one's.
    async function history(st, { whole = false } = {}) {
      if (!whole && st.epoch > 0 && (await held(st.epoch - 1)) && (await held(0))) return 0;
      let [e, secret, n] = [st.epoch, st.secret, 0];
      while (e > 0) {
        const log = await storage.log(g.channel, glue.epoch_log_public(secret), { sealWith: await g.seal(e), space: g.space });
        const prev = parse(log.rows().find(r => r.key === "open")?.value ?? "{}").prev;
        if (!prev) break;
        e -= 1;
        secret = bytes(prev);
        await auth.identity.epochKeep(e, secret, g.space);
        n += 1;
      }
      return n;
    }
    // Apply every commit newer than this node's epoch, in order, keeping every epoch passed (rows sealed then must open
    // here too). `reload(st)` first, before the first (the account: its key log as it is NOW).
    // The secrets of THIS node's branch from epoch `from` up to `st` (its own epoch logs: each one's `open` row names
    // the epoch before's secret) — kept before leaving the branch, so nothing sealed on it becomes unreadable.
    async function branch(st, from) {
      const out = [{ epoch: st.epoch, secret: hexOf(st.secret) }];
      let [e, secret] = [st.epoch, st.secret];
      while (e > from) {
        // A space's log is sealed with its epoch's key of the channel — derived from the secret here (a branch just
        // joined: the identity holds none of its keys yet).
        const sealWith = g.space ? glue.epoch_table_key(secret, g.channel) : await g.seal(e);
        const log = await storage.log(g.channel, glue.epoch_log_public(secret), { sealWith, space: g.space });
        await log.answer?.();
        const prev = parse(log.rows().find(r => r.key === "open")?.value ?? "{}").prev;
        if (!prev) break;
        e -= 1;
        secret = bytes(prev);
        out.push({ epoch: e, secret: prev });
      }
      return out.filter(x => x.epoch >= from);
    }
    // A FORK HEALED. Two nodes committing from one epoch at once each see only their own write at first (the log is
    // read where each writes); the log keeps the entries (`ordering`: each under its own key, the LOWEST is the
    // position's). Only what THIS node committed is ever reconsidered — its own changes, kept with the group BEFORE
    // them (`g.snaps`) — never a commit it received: a member who held an old epoch's key and writes into its log
    // later rolls nobody back. Lost (the log's entry is another): the branch's secrets kept (`g.keepLost`: what was
    // sealed on it stays readable, sealed over to the winner's), the group as it was before, the winner applied
    // next, and the change made again where it still applies (`g.redo`). True when it went back.
    async function heal() {
      if (!g.snaps) return false;
      for (const s of await g.snaps()) {
        const won = await commitFrom(s.epoch, bytes(s.secret), true).catch(() => null);
        if (!won || won === s.entry) continue;
        ctx.log(g.label, { what: `epoch ${s.epoch}: this node's commit lost the race for it — the winner's applied, the change made again where it still applies` });
        await g.keepLost(await branch(g.mls().status(), s.epoch + 1).catch(() => [{ epoch: g.mls().status().epoch, secret: hexOf(g.mls().status().secret) }]));
        await g.restore(s);
        return true;
      }
      return false;
    }
    async function catchUp(reload, fresh = false) {
      const healed = fresh ? await heal() : false;
      const m = g.mls();
      if (m.status().removed) return 0; // removed: nothing after that applies
      let n = 0;
      for (;;) {
        const st = m.status();
        const entry = await commitFrom(st.epoch, st.secret, fresh);
        if (!entry) break;
        if (!n && reload) await reload(st);
        m.process(bytes(parse(entry).commit));
        n += 1;
        if (m.status().removed) {
          ctx.log(g.label, { what: "this node was removed: it keeps what it could read, and gets nothing newer" });
          break;
        }
        const now = m.status();
        await auth.identity.epochKeep(now.epoch, now.secret, g.space);
      }
      if (n) ctx.log(g.label, { what: `${n} commit(s) applied: epoch ${m.status().epoch}` });
      // Gone back (a lost commit undone) counts as a change: the group as it is now is saved.
      return n || (healed ? 1 : 0);
    }
    return { commitFrom, commitAt, ensure, catchUp, history, heal, branch };
  }

  // THE ACCOUNT's group.
  // (No space: the account's group is the account's own — every identity call and tail defaults to it.)
  const account = logsOf({ mls: () => mls, space: undefined, channel: CHANNEL, seal: async () => undefined, label: "account keys" });
  const { commitFrom, commitAt } = account;
  const ensureLog = account.ensure;
  const catchUp = () =>
    account.catchUp(async st => {
      const s = await auth.check();
      mls.load(s.didBytes, await keyLog(s.didBytes), st.state);
    });

  // MADE or JOINED with the words. A join whose epoch someone else moved first is refused by the channel's order:
  // read again and join from the newer group info.
  auth.onJoined(({ entropy, did, node }) =>
    (busy = busy.then(async () => {
      // Only the home site keeps the account's keys: anywhere else, nothing to do (and no table to ask for).
      const home = await auth.identity.mlsLoad();
      if (home.refused) return;
      const channel = await storage.table(CHANNEL);
      const kept = new Set();
      // The WALK, with the words: from the channel's pointer, every epoch's commit and escrowed next secret, to the
      // present — each epoch's secret kept here on the way (this node then reads what was written before it joined).
      async function walk() {
        let e = 0;
        let info = channel.rows().find(x => x.key === "info")?.value;
        const first = channel.rows().find(x => x.key === escrowKey(e))?.value;
        if (!first) return { e, info, secret: null };
        let secret = mlsGlue.Mls.open_escrow(entropy, bytes(first));
        for (;;) {
          if (!kept.has(e) && (await auth.identity.epochKeep(e, secret)).mlsSaved) kept.add(e);
          const entry = await commitFrom(e, secret);
          if (!entry) return { e, info, secret };
          const c = parse(entry);
          info = c.info ?? info;
          secret = mlsGlue.Mls.open_escrow(entropy, bytes(c.next));
          e += 1;
        }
      }
      for (let round = 0; round < 4; round++) {
        const at = await walk();
        const [kind, commit] = mls.with_words(did, await keyLog(did), entropy, node, at.info ? bytes(at.info) : new Uint8Array(0));
        if (kind === "joined") {
          // The join's commit, in the log of the epoch it moved from: if another node moved the group first, walk on.
          if (!at.secret) throw new Error("the account's group has no escrow to join from");
          const r = await commitAt(at.e, at.secret, commit);
          if (!r.ok) continue;
        }
        const st = await keep(channel, { made: true });
        ctx.log("account keys", { what: `this node ${kind} the account's group: epoch ${st.epoch}, ${st.members.length} node(s)` });
        if (kept.size) ctx.log("account keys", { what: `${kept.size} earlier epoch(s) recovered from escrow` });
        // Every account has a CARD from the start (its inbox, this node's key packages): nobody who wants to write to it
        // ever waits on one that does not exist. After this turn of the queue (the card asks for key packages).
        setTimeout(() => ctx.require("directory").then(d => d.publish()).catch(e => ctx.log("account keys", { what: `the card: ${e.message}` })));
        return;
      }
      throw new Error("the account's group kept moving: joining it again next time");
    })));

  // NEW WORDS: every escrow sealed again for them (the old words open nothing after the change), and from now on escrows
  // are sealed for them.
  auth.identity.onWordsChanged(({ old, fresh }) =>
    (busy = busy.then(async () => {
      const home = await auth.identity.mlsLoad();
      if (home.refused) return;
      const channel = await storage.table(CHANNEL);
      const reseal = hex => hexOf(mlsGlue.Mls.reseal_escrow(old, fresh, bytes(hex)));
      const rows = channel.rows().filter(x => x.key.startsWith("e/"));
      // The walk starts from the first escrow AS IT WAS (sealed for the old words) — read before it is sealed again.
      let e = 0;
      const first = rows.find(x => x.key === escrowKey(e))?.value;
      for (const row of rows) await channel.put(row.key, reseal(row.value));
      // And every epoch log's escrowed next secret, walking with the old words.
      let n = rows.length;
      let secret = first ? mlsGlue.Mls.open_escrow(old, bytes(first)) : null;
      while (secret) {
        const entry = await commitFrom(e, secret);
        if (!entry) break;
        const c = parse(entry);
        await (await storage.log(CHANNEL, glue.epoch_log_public(secret))).put(`c/${String(e).padStart(12, "0")}`, JSON.stringify({ ...c, next: reseal(c.next) }));
        secret = mlsGlue.Mls.open_escrow(old, bytes(c.next));
        e += 1;
        n += 1;
      }
      mls.escrow_to(fresh);
      ctx.log("account keys", { what: `${n} escrow(s) sealed again for the new words` });
    })));

  // Loaded (after a PIN login) and brought current.
  // READY, SHARED as a space's group's is: one check, every read waiting on it, until the group moves (its current
  // epoch's log changes, or this page changes it: `keep`).
  let readyShared = null;
  let readyLog = null;
  function ready() {
    if (readyShared) return readyShared;
    const p = (busy = busy.then(async () => {
      const s = await auth.check();
      if (!s) return null;
      if (!status) {
        const r = await auth.identity.mlsLoad();
        if (!r.mlsState) return null; // not the home site, or this node has not joined yet
        mls.load(s.didBytes, await keyLog(s.didBytes), bytes(r.mlsState));
      }
      const channel = await storage.table(CHANNEL);
      const applied = await catchUp();
      if (mls.status().removed) return forget();
      if (applied) await keep(channel);
      else {
        const st = mls.status();
        const log = await ensureLog(st, false);
        if (log && readyLog !== log) (readyLog = log), log.onChange?.(() => readyLog === log && (readyShared = null));
        status ??= (({ epoch, me, members, removed }) => ({ epoch, me, members, removed }))(st);
      }
      return status;
    }));
    readyShared = p;
    p.catch(() => readyShared === p && (readyShared = null));
    return p;
  }

  // REMOVED: the group says this node is out of the account. It FORGETS its member at once — key, PIN, grants, the
  // group's state and every epoch's secret — so a lost or stolen node that comes online again gives nothing away,
  // and the app starts again logged out. The account is untouched: the recovery words bring the node back.
  async function forget() {
    storage.refuse("this node was removed from your account: it cannot change it");
    const r = await auth.identity.forget().catch(e => ({ refused: e?.message ?? String(e) }));
    ctx.log("account keys", { what: r.loggedOut ? "this node was removed from the account: it has forgotten everything it held" : `this node was removed, and could not forget: ${r.refused ?? JSON.stringify(r)}` });
    status = { removed: true, members: [], epoch: mls.status().epoch };
    if (r.loggedOut) {
      alert("This node was removed from your account, so it has forgotten your account's keys. To use it again, log in with your recovery words.");
      await auth.logout();
    }
    return status;
  }

  // REMOVE a node (lost, stolen, retired): an MLS removal, committed in the group's order. The group moves to a new
  // epoch; the removed node reads nothing written from then on. If another node moved the group first, this node's
  // view is taken back to what it kept and brought current, and the removal must be asked again.
  async function remove(index) {
    return (busy = busy.then(async () => {
      if (!status) throw new Error("the account's keys are not held on this site");
      const channel = await storage.table(CHANNEL);
      const from = mls.status();
      const commit = mls.remove(index);
      const r = await commitAt(from.epoch, from.secret, commit);
      if (!r.ok) {
        const kept = await auth.identity.mlsLoad();
        const s = await auth.check();
        mls.load(s.didBytes, await keyLog(s.didBytes), bytes(kept.mlsState));
        await catchUp();
        await keep(channel);
        throw new Error("the account's group moved meanwhile: look again and remove it again");
      }
      const st = await keep(channel, { made: true });
      ctx.log("account keys", { what: `a node removed: epoch ${st.epoch}, ${st.members.length} node(s)` });
      // Every space this account is in: its member's keys refreshed, so the removed node — which held them — follows
      // nothing there from now on (after this turn: each space's queue, not the account's).
      setTimeout(async () => {
        // Writes onto the new epoch first (the removed node holds the old ones), then the card: the removed node no
        // longer listed as the account's (what it writes counts for nothing).
        await storage.sealNewest().catch(() => {});
        await ctx.require("directory").then(d => d.publish()).catch(e => ctx.log("account keys", { what: `the card: ${e.message}` }));
        for (const sp of await space.mine().catch(() => [])) {
          const g = group(sp);
          if (!(await g.ready().catch(() => null))) continue;
          await g.refresh().catch(e => ctx.log(`${sp.name ?? "space"} keys`, { what: `refreshing after the removal: ${e.message}` }));
        }
      });
      return st;
    }));
  }

  // THE DID's MEMBER in spaces (ARCHITECTURE: a DID is the member, a device only signs in): its keys from the identity
  // (the same on every device of the account), its key packages' secrets in the account's table `spacekeys` — one
  // row per batch (`packages/<id>`), so two devices making key packages never write over each other's.
  const SPACEKEYS = "spacekeys";
  const newId = () => hexOf(crypto.getRandomValues(new Uint8Array(8)));
  let memberKeys = null;
  const keysOfMember = () =>
    (memberKeys ??= auth.identity.spaceMember().then(r => {
      if (!r.spaceMember) throw new Error(`no member for spaces here: ${r.refused ?? JSON.stringify(r)}`);
      return r.spaceMember;
    })).catch(e => {
      memberKeys = null;
      throw e;
    });
  const spacekeys = async () => {
    const t = await storage.table(SPACEKEYS);
    await t.settled;
    return t;
  };
  // READ, never changed here (the MLS code is the identity's: one byte of it forks every account): the references a
  // batch of key packages holds (its encoding: u32 count, then each id and data as u32 length ‖ bytes) and those a
  // WELCOME is made for (RFC 9420: version, wire format 3, cipher suite, then secrets<V> of { new_member<V>,
  // kem_output<V>, ciphertext<V> } — V a variable-length size).
  const hexb = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  function batchRefs(b) {
    const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
    let at = 0;
    const n = dv.getUint32(at, true);
    at += 4;
    const out = [];
    for (let i = 0; i < n; i++) {
      const kl = dv.getUint32(at, true);
      out.push(hexb(b.subarray(at + 4, at + 4 + kl)));
      at += 4 + kl;
      at += 4 + dv.getUint32(at, true);
    }
    return out;
  }
  function welcomeRefs(b) {
    let at = 6; // version, wire format, cipher suite
    const v = () => {
      const f = b[at] >> 6;
      const len = f === 0 ? b[at] & 0x3f : f === 1 ? ((b[at] & 0x3f) << 8) | b[at + 1] : ((b[at] & 0x3f) << 24) | (b[at + 1] << 16) | (b[at + 2] << 8) | b[at + 3];
      at += f === 0 ? 1 : f === 1 ? 2 : 4;
      const s = b.subarray(at, at + len);
      at += len;
      return s;
    };
    const secrets = v();
    const outer = at;
    at = 0;
    const bb = b;
    b = secrets;
    const out = [];
    while (at < secrets.length) {
      out.push(hexb(v()));
      v();
      v();
    }
    b = bb;
    at = outer;
    return out;
  }
  // A key package's REFERENCE (RFC 9420 RefHash "MLS 1.0 KeyPackage Reference" over the KeyPackage: the message after
  // its version and wire format; this suite's hash is SHA-256) — what a welcome names it by.
  const vlen = n => (n < 64 ? [n] : n < 16384 ? [0x40 | (n >> 8), n & 255] : [0x80 | (n >>> 24), (n >> 16) & 255, (n >> 8) & 255, n & 255]);
  async function refOf(kpHex) {
    const kp = bytes(kpHex).subarray(4);
    const label = new TextEncoder().encode("MLS 1.0 KeyPackage Reference");
    return hexb(new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array([...vlen(label.length), ...label, ...vlen(kp.length), ...kp]))));
  }
  // WHY a welcome does not open, measured: which batch OFFERED the package it names (by its offers row), and whether that
  // batch holds that package's secret — and how many of what it offered it holds at all.
  async function explain(t, welcome) {
    const aimed = new Set(welcomeRefs(bytes(welcome)));
    for (const r of t.rows().filter(x => x.key.startsWith("offers/") && x.value)) {
      let kps = [];
      try {
        kps = JSON.parse(r.value);
      } catch {}
      const refs = await Promise.all(kps.map(refOf));
      if (!refs.some(x => aimed.has(x))) continue;
      const id = r.key.slice("offers/".length);
      const held = new Set(batchRefs(bytes(t.rows().find(x => x.key === `packages/${id}`)?.value ?? "00000000")));
      return `offered by batch ${id.slice(0, 8)}, which holds ${refs.filter(x => held.has(x)).length} of the ${refs.length} it offered${refs.some(x => aimed.has(x) && held.has(x)) ? " (this one among them)" : " (NOT this one)"}`;
    }
    return "named by no offers row here: a package this account never offered";
  }
  const memberWith = async packages => {
    const k = await keysOfMember();
    return new mlsGlue.SpaceMember(bytes(k.seed), bytes(k.credential), packages ? bytes(packages) : new Uint8Array(0));
  };

  // A SPACE's GROUP, as this DID's member (a server, a chat): made by its first member, or joined from a welcome; its
  // state the ACCOUNT's (the table `spacekeys`, row `s/<space>`: the newest any device saved), brought current through
  // the space's epoch logs — each sealed with its epoch's own key. This device keeps each epoch's secret it holds (its
  // identity signs and seals with them). A commit this DID made on another device is not processed here (MLS: a
  // member's own): the state that device saved is loaded instead.
  const groups = new Map();
  function group(sp) {
    if (groups.has(sp.id)) return groups.get(sp.id);
    let m = null;
    let st = null;
    let queue = Promise.resolve();
    const ch = sp.tables.channel;
    const at = `s/${sp.id}`;
    // SNAPSHOTS: the group BEFORE each commit applied here, by epoch — `<at>@<epoch>` in the account's `spacekeys`, the
    // last few kept (a commit that lost the race for its epoch is undone from one: `logs.heal`).
    const KEEP = 6;
    const snapKey = e => `${at}@${String(e).padStart(12, "0")}`;
    // A change of this node's to make again after a heal (`{ kind: "remove", creds } | { kind: "refresh" }`).
    let redo = null;
    const keepSnap = async (from, entry, intent = null) => {
      const t = await spacekeys();
      await t.put(snapKey(from.epoch), JSON.stringify({ epoch: from.epoch, secret: hexOf(from.secret), state: hexOf(from.state), entry, ...(intent ? { intent } : {}) }));
      for (const k of t.rows().filter(r => r.key.startsWith(`${at}@`) && r.value).map(r => r.key).sort().slice(0, -KEEP)) await t.remove(k);
    };
    // The LOST branches' epoch secrets (`<at>~lost`): read with, as alternates (`storage`), never written with.
    // The LOST branches' epoch secrets (`<at>~lost`): read with, as alternates (`storage`), never written with.
    // A BRANCH's FINGERPRINT at epoch `e`: its log's public key (from that epoch's secret) — the same on every node of
    // one branch, another on another. This node's, from its own logs (null: an epoch it has not reached).
    async function fingerprint(e) {
      const s = m?.status();
      if (!s || e > s.epoch) return null;
      const list = e === s.epoch ? [{ epoch: s.epoch, secret: hexOf(s.secret) }] : await logs.branch(s, e);
      const x = list.find(l => l.epoch === e);
      return x ? glue.epoch_log_public(bytes(x.secret)) : null;
    }
    // ANNOUNCED in the space's writers list (`{ w, epoch, branch }`, once per epoch): which branch this device is on —
    // what `conversation.repair` diffs against the owner's.
    // Once: the epoch claimed before anything is awaited (many reads at once announce nothing twice), and nothing
    // written when the list holds that very entry already (a page opened again).
    let announced = -1;
    async function announce() {
      const s = m?.status();
      if (!s || s.removed || s.epoch === announced || !sp.self) return;
      announced = s.epoch;
      const branch = glue.epoch_log_public(s.secret);
      const index = await ctx.require("index");
      // (Only an entry every branch reads counts: one sealed with a later epoch is this branch's alone.)
      const had = (await index.spacePointers(sp, "writers").catch(() => [])).some(p => p?.w === sp.self && p.epoch === s.epoch && p.branch === branch && p.sealedAt === 0);
      if (!had) await index.spacePoint(sp, "writers", { w: sp.self, epoch: s.epoch, branch }).catch(e => ((announced = -1), Promise.reject(e)));
    }
    // EVERY SWITCH of the group keeps the branch it leaves: its epoch secrets that the group now on does not share
    // (the epochs where the two differ) — kept as lost, read with as alternates. Nothing is ever sealed under a key
    // no device keeps.
    const keepLeft = async wasList => {
      if (!wasList.length || !m) return;
      const from = Math.min(...wasList.map(l => l.epoch));
      const now = await logs.branch(m.status(), from).catch(() => [{ epoch: m.status().epoch, secret: hexOf(m.status().secret) }]);
      const left = wasList.filter(l => !now.some(n => n.epoch === l.epoch && n.secret === l.secret) && now.some(n => n.epoch === l.epoch));
      if (left.length) await keepLostHere(left);
    };
    const keepLostHere = async list => {
      const t = await spacekeys();
      const had = JSON.parse(t.rows().find(r => r.key === `${at}~lost`)?.value ?? "[]");
      const all = [...had, ...list.filter(l => !had.some(h => h.secret === l.secret))];
      await t.put(`${at}~lost`, JSON.stringify(all));
      ctx.log(`${sp.name ?? "space"} keys`, { what: `${list.length} epoch key(s) of a lost branch kept (what was written there stays readable)` });
    };
    const logs = logsOf({
      mls: () => m,
      space: sp.idBytes,
      channel: ch,
      seal: async e => (await auth.identity.tableKeyAt(ch, e, sp.idBytes)).tableKey,
      label: `${sp.name ?? "space"} keys`,
      keepLost: list => keepLostHere(list),
      snaps: async () =>
        (await spacekeys())
          .rows()
          .filter(r => r.key.startsWith(`${at}@`) && r.value)
          .map(r => JSON.parse(r.value))
          .sort((a, b) => a.epoch - b.epoch),
      // Back to the group before epoch `s.epoch`'s commit; every snapshot after it dropped (that branch lost). Its
      // change, made again once the group is current (`redo`).
      restore: async s => {
        if (s.intent) redo = s.intent;
        m = mlsGlue.Mls.load_space(sp.idBytes, bytes(s.state));
        const t = await spacekeys();
        for (const r of t.rows().filter(r => r.key.startsWith(`${at}@`) && r.value)) if (JSON.parse(r.value).epoch >= s.epoch) await t.remove(r.key);
        status(m.status());
      },
    });
    // The shared check (`ready`), and the epoch log it follows; any change of the group's state ends the shared one.
    let shared = null;
    let followed = null;
    const status = s => {
      const changed = !st || st.epoch !== s.epoch || st.removed !== s.removed || st.members?.length !== s.members?.length;
      if (changed) shared = null;
      return (st = { epoch: s.epoch, me: s.me, members: s.members, removed: s.removed });
    };
    // SAVE: this epoch's secret on this device, the state for the account's devices, the epoch's log.
    async function save(made, prev = null) {
      const s = m.status();
      const k = await auth.identity.epochKeep(s.epoch, s.secret, sp.idBytes);
      if (!k.mlsSaved) throw new Error(`the identity would not keep the space's key: ${k.refused ?? JSON.stringify(k)}`);
      await (await spacekeys()).put(at, JSON.stringify({ epoch: s.epoch, state: hexOf(s.state) }));
      await logs.ensure(s, made, prev);
      return status(s);
    }
    // LOAD the account's newest state of this space, where it is newer than this device's (or this device has none):
    // its secret kept here, and — on a device that never held this space — every earlier epoch its log hands on.
    async function load() {
      const v = (await spacekeys()).rows().find(r => r.key === at)?.value;
      if (!v) return false;
      const r = JSON.parse(v);
      if (m && m.status().epoch >= r.epoch) return false;
      const first = !m;
      // The branch this device is on, before it is replaced: a newer state saved by another device of the account
      // (or by upkeep) may be ANOTHER branch — what was sealed on this one must stay readable.
      const was = m ? m.status() : null;
      const wasList = was ? await logs.branch(was, Math.max(0, was.epoch - 8)).catch(() => [{ epoch: was.epoch, secret: hexOf(was.secret) }]) : [];
      m = mlsGlue.Mls.load_space(sp.idBytes, bytes(r.state));
      const s = m.status();
      await auth.identity.epochKeep(s.epoch, s.secret, sp.idBytes);
      await keepLeft(wasList);
      if (first) {
        const n = await logs.history(s).catch(() => 0);
        if (n) ctx.log(`${sp.name ?? "space"} keys`, { what: `${n} earlier epoch(s) kept on this device` });
      }
      status(s);
      return true;
    }
    // A commit that did not apply here (this DID's own, from another device): its state, when that device has saved it.
    async function current(fresh = false) {
      for (let tries = 0; ; tries++) {
        try {
          if (await logs.catchUp(undefined, fresh)) return save(false);
          return status(m.status());
        } catch (e) {
          if (await load()) continue;
          if (tries >= 3) throw e;
          await new Promise(r => setTimeout(r, 1500));
          await (await spacekeys()).reread?.();
        }
      }
    }
    // A change of the group (add, remove): its commit in the log of the epoch it moves from, then saved. Lost to
    // another change first: this device takes the newer group and says so.
    async function change(make, intent = null) {
      if (!m) throw new Error("this account is not in the space's group");
      const from = m.status();
      const [commit, out] = make();
      const r = await logs.commitAt(from.epoch, from.secret, commit);
      if (r.ok) await keepSnap(from, r.entry, intent);
      if (!r.ok) {
        m = null;
        await load();
        await current().catch(() => {});
        throw new Error("the group moved meanwhile: try again");
      }
      await save(true, hexOf(from.secret));
      return out;
    }
    const g = {
      // ADD a person (a DID) by a key package from their card: the WELCOME (hex) for them.
      add: keyPackage =>
        (queue = queue.then(() =>
          change(() => {
            const [commit, welcome] = m.add(bytes(keyPackage));
            return [commit, hexOf(welcome)];
          }),
        )),
      // REMOVE members (their indexes in the group): one commit each, a new epoch each, which they cannot read.
      remove: indexes =>
        (queue = queue.then(async () => {
          for (const i of [...indexes].sort((a, b) => b - a)) {
            const cred = m.status().members.find(x => x.index === i)?.cred ?? null;
            await change(() => [m.remove(i), null], cred ? { kind: "remove", creds: [cred] } : null);
          }
          ctx.log(`${sp.name ?? "space"} keys`, { what: `${indexes.length} member(s) removed: epoch ${m.status().epoch}` });
          return st;
        })),
      // REFRESH this DID's own keys in the space (a device of the account was removed: what it held follows nothing
      // after this commit).
      refresh: () =>
        (queue = queue.then(async () => {
          await change(() => [m.update(), null], { kind: "refresh" });
          ctx.log(`${sp.name ?? "space"} keys`, { what: `this account's keys refreshed: epoch ${m.status().epoch}` });
          return st;
        })),
      // JOINED from a welcome (someone added this DID): answered by whichever batch of key packages holds its key
      // package; that batch kept without it (a key package works once).
      // `expect` ({ epoch, branch }: a REPAIR, `conversation`): taken only if it leads onto that branch — checked before
      // anything is kept (a member who made a group of their own under this space's id pulls nobody into it).
      join: (welcome, { expect = null } = {}) =>
        (queue = queue.then(async () => {
          const t = await spacekeys();
          await t.reread?.();
          // Already in the group on ANOTHER branch (a fork, healed by being added again): its epoch secrets kept
          // first — what this node wrote there stays readable — then the welcome's branch joined.
          if (m && !m.status().removed) {
            const was = m.status();
            await keepLostHere(await logs.branch(was, Math.max(0, was.epoch - 8)).catch(() => [{ epoch: was.epoch, secret: hexOf(was.secret) }]));
          }
          // Each batch's own failure: one that does not hold its key package says NotFound; the one that HOLDS it says
          // why it did not open (what to report — never hidden behind the others' NotFound).
          const failed = [];
          for (const row of t.rows().filter(r => r.key.startsWith("packages/"))) {
            const before = m;
            let mb = null;
            try {
              mb = await memberWith(row.value);
              m = mb.join_space(sp.idBytes, bytes(welcome));
            } catch (e) {
              // Whether this batch HOLDS a package the welcome is made for (then NotFound means: not found in the GROUP's
              // tree — its entry there is not the one this package made — never "not held").
              let holds = false;
              try {
                const aimed = new Set(welcomeRefs(bytes(welcome)));
                holds = batchRefs(bytes(row.value)).some(r => aimed.has(r));
              } catch {}
              failed.push({ batch: row.key.slice("packages/".length, "packages/".length + 8), size: String(row.value ?? "").length, holds, why: String(e?.message ?? e) });
              continue;
            }
            if (expect) {
              const fp = await fingerprint(expect.epoch).catch(() => null);
              if (fp !== expect.branch) {
                m = before;
                throw new Error("that welcome does not lead onto the space owner's branch: not taken");
              }
            }
            await t.put(row.key, hexOf(mb.packages()));
            ctx.log(`${sp.name ?? "space"} keys`, { what: `joined the space's group: epoch ${m.status().epoch}` });
            const kept = await save(false);
            const n = await logs.history(m.status(), { whole: !!expect }).catch(e => (ctx.log(`${sp.name ?? "space"} keys`, { what: `its history: ${e.message}` }), 0));
            if (n) ctx.log(`${sp.name ?? "space"} keys`, { what: `${n} earlier epoch(s) of its history kept` });
            return kept;
          }
          const holding = failed.filter(f => f.holds);
          if (holding.length)
            throw new Error(`the welcome's key package IS held here (batch ${holding.map(f => f.batch).join(", ")}) but the group it describes has no entry matching it: ${holding[0].why}`);
          const real = failed.filter(f => !/KeyPackageNotFound/.test(f.why));
          throw new Error(
            real.length
              ? `the batch holding its key package refused it — ${real.map(f => `${f.batch}: ${f.why}`).join("; ")}`
              : `no key package of this account answers that welcome (${failed.length} batch(es) tried; ${await explain(t, welcome).catch(e => `unexplained: ${e.message}`)}): ${failed[0]?.why ?? "none held"}`,
          );
        })),
      // MADE by this DID, its first member.
      create: () =>
        (queue = queue.then(async () => {
          m = (await memberWith(null)).create_space(sp.idBytes);
          ctx.log(`${sp.name ?? "space"} keys`, { what: "made the space's group: epoch 0" });
          return save(true);
        })),
      // UPKEEP's view (`upkeep`): the group's epoch and state, for the mandate the identity delegate admits by while no
      // page runs; and a NEWER group it made meanwhile (it let someone in), ADOPTED here (kept for the account's
      // devices, loaded).
      snapshot: () =>
        (queue = queue.catch(() => {}).then(async () => {
          if (!m && !(await load())) return null;
          const s = m.status();
          return s.removed ? null : { epoch: s.epoch, state: hexOf(s.state) };
        })),
      adopt: (epoch, stateHex) =>
        (queue = queue.catch(() => {}).then(async () => {
          if (m && m.status().epoch >= epoch) return false;
          // The group as it was here, kept with the commit upkeep wrote from it (its epoch log's): undone like any lost
          // commit if the log's winner turns out another (`logs.heal`).
          if (m) {
            const was = m.status();
            const entry = await logs.commitFrom(was.epoch, was.secret, true).catch(() => null);
            if (entry) await keepSnap(was, entry);
          }
          await (await spacekeys()).put(at, JSON.stringify({ epoch, state: stateHex }));
          await load();
          ctx.log(`${sp.name ?? "space"} keys`, { what: `the group upkeep moved (someone let in): epoch ${epoch}` });
          return true;
        })),
      // Loaded and brought current; null where this account is not in the space's group. `fresh`: its log read again
      // from the network first (before a write: sealed with the newest epoch, never one a removed member holds).
      // READY, SHARED: a read takes the group as it stands — one check per space, every table of it waiting on the
      // same — until the group changes (its state saved again, or its current epoch's log moving: someone committed).
      // A write asks `fresh` (read again from the network: sealed with the newest epoch).
      ready: ({ fresh = false } = {}) => {
        if (!fresh && shared) return shared;
        const p = (queue = queue.catch(() => {}).then(async () => {
          if (!m && !(await load())) return null;
          const s = await current(fresh);
          if (!s) return null;
          const log = await logs.ensure(m.status(), false);
          if (log && followed !== log) (followed = log), log.onChange?.(() => followed === log && (shared = null));
          announce().catch(e => ctx.log(`${sp.name ?? "space"} keys`, { what: `announcing its branch: ${e.message}` }));
          // A change of this node's lost a race: made again, where it still applies — a removal of whoever is still a
          // member, a refresh. (An admission is made again by upkeep itself: whoever asked and is not in yet.)
          if (redo) {
            const r = redo;
            redo = null;
            queueMicrotask(() => {
              if (r.kind === "refresh") g.refresh().catch(e => ctx.log(`${sp.name ?? "space"} keys`, { what: `refresh again: ${e.message}` }));
              else if (r.kind === "remove") {
                const idx = (m?.status().members ?? []).filter(x => r.creds.includes(x.cred)).map(x => x.index);
                if (idx.length) g.remove(idx).catch(e => ctx.log(`${sp.name ?? "space"} keys`, { what: `remove again: ${e.message}` }));
              }
            });
          }
          return s;
        }));
        shared = p;
        // Finished: current (every change of the group runs in this same queue) — what reads share from now on.
        p.then(() => shared === null && (shared = p), () => shared === p && (shared = null));
        return p;
      },
      lost: async () => JSON.parse((await spacekeys()).rows().find(r => r.key === `${at}~lost`)?.value ?? "[]"),
      fingerprint: e => (queue = queue.catch(() => {}).then(() => fingerprint(e))),
    };
    groups.set(sp.id, g);
    return g;
  }

  // How many epochs are in escrow for the words.
  // Every epoch is escrowed: the first from the channel, each later one with the commit that made it.
  async function escrowed() {
    const st = await ready().catch(() => null);
    return st && !st.removed ? st.epoch + 1 : 0;
  }

  // Once the account's keys are ready here: its CARD is current — the DID's key packages (a card whose key packages
  // were used up gets a fresh set: nobody who wants to add this person ever finds none) and its
  // devices' credentials (a device joined since).
  const cardReady = () =>
    ctx
      .require("directory")
      .then(async d => {
        const me = await space.account();
        if (me) await d.publish(); // writes only what is missing or changed: key packages, the devices' credentials
      })
      .catch(e => ctx.log("account keys", { what: `the card: ${e.message}` }));
  const start = () =>
    ready()
      .then(st => st && !st.removed && setTimeout(cardReady))
      .catch(e => ctx.log("account keys", { what: e?.message ?? String(e) }));
  start();
  addEventListener("craftworks:auth", e => {
    if (!e.detail) status = null;
    else start();
  });

  // KEY PACKAGES of this DID (for its card): anyone adds it to a space with one while it is away (each works once;
  // several, so two people starting a conversation at once rarely pick the same). A fresh batch, its secrets kept in
  // the account's table (any device of the account answers the welcome).
  async function keyPackages(n = 4) {
    const mb = await memberWith(null);
    const kps = Array.from({ length: n }, () => hexOf(mb.key_package()));
    // Its id starts with when it was made (base 36): what retires the batches before it. What it OFFERS (the key
    // packages a card may list) beside it, under the same id: whether a card's packages are answerable here.
    const id = `${Date.now().toString(36)}-${newId()}`;
    const t = await spacekeys();
    await t.put(`packages/${id}`, hexOf(mb.packages()));
    await t.put(`offers/${id}`, JSON.stringify(kps));
    prunePackages().catch(e => ctx.log("account keys", { what: `pruning key packages: ${e.message}` }));
    return kps;
  }
  // SPENT BATCHES: a batch is off the card once the next one is made; a welcome made from one of its key packages
  // before that may still arrive, so it is kept a WEEK longer, then dropped. The newest is always kept.
  const WEEK = 7 * 24 * 3600 * 1000;
  async function prunePackages() {
    const t = await spacekeys();
    const made = key => {
      const m = /^packages\/([0-9a-z]+)-/.exec(key);
      return m ? parseInt(m[1], 36) : 0;
    };
    const batches = t.rows().filter(r => r.key.startsWith("packages/") && r.value).map(r => ({ key: r.key, at: made(r.key) })).sort((a, b) => a.at - b.at);
    for (let i = 0; i + 1 < batches.length; i++) {
      const retired = batches[i + 1].at;
      if (retired && Date.now() - retired > WEEK) {
        await t.remove(batches[i].key);
        await t.remove(batches[i].key.replace(/^packages\//, "offers/"));
        ctx.log("account keys", { what: `a spent batch of key packages dropped (${batches[i].key})` });
      }
    }
  }

  // Whether a welcome to one of these key packages (a card's) opens here: one of them offered by a batch this
  // account holds. None: every welcome made from that card fails.
  async function answers(kps) {
    const t = await spacekeys();
    await t.settled;
    const held = new Set(t.rows().filter(r => r.key.startsWith("packages/") && r.value).map(r => r.key.slice("packages/".length)));
    const offered = new Set(t.rows().filter(r => r.key.startsWith("offers/") && r.value && held.has(r.key.slice("offers/".length))).flatMap(r => {
      try {
        return JSON.parse(r.value);
      } catch {
        return [];
      }
    }));
    return (kps ?? []).some(k => offered.has(k));
  }
  // Whether a key package this account offered (by its TAG: sha-256, 16 bytes, hex — as `keypacks` names them) is held.
  async function holdsTag(tag) {
    const t = await spacekeys();
    await t.settled;
    const held = new Set(t.rows().filter(r => r.key.startsWith("packages/") && r.value).map(r => r.key.slice("packages/".length)));
    for (const r of t.rows().filter(r => r.key.startsWith("offers/") && r.value && held.has(r.key.slice("offers/".length)))) {
      let kps = [];
      try {
        kps = JSON.parse(r.value);
      } catch {}
      for (const kp of kps) {
        const h = [...new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(kp)))].slice(0, 16).map(x => x.toString(16).padStart(2, "0")).join("");
        if (h === tag) return r.key.slice("offers/".length, "offers/".length + 8);
      }
    }
    return false;
  }
  return { ready, remove, escrowed, group, keyPackages, answers, holdsTag, onChange: f => watchers.push(f) };
}
