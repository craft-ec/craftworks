// MEMBERS AND KEYS ON BEEKEM (docs/proposals/members-and-keys-beekem.md, K3): each group's KEYS are a BeeKEM tree; who
// should hold them is the list (the acts). This service holds the tree (BeeKEM is its own package, loaded here only)
// and moves its OPERATIONS: this device writes its own into the space's `pub-keyops` (one feed per device, public as
// the operations are — a newcomer reads them before it holds any key), and every device's are merged — in any order,
// THROUGH OUR CHECK (`craftworks_groups`: a listed account's signer; an add or a removal naming an act that allows
// exactly it). K4: a space's content keys are its tree's generations; K7: every space kept to its list here.
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
  // (Each account's member, as last read: kept 10 minutes — a reconcile of every space every few minutes read every
  // listed member's card FRESH each time, all of it load on the node for nothing that changed.)
  const memberRead = new Map(); // did → { at, m }
  async function memberOf(did) {
    const held = memberRead.get(did);
    if (held && !held.m.why && Date.now() - held.at < 600e3) return held.m;
    const m = await memberOfNow(did);
    memberRead.set(did, { at: Date.now(), m });
    return m;
  }
  async function memberOfNow(did) {
    // (FRESH: a card just written elsewhere — a member being added — is read as it is now, not as last seen.)
    const card = await directory.card(did, { fresh: true });
    if (!card) return { why: "no card" };
    const v = card.beekem ? JSON.parse(card.beekem) : null;
    // (No member on its card YET: the account has not opened a release since BeeKEM — it is added once it does.)
    if (!v?.cred || !v?.leaf) return { why: card.beekem ? "its card's member does not read" : "no member on its card yet (not opened since BeeKEM)" };
    const c = JSON.parse(glue.credential_parts(bytes(v.cred)) ?? "null");
    if (!c) return { why: "its member credential does not read" };
    if (c.did !== did) return { why: `its credential names ${String(c.did).slice(0, 24)}` };
    const dk = await directory.dataKey(did);
    if (dk !== c.owner) return { why: `its credential is signed by ${String(c.owner).slice(0, 12)}, the data key is ${String(dk).slice(0, 12)}` };
    return { did, id: c.member, leaf: v.leaf };
  }

  const groups = new Map();
  const life = () => ctx.require("lifecycle");
  function group(sp) {
    if (groups.has(sp.id)) return groups.get(sp.id);
    let g = null;
    let queue = Promise.resolve();
    const once = f => (queue = queue.then(f, f));
    const at = `g/${sp.id}`;
    // (What was last REPORTED, so each change is said once — never the same report every round.)
    let lastUnmapped = "", lastWaiting = "", lastShort = null, lastInStep = null;
    const state = () => storage.table("beekem");
    const feed = () => storage.table(space.tableOf(sp, "pub-keyops"), sp);
    async function load() {
      if (g) return g;
      const m = await me();
      const row = (await state()).rows().find(r => r.key === at)?.value;
      g = row ? G.BeeGroup.load(bytes(row), m.seed) : G.BeeGroup.empty(sp.idBytes, m.seed);
      savedHex = row ?? null;
      return g;
    }
    // SAVED ONLY WHEN IT CHANGED: every sync and every reconcile used to write the whole tree into the account table —
    // every space, every 2 minutes (v401, 10-18: the node signing them answered nothing else in time; a conversation's
    // keys went unanswered and its messages did not read).
    let savedHex = null;
    const save = async () => {
      const h = hexOf(g.save());
      if (h === savedHex) return;
      await (await state()).put(at, h);
      savedHex = h;
    };
    // An operation in a row (its SHA-256 the key); one too large for a row (a row holds 64 KB: a worst-case rotation from
    // ~270 members) as a PUBLIC file of the space, the row naming it — read back and checked against the key.
    const ROW = 60000;
    const send = async op => {
      const key = hexOf(new Uint8Array(await crypto.subtle.digest("SHA-256", op)));
      const hex = hexOf(op);
      if (hex.length <= ROW) return (await feed()).put(key, hex);
      const files = await ctx.require("files");
      const ref = await files.put(new Blob([op]), { space: sp, public: true, app: "members" });
      return (await feed()).put(key, JSON.stringify({ file: ref }));
    };
    const opOf = async row => {
      if (!row.value.startsWith("{")) return bytes(row.value);
      const ref = JSON.parse(row.value).file;
      const got = new Uint8Array(await (await (await ctx.require("files")).get(ref)).arrayBuffer());
      return hexOf(new Uint8Array(await crypto.subtle.digest("SHA-256", got))) === row.key ? got : null;
    };

    // OUR CHECK, from the list (`roles`: its roster, owner, acts) and its accounts' cards.
    async function check() {
      const r = await (await ctx.require("roles")).of(sp);
      // (The acts as they are NOW — an act just written elsewhere, the add of this very member, read before judging.)
      await r.refresh?.();
      const roster = r.roster();
      const owner = sp.governance?.owner ?? null;
      // (Whom each act lets in or puts out, and each account's LATEST such act: what an add or a removal names.)
      const moves = [];
      for (const a of r.acts()) {
        const mv = JSON.parse(glue.entry_move(JSON.stringify(a)) ?? "null");
        if (mv) moves.push({ inn: mv[0], did: mv[1], act: await sha(a.id) });
      }
      // Cards of the listed AND of everyone an act moved (whom to take out is found by their card too).
      const found = await Promise.all([...new Set([...roster, ...(owner ? [owner] : []), ...moves.map(m => m.did)])].map(async d => [d, await memberOf(d)]));
      const unmapped = found.filter(([, m]) => m.why);
      // (Said ONCE per change — never a report every round for the same card.)
      const um = unmapped.map(([d, m]) => `${d.slice(12, 20)}: ${m.why}`).join("; ");
      if (um && um !== lastUnmapped) life().then(l => l.transition("group", sp.id, "keys-unmapped", { why: um.slice(0, 300), n: unmapped.length }));
      lastUnmapped = um;
      const members = found.map(([, m]) => m).filter(m => !m.why);
      const byMember = new Map(members.map(m => [m.id, m.did]));
      // (Whom each act lets in or puts out: the replay's one rule — `gov::entry_move`, the node's too.)
      const acts = new Map(moves.map(m => [m.act, { kind: m.inn ? "add" : "remove", target: m.did }]));
      const latest = inn => Object.fromEntries(moves.filter(m => m.inn === inn).map(m => [m.did, bytes(m.act)]));
      const misses = [];
      return {
        misses,
        // For RECONCILING (`BeeGroup.reconcile`, the node's one plan): every card's member, who is listed now, the acts.
        cards: members.map(m => [m.did, bytes(m.id), bytes(m.leaf)]),
        listedNow: [...new Set([...roster, ...(owner ? [owner] : [])])],
        ins: latest(true),
        outs: latest(false),
        whole: () => r.whole?.() ?? false,
        owner,
        accountOf: memberHex => byMember.get(memberHex) ?? null,
        listed: did => roster.includes(did) || did === owner,
        allows: (actHex, add, targetHex) => {
          const target = byMember.get(targetHex);
          const ok = actHex === NO_ACT ? add && !!target && target === owner : (a => !!a && a.kind === (add ? "add" : "remove") && a.target === target)(acts.get(actHex));
          // (A refusal carries what it saw: the act known or not, what it allows, whom the member is.)
          if (!ok) misses.push({ act: actHex.slice(0, 8), known: acts.has(actHex), acts: acts.size, allows: acts.get(actHex) ?? null, add, target: target?.slice(12, 20) ?? `unmapped ${targetHex.slice(0, 8)}`, mapped: byMember.size });
          return ok;
        },
      };
    }

    // THE GENERATIONS (K4: a generation is the space's epoch — its number grows along the history): every one reached,
    // handed to the identity as the space's epochs (`epochKeep`), so the storage seals and reads this space's tables with
    // them — writing with the newest. Each once; then the tables are told (`craftworks:keys`).
    const kept = new Set();
    async function keepGenerations() {
      let n = 0;
      for (const [num, secret] of g.generations()) {
        if (kept.has(num)) continue;
        await auth.identity.epochKeep(Number(num), secret, sp.idBytes);
        kept.add(num);
        n += 1;
      }
      if (n) dispatchEvent(new CustomEvent("craftworks:keys", { detail: { space: sp.id } }));
      return kept.size;
    }

    // SYNC: every device's operations, merged in any order (one ahead of its predecessors offered again).
    async function syncNow() {
      await load();
      const t = await feed();
      await t.settled;
      const c = await check();
      const read = await Promise.all(t.rows().filter(x => x.value).map(x => ctx.optional("members: an operation", opOf(x), () => null)));
      let pending = read.filter(Boolean);
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
      said.generations = await keepGenerations();
      if (c.misses.length) said.misses = c.misses.slice(-3);
      await save();
      if (said.refused.length) life().then(l => l.transition("group", sp.id, "keys-refused", { why: said.refused.slice(0, 3).join("; "), n: said.refused.length, misses: JSON.stringify(said.misses ?? []).slice(0, 300) }));
      const w = said.waiting ? String(said.waiting) : "";
      if (w !== lastWaiting && said.waiting) life().then(l => l.transition("group", sp.id, "keys-waiting", { why: `${said.waiting} operation(s) wait on their predecessors`, n: said.waiting }));
      lastWaiting = w;
      return said;
    }

    // THE BRIDGE (K7): every MLS epoch secret this device holds of the space, into its table `keys-history` (sealed with
    // the newest epoch — a generation once the tree has one): whoever the tree places reads them and keeps them, so what
    // was written before the move reads for a newcomer too. Each epoch once.
    // A FORKED epoch (MLS, before BeeKEM) has a key per branch: each one this device holds that the bridge does not
    // carry yet goes as `a<epoch>~<its hash>` beside the `e<epoch>` row — what was written on any branch reads for
    // everyone (10-18, live: rows on Ivvor's space sealed with "another branch's" key at epoch 143, unreadable to every
    // member on the other branch, because the bridge carried one key per epoch).
    const BELOW = 2 ** 24;
    const tagOf = async secret => [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes(String(secret))))].slice(0, 6).map(b => b.toString(16).padStart(2, "0")).join("");
    async function bridge() {
      const held = await ctx.optional("members: epochSecrets", auth.identity.epochSecrets(sp.idBytes, BELOW), () => []);
      const lost = await ctx.optional("members: lost", ctx.require("keys").then(k => k.group(sp).lost()), () => []);
      const t = await storage.table(space.tableOf(sp, "keys-history"), sp);
      await t.settled;
      const rows = new Map(t.rows().filter(x => x.value).map(x => [x.key, x.value]));
      let n = 0;
      const carry = async (e, secret) => {
        if (typeof e !== "number" || e >= BELOW || !secret) return;
        const s = String(secret);
        if (!rows.has(`e${e}`)) return (await t.put(`e${e}`, s), rows.set(`e${e}`, s), (n += 1));
        if (rows.get(`e${e}`) === s) return;
        const k = `a${e}~${await tagOf(s)}`;
        if (!rows.has(k)) (await t.put(k, s), rows.set(k, s), (n += 1));
      };
      for (const [e, secret] of held) await carry(e, secret);
      for (const l of lost) await carry(Number(l.epoch), l.secret);
      return n;
    }
    // …and READ: the epochs the bridge carries, kept (`epochKeep`) — the storage reads the space's history with them.
    async function crossBridge() {
      const t = await storage.table(space.tableOf(sp, "keys-history"), sp);
      await t.settled;
      let n = 0;
      const alt = [];
      for (const x of t.rows()) {
        // ANOTHER BRANCH's key of a forked epoch: kept as lost (an alternate to read with), never as the epoch's key.
        if (x.value && x.key.startsWith("a") && !kept.has(x.key)) {
          const e = Number(x.key.slice(1).split("~")[0]);
          if (Number.isInteger(e)) alt.push({ epoch: e, secret: String(x.value) });
          kept.add(x.key);
          continue;
        }
        const e = Number(x.key.slice(1));
        if (!x.value || !x.key.startsWith("e") || !Number.isInteger(e) || kept.has(`m${e}`)) continue;
        await auth.identity.epochKeep(e, bytes(x.value), sp.idBytes);
        kept.add(`m${e}`);
        n += 1;
      }
      if (alt.length) {
        await (await ctx.require("keys")).group(sp).keepLost(alt);
        n += alt.length;
      }
      if (n) dispatchEvent(new CustomEvent("craftworks:keys", { detail: { space: sp.id } }));
      return n;
    }

    // RECONCILE (K7, on a page — the node does it with every page closed: K5): the tree made if the space has none (this
    // account listed: it places itself, naming its act), then the one plan (`BeeGroup.reconcile`) from a WHOLE read of the
    // list, its operations sent; the bridge written when this device holds MLS epochs, and crossed.
    async function reconcileNow() {
      const said = await syncNow();
      const m = await me();
      const c = await check();
      const myDid = (await space.account())?.id;
      const listed = c.listedNow.includes(myDid);
      if (!g.placed() && !(await feed()).rows().some(x => x.value) && listed) {
        const act = myDid === c.owner ? null : (c.ins[myDid] ?? null);
        const [made, op] = G.BeeGroup.create(sp.idBytes, m.seed, act);
        g = made;
        await send(op);
        await save();
        said.made = true;
        life().then(l => l.transition("group", sp.id, "keys-tree-made", { why: myDid === c.owner ? "the owner placed itself" : "a listed member placed itself (naming its act)" }));
      }
      if (!c.whole()) {
        if (lastShort !== true) life().then(l => l.transition("group", sp.id, "keys-short", { why: "the list not read whole: the keys not changed (I6)" }));
        lastShort = true;
        return { ...said, short: "the list not read whole: nothing changed" };
      }
      lastShort = false;
      if (g.placed()) {
        const r = g.reconcile(c.cards, c.listedNow, c.ins, c.outs);
        for (const op of r.ops) await send(op);
        if (r.ops.length) await save();
        said.added = r.added;
        said.removed = r.removed;
        if (r.not.length) said.not = r.not;
        said.generations = await keepGenerations();
        said.bridged = await ctx.optional("members: bridge", bridge(), () => 0);
      }
      said.crossed = await ctx.optional("members: crossBridge", crossBridge(), () => 0);
      const l = await life();
      const short = d => d.replace(/^did:craftec:/, "").slice(0, 6);
      if (said.added?.length || said.removed?.length || said.not?.length)
        l.transition("group", sp.id, "keys-reconciled", { why: `${said.added?.length ?? 0} added (${(said.added ?? []).map(short).join(", ")}), ${said.removed?.length ?? 0} removed (${(said.removed ?? []).map(short).join(", ")}) — one key change${said.not?.length ? ` — not: ${said.not.join("; ").slice(0, 200)}` : ""}`, generations: said.generations, members: g.members().length });
      else if (g.placed() && lastInStep !== g.members().length) l.transition("group", sp.id, "keys-in-step", { why: `the tree places the list (${g.members().length})`, generations: said.generations });
      lastInStep = g.placed() ? g.members().length : null;
      if (said.bridged) l.transition("group", sp.id, "keys-bridged", { why: `${said.bridged} MLS epoch(s) carried into the tree's history`, n: said.bridged });
      if (said.crossed) l.transition("group", sp.id, "keys-crossed", { why: `${said.crossed} MLS epoch(s) of its history kept from the bridge`, n: said.crossed });
      return said;
    }

    const handle = {
      reconcile: () => once(reconcileNow),
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

  // EVERY SPACE KEPT TO ITS LIST on this page (K7): each of this account's spaces (every kind — a server, a chat, a
  // circle, a level group; never the account's own) reconciled every 10 minutes — its tree made if it has none, the bridge
  // carried. (With every page closed, the node does it: K5.)
  async function reconcileAll() {
    if (!(await space.account())) return 0;
    let n = 0;
    for (const sp of (await space.mine.strict()).filter(s => s.kind !== "account")) {
      const r = await ctx.optional(`members: reconcile ${sp.id.slice(0, 8)}`, group(sp).reconcile(), () => null);
      if (r?.made || r?.added?.length || r?.removed?.length) n += 1;
    }
    return n;
  }
  (await ctx.require("lifecycle")).job("members reconcile", { every: 600e3, delay: 30e3, needs: async () => !!(await space.account()) || "nobody is logged in", run: () => reconcileAll() });

  return { group, me, memberOf, reconcileAll };
}
