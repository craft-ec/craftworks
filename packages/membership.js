// MEMBERSHIP, a capability: who belongs. For your ACCOUNT, its members are its NODES (spaces' members come here too,
// with spaces). Membership is SELF-CERTIFYING and GOSSIPED (ARCHITECTURE: membership): each node's credential is
// signed by an owner key of the account, and each node lists, in its own `members` feed, every credential it knows
// (`n/<node>`) and every removal it made (`x/<node>`, with the epoch). Who belongs is computed from the UNION of all
// the feeds found — starting from this node and the directory, following every credential — by the account's rule
// (`account::members` in the core: credentials checked against the key log; removals by nodes still in, in epoch
// order). No single writer can hide a node or take a removal back.
//
// The account's DEVICES LIST (`keys`: credentials on the key log's current key — K8.6) is where the nodes are added and
// removed; this capability tells everyone.
//
//   const membership = await ctx.require("membership");
//   await membership.writers() // the node keys (hex) whose feeds count
//   await membership.nodes()   // [{ index, key, me }] from the devices list — null where it is not kept (not the home site)
//   await membership.remove(index)
export async function start(ctx) {
  const keys = await ctx.require("keys");
  const auth = await ctx.require("auth");
  const space = await ctx.require("space");
  const storage = await ctx.require("storage");
  const { core } = await ctx.require("node");
  const idlogCode = await ctx.require("idlog-wasm");
  const MEMBERS = space.tables.members;
  const bytes = (await ctx.require("node")).bytes;

  // WHO BELONGS: gathered from the members feeds, then the rule of the space's governance.
  // (Kept for the directory's devices as they were when gathered: a device listed since is gathered — fx 10-26, a
  // second device's rows never read on the first: its answer, gathered before the second was listed, was kept for good.)
  let cache = null;
  let cachedFor = "";
  async function writers() {
    const listed = (await storage.nodes()).slice().sort().join(",");
    if (listed !== cachedFor) (cache = null), (cachedFor = listed);
    return (cache ??= gather().catch(e => {
      cache = null;
      throw e;
    }));
  }
  async function gather() {
    const sp = await space.account.strict();
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
    const have = new Map(t.rows().map(r => [r.key, r.value]));
    let n = 0;
    for (const m of st.members) {
      // (A credential issued AGAIN — the key rotated at a removal — replaces the one told: fx/fy 10-27, the old one
      // stayed and the device that stayed counted as no writer.)
      if (!m.cred || have.get(`n/${m.key}`) === m.cred) continue;
      await t.put(`n/${m.key}`, m.cred);
      n += 1;
    }
    if (n) {
      cache = null;
      ctx.log("members", { what: `${n} node credential(s) told in this node's members feed` });
    }
  }
  keys.onChange(tell);
  ctx.background("membership: keys.ready", keys.ready.strict().then(tell));

  async function nodes() {
    const st = await keys.ready.strict();
    if (!st) return null;
    if (st.removed) return "removed"; // this node was removed from the account
    return st.members.map(m => ({ index: m.index, key: m.key, me: m.index === st.me }));
  }

  // The account's devices' CREDENTIALS (bytes), as its devices list holds them — but `except` (an index in it).
  async function credentials(except = null) {
    const st = await keys.ready.strict();
    return (st?.members ?? []).filter(m => m.cred && m.index !== except).map(m => bytes(m.cred));
  }

  // REMOVE a node (by its index in the group) — with the account's recovery WORDS (owner 10-09: a removed or stolen
  // device cut off for real): the key ROTATES and the devices that stay are credentialed again (the removed one stops
  // counting everywhere, and nothing it holds credentials it again); then out of the account's tree (a new generation:
  // it reads nothing written after), its current rows adopted into this node's feeds, its removal told.
  async function remove(index, words) {
    const before = await keys.ready.strict();
    const node = before?.members.find(m => m.index === index)?.key;
    if (!node) throw new Error("no such node among the account's devices");
    if (!words) throw new Error("removing a device needs the account's recovery words");
    const me = await auth.check.strict();
    // ONLY A PLACED DEVICE TAKES ANOTHER OUT: it writes the removal in the account's tree in the same step — a device
    // off the devices list counts in the tree as history only with its removal on record (a removal is written by the device that makes it — FOUNDATION "WHO RECEIVES IS DECIDED BY THE ONE WHO SEALS").
    const tree = await (await ctx.require("members")).account();
    await tree.sync();
    if (!tree.placed()) throw new Error("this device is not in the account's tree yet — once it is, it can take another out");
    // THE DATA KEY ROTATES WITH IT (K8.7): a fresh seed, in the key log's new event (sealed there for the words).
    const seed = crypto.getRandomValues(new Uint8Array(32));
    const { seq } = await auth.identity.removeDevice(me.didBytes, words, await credentials(index), seed);
    const st = await keys.remove(index);
    await storage.adopt(node).catch(e => ctx.failed("members", e, `adopting ${node.slice(0, 12)}…'s rows: ${e.message}`));
    await (await storage.table(MEMBERS)).put(`x/${node}`, JSON.stringify({ epoch: st.epoch, at: Date.now() }));
    // Its writing in every SPACE: recorded as departed, where it stood (`moderation.retire`, owner 10-16).
    await ctx.background("membership: moderation.retire", (await ctx.require("moderation")).retire());
    cache = null;
    ctx.log("members", { what: `${node.slice(0, 12)}… removed at epoch ${st.epoch}: told in this node's members feed` });
    // …then the new data key to the devices that stay, the account's shared tables moved to it, the page loaded again.
    await keys.rotateData(seq, seed);
    return st;
  }

  return { nodes: ctx.total("membership.nodes", () => null, nodes), remove, writers, credentials };
}
