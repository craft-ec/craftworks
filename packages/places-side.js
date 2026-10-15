// PLACES SIDE, a component (the layout's side slot — owner 10-15: "for all app, move spaces to side panel. youtube and
// reddit works like that"): every app's places in one panel (`places.side`), the app's own content beside it. On a
// phone the panel is a drawer: a ☰ button at the left of the page opens it, a choice or a tap outside closes it.
// Chat's page has no side panel: its rail is its own (Discord's).
export async function mount(ctx, el) {
  const places = await ctx.require("places");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const style = h("style", {
    textContent: `@layer components {
      .cw-side { width: 220px; box-sizing: border-box; padding: var(--cw-space-3) var(--cw-space-2); border-right: 1px solid var(--cw-line); overflow-y: auto; }
      .cw-side:empty { display: none; }
      .cw-side ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 2px; }
      .cw-side a { display: flex; align-items: center; gap: 8px; padding: 6px var(--cw-space-2); border-radius: var(--cw-radius-sm); color: inherit; text-decoration: none; }
      .cw-side a:hover { background: var(--cw-hover); }
      .cw-side a.on { background: var(--cw-hover); font-weight: 600; }
      .cw-side a .cw-badge { margin-left: auto; }
      .cw-side .more { margin-top: var(--cw-space-2); color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .cw-side section + section { margin-top: var(--cw-space-3); padding-top: var(--cw-space-3); border-top: 1px solid var(--cw-line); }
      .cw-side h4 { margin: 0 0 4px; padding: 0 var(--cw-space-2); font-size: var(--cw-text-xs); text-transform: uppercase; letter-spacing: .04em; color: var(--cw-muted); }
      .cw-side li.group { padding: 6px var(--cw-space-2) 2px; font-size: var(--cw-text-sm); color: var(--cw-muted); }
      .cw-side a.sub { padding-left: var(--cw-space-4); }
      .cw-side .make { font-weight: 600; }
      .cw-side button.cw-flat { display: block; width: 100%; text-align: left; padding: 6px var(--cw-space-2); }
      .cw-side-open { display: none; }
      @media (max-width: 760px) {
        .cw-side-open { display: inline-flex; position: fixed; left: 8px; bottom: 56px; z-index: 41; border-radius: 999px; padding: 8px 12px; box-shadow: var(--cw-shadow-lg); }
        .cw-side-open[hidden] { display: none; }
      }
    }`,
  });
  const panel = h("div");
  const slot = () => el.closest(".slot-side");
  const open = h("button", { type: "button", className: "cw-side-open cw-btn", textContent: "☰ Places", hidden: true, onclick: () => slot()?.classList.toggle("open") });
  el.replaceChildren(style, panel);
  document.body.append(open);
  // A choice made, or a tap outside: the drawer closes (on a phone).
  panel.addEventListener("click", e => e.target.closest("a") && slot()?.classList.remove("open"));
  addEventListener("pointerdown", e => !el.contains(e.target) && e.target !== open && slot()?.classList.remove("open"), { signal: ctx.signal });
  ctx.signal?.addEventListener("abort", () => open.remove());
  places.side(panel);
  // The ☰ only where the panel has something to show; and while it does, the header leaves the app's entries to it
  // (`sideNav`, on the page's own ctx: what the header reads).
  const base = Object.getPrototypeOf(ctx);
  const showing = () => {
    const on = !!panel.childElementCount && !!panel.textContent.trim();
    open.hidden = !on;
    if (base.sideNav !== on) (base.sideNav = on), dispatchEvent(new CustomEvent("craftworks:actions"));
  };
  new MutationObserver(showing).observe(panel, { childList: true });
  ctx.signal?.addEventListener("abort", () => base.sideNav && ((base.sideNav = false), dispatchEvent(new CustomEvent("craftworks:actions"))));
}
