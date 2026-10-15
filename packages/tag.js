// TAG, a page: everything tagged `#<tag>` (`#/tag/<tag>`) — every kind, the same look as anywhere (`cards`), from
// what this person reads (the spaces they are in, whom they follow) and from Discover (public). A TAG is an item's
// own, its author's (`items.tagsOf`: what it is, for everyone); NSFW stays hidden unless chosen (`items.list`).
export async function mount(ctx, el) {
  const [items, cards, actions, theme, kinds] = await Promise.all(["items", "cards", "actions", "theme", "kinds"].map(n => ctx.require(n)));
  const bar = (await ctx.require("feed-bar")).create({ start: "hot", onChange: () => draw() }); // (draw: below)
  const tagNow = () => decodeURIComponent(String(ctx.sub ?? "").split("/")[0] ?? "").toLowerCase();
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  el.innerHTML = `
    <style>@layer apps {
      .tg { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); }
      .tg h2 { margin: 0; }
      .tg .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: var(--cw-space-3); align-items: start; }
      .tg .none { color: var(--cw-muted); }
    }</style>
    <div class="tg"><h2></h2><div class="bar"></div><div class="results"></div></div>`;
  const grid = el.querySelector(".results");
  // Every kind that stands on its own and has a look (a channel is a place, a folder Drive's, a delegation Vote's record).
  const shown = kinds.all().filter(k => !["channel", "folder", "delegation"].includes(k));
  // THE RESULTS: a VIEW (`ctx.view` — loading, empty, redrawn when what it read changes); the tag's own cards.
  const results = ctx.view(grid, {
    read: async () => {
      const tag = tagNow();
      const [mine, open] = await Promise.all([
        items.list({ feed: true, tag }, bar.sort(), shown, bar.options()),
        items.list({ discover: true, tag }, bar.sort(), shown, bar.options()),
      ]);
      const seen = new Set();
      return bar.reorder([...mine, ...open].filter(p => !seen.has(p.ref) && seen.add(p.ref)));
    },
    draw: list =>
      h(
        "div",
        { className: "grid" },
        ...list.map(p => {
          const open = () => (location.hash = items.pageOf(p.ref, p.kind));
          return cards.card(p, { open, actions: [actions.bar(p, { open, changed: () => results.refresh(), removed: () => results.refresh() })] });
        }),
      ),
    looks: { empty: () => h("p", { className: "none", textContent: `Nothing tagged #${tagNow()} ${bar.span()}.` }), loading: () => theme.loading(`Finding #${tagNow()}…`) },
  });
  function draw() {
    el.querySelector("h2").textContent = `#${tagNow()}`;
    el.querySelector(".bar").replaceChildren(bar.el());
    return results.refresh();
  }
  draw();
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/tag" && draw());
}
