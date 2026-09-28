// ACCESS, a capability: who may read and write the person's data from THIS site. A table is sealed, so reading needs its
// key, and the identity delegate gives a site the key only with the person's GRANT for that table (the home site needs
// none). Grants are asked once per page for every kind of data the site uses (its manifest's `uses`) in ONE prompt; a
// table outside that list is asked for on its own. A "no" stands until the page is opened again, so a refused site
// does not re-ask on every write; a failure to ask (no answer from the node) is not a "no" and is asked again.
//
//   const access = await ctx.require("access");
//   await access.allowed("notes")          // true once the person allowed this site "notes"
//   await access.key("notes")              // { key: hex } or { why }; the catalog's with { catalog: true }
export async function start(ctx) {
  const auth = await ctx.require("auth");

  // Asked once per page for every kind of data the site uses (its manifest's `uses`) in ONE prompt; a table
  // outside that list is asked for on its own. A "no" stands until the page is opened again, so a refused site does
  // not re-ask on every write; a failure to ask (no answer from the node) is not a "no" and is asked again.
  const asked = new Map(); // table -> Promise<bool>
  function grant(name) {
    if (asked.has(name)) return asked.get(name);
    const list = ctx.uses.includes(name) ? ctx.uses.filter(t => !asked.has(t)) : [name];
    const p = auth.identity.grant(list).then(
      g => {
        ctx.log(g.granted ? "allowed" : "not allowed", { what: `${list.join(", ")}${g.granted ? "" : `: ${g.refused ?? JSON.stringify(g)}`}` });
        return !!g.granted;
      },
      e => {
        for (const t of list) asked.delete(t);
        ctx.log("not allowed", { what: `${list.join(", ")}: ${e.message}` });
        return false;
      },
    );
    for (const t of list) asked.set(t, p);
    return p;
  }

  const allowed = name => grant(name);

  async function key(table, { catalog = false } = {}) {
    // The catalog's key comes with any grant: listing a table is part of using it.
    const ok = catalog || (await allowed(table).catch(() => false));
    if (!ok) return { why: "not allowed" };
    const k = await auth.identity.tableKey(table).catch(e => ({ refused: e?.message ?? String(e) }));
    return k?.tableKey ? { key: k.tableKey } : { why: k?.refused ?? "no key" };
  }

  return { allowed, key };
}
