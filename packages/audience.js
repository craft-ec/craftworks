// AUDIENCE, a component: the ONE choice of who sees an item, for every app that makes one (Board, Videos, Audio,
// Notes, Drive). In YOUR space: everyone (public) or only you. In a SHARED space: its members, or everyone — public
// only while the space lets its app read in public (its policy is the ceiling: members-only = members post and read
// it, nobody makes it public). What it means for where the item is kept is `items`' (`submit({ audience })`).
//
//   const audience = await ctx.require("audience");
//   const a = await audience.picker({ space, kind })   // space: null = yours; kind: what is made ("post", "movie" …)
//   form.append(a.el)
//   a.value()      // "public" | "private" (yours) | "members" (a space's)
//   a.isPublic()   // read by everyone (its files are put public)
export async function start(ctx) {
  const [items, space] = await Promise.all(["items", "space"].map(n => ctx.require(n)));
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };

  async function picker({ space: sp = null, kind = "post" } = {}) {
    // The choices here: yours, or what the space's policy allows for this kind.
    const choices = sp
      ? (await items.publicIn(sp, kind).catch(() => false))
        ? [["public", "🌐 Everyone (public)"], ["members", `👥 ${space.shown(sp)}'s members`]]
        : [["members", `👥 ${space.shown(sp)}'s members (the space keeps this app to its members)`]]
      : [["public", "🌐 Everyone (public, your followers read it)"], ["private", "🔒 Only you"]];
    const sel = h("select", { className: "field", name: "audience" }, ...choices.map(([value, textContent]) => h("option", { value, textContent })));
    const el = h("label", {}, "Who sees it", sel);
    return { el, value: () => sel.value, isPublic: () => sel.value === "public" };
  }
  return { picker };
}
