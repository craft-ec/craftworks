// HEADER: a top bar like a Mac's menu bar. On the left: ⌂ (home), the SPACE you are in — its name opens the spaces panel,
// every space you can go to (yours, your friends', those you follow) —, the CURRENT APP — a dropdown switching between the apps of that space
// (its Home, and the apps it uses; Personal: the apps with a personal view) — and
// that app's MENU — its sub-pages and actions; on the right: the app's ending actions, and Account. It changes with the app: on Home it reads
// "Home"; in an app, the app's name (from the manifest's apps) and what the app put under its route in `ctx.actions`:
// `{ label, href }` (a sub-page: a link; `on` when it is the one shown), `{ label, run }` (an action; `end`: at the
// right), a search box, or a MENU (`{ label, menu: [{ label, href, on } | { label, run }] }`: a button opening a dropdown, closed
// by a choice, a click outside or Escape). An app that changes them later says so with a `craftworks:actions` event. The app's own
// component (the layout names it), so editing it is publishing the app, never the loader.
export function mount(ctx, el) {
  el.innerHTML = `
    <style>
      .bar { display: flex; align-items: center; gap: 14px; border-bottom: 1px solid var(--cw-line); height: var(--cw-bar); box-sizing: border-box;
        font-size: var(--cw-text-sm); }
      .bar .home { text-decoration: none; font-size: 1.1rem; color: inherit; }
      .bar .space-name { border: 0; background: none; font: inherit; font-weight: 700; color: inherit; cursor: pointer; padding: 2px var(--cw-space-2);
        border-radius: var(--cw-radius-sm); max-width: 16em; min-width: 6em; flex: 0 1 auto; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      /* The app's own actions give way first (they scroll), never the space's name. */
      .bar .actions { min-width: 0; overflow-x: auto; scrollbar-width: none; }
      .bar .space-name:hover { background: var(--cw-hover); }
      .bar .space-name .cw-badge { margin-left: 4px; }
      .bar .name { font-weight: 600; }
      .bar .name .drop > button { border: 0; background: none; font-weight: 700; padding: 2px var(--cw-space-2); }
      .bar .name .drop > button:hover { background: var(--cw-hover); }
      .bar .actions { display: flex; gap: 10px; align-items: center; }
      .bar .actions button { border: 0; background: none; padding: 2px var(--cw-space-1); cursor: pointer; border-radius: var(--cw-radius-sm); }
      .bar .actions button:hover { background: var(--cw-hover); }
      .bar .actions button[aria-pressed="true"], .bar .actions a[aria-current="page"] { background: var(--cw-pressed); }
      .bar .actions a { color: inherit; text-decoration: none; padding: 2px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
      .bar .actions a:hover { background: var(--cw-hover); }
      .bar .end { display: flex; gap: 10px; margin-left: auto; }
      .bar .end button { border: 0; background: none; padding: 2px var(--cw-space-1); cursor: pointer; border-radius: var(--cw-radius-sm); }
      .bar .end button:hover { background: var(--cw-hover); }
      .bar .actions .search { padding: var(--cw-space-1) 10px; border-radius: var(--cw-radius-pill); width: 16em; }
      .bar .drop { position: relative; }
      .bar .drop > button { border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); padding: 2px var(--cw-space-2); font-weight: 600;
        max-width: 22em; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .bar .drop .list { position: absolute; top: calc(100% + 4px); left: 0; z-index: 60; min-width: 12em; display: grid; padding: var(--cw-space-1);
        background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); box-shadow: var(--cw-shadow-lg); }
      .bar .drop .list[hidden] { display: none; }
      .bar .drop .list a, .bar .drop .list button { padding: 6px var(--cw-space-3); text-align: left; font: inherit; color: inherit; }
      .bar .drop .list button { border-top: 1px solid var(--cw-line); border-radius: 0; margin-top: var(--cw-space-1); }
      .bar .drop .list a[aria-current="page"] { font-weight: 600; }

    </style>
    <nav class="bar">
      <a class="home" href="#/" title="Home">⌂</a>
      <button type="button" class="space-name" data-spaces title="Spaces: yours, your friends', following" hidden></button>
      <span class="name"></span>
      <span class="actions"></span>
      <span class="end"></span>
      <a class="account" href="#/account">Account</a>
    </nav>`;
  // A DROPDOWN: a button, and its list of links and actions (closed by a choice, a click outside or Escape).
  const dropdown = (label, entries, item) => {
    const list = Object.assign(document.createElement("div"), { className: "list", hidden: true });
    list.setAttribute("role", "menu");
    list.append(
      ...entries.map(
        // A link, or an action (a button: a `#` link is the loader's, never a click handler's).
        m => item(m) ?? Object.assign(document.createElement("button"), { type: "button", textContent: m.label, onclick: () => m.run() }),
      ),
    );
    const b = Object.assign(document.createElement("button"), { type: "button", textContent: `${label} ▾` });
    b.setAttribute("aria-haspopup", "menu");
    const wrap = Object.assign(document.createElement("span"), { className: "drop" });
    const close = () => {
      list.hidden = true;
      removeEventListener("click", outside, true);
      removeEventListener("keydown", esc);
    };
    const outside = e => !wrap.contains(e.target) && close();
    const esc = e => e.key === "Escape" && close();
    b.onclick = () => {
      if (!list.hidden) return close();
      list.hidden = false;
      addEventListener("click", outside, true);
      addEventListener("keydown", esc);
    };
    list.onclick = e => e.target.closest("a, button") && close();
    wrap.append(b, list);
    return wrap;
  };
  const link = a => {
    if (!a.href) return null;
    const l = Object.assign(document.createElement("a"), { href: a.href, textContent: a.label });
    if (a.on) l.setAttribute("aria-current", "page");
    return l;
  };

  // THE APP SWITCHER: the apps of the space open (a shared space: its Home and the apps it uses; Personal: Home and the
  // apps with a personal view). Logged out: "Home".
  let drawing = 0;
  const watched = new Set();
  async function switcher() {
    const n = ++drawing;
    const box = el.querySelector(".name");
    const app = ctx.apps.find(a => a.route === ctx.route);
    const here = ctx.route === "/" || ctx.route === "/space" ? "Home" : ctx.route === "/account" ? "Account" : (app?.name ?? "");
    // Asked quietly (never a login dialog: the header is on every page).
    const session = await ctx.require("auth").then(a => a.check()).catch(() => null);
    if (n !== drawing) return;
    if (!session) return box.replaceChildren(here);
    let entries;
    if (ctx.space === "discover")
      entries = [
        { label: "Discover · Home", href: "#/discover", on: ctx.route === "/space" },
        ...ctx.apps.filter(a => (a.views ?? []).includes("public")).map(a => ({ label: `${a.icon ?? ""} ${a.name}`, href: `#/discover${a.route}`, on: a.route === ctx.route })),
      ];
    else if (ctx.space) {
      const space = await ctx.require("space");
      const sp = (await space.mine()).find(s => s.id === ctx.space);
      const r = sp ? await (await ctx.require("roles")).of(sp) : null;
      const on = r ? r.apps() : [];
      // The space's apps changing (one added or removed): the switcher again.
      if (r && !watched.has(sp.id)) {
        watched.add(sp.id);
        r.onChange(() => ctx.space === sp.id && switcher());
      }
      entries = [
        { label: `${sp ? space.shown(sp) : "Space"} · Home`, href: `#/s/${ctx.space}`, on: ctx.route === "/space" },
        ...ctx.apps.filter(a => (a.views ?? []).includes("shared") && on.includes(a.route.slice(1))).map(a => ({ label: `${a.icon ?? ""} ${a.name}`, href: `#/s/${ctx.space}${a.route}`, on: a.route === ctx.route })),
      ];
    } else
      entries = [
        { label: "Home", href: "#/", on: ctx.route === "/" },
        ...ctx.apps.filter(a => (a.views ?? []).includes("personal")).map(a => ({ label: `${a.icon ?? ""} ${a.name}`, href: a.route, on: a.route === ctx.route })),
      ].map(e => ({ ...e, href: e.href.startsWith("#") ? e.href : `#${e.href}` }));
    if (n !== drawing) return;
    box.replaceChildren(dropdown(here, entries, link));
  }

  // THE SPACE you are in: its name (logged in); a click opens the spaces panel. What is unread in your other spaces on it.
  const spaceB = el.querySelector(".space-name");
  spaceB.onclick = () => ctx.require("spaces-panel").then(p => p.open());
  let naming = 0;
  async function spaceName() {
    const n = ++naming;
    const session = await ctx.require("auth").then(a => a.check()).catch(() => null);
    if (n !== naming) return;
    spaceB.hidden = !session;
    if (!session) return;
    const [panel, activity] = await Promise.all([ctx.require("spaces-panel"), ctx.require("activity").catch(() => null)]);
    const name = await panel.here();
    const elsewhere = activity ? (await (await ctx.require("space")).mine().catch(() => [])).filter(s => s.kind === "server" && !s.circle && s.id !== ctx.space).reduce((t, s) => t + (activity.of(s.id) ?? 0), 0) : 0;
    if (n !== naming) return;
    spaceB.replaceChildren(`${name} ▾`, ...(elsewhere ? [Object.assign(document.createElement("span"), { className: "cw-badge", textContent: String(elsewhere) })] : []));
    if (activity && !spaceB.dataset.watching) (spaceB.dataset.watching = "1"), activity.onChange(() => spaceName());
  }
  const draw = () => {
    spaceName();
    switcher();
    const actions = el.querySelector(".actions");
    const all = ctx.actions[ctx.route] ?? [];
    const item = link;
    el.querySelector(".end").replaceChildren(
      ...all.filter(a => a.end).map(a => Object.assign(document.createElement("button"), { type: "button", textContent: a.label, onclick: () => a.run() })),
    );
    actions.replaceChildren(
      ...all.filter(a => !a.end).map(a => {
        const link = item(a);
        if (link) return link;
        // A MENU: a button, and its dropdown.
        if (a.menu) return dropdown(a.label, a.menu, item);
        // A search box ({ search: fn, placeholder, value }) or a button ({ label, run, on }).
        if (a.search) {
          const input = document.createElement("input");
          input.type = "search";
          input.className = "search";
          input.placeholder = a.placeholder ?? "Search";
          input.value = a.value ?? "";
          input.oninput = () => a.search(input.value);
          return input;
        }
        const b = document.createElement("button");
        b.type = "button";
        b.textContent = a.label;
        if (a.on) b.setAttribute("aria-pressed", "true");
        b.onclick = () => a.run();
        return b;
      }),
    );
    el.querySelector(".account").style.fontWeight = ctx.route === "/account" ? "bold" : "normal";
  };
  draw();
  addEventListener("craftworks:route", draw);
  addEventListener("craftworks:actions", draw);
  addEventListener("craftworks:auth", () => (spaceName(), switcher()));
  // ACTIVITY runs on every page (the header is on every page): new messages notify, whichever app is open. After the
  // page is up, never holding it.
  setTimeout(() => ctx.require("activity").catch(() => {}), 1500);
  // UPKEEP on every page too: welcomes joined, askers let in (whichever page is open).
  setTimeout(() => ctx.require("upkeep").catch(() => {}), 2500);
  // FILE KEYS too: every file on the key its access calls for (a member removed, a board made private), re-keyed here.
  setTimeout(() => ctx.require("file-keys").catch(() => {}), 4000);
  // Keeping this person's data on the network (phase 4, Lifecycle): due tables, one at a time, in the background.
  setTimeout(() => ctx.require("keep").catch(() => {}), 5000);
  // VIDEOS still being made (renditions pending): made here in the background, whichever page is open.
  setTimeout(() => ctx.require("video-studio").catch(() => {}), 6000);
}
