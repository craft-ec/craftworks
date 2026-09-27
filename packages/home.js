// HOME: public and private in one page.
// - PUBLIC (nobody logged in): a welcome, and Log in / Register, which open auth's dialog.
// - PRIVATE (logged in): the DESKTOP: the site's apps as icons, the ones this account pinned first. Pins are a table of
//   the account's (the `data` service), so they are the same on every node of the account.
// It asks auth quietly (`check`, never a dialog) which view to show, and switches when someone logs in or out.
export async function mount(ctx, el) {
  const auth = await ctx.require("auth");

  const publicView = () => {
    el.innerHTML = `<h2>Welcome to Craftworks</h2>
      <p>Apps that live on freenet: no server, no company holding your data. Your account is yours, named by twelve
      recovery words, and your data belongs to your account, on whichever of your nodes you use.</p>
      <p><button type="button" data-tab="login">Log in</button> <button type="button" data-tab="register">Register</button></p>`;
    for (const b of el.querySelectorAll("[data-tab]")) b.onclick = () => auth.session({ tab: b.dataset.tab });
  };

  const desktop = async () => {
    el.innerHTML = `
      <style>
        .desk h3 { margin: 1.2em 0 .5em; font-size: 1rem; opacity: .75; }
        .desk .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(96px, 1fr)); gap: 12px; }
        .desk .tile { position: relative; }
        .desk .app { display: grid; justify-items: center; gap: 6px; padding: 12px 6px;
          border-radius: 12px; text-decoration: none; color: inherit; }
        .desk .app:hover { background: #8881; }
        .desk .icon { font-size: 40px; line-height: 1; }
        .desk .pin { position: absolute; top: 2px; right: 2px; border: 0; background: none; cursor: pointer;
          font-size: 14px; opacity: .35; }
        .desk .pin[aria-pressed="true"] { opacity: 1; }
        .desk .empty { opacity: .6; font-size: .9em; }
      </style>
      <div class="desk">
        <h3>Pinned</h3><div class="grid pinned"></div>
        <h3>All apps</h3><div class="grid all"></div>
      </div>`;
    // The icons show at once; the pins arrive when the account's desktop table has been read (a table the account
    // never wrote takes the network a while to report absent).
    let pins = null;
    const pinned = () => new Set((pins?.rows() ?? []).map(r => r.key));
    // A tile is the app's link and, beside it (never inside: the loader takes every click on a `#` link), its pin.
    const icon = (a, on) => {
      const tile = document.createElement("div");
      tile.className = "tile";
      const link = document.createElement("a");
      link.className = "app";
      link.href = `#${a.route}`;
      link.innerHTML = `<span class="icon"></span><span class="name"></span>`;
      link.querySelector(".icon").textContent = a.icon ?? "▫️";
      link.querySelector(".name").textContent = a.name;
      tile.append(link);
      if (pins) {
        const pin = document.createElement("button");
        pin.type = "button";
        pin.className = "pin";
        pin.textContent = "📌";
        pin.title = on ? "Unpin" : "Pin";
        pin.setAttribute("aria-pressed", String(on));
        pin.onclick = () =>
          (on ? pins.remove(a.route) : pins.put(a.route, "pinned")).catch(err => ctx.log("pin failed", { what: err?.message ?? String(err) }));
        tile.append(pin);
      }
      return tile;
    };
    const render = () => {
      if (!el.isConnected) return;
      const on = pinned();
      const [pinnedGrid, allGrid] = [el.querySelector(".pinned"), el.querySelector(".all")];
      const mine = ctx.apps.filter(a => on.has(a.route));
      pinnedGrid.replaceChildren(...mine.map(a => icon(a, true)));
      if (!mine.length) pinnedGrid.append(Object.assign(document.createElement("p"), { className: "empty", textContent: "Pin an app with 📌 to keep it here." }));
      allGrid.replaceChildren(...ctx.apps.map(a => icon(a, on.has(a.route))));
    };
    render();
    (await ctx.require("data")).table("desktop").then(
      t => {
        pins = t;
        pins.onChange(render);
        render();
      },
      e => ctx.log("desktop without pins", { what: e?.message ?? String(e) }),
    );
  };

  const show = s => (s ? desktop() : publicView());
  const onAuth = e => (el.isConnected ? show(e.detail) : removeEventListener("craftworks:auth", onAuth));
  addEventListener("craftworks:auth", onAuth);
  await show(await auth.check().catch(() => null));
}
