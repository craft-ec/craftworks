// MEMBERSHIP, a capability: who belongs. For your ACCOUNT, its members are its NODES (spaces' members come here too,
// with spaces). Membership is SELF-CERTIFYING and GOSSIPED (ARCHITECTURE: membership): each node's credential is
// signed by an owner key of the account, and each node lists, in its own `members` feed, every credential it knows
// (`n/<node>`) and every removal it made (`x/<node>`, with the epoch). Who belongs is computed from the UNION of all
// the feeds found — starting from this node and the directory, following every credential — by the account's rule
// (`account::members` in the core: credentials checked against the key log; removals by nodes still in, in epoch
// order). No single writer can hide a node or take a removal back.
//
// The account's MLS group (`keys`) is where the nodes are added and removed; this capability tells everyone.
//
//   const membership = await ctx.require("membership");
//   await membership.writers() // the node keys (hex) whose feeds count
//   await membership.nodes()   // [{ index, key, me }] from the group — null where it is not kept (not the home site)
//   await membership.remove(index)
export async function start(ctx) {
  const keys = await ctx.require("keys");
  const auth = await ctx.require("auth");
  const space = await ctx.require("space");
  const storage = await ctx.require("storage");
  const { core } = await ctx.require("node");
  const idlogCode = await ctx.require("idlog-wasm");
  const MEMBERS = space.tables.members;
  const bytes = hex => new Uint8Array(hex.match(/../g).map(b => parseInt(b, 16)));

  // WHO BELONGS: gathered from the members feeds, then the rule of the space's governance.
  let cache = null;
  function writers() {
    return (cache ??= gather().catch(e => {
      cache = null;
      throw e;
    }));
  }
  async function gather() {
    const sp = await space.account();
    if (!sp) return [];
    const known = new Set([sp.self, ...(await storage.nodes())]);
    const seen = new Set();
    const creds = new Set();
    const removals = [];
    for (let round = 0; round < 16; round++) {
      const next = [...known].filter(n => !seen.has(n));
      if (!next.length) break;
      for (const n of next) seen.add(n);
      for (const { owner, rows } of await storage.feedsOf(MEMBERS, next)) {
        for (const r of rows) {
          if (r.key.startsWith("n/")) {
            creds.add(r.value);
            known.add(r.key.slice(2));
          } else if (r.key.startsWith("x/")) {
            // A removal counts only from its remover's own feed.
            const epoch = (() => { try { return Number(JSON.parse(r.value).epoch) || 0; } catch { return 0; } })();
            removals.push([bytes(owner), bytes(r.key.slice(2)), epoch]);
          }
        }
      }
    }
    return Array.from((await admittedBy(sp))([...creds].map(bytes), removals));
  }

  // WHO BELONGS by the space's GOVERNANCE: for an account, credentials signed by an owner key of its key log.
  async function admittedBy(sp) {
    if (sp.governance.kind !== "key-log") throw new Error(`no membership rule for a space governed by ${sp.governance.kind}`);
    if (!(await auth.identity.readKeyLog(sp.governance.did))) throw new Error("the account's key log is not on the network");
    return (creds, removals) => core.account_members(idlogCode, sp.governance.did, creds, removals);
  }

  // TELL: every credential this node's group holds, in this node's own members feed (the home site only: where the
  // group runs, and where the members table is written).
  async function tell(st) {
    if (!st || st.removed || !st.members?.length) return;
    const t = await storage.table(MEMBERS);
    const have = new Set(t.rows().map(r => r.key));
    let n = 0;
    for (const m of st.members) {
      if (have.has(`n/${m.key}`) || !m.cred) continue;
      await t.put(`n/${m.key}`, m.cred);
      n += 1;
    }
    if (n) {
      cache = null;
      ctx.log("members", { what: `${n} node credential(s) told in this node's members feed` });
    }
  }
  keys.onChange(tell);
  keys.ready().then(tell).catch(() => {});

  async function nodes() {
    const st = await keys.ready();
    if (!st) return null;
    if (st.removed) return "removed"; // this node was removed from the account
    return st.members.map(m => ({ index: m.index, key: m.key, me: m.index === st.me }));
  }

  // REMOVE a node (by its index in the group): out of the group (a new epoch: it reads nothing written after), its
  // current rows adopted into this node's feeds, then its removal told — from then on its feed counts for nobody.
  async function remove(index) {
    const before = await keys.ready();
    const node = before?.members.find(m => m.index === index)?.key;
    if (!node) throw new Error("no such node in the account's group");
    const st = await keys.remove(index);
    await storage.adopt(node).catch(e => ctx.log("members", { what: `adopting ${node.slice(0, 12)}…'s rows: ${e.message}` }));
    await (await storage.table(MEMBERS)).put(`x/${node}`, JSON.stringify({ epoch: st.epoch, at: Date.now() }));
    cache = null;
    ctx.log("members", { what: `${node.slice(0, 12)}… removed at epoch ${st.epoch}: told in this node's members feed` });
    return st;
  }

  return { nodes, remove, writers };
}
