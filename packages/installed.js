// INSTALLED APPS, a service (ARCHITECTURE §1, "The desktop"; FOUNDATION.md §3): which apps this person has, kept in the
// account's `installed` table (on the network: the same on every device). Every app is installed until it is removed
// (a row `{ removed: true }`); installing again drops that. Home and the App Store read it; nothing else decides it.
// Apps' data stays the person's when an app is removed (its tables are not touched).
//
//   const installed = await ctx.require("installed");
//   await installed.has("/wiki")      // true unless removed
//   await installed.list()            // the site's apps installed here (manifest entries)
//   await installed.set("/wiki", false)
export async function start(ctx) {
  const [space, storage] = await Promise.all([ctx.require("space"), ctx.require("storage")]);
  const SYSTEM = new Set(["/settings", "/spaces", "/store"]); // never removed
  let table = null;
  const tableOf = async () => (table ??= (await space.account()) ? storage.table("installed") : null);
  addEventListener("craftworks:auth", () => (table = null));
  async function removed() {
    const t = await tableOf();
    if (!t) return new Set();
    await t.decided?.();
    const out = new Set();
    for (const r of t.rows()) {
      try {
        if (JSON.parse(r.value)?.removed) out.add(r.key);
      } catch {}
    }
    return out;
  }
  const has = async route => SYSTEM.has(route) || !(await removed()).has(route);
  const list = async () => {
    const gone = await removed();
    return (ctx.apps ?? []).filter(a => SYSTEM.has(a.route) || !gone.has(a.route));
  };
  async function set(route, on) {
    if (SYSTEM.has(route) && !on) throw new Error("a system app is always installed");
    const t = await tableOf();
    if (!t) throw space.loginNeeded();
    await t.put(route, JSON.stringify(on ? { at: Date.now() } : { removed: true, at: Date.now() }));
    dispatchEvent(new CustomEvent("craftworks:installed"));
  }
  return { has, list, set, system: route => SYSTEM.has(route) };
}
