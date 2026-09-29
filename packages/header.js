// HEADER: a top bar like a Mac's menu bar. On the left: ⌂ (home), the CURRENT APP's name, and that app's MENU — its
// sub-pages and actions; on the right: the app's ending actions, and Account. It changes with the app: on Home it reads
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
        // A MENU: a button, and its dropdown of links.
        if (a.menu) {
          const list = Object.assign(document.createElement("div"), { className: "list", hidden: true });
          list.setAttribute("role", "menu");
          list.append(
            ...a.menu.map(
              // A link, or an action (a button: a `#` link is the loader's, never a click handler's).
              m => item(m) ?? Object.assign(document.createElement("button"), { type: "button", textContent: m.label, onclick: () => m.run() }),
            ),
          );
          const b = Object.assign(document.createElement("button"), { type: "button", textContent: `${a.label} ▾` });
          b.setAttribute("aria-haspopup", "menu");
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
          const wrap = Object.assign(document.createElement("span"), { className: "drop" });
          wrap.append(b, list);
          return wrap;
        }
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
