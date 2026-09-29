// APP ICONS, a component: an app as every Home shows it — its icon and name, a pill with what is new in it, and
// (where the Home keeps them) its pin; or DIMMED, with an Add, for an app a space does not use yet. One look for the
// personal Home, a space's Home and Discover's. UI only: counts are the caller's (`activity`), pins `pin-button`'s.
//
//   const icons = await ctx.require("app-icons");
//   icons.grid(host, [{ app, href, count, pin, add }])   // pin: an element beside the link; add: fn (dimmed, "+ Add")
export async function start() {
  const style = document.createElement("style");
  style.textContent = `
    .cw-icons { display: grid; grid-template-columns: repeat(auto-fit, 96px); justify-content: center; gap: var(--cw-space-3); }
    .cw-icons .tile { position: relative; }
    .cw-icons .app { position: relative; display: grid; justify-items: center; gap: 6px; padding: var(--cw-space-3) 6px; border-radius: var(--cw-radius);
      text-decoration: none; color: inherit; text-align: center; }
    .cw-icons a.app:hover { background: var(--cw-hover); }
    .cw-icons .icon { font-size: 40px; line-height: 1; }
    .cw-icons .name { font-size: var(--cw-text-sm); }
    .cw-icons .app .new { position: absolute; top: 2px; left: calc(50% + 10px); margin: 0; }
    .cw-icons .cw-pin { position: absolute; top: 2px; right: 2px; font-size: 14px; }
    .cw-icons .off .icon, .cw-icons .off .name { opacity: .45; }
    .cw-icons .off button { font: inherit; font-size: var(--cw-text-xs); border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-accent);
      border-radius: var(--cw-radius-pill); padding: 1px 8px; cursor: pointer; }
    .cw-icons .empty { grid-column: 1 / -1; color: var(--cw-muted); font-size: var(--cw-text-sm); text-align: center; }`;
  document.head.append(style);
  const el = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  // A tile: the app's link (or, not used yet, the dimmed app and its Add) and, beside it — never inside: the loader
  // takes every click on a `#` link — its pin.
  function tile({ app, href, count = 0, pin = null, add = null }) {
    const face = [el("span", { className: "icon", textContent: app.icon ?? "▫️" }), el("span", { className: "name", textContent: app.name })];
    const body = add
      ? el("div", { className: "app off" }, ...face, el("button", { type: "button", textContent: "+ Add", onclick: add }))
      : el("a", { className: "app", href }, ...face, count ? el("span", { className: "cw-badge new", textContent: String(count) }) : null);
    return el("div", { className: "tile" }, body, pin);
  }
  function grid(host, items, empty = null) {
    host.classList.add("cw-icons");
    host.replaceChildren(...(items.length ? items.map(tile) : empty ? [el("p", { className: "empty", textContent: empty })] : []));
  }
  return { grid, tile };
}
