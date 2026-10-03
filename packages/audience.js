// AUDIENCE, a component: the ONE choice of who sees an item, for every app that makes one (Board, Videos, Audio,
// Notes, Drive). In YOUR space: everyone (public), your followers, your friends (each a CIRCLE: `circles`), or only
// you. In a SHARED space: its members, or everyone — public
// only while the space lets its app read in public (its policy is the ceiling: members-only = members post and read
// it, nobody makes it public). What it means for where the item is kept is `items`' (`submit({ audience })`).
//
//   const audience = await ctx.require("audience");
//   const a = await audience.picker({ space, kind, initial })   // space: null = yours; kind: what is made ("post",
//                                                               // "movie", "note" …); initial: the choice it starts on
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

  async function picker({ space: sp = null, kind = "post", initial = null } = {}) {
    // The choices here: yours, or what the space's policy allows for this kind.
    const choices = sp
      ? (await items.publicIn(sp, kind).catch(() => false))
        ? [["public", "🌐 Everyone (public)"], ["members", `👥 ${space.shown(sp)}'s members`]]
        : [["members", `👥 ${space.shown(sp)}'s members (the space keeps this app to its members)`]]
      : [["public", "🌐 Everyone (public)"], ["followers", "👣 Your followers"], ["friends", "🤝 Your friends"], ["private", "🔒 Only you"]];
    const sel = h("select", { className: "field", name: "audience" }, ...choices.map(([value, textContent]) => h("option", { value, textContent })));
    // Its starting choice is the select's DEFAULT: a form reset (a composer closing) returns to it, never to the first.
    for (const o of sel.options) o.defaultSelected = o.value === initial;
    if (initial && choices.some(([v]) => v === initial)) sel.value = initial;
    // WHO MAY COMMENT AND VOTE — its own rule (`roles.mayWrite`, the one check), apart from who sees it, as a
    // database's read and write: in your space anyone · your followers · your friends · only you; in a space its
    // members (as its policy allows) · only you.
    const writes = sp
      ? [["members", "👥 Who the space allows"], ["author", "🔒 Only you"]]
      : [["anyone", "🌐 Anyone who sees it"], ["followers", "👣 Your followers"], ["friends", "🤝 Your friends"], ["author", "🔒 Only you"]];
    const wsel = h("select", { className: "field", name: "write" }, ...writes.map(([value, textContent]) => h("option", { value, textContent })));
    const el = h("div", { className: "cw-aud", style: "display:grid;gap:6px" }, h("label", {}, "Who sees it", sel), h("label", {}, "Who may comment and vote", wsel));
    return {
      el,
      value: () => sel.value,
      isPublic: () => sel.value === "public",
      // The write rule ({ comment, vote }), or null where it is the space's own (nothing to set on the item).
      write: () => (wsel.value === "anyone" || wsel.value === "members" ? null : { comment: wsel.value, vote: wsel.value }),
    };
  }
  return { picker };
}
