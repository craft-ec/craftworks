// SPACE HOME, a page (`#/s/<space id>`): a SHARED space's own page — its APPS (the site's apps with a shared view that
// this space uses), its MEMBERS, and, for who may, its APPS to add or remove and its SETTINGS (members and roles,
// invites, the moderation log, leaving: the one `server-settings`). The space is the rail's choice. UI only: which apps
// a space uses is `roles`' (an `app` act), its people `roles`' and `conversation`'s, names `directory`'s.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [space, roles, conversation, directory, person, settings, theme, icons] = await Promise.all(
    ["space", "roles", "conversation", "directory", "person", "server-settings", "theme", "app-icons"].map(n => ctx.require(n)),
  );
  // What is new in each app (Chat's unread in this space): its pill, kept current.
  let activity = null;
  ctx.require("activity").then(a => ((activity = a), a.onChange(() => el.isConnected && ctx.route === "/space" && draw())), () => {});
  const me = (await space.account()).id;
  el.innerHTML = `
    <style>
      .sh { max-width: 880px; margin: 0 auto; display: grid; gap: var(--cw-space-4); }
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

  // DISCOVER's Home: like every Home, its apps — those with a public view (what each shows of the public network is
  // its own: Board's public spaces and posts). What you see there is filtered by your moderation lists (Account).
  async function drawDiscover() {
    const pub = ctx.apps.filter(a => (a.views ?? []).includes("public"));
    root.replaceChildren(
      h("div", { className: "top" }, h("h2", { textContent: "🧭 Discover" }), h("p", { textContent: "The public network: what public spaces publish. Nobody owns it; your moderation lists (Account → Moderation) filter what you see." })),
      (() => {
        const grid = h("div", {});
        icons.grid(grid, pub.map(a => ({ app: a, href: `#/discover${a.route}` })));
        return h("section", {}, grid);
      })(),
    );
  }

  async function draw() {
    if (ctx.space === "discover") return drawDiscover();
    const sp = ctx.space && (await space.mine()).find(s => s.id === ctx.space && s.kind === "server");
    if (!sp) return root.replaceChildren(h("p", { className: "none", textContent: "You are not in this space (left, or not joined yet)." }));
    // Its roles as known (`roles.of` brought the group current when it opened the space; its acts keep it so): drawn
    // at once, never after another round of the group's log and every member's card.
    const r = await roles.of(sp);
    const on = r.apps();
    const may = r.can(me, "apps");
    const said = h("p", { className: "said", hidden: true });
    const members = r.members();
    // Its settings: the space's own (members and roles, invites, the log, leaving); each app's are in the app.
    const openSettings = tab => settings.open(sp, { tab, left: () => (location.hash = "#/") });
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
    const appGrid = h("div", {});
    icons.grid(
      appGrid,
      sharedApps
        .filter(a => on.includes(keyOf(a)) || may)
        .map(a => (on.includes(keyOf(a)) ? { app: a, href: `#/s/${sp.id}${a.route}`, count: activity && ["chat", "board"].includes(keyOf(a)) ? activity.of(sp.id, keyOf(a)) : 0 } : { app: a, add: add(keyOf(a)) })),
      "No apps yet: its owner or an admin adds them.",
    );
    const removable = may ? sharedApps.filter(a => on.includes(keyOf(a))) : [];
    const people = h(
      "ul",
      {},
      ...members.map(m => {
        const li = h("li", { onclick: e => person.open(e.currentTarget, m.did, { space: sp }) }, directory.shown(m.did), m.role !== "member" ? h("i", { textContent: m.role }) : null);
        directory.name(m.did).then(t => (li.firstChild.textContent = t), () => {});
        return li;
      }),
    );
    root.replaceChildren(
      ...[
      h(
        "div",
        { className: "top" },
        h("h2", { textContent: space.shown(sp) }),
        r.can(me, "invite") ? h("button", { type: "button", className: "btn main", textContent: "Invite", onclick: () => openSettings("invites") }) : null,
        // PERMISSIONS (owner, admins): the space's policies, which every app inherits unless it says otherwise.
        r.can(me, "apps")
          ? h("button", {
              type: "button",
              className: "btn",
              textContent: "Permissions",
              onclick: async () =>
                (await ctx.require("app-settings")).open(sp, "Permissions", [
                  { action: "join", path: "", label: "Who may join (Anyone: whoever asks is let in; Members: by an invite)" },
                  { action: "invite", path: "", label: "Who may invite (make invite codes, add people, let askers in)" },
                  { action: "post", path: "", label: "Who may post (every app, unless it says otherwise)" },
                  { action: "comment", path: "", label: "Who may comment" },
                  { action: "vote", path: "", label: "Who may vote" },
                  { action: "edit", path: "", label: "Who may edit (Notes)" },
                ], {
                  // Open to anyone: its join requests' bag made, its acts published (who is in, how to join), and the space
                  // listed in Discover.
                  saved: async changed => {
                    if (changed["|join"] === "anyone") {
                      const index = await ctx.require("index");
                      await index.openRequests(`open ${sp.id}`);
                      await r.publish().catch(() => {}); // the owner's: an admin's space goes public from here on
                      await index.listSpace(sp);
                    }
                    await draw();
                  },
                }),
            })
          : null,
        h("button", { type: "button", className: "btn", textContent: "Settings", onclick: () => openSettings("overview") }),
        h("p", { textContent: `${members.length} member${members.length === 1 ? "" : "s"} · you: ${r.role(me) ?? "member"}` }),
      ),
      h(
        "section",
        {},
        h("h3", { textContent: "Apps" }),
        appGrid,
      ),
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
      h("section", {}, h("h3", { textContent: "Members" }), people),
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
