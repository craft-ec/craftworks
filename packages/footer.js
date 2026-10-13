// THE DOCK (the layout's footer), as macOS's: Home, the apps PINNED (your Home's pins, `app:<route>`), then the apps
// OPEN this session — each a dot under it, and going back to it where you left it — and the APP VIEWER: the apps open,
// each where it was, and every app installed (Mission Control and Launchpad in one overlay; Esc or a click outside
// closes it). Switching apps is here; the header names the app.
// (A new version of the app reaches an open page by itself: the loader loads it once nothing would be lost.)
export function mount(ctx, el) {
  // Every open page keeps the network's clock going (`beacon`: it extends the beacon when it stalls) — where the site
  // has the token's packages at all.
  if (!ctx.shaOf?.("beacon")) return mountDock(ctx, el);
  ctx.require("beacon").catch(e => ctx.log("beacon", { what: e.message ?? String(e) }));
  // …and publishes the current epoch's statement if nobody has (`statement`: anyone may).
  ctx.require("statement").catch(e => ctx.log("statement", { what: e.message ?? String(e) }));
  ctx.require("keeper").catch(e => ctx.log("keeper", { what: e.message ?? String(e) }));
  // …and witnesses what its stake is drawn for (`witness`: its member key, through its identity delegate).
  ctx.require("witness").catch(e => ctx.log("witness", { what: e.message ?? String(e) }));
  mountDock(ctx, el);
}

function mountDock(ctx, el) {
  el.innerHTML = `<style>@layer components {
    .cw-dock-pad { height: 76px; }
    .cw-dock { position: fixed; left: 50%; bottom: 8px; transform: translateX(-50%); z-index: 2147482000; display: flex; align-items: flex-end; gap: 4px;
      padding: 6px 10px; border-radius: 18px; background: color-mix(in srgb, var(--cw-surface) 82%, transparent); backdrop-filter: blur(14px);
      border: 1px solid var(--cw-line); box-shadow: var(--cw-shadow-lg); max-width: calc(100vw - 24px); overflow-x: auto; }
    .cw-dock a, .cw-dock button { position: relative; display: grid; place-items: center; width: 46px; height: 46px; font-size: 30px; line-height: 1;
      text-decoration: none; border: 0; background: none; cursor: pointer; border-radius: 12px; transition: transform .12s; color: inherit; }
    .cw-dock a:hover, .cw-dock button:hover { transform: translateY(-4px) scale(1.12); }
    .cw-dock .open::after { content: ""; position: absolute; bottom: -3px; width: 4px; height: 4px; border-radius: 50%; background: var(--cw-fg); }
    .cw-dock .here { background: var(--cw-hover); }
    .cw-dock .sep { width: 1px; align-self: stretch; margin: 4px 4px; background: var(--cw-line); }
    .cw-dock .tip { position: absolute; bottom: 54px; padding: 2px 8px; border-radius: 6px; background: var(--cw-surface); border: 1px solid var(--cw-line);
      font-size: 12px; white-space: nowrap; pointer-events: none; opacity: 0; transition: opacity .1s; }
    .cw-dock a:hover .tip, .cw-dock button:hover .tip, .cw-dock a:focus-visible .tip { opacity: 1; }
    .cw-viewer { position: fixed; inset: 0; z-index: 2147482500; background: color-mix(in srgb, var(--cw-bg) 70%, transparent); backdrop-filter: blur(18px);
      overflow: auto; padding: 48px 24px 120px; }
    .cw-viewer h3 { max-width: 1000px; margin: 0 auto 12px; color: var(--cw-muted); font-size: var(--cw-text-sm); letter-spacing: .06em; }
    .cw-viewer .open { max-width: 1000px; margin: 0 auto 32px; display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 14px; }
    .cw-viewer .card { display: grid; gap: 6px; padding: 16px; border-radius: 14px; background: var(--cw-surface); border: 1px solid var(--cw-line);
      box-shadow: var(--cw-shadow); text-decoration: none; color: inherit; }
    .cw-viewer .card .ic { font-size: 36px; }
    .cw-viewer .card small { color: var(--cw-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cw-viewer .card.here { outline: 2px solid var(--cw-accent); }
    .cw-viewer .all { max-width: 1000px; margin: 0 auto; display: grid; grid-template-columns: repeat(auto-fill, minmax(96px, 1fr)); gap: 18px 8px; }
    .cw-viewer .all a { display: grid; justify-items: center; gap: 6px; text-decoration: none; color: inherit; font-size: 13px; }
    .cw-viewer .all a span:first-child { font-size: 44px; }
    .cw-viewer .none { max-width: 1000px; margin: 0 auto 32px; color: var(--cw-muted); }
  }</style><div class="cw-dock-pad"></div><nav class="cw-dock" aria-label="Dock"></nav>`;
  const dock = el.querySelector(".cw-dock");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const appOf = route => (ctx.apps ?? []).find(a => a.route === route);

  // THE APPS OPEN this session: each app's last place (its address), newest first.
  const open = new Map(); // route → { hash, at }
  const note = () => {
    const a = appOf(ctx.route);
    if (!a) return;
    open.delete(a.route);
    open.set(a.route, { hash: location.hash || `#${a.route}`, at: Date.now() });
  };
  const placeOf = hash => {
    const s = decodeURIComponent(String(hash).replace(/^#\/?/, ""));
    const m = /^s\/([0-9a-f]{8})/.exec(s);
    return m ? `in a space (${m[1]}…)` : s.includes("/p/") ? "an item" : s.split("/").slice(1).join(" / ") || "its start";
  };

  let pinned = new Set();
  let installed = null;
  const icon = (a, { href, here, isOpen, label }) =>
    h("a", { href, className: `${isOpen ? "open" : ""} ${here ? "here" : ""}`.trim(), ariaLabel: label ?? a.name, textContent: a.icon ?? "▫️" }, h("span", { className: "tip", textContent: label ?? a.name }));

  function draw() {
    const here = ctx.route;
    const pins = (ctx.apps ?? []).filter(a => pinned.has(a.route));
    const recent = [...open.keys()].reverse().map(appOf).filter(a => a && !pinned.has(a.route)).slice(0, 8);
    dock.replaceChildren(
      icon({ icon: "🏠", name: "Home" }, { href: "#/", here: here === "/", label: "Home" }),
      ...(pins.length ? [h("span", { className: "sep" })] : []),
      ...pins.map(a => icon(a, { href: open.get(a.route)?.hash ?? `#${a.route}`, here: here === a.route, isOpen: open.has(a.route) })),
      ...(recent.length ? [h("span", { className: "sep" })] : []),
      ...recent.map(a => icon(a, { href: open.get(a.route).hash, here: here === a.route, isOpen: true })),
      h("span", { className: "sep" }),
      h("button", { type: "button", ariaLabel: "Open apps and all apps", textContent: "▦", onclick: () => viewer() }, h("span", { className: "tip", textContent: "Apps" })),
    );
  }

  // THE APP VIEWER: the apps open (where each was), then every app installed. Drawn AT ONCE from the apps this page
  // knows; the installed list filled in when it answers (audit 10-13: a slow node's read held the overlay for minutes,
  // and each click stacked another). One at a time: opening it again closes the one before.
  let closeViewer = null;
  function viewer() {
    closeViewer?.();
    const v = h("div", { className: "cw-viewer", role: "dialog", ariaLabel: "Apps" });
    const close = () => (v.remove(), removeEventListener("keydown", onKey), removeEventListener("hashchange", close), closeViewer === close && (closeViewer = null));
    const onKey = e => e.key === "Escape" && close();
    addEventListener("keydown", onKey);
    // (A choice moves the page — the loader takes `#` links before this overlay sees the click: closed on the move.)
    addEventListener("hashchange", close);
    closeViewer = close;
    v.onclick = e => (e.target === v || e.target.closest("a")) && close();
    const openList = [...open.entries()].reverse().map(([route, o]) => [appOf(route), o]).filter(([a]) => a);
    const grid = h("div", { className: "all" });
    const fill = all => grid.replaceChildren(...[...all].sort((x, y) => x.name.localeCompare(y.name)).map(a => h("a", { href: open.get(a.route)?.hash ?? `#${a.route}` }, h("span", { textContent: a.icon ?? "▫️" }), h("span", { textContent: a.name }))));
    fill(ctx.apps ?? []);
    v.append(
      h("h3", { textContent: "OPEN" }),
      openList.length
        ? h("div", { className: "open" }, ...openList.map(([a, o]) => h("a", { href: o.hash, className: `card ${a.route === ctx.route ? "here" : ""}` }, h("span", { className: "ic", textContent: a.icon ?? "▫️" }), h("b", { textContent: a.name }), h("small", { textContent: placeOf(o.hash) }))))
        : h("p", { className: "none", textContent: "No app open yet." }),
      h("h3", { textContent: "ALL APPS" }),
      grid,
    );
    document.body.append(v);
    if (installed) ctx.optional("footer: installed.list", installed.list(), () => null).then(all => all?.length && v.isConnected && fill(all));
  }

  addEventListener("craftworks:route", () => (note(), draw()));
  addEventListener("craftworks:installed", () => draw());
  note();
  draw();
  // Your Home's pins (the account's: the same on every device).
  ctx.background("footer: edge", ctx
    .require("edge")
    .then(async edge => {
      const p = await edge.pins();
      const read = () => ((pinned = new Set((ctx.apps ?? []).map(a => a.route).filter(r => p.has(`app:${r}`)))), draw());
      p.onChange(read);
      read();
    }));
  ctx.background("footer: installed", ctx.require("installed").then(i => (installed = i)));
}
