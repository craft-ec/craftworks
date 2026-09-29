// HOME: public and private in one page.
// - PUBLIC (nobody logged in): a welcome, and Log in / Register, which open auth's dialog.
// - PRIVATE (logged in): the DESKTOP: the site's apps as icons, the ones this account pinned first. Pins are the
//   account's (the `edge` capability's pins, shown with the one `pin-button`), so they are the same on every node of the account.
// It asks auth quietly (`check`, never a dialog) which view to show, and switches when someone logs in or out.
export async function mount(ctx, el) {
  const [auth, login] = await Promise.all([ctx.require("auth"), ctx.require("login")]);

  // The WELCOME: what Craftworks is, for someone who has not logged in (or never has).
  const publicView = () => {
    el.innerHTML = `
      <style>
        .welcome { display: grid; gap: var(--cw-space-5); padding: var(--cw-space-5) 0; }
        .welcome .hero { display: grid; gap: var(--cw-space-2); }
        .welcome .tag { margin: 0; color: var(--cw-accent); font-size: var(--cw-text-sm); letter-spacing: .12em; text-transform: uppercase; font-weight: 600; }
        .welcome h2 { margin: 0; font-size: clamp(1.8rem, 5vw, 2.6rem); line-height: 1.1; }
        .welcome .lead { margin: 0; font-size: 1.1rem; color: var(--cw-muted); max-width: 60ch; }
        .welcome .go { display: flex; gap: var(--cw-space-2); flex-wrap: wrap; }
        .welcome .go button { border: 1px solid var(--cw-line); background: var(--cw-surface); border-radius: var(--cw-radius-sm);
          padding: var(--cw-space-2) var(--cw-space-4); cursor: pointer; }
        .welcome .go button.main { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
        .welcome .apps { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: var(--cw-space-3); }
        .welcome .app { border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); background: var(--cw-surface); }
        .welcome .app b { display: block; margin-bottom: var(--cw-space-1); }
        .welcome .app p, .welcome li { margin: 0; color: var(--cw-muted); font-size: var(--cw-text-sm); }
        .welcome h3 { margin: 0 0 var(--cw-space-2); font-size: 1rem; }
        .welcome ul { margin: 0; padding-left: 1.2em; display: grid; gap: var(--cw-space-1); }
      </style>
      <div class="welcome">
        <div class="hero">
          <p class="tag">Craftec · Craft The Future</p>
          <h2>Craftworks</h2>
          <p class="lead">One stop centre for everything Freenet. Notes, messages, chat and mail that live on the Freenet network
            itself: no server in between, no company holding your data.</p>
          <div class="go"><button type="button" class="main" data-tab="register">Create an account</button>
            <button type="button" data-tab="login">Log in</button></div>
        </div>
        <div>
          <h3>What is here</h3>
          <div class="apps"></div>
        </div>
        <div>
          <h3>How it works</h3>
          <ul>
            <li>Your account is yours: twelve recovery words make it, and bring it back on any machine.</li>
            <li>Your data is sealed for your account and kept on Freenet, on whichever of your nodes you use.</li>
            <li>Every part of this app is fetched from Freenet and checked by its hash before it runs.</li>
            <li>You are shown as <code>name#abc123</code>. Someone new reaches you by your full id (on your Card page), never by a phone number or email.</li>
          </ul>
        </div>
      </div>`;
    // The site's own apps, as its manifest describes them.
    el.querySelector(".apps").replaceChildren(
      ...ctx.apps.map(a => {
        const card = document.createElement("div");
        card.className = "app";
        card.innerHTML = "<b></b><p></p>";
        card.querySelector("b").textContent = `${a.icon ?? ""} ${a.name}`;
        card.querySelector("p").textContent = a.about ?? "";
        return card;
      }),
    );
    for (const b of el.querySelectorAll("[data-tab]")) b.onclick = () => login.session({ tab: b.dataset.tab });
  };

  const desktop = async () => {
    el.innerHTML = `
      <style>
        .desk h3 { margin: 1.2em 0 .5em; font-size: 1rem; color: var(--cw-muted); text-align: center; }
        .desk .grid { display: grid; grid-template-columns: repeat(auto-fit, 96px); justify-content: center; gap: var(--cw-space-3); }
        .desk .tile { position: relative; }
        .desk .app { display: grid; justify-items: center; gap: 6px; padding: var(--cw-space-3) 6px;
          border-radius: var(--cw-radius); text-decoration: none; color: inherit; }
        .desk .app:hover { background: var(--cw-hover); }
        .desk .icon { font-size: 40px; line-height: 1; }
        .desk .cw-pin { position: absolute; top: 2px; right: 2px; font-size: 14px; }
        .desk .app { position: relative; }
        .desk .app .new { position: absolute; top: 2px; left: calc(50% + 10px); margin: 0; }
        .desk .empty { grid-column: 1 / -1; color: var(--cw-muted); font-size: var(--cw-text-sm); text-align: center; }
      </style>
      <div class="desk">
        <h3>Pinned</h3><div class="grid pinned"></div>
        <h3>All apps</h3><div class="grid all"></div>
      </div>`;
    // The icons show at once; the pins arrive when the account's desktop table has been read (a table the account
    // never wrote takes the network a while to report absent).
    let pins = null;
    let pinUI = null;
    let activity = null;
    // An app's pin is the account's pin of `app:<route>`: the same pins every app uses.
    const pinned = () => new Set((pins?.refs("app:") ?? []).map(r => r.slice(4)));
    // A tile is the app's link and, beside it (never inside: the loader takes every click on a `#` link), its pin.
    const icon = a => {
      const tile = document.createElement("div");
      tile.className = "tile";
      const link = document.createElement("a");
      link.className = "app";
      link.href = `#${a.route}`;
      link.innerHTML = `<span class="icon"></span><span class="name"></span>`;
      link.querySelector(".icon").textContent = a.icon ?? "▫️";
      link.querySelector(".name").textContent = a.name;
      // What is new in it (an app that counts: the manifest says what — `activity`'s totals).
      const n = a.counts && activity ? activity.total(a.counts) : 0;
      if (n) link.append(Object.assign(document.createElement("span"), { className: "cw-badge new", textContent: String(n) }));
      tile.append(link);
      if (pins) tile.append(pinUI.button(`app:${a.route}`));
      return tile;
    };
    const render = () => {
      if (!el.isConnected) return;
      const on = pinned();
      const [pinnedGrid, allGrid] = [el.querySelector(".pinned"), el.querySelector(".all")];
      // The PERSONAL space's apps: those with a personal view (a shared space's are on its own Home).
      const personal = ctx.apps.filter(a => (a.views ?? ["personal"]).includes("personal"));
      const mine = personal.filter(a => on.has(a.route));
      pinnedGrid.replaceChildren(...mine.map(icon));
      if (!mine.length) pinnedGrid.append(Object.assign(document.createElement("p"), { className: "empty", textContent: "Pin an app with 📌 to keep it here." }));
      allGrid.replaceChildren(...personal.map(icon));
    };
    render();
    Promise.all([ctx.require("edge").then(e => e.pins()), ctx.require("pin-button")]).then(
      ([t, ui]) => {
        pins = t;
        pinUI = ui;
        pins.onChange(render);
        render();
      },
      e => ctx.log("desktop without pins", { what: e?.message ?? String(e) }),
    );
    // What is new in each app: its badge, kept current.
    ctx.require("activity").then(a => {
      activity = a;
      a.onChange(render);
      render();
    }, () => {});
  };

  const show = s => (s ? desktop() : publicView());
  const onAuth = e => (el.isConnected ? show(e.detail) : removeEventListener("craftworks:auth", onAuth));
  addEventListener("craftworks:auth", onAuth);
  await show(await auth.check().catch(() => null));
}
