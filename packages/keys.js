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
    // The commit that moved the group FROM epoch e.
    async function commitFrom(e, secret) {
      const old = g.before && (await g.before()).from(e)[0];
      if (old) return old.entry;
      return (await open(e, secret)).from(e)[0]?.entry ?? null;
    }
    // COMMIT from epoch e: into e's log, with the group info after it (and, for the account, the next secret in escrow).
    async function commitAt(e, secret, commit) {
      if (g.before && (await g.before()).from(e)[0]) return { ok: false, taken: true };
      const st = g.mls().status();
      return (await open(e, secret)).append(e, JSON.stringify({ commit: hexOf(commit), info: hexOf(st.info), ...(st.escrow ? { next: hexOf(st.escrow) } : {}) }));
    }
    // This epoch's LOG exists: made now by the node that made the epoch (`made`: known new, nothing to ask), else found —
    // or, for an epoch from before epoch logs, made the first time it is looked for.
    async function ensure(st, made) {
      const log = await storage.log(g.channel, glue.epoch_log_public(st.secret), { known: made ? false : null, sealWith: await g.seal(st.epoch), space: g.space });
      if (log.absent) await log.put("open", JSON.stringify({ info: hexOf(st.info) }));
    }
    // Apply every commit newer than this node's epoch, in order, keeping every epoch passed (rows sealed then must open
    // here too). `reload(st)` first, before the first (the account: its key log as it is NOW).
    async function catchUp(reload) {
      const m = g.mls();
      if (m.status().removed) return 0; // removed: nothing after that applies
      let n = 0;
      for (;;) {
        const st = m.status();
        const entry = await commitFrom(st.epoch, st.secret);
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
    return { commitFrom, commitAt, ensure, catchUp };
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
      return st;
    }));
  }

  // A SPACE's GROUP on this node (a server, a chat): made here by its first member, or loaded as the identity kept it,
  // and brought current through its epoch logs — each sealed with its epoch's own key (a space has no shared key).
  const groups = new Map();
  function group(sp) {
    if (groups.has(sp.id)) return groups.get(sp.id);
    let m = null;
    let st = null;
    let queue = Promise.resolve();
    const ch = sp.tables.channel;
    const logs = logsOf({
      mls: () => m,
      space: sp.idBytes,
      channel: ch,
      seal: async e => (await auth.identity.tableKeyAt(ch, e, sp.idBytes)).tableKey,
      label: `${sp.name ?? "space"} keys`,
    });
    async function keep(made) {
      const s = m.status();
      const r = await auth.identity.mlsSave(s.state, s.epoch, s.secret, sp.idBytes);
      if (!r.mlsSaved) throw new Error(`the identity would not keep the space's keys: ${r.refused ?? JSON.stringify(r)}`);
      await logs.ensure(s, made);
      st = { epoch: s.epoch, me: s.me, members: s.members, removed: s.removed };
      return st;
    }
    const g = {
      // MADE by this node, its first member (with its account membership's credential).
      create: () =>
        (queue = queue.then(async () => {
          if (!(await ready())) throw new Error("this node is not in its account's group here: log in once with your recovery words");
          m = mls.create_space(sp.idBytes);
          ctx.log(`${sp.name ?? "space"} keys`, { what: "this node made the space's group: epoch 0" });
          return keep(true);
        })),
      // Loaded and brought current; null where this node is not in the space's group.
      ready: () =>
        (queue = queue.catch(() => {}).then(async () => {
          if (!m) {
            const r = await auth.identity.mlsLoad(sp.idBytes);
            if (!r.mlsState) return null;
            m = mlsGlue.Mls.load_space(sp.idBytes, bytes(r.mlsState));
          }
          if (await logs.catchUp()) return keep(false);
          if (!st) {
            const s = m.status();
            await logs.ensure(s, false);
            st = { epoch: s.epoch, me: s.me, members: s.members, removed: s.removed };
          }
          return st;
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

  ready().catch(e => ctx.log("account keys", { what: e?.message ?? String(e) }));
  addEventListener("craftworks:auth", e => {
    if (!e.detail) status = null;
    else ready().catch(err => ctx.log("account keys", { what: err?.message ?? String(err) }));
  });

  return { ready, remove, escrowed, group, onChange: f => watchers.push(f) };
}
