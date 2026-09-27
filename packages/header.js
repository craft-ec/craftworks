// HEADER: a top bar like a desktop's. On the left: ⌂ (home), the CURRENT APP's name, and that app's ACTIONS; on the
// right: Account. It changes with the app: on Home it reads "Home"; in an app, the app's name (from the manifest's
// apps) and the actions the app put under its route in `ctx.actions` ({ label, run }). An app that changes its
// actions later says so with a `craftworks:actions` event. The app's own component (the layout names it), so editing
// it is publishing the app, never the loader.
export function mount(ctx, el) {
  el.innerHTML = `
    <style>
      .bar { display: flex; align-items: center; gap: 14px; border-bottom: 1px solid #8884; padding-bottom: 8px; }
      .bar .home { text-decoration: none; font-size: 1.1rem; }
      .bar .name { font-weight: 600; }
      .bar .actions { display: flex; gap: 10px; align-items: center; }
      .bar .actions button { border: 0; background: none; padding: 2px 4px; cursor: pointer; font: inherit; }
      .bar .actions button:hover { background: #8882; border-radius: 4px; }
      .bar .actions button[aria-pressed="true"] { background: #8883; border-radius: 4px; }
      .bar .actions .search { font: inherit; padding: 4px 10px; border: 1px solid #8885; border-radius: 16px; width: 16em; }
      .bar .account { margin-left: auto; }
    </style>
    <nav class="bar">
      <a class="home" href="#/" title="Home">⌂</a>
      <span class="name"></span>
      <span class="actions"></span>
      <a class="account" href="#/account">Account</a>
    </nav>`;
  const draw = () => {
    const app = ctx.apps.find(a => a.route === ctx.route);
    el.querySelector(".name").textContent = ctx.route === "/" ? "Home" : ctx.route === "/account" ? "Account" : app?.name ?? "";
    const actions = el.querySelector(".actions");
    actions.replaceChildren(
      ...(ctx.actions[ctx.route] ?? []).map(a => {
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
}
