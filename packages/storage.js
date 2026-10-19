// STORAGE, a capability: the logged-in ACCOUNT's tables (notes, pins, …). It composes: the core (a tail under one key:
// read, sealed, written, flushed into its tree), `feed` (a row as a version, and the merge), `membership` (whose feeds
// count), `blocks` (tree blocks) and `access` (the grant and the keys that come with it).
//
// Every node writes its OWN FEED of a table — a tail under its node key, labelled `t/<table>` — and reads the table as
// the MERGE of the feeds of the account's nodes (ARCHITECTURE: every writer its own feed; versions are causal). Any
// SITE the person allows reads and writes the same table (the grant is per site and table; the first time, the node asks them). Every feed is
// followed, so a row written elsewhere arrives here as it lands.
//
// Nothing is asked of the network that is not known to exist (a GET of a tail nobody wrote gets no answer): each
// writer's CATALOG feed lists the tables it has a feed of, listed BEFORE the feed is made; the account's DIRECTORY (its
// shared catalog) lists the nodes that have feeds, and its channels.
//
//   const notes = await (await ctx.require("storage")).table("notes");
//   notes.rows()                [{ key, value, id }]
//   await notes.put(key, value)   await notes.remove(key)   notes.onChange(fn)
// HISTORY's retention (baseline; a plan raises it later): the roots a table had are kept this long, at most this many.
const HISTORY = { days: 7, roots: 200 };
// A SPACE's own retention (its setting, `roles` hands it here): at most the plan's (HISTORY.days); 0: no history kept.
const retention = new Map(); // space id → days
const daysFor = sp => Math.min(HISTORY.days, retention.get(sp?.id ?? sp) ?? HISTORY.days);
export async function start(ctx) {
  const { Fault } = await ctx.require("fault");
  const auth = await ctx.require("auth");
  // What a space is: its keys and its own tables' names (the account, here).
  const space = await ctx.require("space");
  // Who may read and write a table here: the person's grant for this site, and the table's key that comes with it.
  const access = await ctx.require("access");
  const { core, glue, ask, listen, WAIT, within, contract, onContract, changed: wrote, dataChanged } = await ctx.require("node");
  const tailCode = await ctx.require("tail-wasm");
  // A table's tree lives in Sealed contracts, sealed whole (a public table's, in Block contracts): the core names them
  // from this code.
  core.set_block_code(await ctx.require("block-wasm"));
  core.set_sealed_code(await ctx.require("sealed-wasm"));
  const Core = glue.CraftworksCore;
  // Tree blocks go through the ONE door for them (fetch raced against parity, put).
  const blocks = await ctx.require("blocks");
  // A row as a VERSION, and the MERGE of feeds: its own package.
  const feed = await ctx.require("feed-glue");
  await feed.default({ module_or_path: await ctx.require("feed-wasm") });
  const bytes = hex => new Uint8Array(hex.match(/../g).map(b => parseInt(b, 16)));
  const QUIET_MS = 60000; // a tail with rows flushes after this long with no write
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  // The account's own CHANNELS: one shared tail each, outside the catalog's feeds (the MLS group's, which `membership`
  // reads to say whose feeds count).
  const { catalog: CATALOG, members: MEMBERS, channel: CHANNEL } = space.tables;
  const CHANNELS = new Set([CHANNEL]);
  // Tables whose key comes with ANY grant of the site: the catalog, and the space's members.
  const ANY_GRANT = new Set([CATALOG, MEMBERS]);

  // A tail's VIEW, walked to the end: the tail names the tree's root; a view that needs tree blocks names their
  // contracts, fetched (raced, rebuilt if missing) until the rows are all there; one that names EPOCHS gets their keys
  // for this table from the identity (none, for an epoch this node has no key of: what is sealed with it stays
  // unread). Returns the last view: `{ kind: "tail", tail: { rows, … } }`, or `tail-unreadable`.
  // PROGRESS, not a count of rounds: a walk goes on while each round asks for something not asked before (a big
  // table — many epochs' keys — needs many rounds), and stops after STALLED rounds that ask for nothing new.
  const STALLED = 3;
  const progress = () => {
    const asked = new Set();
    let stalled = 0;
    return v => {
      const wanted = v.kind === "tail-keys" ? v.epochs.map(e => `e${e}`) : (v.blocks ?? []);
      const fresh = wanted.filter(x => !asked.has(x));
      wanted.forEach(x => asked.add(x));
      stalled = fresh.length ? 0 : stalled + 1;
      return stalled < STALLED;
    };
  };
  async function settle(view, app) {
    const going = progress();
    for (let round = 0; view.kind === "tail-need" || view.kind === "tail-keys"; round++) {
      if (!going(view)) throw new Fault("not-yet", `${app}: the tree did not finish loading`, { message: `${app}: the tree did not finish loading (${round} rounds, the last ${STALLED} asked nothing new)`, d: { app, rounds: round } });
      if (view.kind === "tail-keys") await epochKeys(view.id, view.epochs, app);
      else await blocks.fetch(view.id, view.blocks, app);
      view = JSON.parse(core.tail_view(bytes(view.id)));
    }
    if (view.kind === "tail-unreadable") throw new Fault("sealed", `${app}: unreadable here`, { message: `${app}: ${view.said}`, d: { app } });
    return view;
  }

  // A PAGE of one tail (phase 3, Reads): `{ lo, hi, after, reverse, limit }` (keys as text) → `{ rows: [[key bytes,
  // stored value bytes]], next }` — the blocks on its path fetched, and only those.
  async function pageOf(idHex, app, { lo = "", hi = "", after = "", reverse = true, limit = 50 } = {}) {
    const b = x => (x ? enc.encode(x) : new Uint8Array(0));
    const going = progress();
    for (let round = 0; ; round++) {
      const v = JSON.parse(core.tail_page(bytes(idHex), b(lo), b(hi), reverse, b(after), limit));
      if (v.kind === "tail-page") return { rows: v.rows.map(([k, val]) => [bytes(k), bytes(val)]), next: v.next ? dec.decode(bytes(v.next)) : null };
      if ((v.kind === "tail-keys" || v.kind === "tail-need") && !going(v)) throw new Fault("not-yet", `${app}: a page did not finish loading`, { message: `${app}: a page did not finish loading (${round} rounds, the last ${STALLED} asked nothing new)`, d: { app, rounds: round } });
      if (v.kind === "tail-keys") await epochKeys(v.id, v.epochs, app);
      else if (v.kind === "tail-need") await blocks.fetch(v.id, v.blocks, app);
      else if (v.kind === "tail-unreadable") throw new Fault("sealed", `${app}: unreadable here`, { message: `${app}: ${v.said}`, d: { app } });
      else throw new Error(`${app}: ${v.said ?? v.kind}`);
    }
  }

  // REFUSING: set by `keys` when the group says this node was REMOVED from the account. From then on it writes
  // nothing — not even sealing rows over, which would take them out of reach of the nodes that remain.
  let refusing = null;

  // Tails holding rows sealed with an epoch this node had no key of when read: `id` → { app, epochs }. When a key
  // arrives (`craftworks:keys`: the identity kept an epoch — this node caught up, or joined, or another device of the
  // account moved the group), they are given it and read again: a device added while this page is open is read.
  const unkeyed = new Map();
  // A space table's ALTERNATE keys: each epoch secret of a group branch this node left (`keys.group(sp).lost()`),
  // as this table's key in that epoch.
  async function alternates(id, app, sp) {
    const lost = await (await ctx.require("keys")).group(sp).lost();
    for (const l of lost) core.tail_epoch_key_also(id, l.epoch, bytes(glue.epoch_table_key(bytes(l.secret), app)));
    return lost.length;
  }
  // WHAT DOES NOT READ here, per space (`problemsIn`): a table with no key for an epoch, a sealed one that does not
  // open, another member's feed that fails — what this page reports into the space (`conversation.reportReads`), so any
  // member sees it on Members & roles without asking for a log. Cleared when it reads after all.
  const problems = new Map(); // space id (hex) → Map(what → when)
  const problemKinds = new Map(); // `${space}|${what}` → its kind
  const spaceKey = x => (!x ? null : typeof x === "string" ? x : [...x].map(b => b.toString(16).padStart(2, "0")).join(""));
  // What settles WHY a read fails (into the report, so it can be resolved from the report alone — never a key, never
  // content): the epoch the failure names; whether this page holds that epoch's key for the table; the group's epoch
  // here and whether removed; the other branches' epochs it keeps keys of.
  async function diagnose(spObj, table, why) {
    const e = Number((/epoch (\d+)/.exec(why) ?? [])[1]);
    const d = { ep: Number.isFinite(e) ? e : null };
    if (!spObj) return d;
    const g = (await ctx.require("keys")).group(spObj);
    const st = await g.ready();
    d.at = st?.epoch ?? null;
    d.removed = !!st?.removed;
    const lost = await ctx.optional("storage: g.lost", g.lost(), () => []);
    d.lost = lost.map(l => l.epoch).slice(-8);
    if (d.ep != null && table) {
      const k = await (await ctx.require("access")).keyAt(table, d.ep, { space: spObj.id }).catch(e => ({ why: e?.message ?? String(e) }));
      d.held = !!k?.key;
      if (!k?.key) d.heldWhy = String(k?.why ?? "").slice(0, 60);
    }
    return d;
  }
  // `kind`: "key" (this device holds no key for the table at all — its keys UNREADABLE here), "history" (an earlier
  // epoch's key not given: asked of the members), "sealed", "feed" (another member's feed failing: theirs).
  const problem = (sp, what, spObj = null, table = null, kind = "sealed") => {
    const k = spaceKey(sp);
    if (!k) return;
    const m = problems.get(k) ?? new Map();
    problemKinds.set(`${k}|${String(what).slice(0, 160)}`, kind);
    const w = String(what).slice(0, 160);
    // New here: also into the day's public health bag (`index.health`, once a kind per space per hour), with what
    // settles its cause.
    if (!m.has(w))
      ctx.background("storage: diagnose", diagnose(spObj, table, w)
        .catch(e => ({ diag: String(e?.message ?? e).slice(0, 60) }))
        .then(d => ctx.require("index").then(ix => ix.health({ k: "cannot-read", s: k.slice(0, 6), why: w, d }))));
    m.set(w, Date.now());
    problems.set(k, m);
  };
  const solved = (sp, prefix) => problems.get(spaceKey(sp))?.forEach((_, w, m) => w.startsWith(prefix) && m.delete(w));
  async function epochKeys(idHex, epochs, app) {
    const spaceId = live.get(idHex)?.spaceId ?? null;
    for (const e of epochs) {
      const k = await access.keyAt(app, e, { catalog: app === CATALOG, space: spaceId });
      core.tail_epoch_key(bytes(idHex), e, k.key ? bytes(k.key) : new Uint8Array(0), false);
      const u = unkeyed.get(idHex) ?? { app, epochs: new Set() };
      if (k.key) (u.epochs.delete(e), solved(spaceId, `${app}: no key for epoch ${e}`));
      else {
        u.epochs.add(e);
        ctx.log("table sealed", { what: `${app}: no key here for epoch ${e} (${k.why})` });
        // (An EPOCH's key not here is its HISTORY — asked of the members below — never this device's keys unreadable.)
        problem(spaceId, `${app}: no key for epoch ${e} (${k.why})`, null, null, "history");
        // A space's epoch this identity was never given (its history): ASKED of the space's members (`conversation`),
        // then this table read again when the key arrives (`craftworks:keys`).
        if (spaceId) dispatchEvent(new CustomEvent("craftworks:history-gap", { detail: { space: spaceKey(spaceId), below: e } }));
      }
      if (u.epochs.size) unkeyed.set(idHex, u);
      else unkeyed.delete(idHex);
    }
  }
  // WRITES MOVE ONTO THE NEWEST EPOCH: an open tail seals with the epoch current when it opened; when the group has
  // moved since (a node removed: it holds the older epochs), what is written from now on is sealed with the newest.
  let resealing = null;
  const sealNewest = () =>
    (resealing ??= (async () => {
      for (const [idHex, t] of live) {
        const w = t.sealing;
        if (!w) continue;
        const e = await ctx.optional("storage: access.keyAt", access.keyAt(w.app, -1, { catalog: w.catalog, space: w.space }), () => ({}));
        // A newer epoch — or the SAME epoch with another key (a fork healed: the winner's key for that epoch).
        if (!e.key || e.epoch < w.epoch || (e.epoch === w.epoch && e.key === w.key)) continue;
        core.tail_epoch_key(bytes(idHex), e.epoch, bytes(e.key), true);
        const healed = e.epoch === w.epoch;
        if (healed && w.sp) await ctx.background("storage: alternates", alternates(bytes(idHex), w.app, w.sp));
        w.epoch = e.epoch;
        w.key = e.key;
        ctx.log("table sealing", { what: `${w.app}: writes now with epoch ${e.epoch}${healed ? " (its winning key: a fork healed)" : ""}` });
      }
    })().finally(() => (resealing = null)));
  addEventListener("craftworks:keys", () => sealNewest());
  let rekeying = null;
  addEventListener("craftworks:keys", () => {
    if (!unkeyed.size || rekeying) return;
    rekeying = (async () => {
      for (const [idHex, u] of [...unkeyed]) {
        await ctx.background("storage: epochKeys", epochKeys(idHex, [...u.epochs], u.app));
        if (unkeyed.has(idHex)) continue;
        const t = live.get(idHex);
        if (t?.lazy) {
          t.poke();
          continue;
        }
        const v = t && (await ctx.optional("storage: settle", settle(JSON.parse(core.tail_view(bytes(idHex))), u.app), () => null));
        if (v?.tail) {
          ctx.log("table opened", { what: `${u.app}: the epoch key arrived` });
          t.took(v.tail);
        }
      }
    })().finally(() => (rekeying = null));
  });

  let saves = 0; // this page's saves, numbered (each its own `save` process)
  const tails = new Map(); // id hex -> Promise<tail> (opened once per page)
  // R3 (FOUNDATION.md §2): what did NOT read here — UNREADABLE (sealed with a key not held), or opened with NO KEY —
  // is not terminal: when keys arrive (`craftworks:keys`, or the account's group turns member) every such tail leaves
  // the caches, with the merged tables and the directory built on it, and the page is told to show what it is on again
  // (`craftworks:reopen`): opened anew, with the keys now held. No reload.
  const waiting = new Set(); // id hex of tails read unreadable
  const mergedNow = new Map(); // at → the merged table, once open
  let reopenedAt = 0;
  async function reopenAfterKeys() {
    // UNREADABLE ones: read again IN PLACE — a table that opens now changes, and whoever reads it draws again. A space
    // table first takes its space's alternates as they are NOW (another branch's keys a member handed over the bridge).
    for (const idHex of [...waiting]) {
      const t = live.get(idHex);
      if (t?.sealing?.sp) await ctx.optional("storage: alternates", alternates(bytes(idHex), t.sealing.app, t.sealing.sp), () => 0);
      ctx.background("storage: live.get", t?.reread());
    }
    // A merged table's writer feeds that did not open: gathered again (merged in as they arrive).
    for (const m of mergedNow.values()) m.retryFailed?.();
    // Opened with NO KEY: left behind only when its key is held NOW (its key is set when it opens) — one still without
    // a key stays as it is (never a loop of reopening what cannot open).
    const out = [];
    for (const [idHex, t] of [...live]) {
      if (!t.keyless?.()) continue;
      const k = await ctx.optional("storage: access.key", access.key(t.app, { space: t.spaceId }), () => ({}));
      if (k.key) out.push(idHex);
    }
    if (!out.length) return 0;
    for (const idHex of out) (tails.delete(idHex), live.delete(idHex), waiting.delete(idHex), unkeyed.delete(idHex));
    for (const [k, p] of [...opening]) ctx.background("storage: p.then", p.then(t => out.includes(hexOf(t.id)) && opening.get(k) === p && opening.delete(k)));
    for (const [at, m] of [...mergedNow]) if (m.built.some(id => out.includes(id))) (tables.delete(at), mergedNow.delete(at));
    if (directoryTail && out.includes(hexOf(directoryTail.id))) (directoryOpen = null), (directoryTail = null);
    ctx.log("tables reopened", { what: `${out.length} table(s) opened before their key was held: opened again with it` });
    // The page shown again (at most every 30 s): its components open them anew.
    if (Date.now() - reopenedAt > 30000) (reopenedAt = Date.now()), dispatchEvent(new CustomEvent("craftworks:reopen", { detail: { tables: out.length } }));
    return out.length;
  }
  let reopenTimer = null;
  const reopenSoon = () => (clearTimeout(reopenTimer), (reopenTimer = setTimeout(reopenAfterKeys, 500)));
  addEventListener("craftworks:keys", reopenSoon);
  ctx.background("storage: ctx.require", ctx.require("lifecycle").then(l => l.on("account", (_, to) => to === "MEMBER" && reopenSoon())));
  const live = new Map(); // id hex -> tail, for what the node pushes
  const ownKeys = new Set(); // this node's writer keys in the spaces opened (its own feeds)
  listen(said => {
    const t = (said.kind === "tail" || said.kind === "tail-need" || said.kind === "tail-keys") && live.get(said.id);
    // A LAZY tail (read by pages: phase 3) is only told; its readers page again (the blocks they hold are reused).
    if (t?.lazy) return void t.poke();
    if (t)
      settle(said, t.app).then(
        v => {
          ctx.log("table changed", { what: `${t.app}: seq ${v.tail.seq}, pushed by the node` });
          t.took(v.tail);
        },
        e => ctx.failed("table changed", e, `${t.app}: ${e.message}`),
      );
  });

  // ONE TAIL: writer `owner`'s (a key, hex) tail of `app`. `known`: whether it exists (true), is new (false: opened
  // empty, no read), or is unknown (null: read). `beforeCreate`: run once before its first write makes it (its
  // listing). `sealWith` (hex): its sealing key, given (a space's epoch log: its epoch's own), instead of the table's.
  // `space` (id bytes): the space it belongs to, not the account — signed in that space, with no site grant. Its rows AS
  // STORED are `raw()` (bytes); `rows()` are its view's.
  // BLINDED NAMES (phase 4, Lifecycle): an account's or a space's table lives at a label only its readers can tell —
  // its name blinded under its own key (`blind_name`), so hosting nodes see neither which table a tail is nor, by a
  // shared prefix, which feeds are one space's. Found by name (catalogs, members, channels, epoch logs) or read by
  // anyone (public tails): at their names.
  // WHERE IT IS comes from its writer's CATALOG (`catalog`: the one place a table's place is kept — `placeOf` reads
  // it, `notePlace` writes it, both only here): not listed or never made — not looked for, made at its blinded name by
  // its first write; anything else — read at its blinded name, the one place. "Never made" only when the node ANSWERED.
  const opening = new Map(); // `${owner}|${app}` → Promise<the tail>
  const opensLog = [];
  // A table FOUND BY ITS NAME (never blinded): catalogs, members, channels, epoch logs, public tails.
  const byNameTable = (app, opts) => !!(opts.public || opts.sealWith || opts.catalogKey || CHANNELS.has(app) || ANY_GRANT.has(app) || (opts.space?.tables && Object.values(opts.space.tables).includes(app)));
  // NOT LISTED IS NOT FOR GOOD (R2 with R6): a table opened as not listed in its writer's catalog is read once the
  // catalog lists it — the catalog is followed, so its listing arrives as a push (10-15, fx: another page of the
  // account made the first space; this page's `spaces`, opened as not listed, never heard of it until reloaded).
  // (`listed`: the list's own test — a catalog's place for the table, the directory's row for a node or a channel.)
  const whenListedIn = (list, listed, p) => {
    const off = list.onChange?.(() => {
      if (!listed()) return;
      if (typeof off === "function") off();
      ctx.background("storage: listed now", p.then(t => t.assumed && ((t.assumed = false), t.reread())));
    });
    return p;
  };
  const whenListed = (cat, app, p) => whenListedIn(cat, () => !["unlisted", "none"].includes(placeOf(cat, app)), p);
  function tail(owner, app, opts = {}) {
    const inSpace = opts.space ?? null;
    const byName = byNameTable(app, opts);
    // A table found by its NAME: only whether its catalog lists it (not listed or never made: not looked for).
    if (byName) {
      const { catalog: cat = null, ...rest } = opts;
      // A PUBLIC table of a space (`pub-…`): also named in the space's open writers list by its writer (`listPublic`).
      const pubIn = opts.public && inSpace?.id ? inSpace : null;
      const listPub = () => (pubIn ? listPublic(pubIn, owner, app) : Promise.resolve());
      const made = { ...rest, beforeCreate: cat && ownKeys.has(owner) ? () => notePlace(cat, app, "blinded").then(listPub) : rest.beforeCreate };
      if (!cat) return tailAt(owner, app, app, made);
      return cat.answer().then(async () => {
        const place = placeOf(cat, app);
        if (place === "unlisted" || place === "none") {
          // Not in a catalog this reader opens — but named in the space's OPEN writers list: read by its name (a newcomer
          // before it holds the epochs the catalog is sealed with: fx 10-17, a node's key operations never read).
          if (pubIn && !ownKeys.has(owner) && (await publicListed(pubIn, owner, app))) return tailAt(owner, app, app, made);
          return whenListed(cat, app, tailAt(owner, app, app, { ...made, known: false }));
        }
        // One of this account's own, made before the open list named public tables: named there now (once a page).
        if (pubIn && ownKeys.has(owner)) ctx.background("storage: listPublic", listPub());
        return tailAt(owner, app, app, made);
      });
    }
    const k = `${owner}|${app}`;
    if (!opening.has(k))
      opening.set(
        k,
        (async () => {
          const key = await access.key(app, { space: inSpace?.idBytes ?? inSpace });
          if (!key.key) return tailAt(owner, app, app, opts); // no key here: nothing of it reads either way
          const label = glue.blind_name(bytes(key.key), app);
          const { catalog: cat = null, ...rest } = opts;
          // Its PLACE (its writer's catalog), and whether this node notes it (its own catalog only).
          const sp = await space.account.strict();
          const ours = owner === sp?.self || owner === sp?.shared || ownKeys.has(owner);
          if (cat) await cat.answer();
          const place = cat ? placeOf(cat, app) : opts.known === false ? "unlisted" : "unknown";
          const note = p => (cat && ours ? notePlace(cat, app, p).catch(e => ctx.failed("storage", e, `${app}: noting its place: ${e?.message ?? e}`)) : Promise.resolve());
          // Its first write here makes it at its blinded name, listed so first.
          const made = { ...rest, beforeCreate: cat && ours ? () => notePlace(cat, app, "blinded") : rest.beforeCreate };
          // Not looked for: not listed.
          if (place === "unlisted") return cat ? whenListed(cat, app, tailAt(owner, app, label, { ...made, known: false })) : tailAt(owner, app, label, { ...made, known: false });
          // MARKED NEVER MADE — one of this account's own: looked for ONCE at its blinded name, and listed again where it
          // is there (owner 10-07: a made table, `spacekeys`, was marked "none" by one failed read — its tree missing a
          // block its group could rebuild — and was never read again). Another's: not looked for.
          if (place === "none") {
            if (!(ours && cat)) return cat ? whenListed(cat, app, tailAt(owner, app, label, { ...made, known: false })) : tailAt(owner, app, label, { ...made, known: false });
            const t = await tailAt(owner, app, label, { ...made, wait: WAIT.answer });
            await t.answer?.();
            if (!t.absent) {
              await note("blinded");
              ctx.log("storage", { what: `${app}: marked never made, but it is there — listed again` });
            }
            return t;
          }
          // READ WHERE ITS CATALOG SAYS — its blinded name, the ONE place a table is read. A table its catalog says was
          // MADE is never marked "never made": a failed read means not read NOW (a block, the node, the network).
          return tailAt(owner, app, label, { ...made, wait: ours && cat ? WAIT.answer : WAIT.ask });
        })().catch(e => (opening.delete(k), Promise.reject(e))),
      );
    const p = opening.get(k);
    return opts.lazy ? p : p.then(t => (t.lazy ? t.whole().then(() => t) : t));
  }

  function tailAt(owner, app, label, { known = null, catalogKey = false, beforeCreate = null, sealWith = null, space: inSpace = null, public: open = false, lazy = false, wait = WAIT.ask } = {}) {
    // The space: an object (a table of a space: its group keeps it current) or its id's bytes (an epoch log).
    const spaceId = inSpace?.idBytes ?? inSpace;
    // Its label: its name, or its blinded name (then its writes are signed by the name, which the identity checks).
    const blindedAs = label !== app ? app : "";
    const idHex = core.tail_open(tailCode, bytes(owner), label);
    // Opened already: as it is — or, opened LAZY and now wanted whole, read whole.
    if (tails.has(idHex)) {
      const had = tails.get(idHex);
      // OPENED AS NEVER MADE (`known: false`, an assumption) and now asked for as unknown: READ (R4 — an assumption
      // never outlives the next open that does not make it).
      if (known !== false) ctx.background("storage: had.then", had.then(t => t.assumed && ((t.assumed = false), t.reread())));
      return lazy ? had : had.then(t => (t.lazy ? t.whole().then(() => t) : t));
    }
    const id = bytes(idHex);
    const name = Core.id_name(id);
    let rows = [];
    const changed = [];
    let queue = Promise.resolve(); // one write at a time: each builds on the step before it
    const t = {
      app,
      owner,
      id,
      spaceId,
      absent: false,
      took: tail => took(tail),
      rows: () => rows,
      raw: () => core.tail_raw(id) ?? [],
      onChange: f => (changed.push(f), () => changed.includes(f) && changed.splice(changed.indexOf(f), 1)),
      // `value`: bytes, text, or a function giving the bytes at the moment of writing (a version names the feed's
      // next sequence, so it is made in the write's own turn).
      // (`user`: asked by the person — a click or key just now — so said on the save line; background writes are not.)
      put: (key, value, user = !!navigator.userActivation?.isActive) => (queue = ctx.background("storage: queue", queue).then(() => saving(write(key, value), user))),
      remove: (key, user = !!navigator.userActivation?.isActive) => (queue = ctx.background("storage: queue", queue).then(() => saving(write(key, ""), user))),
      tailNext: () => core.tail_next(id),
      // Ask again (a tail that was not there: its writer may have made it since).
      reread: () => read(),
      // LAZY (phase 3, Reads): its tree read only by PAGES — the blocks on a page's path, never the whole tree.
      lazy,
      page: o => pageOf(idHex, app, o),
      poke: () => changed.forEach(f => f()),
      // Wanted WHOLE after a lazy open: read whole — unless it is not there yet (a view would mark it made, and its
      // first write would then skip listing it in the catalog: a table nobody finds after a reload).
      // ONCE: every caller after the first shares that read (several wanted it whole at once: each read the tree again).
      whole: () => {
        if (!t.lazy && wholeRead) return wholeRead;
        t.lazy = false;
        return (wholeRead = t.absent ? Promise.resolve() : view().catch(e => ((wholeRead = null), Promise.reject(e))));
      },
      // KEEP it (phase 4, Lifecycle): see `keep` below.
      keep: () => (queue = ctx.background("storage: queue", queue).then(keep)),
      // WRITTEN OUT NOW (what is waiting in the tail, into the tree): before an EDIT, so the version it replaces is
      // in a tree — and its root logged — and every edit leaves the one before in its history.
      flushNow: () => (queue = ctx.background("storage: queue", queue).then(() => core.tail_pending(id) > 0 && flush())),
      // REPLACED with exactly `rows` (`[[key, value]]`, text) — a REPAIR only (`keys.repairAccount`): for a table no key
      // held here opens any more (another branch of the group sealed it over). What it held is not read; it is gone.
      replaceWith: list => (queue = ctx.background("storage: queue", queue).then(() => replace(list))),
      // ITS HISTORY of one row: its value at each root the table had (`[{ at, root, value }]`, oldest first; `at`: when
      // that tree was replaced, `value`: bytes, or null — no such row then, or sealed with a key not held here). The old
      // trees' blocks on the row's path fetched as any read's.
      history: async key => {
        const k = typeof key === "string" ? enc.encode(key) : key;
        const out = [];
        // Within the retention, whenever it is read (the log is pruned only by the next flush: a table not written
        // again keeps older entries, never shown).
        const since = Date.now() - daysFor(inSpace) * 86400e3;
        // The CURRENT tree too: a row changed since the last flush (in the tail) had its tree's value before that
        // change — an earlier version as much as a logged root's (its `at`: now — replaced by what the tail holds).
        const now = core.tail_root(id);
        const roots = [...core.tail_root_log(id), ...(now ? [[Date.now(), now]] : [])];
        for (const [at, root] of roots) {
          if (at < since) continue;
          try {
            const going = progress();
            for (let round = 0; ; round++) {
              const r = core.tail_value_at(id, root, k);
              if (!r.fetch) {
                out.push({ at, root, value: r.value });
                break;
              }
              if (!going({ kind: "tail-need", blocks: r.fetch })) throw new Error("its blocks did not arrive");
              await blocks.fetch(idHex, r.fetch, app);
            }
          } catch (e) {
            ctx.failed("history", e, `${app}: a version not read: ${e.message}`);
          }
        }
        return out;
      },
      // Whether the node ANSWERED the last read (the tail, or "not found"), not silence.
      answered: () => answered,
      // The first read to its end (the node's answer, or its silence past the read's wait): what a DECISION waits for.
      answer: () => ctx.background("storage: answer", answer ?? Promise.resolve()),
      // ITS READ STATE (FOUNDATION.md §2, the read machine): `{ s: "unknown" }` (not asked, silent, or shown before
      // its answer), `{ s: "absent" }` (the node ANSWERED not found, or the catalog that would list it says none),
      // `{ s: "present" }`, `{ s: "unreadable", why }`. `absent` above is what a page SHOWS; this is what a DECISION
      // reads.
      get state() {
      return st;
      },
      // DECIDED: the first read's end; still unknown (the node silent), asked once more with the long wait
      // (WAIT.answer). Never reports unknown as anything else.
      decided: () => decide(),
    };
    let answered = false; // the last read had the node's answer (the tail, or "not found"), not silence
    let st = { s: "unknown" }; // the read state (FOUNDATION.md §2)
    let deciding = null; // the second, long read of an undecided tail (shared)
    let longReadAt = 0; // when the last long read ended still undecided (not tried again within a minute)
    async function decide() {
      await t.answer();
      if (st.s !== "unknown") return st;
      if (Date.now() - longReadAt < 60000) return st;
      deciding ??= read(WAIT.answer)
        .catch(e => ctx.failed("table not decided", e, `${app}: ${e?.message ?? e}`))
        .finally(() => ((deciding = null), st.s === "unknown" && (longReadAt = Date.now())));
      await deciding;
      return st;
    }
    let wholeRead = null; // the read that made it whole (shared by every later `whole()`)
    let answer = null; // the first read, to its end: what a decision waits for
    // Each open's PHASES (ms from its start): what a page's measure reads (`opens()`).
    const ph = { app, t0: performance.now() };
    const mark = k => (ph[k] = Math.round(performance.now() - ph.t0));
    opensLog.push(ph);
    const ready = (async () => {
      // THE TABLE'S KEY: the table is sealed, so reading it needs its key, and the key comes only with the person's
      // grant for this site (asked NOW: the node prompts the first time). No grant: nothing of the table reads here.
      // The catalog's key comes with any grant: listing a table is part of using it.
      // A PUBLIC tail (a person's card): in the clear, no key.
      if (open) core.tail_public(id);
      const k = open ? {} : sealWith ? { key: sealWith } : await access.key(app, { catalog: catalogKey, space: spaceId });
      mark("key");
      t.sealed = !!k.key;
      // (Read now: its "no key" said before is solved.)
      if (k.key) (solved(spaceId, `${app}: no key (`), core.tail_seal(id, bytes(k.key)));
      else if (!open) (ctx.log("table sealed", { what: `${app}: no key here (${k.why}): nothing of it reads` }), problem(spaceId, `${app}: no key (${k.why})`, null, null, "key"));
      // WRITES are sealed with the account's newest EPOCH (a node removed from the account then reads nothing written
      // after): the group brought current first. Not a channel (a node reads the MLS channel before it has any epoch),
      // and the catalog not after the group (the group's channel is found through it). No group here: the table's key.
      if (k.key && !sealWith && !CHANNELS.has(app)) {
        const keys = await ctx.require("keys");
        if (inSpace?.id) await keys.group(inSpace).ready();
        else if (!catalogKey) await keys.ready();
        mark("group");
        const e = await access.keyAt(app, -1, { catalog: catalogKey, space: spaceId });
        mark("epoch");
        if (e.key) core.tail_epoch_key(id, e.epoch, bytes(e.key), true);
        else ctx.log("table sealed", { what: `${app}: no epoch here (${e.why}): written with the table's key` });
        // Kept: when a newer epoch arrives (the group moved: a node removed, one added), writes move onto it.
        t.sealing = { app, catalog: catalogKey, space: spaceId, sp: inSpace?.id ? inSpace : null, epoch: e.key ? e.epoch : -1, key: e.key ?? null };
        // A space's LOST BRANCH (`keys`' heal: this node's commits another beat): its epochs' keys read with, as
        // alternates — what was written there stays readable, and is sealed over to the winner's.
        if (inSpace?.id) await ctx.background("storage: alternates", alternates(id, app, inSpace));
        mark("alternates");
      }
      if (known === false) {
        core.tail_absent(id);
        t.absent = true;
        t.assumed = true; // absent by the caller's word, not the node's: read when asked for without it
        st = { s: "absent" };
        answer = Promise.resolve();
      } else {
        // SHOWN within WAIT.hint, DECIDED on the answer — the one rule of every read: what is held is shown after at
        // most WAIT.hint (no answer yet: shown as not there), and the table changes when the node answers; whatever
        // decides from it (a write, a place noted, a catalog read) waits for the answer (`t.answer()`).
        answer = read().finally(() => mark("read"));
        const shown = await within(answer.then(() => true), WAIT.hint, false);
        if (!shown) {
          t.absent = true;
          ctx.log("table shown before its answer", { what: `${app}: shown after ${WAIT.hint / 1000} s as held; merged when the node answers` });
        }
      }
      // Rows under an older key: sealed over now, a batch per step (in the write queue), where this node signs the tail.
      const sp = await space.account.strict();
      if ((legacy > 0 || reseal) && (owner === sp?.self || owner === sp?.shared)) queue = ctx.background("storage: queue", queue).then(sealOld);
      // Rows a page left in this node's own tail (it closed before its quiet flush): flushed once quiet here. A public
      // tail only after a write here (its writes need the person's grant, which a read of it does not show).
      else if (!open && (owner === sp?.self || owner === sp?.shared) && !t.absent && core.tail_pending(id) > 0) whenQuiet();
      return t;
    })();
    tails.set(idHex, ready);
    live.set(idHex, t);
    // R4: a failed open is never kept — the next asks again.
    ctx.optional("storage: ready", ready, () => tails.get(idHex) === ready && (tails.delete(idHex), live.delete(idHex)));
    t.keyless = () => !open && !t.sealed;

    const allowed = () => (catalogKey || spaceId ? Promise.resolve(true) : access.allowed(app));
    // EVERY WRITE IS A SAVE (FOUNDATION §3 `save`): OWED, then HELD or REFUSED with its fault's words — the lifecycle
    // says it in ONE place (the pill) and reports a refusal; an update waits for nothing to be owed.
    const saving = (p, user) => {
      const key = `${app}#${++saves}`;
      const lc = ctx.require("lifecycle");
      ctx.background("storage: save owed", lc.then(l => l.transition("save", key, "owe", { user, what: app })));
      return p.then(
        v => (ctx.background("storage: save held", lc.then(l => l.transition("save", key, "held"))), v),
        e => (ctx.background("storage: save refused", lc.then(l => l.transition("save", key, "refuse", { user, what: app, kind: e?.kind ?? "bug", why: ctx.say(e) }))), Promise.reject(e)),
      );
    };

    // Read it and follow it. None on the network: its first write is a PUT. No answer at all: the same as none, so
    // it opens empty; a first write the network then refuses resets it and reads it again.
    // Through the DOOR (FOUNDATION §2 R6): read once and followed; a HELD tail answers from the core's copy (its view,
    // with whatever keys are held now) — no GET. `again`: this page knows its copy is wrong (a write refused, its tail
    // reset): read from the node.
    async function read(ms = wait, { again = false } = {}) {
      if (again) wrote(idHex);
      let said;
      try {
        said = await contract(idHex, `reading ${app}`, { frames: () => core.tail_get(id)[1], match: x => (x.kind === "tail" || x.kind === "tail-need" || x.kind === "tail-keys" || x.kind === "tail-unreadable" || x.kind === "get-failed") && x.id === idHex, ms });
        if (said.held) said = JSON.parse(core.tail_view(id));
        answered = true;
      } catch (e) {
        // SILENT: shown empty, but UNKNOWN — never "not there" (R1: nothing is made over a table not answered for).
        ctx.log("table not found yet", { what: `${app}: ${e.message}; shown empty, not decided` }); // (counted in the visit's waits)
        answered = false;
        if (st.s === "unknown") t.absent = true;
        return;
      }
      // The tail is there: its tree must load too. A tree that does not is an error, never an empty table (that
      // would write over rows it could not see).
      // LAZY: the tail's own state is enough (its pages read the tree when asked).
      if (t.lazy && said.kind !== "get-failed") {
        t.absent = false;
        st = said.kind === "tail-unreadable" ? { s: "unreadable", why: said.said } : { s: "present" };
        if (said.kind === "tail") took(said.tail);
        else for (const f of changed) f();
      dataChanged(); // (a view reads it again only once it is here)
        return;
      }
      // Asked and not there: a network search spent (seconds, on a real network) — named, so what is asked for no
      // reason shows.
      if (said.kind === "get-failed" && answered) ctx.log("asked, not there", { what: `${app} (${label === app ? "by name" : "blinded"}, ${owner.slice(0, 8)}…)` });
      // UNREADABLE (the node answered; this page cannot open it): said at once — never waited on until the read's end.
      if (said.kind === "tail-unreadable") {
        t.absent = false;
        t.unreadable = said.said;
        st = { s: "unreadable", why: said.said };
        waiting.add(idHex);
        ctx.log("table unreadable", { what: `${app}: ${said.said}` });
        problem(spaceId, `${app}: unreadable (${said.said})`);
        return;
      }
      if (said.kind !== "get-failed")
        said = await settle(said, app).catch(e => {
          // There, but its tree did not open here (a key not held: unreadable; blocks not arriving: still unknown).
          if (e?.kind === "sealed") (st = { s: "unreadable", why: e.message }), waiting.add(idHex);
          throw e;
        });
      t.absent = said.kind !== "tail";
      st = said.kind === "tail" ? { s: "present" } : { s: "absent" };
      waiting.delete(idHex);
      if (said.kind === "tail") took(said.tail);
      else core.tail_absent(id);
    }

    // How long a step the identity signed, not yet on the network as read here, is waited for before it counts as
    // never landed (a page closed between signing and sending).
    const FORK_WAIT = 30000;
    // EVERY STEP of this tail (a write, a flush, a replace) signed through here. A step the identity refuses as a FORK
    // (it signed this tail further than this page holds): the network read again (`step`), the step prepared again.
    // Caught up — written on what it holds. The node ANSWERED and holds less — those signed steps never landed (or were
    // lost — none arrived in FORK_WAIT): this step goes past them, as a whole state. The node silent — nothing is overwritten on a guess: try again
    // once it answers. Another page of this account writing the same table at once (two tabs, a phone beside a
    // desktop on one node) is a fork every time either moves: caught up, tried again — after a short wait of its own
    // so the two do not meet again in step. (10-15: a flush refused `WouldFork { last_seq: 47 }` had no such path; and
    // two fx pages writing one table lost 7 of 97 writes — one try again was all a write had.)
    async function signed(prepare, doing) {
      let past = null;
      for (let attempt = 0; ; attempt++) {
        let p = await prepare();
        if (past != null) p = core.tail_skip(id, past) ?? p;
        try {
          return void (await step(p, doing));
        } catch (e) {
          if (e.forkedAt == null || attempt >= 16) throw e;
          // NOT YET SEEN is not NEVER LANDED: the other page's step is on its way to this node. Read again until it
          // arrives (10-15: going past at once wrote this page's whole state over steps that had landed — two fx pages
          // writing one table lost 41 of 97 rows, silently).
          const t0 = Date.now();
          for (let wait = 250; core.tail_next(id) <= e.forkedAt && Date.now() - t0 < FORK_WAIT; wait = Math.min(wait * 2, 4000)) {
            await new Promise(r => setTimeout(r, wait));
            await read(undefined, { again: true });
          }
          if (core.tail_next(id) > e.forkedAt) {
            await new Promise(r => setTimeout(r, 50 + Math.random() * 250 * Math.min(attempt + 1, 4)));
            continue;
          }
          if (past != null) throw e;
          if (!answered) throw new Error(`${app}: your node has written this further than it can read back right now — try again in a minute`);
          ctx.log("write", { what: `${app}: the identity signed through step ${e.forkedAt}, the network holds step ${core.tail_next(id) - 1} and nothing past it arrived in ${FORK_WAIT / 1000} s: written past it` });
          past = e.forkedAt;
        }
      }
    }

    // ROWS UNDER AN EPOCH WHOSE KEY IS NOT HERE (`unkeyed`: read as absent — right for reading another's history,
    // never for WRITING this one): nothing written, nothing flushed, until the key arrives. A flush rebuilds the tree from
    // the rows this page reads, so it dropped every row it could not (10-18, fx: after an update that left epoch 0
    // behind, a space's catalog was flushed with one row of six — its acts, board and keys unlisted, the space empty).
    const sealedBelow = doing => {
      const u = unkeyed.get(idHex);
      if (u?.epochs.size) throw new Fault("sealed", `${app}: ${doing}, rows under epoch(s) ${[...u.epochs].join(", ")} not readable here yet`, { message: `${app}: ${doing} — part of it is sealed with a key this device does not hold yet; it is saved once the key arrives`, d: { app, epochs: [...u.epochs] } });
    };
    // One write: prepared by the core, signed by the identity delegate with this tail's key, sent as one delta.
    async function write(key, value) {
      if (!(await allowed())) throw new Error(`this app may not change your “${app}”: allow it when your node asks`);
      if (refusing) throw new Error(refusing);
      // R1 (FOUNDATION.md §2): written only on a DECIDED table — made only where the node answered it is not there,
      // never over one it did not answer for (that would write over rows it could not see).
      const now = await t.decided();
      if (now.s === "unknown") throw new Fault("not-yet", `${app}: not saved, its table not read yet`, { message: `${app}: not saved — your node has not answered whether it exists yet; try again in a moment`, d: { app } });
      if (now.s === "unreadable") throw new Fault("sealed", `${app}: not saved, its table unreadable here`, { message: `${app}: not saved — ${now.why}`, d: { app } });
      sealedBelow("not saved");
      if (t.refuse) throw new Error(t.refuse);
      // Listed BEFORE it is created: a failure between the two leaves a listed tail that is empty, never one nobody
      // can find.
      if (now.s === "absent" && beforeCreate) await beforeCreate();
      await signed(() => {
        const v = typeof value === "function" ? value() : typeof value === "string" ? enc.encode(value) : value;
        return core.tail_prepare(id, enc.encode(key), v);
      }, "saving to");
      // FLUSH once the tail is long: its rows into the tree, the tail emptied (in this write's turn of the queue).
      if (Core.flush_due(core.tail_pending(id))) await flush().catch(e => ctx.failed("flush failed", e, `${app}: ${e.message}`));
      t.absent = false;
      st = { s: "present" };
      whenQuiet();
    }
    // FLUSH WHEN QUIET (phase 4, Lifecycle): rows still in the tail once writing has stopped for a while go into the
    // tree — erasure-coded, so they outlive this one tail contract. Each write starts the wait again.
    let quiet = null;
    function whenQuiet() {
      clearTimeout(quiet);
      quiet = setTimeout(() => {
        if (core.tail_pending(id) > 0) queue = ctx.background("storage: queue", queue).then(() => core.tail_pending(id) > 0 && flush()).catch(e => ctx.failed("flush failed", e, `${app}: ${e?.message ?? e}`));
      }, QUIET_MS + Math.random() * QUIET_MS); // spread: tables quiet together do not flush together
    }
    // ONE STEP of this tail, the one way a tail moves: the prepared step signed by the identity delegate, sent (the
    // first as a PUT, then one delta each), and confirmed. Refused anywhere — by the identity or by the node — it never
    // landed: what this page holds is dropped and the tail read again, so the next step builds on what the network has.
    async function step(p, doing) {
      const t0 = performance.now();
      const r = await auth.identity.sign(p.params, p.seq, p.valueHash, spaceId ?? undefined, blindedAs);
      let refused = r.signed ? null : `the identity said ${r.refused ?? JSON.stringify(r)}`;
      let kind = null;
      if (!refused) {
        const [k, frames] = core.tail_commit(id, r.signed);
        kind = k;
        const ok = k === "put" ? "put" : "updated";
        const said = await ask(frames, x => (x.kind === ok && x.key === name) || x.kind === "refused", `${doing} ${app}`, 60000);
        if (said.kind === "refused") refused = `the node said ${said.said}`;
        // MADE HERE by a PUT (which does not subscribe): read once through the door, so this page follows it — another
        // page's or device's writes then reach it (10-15: a table made here never heard of them, and after a reconnect
        // nothing followed it again).
        else if (k === "put") ctx.background("storage: follow what was made", read(wait, { again: true }));
      }
      ctx.log(refused ? "write refused" : "written", {
        what: `${app} seq ${p.seq}${refused ? `: ${refused}` : ` as ${kind === "put" ? "a PUT" : "an UPDATE (one delta)"}`}`,
        ms: Math.round(performance.now() - t0),
      });
      if (refused) {
        core.tail_reset(id);
        await read(wait, { again: true });
        const err = new Error(`the write was refused: ${refused}`);
        const fork = /WouldFork \{ last_seq: (\d+) \}/.exec(refused);
        if (fork) err.forkedAt = Number(fork[1]);
        throw err;
      }
      await view();
    }

    let legacy = 0;
    let reseal = false;
    function took(tail) {
      // Rows arrived (pushed, or read): it exists now, whatever an earlier read found.
      t.absent = false;
      st = { s: "present" };
      legacy = tail.legacy ?? 0;
      reseal = !!tail.rekey;
      t.info = { rows: tail.rows.length, pending: tail.pending, flushed: !!tail.root, legacy, unreadable: tail.unreadable ?? 0, writes: tail.writes };
      rows = tail.rows;
      for (const f of changed) f();
      dataChanged(); // (a view reads it again only once it is here)
    }
    async function view() {
      if (t.lazy) return void t.poke();
      const v = await settle(JSON.parse(core.tail_view(id)), app);
      took(v.tail);
    }

    // SEAL OVER the rows under an older key (the table's key, an earlier epoch, a lost branch's): 16 rows a step, each
    // sealed with the newest key and its old copy deleted in that same step, signed like any write. A tree under
    // another key than writes' (`rekey`) is built again whole under it by a flush.
    async function sealOld() {
      if (refusing) return;
      let n = 0;
      for (let round = 0; round < 64; round++) {
        const p = core.tail_migrate(id, 16);
        if (!p) break;
        await step(p, "sealing");
        n += 1;
      }
      if (n) ctx.log("sealed", { what: `${app}: rows under an older key, sealed over in ${n} step(s)` });
      if (reseal || Core.flush_due(core.tail_pending(id))) await flush();
    }

    // KEEP (phase 4, Lifecycle): the table's ASSET — every block its current tree reaches, in its groups — each block
    // ASKED (its health: a group is WHOLE with all its blocks there, DEGRADED with at least its k, DAMAGED below), then
    // PUT again (re-published where it is there; repaired — made from what is held, parity coded again — where it is
    // not), then the tail's signed state put again. In the write queue: no flush moves the tree under it.
    async function keep() {
      await t.answer();
      if (t.absent) return null;
      const t0 = performance.now();
      if (t.lazy) await t.whole();
      // Its groups: the current tree's, or (`history`) those only its logged roots reach.
      const assetOf = async history => {
        let a = null;
        for (let round = 0; round < 64; round++) {
          a = core.tail_asset(id, history);
          if (a.need) await settle({ kind: "tail-need", id: idHex, blocks: a.need }, app);
          else if (a.keys) await settle({ kind: "tail-keys", id: idHex, epochs: a.keys }, app);
          else break;
        }
        if (!a?.groups) throw new Error(`${app}: its ${history ? "history" : "tree"} did not finish loading`);
        return a.groups;
      };
      const current = await assetOf(false);
      // ITS HISTORY kept too (every version it can read back) — never holding up the current tree's keeping.
      const older = await assetOf(true).catch(e => (ctx.failed("history", e, `${app}: its history not kept: ${e.message}`), []));
      const out = { name: app, at: Date.now(), groups: current.length + older.length, history: older.length, blocks: 0, whole: 0, degraded: 0, damaged: 0, missing: 0, put: 0, unmade: 0 };
      for (const g of [...current, ...older]) {
        const there = await Promise.all(g.slots.map(([, c]) => blocks.probe(c, `keeping ${app}`)));
        const present = there.filter(Boolean).length;
        const parity = g.slots.length - g.k;
        out.blocks += g.slots.length;
        out.missing += g.slots.length - present;
        out[present - g.k >= parity ? "whole" : present >= g.k ? "degraded" : "damaged"] += 1;
        // Only what did not answer is put (rebuilt from its group where needed): re-putting every block read floods the
        // node until its own reads fail (measured on files 2026-10-06: 256 of 433 unanswered re-putting all, 0 reading).
        const puts = [];
        g.slots.forEach(([b], i) => {
          if (there[i]) return;
          const p = core.tail_keep_put(id, b);
          if (p) puts.push(p);
          else out.unmade += 1;
        });
        await blocks.put(puts, `keeping ${app}`);
        out.put += puts.length;
      }
      const said = await ask(core.tail_keep_state(id), x => (x.kind === "put" && x.key === name) || x.kind === "refused", `keeping ${app}`, 60000);
      if (said.kind === "refused") throw new Error(`${app}: its state was refused: ${said.said}`);
      out.ms = Math.round(performance.now() - t0);
      ctx.log("kept", { what: `${app}: ${out.blocks} block(s) in ${out.groups} group(s) — ${out.whole} whole, ${out.degraded} degraded, ${out.damaged} damaged; ${out.put} put again (not answering)${out.unmade ? `, ${out.unmade} not made here` : ""}`, ms: out.ms });
      return out;
    }

    // FLUSH: the tree's new blocks PUT first (a tail must never name a root whose blocks are not there), then the step
    // naming the new root, signed like any write.
    // A TREE STEP (a flush, a replace): its blocks loaded and put, then the step — prepared again whole on a fork.
    async function treeStep(make, doing) {
      let made = null;
      await signed(async () => {
        for (let round = 0; round < 16; round++) {
          const f = make();
          if (f.need || f.keys) {
            await settle(f.need ? { kind: "tail-need", id: idHex, blocks: f.need } : { kind: "tail-keys", id: idHex, epochs: f.keys }, app);
            continue;
          }
          try {
            await blocks.put(f.puts, app);
          } catch (e) {
            core.tail_reset(id);
            await read(wait, { again: true });
            throw e;
          }
          return (made = f);
        }
        throw new Error("the tree did not finish loading");
      }, doing);
      return made;
    }
    async function flush() {
      sealedBelow("not flushed");
      const t0 = performance.now();
      // ITS HISTORY: the root this flush replaces, and the log as it is BEFORE it (a flush drops the log).
      let was, prior;
      const f = await treeStep(() => ((was = core.tail_root(id)), (prior = core.tail_root_log(id)), core.tail_flush(id)), "flushing");
      ctx.log("flushed", { what: `${app}: ${f.puts.length} tree block(s) put, the tail emptied (seq ${f.seq})`, ms: Math.round(performance.now() - t0) });
      // …then the replaced root LOGGED (its history: kept for the retention — every row as it was then reads back).
      if (was) {
        const now = Date.now();
        // The space's retention (0: Off — the log pruned to nothing, the replaced root not kept).
        const days = daysFor(inSpace);
        const l = core.tail_log_root(id, prior, was, now, now - days * 86400e3, days ? HISTORY.roots : 0);
        if (l) await step(l, "keeping its history").catch(e => ctx.failed("history", e, `${app}: its history not logged: ${e.message}`));
      }
    }

    // REPLACE (see `replaceWith`): a new tree of `list` alone under the key writes use — its blocks put first, then the
    // step, as a flush.
    async function replace(list) {
      const given = list.map(([k, v]) => [enc.encode(k), typeof v === "string" ? enc.encode(v) : v]);
      const f = await treeStep(() => core.tail_replace(id, given), "replacing");
      ctx.log("replaced", { what: `${app}: replaced with ${list.length} row(s) (repair), seq ${f.seq}` });
      st = { s: "present" };
      waiting.delete(idHex);
      await view();
    }

    return ready;
  }

  // A feed as it stood at sequence `cap`: its versions up to there (`feed::within_cap`, the node's too).
  const withinCap = (v, cap) => !!v && feed.within_cap(v, cap);
  const capped = (f, cap) => ({
    owner: f.owner,
    raw: () => (f.raw() ?? []).filter(([, v]) => withinCap(v, cap)),
    page: async o => {
      const p = await f.page(o);
      return { ...p, rows: p.rows.filter(([, v]) => withinCap(v, cap)) };
    },
    whole: () => f.whole?.(),
    answer: () => f.answer?.() ?? Promise.resolve(),
    decided: () => f.decided?.() ?? Promise.resolve(f.state ?? { s: "unknown" }),
    onChange: fn => f.onChange(fn),
    get absent() {
      return f.absent;
    },
    get state() {
      return f.state;
    },
    get info() {
      return f.info;
    },
  });
  // Each of `nodes`' feeds in a space, as it stands now: `{ node: { table: last sequence } }` — what a departure
  // records before the commit that takes them out.
  async function headsOf(sp, nodes) {
    const scope = scopeOf(sp);
    const out = {};
    await Promise.all(
      nodes.map(async node => {
        const cat = await ctx.optional("storage: scope.catalogOf", scope.catalogOf(node), () => null);
        await cat?.answer();
        if (!cat || cat.absent) return;
        const heads = {};
        await Promise.all(
          listedIn(cat).map(async r => {
            const t = await ctx.optional("storage: tail", tail(node, r.key, { ...scope.opts(r.key), catalog: cat }), () => null);
            await t?.answer();
            if (!t || t.absent) return;
            const seqs = (t.raw() ?? []).map(([, v]) => (v ? feed.seq_of(v) : NaN)).filter(Number.isFinite);
            if (seqs.length) heads[r.key] = Math.max(...seqs);
          }),
        );
        out[node] = heads;
      }),
    );
    return out;
  }
  // A feed's own current rows, merged alone: `[{ key, value, id }]` (text; its deletes left out).
  const decoded = rows => rows.map(r => ({ key: dec.decode(r.key), value: dec.decode(r.value), id: r.id }));
  const own = t => decoded(Array.from(feed.merge_feeds([[bytes(t.owner), t.raw()]])));
  // A CATALOG's tables: its rows but `HERE` — the row a member's catalog of a space is made with, the first time it
  // opens the space, so a catalog that is not there means a member never there (never asked for it in vain).
  const HERE = "~here";
  const listedIn = cat => own(cat).filter(r => r.key !== HERE);
  // ONE CATALOG MODEL. A catalog is a writer's list of its tables, COMPLETE: a table it does not list it has none of
  // (never looked for). A node's holds versions; the account's DIRECTORY (its channels and nodes) and a person's CARD
  // (their public tables) hold plain rows.
  const plain = cat => cat === directoryTail || cat?.plainCatalog === true;
  // A table's PLACE, the one reader of it: "unlisted" (not listed: none), "none" (listed, never made), "blinded" (at
  // its blinded name), "listed" (a table found by its name: there) — or "unknown": R2 (FOUNDATION.md §2), "not
  // listed" is an answer only when the catalog was READ (present, or answered absent); a catalog silent or unreadable
  // lists nothing either way.
  // DECIDED (R5, the one rule — `space.mine`'s too): no state kept (a table read whole), or the node answered present
  // or absent. Anything else (unknown, unreadable) is never the answer "not in it".
  const decided = st => !st || st.s === "present" || st.s === "absent";
  const decidedCatalog = cat => decided(cat?.state);
  const placeOf = (cat, n) => {
    if (!decidedCatalog(cat)) return "unknown";
    const r = (plain(cat) ? cat.rows() : own(cat)).find(x => x.key === n && x.value);
    // (`data::catalog_place`, the node's too.)
    return r ? Core.catalog_place(r.value) : "unlisted";
  };
  // NOTE a table's place in a catalog this account writes, the one writer of it: "blinded", "none" or "listed".
  const notePlace = (cat, n, p) => {
    if (placeOf(cat, n) === p) return Promise.resolve();
    const v = Core.catalog_value(p, Date.now());
    return plain(cat) ? cat.put(n, v) : versioned(cat, n, v, own(cat).find(r => r.key === n)?.id);
  };
  // A write to a feed, as a VERSION replacing `after` (the id of the version current where it is written).
  // (`user`: the person's own save — said by the `save` machine; carried from the merged table's put.)
  const versioned = (t, key, value, after, user) =>
    t.put(key, () => feed.version(bytes(t.owner), t.tailNext(), after ?? "", value === null ? undefined : enc.encode(value)), user);

  // THE DIRECTORY: the account's shared catalog (its data key). Its rows: its channels and public tables found by name,
  // and `node:<key>` for every node that has feeds; its `CATALOG` row keeps upkeep's marks. Made with the account.
  let directoryOpen = null;
  let directoryTail = null; // the directory, once open: a catalog of plain rows (a node's catalog: versions)
  function directory() {
    const p = (directoryOpen ??= (async () => {
      const sp = await space.account.strict();
      const d = await tail(sp.shared, CATALOG, { known: sp.fresh ? false : null, catalogKey: true });
      await d.answer();
      if (d.absent && sp.fresh) await d.put(CATALOG, JSON.stringify({ at: Date.now() }));
      directoryTail = d;
      return d;
    })());
    // R4: a directory that did not open is asked again next time.
    ctx.optional("storage: p", p, () => directoryOpen === p && (directoryOpen = null));
    return p;
  }
  // Whether the directory lists `name`: not listed (false) is never looked for; listed (null) is read.
  const inDirectory = (d, name) => (["none", "unlisted"].includes(placeOf(d, name)) ? false : null);
  async function listInDirectory(key) {
    const d = await directory();
    if (d.rows().some(r => r.key === key)) return;
    await d.put(key, Core.catalog_value("blinded", Date.now())); // made new: at its blinded name
  }

  // A WRITER's CATALOG feed: the tables it has a feed of. Known to exist once the directory lists its node.
  async function catalogOf(writer) {
    const d = await directory();
    // Not listed — never made — only when the directory was READ (R2); otherwise it is read.
    const notListed = decidedCatalog(d) && !d.rows().some(r => r.key === `node:${writer}`);
    const t = tail(writer, CATALOG, { known: notListed ? false : null, catalogKey: true, beforeCreate: () => listInDirectory(`node:${writer}`) });
    return notListed ? whenListedIn(d, () => d.rows().some(r => r.key === `node:${writer}`), t) : t;
  }
  // The other writers whose feeds count: the nodes the directory lists, those the account's group holds (where it is
  // kept; elsewhere every listed node, until membership is gossiped in the feeds).
  async function writers(me) {
    const d = await directory();
    const listed = d.rows().filter(r => r.key.startsWith("node:")).map(r => r.key.slice(5));
    const members = await ctx.optional("storage: writers", (await ctx.require("membership")).writers(), () => null);
    return listed.filter(k => k !== me && (!members || members.includes(k)));
  }

  // A CHANNEL: one shared tail, listed in the directory before it is made.
  async function channel(name) {
    const sp = await space.account.strict();
    // The DIRECTORY is not asked: a node that just logged in with the words cannot open it yet — it is sealed with the
    // account's epoch keys, which this very channel's escrows give it (10-12: every words login on a new node failed
    // here — "tables: its tree is sealed with a key this page does not hold"). The channel is keyed by the account's
    // data key, which the words give.
    // So the directory never decides that the channel is ABSENT: a sealed directory reads as empty on such a node, and
    // "not listed" made it start a SECOND group for the account (fx/fy 10-12: "this node created the account's group"
    // instead of joining it). The channel is always read for what it is.
    // A REGISTRATION's channel cannot exist yet (the account was made this moment): known absent, never waited on
    // (audit 10-13: on a slow node the long read ended undecided and the new group was never made).
    const t = tail(sp.shared, name, { known: sp.fresh ? false : null, beforeCreate: () => ctx.background("storage: listInDirectory", listInDirectory(name)) });
    if (!sp.fresh) return t;
    const d = await directory();
    return whenListedIn(d, () => d.rows().some(r => r.key === name), t);
  }
  // THE SPACE's MEMBERS and their CURRENT devices: the group's members whose credential is really their DID's (signed
  // by the data key its key log names — a credential names any DID it likes), each DID's devices (`directory.devices`:
  // the one resolution). → { members: [{ did, key, index }], devices: [[did, nodes]], nodes }.
  async function currentWriters(sp) {
    const [keys, directory] = await Promise.all(["keys", "directory"].map(n => ctx.require(n)));
    const st = await keys.group(sp).ready.strict();
    const byte = h => new Uint8Array(h.match(/../g).map(x => parseInt(x, 16)));
    // (Each credential read by the one reader — `identity::read_credential`, its signature checked.)
    const all = (st?.members ?? []).filter(m => m.cred).map(m => ({ m, c: JSON.parse(glue.credential_parts(byte(m.cred)) ?? "null") })).filter(x => x.c).map(({ m, c }) => ({ key: m.key, index: m.index, did: c.did, signer: c.owner }));
    const real = await Promise.all(all.map(async m => (await directory.dataKey(m.did)) === m.signer));
    for (const [i, m] of all.entries()) if (!real[i]) ctx.log("storage", { what: `${sp.name ?? sp.id.slice(0, 8)}: a member claims ${m.did.slice(12, 20)}… with a key that is not its: left out` });
    const members = all.filter((_, i) => real[i]);
    // WHOSE DEVICES are read (`account::reading_dids`, the node's too): the group's members and the space's owner.
    const dids = JSON.parse(glue.CraftworksCore.reading_dids(JSON.stringify(members.map(m => m.did)), sp.governance?.owner ?? undefined));
    const sets = await Promise.all(dids.map(d => directory.devices.strict(d)));
    const devices = dids.map((d, i) => [d, sets[i]]);
    return { st, members, devices, nodes: [...new Set(sets.flat())] };
  }
  // WHO WROTE A SPACE's ROWS — THE ONE MAP (owner 10-16: "any question about access should be centralized"): its
  // members' current devices and its DEPARTED writers (a member put out, or a device dropped from its account — the
  // space's `departed` table), combined by the core (`space_writers` = `craftworks_account::writers`, the node's too):
  // `{ members, byNode: Map(node → { did, caps }) }` (`caps`: a departed writer's — counted up to there).
  // From OUTSIDE (`roster`: the DIDs its public acts name; `departedRows`: its `pub-departed`): the same rule.
  async function writersOf(sp, { roster = null, departedRows = null } = {}) {
    const directory = await ctx.require("directory");
    const cur = roster ? { members: [], devices: await Promise.all(roster.map(async d => [d, await directory.devices(d)])) } : await currentWriters(sp);
    const rows = departedRows ?? (await (async () => {
      const t = await merged(space.tableOf(sp, "departed"), sp);
      await t.settled;
      return t.rows();
    })());
    const list = JSON.parse(glue.CraftworksCore.space_writers(JSON.stringify(cur.devices), JSON.stringify(rows.filter(r => r.value).map(r => [r.key, r.value]))));
    return { members: cur.members, byNode: new Map(list.map(w => [w.node, { did: w.did, caps: w.caps }])) };
  }

  // A SCOPE: a space seen from this node — who writes in it, where each writer lists its tables, how its tails open.
  // The account: the directory names its nodes, their catalogs are account tables. Another space: its GROUP's members write, each listing the space's tables in its own catalog
  // feed of the space; every tail of it is the space's (sealed with its epoch keys, signed in it).
  function scopeOf(sp) {
    if (sp.kind === "account")
      return {
        key: "account",
        self: sp.self,
        opts: name => ({ catalogKey: ANY_GRANT.has(name) }),
        writers: () => writers(sp.self),
        // New nodes of the account: listed in its directory.
        watch: fn => directory().then(d => d.onChange(fn)),
        catalogOf,
      };
    return {
      key: sp.id,
      self: sp.self,
      // A PUBLIC table of the space (its own name starts `pub-`: a public board, the public acts): written in the
      // clear, so anyone reads it; still signed in the space (only its members write it).
      opts: name => ({ space: sp, public: /^x[0-9a-f]{12}-pub-/.test(name) }),
      // Its WRITERS BAG: who has a catalog here (`index`'s sealed bag of the space). Its writers are read from it.
      bag: fresh => writersBag(sp, fresh),
      list: ws => listWriters(sp, ws),
      // Its WRITERS NOW: every current device of every member (`writersOf`'s current part) — this device's siblings too.
      writers: async () => (await currentWriters(sp)).nodes.filter(k => k !== sp.self),
      // New writers: a member's devices changed (their card), or the members did (the group moved).
      watch: fn => {
        const mine = e => !e.detail?.space || e.detail.space === sp.id;
        addEventListener("craftworks:keys", e => mine(e) && fn());
        ctx.background("storage: async", (async () => {
          const [keys, directory] = await Promise.all(["keys", "directory"].map(n => ctx.require(n)));
          const members = (await keys.group(sp).ready.strict())?.members ?? [];
          const watched = new Set();
          const each = () => {
            for (const m of members.filter(m => m.cred)) {
              const d = glue.credential_did(new Uint8Array(m.cred.match(/../g).map(x => parseInt(x, 16))));
              if (!watched.has(d)) watched.add(d), directory.onDevices(d, fn);
            }
          };
          each();
          addEventListener("craftworks:keys", async e => {
            if (!mine(e)) return;
            members.splice(0, members.length, ...((await keys.group(sp).ready())?.members ?? []));
            each();
          });
        })());
      },
      // This device's catalog listed in the space's writers bag BEFORE it is made: nobody asks for it before then.
      catalogOf: w =>
        tail(w, sp.tables.catalog, {
          space: sp,
          ...(w === sp.self ? { beforeCreate: () => listWriters(sp, [w]).catch(e => ctx.failed("storage", e, `${sp.name ?? sp.id}: listing this device as a writer: ${e?.message ?? e}`)) } : {}),
        }),
      // DEPARTED writers (removed, banned, left): the space's table `departed` — written by the member who took their
      // nodes out of the group, BEFORE the commit — gives each node's feeds' last sequence then (`heads`), and what
      // they wrote up to there still counts; nothing after (they keep older epochs' keys). The table itself: none.
      departedTable: () => merged(space.tableOf(sp, "departed"), sp),
      departed: name => name !== space.tableOf(sp, "departed"),
    };
  }

  // A writer's catalog not there yet (a device that has not written here): asked again, soon at first (it may be
  // writing now), then less and less often, to every 5 min — each ask of a missing contract is a whole network search
  // (6 s to over a minute on the node). ONE poll per catalog, however many tables wait on it (each follows its changes).
  // Followed through the door (R6): the door alone asks again on its backoff, and says when it appears.
  const polled = new Set();
  function absentCatalog(c) {
    if (polled.has(c)) return;
    polled.add(c);
    const stop = onContract(hexOf(c.id), () => ctx.background("storage: c.reread", c.reread().then(() => !c.absent && (polled.delete(c), stop()))));
  }

  // A TABLE: the merge of its writers' feeds, and this node's feed to write.
  // A SPACE's WRITERS BAG (`index`'s sealed bag): the members' devices that have a catalog there — each listed BEFORE
  // its catalog is made (`catalogOf`), so it is the whole list: nobody's catalog is asked for before they have one.
  // Read once per space (again when asked).
  const bags = new Map(); // space id → Promise<{ set }>
  const copied = new Set(); // `space|writer` listed again with the space's own key
  // A FRESH read shared: one in flight, or one done in the last few seconds, answers every table that asks (each open
  // table asked on every key or device change: a dozen identical reads at once).
  const freshAt = new Map(); // space id → when its last fresh read began
  function writersBag(sp, fresh = false) {
    if (fresh && bags.has(sp.id) && performance.now() - (freshAt.get(sp.id) ?? -1e9) < 4000) return bags.get(sp.id);
    if (fresh) freshAt.set(sp.id, performance.now());
    if (fresh || !bags.has(sp.id))
      bags.set(
        sp.id,
        (async () => {
          const t0 = performance.now();
          const index = await ctx.require("index");
          const items = await index.spacePointers.strict(sp, "writers");
          // `blind`: entries no key here opens — who they name is unknown, so every member's device is read (a
          // missing catalog only costs a lookup, asked less and less often).
          const b = { set: new Set(items.map(i => i?.w).filter(w => typeof w === "string")), blind: (items.unread ?? 0) > 0, pub: new Set(items.filter(i => typeof i?.w === "string" && typeof i?.pub === "string").map(i => `${i.w}|${i.pub}`)) };
          ctx.log("storage", { what: `${sp.name ?? sp.id.slice(0, 8)}: writers bag — ${b.set.size} writer(s)`, ms: Math.round(performance.now() - t0) });
          // A writer listed only under an EPOCH (before the list was sealed with the space's own key): listed again as
          // every member reads it, by whoever reads it now — once per page and writer.
          for (const w of b.set)
            if (!copied.has(`${sp.id}|${w}`) && !items.some(i => i?.w === w && i.sealedAt === index.OPEN)) {
              copied.add(`${sp.id}|${w}`);
              index.spacePoint(sp, "writers", { w }).catch(e => (copied.delete(`${sp.id}|${w}`), ctx.failed("storage", e, `${sp.name ?? sp.id.slice(0, 8)}: listing ${w.slice(0, 8)} for every member: ${e?.message ?? e}`)));
            }
          return b;
        })().catch(e => (bags.delete(sp.id), Promise.reject(e))),
      );
    return bags.get(sp.id);
  }
  // A writer's PUBLIC table named in the space's OPEN writers list (`{ w, pub }`): what a reader whose catalog read is
  // sealed to it (a newcomer, before it holds the space's epochs) looks at — only what is listed is looked up.
  async function listPublic(sp, w, app) {
    const b = await writersBag(sp);
    if (b.pub?.has(`${w}|${app}`)) return;
    await (await ctx.require("index")).spacePoint(sp, "writers", { w, pub: app });
    b.pub?.add(`${w}|${app}`);
  }
  const publicListed = async (sp, w, app) => !!(await ctx.optional("storage: writersBag", writersBag(sp), () => null))?.pub?.has(`${w}|${app}`);
  async function listWriters(sp, ws) {
    const index = await ctx.require("index");
    const b = await ctx.optional("storage: writersBag", writersBag(sp), () => ({ set: new Set() }));
    for (const w of ws)
      if (!b.set.has(w)) {
        await index.spacePoint(sp, "writers", { w });
        b.set.add(w);
      }
  }

  // The bag FOLLOWED (R6): each change the node pushes hands its set to every table of the space that waits on new
  // writers — one follow for the whole space, however many tables; never read again on a timer.
  const bagPolls = new Map(); // space id → [fn]
  function pollBag(sp, scope, fn) {
    if (bagPolls.has(sp.id)) return void bagPolls.get(sp.id).push(fn);
    const fns = [fn];
    bagPolls.set(sp.id, fns);
    ctx.background("storage: bag follow", ctx.require("index").then(index => index.watchSpace(sp, "writers", async () => {
      const b = await ctx.optional("storage: scope.bag", scope.bag(true), () => null);
      if (b) for (const f of fns) f(b.set, b);
    })));
  }

  const tables = new Map(); // scope + name -> Promise<table>
  function merged(name, sp, { lazy = false } = {}) {
    const scope = scopeOf(sp);
    const at = `${scope.key}/${name}`;
    // Opened already: as it is — or, opened LAZY and now wanted whole, every feed read whole.
    if (tables.has(at)) return lazy ? tables.get(at) : tables.get(at).then(t => (t.lazy ? t.whole().then(() => t) : t));
    const ready = (async () => {
      if (sp.kind === "account" && !sp.shared) throw new Error("this node does not hold the account's data key: log in once with the recovery words");
      const [mine, nowWriters] = await Promise.all([scope.catalogOf(scope.self).then(async c => (await c.answer(), c)), scope.writers()]);
      // A space: its WRITERS BAG says whose catalogs exist — only those are read (the account: its directory's nodes).
      const bag = scope.bag ? await ctx.optional("storage: scope.bag", scope.bag(), () => null) : null;
      // THE WRITERS (`account::table_writers`, the node's too): the members' devices the bag lists (all, a blind bag) and,
      // a public table, the open list's writers for it. (The account: its directory's nodes, no bag.)
      const named = bag?.pub ? [...bag.pub].filter(x => x.endsWith(`|${name}`)).map(x => x.split("|")[0]) : [];
      const others = scope.bag && bag ? JSON.parse(glue.CraftworksCore.table_writers(JSON.stringify(nowWriters), JSON.stringify([...bag.set]), !!bag.blind, JSON.stringify(named), !!scope.opts(name).public, scope.self)) : nowWriters;
      const listed = o => !scope.bag || (!!bag && others.includes(o));
      const lists = (cat, n) => own(cat).some(r => r.key === n);
      const all = [];
      // Another writer's feed that does not open here (sealed under a key this node lacks — a node that has not
      // recovered its epochs) is left out and COUNTED, never fatal: this node writes only its own feed, so nothing it
      // cannot see is written over. Its own feed must open.
      let unopened = 0;
      const failedOwners = new Set(); // writers whose feed did not open here: gathered again when keys arrive (R3)
      const opts = scope.opts(name);
      const theirs = (owner, known, cat = null) =>
        tail(owner, name, { known, ...(cat ? { catalog: cat } : {}), ...opts, lazy }).catch(e => {
          unopened += 1;
          failedOwners.add(owner);
          ctx.failed("feed not read", e, `${name}: ${owner.slice(0, 12)}…: ${e.message}`);
          problem(sp.id, `${name}: ${owner.slice(0, 8)}…'s feed not read (${e.message})`, sp, name, "feed");
          return null;
        });
      let rows = [];
      const changed = [];
      const remerge = () => {
        rows = decoded(Array.from(feed.merge_feeds(all.map(f => [bytes(f.owner), f.raw()]))));
        for (const f of changed) f();
      dataChanged(); // (a view reads it again only once it is here)
      };
      // Wanted WHOLE (a lazy table, then everything asked of it): a feed that arrives after (another writer's, found
      // late) read whole too — opened as the table first was (lazy), its rows would never join what is read.
      let wantWhole = false;
      const take = f => {
        if (!f || all.includes(f)) return;
        all.push(f);
        f.onChange(remerge);
        remerge();
        if (wantWhole && f.lazy) ctx.background("storage: f.whole", f.whole?.().then(remerge));
      };
      // This node's: listed in its catalog BEFORE it is made (`tail`, by its catalog). The table opens once it is open.
      ownKeys.add(scope.self);
      const me = await tail(scope.self, name, { ...opts, lazy, catalog: mine });
      take(me);
      // The OTHER writers' feeds, in the background: each merged in as it arrives, never holding the table up. A writer
      // whose catalog is not there yet (a node that has not written here), or does not list this table yet, is asked
      // again — its catalog is followed once it exists, and a missing one is asked every 30 s — so what it writes later
      // arrives without a reload.
      const opened = new Set();
      const notOpened = new Map(); // writer → its open (a public table): asked again as the open writers list changes
      const gather = async o => {
        const c = await ctx.optional("storage: scope.catalogOf", scope.catalogOf(o), () => null);
        if (!c) return;
        await c.answer();
        const open = async () => {
          // Listed in its catalog — or, a PUBLIC table, named in the space's open writers list (`listPublic`: what a
          // reader whose catalog read is sealed to it reads by; fx 10-17, a newcomer read 0 feeds of `pub-keyops`).
          if (opened.has(o) || !(lists(c, name) || (opts.public && sp.kind !== "account" && (await publicListed(sp, o, name))))) return;
          opened.add(o);
          take(await theirs(o, true, c));
        };
        c.onChange(() => ctx.background("storage: open", open()));
        // (A PUBLIC table not opened yet: tried again when the open writers list changes — it may name it later.)
        if (opts.public) notOpened.set(o, open);
        if (c.absent) {
          // Listed, not there yet (being made as it was listed): asked again until it is.
          absentCatalog(c);
          return;
        }
        found.add(o);
        await open();
      };
      const found = new Set();
      // DEPARTED writers' feeds, capped where they stood when they left (a space's).
      const departedDone = new Set();
      // (Who they are and up to where: the one map, `writersOf`.)
      const gatherDeparted = async () => {
        if (!scope.departed?.(name)) return;
        const { byNode } = await writersOf(sp);
        for (const [node, w] of byNode) {
          const cap = w.caps?.[name];
          if (typeof cap !== "number" || node === scope.self || departedDone.has(node)) continue;
          departedDone.add(node);
          const cat = await ctx.optional("storage: scope.catalogOf", scope.catalogOf(node), () => null);
          await cat?.answer();
          const f = await theirs(node, true, cat);
          if (f) take(capped(f, cap));
        }
      };
      const gathering = Promise.all([...others.filter(listed).map(o => ctx.background("storage: gather", gather(o))), gatherDeparted().catch(e => ctx.failed("feed not read", e, `${name}: departed writers: ${e.message}`))]);
      const settled = soon(gathering, name);
      // GATHERED: every listed writer's feed tried (not merely shown-after-WAIT.hint): what DECIDES on the table waits
      // for it (R5) — 10-13: the spaces list "decided" after 1.5 s while the other device's feed was still being read
      // read empty, and a recovered space showed "Not listed".
      let gathered = false;
      gathering.finally(() => (gathered = true));
      // New writers appear in the bag — read again now and then (one read for the whole space).
      if (scope.bag)
        pollBag(sp, scope, (set, b) => {
          // (A public table: a writer the open list names for it NOW counts too — named after this table opened.)
          const pubNow = o => scope.opts(name).public && !!b?.pub?.has(`${o}|${name}`);
          set.forEach(o => o !== scope.self && (others.includes(o) || pubNow(o)) && !seen.has(o) && (seen.add(o), ctx.background("storage: gather", gather(o))));
          // A PUBLIC table's writers found but not opened: the list may name it now (fx 10-18: named after this table
          // first looked, a newcomer read it only when the writer's catalog happened to change — or never).
          for (const [o, open] of notOpened) if (opened.has(o)) notOpened.delete(o);
          else ctx.background("storage: open public", open());
        });
      if (scope.departed?.(name)) ctx.background("storage: scope.departedTable", scope.departedTable().then(d => d.onChange(() => ctx.background("storage: gatherDeparted", gatherDeparted()))));
      // A WRITER NEW since the table opened (a member's new device, a new member): gathered when the scope says so.
      const seen = new Set(others.filter(listed));
      // The bag AS HELD answers; read again only for a writer it does not list (10-15, owner's trace: every directory
      // change — each card read — read every space's bag again, one every ~5 s for as long as the page was open).
      scope.watch?.(async () => {
        const fresh = (await ctx.optional("storage: scope.writers", scope.writers(), () => [])).filter(o => !seen.has(o));
        if (!fresh.length) return;
        let b = scope.bag ? await ctx.optional("storage: scope.bag", scope.bag(false), () => bag) : null;
        if (b && !b.blind && fresh.some(o => !b.set.has(o))) b = await ctx.optional("storage: scope.bag", scope.bag(true), () => b);
        const now = fresh.filter(o => !scope.bag || b?.blind || b?.set.has(o));
        for (const o of now) seen.add(o), ctx.background("storage: gather", gather(o));
        // A NEW WRITER of a space's table: read now, or not listed in its writers bag yet (read when it is) — said, so a
        // member's write that does not show is seen where it stopped (fx 10-17: one stopped here for minutes, unrecorded).
        // (Read: routine, journaled. One WAITING — not in the writers bag yet — REPORTED: what a user would otherwise only
        // see as a member's writing that never shows.)
        if (sp?.id && sp.kind !== "account") (await ctx.require("lifecycle")).transition("group", sp.id, fresh.length > now.length ? "writers-waiting" : "writers-found", { table: name.slice(-12), fresh: fresh.length, read: now.length, waiting: fresh.length - now.length, why: fresh.length > now.length ? `${fresh.length - now.length} new writer(s) not in the writers bag yet` : undefined });
      });
      ctx.log("table open", { what: `${name}: ${rows.length} row(s) from ${all.filter(f => !f.absent).length} feed(s)` });
      // A write in a space: its group brought current first, so what is written is sealed with the newest epoch's key
      // (never one a member removed since still holds).
      const current = sp.kind === "account" ? async () => {} : async () => ctx.optional("storage: group", (await ctx.require("keys")).group(sp).ready({ fresh: true }), () => null);
      // A PAGE across the writers (phase 3, Reads): each feed's page, their versions merged, the newest `limit` rows
      // kept — each feed has at most `limit` keys at or past the page's last, so none is skipped — and where to go on.
      // `{ lo, hi, before, limit }` (keys as text; `before`: continue below that key).
      const page = async ({ lo = "", hi = "", before = "", limit = 50 } = {}) => {
        // A writer's page not answered within WAIT.hint is NAMED (which table, whose feed, what range) — what holds a list.
        const timed = f => {
          const t0 = performance.now();
          const slow = setTimeout(() => ctx.log("page slow", { what: `${name}: ${f.owner.slice(0, 12)}…'s page [${lo || "start"}…${hi || "end"}] not answered in ${WAIT.hint / 1000} s` }), WAIT.hint);
          return f.page({ lo, hi, after: before, reverse: true, limit }).then(
            p => (clearTimeout(slow), performance.now() - t0 > WAIT.hint && ctx.log("page slow", { what: `${name}: ${f.owner.slice(0, 12)}…'s page answered`, ms: Math.round(performance.now() - t0) }), { f, p }),
            e => (clearTimeout(slow), ctx.failed("page slow", e, `${name}: ${f.owner.slice(0, 12)}…'s page failed: ${e?.message ?? e}`), null),
          );
        };
        // The same rule as a read: the writers' pages that answered within WAIT.hint shown; a later one, when it comes,
        // tells the table's readers to read again (`onChange`) — never waited on.
        let late = false;
        const pages = all.filter(f => !f.absent && f.page).map(f => {
          const p = timed(f);
          return within(p, WAIT.hint).then(x => {
            if (x !== undefined) return x;
            p.then(got => got && !late && ((late = true), changed.forEach(fn => fn())));
            return null;
          });
        });
        const got = await Promise.all(pages);
        const ok = got.filter(Boolean);
        const merged_ = decoded(Array.from(feed.merge_feeds(ok.map(({ f, p }) => [bytes(f.owner), p.rows])))).sort((a, b) => (a.key < b.key ? 1 : a.key > b.key ? -1 : 0));
        const out = merged_.slice(0, limit);
        const more = merged_.length > limit || ok.some(({ p }) => p.next);
        return { rows: out, next: more && out.length ? out.at(-1).key : null };
      };
      // A row's current version (to write the next one over it): held, or — a lazy table — read by its one key.
      const currentId = async key => {
        const held = rows.find(r => r.key === key);
        if (held || !t.lazy) return held?.id;
        return (await page({ lo: key, hi: `${key}\u0000`, limit: 1 })).rows.find(r => r.key === key)?.id;
      };
      const write = async (key, value, user = !!navigator.userActivation?.isActive) => {
        await current();
        // A newer epoch just learned: the open tails move onto it before this is sealed.
        if (sp.kind !== "account") await ctx.background("storage: sealNewest", sealNewest());
        return versioned(me, key, value, await currentId(key), user);
      };
      const t = {
        app: name,
        lazy,
        page,
        // What each writer's feed is here (to see why a table reads as it does).
        feedsNow: () => all.map(f => ({ owner: f.owner.slice(0, 12), absent: !!f.absent, lazy: !!f.lazy, state: f.state?.s ?? null, info: f.info ?? null })),
        // THIS WRITER's own feed read whole (a lazy table: what it copies or checks of its own), the others as paged.
        ownWhole: async () => {
          await me.whole?.();
          remerge();
        },
        // A LAZY table wanted whole: every feed read whole, merged again.
        whole: async () => {
          t.lazy = false;
          wantWhole = true;
          await Promise.all(all.map(f => f.whole?.()));
          remerge();
        },
        // Every writer tried once (some may still arrive later): for what needs the whole table now (adopting a node's
        // rows before its removal).
        settled,
        rows: () => rows,
        // ITS HISTORY of one row, across every writer's feed: each version it had at each root a feed logged —
        // `[{ at, key, value, id }]` (text; `at`: when that tree was replaced), deletions left out.
        history: async key => {
          const k = enc.encode(key);
          // Each writer's feed BOUNDED (its old trees' blocks fetched from the network): one slow or missing never
          // holds the others — what came in time is shown, the rest named.
          const LIMIT = 20000;
          const per = await Promise.all(
            all.filter(f => !f.absent && f.history).map(async f => {
              const out = [];
              let timer;
              const hs = await Promise.race([
                ctx.optional("storage: f.history", f.history(k), () => []),
                new Promise(r => (timer = setTimeout(() => (ctx.log("history", { what: `${name}: ${f.owner.slice(0, 12)}…'s history not read in ${LIMIT / 1000} s` }), r([])), LIMIT))),
              ]).finally(() => clearTimeout(timer));
              for (const h of hs) {
                if (!h.value) continue;
                for (const r of decoded(Array.from(feed.merge_feeds([[bytes(f.owner), [[k, h.value]]]])))) if (r.value) out.push({ at: h.at, ...r });
              }
              return out;
            }),
          );
          return per.flat();
        },
        onChange: f => changed.push(f),
        // R5: its READ STATE — its own feed's, and UNREADABLE (never an empty "present") while a writer's feed did not
        // open here; DECIDED once its own feed is and every listed writer was tried.
        get state() {
          const own = me.state ?? { s: "unknown" };
          if (own.s !== "present" && own.s !== "absent") return own;
          if (!gathered) return { s: "unknown", why: "writers' feeds still being read" };
          // (Every other writer's feed ANSWERED too — not merely opened, shown after WAIT.hint: audit 10-13.)
          if (all.some(f => f !== me && f.state?.s === "unknown")) return { s: "unknown", why: "a writer's feed not answered yet" };
          // (A writer's feed there but SEALED here — a recovery before its keys arrived: never an empty "present", R5.)
          if (all.some(f => f !== me && f.state?.s === "unreadable")) return { s: "unreadable", why: "a writer's feed sealed here" };
          return unopened ? { s: "unreadable", why: `${unopened} writer feed(s) not read here` } : { s: "present" };
        },
        decided: async () => {
          await me.decided?.();
          await within(gathering.then(() => Promise.all(all.map(f => f.decided?.()))), WAIT.answer);
          return t.state;
        },
        // R3: the writers whose feed did not open here, GATHERED AGAIN (keys arrived since).
        retryFailed: () => {
          for (const o of [...failedOwners]) {
            failedOwners.delete(o);
            opened.delete(o);
            unopened = Math.max(0, unopened - 1);
            ctx.background("storage: gather", gather(o));
          }
        },
        // The tails it is built on (their ids, hex): what reopens it when one did not read.
        get built() {
          return all.map(f => hexOf(f.id ?? new Uint8Array()));
        },
        put: (key, value, user) => write(key, value, user),
        // This writer's own feed written out now (`flushNow`): before an edit.
        flushOwn: () => me.flushNow?.(),
        remove: (key, user) => write(key, null, user),
        get sealed() {
          return me.sealed;
        },
        get info() {
          const i = all.map(f => f.info).filter(Boolean);
          return {
            rows: rows.length,
            pending: me.info?.pending ?? 0,
            flushed: i.some(x => x.flushed),
            legacy: i.reduce((n, x) => n + x.legacy, 0),
            unreadable: i.reduce((n, x) => n + x.unreadable, 0),
            writes: me.info?.writes,
            feeds: all.filter(f => !f.absent).length,
            unopened,
          };
        },
      };
      return t;
    })();
    tables.set(at, ready);
    // R4: a failed table is never kept; an open one is known by what it is built on (R3 reopens it with them).
    ctx.optional("storage: ready.then", ready.then(t => tables.get(at) === ready && mergedNow.set(at, t)), () => tables.get(at) === ready && tables.delete(at));
    return ready;
  }

  const openedNames = new Set(); // the account's tables this page asked for
  // A table of the account, or (`sp`) of another space.
  // A SPACE's RETENTION (`roles`, from its `space`/`history` setting): days, at most the plan's.
  const setRetention = (spaceId, days) => (days == null ? retention.delete(spaceId) : retention.set(spaceId, Math.max(0, Math.min(HISTORY.days, days))));
  async function table(name, sp = null, opts = {}) {
    if (sp && sp.kind !== "account") return merged(name, sp, opts);
    if (CHANNELS.has(name)) return channel(name);
    openedNames.add(name);
    const acc = await space.account.strict();
    if (!acc) return guestTable(name, opts);
    return merged(name, acc, opts);
  }
  // An EMPTY table: a GUEST's own (a public page opened by someone not logged in: nothing is theirs), or nobody's
  // (`empty`) — it reads empty, and a write says why it cannot be. Every reader of the account's tables (pins, labels,
  // saves, follows…) and of a person's public ones works for a guest as is.
  function guestTable(name, { lazy = false } = {}) {
    const refuse = async () => {
      throw space.loginNeeded();
    };
    return {
      app: name,
      lazy,
      guest: true,
      page: async () => ({ rows: [], next: null }),
      feedsNow: () => [],
      ownWhole: async () => {},
      whole: async () => {},
      settled: Promise.resolve(),
      rows: () => [],
      history: async () => [],
      onChange: () => {},
      put: refuse,
      flushOwn: async () => {},
      remove: refuse,
      sealed: false,
      // As a public tail too (`publicTail`'s): nothing there, nothing to wait for.
      absent: true,
      answer: async () => {},
      reread: async () => {},
      info: { rows: 0, pending: 0, flushed: false, legacy: 0, unreadable: 0, feeds: 0, unopened: 0 },
    };
  }

  // Every table of the account, as `{ name, rows, pending, flushed, sealed, legacy, unreadable, feeds }` (for the
  // Account page's Storage section): the directory's channels and every writer's catalog. Only the
  // tables this site uses, or this page already opened, are opened: another app's table is listed by its name
  // (`{ name, closed: true }`), never asked for — no prompt from a list.
  async function describe() {
    const sp = await space.account.strict();
    const d = await directory();
    const cats = await Promise.all([sp.self, ...(await writers(sp.self))].map(catalogOf));
    const names = new Set(d.rows().map(r => r.key).filter(n => n !== CATALOG && !n.startsWith("node:")));
    for (const c of cats) for (const r of listedIn(c)) names.add(r.key);
    const out = [];
    for (const name of names) {
      if (CHANNELS.has(name)) {
        const c = await channel(name);
        out.push({ name, sealed: !!c.sealed, ...(c.info ?? { rows: c.rows().length, pending: 0, flushed: false, legacy: 0, unreadable: 0 }), feeds: 1 });
        continue;
      }
      if (!ctx.uses.includes(name) && !openedNames.has(name)) {
        out.push({ name, closed: true });
        continue;
      }
      const t = await ctx.optional("storage: table", table(name), () => null);
      if (t) out.push({ name, sealed: !!t.sealed, ...t.info, address: await addressOf(name) });
    }
    return out;
  }

  // A table's NETWORK ADDRESS (its contract id, hex): this node's own feed of it — what others ask for to read it.
  const hexOf = id => (id ? [...id].map(b => b.toString(16).padStart(2, "0")).join("") : null);
  const addressOf = async name => hexOf((await ownTables()).find(t => t.app === name)?.id);
  // EACH SPACE's tables this node writes — its catalog there, and every table it lists — read as `describe` reads the
  // account's (the Storage page; opened, so "Keep all now" keeps them too).
  async function describeSpaces() {
    const out = [];
    for (const sp of (await space.mine.strict()).filter(s => s.kind === "server")) {
      const scope = scopeOf(sp);
      const cat = await ctx.optional("storage: scope.catalogOf", scope.catalogOf(scope.self), () => null);
      if (!cat) continue;
      await cat.answer?.();
      const tables = [{ name: sp.tables.catalog, sealed: !!cat.sealed, rows: own(cat).length, flushed: true, pending: 0, catalog: true, absent: !!cat.absent, address: hexOf(cat.id) }];
      for (const r of own(cat).filter(x => x.value && placeOf(cat, x.key) !== "none")) {
        const t = await ctx.optional("storage: table", table(r.key, sp), () => null);
        if (t) tables.push({ name: r.key, sealed: !!t.sealed, ...t.info, address: await addressOf(r.key) });
      }
      out.push({ space: { id: sp.id, name: sp.name }, tables });
    }
    return out;
  }

  // A LOG: one shared tail under `owner`'s key (an epoch's, for the account's MLS commits). Read if it is there; made
  // by its first write if not.
  function log(name, owner, { known = null, sealWith = null, space = null } = {}) {
    return tail(owner, name, { known, sealWith, space });
  }

  // A PUBLIC tail under `owner`'s key (a person's card, under their account's data key): anyone reads it. This
  // account's own is listed in its directory before it is made (so it is never asked for while it does not exist).
  // A person's PUBLIC TABLES are listed on their CARD — a catalog (plain rows), read and written as every catalog is.
  // Another person's: one their card does not list is not asked for. This account's: listed before its first write.
  const CARD = "card";
  const PUBLIC = ["mail", "posts", "modlist"];
  const cardOf = async owner => {
    const c = await tail(owner, CARD, { public: true });
    await c.answer();
    c.plainCatalog = true;
    return c;
  };
  // `unlisted`: a public table found only by its NAME (a write credential, `circles`): never noted on the card — no
  // one who is not told its name can find it — and read by name, whatever the card lists.
  async function publicTail(name, owner, { unlisted = false } = {}) {
    const sp = await space.account.strict();
    if (unlisted) return tail(owner, name, { public: true, ...(sp && owner === sp.shared ? { beforeCreate: () => listInDirectory(name) } : {}) });
    if (sp && owner === sp.shared) {
      if (name === CARD) {
        const c = await tail(owner, name, { public: true, known: inDirectory(await directory(), name) === false ? false : null, beforeCreate: () => listInDirectory(name) });
        await c.answer();
        c.plainCatalog = true;
        return c;
      }
      const listed = inDirectory(await directory(), name);
      return tail(owner, name, { public: true, known: listed === false ? false : null, beforeCreate: async () => (await listInDirectory(name), await notePlace(await cardOf(owner), name, "listed")) });
    }
    if (name === CARD) return cardOf(owner);
    const place = placeOf(await cardOf(owner), name);
    return tail(owner, name, { public: true, ...(place === "unlisted" || place === "none" ? { known: false } : {}) });
  }

  // The nodes the directory lists as having feeds (hex keys): where `membership` starts gathering.
  async function nodes() {
    return (await directory()).rows().filter(r => r.key.startsWith("node:")).map(r => r.key.slice(5));
  }

  // The OWN ROWS of table `name` in each of `owners`' feeds (where its catalog lists one): `[{ owner, rows }]`. A
  // union, not a merge — for what must not be overwritten by one writer (a removal stays found in its remover's feed).
  async function feedsOf(name, owners) {
    const out = [];
    await Promise.all(
      owners.map(async owner => {
        const cat = await ctx.optional("storage: catalogOf", catalogOf(owner), () => null);
        if (!cat || !own(cat).some(r => r.key === name)) return;
        const t = await ctx.optional("storage: tail", tail(owner, name, { catalogKey: ANY_GRANT.has(name), catalog: cat }), () => null);
        if (t) out.push({ owner, rows: own(t) });
      }),
    );
    return out;
  }

  // ADOPT a node's current rows (before it is removed): every row whose current version is `node`'s is written again
  // in this node's own feed, replacing it — so when its feed stops counting, nothing it wrote is lost.
  async function adopt(node) {
    const sp = await space.account.strict();
    const d = await directory();
    const cats = await Promise.all([sp.self, node].map(o => ctx.optional("storage: catalogOf", catalogOf(o), () => null)));
    const names = new Set(cats.filter(Boolean).flatMap(c => listedIn(c).map(r => r.key)));
    for (const n of d.rows().map(r => r.key)) if (n !== CATALOG && !n.startsWith("node:")) names.add(n);
    let moved = 0;
    for (const name of names) {
      if (CHANNELS.has(name)) continue;
      const t = await ctx.optional("storage: table", table(name), () => null);
      if (!t) continue;
      await t.settled;
      for (const r of t.rows().filter(r => r.id?.startsWith(node))) {
        await t.put(r.key, r.value);
        moved += 1;
      }
    }
    ctx.log("adopted", { what: `${moved} row(s) of ${node.slice(0, 12)}… now in this node's feeds` });
    return moved;
  }

  // SETTLED SOON: a table is ready once every other writer's feed has answered — or after WAIT.hint (`node`), whichever is
  // first. A feed not there (never written) costs the node a whole network search (seconds to over a minute); nobody
  // waits on that. One that arrives later is merged in as it comes (the table's `onChange`): nothing is dropped.
  const soon = async (all, name) => {
    const LATE = Symbol("late");
    const x = await within(all, WAIT.hint, LATE);
    if (x !== LATE) return x;
    ctx.log("storage", { what: `${name}: shown without the feeds still being asked (merged as they come)` });
  };

  // A PUBLIC TABLE READ FROM OUTSIDE (not a member: no catalog, no keys): the merge of these writers' public tails of
  // `name` (each at the address its writer and name give). Read-only; `add(writers)` takes more writers in.
  function readOnly(name, owners = []) {
    let rows = [];
    const all = [];
    const seen = new Set();
    const changed = [];
    const remerge = () => {
      rows = decoded(Array.from(feed.merge_feeds(all.filter(t => !t.absent).map(t => [bytes(t.owner), t.raw()]))));
      for (const f of changed) f();
      dataChanged(); // (a view reads it again only once it is here)
    };
    // More writers taken in: ready as `settled` is (5 s at most; later ones merged as they come). `caps`: a DEPARTED
    // writer's (node → seq: `writersOf`), its rows counted up to there — as a member's merged table counts them.
    const add = (list, caps = null) =>
      soon(
        Promise.all(
          list
            .filter(o => !seen.has(o) && seen.add(o))
            .map(o =>
              ctx.background("storage: tail", tail(o, name, { public: true }).then(t => {
                  const cap = caps?.get?.(o);
                  all.push(typeof cap === "number" ? { owner: t.owner, get absent() { return t.absent; }, raw: () => (t.raw() ?? []).filter(([, v]) => withinCap(v, cap)) } : t);
                  t.onChange(remerge);
                  remerge();
                })),
            ),
        ),
        name,
      );
    const settled = add(owners);
    return { rows: () => rows, onChange: f => changed.push(f), settled, add };
  }

  // THIS NODE's tables opened on this page (the account's, and its own feeds in spaces): what keeping keeps.
  async function ownTables() {
    const sp = await space.account.strict();
    const mine = new Set([sp?.self, sp?.shared, ...ownKeys].filter(Boolean));
    return [...live.values()].filter(t => mine.has(t.owner) && !t.absent);
  }

  // REPAIR (`keys.repairAccount`, 10-13: an account whose directory a SECOND group sealed over — the owner's, 10-12):
  // THE DIRECTORY made again, from what is known without reading it: this node (its feeds), the card, every public
  // table the card lists (they are found by name: not listed would read as never made), the directory's own row.
  // Upkeep's marks in that row start again (each is a "done already" a page re-checks).
  async function repairDirectory(groupNodes = []) {
    const sp = await space.account.strict();
    const d = await directory();
    const card = await cardOf(sp.shared);
    const at = Date.now();
    const names = new Set([CARD, ...PUBLIC, ...card.rows().filter(r => r.value).map(r => r.key)]);
    // EVERY node of the group (`groupNodes`, from its credentials) and this one: each one's feeds listed.
    const nodes = new Set([sp.self, ...groupNodes]);
    const rows = [[CATALOG, JSON.stringify({ at })], ...[...nodes].map(k => [`node:${k}`, JSON.stringify({ at, b: 1 })]), ...[...names].map(n => [n, JSON.stringify({ at, b: 1 })])];
    await d.replaceWith(rows);
    directoryOpen = null;
    directoryTail = null;
    return rows.map(([k]) => k);
  }
  // …or, where it reads, every node of the group listed in it (each one's feeds found).
  async function listNodes(groupNodes = []) {
    const sp = await space.account.strict();
    const keys = [...new Set([sp.self, ...groupNodes])];
    for (const k of keys) await listInDirectory(`node:${k}`);
    return keys;
  }
  // The directory's read STATE here (`unreadable`: sealed with a key this login does not hold — a forked account's
  // original login; `keys.repairAccount` runs only there).
  async function directoryState() {
    const d = await directory();
    return (await d.decided?.()) ?? d.state ?? { s: "unknown" };
  }
  // …and EVERY table this node writes, opened: rows under an older key sealed over, a tree under another built again,
  // all under the group's CURRENT epoch (a node joining with the words from that epoch reads them). Waited for.
  async function resealOwn() {
    const sp = await space.account.strict();
    const cat = await catalogOf(sp.self);
    await cat.answer?.();
    const names = cat.rows().filter(r => r.value && r.key !== CATALOG && !r.key.startsWith("node:")).map(r => r.key);
    const done = [];
    for (const n of names) {
      const t = await ctx.optional("storage: reseal", table(n), () => null);
      if (!t) continue;
      await t.answer?.();
      done.push(n);
    }
    // Every own tail's queue (its sealing over, its flush) drained.
    for (const t of await ownTables()) await ctx.optional("storage: reseal", t.flushNow(), () => null);
    return done;
  }

  // UPKEEP'S MARKS — what background work this account has done, so a page after does not do it again (the version
  // of the MIGRATIONS done, what each board's public copies were synced as): kept on the account's directory, its
  // `CATALOG` row (`upkeep`: { key: value }). A page has no other store (it runs sandboxed: no localStorage).
  const directoryMark = async () => {
    const d = await directory();
    try {
      return { d, v: JSON.parse(d.rows().find(r => r.key === CATALOG)?.value ?? "{}") ?? {} };
    } catch {
      return { d, v: {} };
    }
  };
  const upkeepMark = async key => (await directoryMark()).v.upkeep?.[key] ?? null;
  const setUpkeepMark = async (key, value) => {
    const { d, v } = await directoryMark();
    if (v.upkeep?.[key] !== value) await d.put(CATALOG, JSON.stringify({ ...v, upkeep: { ...(v.upkeep ?? {}), [key]: value } }));
  };
  return { decided, writersOf, currentWriters, empty: guestTable, problemsIn: id => [...(problems.get(spaceKey(id))?.keys() ?? [])], keyGapsIn: id => [...(problems.get(spaceKey(id))?.keys() ?? [])].filter(w => problemKinds.get(`${spaceKey(id)}|${w}`) === "key").length, own: ownTables, table, setRetention, log, publicTail, readOnly, describe, describeSpaces: ctx.total("storage.describeSpaces", () => [], describeSpaces), nodes, feedsOf, adopt, sealNewest, headsOf, refuse: why => (refusing = why), repairDirectory, listNodes, resealOwn, directoryState, upkeepMark, setUpkeepMark, opens: () => opensLog.map(({ t0, ...x }) => ({ at: Math.round(t0), ...x })) };
}
