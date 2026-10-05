// SPACE HOME, a page (`#/s/<space id>`): a SHARED space's own page — its APPS (the site's apps with a shared view that
// this space uses), its MEMBERS, and, for who may, its APPS to add or remove and its SETTINGS (members and roles,
// invites, the moderation log, leaving: the one `server-settings`). The space is the one chosen in the spaces panel. UI only: which apps
// a space uses is `roles`' (an `app` act), its people `roles`' and `conversation`'s, names `directory`'s.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [space, roles, conversation, theme, icons] = await Promise.all(
    ["space", "roles", "conversation", "theme", "app-icons"].map(n => ctx.require(n)),
  );
  // What is new in each app (Chat's unread in this space): its pill, kept current.
  let activity = null;
  ctx.require("activity").then(a => ((activity = a), a.onChange(() => el.isConnected && ctx.route === "/space" && draw())), () => {});
  const me = (await space.account()).id;
  el.innerHTML = `
    <style>
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
    </style>
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
    if (!desc) return root.replaceChildren(h("p", { className: "none", textContent: "This space is not listed (private, or not found yet): join it with an invite code." }));
    const pr = await roles.ofPublic(desc);
    await pr.settled;
    const on = pr.apps();
    const about = pr.config?.("space", "about", "") ?? "";
    const n = pr.members().length;
    const join = await (await ctx.require("join-button")).control(desc, { open: `#/s/${desc.id}`, joinable: pr.policy("", "join") === "anyone" });
    const appGrid = h("div", {});
    icons.grid(
      appGrid,
      sharedApps.filter(a => !a.always && on.includes(keyOf(a))).map(a => ({ app: a, href: keyOf(a) === "board" ? `#/discover/board/b/${desc.id}` : null, note: keyOf(a) === "board" ? null : "members" })),
      "No apps yet.",
    );
    root.replaceChildren(
      h("div", { className: "top" }, h("h2", { textContent: space.shown(desc) }), about ? h("p", { className: "about", textContent: String(about) }) : null, h("p", { textContent: `${n} member${n === 1 ? "" : "s"} · you are not in it` }), join),
      h("section", {}, h("h3", { textContent: "Apps" }), appGrid, h("p", { className: "none", textContent: "Board reads from outside; the others open once you are in." })),
    );
  }

  async function draw() {
    // (#/discover alone: Discover is each app's tab in your personal space — your Home.)
    if (ctx.space === "discover") return location.replace("#/");
    const sp = ctx.space && (await space.mine()).find(s => s.id === ctx.space && s.kind === "server");
    if (!sp) return outside();
    // Its roles as known (`roles.of` brought the group current when it opened the space; its acts keep it so): drawn
    // at once, never after another round of the group's log and every member's card.
    const r = await roles.of(sp);
    const on = r.apps();
    const may = r.can(me, "apps");
    const said = h("p", { className: "said", hidden: true });
    const members = r.members();
    // Its SETTINGS: the Settings app (`settings`), at a tab.
    const openSettings = tab => (location.hash = `#/s/${sp.id}/settings${tab && tab !== "overview" ? `/${tab}` : ""}`);
    // Its apps, as every Home shows them (`app-icons`): a pill with what is new (Chat: its unread), and — for who may —
    // the apps it does not use yet, dimmed, with Add.
    const add = k => async () => {
      try {
        await r.act({ act: "app", app: k, on: true });
        // Chat added: its first channel, if it has none.
        if (k === "chat") {
          const c = await conversation.channels(sp);
          await c.settled;
          if (!c.list().length) await c.add("general");
        }
        await draw();
      } catch (e) {
        said.textContent = e.message;
        said.hidden = false;
      }
    };
    // An app ALWAYS there (Settings): never added or removed.
    const uses = a => a.always || on.includes(keyOf(a));
    // THE DESK (`app-icons`: the same layout as your own Home): Pinned (this space's pins, its own) and All apps — the
    // apps it uses, and for who may, those it does not yet, dimmed with Add.
    const appGrid = h("div", {});
    icons.desk(appGrid, {
      items: () =>
        sharedApps
          .filter(a => uses(a) || may)
          .map(a => (uses(a) ? { app: a, href: `#/s/${sp.id}${a.route}`, count: activity && ["chat", "board"].includes(keyOf(a)) ? activity.of(sp.id, keyOf(a)) : 0 } : { app: a, add: add(keyOf(a)) })),
      pinKey: a => `app:/s/${sp.id}${a.route}`,
      empty: "No apps yet: its owner or an admin adds them.",
    });
    const removable = may ? sharedApps.filter(a => !a.always && on.includes(keyOf(a))) : [];
    root.replaceChildren(
      ...[
      h(
        "div",
        { className: "top" },
        h("h2", { textContent: space.shown(sp) }),
        r.config("space", "about", "") ? h("p", { className: "about", textContent: String(r.config("space", "about", "")) }) : null,
        r.can(me, "invite") ? h("button", { type: "button", className: "btn main", textContent: "Invite", onclick: () => openSettings("invites") }) : null,

        // Its PEOPLE: the space's Contact (`members-list`: the one list, its roles).
        h("p", {}, h("a", { href: `#/s/${sp.id}/contact`, textContent: `${members.length} member${members.length === 1 ? "" : "s"}` }), ` · you: ${r.role(me) ?? "member"}`),
      ),
      appGrid,
      removable.length
        ? h(
            "section",
            {},
            h("h3", { textContent: "Remove an app" }),
            h(
              "ul",
              {},
              ...removable.map(a =>
                h("li", { onclick: () => r.act({ act: "app", app: keyOf(a), on: false }).then(draw, e => ((said.textContent = e.message), (said.hidden = false))) }, `Remove ${a.name}`),
              ),
            ),
          )
        : null,
      said,
      ].filter(Boolean),
    );
  }
  root.replaceChildren(theme.loading("Opening the space…"));
  await draw();
  // Its roles or apps changing (an act from anyone): drawn again.
  const sp0 = ctx.space && (await space.mine()).find(s => s.id === ctx.space);
  if (sp0) (await roles.of(sp0)).onChange(() => el.isConnected && ctx.route === "/space" && ctx.space === sp0.id && draw());
  // Welcomes and askers are `upkeep`'s (every page, every 30 s): asked once now.
  ctx.require("upkeep").then(u => u.tick(), () => {});
  // Drawn again only for another space (the route event right after mounting names this one).
  let drawnFor = ctx.space;
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/space" && ctx.space !== drawnFor && ((drawnFor = ctx.space), draw()));
}
