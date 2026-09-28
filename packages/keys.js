// KEYS, a capability: the account's MLS GROUP on this node — its members are the account's nodes, and its epoch secret
// is where every table key comes from. The protocol runs here, in the page's core; the identity delegate keeps this
// node's member state and each epoch's secret, and gives table keys to granted sites.
//
// THE CHANNEL is the account's table `mls`: `info` = the group info a node holding the words joins from; the COMMITS in
// one agreed order through the `ordering` capability (a `tail` log, `c/<epoch>` = the commit that moved the group FROM
// that epoch — a position is an epoch); `e/<epoch>` = that
// epoch's secret in ESCROW, sealed to the account's encryption key, so the recovery words alone reopen every epoch
// (a node never can: it holds no words — a removed node opens nothing escrowed after its removal). It is sealed with the
// words-derived key, not an MLS one: a node must read it before it has any epoch.
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
  // The group's commits in one agreed order: the account's nodes share one key, so a `tail` ordering.
  const ordering = await ctx.require("ordering");
  let commits = null;
  const commitLog = async () => (commits ??= await ordering.open({ type: "tail", table: "mls", prefix: "c/" }));
  const { core } = await ctx.require("node");
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

  let status = null; // the group as this page holds it
  let published = null; // the group info last written
  let busy = Promise.resolve();

  // Keep the state and the epoch's secret (the delegate), and publish the group info when it changed.
  async function keep(channel) {
    const st = mls.status();
    const r = await auth.identity.mlsSave(st.state, st.epoch, st.secret);
    if (!r.mlsSaved) throw new Error(`the identity would not keep the account's keys: ${r.refused ?? JSON.stringify(r)}`);
    // This epoch in escrow, once.
    if (!channel.rows().some(x => x.key === escrowKey(st.epoch))) await channel.put(escrowKey(st.epoch), hexOf(st.escrow));
    const info = hexOf(st.info);
    const row = channel.rows().find(x => x.key === "info")?.value;
    if (info !== row && info !== published) {
      await channel.put("info", info);
      published = info;
    }
    status = { epoch: st.epoch, me: st.me, members: st.members, removed: st.removed };
    return status;
  }

  // Apply every commit newer than this node's epoch, in order. First the group is loaded again with the key log as it
  // is NOW: a node joining after the words changed carries the new owner's signature.
  async function catchUp() {
    if (mls.status().removed) return 0; // removed from the account: nothing after that applies
    const pending = (await commitLog()).from(mls.status().epoch);
    if (pending.length) {
      const s = await auth.check();
      mls.load(s.didBytes, await keyLog(s.didBytes), mls.status().state);
    }
    let n = 0;
    for (const { entry } of pending) {
      mls.process(bytes(entry));
      n += 1;
      if (mls.status().removed) {
        ctx.log("account keys", { what: "this node was removed from the account: it keeps what it could read, and gets nothing newer" });
        break;
      }
    }
    if (n) ctx.log("account keys", { what: `${n} commit(s) applied: epoch ${mls.status().epoch}` });
    return n;
  }

  // MADE or JOINED with the words. A join whose epoch someone else moved first is refused by the channel's order:
  // read again and join from the newer group info.
  auth.onJoined(({ entropy, did, node }) =>
    (busy = busy.then(async () => {
      // Only the home site keeps the account's keys: anywhere else, nothing to do (and no table to ask for).
      const home = await auth.identity.mlsLoad();
      if (home.refused) return;
      const channel = await storage.table("mls");
      for (let round = 0; round < 4; round++) {
        const info = channel.rows().find(x => x.key === "info")?.value;
        const [kind, commit] = mls.with_words(did, await keyLog(did), entropy, node, info ? bytes(info) : new Uint8Array(0));
        if (kind === "joined") {
          // The join's commit at the epoch it moved from: if another node moved the group first, join again.
          const r = await (await commitLog()).append(mls.status().epoch - 1, hexOf(commit));
          if (!r.ok) continue;
        }
        const st = await keep(channel);
        ctx.log("account keys", { what: `this node ${kind} the account's group: epoch ${st.epoch}, ${st.members.length} node(s)` });
        // The epochs before this node joined: out of escrow with the words, kept here, so it reads what was written then.
        let recovered = 0;
        for (const row of channel.rows().filter(x => x.key.startsWith("e/") && Number(x.key.slice(2)) < st.epoch)) {
          const secret = mlsGlue.Mls.open_escrow(entropy, bytes(row.value));
          const r = await auth.identity.epochKeep(Number(row.key.slice(2)), secret);
          if (r.mlsSaved) recovered += 1;
        }
        if (recovered) ctx.log("account keys", { what: `${recovered} earlier epoch(s) recovered from escrow` });
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
      const channel = await storage.table("mls");
      const rows = channel.rows().filter(x => x.key.startsWith("e/"));
      for (const row of rows) await channel.put(row.key, hexOf(mlsGlue.Mls.reseal_escrow(old, fresh, bytes(row.value))));
      mls.escrow_to(fresh);
      ctx.log("account keys", { what: `${rows.length} escrow(s) sealed again for the new words` });
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
      const channel = await storage.table("mls");
      if (await catchUp()) await keep(channel);
      else status ??= (({ epoch, me, members, removed }) => ({ epoch, me, members, removed }))(mls.status());
      return status;
    }));
  }

  // REMOVE a node (lost, stolen, retired): an MLS removal, committed in the group's order. The group moves to a new
  // epoch; the removed node reads nothing written from then on. If another node moved the group first, this node's
  // view is taken back to what it kept and brought current, and the removal must be asked again.
  async function remove(index) {
    return (busy = busy.then(async () => {
      if (!status) throw new Error("the account's keys are not held on this site");
      const channel = await storage.table("mls");
      const commit = mls.remove(index);
      const r = await (await commitLog()).append(mls.status().epoch - 1, hexOf(commit));
      if (!r.ok) {
        const kept = await auth.identity.mlsLoad();
        const s = await auth.check();
        mls.load(s.didBytes, await keyLog(s.didBytes), bytes(kept.mlsState));
        await catchUp();
        await keep(channel);
        throw new Error("the account's group moved meanwhile: look again and remove it again");
      }
      const st = await keep(channel);
      ctx.log("account keys", { what: `a node removed: epoch ${st.epoch}, ${st.members.length} node(s)` });
      return st;
    }));
  }

  // How many epochs are in escrow for the words.
  async function escrowed() {
    const channel = await storage.table("mls");
    return channel.rows().filter(x => x.key.startsWith("e/")).length;
  }

  ready().catch(e => ctx.log("account keys", { what: e?.message ?? String(e) }));
  addEventListener("craftworks:auth", e => {
    if (!e.detail) status = null;
    else ready().catch(err => ctx.log("account keys", { what: err?.message ?? String(err) }));
  });

  return { ready, remove, escrowed };
}
