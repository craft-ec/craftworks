// KEEP, a service (started by the header, on every page, after the page is up): the LIFECYCLE of this person's data
// (phase 4) — what keeps it on the network with no server. A table's ASSET is every block its current tree reaches;
// KEEPING it asks each block (its HEALTH: per group, WHOLE / DEGRADED / DAMAGED), puts again only those that do not
// answer (rebuilt from their group) and puts the tail's signed state again (`storage` tables' `keep`).
// DUE, with no job list: a table this node writes is due when its record in the account's table `keep` is missing or
// older than a week. FILES too (`files.keep`): every coded file of the account and of each space this person is in —
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
//   await keep.setLimit(bytes)   // how much of what was played this node keeps (0: none)
//   await keep.keepers(root) // [did, …]: who keeps a file (this month or last) — the KEEPERS BAG (rewards step 3)
//
// THE KEEPERS BAG (rewards step 3): each file's public bag `keepers:<root>` (`index.point`), where a keeper says, once
// a month, that it keeps the file (`{ did, m: "YYYY-MM" }`; the same claim again merges). A CLAIM, unsigned like every
// pointer and public like a view count (owner 10-06: confidential later): it shows who keeps what; the proofs (step 4)
// are what will make a claim count for pay. Its OWN files' and its spaces' files' holders claim too, once a pass finds
// them whole (the uploader's node holds the original: the first keeper of anything).
export async function start(ctx) {
  const [storage, space, files] = await Promise.all(["storage", "space", "files"].map(n => ctx.require(n)));
  const EVERY = 7 * 86400000; // a table kept longer ago than this is due
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
        await (await records()).put(idOf(t), JSON.stringify({ name: t.app, at: Date.now(), error: e?.message ?? String(e) })).catch(() => {});
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
    for (const sp of [null, ...(await space.mine()).filter(s => s.kind === "server")]) for (const row of await files.rows(sp).catch(() => [])) out.push({ sp, row, key: `f/${sp?.id ?? ""}/${row.id}` });
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
    else if (rec.gens && rec.whole === rec.gens) await claim(row.root).catch(e => ctx.log("keep", { what: `not said in the keepers bag: ${e?.message ?? e}` }));
    await (await records()).put(key, JSON.stringify(rec));
    return rec;
  }
  async function due() {
    const held = new Map((await records()).rows().map(r => [r.key, read(r)]));
    const stale = k => {
      const r = held.get(k);
      // A FAILED attempt is not a keep. "Not found" can be a search that missed (asked again within the hour) or a
      // loss (no node holds it — this one neither): after TRIES failures it is LOST, said so, and never asked again.
      if (r?.lost) return false;
      return !r || Date.now() - r.at > (r.error ? RETRY : EVERY);
    };
    return { tables: catalogFirst((await storage.own()).filter(t => stale(idOf(t)))), files: (await allFiles()).filter(f => stale(f.key)) };
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
  const limitOf = t => {
    const v = read(t.rows().find(r => r.key === "limit") ?? { value: "null" });
    return typeof v === "number" ? v : DEFAULT_LIMIT;
  };
  async function setLimit(bytes) {
    await (await keptTable()).put("limit", JSON.stringify(Math.max(0, Number(bytes) || 0)));
  }
  // What is kept: newest used first, while the sizes fit the limit; the rest dropped (no longer re-read).
  async function kept() {
    const t = await keptTable();
    const limit = limitOf(t);
    const all = t.rows().filter(r => r.key.startsWith("w/") && r.value).map(r => ({ key: r.key, rec: read(r) })).filter(w => w.rec?.file).map(w => ({ ...w.rec, key: w.key, w: w.rec })).sort((a, b) => (b.used ?? 0) - (a.used ?? 0));
    let total = 0;
    const files = [];
    for (const w of all) {
      if (total + (w.file.size ?? 0) > limit) {
        await t.remove(w.key).catch(() => {});
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
    if (!rec.error && rec.gens && rec.whole === rec.gens) await claim(w.root).catch(e => ctx.log("keep", { what: `not said in the keepers bag: ${e?.message ?? e}` }));
    // A failure is retried within the hour, as a table's; a keep is good for a DAY.
    await (await keptTable()).put(w.key, JSON.stringify({ ...w.w, ...rec, at: rec.error ? Date.now() - DAY + RETRY : Date.now() }));
  }
  const monthOf = (t = Date.now()) => new Date(t).toISOString().slice(0, 7);
  async function keepers(root) {
    const index = await ctx.require("index");
    const now = new Date();
    const last = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth() - 1, 1));
    const months = new Set([monthOf(), monthOf(last.getTime())]);
    const claims = await index.pointers(`keepers:${root}`).catch(() => []);
    return [...new Set(claims.filter(c => c && typeof c.did === "string" && months.has(c.m)).map(c => c.did))];
  }
  // Said in the bag once the file is WHOLE here (every generation), once a month (the same claim merges).
  async function claim(root) {
    const me = (await space.account())?.id;
    if (!me) return;
    await (await ctx.require("index")).point(`keepers:${root}`, { did: me, m: monthOf() });
  }
  // ITS OWN FILES' CLAIMS, once a month (no re-read: a file its weekly pass found whole is held here): one a turn.
  async function claimOwn() {
    const t = await records();
    const m = monthOf();
    const r = t.rows().find(x => x.key.startsWith("f/") && (v => v && !v.error && v.gens && v.whole === v.gens && v.claimed !== m)(read(x)));
    if (!r) return;
    const v = read(r);
    const row = (await files.rows(v.space ? (await space.mine()).find(s => s.id === v.space) ?? null : null).catch(() => [])).find(x => x.id === v.file);
    if (row?.root) await claim(row.root);
    await t.put(r.key, JSON.stringify({ ...v, claimed: m }));
  }
  const watchedDue = async () => (await kept()).files.filter(w => Date.now() - w.at > DAY);

  let running = false;
  let said = false;
  async function tick() {
    if (running || !(await space.account())) return;
    running = true;
    try {
      // Its records unreadable here (no grant for them on this site yet, say): said once, and tried next time.
      const d = await due().catch(e => {
        if (!said) ctx.log("keep", { what: `not keeping yet: ${e?.message ?? e}` });
        said = true;
        return { tables: [], files: [] };
      });
      if (d.tables[0]) await keepOne(d.tables[0]).catch(e => ctx.log("keep", { what: `${d.tables[0].app}: ${e?.message ?? e}` }));
      else if (d.files[0]) await keepFile(d.files[0]);
      else {
        // Its own first; then what it played, kept for others.
        const w = (await watchedDue().catch(() => []))[0];
        if (w) await keepWatched(w);
        else await claimOwn().catch(e => ctx.log("keep", { what: `own files not said in the keepers bag: ${e?.message ?? e}` }));
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
  setTimeout(() => {
    tick();
    setInterval(tick, 60000);
  }, 120000);
  return { status, now, watched: (f, item, o) => watched(f, item, o).catch(e => ctx.log("keep", { what: `not noted for keeping: ${e?.message ?? e}` })), kept, setLimit, keepers };
}
