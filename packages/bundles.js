// BUNDLES, a capability (`kinds`: the BUNDLE kind): a set that is an ITEM of its own — an album, a course, a
// collection — naming its members in order (`meta.members`: item refs, of ANY kind, across capabilities). Its own page,
// comments, votes and audience are every item's; this is only what is particular to it: which bundles a group holds,
// making one, and its members. No UI (the Bundle app, and "Add to a bundle" on any item's page).
//
//   const bundles = await ctx.require("bundles");
//   await bundles.of(sp /* null: yours */)        // [{ ref, title, members, item }]
//   await bundles.make("Course", sp)               // its ref
//   await bundles.add(bref, ref); await bundles.drop(bref, ref); await bundles.move(bref, ref, -1)
export async function start(ctx) {
  const [items, space] = await Promise.all([ctx.require("items"), ctx.require("space")]);
  const KIND = "bundle";
  const MAX = 500;
  const shape = b => ({ ref: b.ref, title: b.title || "untitled", members: Array.isArray(b.meta?.members) ? b.meta.members.filter(m => typeof m === "string") : [], item: b });
  async function of(sp = null) {
    const where = sp ? { board: sp.id } : { by: await space.me() };
    return (await items.list(where, "new", [KIND], { window: "all" })).map(shape);
  }
  const make = (title, sp = null, { audience = sp ? "members" : "private" } = {}) =>
    items.submit({ kind: KIND, title: String(title ?? "").trim(), body: "", meta: { members: [] }, board: sp?.id ?? null, audience });
  async function members(bref, change) {
    const b = await items.get(bref);
    if (!b || b.kind !== KIND) throw new Error("not a bundle");
    const now = shape(b).members;
    const next = change(now);
    if (next.length > MAX) throw new Error(`a bundle holds at most ${MAX} items`);
    if (next.length === now.length && next.every((x, i) => x === now[i])) return;
    await items.editItem(bref, b.body ?? "", { meta: { ...(b.meta ?? {}), members: next } });
  }
  const add = (bref, ref) => {
    if (ref === bref) return Promise.reject(new Error("a bundle cannot hold itself"));
    return members(bref, m => (m.includes(ref) ? m : [...m, ref]));
  };
  const drop = (bref, ref) => members(bref, m => m.filter(x => x !== ref));
  const move = (bref, ref, by) =>
    members(bref, m => {
      const i = m.indexOf(ref);
      const j = Math.max(0, Math.min(m.length - 1, i + by));
      if (i < 0 || i === j) return m;
      const out = [...m];
      out.splice(j, 0, out.splice(i, 1)[0]);
      return out;
    });
  return { KIND, of, make, add, drop, move };
}
