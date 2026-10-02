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
// the first epoch — its escrow (`e/<epoch>`) and group info (`info`) — and the commits from before epoch logs
// (`c/<epoch>`). A node joining with the words starts there and walks: each log's commit, its escrowed next secret,
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
  // The group's commits in one agreed order (the `ordering` capability's `tail`): the account's commits from before
  // epoch logs in its channel; every later commit in its epoch's own log.
  const ordering = await ctx.require("ordering");
  let commits = null;
  const commitLog = async () => (commits ??= await ordering.open({ type: "tail", table: CHANNEL, prefix: "c/" }));
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
  const parse = e => {
    try {
      const j = JSON.parse(e);
      return typeof j === "object" && j ? j : { commit: e };
    } catch {
      return { commit: e };
    }
  };

  let status = null; // the group as this page holds it
  const watchers = []; // told the group's status whenever it changes (`membership` gossips its members from it)
  const told = st => {
    for (const f of watchers) Promise.resolve(f(st)).catch(e => ctx.log("account keys", { what: e?.message ?? String(e) }));
  };
  let busy = Promise.resolve();

  // Keep the state and the epoch's secret (the delegate); make this epoch's log (its first row: the group info), so
  // every node reading it finds it; a new group's first epoch is pointed to from the channel.
  async function keep(channel, { made = false } = {}) {
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
  // epoch's own key; the account's with its channel's: undefined), `before` (the account's commits from before epoch
  // logs, in its channel), `label` (for the trace).
  function logsOf(g) {
    const open = async (e, secret, known = null) =>
      ordering.open({ type: "tail", table: g.channel, prefix: "c/", owner: glue.epoch_log_public(secret), known, sealWith: await g.seal(e), space: g.space });
    // The commit that moved the group FROM epoch e (`fresh`: the log read again from the network, not as last seen).
    async function commitFrom(e, secret, fresh = false) {
      const old = g.before && (await g.before()).from(e)[0];
      if (old) return old.entry;
      const log = await open(e, secret);
      if (fresh) await log.reread?.().catch(() => {});
      return log.from(e)[0]?.entry ?? null;
    }
    // COMMIT from epoch e: into e's log, with the group info after it (and, for the account, the next secret in escrow).
    async function commitAt(e, secret, commit) {
      if (g.before && (await g.before()).from(e)[0]) return { ok: false, taken: true };
      const st = g.mls().status();
      return (await open(e, secret)).append(e, JSON.stringify({ commit: hexOf(commit), info: hexOf(st.info), ...(st.escrow ? { next: hexOf(st.escrow) } : {}) }));
    }
    // This epoch's LOG exists: made now by the node that made the epoch (`made`: known new, nothing to ask), else found —
    // or, for an epoch from before epoch logs, made the first time it is looked for. `prev`: the epoch before's secret
    // (a space's HISTORY: whoever holds this epoch opens every earlier one, walking back — the account has escrow).
    async function ensure(st, made, prev = null) {
      const log = await storage.log(g.channel, glue.epoch_log_public(st.secret), { known: made ? false : null, sealWith: await g.seal(st.epoch), space: g.space });
      if (log.absent) await log.put("open", JSON.stringify({ info: hexOf(st.info), ...(prev ? { prev } : {}) }));
      return log;
    }
    // Every EARLIER epoch this group's log hands on, from `st` back: each secret kept here, so rows sealed before this
    // node joined open too. How many were kept.
    // Walked once per device: where the identity holds the epoch before this one and the first (a walk that finished
    // left them all), nothing is read again — each step is a log read and a delegate call.
    const held = async e => !!(await auth.identity.tableKeyAt(g.channel, e, g.space).catch(() => null))?.tableKey;
    async function history(st) {
      if (st.epoch > 0 && (await held(st.epoch - 1)) && (await held(0))) return 0;
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
    async function catchUp(reload, fresh = false) {
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
      return n;
    }
    return { commitFrom, commitAt, ensure, catchUp, history };
  }

  // THE ACCOUNT's group.
  // (No space: the account's group is the account's own — every identity call and tail defaults to it.)
  const account = logsOf({ mls: () => mls, space: undefined, channel: CHANNEL, seal: async () => undefined, before: commitLog, label: "account keys" });
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
        const old = (await commitLog()).from(0);
        let e = old.length;
        let info = (old.length && parse(old.at(-1).entry).info) || channel.rows().find(x => x.key === "info")?.value;
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
      for (const row of rows) await channel.put(row.key, reseal(row.value));
      // And every epoch log's escrowed next secret, walking with the old words.
      let n = rows.length;
      let e = (await commitLog()).from(0).length;
      const first = channel.rows().find(x => x.key === escrowKey(e))?.value;
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
  async function ready() {
    return (busy = busy.then(async () => {
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
        await ensureLog(st, false);
        status ??= (({ epoch, me, members, removed }) => ({ epoch, me, members, removed }))(st);
      }
      return status;
    }));
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
    const logs = logsOf({
      mls: () => m,
      space: sp.idBytes,
      channel: ch,
      seal: async e => (await auth.identity.tableKeyAt(ch, e, sp.idBytes)).tableKey,
      label: `${sp.name ?? "space"} keys`,
    });
    const status = s => (st = { epoch: s.epoch, me: s.me, members: s.members, removed: s.removed });
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
      m = mlsGlue.Mls.load_space(sp.idBytes, bytes(r.state));
      const s = m.status();
      await auth.identity.epochKeep(s.epoch, s.secret, sp.idBytes);
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
    async function change(make) {
      if (!m) throw new Error("this account is not in the space's group");
      const from = m.status();
      const [commit, out] = make();
      const r = await logs.commitAt(from.epoch, from.secret, commit);
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
          for (const i of [...indexes].sort((a, b) => b - a)) await change(() => [m.remove(i), null]);
          ctx.log(`${sp.name ?? "space"} keys`, { what: `${indexes.length} member(s) removed: epoch ${m.status().epoch}` });
          return st;
        })),
      // REFRESH this DID's own keys in the space (a device of the account was removed: what it held follows nothing
      // after this commit).
      refresh: () =>
        (queue = queue.then(async () => {
          await change(() => [m.update(), null]);
          ctx.log(`${sp.name ?? "space"} keys`, { what: `this account's keys refreshed: epoch ${m.status().epoch}` });
          return st;
        })),
      // JOINED from a welcome (someone added this DID): answered by whichever batch of key packages holds its key
      // package; that batch kept without it (a key package works once).
      join: welcome =>
        (queue = queue.then(async () => {
          const t = await spacekeys();
          await t.reread?.();
          let last = null;
          for (const row of t.rows().filter(r => r.key.startsWith("packages/"))) {
            const mb = await memberWith(row.value);
            try {
              m = mb.join_space(sp.idBytes, bytes(welcome));
            } catch (e) {
              last = e;
              continue;
            }
            await t.put(row.key, hexOf(mb.packages()));
            ctx.log(`${sp.name ?? "space"} keys`, { what: `joined the space's group: epoch ${m.status().epoch}` });
            const kept = await save(false);
            const n = await logs.history(m.status()).catch(e => (ctx.log(`${sp.name ?? "space"} keys`, { what: `its history: ${e.message}` }), 0));
            if (n) ctx.log(`${sp.name ?? "space"} keys`, { what: `${n} earlier epoch(s) of its history kept` });
            return kept;
          }
          throw new Error(`no key package of this account answers that welcome${last ? `: ${last.message ?? last}` : ""}`);
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
          await (await spacekeys()).put(at, JSON.stringify({ epoch, state: stateHex }));
          await load();
          ctx.log(`${sp.name ?? "space"} keys`, { what: `the group upkeep moved (someone let in): epoch ${epoch}` });
          return true;
        })),
      // Loaded and brought current; null where this account is not in the space's group. `fresh`: its log read again
      // from the network first (before a write: sealed with the newest epoch, never one a removed member holds).
      ready: ({ fresh = false } = {}) =>
        (queue = queue.catch(() => {}).then(async () => {
          if (!m && !(await load())) return null;
          const s = await current(fresh);
          if (!s) return null;
          await logs.ensure(m.status(), false);
          return s;
        })),
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

  // Once the account's keys are ready here: its CARD is current — the DID's key packages (a card from before, or one
  // whose key packages were used up, gets a fresh set: nobody who wants to add this person ever finds none) and its
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
    // Its id starts with when it was made (base 36): what retires the batches before it.
    await (await spacekeys()).put(`packages/${Date.now().toString(36)}-${newId()}`, hexOf(mb.packages()));
    prunePackages().catch(e => ctx.log("account keys", { what: `pruning key packages: ${e.message}` }));
    return kps;
  }
  // SPENT BATCHES: a batch is off the card once the next one is made; a welcome made from one of its key packages
  // before that may still arrive, so it is kept a WEEK longer, then dropped. The newest is always kept. (A batch from
  // before ids carried their time counts as the oldest.)
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
        ctx.log("account keys", { what: `a spent batch of key packages dropped (${batches[i].key})` });
      }
    }
  }

  // Whether this account holds ANY batch of key packages (none: the ones its card offers answer nothing).
  const holdsPackages = async () => (await spacekeys()).rows().some(r => r.key.startsWith("packages/") && r.value);
  return { ready, remove, escrowed, group, keyPackages, holdsPackages, onChange: f => watchers.push(f) };
}
