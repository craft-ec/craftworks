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
  const [space, roles, conversation, directory, person, settings, theme] = await Promise.all(
    ["space", "roles", "conversation", "directory", "person", "server-settings", "theme"].map(n => ctx.require(n)),
  );
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
      .sh .apps { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: var(--cw-space-3); }
      .sh .app { display: grid; gap: var(--cw-space-1); padding: var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius);
        background: var(--cw-surface); color: inherit; text-decoration: none; }
      .sh a.app:hover { border-color: var(--cw-muted); }
      .sh .app b { font-size: 1.05rem; }
      .sh .app span { color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .sh .app.off { border-style: dashed; }
      .sh .app .row { display: flex; justify-content: flex-end; }
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
      h("section", {}, h("h3", { textContent: "Apps" }), h("div", { className: "apps" }, ...pub.map(a => h("a", { className: "app", href: `#/discover${a.route}` }, h("b", { textContent: `${a.icon ?? ""} ${a.name}` }), h("span", { textContent: a.about ?? "" }))))),
    );
  }

  async function draw() {
    if (ctx.space === "discover") return drawDiscover();
    const sp = ctx.space && (await space.mine()).find(s => s.id === ctx.space && s.kind === "server");
    if (!sp) return root.replaceChildren(h("p", { className: "none", textContent: "You are not in this space (left, or not joined yet)." }));
    const r = await roles.of(sp);
    await r.refresh().catch(() => {});
    const on = r.apps();
    const may = r.can(me, "apps");
    const said = h("p", { className: "said", hidden: true });
    const members = r.members();
    // Its settings: the space's own (members and roles, invites, the log, leaving); each app's are in the app.
    const openSettings = tab => settings.open(sp, { tab, left: () => (location.hash = "#/") });
    const tile = a => {
      const k = keyOf(a);
      const used = on.includes(k);
      if (used) return h("a", { className: "app", href: `#/s/${sp.id}${a.route}` }, h("b", { textContent: `${a.icon ?? ""} ${a.name}` }), h("span", { textContent: a.about ?? "" }));
      if (!may) return null;
      const add = h("button", { type: "button", className: "btn main", textContent: "Add" });
      add.onclick = async () => {
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
      return h("div", { className: "app off" }, h("b", { textContent: `${a.icon ?? ""} ${a.name}` }), h("span", { textContent: a.about ?? "" }), h("div", { className: "row" }, add));
    };
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
        h("h2", { textContent: sp.name }),
        r.can(me, "invite") ? h("button", { type: "button", className: "btn main", textContent: "Invite", onclick: () => openSettings("invites") }) : null,
        // WHO MAY JOIN (owner, admins): by an invite, or anyone who asks (an open space; Discover shows Join).
        r.can(me, "apps")
          ? (() => {
              const sel = h("select", { className: "btn", ariaLabel: "Who may join" }, h("option", { value: "invite", textContent: "Joining: by invite" }), h("option", { value: "open", textContent: "Joining: anyone may join" }));
              sel.value = r.config("space", "join", "invite");
              sel.onchange = () => conversation.setJoin(sp, sel.value).then(draw, e => ((said.textContent = e.message), (said.hidden = false)));
              return sel;
            })()
          : null,
        h("button", { type: "button", className: "btn", textContent: "Settings", onclick: () => openSettings("overview") }),
        h("p", { textContent: `${members.length} member${members.length === 1 ? "" : "s"} · you: ${r.role(me) ?? "member"}` }),
      ),
      h(
        "section",
        {},
        h("h3", { textContent: "Apps" }),
        on.length || may ? h("div", { className: "apps" }, ...sharedApps.map(tile)) : h("p", { className: "none", textContent: "No apps yet: its owner or an admin adds them." }),
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
  // Welcomes waiting (a space joined by a code), and who asked by a code of this space let in (as Chat does).
  const tick = async () => {
    if (ctx.space === "discover") return;
    await conversation.accept().catch(() => []);
    const sp = ctx.space && (await space.mine()).find(s => s.id === ctx.space);
    if (sp && (await roles.of(sp)).can(me, "invite")) await conversation.admit(sp).catch(() => []);
  };
  tick();
  const every = setInterval(() => (el.isConnected ? tick() : clearInterval(every)), 30000);
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/space" && draw());
}
