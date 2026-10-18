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
  const activity = await ctx.optional("places: ctx.require", ctx.require("activity"), () => null);
  const appOf = route => (ctx.apps ?? []).find(a => a.route === route);

  async function list(route = ctx.route) {
    const app = appOf(route);
    if (!app) return [];
    const key = app.route.slice(1);
    const views = app.views ?? [];
    const out = [];
    if (views.includes("personal")) out.push({ id: "", label: "You", short: "Me", href: `#${app.route}`, on: !ctx.space });
    if (views.includes("shared") && (await space.account())) {
      for (const sp of (await space.mine()).filter(s => s.kind === "server")) {
        const r = await roles.of(sp);
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
    for (const [host, draw] of hosts) host.isConnected ? ctx.background("places: draw", draw()) : hosts.delete(host);
  };
  addEventListener("craftworks:route", redraw);
  addEventListener("craftworks:route", () => dispatchEvent(new CustomEvent("craftworks:data"))); // (the side panel's mark moves)
  addEventListener("craftworks:auth", redraw);
  const keep = (host, draw) => {
    hosts.set(host, draw);
    ctx.background("places: draw", draw());
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

  // THE SIDE PANEL — THE APP's ONE NAVIGATION (owner 10-15: "remove the top navigation and maintain only side
  // navigation and load discover by default. also separate the spaces with you/discover into its own section. show join /
  // follow"). Three sections, a VIEW (`ctx.view`: drawn when read, again when the app's entries, its places or the
  // spaces change):
  //   the APP — its own entries (`ctx.actions`, what `where.tabs` gives every app: Discover first, then Following /
  //     Yours / Saved / its extras, ＋ its create), else You and Discover;
  //   THIS SPACE — a space seen from outside: Join or Request (`join-button`) and Follow;
  //   SPACES — each of yours that has the app (its unread count), ＋ Join or make a space.
  // Search boxes stay in the header (a tool, not a place). Nothing when the app has one place and no entries.
  function side(host) {
    host.classList.add("cw-side");
    const entry = a => ({ label: a.label, href: a.href ?? null, on: !!a.on, end: !!a.end, role: a.role ?? (a.end ? "create" : "you"), menu: a.menu ? a.menu.map(m => ({ label: m.label, href: m.href ?? null, on: !!m.on })) : null, run: !!a.run && !a.href });
    let more = false; // Following's list shown whole ("Show more")
    let runs = new Map(); // label → the entry's own run (a button that is not a link)
    const v = ctx.view(host, {
      read: async () => {
        const ps = await list();
        const acts = ctx.actions?.[ctx.route] ?? [];
        runs = new Map(acts.filter(a => a.run && !a.href).map(a => [a.label, a.run]));
        const own = acts.map(entry);
        const app = own.length ? own : ps.filter(p => p.id === "" || p.id === "discover").map(p => ({ label: p.label, href: p.href, on: p.on, role: p.id === "discover" ? "discover" : "you" }));
        // MAIN (YouTube's Home · Subscriptions): Discover, then the feed — and under it WHO you follow (people and spaces).
        const main = [...app.filter(a => a.role === "discover"), ...app.filter(a => a.role === "feed")];
        const feed = app.find(a => a.role === "feed");
        let follows = [];
        if (feed) {
          const people = await ctx.optional("places: follows", (await ctx.require("edge")).people(), () => null);
          const directory = await ctx.require("directory");
          const route = appOf(ctx.route)?.route ?? ctx.route;
          follows = await Promise.all((people?.list("follow") ?? []).map(async id => {
            const did = String(id).startsWith("did:");
            const name = did ? directory.shown(id, await ctx.optional("places: handle", directory.handle(id), () => null)) : people.about?.("follow", id)?.name ?? `a space #${String(id).slice(0, 6)}`;
            return { label: name, href: did ? `#${route}/u/${id}` : `#/s/${id}${route}`, on: false };
          }));
        }
        // YOU: yours, saved, the app's own; its create last.
        const you = [...app.filter(a => a.role === "you"), ...app.filter(a => a.role === "create")];
        const spaces = ps.filter(p => p.id && p.id !== "discover").map(p => ({ label: p.label, href: p.href, on: p.on, count: p.count ?? 0 }));
        // A SPACE SEEN FROM OUTSIDE (in it nowhere: Discover's, a link): Join / Follow there.
        const here = ctx.space && ctx.space !== "discover" && !spaces.some(p => p.on) ? ctx.space : null;
        return app.length + spaces.length + (here ? 1 : 0) < 2 && !here ? [] : { main, follows, more, you, spaces, here };
      },
      empty: d => Array.isArray(d) && !d.length,
      draw: async ({ main, follows, more: whole, you, spaces, here }) => {
        const link = (x, cls = "") => {
          const a = x.href ? h("a", { href: x.href, className: `${cls}${x.on ? " on" : ""}`.trim(), textContent: x.label }) : h("button", { type: "button", className: `cw-flat ${cls}`.trim(), textContent: x.label, onclick: () => runs.get(x.label)?.() });
          if (x.on) a.setAttribute("aria-current", "page");
          if (x.count) a.append(h("span", { className: "cw-badge", textContent: String(x.count) }));
          return a;
        };
        const items = xs => h("ul", {}, ...xs.flatMap(x => (x.menu ? [h("li", { className: "group" }, x.label), ...x.menu.map(m => h("li", {}, link(m, "sub")))] : [h("li", {}, link(x, x.end ? "make" : ""))])));
        const FEW = 7;
        const followed = follows.length
          ? [
              h("ul", { className: "follows" }, ...(whole ? follows : follows.slice(0, FEW)).map(f => h("li", {}, link(f, "sub")))),
              follows.length > FEW ? h("button", { type: "button", className: "cw-flat more", textContent: whole ? "Show less" : `Show more (${follows.length - FEW})`, onclick: () => ((more = !more), v.refresh()) }) : null,
            ]
          : [];
        const sections = [];
        if (main.length) sections.push(h("section", { ariaLabel: "Main" }, items(main), ...followed));
        if (you.length) sections.push(h("section", { ariaLabel: "You" }, h("h4", { textContent: "You" }), items(you)));
        if (here) {
          const join = await ctx.optional("places: join", (async () => {
            const items = await ctx.require("items");
            const d = await items.publicSpace(here);
            if (!d) return null;
            const pr = await (await ctx.require("roles")).ofPublic(d);
            await pr.settled;
            const ctl = await (await ctx.require("join-button")).control(d, { open: `#/spaces/${d.id}`, policy: pr.policy("", "join") });
            return h("section", { ariaLabel: "This space" }, h("h4", { textContent: d.name ?? "This space" }), ctl);
          })(), null);
          if (join) sections.push(join);
        }
        sections.push(h("section", { ariaLabel: "Spaces" }, h("h4", { textContent: "Spaces" }), items(spaces), h("a", { className: "more", href: "#/spaces", textContent: "＋ Join or make a space" })));
        return h("nav", { ariaLabel: "Places" }, ...sections);
      },
      looks: { empty: () => document.createTextNode(""), loading: () => document.createTextNode(""), failed: () => document.createTextNode("") },
    });
    addEventListener("craftworks:actions", () => (host.isConnected ? v.refresh() : null));
    return v;
  }

  return { list, rail, group, picker, attach, side };
}
