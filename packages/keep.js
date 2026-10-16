// KEEP, a service (started by the header, on every page, after the page is up): the LIFECYCLE of this person's data
// (phase 4) — what keeps it on the network with no server. A table's ASSET is every block its current tree reaches;
// KEEPING it asks each block (its HEALTH: per group, WHOLE / DEGRADED / DAMAGED), puts again only those that do not
// answer (rebuilt from their group) and puts the tail's signed state again (`storage` tables' `keep`).
// DUE, with no job list: a table this node writes is due when its record in the account's table `keep` is missing or
// older than a day (a file: a week). FILES too (`files.keep`): every coded file of the account and of each space this person is in —
// each piece asked (HEALTH per generation), a generation not whole rebuilt and its missing fragments made again.
// One table or file at a time, in the background, while any page of the account is open — and the Storage page keeps them all at once on a click.
// WATCHED (rewards step 2, docs/REWARDS.md §4: the users are the keepers): the files this person PLAYED — anyone's —
// kept on this node by RE-READING them (Freenet evicts what was read least recently: measured 2026-10-07, a piece
// re-read survives a flood that evicts its control), a day apart, newest used first, within a LIMIT the person sets
// (Usage app; oldest dropped past it). Each pass is `files.keep` READ ONLY (owner 10-07: a keeper only keeps; healing
// is every reader's, during its read): a piece the node no longer holds is fetched back by the read itself.
//
//   const keep = await ctx.require("keep");
//   await keep.status()      // tables: [{ name, at, groups, blocks, whole, degraded, damaged, missing, put, unmade, ms }]
//                            // and files: [{ file: id, space, at, pieces, gens, whole, degraded, damaged, missing, put }]
//   await keep.now()         // every table kept now; their records
//   keep.watched(fileRef, itemRef, { title })   // a file this person played: kept for others (rewards step 2)
//   await keep.kept()        // { limit, files: [{ root, item, title, size, used, at, whole, missing, error }] }
//   await keep.setLimit(bytes)   // how much THIS NODE keeps (0: none) — each node its own
//   await keep.limits(keys)      // { key: bytes } for the account's nodes
//   await keep.keepers(root) // [did, …]: who keeps a file (this month or last) — the KEEPERS BAG (rewards step 3)
//   await keep.holdings(epoch) // [{ sp, row, watched } | { assigned: { key, size, missing } }]: what it keeps, fixed for
//                            // standing epoch `epoch` (the keeper's)
//   await keep.assign(epoch, candidates) // the registry files this keeper ranks among the keepers of ([{ key, size }], ranked)
//   await keep.assigned()    // what it keeps of them ([{ key, size }]): found first, then unread, within its limit — never absent
//
// THE KEEPERS BAG (rewards step 3): each file's public bag `keepers:<root>` (`index.point`), where a keeper says, once
// a month, that it keeps the file (`{ did, m: "YYYY-MM" }`; the same claim again merges). A CLAIM, unsigned like every
// pointer and public like a view count (owner 10-06: confidential later): it shows who keeps what; the proofs (step 4)
// are what will make a claim count for pay. Its OWN files' and its spaces' files' holders claim too, once a pass finds
// them whole (the uploader's node holds the original: the first keeper of anything).
export async function start(ctx) {
  const [storage, space, files] = await Promise.all(["storage", "space", "files"].map(n => ctx.require(n)));
  const EVERY = 7 * 86400000; // a FILE kept longer ago than this is due
  // A TABLE: a DAY (owner 10-07, measured: a block of `spacekeys` put 10-06 08:03 was evicted from the owner's node at
  // 19:30, its table last kept 10-04 — a week between keeps let it go unnoticed; a keep rebuilds a missing block from
  // its group and puts it back while the group is still there).
  const TABLE_EVERY = 86400000;
  const RETRY = 3600000; // one that failed: due again after this
  const TRIES = 3; // failures in a row before it is called lost
  // A failure's record: how many in a row (`tries`), and LOST once there are TRIES.
  const failed = async (key, rec) => {
    const was = read((await records()).rows().find(r => r.key === key) ?? { value: "null" });
    const tries = (was?.error ? (was.tries ?? 1) : 0) + 1;
    return { ...rec, tries, ...(tries >= TRIES ? { lost: true } : {}) };
  };
  const records = async () => {
    const t = await storage.table("keep");
    await t.settled;
    return t;
  };
  const dec = new TextDecoder();
  const read = r => {
    try {
      return JSON.parse(typeof r.value === "string" ? r.value : dec.decode(r.value));
    } catch {
      return null;
    }
  };

  async function status() {
    return (await records()).rows().map(read).filter(Boolean);
  }
  // Its record, by the table's contract id (a space's feed and the account's table may share a name).
  const idOf = t => [...t.id].map(b => b.toString(16).padStart(2, "0")).join("");
  // ONE table kept — within LIMIT: a table whose blocks the network lost can take any time to load or ask; past it, it
  // is said so and left due (tried again later), never holding up the tables after it.
  const LIMIT = 120000;
  async function keepOne(t) {
    let timer;
    const out = await Promise.race([
      t.keep(),
      new Promise((_, no) => (timer = setTimeout(() => no(new Error(`${t.app}: not kept in ${LIMIT / 60000} min (its blocks may be missing): tried again later`)), LIMIT))),
    ])
      .catch(async e => {
        // Its own table: this node holds it, so never lost — asked again within the hour.
        await ctx.background("keep: put", (await records()).put(idOf(t), JSON.stringify({ name: t.app, at: Date.now(), error: e?.message ?? String(e) })));
        throw e;
      })
      .finally(() => clearTimeout(timer));
    if (out) await (await records()).put(idOf(t), JSON.stringify(out));
    return out;
  }
  // CATALOGS first: what every other reader needs to find the rest (a space's `x…-tables`, the account's directory).
  const catalogFirst = ts => [...ts].sort((a, b) => Number(/(^|-)tables$|^catalog$/.test(b.app)) - Number(/(^|-)tables$|^catalog$/.test(a.app)));
  // Every coded file where this person reads: the account's, and each space's (null: the account).
  async function allFiles() {
    const out = [];
    for (const sp of [null, ...(await space.mine.strict()).filter(s => s.kind === "server")]) for (const row of await files.rows(sp)) out.push({ sp, row, key: `f/${sp?.id ?? ""}/${row.id}` });
    return out;
  }
  async function keepFile({ sp, row, key }) {
    let timer;
    const out = await Promise.race([files.keep(sp, row), new Promise((_, no) => (timer = setTimeout(() => no(new Error(`not kept in ${LIMIT / 60000} min: tried again later`)), LIMIT)))])
      .catch(e => ({ id: row.id, at: Date.now(), error: e?.message ?? String(e) }))
      .finally(() => clearTimeout(timer));
    let rec = { ...out, file: row.id, space: sp?.id ?? null };
    delete rec.id;
    if (rec.error) rec = await failed(key, rec);
    // ITS KEEPER too (owner 10-07): the uploader's node holds the original — said in the keepers bag, as a viewer who
    // keeps it is, so the data others fetch of it pays its holders (rewards §2), not "no keeper".
    else if (rec.gens && rec.whole === rec.gens) await claim(row.root).catch(e => ctx.failed("keep", e, `not said in the keepers bag: ${e?.message ?? e}`));
    await (await records()).put(key, JSON.stringify(rec));
    return rec;
  }
  async function due() {
    const held = new Map((await records()).rows().map(r => [r.key, read(r)]));
    const stale = (k, every) => {
      const r = held.get(k);
      // A FAILED attempt is not a keep. "Not found" can be a search that missed (asked again within the hour) or a
      // loss (no node holds it — this one neither): after TRIES failures it is LOST, said so, and never asked again.
      if (r?.lost) return false;
      return !r || Date.now() - r.at > (r.error ? RETRY : every);
    };
    return { tables: catalogFirst((await storage.own()).filter(t => stale(idOf(t), TABLE_EVERY))), files: (await allFiles()).filter(f => stale(f.key, EVERY)) };
  }

  // WATCHED: a file this person played, in their table `kept` (`w/<root>`): its ref (to read it again), the item it
  // is, its size; `used` (last played) orders them, `at` (last kept) makes one due a DAY later.
  const DAY = 86400000;
  const WATCH_LIMIT = 10 * 60000; // a large file's re-read takes longer than a table's
  const DEFAULT_LIMIT = 512e6;
  const keptTable = async () => {
    const t = await storage.table("kept");
    await t.settled;
    return t;
  };
  const noted = new Set(); // once per page
  async function watched(file, item, { title = null, kind = null } = {}) {
    if (!file?.root || !file?.key || noted.has(file.root)) return;
    noted.add(file.root);
    const t = await keptTable();
    const k = `w/${file.root}`;
    const was = read(t.rows().find(r => r.key === k) ?? { value: "null" }) ?? {};
    await t.put(k, JSON.stringify({ ...was, file: { key: file.key, root: file.root, size: file.size ?? 0, ...(file.b ? { b: file.b } : {}) }, item, title: title ?? was.title ?? null, kind: kind ?? was.kind ?? null, used: Date.now() }));
  }
  // THE LIMIT is each NODE's (owner 10-11: a phone and a home server keep different amounts): `limit/<member key>`;
  // a node without its own falls back to the account's older one, then the default.
  const thisNode = async () => (await (await ctx.require("auth")).check())?.member?.toLowerCase() ?? null;
  const limitOf = (t, node) => {
    const v = read(t.rows().find(r => r.key === `limit/${node}`) ?? t.rows().find(r => r.key === "limit") ?? { value: "null" });
    return typeof v === "number" ? v : DEFAULT_LIMIT;
  };
  async function setLimit(bytes) {
    await (await keptTable()).put(`limit/${await thisNode()}`, JSON.stringify(Math.max(0, Number(bytes) || 0)));
  }
  // Each node's limit (`keys`: the account's node keys, hex).
  async function limits(keys) {
    const t = await keptTable();
    return Object.fromEntries(keys.map(k => [k.toLowerCase(), limitOf(t, k.toLowerCase())]));
  }
  // What is kept: newest used first, while the sizes fit the limit; the rest dropped (no longer re-read).
  async function kept() {
    const t = await keptTable();
    const limit = limitOf(t, await thisNode());
    const all = t.rows().filter(r => r.key.startsWith("w/") && r.value).map(r => ({ key: r.key, rec: read(r) })).filter(w => w.rec?.file).map(w => ({ ...w.rec, key: w.key, w: w.rec })).sort((a, b) => (b.used ?? 0) - (a.used ?? 0));
    let total = 0;
    const files = [];
    for (const w of all) {
      if (total + (w.file.size ?? 0) > limit) {
        await ctx.background("keep: t.remove", t.remove(w.key));
        continue;
      }
      total += w.file.size ?? 0;
      files.push({ root: w.file.root, item: w.item, title: w.title, kind: w.kind ?? null, size: w.file.size ?? 0, used: w.used, at: w.at ?? 0, whole: w.whole, gens: w.gens, missing: w.missing, error: w.error ?? null, key: w.key, w });
    }
    return { limit, total, files };
  }
  async function keepWatched(w) {
    let timer;
    const row = { id: w.root, key: w.w.file.key, root: w.root, ...(w.w.file.b ? { b: w.w.file.b } : {}) };
    const out = await Promise.race([files.keep(null, row, { repair: false }), new Promise((_, no) => (timer = setTimeout(() => no(new Error(`not kept in ${WATCH_LIMIT / 60000} min: tried again later`)), WATCH_LIMIT)))])
      .catch(e => ({ error: e?.message ?? String(e) }))
      .finally(() => clearTimeout(timer));
    const { id: _id, ...rec } = out;
    if (!rec.error && rec.gens && rec.whole === rec.gens) await claim(w.root).catch(e => ctx.failed("keep", e, `not said in the keepers bag: ${e?.message ?? e}`));
    // A failure is retried within the hour, as a table's; a keep is good for a DAY.
    await (await keptTable()).put(w.key, JSON.stringify({ ...w.w, ...rec, at: rec.error ? Date.now() - DAY + RETRY : Date.now() }));
  }
  async function keepers(root) {
    const index = await ctx.require("index");
    const months = new Set([index.monthOf(), index.monthBefore()]);
    const claims = await index.pointers(`keepers:${root}`);
    return [...new Set(claims.filter(c => c && typeof c.did === "string" && months.has(c.m)).map(c => c.did))];
  }
  // Said in the bag once the file is WHOLE here (every generation), once a month (the same claim merges).
  async function claim(root) {
    const me = await space.me.strict();
    if (!me) return;
    const index = await ctx.require("index");
    await index.point(`keepers:${root}`, { did: me, m: index.monthOf() });
  }
  // ITS OWN FILES' CLAIMS, once a month (no re-read: a file its weekly pass found whole is held here): one a turn.
  async function claimOwn() {
    const t = await records();
    const m = (await ctx.require("index")).monthOf();
    const r = t.rows().find(x => x.key.startsWith("f/") && (v => v && !v.error && v.gens && v.whole === v.gens && v.claimed !== m)(read(x)));
    if (!r) return;
    const v = read(r);
    const list = await space.mine.strict();
    // (A space not in a list still being read: not "claimed" on a guess — asked again on a later tick.)
    if (v.space && list.complete === false && !list.some(s => s.id === v.space)) return;
    const row = (await files.rows.strict(v.space ? list.find(s => s.id === v.space) ?? null : null)).find(x => x.id === v.file);
    if (row?.root) await claim(row.root);
    await t.put(r.key, JSON.stringify({ ...v, claimed: m }));
  }
  // ITS HOLDINGS in standing epoch `epoch` (TOKEN §7a: what its keeper proves): the files it keeps WHOLE — its own and
  // its spaces' (a pass found every generation), and those it watched — as `{ sp, row }`, by file id. Fixed for the
  // epoch on first ask (`h/<epoch>` in the table `keep`: the record commits to this list, and a page opened later in
  // the epoch must answer from the same one); earlier epochs' lists dropped.
  async function holdings(epoch) {
    const t = await records();
    const k = `h/${epoch}`;
    const fixed = read(t.rows().find(r => r.key === k) ?? { value: "null" });
    const spaces = new Map((await space.mine.strict()).filter(s => s.kind === "server").map(s => [s.id, s]));
    const own = async () => {
      const out = [];
      for (const r of t.rows().filter(x => x.key.startsWith("f/"))) {
        const v = read(r);
        if (!v || v.error || !v.gens || v.whole !== v.gens) continue;
        out.push({ space: v.space ?? null, file: v.file });
      }
      for (const w of (await kept()).files) if (w.gens && w.whole === w.gens) out.push({ space: null, file: w.root, watched: true });
      // ASSIGNED files a pass has read and FOUND (`registry_found`: there at their listed size).
      for (const a of await assignedNow()) {
        const r = read(t.rows().find(x => x.key === `a/${a.key}`) ?? { value: "null" });
        if (r && !r.error && r.found === true) out.push({ space: null, file: a.key, assigned: true, size: a.size, missing: r.missing ?? [] });
      }
      return out.sort((a, b) => (a.file < b.file ? -1 : a.file > b.file ? 1 : 0));
    };
    const list = fixed ?? (await own());
    // (Nothing whole yet — the first passes still running: not fixed, asked again.)
    if (!fixed && list.length) {
      await t.put(k, JSON.stringify(list));
      for (const r of t.rows().filter(x => x.key.startsWith("h/") && x.key !== k)) await ctx.background("keep: t.remove", t.remove(r.key));
    }
    // Each one's row (its key and root): its space's, or the watched record's own ref.
    const watched = new Map((await kept()).files.map(w => [w.root, w.w.file]));
    const rowsOf = new Map();
    const out = [];
    for (const h of list) {
      if (h.assigned) {
        out.push({ assigned: { key: h.file, size: h.size, missing: h.missing } });
        continue;
      }
      if (h.watched) {
        const f = watched.get(h.file);
        if (f) out.push({ sp: null, row: { id: f.root, key: f.key, root: f.root, ...(f.b ? { b: f.b } : {}) }, watched: true });
        continue;
      }
      const sp = h.space ? (spaces.get(h.space) ?? null) : null;
      if (h.space && !sp) continue;
      if (!rowsOf.has(h.space)) rowsOf.set(h.space, await files.rows(sp));
      const row = rowsOf.get(h.space).find(x => x.id === h.file);
      if (row?.root) out.push({ sp, row });
    }
    return out;
  }
  // THE ASSIGNMENT (`keeper`: the registry's files this keeper ranks among the copies of — its CANDIDATES, ranked),
  // kept like what was watched: each re-read a DAY apart (`files.keepAt`: read only), its record `a/<address key>`
  // saying whether the read FOUND it. What it keeps of its candidates is `keeper_fill`'s (Rust: the one rule) — found
  // first, then unread, within its limit; absent never — so a listing nothing serves costs one read, never a place.
  async function assign(epoch, candidates) {
    await (await records()).put("assigned", JSON.stringify({ epoch, candidates }));
  }
  const readOf = (rows, key) => read(rows.get(`a/${key}`) ?? { value: "null" });
  // A read that decided (no error, its verdict said): what `keeper_fill` ranks by. ABSENT holds a DAY, then counts as
  // unread again (a file missed while its pieces were slow to answer competes again, for the room found files leave).
  const decided = r => r && !r.error && typeof r.found === "boolean" && (r.found || Date.now() - r.at <= DAY);
  async function assignedNow() {
    const t = await records();
    const candidates = read(t.rows().find(r => r.key === "assigned") ?? { value: "null" })?.candidates ?? [];
    if (!candidates.length) return [];
    const rows = new Map(t.rows().filter(r => r.key.startsWith("a/")).map(r => [r.key, r]));
    const reads = candidates.map(a => [a.key, readOf(rows, a.key)]).filter(([, r]) => decided(r)).map(([key, r]) => ({ key, found: r.found }));
    const Core = (await ctx.require("node")).glue.CraftworksCore;
    return Core.keeper_fill(candidates, (await kept()).limit, reads);
  }
  async function assignedDue() {
    const t = await records();
    const rows = new Map(t.rows().filter(r => r.key.startsWith("a/")).map(r => [r.key, r]));
    return (await assignedNow()).filter(a => {
      const r = readOf(rows, a.key);
      return !r || (!r.error && !decided(r)) || Date.now() - r.at > (r.error ? RETRY : DAY);
    });
  }
  async function keepAssigned(a) {
    let timer;
    const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
    const out = await Promise.race([files.keepAt(bytes(a.key), a.size), new Promise((_, no) => (timer = setTimeout(() => no(new Error(`not kept in ${WATCH_LIMIT / 60000} min: tried again later`)), WATCH_LIMIT)))])
      .catch(e => ({ error: e?.message ?? String(e) }))
      .finally(() => clearTimeout(timer));
    await (await records()).put(`a/${a.key}`, JSON.stringify({ size: a.size, at: Date.now(), ...out }));
    ctx.log("kept", { what: `assigned file ${a.key.slice(0, 8)}…: ${out.error ?? (out.found ? `${out.pieces - out.missing.length}/${out.pieces} piece(s) answered` : `NOT THERE at its listed size (${out.missing.length} piece(s) unanswered): its place goes to the next`)}` });
  }
  const watchedDue = async () => (await kept()).files.filter(w => Date.now() - w.at > DAY);

  let running = false;
  let said = false;
  async function tick() {
    if (running || !(await space.account.strict())) return;
    running = true;
    try {
      // Its records unreadable here (no grant for them on this site yet, say): said once, and tried next time.
      const d = await due().catch(e => {
        if (!said) ctx.failed("keep", e, `not keeping yet: ${e?.message ?? e}`);
        said = true;
        return { tables: [], files: [] };
      });
      if (d.tables[0]) await keepOne(d.tables[0]).catch(e => ctx.failed("keep", e, `${d.tables[0].app}: ${e?.message ?? e}`));
      else if (d.files[0]) await keepFile(d.files[0]);
      else {
        // Its own first; then what it is assigned; then what it played, kept for others.
        const a = (await ctx.optional("keep: assignedDue", assignedDue(), () => []))[0];
        const w = a ? null : (await ctx.optional("keep: watchedDue", watchedDue(), () => []))[0];
        if (a) await keepAssigned(a);
        else if (w) await keepWatched(w);
        else await claimOwn().catch(e => ctx.failed("keep", e, `own files not said in the keepers bag: ${e?.message ?? e}`));
      }
    } finally {
      running = false;
    }
  }
  // EVERY table and file kept now, saying how far it is (`onProgress({ done, of, name })`).
  async function now({ onProgress = () => {} } = {}) {
    const out = [];
    const ts = catalogFirst(await storage.own());
    const fs = await allFiles();
    const of = ts.length + fs.length;
    let done = 0;
    for (const t of ts) {
      onProgress({ done, of, name: t.app });
      out.push(await keepOne(t).catch(e => ({ name: t.app, error: e?.message ?? String(e) })));
      done += 1;
    }
    for (const f of fs) {
      onProgress({ done, of, name: `file ${f.row.id.slice(0, 8)}` });
      out.push(await keepFile(f));
      done += 1;
    }
    return out.filter(Boolean);
  }
  // After the page has settled; then one due table a minute.
  // (A JOB: every minute, two minutes after the page is up, while someone is logged in.)
  (await ctx.require("lifecycle")).job("keep", { every: 60000, delay: 120000, needs: async () => !!(await space.account()) || "nobody is logged in", run: () => tick() });
  return { status: ctx.total("keep.status", () => [], status), now, watched: (f, item, o) => watched(f, item, o).catch(e => ctx.failed("keep", e, `not noted for keeping: ${e?.message ?? e}`)), kept, setLimit, limits, keepers: ctx.total("keep.keepers", () => [], keepers), holdings, assign, assigned: assignedNow };
}
