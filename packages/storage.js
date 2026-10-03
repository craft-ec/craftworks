// STORAGE, a capability: the logged-in ACCOUNT's tables (notes, pins, …). It composes: the core (a tail under one key:
// read, sealed, written, flushed into its tree), `feed` (a row as a version, and the merge), `membership` (whose feeds
// count), `blocks` (tree blocks) and `access` (the grant and the keys that come with it).
//
// Every node writes its OWN FEED of a table — a tail under its node key, labelled `t/<table>` — and reads the table as
// the MERGE of the feeds of the account's nodes (ARCHITECTURE: every writer its own feed; versions are causal). Tables
// from before feeds, under the account's shared data key, are read as the oldest writer. Any SITE the person allows
// reads and writes the same table (the grant is per site and table; the first time, the node asks them). Every feed is
// followed, so a row written elsewhere arrives here as it lands.
//
// Nothing is asked of the network that is not known to exist (a GET of a tail nobody wrote gets no answer): each
// writer's CATALOG feed lists the tables it has a feed of, listed BEFORE the feed is made; the account's DIRECTORY (its
// shared catalog) lists the nodes that have feeds, and the tables from before feeds.
//
//   const notes = await (await ctx.require("storage")).table("notes");
//   notes.rows()                [{ key, value, id }]
//   await notes.put(key, value)   await notes.remove(key)   notes.onChange(fn)
export async function start(ctx) {
  const auth = await ctx.require("auth");
  // What a space is: its keys and its own tables' names (the account, here).
  const space = await ctx.require("space");
  // Who may read and write a table here: the person's grant for this site, and the table's key that comes with it.
  const access = await ctx.require("access");
  const { core, glue, ask, listen, WAIT, backoff } = await ctx.require("node");
  const tailCode = await ctx.require("tail-wasm");
  // A table's tree lives in Sealed contracts, sealed whole (a tree from before, in Block contracts): the core names
  // them from this code.
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
  async function settle(view, app) {
    for (let round = 0; view.kind === "tail-need" || view.kind === "tail-keys"; round++) {
      if (round >= 24) throw new Error(`${app}: the tree did not finish loading`);
      if (view.kind === "tail-keys") await epochKeys(view.id, view.epochs, app);
      else await blocks.fetch(view.id, view.blocks, app);
      view = JSON.parse(core.tail_view(bytes(view.id)));
    }
    if (view.kind === "tail-unreadable") throw new Error(`${app}: ${view.said}`);
    return view;
  }

  // A PAGE of one tail (phase 3, Reads): `{ lo, hi, after, reverse, limit }` (keys as text) → `{ rows: [[key bytes,
  // stored value bytes]], next }` — the blocks on its path fetched, and only those.
  async function pageOf(idHex, app, { lo = "", hi = "", after = "", reverse = true, limit = 50 } = {}) {
    const b = x => (x ? enc.encode(x) : new Uint8Array(0));
    for (let round = 0; round < 24; round++) {
      const v = JSON.parse(core.tail_page(bytes(idHex), b(lo), b(hi), reverse, b(after), limit));
      if (v.kind === "tail-page") return { rows: v.rows.map(([k, val]) => [bytes(k), bytes(val)]), next: v.next ? dec.decode(bytes(v.next)) : null };
      if (v.kind === "tail-keys") await epochKeys(v.id, v.epochs, app);
      else if (v.kind === "tail-need") await blocks.fetch(v.id, v.blocks, app);
      else throw new Error(`${app}: ${v.said ?? v.kind}`);
    }
    throw new Error(`${app}: a page did not finish loading`);
  }

  // REFUSING: set by `keys` when the group says this node was REMOVED from the account. From then on it writes
  // nothing — not even sealing rows over, which would take them out of reach of the nodes that remain.
  let refusing = null;

  // Tails holding rows sealed with an epoch this node had no key of when read: `id` → { app, epochs }. When a key
  // arrives (`craftworks:keys`: the identity kept an epoch — this node caught up, or joined, or another device of the
  // account moved the group), they are given it and read again: a device added while this page is open is read.
  const unkeyed = new Map();
  async function epochKeys(idHex, epochs, app) {
    const spaceId = live.get(idHex)?.spaceId ?? null;
    for (const e of epochs) {
      const k = await access.keyAt(app, e, { catalog: app === CATALOG, space: spaceId });
      core.tail_epoch_key(bytes(idHex), e, k.key ? bytes(k.key) : new Uint8Array(0), false);
      const u = unkeyed.get(idHex) ?? { app, epochs: new Set() };
      if (k.key) u.epochs.delete(e);
      else {
        u.epochs.add(e);
        ctx.log("table sealed", { what: `${app}: no key here for epoch ${e} (${k.why})` });
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
        const e = await access.keyAt(w.app, -1, { catalog: w.catalog, space: w.space }).catch(() => ({}));
        if (!e.key || e.epoch <= w.epoch) continue;
        core.tail_epoch_key(bytes(idHex), e.epoch, bytes(e.key), true);
        w.epoch = e.epoch;
        ctx.log("table sealing", { what: `${w.app}: writes now with epoch ${e.epoch}` });
      }
    })().finally(() => (resealing = null)));
  addEventListener("craftworks:keys", () => sealNewest());
  let rekeying = null;
  addEventListener("craftworks:keys", () => {
    if (!unkeyed.size || rekeying) return;
    rekeying = (async () => {
      for (const [idHex, u] of [...unkeyed]) {
        await epochKeys(idHex, [...u.epochs], u.app).catch(() => {});
        if (unkeyed.has(idHex)) continue;
        const t = live.get(idHex);
        if (t?.lazy) {
          t.poke();
          continue;
        }
        const v = t && (await settle(JSON.parse(core.tail_view(bytes(idHex))), u.app).catch(() => null));
        if (v?.tail) {
          ctx.log("table opened", { what: `${u.app}: the epoch key arrived` });
          t.took(v.tail);
        }
      }
    })().finally(() => (rekeying = null));
  });

  const tails = new Map(); // id hex -> Promise<tail> (opened once per page)
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
        e => ctx.log("table changed", { what: `${t.app}: ${e.message}` }),
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
  // anyone (public tails): at their names. A table still at its NAME (from before) is MOVED by its writer on its next
  // open — one signed step, its tree kept (the same key addresses it) — and read there by the others until then.
  // Never on a guess: only when the node ANSWERED for both is a table new here; silence keeps it where it was.
  // WHERE IT IS comes from its writer's CATALOG (`catalog`: the one place a table's place is kept — `placeOf` reads
  // it, `notePlace` writes it, both only here): not listed or never made — not looked for, made at its blinded name by
  // its first write; at its blinded name — read there; listed from before — both names read at once (nothing it holds
  // missed), and THIS node's catalog then says which. Never on a guess: "never made" only when the node ANSWERED.
  const opening = new Map(); // `${owner}|${app}` → Promise<the tail>
  function tail(owner, app, opts = {}) {
    const inSpace = opts.space ?? null;
    const byName = opts.public || opts.sealWith || opts.catalogKey || CHANNELS.has(app) || ANY_GRANT.has(app) || (inSpace?.tables && Object.values(inSpace.tables).includes(app));
    // A table found by its NAME: only whether its catalog lists it (not listed or never made: not looked for).
    if (byName) {
      const { catalog: cat = null, ...rest } = opts;
      const place = cat ? placeOf(cat, app) : null;
      const made = { ...rest, beforeCreate: cat && ownKeys.has(owner) ? () => notePlace(cat, app, "blinded") : rest.beforeCreate };
      return tailAt(owner, app, app, place === "unlisted" || place === "none" ? { ...made, known: false } : made);
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
          const sp = await space.account();
          const ours = owner === sp?.self || owner === sp?.shared || ownKeys.has(owner);
          const place = cat ? placeOf(cat, app) : opts.known === false ? "unlisted" : "unknown";
          const note = p => (cat && ours ? notePlace(cat, app, p).catch(e => ctx.log("storage", { what: `${app}: noting its place: ${e?.message ?? e}` })) : Promise.resolve());
          // Its first write here makes it at its blinded name, listed so first.
          const made = { ...rest, beforeCreate: cat && ours ? () => notePlace(cat, app, "blinded") : rest.beforeCreate };
          if (place === "unlisted" || place === "none") return tailAt(owner, app, label, { ...made, known: false });
          if (place === "blinded") {
            const t = await tailAt(owner, app, label, { ...made, wait: ours && cat ? WAIT.answer : WAIT.ask });
            if (t.absent && t.answered()) note("none");
            return t;
          }
          // Listed from before: its blinded name first — there, it is read there and its name never asked. Its NAME is
          // asked only when the blinded name is not there, or silent past a hint (a table not yet moved: nothing it holds
          // missed). Each waited on until the node ANSWERS; its place then noted (its writer's), never asked again.
          const atBlinded = tailAt(owner, app, label, { ...made, wait: WAIT.answer });
          const early = await Promise.race([atBlinded, new Promise(r => setTimeout(r, WAIT.hint, null))]);
          if (early && !early.absent) {
            note("blinded");
            return early;
          }
          ctx.log("storage", { what: `${app}: where it is not noted: ${early ? "not at its blinded name — its name read" : "its blinded name slow — both names read"}` });
          const [blinded, legacy] = await Promise.all([atBlinded, tailAt(owner, app, app, { ...rest, known: null, beforeCreate: null, wait: WAIT.answer })]);
          ctx.log("storage", { what: `${app}: blinded ${blinded.absent ? "absent" : "there"}${blinded.answered() ? "" : " (no answer)"}, by name ${legacy.absent ? "absent" : "there"}${legacy.answered() ? "" : " (no answer)"}` });
          if (!blinded.absent) {
            legacy.moved = true;
            note("blinded");
            return blinded;
          }
          if (legacy.absent && legacy.answered() && blinded.answered()) {
            note("none");
            return blinded;
          }
          if (legacy.absent || !ours) return legacy; // the node silent for one (moved another time), or another's: read there
          ctx.log("storage", { what: `${app}: moving to its blinded name` });
          const moved = await blinded.moveFrom(legacy).catch(e => (ctx.log("storage", { what: `${app}: not moved yet — ${e?.message ?? e}` }), false));
          if (!moved) return legacy;
          legacy.moved = true;
          note("blinded");
          ctx.log("storage", { what: `${app}: moved to its blinded name` });
          return blinded;
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
    if (tails.has(idHex)) return lazy ? tails.get(idHex) : tails.get(idHex).then(t => (t.lazy ? t.whole().then(() => t) : t));
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
      onChange: f => changed.push(f),
      // `value`: bytes, text, or a function giving the bytes at the moment of writing (a version names the feed's
      // next sequence, so it is made in the write's own turn).
      put: (key, value) => (queue = queue.catch(() => {}).then(() => write(key, value))),
      remove: key => (queue = queue.catch(() => {}).then(() => write(key, ""))),
      tailNext: () => core.tail_next(id),
      // Ask again (a tail that was not there: its writer may have made it since).
      reread: () => read(),
      // LAZY (phase 3, Reads): its tree read only by PAGES — the blocks on a page's path, never the whole tree.
      lazy,
      page: o => pageOf(idHex, app, o),
      poke: () => changed.forEach(f => f()),
      // Wanted WHOLE after a lazy open: read whole — unless it is not there yet (a view would mark it made, and its
      // first write would then skip listing it in the catalog: a table nobody finds after a reload).
      whole: async () => {
        t.lazy = false;
        if (!t.absent) await view();
      },
      // KEEP it (phase 4, Lifecycle): see `keep` below.
      keep: () => (queue = queue.catch(() => {}).then(keep)),
      // Whether the node ANSWERED the last read (the tail, or "not found"), not silence.
      answered: () => answered,
      // MOVE here (this tail new, at the table's blinded name) the table from where it was (`from`: open, read): its
      // state as this one's first step. True once moved.
      moveFrom: from =>
        (queue = queue.catch(() => {}).then(async () => {
          const p = core.tail_adopt(id, from.id);
          if (!p) return false;
          await step(p, "moving");
          t.absent = false;
          return true;
        })),
    };
    const ready = (async () => {
      // THE TABLE'S KEY: the table is sealed, so reading it needs its key, and the key comes only with the person's
      // grant for this site (asked NOW: the node prompts the first time). No grant: nothing of the table reads here.
      // The catalog's key comes with any grant: listing a table is part of using it.
      // A PUBLIC tail (a person's card): in the clear, no key.
      if (open) core.tail_public(id);
      const k = open ? {} : sealWith ? { key: sealWith } : await access.key(app, { catalog: catalogKey, space: spaceId });
      t.sealed = !!k.key;
      if (k.key) core.tail_seal(id, bytes(k.key));
      else if (!open) ctx.log("table sealed", { what: `${app}: no key here (${k.why}): nothing of it reads` });
      // WRITES are sealed with the account's newest EPOCH (a node removed from the account then reads nothing written
      // after): the group brought current first. Not a channel (a node reads the MLS channel before it has any epoch),
      // and the catalog not after the group (the group's channel is found through it). No group here: the table's key.
      if (k.key && !sealWith && !CHANNELS.has(app)) {
        const keys = await ctx.require("keys");
        if (inSpace?.id) await keys.group(inSpace).ready().catch(() => null);
        else if (!catalogKey) await keys.ready().catch(() => null);
        const e = await access.keyAt(app, -1, { catalog: catalogKey, space: spaceId });
        if (e.key) core.tail_epoch_key(id, e.epoch, bytes(e.key), true);
        else ctx.log("table sealed", { what: `${app}: no epoch here (${e.why}): written with the table's key` });
        // Kept: when a newer epoch arrives (the group moved: a node removed, one added), writes move onto it.
        t.sealing = { app, catalog: catalogKey, space: spaceId, epoch: e.key ? e.epoch : -1 };
      }
      if (known === false) {
        core.tail_absent(id);
        t.absent = true;
      } else {
        await read();
      }
      // Rows under an older key: sealed over now, a batch per step (in the write queue), where this node signs the tail.
      const sp = await space.account();
      if ((legacy > 0 || reseal) && (owner === sp?.self || owner === sp?.shared)) queue = queue.catch(() => {}).then(sealOld);
      // Rows a page left in this node's own tail (it closed before its quiet flush): flushed once quiet here. A public
      // tail only after a write here (its writes need the person's grant, which a read of it does not show).
      else if (!open && (owner === sp?.self || owner === sp?.shared) && !t.absent && core.tail_pending(id) > 0) whenQuiet();
      return t;
    })();
    tails.set(idHex, ready);
    live.set(idHex, t);

    const allowed = () => (catalogKey || spaceId ? Promise.resolve(true) : access.allowed(app));

    // Read it and follow it. None on the network: its first write is a PUT. No answer at all: the same as none, so
    // it opens empty; a first write the network then refuses resets it and reads it again.
    let answered = false; // the last read had the node's answer (the tail, or "not found"), not silence
    async function read() {
      const [, frames] = core.tail_get(id);
      let said;
      try {
        said = await ask(frames, x => (x.kind === "tail" || x.kind === "tail-need" || x.kind === "tail-keys" || x.kind === "get-failed") && x.id === idHex, `reading ${app}`, wait);
        answered = true;
      } catch (e) {
        ctx.log("table not found yet", { what: `${app}: ${e.message}; opened empty` });
        said = { kind: "get-failed" };
        answered = false;
      }
      // The tail is there: its tree must load too. A tree that does not is an error, never an empty table (that
      // would write over rows it could not see).
      // LAZY: the tail's own state is enough (its pages read the tree when asked).
      if (t.lazy && said.kind !== "get-failed") {
        t.absent = false;
        if (said.kind === "tail") took(said.tail);
        else for (const f of changed) f();
        return;
      }
      // Asked and not there: a network search spent (seconds, on a real network) — named, so what is asked for no
      // reason shows.
      if (said.kind === "get-failed" && answered) ctx.log("asked, not there", { what: `${app} (${label === app ? "by name" : "blinded"}, ${owner.slice(0, 8)}…)` });
      if (said.kind !== "get-failed") said = await settle(said, app);
      t.absent = said.kind !== "tail";
      if (said.kind === "tail") took(said.tail);
      else core.tail_absent(id);
    }

    // One write: prepared by the core, signed by the identity delegate with this tail's key, sent as one delta.
    async function write(key, value) {
      if (!(await allowed())) throw new Error(`this app may not change your “${app}”: allow it when your node asks`);
      if (refusing) throw new Error(refusing);
      // Listed BEFORE it is created: a failure between the two leaves a listed tail that is empty, never one nobody
      // can find.
      if (t.absent && beforeCreate) await beforeCreate();
      // A step the identity refuses as a FORK (it signed this tail further than this page holds): the network read
      // again. Caught up — written again on what it holds. The node ANSWERED and holds less — those signed steps never
      // landed (or were lost): this step goes past them, as a whole state. The node silent — nothing is overwritten
      // on a guess: try again once it answers.
      let past = null;
      for (let attempt = 0; ; attempt++) {
        const v = typeof value === "function" ? value() : typeof value === "string" ? enc.encode(value) : value;
        let p = core.tail_prepare(id, enc.encode(key), v);
        if (past != null) p = core.tail_skip(id, past) ?? p;
        try {
          await step(p, "saving to");
          break;
        } catch (e) {
          if (e.forkedAt == null || attempt >= 1) throw e;
          if (core.tail_next(id) > e.forkedAt) continue;
          if (!answered) throw new Error(`${app}: your node has written this further than it can read back right now — try again in a minute`);
          ctx.log("write", { what: `${app}: the identity signed through step ${e.forkedAt}, the network holds step ${core.tail_next(id) - 1}: written past it` });
          past = e.forkedAt;
        }
      }
      // FLUSH once the tail is long: its rows into the tree, the tail emptied (in this write's turn of the queue).
      if (core.tail_pending(id) >= Core.flush_at()) await flush().catch(e => ctx.log("flush failed", { what: `${app}: ${e.message}` }));
      t.absent = false;
      whenQuiet();
    }
    // FLUSH WHEN QUIET (phase 4, Lifecycle): rows still in the tail once writing has stopped for a while go into the
    // tree — erasure-coded, so they outlive this one tail contract. Each write starts the wait again.
    let quiet = null;
    function whenQuiet() {
      clearTimeout(quiet);
      quiet = setTimeout(() => {
        if (core.tail_pending(id) > 0) queue = queue.catch(() => {}).then(() => core.tail_pending(id) > 0 && flush()).catch(e => ctx.log("flush failed", { what: `${app}: ${e?.message ?? e}` }));
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
      }
      ctx.log(refused ? "write refused" : "written", {
        what: `${app} seq ${p.seq}${refused ? `: ${refused}` : ` as ${kind === "put" ? "a PUT" : "an UPDATE (one delta)"}`}`,
        ms: Math.round(performance.now() - t0),
      });
      if (refused) {
        core.tail_reset(id);
        await read();
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
      legacy = tail.legacy ?? 0;
      reseal = !!tail.resealTree;
      t.info = { rows: tail.rows.length, pending: tail.pending, flushed: !!tail.root, legacy, unreadable: tail.unreadable ?? 0, writes: tail.writes };
      rows = tail.rows;
      for (const f of changed) f();
    }
    async function view() {
      if (t.lazy) return void t.poke();
      const v = await settle(JSON.parse(core.tail_view(id)), app);
      took(v.tail);
    }

    // SEAL OVER the rows under an older key (plaintext from before sealing, the table's key, an earlier epoch): 16
    // rows a step, each sealed with the newest key and its old copy deleted in that same step, signed like any write.
    // A tree from before sealing whole is built again, sealed, by a flush.
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
      if (reseal || core.tail_pending(id) >= Core.flush_at()) await flush();
    }

    // KEEP (phase 4, Lifecycle): the table's ASSET — every block its current tree reaches, in its groups — each block
    // ASKED (its health: a group is WHOLE with all its blocks there, DEGRADED with at least its k, DAMAGED below), then
    // PUT again (re-published where it is there; repaired — made from what is held, parity coded again — where it is
    // not), then the tail's signed state put again. In the write queue: no flush moves the tree under it.
    async function keep() {
      if (t.absent) return null;
      const t0 = performance.now();
      if (t.lazy) await t.whole();
      let a = null;
      for (let round = 0; round < 64; round++) {
        a = core.tail_asset(id);
        if (a.need) await settle({ kind: "tail-need", id: idHex, blocks: a.need }, app);
        else if (a.keys) await settle({ kind: "tail-keys", id: idHex, epochs: a.keys }, app);
        else break;
      }
      if (!a?.groups) throw new Error(`${app}: its tree did not finish loading`);
      const out = { name: app, at: Date.now(), groups: a.groups.length, blocks: 0, whole: 0, degraded: 0, damaged: 0, missing: 0, put: 0, unmade: 0 };
      for (const g of a.groups) {
        const there = await Promise.all(g.slots.map(([, c]) => blocks.probe(c, `keeping ${app}`)));
        const present = there.filter(Boolean).length;
        const parity = g.slots.length - g.k;
        out.blocks += g.slots.length;
        out.missing += g.slots.length - present;
        out[present - g.k >= parity ? "whole" : present >= g.k ? "degraded" : "damaged"] += 1;
        const puts = [];
        for (const [b] of g.slots) {
          const p = core.tail_keep_put(id, b);
          if (p) puts.push(p);
          else out.unmade += 1;
        }
        await blocks.put(puts, `keeping ${app}`);
        out.put += puts.length;
      }
      const said = await ask(core.tail_keep_state(id), x => (x.kind === "put" && x.key === name) || x.kind === "refused", `keeping ${app}`, 60000);
      if (said.kind === "refused") throw new Error(`${app}: its state was refused: ${said.said}`);
      out.ms = Math.round(performance.now() - t0);
      ctx.log("kept", { what: `${app}: ${out.blocks} block(s) in ${out.groups} group(s) — ${out.whole} whole, ${out.degraded} degraded, ${out.damaged} damaged; ${out.put} put again${out.unmade ? `, ${out.unmade} not made here` : ""}`, ms: out.ms });
      return out;
    }

    // FLUSH: the tree's new blocks PUT first (a tail must never name a root whose blocks are not there), then the step
    // naming the new root, signed like any write.
    async function flush() {
      const t0 = performance.now();
      for (let round = 0; round < 16; round++) {
        const f = core.tail_flush(id);
        if (f.need || f.keys) {
          await settle(f.need ? { kind: "tail-need", id: idHex, blocks: f.need } : { kind: "tail-keys", id: idHex, epochs: f.keys }, app);
          continue;
        }
        try {
          await blocks.put(f.puts, app);
        } catch (e) {
          core.tail_reset(id);
          await read();
          throw e;
        }
        await step(f, "flushing");
        ctx.log("flushed", { what: `${app}: ${f.puts.length} tree block(s) put, the tail emptied (seq ${f.seq})`, ms: Math.round(performance.now() - t0) });
        return;
      }
      throw new Error("the tree did not finish loading");
    }

    return ready;
  }

  // A stored version's sequence in its feed (an envelope: magic ‖ writer ‖ seq ‖ …); a row from before feeds: none.
  const seqOf = v => (v?.length >= 42 && v[0] === 0xcf && v[1] === 0x01 ? Number(new DataView(v.buffer, v.byteOffset + 34, 8).getBigUint64(0)) : Infinity);
  // A feed as it stood at sequence `cap`: its versions up to there.
  const capped = (f, cap) => ({
    owner: f.owner,
    raw: () => (f.raw() ?? []).filter(([, v]) => seqOf(v) <= cap),
    page: async o => {
      const p = await f.page(o);
      return { ...p, rows: p.rows.filter(([, v]) => seqOf(v) <= cap) };
    },
    whole: () => f.whole?.(),
    onChange: fn => f.onChange(fn),
    get absent() {
      return f.absent;
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
        const cat = await scope.catalogOf(node).catch(() => null);
        if (!cat || cat.absent) return;
        const heads = {};
        await Promise.all(
          listedIn(cat).map(async r => {
            const t = await tail(node, r.key, { ...scope.opts(r.key), catalog: cat }).catch(() => null);
            if (!t || t.absent) return;
            const seqs = (t.raw() ?? []).map(([, v]) => seqOf(v)).filter(Number.isFinite);
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
  // A table's PLACE as its writer's catalog lists it — the one reader of it: "unlisted"; "none" (listed, never made:
  // the node answered "not there" under both names); "blinded" (at its blinded name); "unknown" (listed from before).
  // (The directory — the account's catalog of tables from before feeds and of channels — holds plain rows; a node's
  // catalog holds versions: the same place either way.)
  // ONE CATALOG MODEL. A catalog is a writer's list of its tables: a node's (versions — COMPLETE: a table it does not
  // list it has none of), or the account's DIRECTORY and a person's CARD (plain rows — complete once marked so: the
  // row `CATALOG` → `{ complete: true }`; before, a table it does not list is unknown and read).
  const plain = cat => cat === directoryTail || cat?.plainCatalog === true;
  const complete = cat => {
    if (!plain(cat)) return true;
    try {
      return JSON.parse(cat.rows().find(r => r.key === CATALOG)?.value ?? "{}").complete === true;
    } catch {
      return false;
    }
  };
  // A table's PLACE, the one reader of it: "unlisted" (a complete catalog does not list it: none), "unknown" (an
  // incomplete one does not), "none" (listed, never made), "blinded" (at its blinded name), "listed" (from before its
  // place was noted — a table found by name: there).
  const placeOf = (cat, n) => {
    const r = (plain(cat) ? cat.rows() : own(cat)).find(x => x.key === n && x.value);
    if (!r) return complete(cat) ? "unlisted" : "unknown";
    try {
      const v = JSON.parse(r.value);
      return v?.n === 1 ? "none" : v?.b === 1 ? "blinded" : "listed";
    } catch {
      return "listed";
    }
  };
  // NOTE a table's place in a catalog this account writes, the one writer of it: "blinded", "none" or "listed".
  const notePlace = (cat, n, p) => {
    if (placeOf(cat, n) === p) return Promise.resolve();
    const v = JSON.stringify({ at: Date.now(), ...(p === "listed" ? {} : { b: 1 }), ...(p === "none" ? { n: 1 } : {}) });
    return plain(cat) ? cat.put(n, v) : versioned(cat, n, v, own(cat).find(r => r.key === n)?.id);
  };
  // A write to a feed, as a VERSION replacing `after` (the id of the version current where it is written).
  const versioned = (t, key, value, after) =>
    t.put(key, () => feed.version(bytes(t.owner), t.tailNext(), after ?? "", value === null ? undefined : enc.encode(value)));

  // THE DIRECTORY: the account's shared catalog (its data key). Its rows: the tables from before feeds (and whether
  // it was made COMPLETE, with the account), and `node:<key>` for every node that has feeds. A new account's is made at
  // once; an account from before the catalog has none, so its old tables are read as before.
  let directoryOpen = null;
  let directoryTail = null; // the directory, once open: a catalog of plain rows (a node's catalog: versions)
  function directory() {
    return (directoryOpen ??= (async () => {
      const sp = await space.account();
      const d = await tail(sp.shared, CATALOG, { known: sp.fresh ? false : null, catalogKey: true });
      if (d.absent && sp.fresh) await d.put(CATALOG, JSON.stringify({ at: Date.now(), complete: true }));
      directoryTail = d;
      return d;
    })());
  }
  // A table from before feeds: listed (true), not listed in a COMPLETE directory (false), or unknown (null: read).
  const legacyListed = (d, name) => (["none", "unlisted"].includes(placeOf(d, name)) ? false : null);
  async function listInDirectory(key) {
    const d = await directory();
    if (d.rows().some(r => r.key === key)) return;
    if (d.absent && !d.rows().length) await d.put(CATALOG, JSON.stringify({ at: Date.now(), complete: false }));
    await d.put(key, JSON.stringify({ at: Date.now(), b: 1 })); // made new: at its blinded name
  }

  // A WRITER's CATALOG feed: the tables it has a feed of. Known to exist once the directory lists its node.
  async function catalogOf(writer) {
    const d = await directory();
    const listed = d.rows().some(r => r.key === `node:${writer}`);
    return tail(writer, CATALOG, { known: listed ? null : false, catalogKey: true, beforeCreate: () => listInDirectory(`node:${writer}`) });
  }
  // The other writers whose feeds count: the nodes the directory lists, those the account's group holds (where it is
  // kept; elsewhere every listed node, until membership is gossiped in the feeds).
  async function writers(me) {
    const d = await directory();
    const listed = d.rows().filter(r => r.key.startsWith("node:")).map(r => r.key.slice(5));
    const members = await (await ctx.require("membership")).writers().catch(() => null);
    return listed.filter(k => k !== me && (!members || members.includes(k)));
  }

  // A CHANNEL: one shared tail, listed in the directory before it is made.
  async function channel(name) {
    const sp = await space.account();
    const d = await directory();
    const listed = legacyListed(d, name);
    return tail(sp.shared, name, { known: listed === false ? false : null, beforeCreate: () => listInDirectory(name) });
  }
  // A SCOPE: a space seen from this node — who writes in it, where each writer lists its tables, how its tails open.
  // The account: the directory names its nodes, their catalogs are account tables, and its tables from before feeds
  // are the oldest writer. Another space: its GROUP's members write, each listing the space's tables in its own catalog
  // feed of the space; every tail of it is the space's (sealed with its epoch keys, signed in it).
  function scopeOf(sp) {
    if (sp.kind === "account")
      return {
        key: "account",
        self: sp.self,
        opts: name => ({ catalogKey: ANY_GRANT.has(name) }),
        old: async name => {
          const d = await directory();
          const o = legacyListed(d, name);
          return o === false ? null : { owner: sp.shared, known: o, catalog: d };
        },
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
      old: async () => null,
      // Its WRITERS BAG: who has a catalog here (`index`'s sealed bag of the space). Its writers are read from it.
      bag: fresh => writersBag(sp, fresh),
      list: (ws, complete) => listWriters(sp, ws, complete),
      // Its WRITERS: every device of every member (the group's members are DIDs; each DID's devices, from its card,
      // checked against its key log) — this device's own siblings too.
      writers: async () => {
        const [keys, directory] = await Promise.all(["keys", "directory"].map(n => ctx.require(n)));
        const members = (await keys.group(sp).ready())?.members ?? [];
        const dids = [...new Set(members.filter(m => m.cred).map(m => glue.did_of(new Uint8Array(m.cred.match(/../g).slice(4, 36).map(x => parseInt(x, 16))))))];
        const all = (await Promise.all(dids.map(d => directory.devices(d)))).flat();
        return [...new Set(all)].filter(k => k !== sp.self);
      },
      // New writers: a member's devices changed (their card), or the members did (the group moved).
      watch: fn => {
        addEventListener("craftworks:keys", fn);
        (async () => {
          const [keys, directory] = await Promise.all(["keys", "directory"].map(n => ctx.require(n)));
          const members = (await keys.group(sp).ready())?.members ?? [];
          const watched = new Set();
          const each = () => {
            for (const m of members.filter(m => m.cred)) {
              const d = glue.did_of(new Uint8Array(m.cred.match(/../g).slice(4, 36).map(x => parseInt(x, 16))));
              if (!watched.has(d)) watched.add(d), directory.onDevices(d, fn);
            }
          };
          each();
          addEventListener("craftworks:keys", async () => {
            members.splice(0, members.length, ...((await keys.group(sp).ready().catch(() => null))?.members ?? []));
            each();
          });
        })().catch(() => {});
      },
      // This device's catalog listed in the space's writers bag BEFORE it is made: nobody asks for it before then.
      catalogOf: w =>
        tail(w, sp.tables.catalog, {
          space: sp,
          ...(w === sp.self ? { beforeCreate: () => listWriters(sp, [w]).catch(e => ctx.log("storage", { what: `${sp.name ?? sp.id}: listing this device as a writer: ${e?.message ?? e}` })) } : {}),
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
  const polled = new Set();
  function absentCatalog(c) {
    if (polled.has(c)) return;
    polled.add(c);
    backoff(async () => {
      await c.reread().catch(() => {});
      return !c.absent && polled.delete(c);
    });
  }

  // A TABLE: the merge of its writers' feeds, and this node's feed to write.
  // A SPACE's WRITERS BAG (`index`'s sealed bag): the members' devices that have a catalog there, and whether it is
  // COMPLETE (every catalog from before the bag listed — whoever first opened the space with it listed them all).
  // Read once per space (again when asked): nobody's catalog is asked for before they have one.
  const bags = new Map(); // space id → Promise<{ set, complete }>
  function writersBag(sp, fresh = false) {
    if (fresh || !bags.has(sp.id))
      bags.set(
        sp.id,
        (async () => {
          const t0 = performance.now();
          const items = await (await ctx.require("index")).spacePointers(sp, "writers");
          const b = { set: new Set(items.map(i => i?.w).filter(w => typeof w === "string")), complete: items.some(i => i?.complete === true) };
          ctx.log("storage", { what: `${sp.name ?? sp.id.slice(0, 8)}: writers bag — ${b.set.size} writer(s)${b.complete ? ", complete" : ", not complete yet"}`, ms: Math.round(performance.now() - t0) });
          return b;
        })().catch(e => (bags.delete(sp.id), Promise.reject(e))),
      );
    return bags.get(sp.id);
  }
  async function listWriters(sp, ws, complete = false) {
    const index = await ctx.require("index");
    const b = await writersBag(sp).catch(() => ({ set: new Set(), complete: false }));
    for (const w of ws)
      if (!b.set.has(w)) {
        await index.spacePoint(sp, "writers", { w });
        b.set.add(w);
      }
    if (complete && !b.complete) {
      await index.spacePoint(sp, "writers", { complete: true });
      b.complete = true;
    }
  }

  // A complete bag READ AGAIN now and then (5 s, doubling, to every 5 min): each read's set handed to every table of the
  // space that waits on new writers — one read for the whole space, however many tables.
  const bagPolls = new Map(); // space id → [fn]
  function pollBag(sp, scope, fn) {
    if (bagPolls.has(sp.id)) return void bagPolls.get(sp.id).push(fn);
    const fns = [fn];
    bagPolls.set(sp.id, fns);
    backoff(async () => {
      const b = await scope.bag(true).catch(() => null);
      if (b) for (const f of fns) f(b.set);
      return false;
    });
  }

  const tables = new Map(); // scope + name -> Promise<table>
  function merged(name, sp, { lazy = false } = {}) {
    const scope = scopeOf(sp);
    const at = `${scope.key}/${name}`;
    // Opened already: as it is — or, opened LAZY and now wanted whole, every feed read whole.
    if (tables.has(at)) return lazy ? tables.get(at) : tables.get(at).then(t => (t.lazy ? t.whole().then(() => t) : t));
    const ready = (async () => {
      if (sp.kind === "account" && !sp.shared) throw new Error("this node does not hold the account's data key: log in once with the recovery words");
      const [mine, others] = await Promise.all([scope.catalogOf(scope.self), scope.writers()]);
      // A space: its WRITERS BAG says whose catalogs exist — only those are read once it is complete. This device
      // listed if it has a catalog and is not yet (a catalog from before the bag).
      // Waited on BRIEFLY: a bag this node holds answers at once; one not answered by then — every member read, as
      // before (nothing missed).
      const bag = scope.bag ? await Promise.race([scope.bag().catch(() => null), new Promise(r => setTimeout(() => r(null), WAIT.hint))]) : null;
      const listed = o => !bag?.complete || bag.set.has(o);
      if (bag && !mine.absent && !bag.set.has(scope.self)) scope.list([scope.self]).catch(() => {});
      const lists = (cat, n) => own(cat).some(r => r.key === n);
      const all = [];
      // Another writer's feed that does not open here (sealed under a key this node lacks — a node that has not
      // recovered its epochs) is left out and COUNTED, never fatal: this node writes only its own feed, so nothing it
      // cannot see is written over. Its own feed must open.
      let unopened = 0;
      const opts = scope.opts(name);
      const theirs = (owner, known, cat = null) =>
        tail(owner, name, { known, ...(cat ? { catalog: cat } : {}), ...opts, lazy }).catch(e => {
          unopened += 1;
          ctx.log("feed not read", { what: `${name}: ${owner.slice(0, 12)}…: ${e.message}` });
          return null;
        });
      let rows = [];
      const changed = [];
      const remerge = () => {
        rows = decoded(Array.from(feed.merge_feeds(all.map(f => [bytes(f.owner), f.raw()]))));
        for (const f of changed) f();
      };
      const take = f => {
        if (!f || all.includes(f)) return;
        all.push(f);
        f.onChange(remerge);
        remerge();
      };
      // The table from before feeds: the oldest writer.
      const old = await scope.old(name);
      if (old) take(await theirs(old.owner, old.known, old.catalog));
      // This node's: listed in its catalog BEFORE it is made (`tail`, by its catalog). The table opens once it is open.
      ownKeys.add(scope.self);
      const me = await tail(scope.self, name, { ...opts, lazy, catalog: mine });
      take(me);
      // The OTHER writers' feeds, in the background: each merged in as it arrives, never holding the table up. A writer
      // whose catalog is not there yet (a node that has not written here), or does not list this table yet, is asked
      // again — its catalog is followed once it exists, and a missing one is asked every 30 s — so what it writes later
      // arrives without a reload.
      const opened = new Set();
      const gather = async o => {
        const c = await scope.catalogOf(o).catch(() => null);
        if (!c) return;
        const open = async () => {
          if (opened.has(o) || !lists(c, name)) return;
          opened.add(o);
          take(await theirs(o, true, c));
        };
        c.onChange(() => open().catch(() => {}));
        if (c.absent) {
          if (!c.answered()) silent += 1;
          if (!bag?.complete) absentCatalog(c);
          return;
        }
        found.add(o);
        await open();
      };
      const found = new Set();
      let silent = 0;
      // DEPARTED writers' feeds, capped where they stood when they left (a space's).
      const departedDone = new Set();
      const gatherDeparted = async () => {
        if (!scope.departed?.(name)) return;
        const d = await scope.departedTable();
        await d.settled;
        const current = new Set(await scope.writers().catch(() => others));
        for (const r of d.rows()) {
          if (!r.value || current.has(r.key) || r.key === scope.self || departedDone.has(r.key)) continue;
          let cap;
          try {
            cap = JSON.parse(r.value)?.heads?.[name];
          } catch {}
          if (typeof cap !== "number") continue;
          departedDone.add(r.key);
          // Its catalog: read, and listed in the writers bag when there (a departed writer from before the bag).
          const cat = await scope.catalogOf(r.key).catch(() => null);
          if (cat && !cat.absent && bag && !bag.set.has(r.key)) scope.list([r.key]).catch(() => {});
          const f = await theirs(r.key, true, cat);
          if (f) take(capped(f, cap));
        }
      };
      const gathering = Promise.all([...others.filter(listed).map(o => gather(o).catch(() => {})), gatherDeparted().catch(e => ctx.log("feed not read", { what: `${name}: departed writers: ${e.message}` }))]);
      const settled = soon(gathering, name);
      // The bag from before it was complete: every catalog found listed, and the bag marked COMPLETE — once every
      // member's catalog was ANSWERED (found, or "not there"); a silent one leaves it for a later open.
      if (bag && !bag.complete)
        gathering.then(() => (silent ? null : scope.list([...found, ...(mine.absent ? [] : [scope.self])], true))).catch(e => ctx.log("storage", { what: `${name}: listing the space's writers: ${e?.message ?? e}` }));
      // A complete bag: new writers appear in it — read again now and then (one read for the whole space).
      if (bag?.complete) pollBag(sp, scope, set => set.forEach(o => o !== scope.self && others.includes(o) && !seen.has(o) && (seen.add(o), gather(o).catch(() => {}))));
      if (scope.departed?.(name)) scope.departedTable().then(d => d.onChange(() => gatherDeparted().catch(() => {})), () => {});
      // A WRITER NEW since the table opened (a member's new device, a new member): gathered when the scope says so.
      const seen = new Set(others.filter(listed));
      scope.watch?.(async () => {
        const b = scope.bag ? await scope.bag(true).catch(() => bag) : null;
        for (const o of await scope.writers().catch(() => [])) if (!seen.has(o) && (!b?.complete || b.set.has(o))) seen.add(o), gather(o).catch(() => {});
      });
      ctx.log("table open", { what: `${name}: ${rows.length} row(s) from ${all.filter(f => !f.absent).length} feed(s)` });
      // A write in a space: its group brought current first, so what is written is sealed with the newest epoch's key
      // (never one a member removed since still holds).
      const current = sp.kind === "account" ? async () => {} : async () => (await ctx.require("keys")).group(sp).ready({ fresh: true }).catch(() => null);
      // A PAGE across the writers (phase 3, Reads): each feed's page, their versions merged, the newest `limit` rows
      // kept — each feed has at most `limit` keys at or past the page's last, so none is skipped — and where to go on.
      // `{ lo, hi, before, limit }` (keys as text; `before`: continue below that key).
      const page = async ({ lo = "", hi = "", before = "", limit = 50 } = {}) => {
        const got = await Promise.all(all.filter(f => !f.absent && f.page).map(f => f.page({ lo, hi, after: before, reverse: true, limit }).then(p => ({ f, p }), () => null)));
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
      const write = async (key, value) => {
        await current();
        // A newer epoch just learned: the open tails move onto it before this is sealed.
        if (sp.kind !== "account") await sealNewest().catch(() => {});
        return versioned(me, key, value, await currentId(key));
      };
      const t = {
        app: name,
        lazy,
        page,
        // What each writer's feed is here (to see why a table reads as it does).
        feedsNow: () => all.map(f => ({ owner: f.owner.slice(0, 12), absent: !!f.absent, lazy: !!f.lazy, info: f.info ?? null })),
        // THIS WRITER's own feed read whole (a lazy table: what it copies or checks of its own), the others as paged.
        ownWhole: async () => {
          await me.whole?.();
          remerge();
        },
        // A LAZY table wanted whole: every feed read whole, merged again.
        whole: async () => {
          t.lazy = false;
          await Promise.all(all.map(f => f.whole?.()));
          remerge();
        },
        // Every writer tried once (some may still arrive later): for what needs the whole table now (adopting a node's
        // rows before its removal).
        settled,
        rows: () => rows,
        onChange: f => changed.push(f),
        put: (key, value) => write(key, value),
        remove: key => write(key, null),
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
    return ready;
  }

  const openedNames = new Set(); // the account's tables this page asked for
  // A table of the account, or (`sp`) of another space.
  async function table(name, sp = null, opts = {}) {
    if (sp && sp.kind !== "account") return merged(name, sp, opts);
    if (CHANNELS.has(name)) return channel(name);
    openedNames.add(name);
    const acc = await space.account();
    if (!acc) throw new Error("nobody is logged in");
    return merged(name, acc, opts);
  }

  // Every table of the account, as `{ name, rows, pending, flushed, sealed, legacy, unreadable, feeds }` (for the
  // Account page's Storage section): the directory's tables from before feeds and every writer's catalog. Only the
  // tables this site uses, or this page already opened, are opened: another app's table is listed by its name
  // (`{ name, closed: true }`), never asked for — no prompt from a list.
  async function describe() {
    const sp = await space.account();
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
      const t = await table(name).catch(() => null);
      if (t) out.push({ name, sealed: !!t.sealed, ...t.info });
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
  // Another person's: one their card does not list is not asked for (a card not yet complete: read). This account's:
  // listed before its first write; its card made COMPLETE once each public table it may have is noted (`PUBLIC`).
  const CARD = "card";
  const PUBLIC = ["mail", "posts", "modlist"];
  const cardOf = async owner => {
    const c = await tail(owner, CARD, { public: true });
    c.plainCatalog = true;
    return c;
  };
  let carding = null;
  async function completeCard(owner) {
    const c = await cardOf(owner);
    if (complete(c)) return;
    for (const n of PUBLIC) {
      const t = await tail(owner, n, { public: true, wait: WAIT.answer });
      if (!t.absent) await notePlace(c, n, "listed");
      else if (!t.answered()) return; // silence: noted another time
    }
    await c.put(CATALOG, JSON.stringify({ at: Date.now(), complete: true }));
  }
  async function publicTail(name, owner) {
    const sp = await space.account();
    if (sp && owner === sp.shared) {
      if (name === CARD) {
        const c = await tail(owner, name, { public: true, known: legacyListed(await directory(), name) === false ? false : null, beforeCreate: () => listInDirectory(name) });
        c.plainCatalog = true;
        carding ??= completeCard(owner).catch(e => ctx.log("storage", { what: `listing this account's public tables on its card: ${e?.message ?? e}` }));
        return c;
      }
      const listed = legacyListed(await directory(), name);
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
        const cat = await catalogOf(owner).catch(() => null);
        if (!cat || !own(cat).some(r => r.key === name)) return;
        const t = await tail(owner, name, { catalogKey: ANY_GRANT.has(name), catalog: cat }).catch(() => null);
        if (t) out.push({ owner, rows: own(t) });
      }),
    );
    return out;
  }

  // ADOPT a node's current rows (before it is removed): every row whose current version is `node`'s is written again
  // in this node's own feed, replacing it — so when its feed stops counting, nothing it wrote is lost.
  async function adopt(node) {
    const sp = await space.account();
    const d = await directory();
    const cats = await Promise.all([sp.self, node].map(o => catalogOf(o).catch(() => null)));
    const names = new Set(cats.filter(Boolean).flatMap(c => listedIn(c).map(r => r.key)));
    for (const n of d.rows().map(r => r.key)) if (n !== CATALOG && !n.startsWith("node:")) names.add(n);
    let moved = 0;
    for (const name of names) {
      if (CHANNELS.has(name)) continue;
      const t = await table(name).catch(() => null);
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

  // SETTLED SOON: a table is ready once every other writer's feed has answered — or after WAIT.show (`node`), whichever is
  // first. A feed not there (never written) costs the node a whole network search (seconds to over a minute); nobody
  // waits on that. One that arrives later is merged in as it comes (the table's `onChange`): nothing is dropped.
  const soon = (all, name) => {
    let done = false;
    all.finally(() => (done = true));
    return Promise.race([
      all,
      new Promise(r => setTimeout(r, WAIT.show)).then(() => {
        if (!done) ctx.log("storage", { what: `${name}: shown without the feeds still being asked (merged as they come)` });
      }),
    ]);
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
    };
    // More writers taken in: ready as `settled` is (5 s at most; later ones merged as they come).
    const add = list =>
      soon(
        Promise.all(
          list
            .filter(o => !seen.has(o) && seen.add(o))
            .map(o =>
              tail(o, name, { public: true }).then(
                t => {
                  all.push(t);
                  t.onChange(remerge);
                  remerge();
                },
                () => {},
              ),
            ),
        ),
        name,
      );
    const settled = add(owners);
    return { rows: () => rows, onChange: f => changed.push(f), settled, add };
  }

  // THIS NODE's tables opened on this page (the account's, and its own feeds in spaces): what keeping keeps.
  async function ownTables() {
    const sp = await space.account();
    const mine = new Set([sp?.self, sp?.shared, ...ownKeys].filter(Boolean));
    return [...live.values()].filter(t => mine.has(t.owner) && !t.absent && !t.moved);
  }

  return { own: ownTables, table, log, publicTail, readOnly, describe, nodes, feedsOf, adopt, sealNewest, headsOf, refuse: why => (refusing = why) };
}
