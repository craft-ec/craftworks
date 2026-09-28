// HEADER: a top bar like a Mac's menu bar. On the left: ⌂ (home), the CURRENT APP's name, and that app's MENU — its
// sub-pages and actions; on the right: the app's ending actions, and Account. It changes with the app: on Home it reads
// "Home"; in an app, the app's name (from the manifest's apps) and what the app put under its route in `ctx.actions`:
// `{ label, href }` (a sub-page: a link; `on` when it is the one shown), `{ label, run }` (an action; `end`: at the
// right), or a search box. An app that changes them later says so with a `craftworks:actions` event. The app's own
// component (the layout names it), so editing it is publishing the app, never the loader.
export function mount(ctx, el) {
  el.innerHTML = `
    <style>
      .bar { display: flex; align-items: center; gap: 14px; border-bottom: 1px solid var(--cw-line); height: var(--cw-bar); box-sizing: border-box;
        font-size: var(--cw-text-sm); }
      .bar .home { text-decoration: none; font-size: 1.1rem; color: inherit; }
      .bar .name { font-weight: 600; }
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

    </style>
    <nav class="bar">
      <a class="home" href="#/" title="Home">⌂</a>
      <span class="name"></span>
      <span class="actions"></span>
      <span class="end"></span>
      <a class="account" href="#/account">Account</a>
    </nav>`;
  const draw = () => {
    const app = ctx.apps.find(a => a.route === ctx.route);
    el.querySelector(".name").textContent = ctx.route === "/" ? "Home" : ctx.route === "/account" ? "Account" : app?.name ?? "";
    const actions = el.querySelector(".actions");
    const all = ctx.actions[ctx.route] ?? [];
    const item = a => {
      if (a.href) {
        const l = Object.assign(document.createElement("a"), { href: a.href, textContent: a.label });
        if (a.on) l.setAttribute("aria-current", "page");
        return l;
      }
      return null;
    };
    el.querySelector(".end").replaceChildren(
      ...all.filter(a => a.end).map(a => Object.assign(document.createElement("button"), { type: "button", textContent: a.label, onclick: () => a.run() })),
    );
    actions.replaceChildren(
      ...all.filter(a => !a.end).map(a => {
        const link = item(a);
        if (link) return link;
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
  // ACTIVITY runs on every page (the header is on every page): new messages notify, whichever app is open. After the
  // page is up, never holding it.
  setTimeout(() => ctx.require("activity").catch(() => {}), 1500);
}
