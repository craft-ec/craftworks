// APP ICONS, a component: an app as every Home shows it — its icon and name, a pill with what is new in it, and
// (where the Home keeps them) its pin; or DIMMED, with an Add, for an app a space does not use yet. One look for the
// personal Home, a space's Home and Discover's. UI only: counts are the caller's (`activity`), pins `pin-button`'s.
//
//   const icons = await ctx.require("app-icons");
//   icons.grid(host, [{ app, href, count, pin, add, note }])   // pin: beside the link; add: fn (dimmed, "+ Add");
//                                                              // no href: dimmed, `note` saying why
//   icons.drawer(host, items, { empty })   // ALL APPS as a phone shows them: by name, or — this person's choice, kept
//                                          // with their account — grouped by CATEGORY (the manifest's), a switch on top
//   const desk = icons.desk(host, { items, pinKey, empty })   // THE HOME LAYOUT, the same for every Home (yours and a
//                                          // space's): PINNED, then ALL APPS (the drawer). `items()`: the tiles now;
//                                          // `pinKey(app)`: its pin's key (a space's own pins are its own). desk.redraw()
export async function start(ctx) {
  const style = document.createElement("style");
  style.textContent = `@layer components {

    .cw-icons { display: grid; grid-template-columns: repeat(auto-fit, 96px); justify-content: center; gap: var(--cw-space-3); }
    .cw-icons .tile { position: relative; }
    .cw-icons .app { position: relative; display: grid; justify-items: center; gap: 6px; padding: var(--cw-space-3) 6px; border-radius: var(--cw-radius);
      text-decoration: none; color: inherit; text-align: center; }
    .cw-icons a.app:hover { background: var(--cw-hover); }
    .cw-icons .icon { font-size: 40px; line-height: 1; }
    .cw-icons .name { font-size: var(--cw-text-sm); }
    .cw-icons .app .new { position: absolute; top: 2px; left: calc(50% + 10px); margin: 0; }
    .cw-icons .cw-pin { position: absolute; top: 2px; right: 2px; }
    .cw-icons .off .icon, .cw-icons .off .name { opacity: .45; }
    .cw-icons .off button { font: inherit; font-size: var(--cw-text-xs); border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-accent);
      border-radius: var(--cw-radius-pill); padding: 1px 8px; cursor: pointer; }
    .cw-icons .empty { grid-column: 1 / -1; color: var(--cw-muted); font-size: var(--cw-text-sm); text-align: center; }
    .cw-desk-h { margin: 1.2em 0 .5em; font-size: 1rem; color: var(--cw-muted); text-align: center; }
    .cw-drawer { display: grid; gap: var(--cw-space-2); }
    .cw-drawer .how { justify-self: center; color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-drawer h4 { margin: var(--cw-space-2) 0 0; text-align: center; font-size: var(--cw-text-sm); color: var(--cw-muted); font-weight: 600; text-transform: uppercase; letter-spacing: .06em; }
}`;
  document.head.append(style);
  const el = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  // A tile: the app's link (or, not used yet, the dimmed app and its Add) and, beside it — never inside: the loader
  // takes every click on a `#` link — its pin.
  function tile({ app, href, count = 0, pin = null, add = null, note = null }) {
    const face = [el("span", { className: "icon", textContent: app.icon ?? "▫️" }), el("span", { className: "name", textContent: app.name })];
    // No link (`note`: why — "members", from outside): the app shown, dimmed, not opened.
    const body = add
      ? el("div", { className: "app off" }, ...face, el("button", { type: "button", textContent: "+ Add", onclick: add }))
      : !href
        ? el("div", { className: "app off", title: note ?? "" }, ...face, note ? el("small", { textContent: note }) : null)
        : el("a", { className: "app", href }, ...face, count ? el("span", { className: "cw-badge new", textContent: String(count) }) : null);
    return el("div", { className: "tile" }, body, pin);
  }
  function grid(host, items, empty = null) {
    host.classList.add("cw-icons");
    host.replaceChildren(...(items.length ? items.map(tile) : empty ? [el("p", { className: "empty", textContent: empty })] : []));
  }
  // THE DRAWER: by name (the loader's order), or grouped by category — categories by name, "Other" last.
  const GROUPED = "apps-grouped";
  async function drawer(host, items, { empty = null } = {}) {
    const prefs = await (await ctx.require("edge")).prefs().catch(() => null);
    const grouped = () => !!prefs?.get(GROUPED);
    const toggle = el("input", { type: "checkbox", checked: grouped() });
    const draw = () => {
      const body = [];
      if (!grouped()) {
        const g = el("div", {});
        grid(g, items, empty);
        body.push(g);
      } else {
        const cats = [...new Set(items.map(i => i.app.category ?? "Other"))].sort((a, b) => (a === "Other") - (b === "Other") || a.localeCompare(b));
        for (const c of cats) {
          const g = el("div", {});
          grid(g, items.filter(i => (i.app.category ?? "Other") === c));
          body.push(el("h4", { textContent: c }), g);
        }
      }
      host.replaceChildren(el("div", { className: "cw-drawer" }, el("label", { className: "how" }, toggle, " Group by category"), ...body));
    };
    toggle.onchange = async () => {
      await prefs?.set(GROUPED, toggle.checked);
      draw();
    };
    draw();
  }
  // THE DESK: Pinned (this person's pins of these apps) and All apps — pins and their buttons once they are read.
  function desk(host, { items, pinKey, empty = null }) {
    const pinned = el("div", {});
    const all = el("div", {});
    host.replaceChildren(el("h3", { className: "cw-desk-h", textContent: "Pinned" }), pinned, el("h3", { className: "cw-desk-h", textContent: "All apps" }), all);
    let pins = null;
    let pinUI = null;
    const redraw = () => {
      if (!host.isConnected) return;
      const on = new Set(pins?.refs("app:") ?? []);
      const tiles = items().map(t => ({ ...t, pin: pins && pinUI && t.href ? pinUI.button(pinKey(t.app), { className: "small" }) : null }));
      grid(pinned, tiles.filter(t => on.has(pinKey(t.app))), "Pin an app with 📌 to keep it here.");
      drawer(all, tiles, { empty });
    };
    redraw();
    Promise.all([ctx.require("edge").then(e => e.pins()), ctx.require("pin-button")]).then(
      ([t, ui]) => {
        pins = t;
        pinUI = ui;
        pins.onChange(redraw);
        redraw();
      },
      e => ctx.log("desk without pins", { what: e?.message ?? String(e) }),
    );
    return { redraw };
  }
  return { grid, tile, drawer, desk };
}
