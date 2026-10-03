// KEEP, a service (started by the header, on every page, after the page is up): the LIFECYCLE of this person's data
// (phase 4) — what keeps it on the network with no server. A table's ASSET is every block its current tree reaches;
// KEEPING it asks each block (its HEALTH: per group, WHOLE / DEGRADED / DAMAGED), puts each one again (re-published
// where it is there, repaired where it is not) and puts the tail's signed state again (`storage` tables' `keep`).
// DUE, with no job list: a table this node writes is due when its record in the account's table `keep` is missing or
// older than a week. FILES too (`files.keep`): every coded file of the account and of each space this person is in —
// each piece asked (HEALTH per generation) and put again. One table or file at a time, in the background, while any
// page of the account is open — and the Storage page keeps them all at once on a click.
//
//   const keep = await ctx.require("keep");
//   await keep.status()      // tables: [{ name, at, groups, blocks, whole, degraded, damaged, missing, put, unmade, ms }]
//                            // and files: [{ file: id, space, at, pieces, gens, whole, degraded, damaged, missing, put }]
//   await keep.now()         // every table kept now; their records
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
  return { status, now };
}
