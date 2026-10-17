// MEMBERS AND KEYS ON BEEKEM (docs/proposals/members-and-keys-beekem.md, K3): each group's KEYS are a BeeKEM tree; who
// should hold them is the list (the acts). This service holds the tree (BeeKEM is its own package, loaded here only)
// and moves its OPERATIONS: this device writes its own into the space's `pub-keyops` (one feed per device, public as
// the operations are — a newcomer reads them before it holds any key), and every device's are merged — in any order,
// THROUGH OUR CHECK (`craftworks_groups`: a listed account's signer; an add or a removal naming an act that allows
// exactly it). K3: the operations travel and merge; content keys still come from MLS until K4.
//
//   const g = (await ctx.require("members")).group(sp)
//   await g.sync()                     // every device's operations merged; the tree saved
//   await g.create()                   // the maker's: a new tree, itself placed
//   await g.add(did, act)              // add a listed account (its card's member), by the act that lets it in; rotate
//   await g.remove(did, act)           // take one out, by the act that puts it out; rotate
//   g.placed()  g.members()            // this account in the tree; the member ids placed
export async function start(ctx) {
  const [storage, space, directory, auth, node] = await Promise.all(["storage", "space", "directory", "auth", "node"].map(n => ctx.require(n)));
  const { glue } = node;
  const G = await ctx.require("groups-glue");
  await G.default({ module_or_path: await ctx.require("groups-wasm") });
  const hexOf = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  const bytes = h => new Uint8Array((String(h).match(/../g) ?? []).map(x => parseInt(x, 16)));
  const sha = async text => hexOf(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text))));
  const NO_ACT = "00".repeat(32);

  // THIS ACCOUNT's member (one per account: every device derives it from the account's member seed) — its card says
  // it, for whoever adds it.
  let mine = null;
  async function me() {
    if (mine) return mine;
    // (The identity's answer: `{ spaceMember: { seed, credential } }`, or why not — as `keys` reads it.)
    const r = await auth.identity.spaceMember();
    const m = r?.spaceMember;
    if (!m) throw new Error(`no member for groups here: ${r?.refused ?? JSON.stringify(r)}`);
    const [id, leaf] = G.beekem_member(bytes(m.seed));
    mine = { seed: bytes(m.seed), id, leaf: hexOf(leaf), cred: m.credential };
    // (Awaited: a card this account's member is not on cannot be added by anyone — a failure is said, not lost.)
    await directory.setMember({ cred: mine.cred, leaf: mine.leaf });
    return mine;
  }

  // AN ACCOUNT's member, from its card (its credential checked: the signing key it names, signed by the data key its
  // key log names — the one check `storage`'s writers use).
  // `{ why }` when it is not (said: a refusal names its cause).
  async function memberOf(did) {
    // (FRESH: a card just written elsewhere — a member being added — is read as it is now, not as last seen.)
    const card = await directory.card(did, { fresh: true });
    if (!card) return { why: "no card" };
    const v = card.beekem ? JSON.parse(card.beekem) : null;
    if (!v?.cred || !v?.leaf) return { why: `no member on its card (fields: ${Object.keys(card).join(",")})` };
    const c = JSON.parse(glue.credential_parts(bytes(v.cred)) ?? "null");
    if (!c) return { why: "its member credential does not read" };
    if (c.did !== did) return { why: `its credential names ${String(c.did).slice(0, 24)}` };
    const dk = await directory.dataKey(did);
    if (dk !== c.owner) return { why: `its credential is signed by ${String(c.owner).slice(0, 12)}, the data key is ${String(dk).slice(0, 12)}` };
    return { did, id: c.member, leaf: v.leaf };
  }

  const groups = new Map();
  function group(sp) {
    if (groups.has(sp.id)) return groups.get(sp.id);
    let g = null;
    let queue = Promise.resolve();
    const once = f => (queue = queue.then(f, f));
    const at = `g/${sp.id}`;
    const state = () => storage.table("beekem");
    const feed = () => storage.table(space.tableOf(sp, "pub-keyops"), sp);
    async function load() {
      if (g) return g;
      const m = await me();
      const row = (await state()).rows().find(r => r.key === at)?.value;
      g = row ? G.BeeGroup.load(bytes(row), m.seed) : G.BeeGroup.empty(sp.idBytes, m.seed);
      return g;
    }
    const save = async () => (await state()).put(at, hexOf(g.save()));
    const send = async op => (await feed()).put(hexOf(new Uint8Array(await crypto.subtle.digest("SHA-256", op))), hexOf(op));

    // OUR CHECK, from the list (`roles`: its roster, owner, acts) and its accounts' cards.
    async function check() {
      const r = await (await ctx.require("roles")).of(sp);
      const roster = r.roster();
      const owner = sp.governance?.owner ?? null;
      const found = await Promise.all([...new Set([...roster, ...(owner ? [owner] : [])])].map(async d => [d, await memberOf(d)]));
      for (const [d, m] of found) if (m.why) ctx.log("members", { what: `${sp.name ?? sp.id.slice(0, 8)}: ${d.slice(12, 20)}… not mapped — ${m.why}` });
      const members = found.map(([, m]) => m).filter(m => !m.why);
      const byMember = new Map(members.map(m => [m.id, m.did]));
      // (Which acts let in or put out: the replay's one list — `gov::moves_entries`.)
      const acts = new Map();
      for (const a of r.acts()) {
        const moves = glue.moves_entries(a.act);
        if (!moves) continue;
        const kind = moves === "in" ? "add" : "remove";
        const target = a.act === "join" || a.act === "leave" ? a.by : a.did;
        acts.set(await sha(a.id), { kind, target });
      }
      return {
        accountOf: memberHex => byMember.get(memberHex) ?? null,
        listed: did => roster.includes(did) || did === owner,
        allows: (actHex, add, targetHex) => {
          const target = byMember.get(targetHex);
          if (actHex === NO_ACT) return add && !!target && target === owner;
          const a = acts.get(actHex);
          return !!a && a.kind === (add ? "add" : "remove") && a.target === target;
        },
      };
    }

    // SYNC: every device's operations, merged in any order (one ahead of its predecessors offered again).
    async function syncNow() {
      await load();
      const t = await feed();
      await t.settled;
      const c = await check();
      let pending = t.rows().filter(x => x.value).map(x => bytes(x.value));
      const said = { applied: 0, waiting: 0, refused: [] };
      for (let round = 0; pending.length && round < 64; round++) {
        const next = [];
        for (const op of pending) {
          const r = g.merge(op, c);
          if (r === "applied") said.applied += 1;
          else if (r === "waiting") next.push(op);
          else said.refused.push(r.slice(9, 80));
        }
        if (next.length === pending.length) break;
        pending = next;
      }
      said.waiting = pending.length;
      await save();
      if (said.refused.length || said.waiting) ctx.log("members", { what: `${sp.name ?? sp.id.slice(0, 8)}: ${said.waiting} operation(s) waiting, ${said.refused.length} refused (${said.refused.slice(0, 2).join("; ")})` });
      return said;
    }

    const handle = {
      sync: () => once(syncNow),
      create: () =>
        once(async () => {
          const m = await me();
          const [made, op] = G.BeeGroup.create(sp.idBytes, m.seed);
          g = made;
          await send(op);
          await save();
        }),
      add: (did, act) =>
        once(async () => {
          await syncNow();
          const m = await memberOf(did);
          if (m.why) throw new Error(`${did.slice(0, 20)}…: ${m.why}`);
          const op = g.add(bytes(m.id), bytes(m.leaf), bytes(await sha(act)));
          if (op) await send(op);
          const [rot] = g.rotate();
          await send(rot);
          await save();
        }),
      remove: (did, act) =>
        once(async () => {
          await syncNow();
          const m = await memberOf(did);
          if (m.why) throw new Error(`${did.slice(0, 20)}…: ${m.why}`);
          const op = g.remove(bytes(m.id), bytes(await sha(act)));
          if (op) await send(op);
          const [rot] = g.rotate();
          await send(rot);
          await save();
        }),
      placed: () => !!g?.placed(),
      members: () => (g ? [...g.members()] : []),
      me,
    };
    groups.set(sp.id, handle);
    return handle;
  }

  return { group, me, memberOf };
}
