// SPACE HOME, a page (`#/s/<space id>`): a SHARED space's own page — its APPS (the site's apps with a shared view that
// this space uses), its MEMBERS, and, for who may, its APPS to add or remove and its SETTINGS (members and roles,
// invites, the moderation log, leaving: the one `server-settings`). The space is the one chosen in the spaces panel. UI only: which apps
// a space uses is `roles`' (an `app` act), its people `roles`' and `conversation`'s, names `directory`'s.
export async function mount(ctx, el) {
  const [space, roles, conversation, theme, icons] = await Promise.all(
    ["space", "roles", "conversation", "theme", "app-icons"].map(n => ctx.require(n)),
  );
  // What is new in each app (Chat's unread in this space): its pill, kept current.
  let activity = null;
  ctx.background("space-home: ctx.require", ctx.require("activity").then(a => ((activity = a), a.onChange(() => el.isConnected && ctx.route === "/space" && draw()))));
  const me = await space.me();
  el.innerHTML = `
    <style>@layer apps {
      .sh { display: grid; gap: var(--cw-space-4); }
      .sh button { font: inherit; cursor: pointer; }
      .sh .top { display: flex; align-items: center; gap: var(--cw-space-3); flex-wrap: wrap; }
      .sh .top h2 { margin: 0; font-size: 1.5rem; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .sh .top p { margin: 0; color: var(--cw-muted); font-size: var(--cw-text-sm); width: 100%; }
      .sh .btn { border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); border-radius: var(--cw-radius-sm); padding: 6px var(--cw-space-3); }
      .sh .btn.main { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
      .sh h3 { margin: 0 0 var(--cw-space-2); font-size: var(--cw-text-xs); letter-spacing: .08em; text-transform: uppercase; color: var(--cw-muted); }
      .sh a.app:hover { border-color: var(--cw-muted); }
      .sh ul { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: var(--cw-space-2); }
      .sh li { border: 1px solid var(--cw-line); border-radius: var(--cw-radius-pill); padding: 2px var(--cw-space-3); cursor: pointer; font-size: var(--cw-text-sm); }
      .sh li:hover { background: var(--cw-hover); }
      .sh li i { color: var(--cw-muted); font-style: normal; margin-left: 4px; }
      .sh .said { color: var(--cw-danger); font-size: var(--cw-text-sm); margin: 0; }
      .sh .none { color: var(--cw-muted); text-align: center; padding: var(--cw-space-5); }
    }</style>
    <div class="sh"></div>`;
  const root = el.querySelector(".sh");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  // The site's apps a shared space may use: those whose manifest entry has a shared view; their key is their route.
  const keyOf = a => a.route.slice(1);
  const sharedApps = ctx.apps.filter(a => (a.views ?? []).includes("shared"));

  // FROM OUTSIDE (a space this person is not in, listed in Discover): what it says it is, its members, its apps — each
  // public one opened from outside where its app reads a space from outside (Board), else after joining — and Join.
  async function outside() {
    const desc = await (await ctx.require("items")).publicSpace(ctx.space);
    if (!desc) return root.replaceChildren(h("p", { className: "none", textContent: "Not listed · join with a code" }));
    const pr = await roles.ofPublic(desc);
    await pr.settled;
    const on = pr.apps();
    const about = pr.config?.("space", "about", "") ?? "";
    const n = pr.members().length;
    const join = await (await ctx.require("join-button")).control(desc, { open: `#/s/${desc.id}`, joinable: pr.policy("", "join") === "anyone" });
    const appGrid = h("div", {});
    icons.grid(
      appGrid,
      // Each app opened where it is (`where` opens it from outside when the space lets anyone read it, else says it is
      // for members); Board reads a public space its own way (Discover's).
      sharedApps.filter(a => !a.always && on.includes(keyOf(a))).map(a => ({ app: a, href: keyOf(a) === "board" ? `#/discover/board/b/${desc.id}` : `#/s/${desc.id}/${keyOf(a)}` })),
      "No apps yet.",
    );
    root.replaceChildren(
      h("div", { className: "top" }, h("h2", { textContent: space.shown(desc) }), about ? h("p", { className: "about", textContent: String(about) }) : null, h("p", { textContent: `${n} member${n === 1 ? "" : "s"} · you are not in it` }), join),
      h("section", {}, h("h3", { textContent: "Apps" }), appGrid, h("p", { className: "none", textContent: "An app the space lets anyone read opens from here; the rest once you are in." })),
    );
  }

  async function draw() {
    // (#/discover alone: Discover is each app's tab in your personal space — your Home.)
    if (ctx.space === "discover") return location.replace("#/");
    // (Found once the list is complete — never "not in it" while a device's list is still arriving.)
    const found = ctx.space ? await space.find(ctx.space) : null;
    const sp = found?.kind === "server" ? found : null;
    if (!sp) return outside();
    // A MEMBER's space Home is the SPACES app's page for it (ARCHITECTURE §1): what it enables, its people and settings.
    // Its apps are not reached from here — each app shows its spaces as its own groups.
    return location.replace(`#/spaces/${sp.id}`);
  }
  root.replaceChildren(theme.loading("Opening the space…"));
  await draw();
  // Its roles or apps changing (an act from anyone): drawn again.
  const sp0 = ctx.space && (await space.mine()).find(s => s.id === ctx.space);
  if (sp0) (await roles.of(sp0)).onChange(() => el.isConnected && ctx.route === "/space" && ctx.space === sp0.id && draw());
  // Welcomes and askers are `upkeep`'s (every page, every 30 s): asked once now.
  ctx.background("space-home: ctx.require", ctx.require("upkeep").then(u => u.tick()));
  // Drawn again only for another space (the route event right after mounting names this one).
  let drawnFor = ctx.space;
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/space" && ctx.space !== drawnFor && ((drawnFor = ctx.space), draw()));
}
