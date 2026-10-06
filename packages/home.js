// HOME: public and private in one page.
// - PUBLIC (nobody logged in): a welcome, and Log in / Register, which open auth's dialog.
// - PRIVATE (logged in): the DESKTOP: the site's apps as icons, the ones this account pinned first. Pins are the
//   account's (the `edge` capability's pins, shown with the one `pin-button`), so they are the same on every node of the account.
// - A PERSON's HOME (`#/u/<did>`: someone else's personal space, from the spaces panel): their name, what you can do
//   with them (the one `person` menu), and their apps that show someone's space (the manifest's `person` view).
// It asks auth quietly (`check`, never a dialog) which view to show, and switches when someone logs in or out.
export async function mount(ctx, el) {
  const [auth, login] = await Promise.all([ctx.require("auth"), ctx.require("login")]);

  // The WELCOME: what Craftworks is, for someone who has not logged in (or never has).
  const publicView = () => {
    el.innerHTML = `
      <style>@layer apps {
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
      }</style>
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
      <style>@layer apps {
        .desk h3 { margin: 1.2em 0 .5em; font-size: 1rem; color: var(--cw-muted); text-align: center; }
        .desk .grid { display: grid; grid-template-columns: repeat(auto-fit, 96px); justify-content: center; gap: var(--cw-space-3); }
        .desk .tile { position: relative; }
        .desk .app { display: grid; justify-items: center; gap: 6px; padding: var(--cw-space-3) 6px;
          border-radius: var(--cw-radius); text-decoration: none; color: inherit; }
        .desk .app:hover { background: var(--cw-hover); }
        .desk .icon { font-size: 40px; line-height: 1; }
        .desk .cw-pin { position: absolute; top: 2px; right: 2px; }
        .desk .app { position: relative; }
        .desk .app .new { position: absolute; top: 2px; left: calc(50% + 10px); margin: 0; }
        .desk .empty { grid-column: 1 / -1; color: var(--cw-muted); font-size: var(--cw-text-sm); text-align: center; }
      }</style>
      <div class="desk"></div>`;
    // THE DESK (`app-icons`: the same layout as a space's Home): the PERSONAL space's apps (those with a personal view;
    // a shared space's are on its own Home), each with its pill (what is new: `activity`) and its pin (`app:<route>`).
    let activity = null;
    const icons = await ctx.require("app-icons");
    const personal = ctx.apps.filter(a => (a.views ?? ["personal"]).includes("personal"));
    const desk = icons.desk(el.querySelector(".desk"), {
      items: () => personal.map(a => ({ app: a, href: `#${a.route}`, count: a.counts && activity ? activity.total(a.counts) : 0 })),
      pinKey: a => `app:${a.route}`,
    });
    // What is new in each app: its badge, kept current.
    ctx.require("activity").then(a => {
      activity = a;
      a.onChange(desk.redraw);
      desk.redraw();
    }, () => {});
  };

  // A PERSON's Home: theirs, seen from outside.
  const personHome = async did => {
    const [directory, person, icons] = await Promise.all(["directory", "person", "app-icons"].map(n => ctx.require(n)));
    el.innerHTML = `
      <style>@layer apps {
        .them { display: grid; gap: var(--cw-space-3); justify-items: center; padding: var(--cw-space-5) 0; }
        .them h2 { margin: 0; font-size: 1.6rem; overflow-wrap: anywhere; text-align: center; }
        .them .did { margin: 0; color: var(--cw-muted); font-size: var(--cw-text-xs); overflow-wrap: anywhere; text-align: center; }
        .them .act { font: inherit; border: 1px solid var(--cw-line); background: var(--cw-surface); color: inherit; border-radius: var(--cw-radius-pill);
          padding: 6px var(--cw-space-4); cursor: pointer; }
        .them .grid { display: grid; grid-template-columns: repeat(auto-fit, 96px); justify-content: center; gap: var(--cw-space-3); width: 100%; }
        .them .app { display: grid; justify-items: center; gap: 6px; padding: var(--cw-space-3) 6px; border-radius: var(--cw-radius); text-decoration: none; color: inherit; }
        .them .app:hover { background: var(--cw-hover); }
        .them .icon { font-size: 40px; line-height: 1; }
      }</style>
      <div class="them"><h2></h2><p class="did"></p><button type="button" class="act">Follow, friend, message…</button><div class="grid"></div></div>`;
    const name = el.querySelector("h2");
    name.textContent = directory.shown(did);
    directory.handle(did).then(n => (name.textContent = directory.shown(did, n)), () => {});
    el.querySelector(".did").textContent = did;
    el.querySelector(".act").onclick = e => person.open(e.currentTarget, did);
    icons.grid(el.querySelector(".grid"), ctx.apps.filter(a => (a.views ?? []).includes("person")).map(a => ({ app: a, href: `#${a.route}/u/${did}` })));
  };
  const show = async s => {
    if (!s) return publicView();
    const who = (await ctx.require("where")).personOf();
    return who && who !== s.did ? personHome(who) : desktop();
  };
  // Home again at another address (yours ↔ a person's): drawn for it.
  let drawnFor = ctx.sub ?? "";
  const onRoute = () => {
    if (!el.isConnected) return removeEventListener("craftworks:route", onRoute);
    if (ctx.route !== "/" || (ctx.sub ?? "") === drawnFor) return;
    drawnFor = ctx.sub ?? "";
    auth.check().then(show, () => {});
  };
  addEventListener("craftworks:route", onRoute);
  const onAuth = e => (el.isConnected ? show(e.detail) : removeEventListener("craftworks:auth", onAuth));
  addEventListener("craftworks:auth", onAuth);
  await show(await auth.check().catch(() => null));
}
