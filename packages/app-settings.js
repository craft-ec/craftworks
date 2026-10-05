// SETTINGS, a component: the ONE place a space's settings are made — on its Home, a SECTION per app it uses (and one
// for the space itself), each field either a POLICY — who may do an action at a path (`roles`' access: inherited
// along the path; "Inherit" drops this path's own policy so its parent's applies) — or a CONTENT setting of the app (a
// `config` act: Board's rules); an app's own rows beside them (Chat: its channels, each with its own policy). Every
// app's fields are listed HERE (`SECTIONS`), nowhere else: apps link to their section, none opens a dialog of its own.
// For the space's owner and admins; reading in public is the owner's.
//
//   const settings = await ctx.require("app-settings");
//   host.append(await settings.page(sp))          // every section of the space (its Home)
//   settings.href(sp, "board")                    // the link to one app's section
//   settings.who(r, path, action)                 // a <select> for one policy
export async function start(ctx) {
  const roles = await ctx.require("roles");
  const style = document.createElement("style");
  style.textContent = `
    .cw-appset { border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-4); background: var(--cw-surface); color: var(--cw-fg); }
    .cw-appset h3 { margin: 0 0 var(--cw-space-3); font-size: 1.05rem; }
    .cw-appset h4 { margin: var(--cw-space-3) 0 var(--cw-space-2); font-size: var(--cw-text-xs); letter-spacing: .08em; text-transform: uppercase; color: var(--cw-muted); }
    .cw-appset label { display: grid; gap: 4px; margin-bottom: var(--cw-space-3); font-size: var(--cw-text-sm); font-weight: 600; }
    .cw-appset select, .cw-appset textarea, .cw-appset input { font: inherit; padding: 6px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
    .cw-appset textarea { min-height: 90px; resize: vertical; }
    .cw-appset button { font: inherit; cursor: pointer; border: 1px solid var(--cw-line); background: none; color: var(--cw-fg); border-radius: var(--cw-radius-sm); padding: 4px var(--cw-space-3); }
    .cw-appset button.main { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
    .cw-appset .row { display: flex; gap: var(--cw-space-2); justify-content: flex-end; align-items: center; }
    .cw-appset .said { color: var(--cw-danger); font-size: var(--cw-text-sm); margin: 0; flex: 1; }
    .cw-appset .ok { color: var(--cw-muted); font-size: var(--cw-text-sm); margin: 0; flex: 1; }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const NAMES = { anyone: "Anyone (public)", members: "Members", admins: "Admins", owner: "The owner", nobody: "Nobody", followers: "Your followers", friends: "Your friends", author: "Only you" };
  const parentOf = path => (path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : path ? "" : null);

  // ONE POLICY's choice: Inherit (what the parent says), or a who. Reading by anyone is offered to the owner only.
  function who(r, path, action, { me = null } = {}) {
    const own = r.policiesAt(path)[action] ?? "";
    // What this path would get without a policy of its own: the nearest level above that sets one — named — else the
    // built-in default (a space's reading, say, is always its members': nothing above sets it).
    let from = null;
    for (let p = parentOf(path); p != null; p = parentOf(p))
      if (r.policiesAt(p)[action]) {
        from = p;
        break;
      }
    // The default (nothing set above): anyone, in a personal space; in a shared one, members — but comments and votes
    // where anyone reads (`roles`' default: public participation).
    const inherited = from == null ? (r.personal || (["comment", "vote"].includes(action) && r.policy(path, "read") === "anyone") ? "anyone" : "members") : r.policy(from, action);
    const LEVEL = p => (p === "" ? (r.personal ? "everything you post" : "the space") : p === "chat" ? "Chat" : p === "board" ? "Board" : p === "note" ? "Note" : p);
    // A personal space's: anyone · your followers · your friends · only you (`roles.personal`).
    // A shared space's: anyone (where it may) · members · admins · owner · nobody — and every ROLE composed there (its
    // holders, the owner and admins: `roles`), for anything but joining.
    const composed = r.personal || action === "join" ? [] : (r.roles?.() ?? []);
    const options = r.personal
      ? ["anyone", "followers", "friends", "author"].filter(w => !(action === "follow" && w === "followers"))
      : [...["anyone", "members", "admins", "owner", "nobody"].filter(w => w !== "anyone" || ["read", "join", "post", "comment", "vote"].includes(action)), ...composed.map(ro => `role:${ro.id}`)];
    const named = w => NAMES[w] ?? (w?.startsWith("role:") ? `Role: ${(r.roles?.() ?? []).find(ro => `role:${ro.id}` === w)?.name ?? "deleted role"}` : w);
    const sel = h(
      "select",
      { ariaLabel: `${action} at ${path || "the space"}` },
      // Inherit, from where; or the built-in default (the space itself has nothing above it — a tenant, later, will).
      h("option", { value: "", textContent: from == null ? `Default (${named(inherited)})` : `Inherit from ${LEVEL(from)} (${named(inherited)})` }),
      ...options.map(w => h("option", { value: w, textContent: named(w) })),
    );
    sel.value = own;
    sel.dataset.was = own;
    sel.dataset.path = path;
    sel.dataset.action = action;
    if (action === "read" && me && r.role(me) !== "owner") sel.disabled = true;
    return sel;
  }
  // Save a policy <select> if it changed: `inherit` drops the override.
  async function save(r, sel) {
    if (sel.value === sel.dataset.was) return false;
    await r.act({ act: "policy", path: sel.dataset.path, action: sel.dataset.action, who: sel.value || "inherit" });
    sel.dataset.was = sel.value;
    return true;
  }

  // EVERY APP's SETTINGS — its fields, what saving one does beyond the act (a space made public: listed).
  const madePublic = async (r, sp) => (await r.publish().catch(() => {}), await (await ctx.require("index")).listSpace(sp));
  const SECTIONS = {
    "": {
      title: "The space",
      fields: [
        { key: "about", app: "space", label: "About (what this space is: on its card and its Home, also from outside)" },
        { action: "join", path: "", label: "Who may join (Anyone: whoever asks is let in; Members: by an invite)" },
        { action: "invite", path: "", label: "Who may invite (make invite codes, add people, let askers in)" },
        { action: "post", path: "", label: "Who may post (every app, unless it says otherwise)" },
        { action: "comment", path: "", label: "Who may comment" },
        { action: "vote", path: "", label: "Who may vote" },
        { action: "edit", path: "", label: "Who may edit (shared notes and files)" },
      ],
      saved: async (changed, r, sp) => {
        if (changed["|join"] !== "anyone") return;
        await (await ctx.require("index")).openRequests(`open ${sp.id}`);
        await madePublic(r, sp);
      },
    },
    // Board shows the TEXT domain (posts): its rules are the domain's (`kinds.policyDomain`), as every app's are.
    board: {
      title: "Board",
      fields: [
        { action: "read", path: "text", label: "Who may read (Anyone: public — its members and moderation too; posts made before stay as they were)" },
        { action: "post", path: "text", label: "Who may post" },
        { action: "comment", path: "text", label: "Who may comment" },
        { action: "vote", path: "text", label: "Who may vote" },
        { key: "rules", app: "board", label: "Rules (shown beside the board)" },
      ],
      saved: async (changed, r, sp) => changed["text|read"] === "anyone" && madePublic(r, sp),
    },
    video: { title: "Video", fields: ["read", "post", "comment", "vote"].map(action => ({ action, path: "video", label: `Who may ${action}` })), saved: async (c, r, sp) => c["video|read"] === "anyone" && madePublic(r, sp) },
    audio: { title: "Audio", fields: ["read", "post", "comment", "vote"].map(action => ({ action, path: "audio", label: `Who may ${action}` })), saved: async (c, r, sp) => c["audio|read"] === "anyone" && madePublic(r, sp) },
    image: { title: "Image", fields: ["read", "post", "comment", "vote"].map(action => ({ action, path: "image", label: `Who may ${action}` })), saved: async (c, r, sp) => c["image|read"] === "anyone" && madePublic(r, sp) },
    book: { title: "Book", fields: ["read", "post", "comment", "vote"].map(action => ({ action, path: "book", label: `Who may ${action}` })), saved: async (c, r, sp) => c["book|read"] === "anyone" && madePublic(r, sp) },
    chat: { title: "Chat", fields: [{ action: "post", path: "chat", label: "Who may post (in every channel that does not say otherwise)" }], extra: chatChannels },
    note: { title: "Note", fields: [{ action: "post", path: "note", label: "Who may add notes" }, { action: "edit", path: "note", label: "Who may edit notes" }] },
    // Drive shows the FILE domain: its rules are the domain's (`kinds.policyDomain`), as Board's are the text domain's.
    drive: { title: "Drive", fields: [{ action: "read", path: "file", label: "Who may read" }, { action: "post", path: "file", label: "Who may upload" }, { action: "edit", path: "file", label: "Who may move files" }] },
  };
  // CHAT's own rows: its channels, each with who may post in it.
  async function chatChannels(host, r, sp) {
    const channels = await (await ctx.require("conversation")).channels(sp);
    await channels.settled;
    const say = t => (host.querySelector(".said") ?? host).append(h("p", { className: "said", textContent: t }));
    const draw = () => {
      const add = h("input", { placeholder: "New channel", ariaLabel: "New channel" });
      host.replaceChildren(
        h("h4", { textContent: "Channels" }),
        ...channels.list().map(c => {
          const name = h("input", { value: c.name, ariaLabel: "Channel name" });
          const row = h("div", { className: "row" }, name, h("button", { type: "button", textContent: "Rename", onclick: () => channels.rename(c, name.value.trim()).then(draw, e => say(e.message)) }), h("button", { type: "button", textContent: "Delete", onclick: () => channels.remove(c).then(draw, e => say(e.message)) }));
          // WHO MAY POST in it: the channel item's own rule (none: as Chat's), saved as chosen.
          const own = c.item?.meta?.write?.post ?? "";
          const sel = h("select", { ariaLabel: `Who may post in ${c.name}` }, h("option", { value: "", textContent: `As Chat (${NAMES[r.policyIn("chat", "post")] ?? r.policyIn("chat", "post")})` }), ...["anyone", "members", "admins", "owner"].map(w => h("option", { value: w, textContent: NAMES[w] })));
          sel.value = own;
          sel.onchange = () => channels.setPost(c, sel.value || null).then(draw, e => say(e.message));
          // WHO MAY READ it: its space's members, or fewer (a role, the admins, the owner) — then it is sealed to them
          // alone (its own group, kept in step).
          const readSel = h("select", { ariaLabel: `Who may read ${c.name}` }, h("option", { value: "", textContent: "Read: the space's members" }), ...["admins", "owner", ...(r.roles?.() ?? []).map(ro => `role:${ro.id}`), ...Object.keys(r.lists?.() ?? {}).map(id => `list:${id}`)].map(w => h("option", { value: w, textContent: `Read: ${w.startsWith("role:") ? `Role: ${(r.roles?.() ?? []).find(ro => `role:${ro.id}` === w)?.name}` : w.startsWith("list:") ? `${(r.lists?.()[w.slice(5)]?.people ?? []).length} chosen people` : NAMES[w]}` })));
          readSel.value = c.item?.meta?.read ?? "";
          readSel.onchange = () => channels.setRead(c, readSel.value || null).then(draw, e => say(e.message));
          row.append(sel, readSel);
          return row;
        }),
        h("div", { className: "row" }, add, h("button", { type: "button", textContent: "Add", onclick: () => add.value.trim() && channels.add(add.value.trim()).then(draw, e => say(e.message)) })),
      );
    };
    draw();
  }

  // ONE SECTION, inline: its fields, its own rows, Save.
  async function section(sp, key, r, me) {
    const S = SECTIONS[key];
    const said = h("p", { className: "said" });
    const inputs = S.fields.map(f => {
      if (f.action) return { f, input: who(r, f.path, f.action, { me }) };
      const now = r.config(f.app, f.key, "");
      return { f, input: h("textarea", { value: now ?? "", maxLength: 2000 }), now };
    });
    const btn = h("button", { type: "submit", className: "main", textContent: "Save" });
    const more = S.extra ? h("div", {}) : null;
    const form = h("form", { className: "cw-appset", id: `settings-${(key || "space").replace("me:", "") || "everything"}` }, h("h3", { textContent: S.title }), ...inputs.map(({ f, input }) => h("label", {}, f.label, input)), more, h("div", { className: "row" }, said, btn));
    form.onsubmit = async e => {
      e.preventDefault();
      btn.disabled = true;
      said.className = "said";
      said.textContent = "";
      try {
        const changed = {};
        for (const { f, input, now } of inputs) {
          if (f.action) {
            if (await save(r, input)) changed[`${f.path}|${f.action}`] = input.value || "inherit";
          } else if (input.value !== (now ?? "")) {
            await r.act({ act: "config", app: f.app, key: f.key, value: input.value });
            changed[f.key] = input.value;
          }
        }
        for (const sel of more?.querySelectorAll("select[data-path]") ?? []) if (await save(r, sel)) changed[`${sel.dataset.path}|${sel.dataset.action}`] = sel.value || "inherit";
        await S.saved?.(changed, r, sp);
        said.className = "ok";
        said.textContent = "Saved.";
      } catch (err) {
        said.textContent = err?.message ?? String(err);
      } finally {
        btn.disabled = false;
      }
    };
    if (S.extra) await S.extra(more, r, sp).catch(err => more.append(h("p", { className: "said", textContent: err?.message ?? String(err) })));
    return form;
  }
  // THE PAGE: the space's own section, then each app it uses that has settings.
  async function page(sp) {
    const r = await roles.of(sp);
    const me = (await (await ctx.require("space")).account()).id;
    const keys = ["", ...r.apps().filter(a => SECTIONS[a])];
    return h("div", { className: "cw-settings", style: "display:grid;gap:var(--cw-space-3)" }, ...(await Promise.all(keys.map(k => section(sp, k, r, me)))));
  }
  // A space's: its Settings, "Apps & rules", at that app's section; yours: Settings, "Permissions", at it.
  const href = (sp, key) => (sp ? `#/s/${sp.id}/settings/apps/${key}` : `#/settings/permissions/${key}`);
  // YOUR SPACE's SETTINGS (your Home): who may comment and vote on what you post — everything, then each app, an
  // item's own setting over both; inherited and kept in time as a space's are. The same sections, read and written
  // through `roles.personal`.
  // Each by its DOMAIN (what governs its items: `kinds.policyDomain`), titled as the app showing it.
  const PERSONAL = { text: "Board", video: "Video", audio: "Audio", image: "Image", book: "Book" };
  async function personalPage() {
    const me = (await (await ctx.require("space")).account()).id;
    const r = roles.personal(me);
    await r.ready;
    // You, as followed: the same rule and credential as a comment on your post (`roles.followable`).
    SECTIONS["me:profile"] ??= { title: "You", fields: [{ action: "follow", path: "profile", label: "Who may follow you" }] };
    // EVERYTHING you post (the space's own level: every app inherits it unless it says otherwise).
    SECTIONS["me:"] ??= { title: "Everything you post", fields: [{ action: "comment", path: "", label: "Who may comment (every app, unless it says otherwise)" }, { action: "vote", path: "", label: "Who may vote" }] };
    const forms = await Promise.all([
      section(null, "me:profile", r, me),
      section(null, "me:", r, me),
      ...Object.entries(PERSONAL).map(([key, title]) => {
        SECTIONS[`me:${key}`] ??= { title, fields: [{ action: "comment", path: key, label: "Who may comment" }, { action: "vote", path: key, label: "Who may vote" }] };
        return section(null, `me:${key}`, r, me);
      }),
    ]);
    return h("div", { className: "cw-settings", style: "display:grid;gap:var(--cw-space-3)" }, ...forms);
  }


  return { page, personalPage, href, who: (r, path, action) => who(r, path, action) };
}
