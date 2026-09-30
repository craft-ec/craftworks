// KEEP, a service (started by the header, on every page, after the page is up): the LIFECYCLE of this person's data
// (phase 4) — what keeps it on the network with no server. A table's ASSET is every block its current tree reaches;
// KEEPING it asks each block (its HEALTH: per group, WHOLE / DEGRADED / DAMAGED), puts each one again (re-published
// where it is there, repaired where it is not) and puts the tail's signed state again (`storage` tables' `keep`).
// DUE, with no job list: a table this node writes is due when its record in the account's table `keep` is missing or
// older than a week. One table at a time, in the background, while any page of the account is open — and the Storage
// page keeps them all at once on a click.
//
//   const keep = await ctx.require("keep");
//   await keep.status()      // [{ name, at, groups, blocks, whole, degraded, damaged, missing, put, unmade, ms }]
//   await keep.now()         // every table kept now; their records
export async function start(ctx) {
  const [storage, space] = await Promise.all(["storage", "space"].map(n => ctx.require(n)));
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
  async function due() {
    const held = new Map((await records()).rows().map(r => [r.key, read(r)]));
    return (await storage.own()).filter(t => {
      const r = held.get(idOf(t));
      return !r || Date.now() - r.at > EVERY;
    });
  }

  let running = false;
  async function tick() {
    if (running || !(await space.account())) return;
    running = true;
    try {
      const [t] = await due();
      if (t) await keepOne(t).catch(e => ctx.log("keep", { what: `${t.app}: ${e?.message ?? e}` }));
    } finally {
      running = false;
    }
  }
  async function now() {
    const out = [];
    for (const t of await storage.own()) out.push(await keepOne(t).catch(e => ({ name: t.app, error: e?.message ?? String(e) })));
    return out.filter(Boolean);
  }
  // After the page has settled; then one due table a minute.
  setTimeout(() => {
    tick();
    setInterval(tick, 60000);
  }, 120000);
  return { status, now };
}
