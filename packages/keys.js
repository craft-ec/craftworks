// KEYS, a capability: the account's MLS GROUP on this node — its members are the account's nodes, and its epoch secret
// is where every table key comes from. The protocol runs here, in the page's core; the identity delegate keeps this
// node's member state and each epoch's secret, and gives table keys to granted sites.
//
// THE CHANNEL is the account's table `mls` (one writer sequence: one agreed order): `info` = the group info a node
// holding the words joins from; `c/<epoch>` = the commit that moved the group FROM that epoch. It is sealed with the
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
  const { core } = await ctx.require("node");
  const idlogCode = await ctx.require("idlog-wasm");
  // MLS is its own wasm package, loaded here only: no other page pays for it.
  const mlsGlue = await ctx.require("mls-glue");
  await mlsGlue.default({ module_or_path: await ctx.require("mls-wasm") });
  const mls = new mlsGlue.Mls();
  // The account's key log, as the core read it (mls verifies it again against the DID).
  const keyLog = async did => {
    if (!(await auth.readKeyLog(did))) throw new Error("the account's key log is not on the network");
    return core.idlog_state(idlogCode, did);
  };
  const hexOf = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const commitKey = e => `c/${String(e).padStart(12, "0")}`;

  let status = null; // the group as this page holds it
  let published = null; // the group info last written
  let busy = Promise.resolve();

  // Keep the state and the epoch's secret (the delegate), and publish the group info when it changed.
  async function keep(channel) {
    const st = mls.status();
    const r = await auth.identity.mlsSave(st.state, st.epoch, st.secret);
    if (!r.mlsSaved) throw new Error(`the identity would not keep the account's keys: ${r.refused ?? JSON.stringify(r)}`);
    const info = hexOf(st.info);
    const row = channel.rows().find(x => x.key === "info")?.value;
    if (info !== row && info !== published) {
      await channel.put("info", info);
      published = info;
    }
    status = { epoch: st.epoch, me: st.me, members: st.members };
    return status;
  }

  // Apply every commit newer than this node's epoch, in order.
  async function catchUp(channel) {
    let n = 0;
    for (;;) {
      const next = channel.rows().find(x => x.key === commitKey(mls.status().epoch));
      if (!next) break;
      mls.process(bytes(next.value));
      n += 1;
    }
    if (n) ctx.log("account keys", { what: `${n} commit(s) applied: epoch ${mls.status().epoch}` });
    return n;
  }

  // MADE or JOINED with the words. A join whose epoch someone else moved first is refused by the channel's order:
  // read again and join from the newer group info.
  auth.onJoined(({ entropy, did }) =>
    (busy = busy.then(async () => {
      // Only the home site keeps the account's keys: anywhere else, nothing to do (and no table to ask for).
      const home = await auth.identity.mlsLoad();
      if (home.refused) return;
      const channel = await storage.table("mls");
      for (let round = 0; round < 4; round++) {
        const info = channel.rows().find(x => x.key === "info")?.value;
        const [kind, commit] = mls.with_words(did, await keyLog(did), entropy, info ? bytes(info) : new Uint8Array(0));
        if (kind === "joined") {
          const from = mls.status().epoch - 1;
          if (channel.rows().some(x => x.key === commitKey(from))) continue; // moved meanwhile: join again
          try {
            await channel.put(commitKey(from), hexOf(commit));
          } catch (e) {
            ctx.log("account keys", { what: `the join's commit was not written (${e?.message ?? e}): reading again` });
            continue;
          }
        }
        const st = await keep(channel);
        ctx.log("account keys", { what: `this node ${kind} the account's group: epoch ${st.epoch}, ${st.members.length} node(s)` });
        return;
      }
      throw new Error("the account's group kept moving: joining it again next time");
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
      if (await catchUp(channel)) await keep(channel);
      else status ??= (({ epoch, me, members }) => ({ epoch, me, members }))(mls.status());
      return status;
    }));
  }

  ready().catch(e => ctx.log("account keys", { what: e?.message ?? String(e) }));
  addEventListener("craftworks:auth", e => {
    if (!e.detail) status = null;
    else ready().catch(err => ctx.log("account keys", { what: err?.message ?? String(err) }));
  });

  return { ready };
}
