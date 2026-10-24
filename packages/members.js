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
  const hexOf = node.hex;
  const bytes = node.bytes;
  const sha = async text => hexOf(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text))));

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
  // (Each account's member, as last read: read again by `members::card_due`, the node's too — a reconcile of every
  // space every few minutes read every listed member's card FRESH each time, all of it load on the node for nothing
  // that changed.)
  const memberRead = new Map(); // did → { at, m }
  async function memberOf(did) {
    const held = memberRead.get(did);
    if (held && !glue.card_due(held.at, !held.m.why, Date.now())) return held.m;
    const m = await memberOfNow(did);
    memberRead.set(did, { at: Date.now(), m });
    return m;
  }
  async function memberOfNow(did) {
    // (FRESH: a card just written elsewhere — a member being added — is read as it is now, not as last seen.)
    const card = await directory.card(did, { fresh: true });
    if (!card) return { why: "no card" };
    // (No member on its card YET: the account has not opened a release since BeeKEM — it is added once it does.)
    if (!card.beekem) return { why: "no member on its card yet (not opened since BeeKEM)" };
    // THE CHECK (`account::beekem_member`, the node's too).
    const m = JSON.parse(glue.beekem_member(glue.did_bytes(did), (await directory.dataKey(did)) ?? "", card.beekem));
    return m.why ? { why: m.why } : { did, id: m.id, leaf: m.leaf };
  }

  const groups = new Map();
  const life = () => ctx.require("lifecycle");
  function group(sp) {
    if (groups.has(sp.id)) return groups.get(sp.id);
    let g = null;
    let queue = Promise.resolve();
    const once = f => (queue = queue.then(f, f));
    // (`g2/`: a tree saved before only the creator's root was taken (10-19) may hold another member's root — rebuilt from
    // the operations under the rule.)
    const at = `g2/${sp.id}`;
    // (What was last REPORTED, so each change is said once — never the same report every round.)
    const state = () => storage.table("beekem");
    const feed = () => storage.table(space.tableOf(sp, "pub-keyops"), sp);
    async function load() {
      if (g) return g;
      const m = await me();
      // (The account table read WHOLE first: a row not arrived yet read as no tree — an empty one made, then SAVED over
      // the tree another device saved, 10-24: the Ivvor DM's tree, its root on no feed, lost so on one device.)
      const t = await state();
      await t.settled;
      const row = t.rows().find(r => r.key === at)?.value;
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
    // An operation in a row, or one too large for a row as a PUBLIC file of the space the row names — read back and
    // checked against the key (`members::keyop_*`, the node's too).
    const send = async op => {
      const key = glue.keyop_key(op);
      if (glue.keyop_inline(op)) return (await feed()).put(key, hexOf(op));
      const files = await ctx.require("files");
      const ref = await files.put(new Blob([op]), { space: sp, public: true, app: "members" });
      return (await feed()).put(key, JSON.stringify({ file: ref }));
    };
    const opOf = async row => {
      const r = JSON.parse(glue.keyop_row(row.value) ?? "null");
      if (!r) return null;
      if (r.op !== undefined) return bytes(r.op);
      const got = new Uint8Array(await (await (await ctx.require("files")).get(r.file)).arrayBuffer());
      return glue.keyop_file_ok(row.key, got) ? got : null;
    };

    // OUR CHECK, from the list (`roles`: its roster, owner, acts) and its accounts' cards.
    async function check() {
      const r = await (await ctx.require("roles")).of.strict(sp);
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
      // WHOSE CARDS (`gov::check_dids`, the node's too): the listed, the owner, everyone an act moved.
      const found = await Promise.all(JSON.parse(glue.check_dids(JSON.stringify(roster), owner ?? "", JSON.stringify(r.acts()))).map(async d => [d, await memberOf(d)]));
      const unmapped = found.filter(([, m]) => m.why);
      for (const [d] of unmapped) watchCard(d);
      // (Said once per change: the lifecycle records a repeated step once.)
      const um = unmapped.map(([d, m]) => `${d.slice(12, 20)}: ${m.why}`).join("; ");
      if (um) life().then(l => l.transition("group", sp.id, "keys-unmapped", { why: um.slice(0, 300), n: unmapped.length }));
      const members = found.map(([, m]) => m).filter(m => !m.why);
      const latest = inn => Object.fromEntries(moves.filter(m => m.inn === inn).map(m => [m.did, bytes(m.act)]));
      // OUR CHECK (`groups::ListCheck`, the node's too): whose each member is, who is listed, which act allows what.
      const listedNow = [...new Set([...roster, ...(owner ? [owner] : [])])];
      const list = new G.BeeList(members.map(m => [m.id, m.did]), listedNow, owner ?? "", moves.map(m => [m.act, m.inn, m.did]));
      return {
        list,
        // THE ONE ACCOUNT that makes the tree (`Check::creator`): the owner; none, the lowest listed DID.
        creator: () => owner ?? [...listedNow].sort()[0] ?? null,
        // For RECONCILING (`BeeGroup.reconcile`, the node's one plan): every card's member, who is listed now, the acts.
        cards: members.map(m => [m.did, bytes(m.id), bytes(m.leaf)]),
        listedNow,
        ins: latest(true),
        outs: latest(false),
        whole: () => r.whole?.() ?? false,
        owner,
      };
    }

    // THE GENERATIONS (K4: a generation is the space's epoch — its number grows along the history): every one reached,
    // handed to the identity as the space's epochs (`epochKeep`), so the storage seals and reads this space's tables with
    // them — writing with the newest. Each once; then the tables are told (`craftworks:keys`).
    const kept = new Set();
    async function keepGenerations(tree = g) {
      let n = 0;
      for (const [num, secret] of tree.generations()) {
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
      // NOTHING FROM A SHORT READ (R4; `guards::list_whole`, the node's too): judged against a list read short, every add
      // of an account it missed is refused, and the tree shows them out of it (10-20, the owner's page: "an add of an
      // account not listed"); merged without an operation it could not read, everyone that one adds is missing. The tree
      // stays as last merged from a whole read.
      const short = why => (life().then(l => l.transition("group", sp.id, "keys-short", { why: `${why}: the keys not changed (R4)` })), { short: why, refused: [], waiting: 0, misses: [] });
      // A SHORT LIST still MERGES what our check takes (K4 fix, 10-23): an operation taken on a short read is taken on
      // the whole one too ("ever listed" only grows — the MLS entry rule, `rules::guards::entry_outcome`); one it does
      // not take waits for the whole list (DEFERRED, never refused). Merging nothing deadlocked a member one generation
      // behind: the acts were sealed with the generation it lacked, so its list never read whole (live, Wren in
      // Craftworks: "keys-short" every pass, the space unreadable).
      const whole = c.whole();
      const UNREAD = {};
      const read = await Promise.all(t.rows().filter(x => x.value).map(x => ctx.optional("members: an operation", opOf(x), () => UNREAD)));
      if (read.includes(UNREAD)) return short(`${read.filter(x => x === UNREAD).length} operation(s) not read`);
      // (`Group::merge_all`: the one loop, the node's too.)
      const said = g.merge_all(read.filter(Boolean), c.list);
      said.refused = said.refused.map(w => w.slice(0, 71));
      if (!whole) {
        said.deferred = said.refused.length;
        said.refused = [];
        said.short = "the list not read whole";
        life().then(l => l.transition("group", sp.id, "keys-short", { why: `the list not read whole: what the check takes merged, ${said.deferred} deferred (R4)` }));
      }
      // THE TREE IS WHAT THE CHECK MAKES OF THE OPERATIONS (10-23): from a whole list, every operation merged afresh — the
      // same tree on every device. A saved tree reaching generations the fresh merge does not (a branch on an operation
      // the check now refuses: a page that took a member's stray root before the creator rule, 10-19) is DROPPED for
      // it; what was sealed with the dropped generations is carried by the bridge as lost alternates (`a<n>~`), so every
      // member reads it. (Craftworks, live: the owner's other device sealed its catalog with such a branch's generation
      // 104120349, unreadable to everyone else.)
      if (whole) {
        // AN OPERATION NO FEED HOLDS that this device's tree holds (and our check admits) is written again, its author's
        // signature as it was (`Group::unpublished`, 10-24, live: the Ivvor DM's root on no feed — the owner's own
        // operations waited on it everywhere, Ivvor never placed, while the owner's saved tree held it).
        // (From EVERY version of this account's saved tree — each device saves its own: one device's may hold what
        // another's lost, 10-24, live: device 22b6aa9b's held the DM's root, the merged row was 7388a5ca's empty one.)
        const m = await me();
        const saved = (await state()).versions(at).filter(v => v.value).map(v => { try { return G.BeeGroup.load(bytes(v.value), m.seed); } catch { return null; } }).filter(Boolean);
        const lost = [...new Map([g, ...saved].flatMap(x => [...x.unpublished(read.filter(Boolean), c.list)]).map(op => [hexOf(op), op])).values()];
        if (lost.length) {
          for (const op of lost) await send(op);
          said.republished = lost.length;
          life().then(l => l.transition("group", sp.id, "keys-republished", { why: `${lost.length} operation(s) no feed held, written again from this device's tree` }));
        }
        const fresh = G.BeeGroup.empty(sp.idBytes, (await me()).seed);
        fresh.merge_all(read.filter(Boolean), c.list);
        const reach = new Set(fresh.generations().map(([n]) => String(n)));
        // Every generation the fresh merge reaches kept too (10-23): a concurrent key change — another member's, made
        // beside this page's newer one — reaches a generation this page's own tree does not list, which a writer sealed
        // with (Craftworks: Ivvor's change, 104120349, a table unreadable to the owner's page).
        await keepGenerations(fresh);
        const extra = g.generations().map(([n]) => String(n)).filter(n => !reach.has(n));
        if (extra.length && fresh.placed()) {
          // (Kept first: the identity holds them, the bridge carries them.)
          await keepGenerations();
          g = fresh;
          said.rebuilt = extra;
          life().then(l => l.transition("group", sp.id, "keys-reconciled", { why: `rebuilt: a branch the check refuses dropped: generation(s) ${extra.join(", ")} carried as lost` }));
        }
      }
      said.generations = await keepGenerations();
      said.misses = said.misses.slice(-3);
      await save();
      if (said.refused.length) life().then(l => l.transition("group", sp.id, "keys-refused", { why: said.refused.slice(0, 3).join("; "), n: said.refused.length, misses: JSON.stringify(said.misses ?? []).slice(0, 300) }));
      if (said.waiting) life().then(l => l.transition("group", sp.id, "keys-waiting", { why: `${said.waiting} operation(s) wait on their predecessors`, n: said.waiting }));
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
    const tagOf = async secret => hexOf(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes(String(secret)))).slice(0, 6));
    async function bridge() {
      const held = await ctx.optional("members: epochSecrets", auth.identity.epochSecrets(sp.idBytes, BELOW), () => []);
      const lost = await ctx.optional("members: lost", ctx.require("keys").then(k => k.group(sp).lost()), () => []);
      const t = await storage.table(space.tableOf(sp, "keys-history"), sp);
      await t.settled;
      const rows = new Map(t.rows().filter(x => x.value).map(x => [x.key, x.value]));
      let n = 0;
      const carryLost = async (e, secret) => {
        const s = String(secret);
        const k = `a${e}~${await tagOf(s)}`;
        if (!rows.has(k)) (await t.put(k, s), rows.set(k, s), (n += 1));
      };
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
      // GENERATIONS of a tree this space no longer has (a member's own root refused, 10-19): what it sealed with them goes
      // as alternates (`a<n>~`), so every member still reads it.
      const reached = new Set(g.generations().map(([n]) => Number(n)));
      const gens = await ctx.optional("members: generations held", auth.identity.epochSecrets(sp.idBytes, 2 ** 62), () => []);
      for (const [e, secret] of gens) if (typeof e === "number" && e >= BELOW && !reached.has(e)) await carryLost(e, secret);
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
      await ctx.optional("members: follow", follow(), () => null);
      const said = await syncNow();
      // (A SHORT read changes nothing — above all it never makes a tree: an operation not merged is not proof there is none.)
      if (said.short) return said;
      const m = await me();
      const c = await check();
      if (!c.whole()) return { ...said, short: "the list not read whole" };
      const myDid = (await space.account())?.id;
      const listed = c.listedNow.includes(myDid);
      // ONLY THE CREATOR makes the tree (one root: 10-19, a member's page made a second one). Everyone else waits for it.
      if (!g.placed() && myDid === c.creator() && !g.members().length) {
        const act = myDid === c.owner ? null : (c.ins[myDid] ?? null);
        const [made, op] = G.BeeGroup.create(sp.idBytes, m.seed, act);
        g = made;
        await send(op);
        await save();
        said.made = true;
        life().then(l => l.transition("group", sp.id, "keys-tree-made", { why: myDid === c.owner ? "the owner placed itself" : "a listed member placed itself (naming its act)" }));
      }
      // NOT PLACED in a tree that exists: nothing this page can add — said once per change, with the tree's members.
      if (!g.placed() && (await feed()).rows().some(x => x.value)) {
        const placed = [...g.members()];
        const names = await Promise.all(placed.map(async id => (await Promise.all(c.listedNow.map(async d => ((await memberOf(d)).id === id ? d : null)))).find(Boolean) ?? `unnamed ${id.slice(0, 8)}`));
        const why = `this account's member ${(typeof m.id === "string" ? m.id : hexOf(m.id)).slice(0, 8)} is not in the tree; it places ${placed.length}: ${names.map(n => n.replace(/^did:craftec:/, "").slice(0, 8)).join(", ")}`;
        life().then(l => l.transition("group", sp.id, "keys-not-placed", { why, placed: placed.length }));
      }
      if (g.placed()) {
        const r = g.reconcile(c.cards, c.listedNow, c.ins, c.outs);
        for (const op of r.ops) await send(op);
        if (r.ops.length) await save();
        said.added = r.added;
        said.removed = r.removed;
        if (r.not.length) said.not = r.not;
        // K4 — THE GENERATION TO WRITE WITH (`BeeGroup.current`: a key change made first when the tree has none current
        // here — one root never updated, members just added, concurrent heads): its operation sent, the generation kept
        // with the others, so the storage seals with it (10-22, measured: nothing ever asked — no generation existed and
        // every write stayed on MLS).
        const [, , op] = g.current();
        if (op) (await send(op), await save());
        said.generations = await keepGenerations();
        said.bridged = await ctx.optional("members: bridge", bridge(), () => 0);
      }
      said.crossed = await ctx.optional("members: crossBridge", crossBridge(), () => 0);
      const l = await life();
      const short = d => d.replace(/^did:craftec:/, "").slice(0, 6);
      if (said.added?.length || said.removed?.length || said.not?.length)
        l.transition("group", sp.id, "keys-reconciled", { why: `${said.added?.length ?? 0} added (${(said.added ?? []).map(short).join(", ")}), ${said.removed?.length ?? 0} removed (${(said.removed ?? []).map(short).join(", ")}) — one key change${said.not?.length ? ` — not: ${said.not.join("; ").slice(0, 200)}` : ""}`, generations: said.generations, members: g.members().length });
      else if (g.placed()) l.transition("group", sp.id, "keys-in-step", { why: `the tree places the list (${g.members().length})`, generations: said.generations });
      if (said.bridged) l.transition("group", sp.id, "keys-bridged", { why: `${said.bridged} MLS epoch(s) carried into the tree's history`, n: said.bridged });
      if (said.crossed) l.transition("group", sp.id, "keys-crossed", { why: `${said.crossed} MLS epoch(s) of its history kept from the bridge`, n: said.crossed });
      return said;
    }

    // FOLLOWED (K4, 10-22): reconciled SOON after what it decides on changes — its list (an act: someone added or put
    // out), its key operations (another's add of this device, a key change), and a listed account's card (a newcomer's
    // member written) — not only on the 10-minute pass (measured: a newcomer placed 10–20 minutes after joining, unable
    // to read meanwhile). One reconcile per burst (`soon`), in the group's queue.
    let due = null;
    const soon = () => (due ??= setTimeout(() => ((due = null), ctx.background("members: followed", once(reconcileNow))), 3000));
    let following = false;
    const watchedCards = new Set();
    async function follow() {
      if (following) return;
      following = true;
      (await feed()).onChange(soon);
      (await (await ctx.require("roles")).of.strict(sp)).onChange(soon);
    }
    // A listed account with no member yet: its card followed — its member written, it is read fresh and placed.
    const watchCard = did => {
      if (watchedCards.has(did)) return;
      watchedCards.add(did);
      directory.onDevices(did, () => (memberRead.delete(did), soon()));
    };

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
      // WHY THE TREE IS WHERE IT IS (diagnostics, 10-23): each key operation of the space — its writer, and what our
      // check makes of it merged afresh in causal order ("applied", "refused: why", or waiting for a predecessor no feed
      // here holds). Read-only: a fresh tree, never this page's.
      diagnose: async () => {
        await load();
        const t = await feed();
        await t.settled;
        const c = await check();
        const r = await (await ctx.require("roles")).of(sp);
        const fresh = G.BeeGroup.empty(sp.idBytes, (await me()).seed);
        const ops = await Promise.all(t.rows().filter(x => x.value).map(async x => ({ key: x.key.slice(0, 12), by: (r?.author(x) ?? "?").slice(12, 18), node: (x.id ?? "").slice(0, 8), op: await ctx.optional("members: diagnose", opOf(x), () => null) })));
        let pending = ops.filter(o => o.op);
        for (let round = 0; round < 64 && pending.length; round++) {
          const before = pending.length;
          pending = pending.filter(o => {
            const said = fresh.merge(o.op, c.list);
            if (said === "waiting") return true;
            o.result = said;
            return false;
          });
          if (pending.length === before) break;
        }
        for (const o of pending) o.result = "waiting: a predecessor no feed here holds";
        // Each operation's hash, kind and predecessors (`beekem_op_info`): a waiting one names what it waits for.
        for (const o of ops) if (o.op) [o.hash, o.kind, o.preds] = (() => { const x = G.beekem_op_info(o.op); return x ? [x[0], x[1], [...x[2]]] : [null, null, []]; })();
        const held = new Set(ops.map(o => o.hash).filter(Boolean));
        for (const o of pending) o.missing = o.preds.filter(h => !held.has(h)).map(h => h.slice(0, 12));
        return { whole: c.whole(), placed: fresh.placed(), gens: fresh.generations().map(([n]) => String(n)), ops: ops.map(({ op, preds, ...o }) => ({ ...o, hash: o.hash?.slice(0, 12), preds: (preds ?? []).map(h => h.slice(0, 12)), result: o.result ?? "not read" })) };
      },
      // Whether the tree still HOLDS an account (K8.2): its member, as its card names it, among the tree's — a person
      // put out by the list is out of the keys once this is false.
      holds: async did => {
        const m = await memberOf(did);
        // (Ids as hex text, whatever form each comes in — a string through `hexOf` compared every id equal.)
        const norm = x => (typeof x === "string" ? x.toLowerCase() : hexOf(x));
        return !m.why && !!g && [...g.members()].some(id => norm(id) === norm(m.id));
      },
      members: () => (g ? [...g.members()] : []),
      // The generation NUMBERS this tree reaches (no secret): what `keepGenerations` hands the identity.
      generationNumbers: () => (g ? g.generations().map(([n]) => String(n)) : []),
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
    // This account's member on its card first — in no space yet, a space that lists it can still add it.
    await ctx.optional("members: me", me(), () => null);
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
