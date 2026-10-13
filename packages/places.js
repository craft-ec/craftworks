// PLACES, a service (ARCHITECTURE §1, "The desktop"): WHERE an app can be — You (your personal space, where it has a
// personal view), each of your spaces that enables what the app needs (`roles.apps()`), and Discover (where it has a
// public view) — kept here, once. HOW an app shows them is the APP's own choice, made in its own code: a RAIL of spaces
// beside it (Discord's — Chat), a subtle GROUP line above it (Reddit's "in: …" — Board, Blog), or the app's own terms
// with no switcher at all: each app lists its groups (yours, each space enabling it) in its own menu (`where`), and
// Contact shows every space's people in one list. The loader draws none.
//
//   const places = await ctx.require("places");
//   await places.list("/chat")          // [{ id, label, href, on, count }]
//   places.rail(host) / places.group(host) / places.picker(host)   // drawn for the app open, kept current
export async function start(ctx) {
  const [space, roles] = await Promise.all([ctx.require("space"), ctx.require("roles")]);
  const activity = await ctx.require("activity").catch(() => null);
  const appOf = route => (ctx.apps ?? []).find(a => a.route === route);

  async function list(route = ctx.route) {
    const app = appOf(route);
    if (!app) return [];
    const key = app.route.slice(1);
    const views = app.views ?? [];
    const out = [];
    if (views.includes("personal")) out.push({ id: "", label: "You", short: "Me", href: `#${app.route}`, on: !ctx.space });
    if (views.includes("shared") && (await space.account().catch(ctx.quiet(() => null)))) {
      for (const sp of (await space.mine().catch(ctx.quiet(() => []))).filter(s => s.kind === "server")) {
        const r = await roles.of(sp).catch(ctx.quiet(() => null));
        if (!r || !r.apps().includes(key)) continue;
        out.push({ id: sp.id, label: space.shown(sp), short: space.shown(sp), href: `#/s/${sp.id}${app.route}`, on: ctx.space === sp.id, count: activity && ["chat", "board"].includes(key) ? activity.of(sp.id, key) : 0 });
      }
    }
    if (views.includes("public")) out.push({ id: "discover", label: "Discover", short: "🌐", href: `#/discover${app.route}`, on: ctx.space === "discover" });
    return out;
  }

  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false && k !== ""));
    return e;
  };
  const initials = s =>
    String(s)
      .split(/\s+/)
      .map(w => [...w][0] ?? "")
      .join("")
      .slice(0, 2)
      .toUpperCase() || "?";
  // Kept current: drawn again on every route, and when a space's apps change or someone logs in.
  // (ONE pair of listeners for every host drawn; a host no longer on the page is dropped.)
  const hosts = new Map(); // host → draw
  const redraw = () => {
    for (const [host, draw] of hosts) host.isConnected ? draw().catch(ctx.quiet(() => {})) : hosts.delete(host);
  };
  addEventListener("craftworks:route", redraw);
  addEventListener("craftworks:auth", redraw);
  const keep = (host, draw) => {
    hosts.set(host, draw);
    draw().catch(ctx.quiet(() => {}));
  };

  // A RAIL (Discord's): a column of the places, each a round mark; the one open lit.
  function rail(host) {
    host.classList.add("cw-rail");
    host.style.cssText = "display:flex;flex-direction:column;gap:8px;padding:8px 6px;width:56px;align-items:center;background:var(--cw-surface-2,#8881);";
    keep(host, async () => {
      const ps = await list();
      host.replaceChildren(
        ...ps.map(p => {
          const a = h("a", { href: p.href, title: p.label, textContent: p.id === "" ? "Me" : p.id === "discover" ? "🌐" : initials(p.label) });
          a.style.cssText = `display:grid;place-items:center;width:42px;height:42px;border-radius:${p.on ? "14px" : "50%"};background:${p.on ? "var(--cw-accent,#5865f2)" : "var(--cw-surface,#8882)"};color:${p.on ? "#fff" : "inherit"};text-decoration:none;font-weight:600;font-size:.85rem;position:relative`;
          if (p.count) a.append(h("span", { className: "cw-badge", textContent: String(p.count) }));
          return a;
        }),
        h("a", { href: "#/spaces", title: "Spaces: make or join one", textContent: "+", style: "display:grid;place-items:center;width:42px;height:42px;border-radius:50%;border:1px dashed #8886;text-decoration:none" }),
      );
    });
    return host;
  }

  // A GROUP line (Reddit's): subtle — "in: <place> ▾" above the content, a choice moving there.
  function group(host) {
    host.classList.add("cw-group");
    host.style.cssText = "font-size:.85rem;color:var(--cw-muted,#888);padding:4px 16px;";
    keep(host, async () => {
      const ps = await list();
      if (ps.length < 2) return void host.replaceChildren();
      const sel = h("select", { ariaLabel: "Where" }, ...ps.map(p => h("option", { value: p.href, textContent: p.label, selected: p.on })));
      sel.style.cssText = "border:0;background:transparent;color:inherit;font:inherit;font-weight:600;cursor:pointer";
      sel.onchange = () => (location.hash = sel.value);
      host.replaceChildren("in ", sel);
    });
    return host;
  }

  // A PICKER: the places as small links in a row.
  function picker(host) {
    host.classList.add("cw-picker");
    host.style.cssText = "display:flex;gap:10px;flex-wrap:wrap;font-size:.85rem;padding:4px 16px;";
    keep(host, async () => {
      const ps = await list();
      if (ps.length < 2) return void host.replaceChildren();
      host.replaceChildren(
        ...ps.map(p => {
          const a = h("a", { href: p.href, textContent: p.label });
          if (p.on) (a.setAttribute("aria-current", "page"), (a.style.fontWeight = "600"));
          return a;
        }),
      );
    });
    return host;
  }

  // ATTACHED to an app's element, in the app's chosen style ("group" or "picker"): kept first in it, put back whenever
  // the app draws itself anew (its element's contents replaced).
  function attach(el, style = "picker") {
    const host = document.createElement("div");
    ({ group, picker, rail }[style] ?? picker)(host);
    const put = () => el.isConnected && host.parentNode !== el && el.prepend(host);
    new MutationObserver(put).observe(el, { childList: true });
    put();
    return host;
  }

  return { list, rail, group, picker, attach };
}
