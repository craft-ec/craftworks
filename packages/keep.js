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
  async function keepOne(t) {
    const out = await t.keep();
    if (out) await (await records()).put(idOf(t), JSON.stringify(out));
    return out;
  }
  // Every coded file where this person reads: the account's, and each space's (null: the account).
  async function allFiles() {
    const out = [];
    for (const sp of [null, ...(await space.mine()).filter(s => s.kind === "server")]) for (const row of await files.rows(sp).catch(() => [])) out.push({ sp, row, key: `f/${sp?.id ?? ""}/${row.id}` });
    return out;
  }
  async function keepFile({ sp, row, key }) {
    const out = await files.keep(sp, row).catch(e => ({ id: row.id, at: Date.now(), error: e?.message ?? String(e) }));
    const rec = { ...out, file: row.id, space: sp?.id ?? null };
    delete rec.id;
    await (await records()).put(key, JSON.stringify(rec));
    return rec;
  }
  async function due() {
    const held = new Map((await records()).rows().map(r => [r.key, read(r)]));
    const stale = k => {
      const r = held.get(k);
      return !r || Date.now() - r.at > EVERY;
    };
    return { tables: (await storage.own()).filter(t => stale(idOf(t))), files: (await allFiles()).filter(f => stale(f.key)) };
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
  async function now() {
    const out = [];
    for (const t of await storage.own()) out.push(await keepOne(t).catch(e => ({ name: t.app, error: e?.message ?? String(e) })));
    for (const f of await allFiles()) out.push(await keepFile(f));
    return out.filter(Boolean);
  }
  // After the page has settled; then one due table a minute.
  setTimeout(() => {
    tick();
    setInterval(tick, 60000);
  }, 120000);
  return { status, now };
}
