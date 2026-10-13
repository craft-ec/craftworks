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
  const S = await ctx.require("signals");
  const style = document.createElement("style");
  style.textContent = `@layer apps {

    .cw-appset { border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-4); background: var(--cw-surface); color: var(--cw-fg); }
    .cw-appset h3 { margin: 0 0 var(--cw-space-3); font-size: 1.05rem; }
    .cw-appset h4 { margin: var(--cw-space-3) 0 var(--cw-space-2); font-size: var(--cw-text-xs); letter-spacing: .08em; text-transform: uppercase; color: var(--cw-muted); }
    .cw-appset label { display: grid; gap: 4px; margin-bottom: var(--cw-space-3); font-size: var(--cw-text-sm); font-weight: 600; }
    .cw-appset select, .cw-appset textarea, .cw-appset input { font: inherit; padding: 6px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
    .cw-appset textarea { min-height: 90px; resize: vertical; }
    .cw-appset .row { display: flex; gap: var(--cw-space-2); justify-content: flex-end; align-items: center; }
    .cw-appset .said { color: var(--cw-danger); font-size: var(--cw-text-sm); margin: 0; flex: 1; }
    .cw-appset .ok { color: var(--cw-muted); font-size: var(--cw-text-sm); margin: 0; flex: 1; }
}`;
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
    // A signal's action with nothing set anywhere: as its parent (`signals`), said so.
    const parent = S.parentOf(action);
    const inherited = from == null ? (parent ? r.policy(path, action) : r.personal || (S.interactions().some(a => a.action === action && a.openWhereRead) && r.policy(path, "read") === "anyone") ? "anyone" : "members") : r.policy(from, action);
    const LEVEL = p => (p === "" ? (r.personal ? "everything you post" : "the space") : p === "chat" ? "Chat" : p === "board" ? "Board" : p === "note" ? "Note" : p);
    // A personal space's: anyone · your followers · your friends · only you (`roles.personal`).
    // A shared space's: anyone (where it may) · members · admins · owner · nobody — and every ROLE composed there (its
    // holders, the owner and admins: `roles`), for anything but joining.
    const composed = r.personal || action === "join" ? [] : (r.roles?.() ?? []);
    const options = r.personal
      ? ["anyone", "followers", "friends", "author"].filter(w => !(action === "follow" && w === "followers"))
      : [...["anyone", "members", "admins", "owner", "nobody"].filter(w => w !== "anyone" || ["read", "join", "post", ...S.interactions().map(a => a.action)].includes(action)), ...composed.map(ro => `role:${ro.id}`)];
    const named = w => NAMES[w] ?? (w?.startsWith("role:") ? `Role: ${(r.roles?.() ?? []).find(ro => `role:${ro.id}` === w)?.name ?? "deleted role"}` : w);
    const sel = h(
      "select",
      { ariaLabel: `${action} at ${path || "the space"}` },
      // Inherit, from where; or the built-in default (the space itself has nothing above it — a tenant, later, will).
      h("option", { value: "", textContent: from == null ? `Default: ${named(inherited)}` : `As ${LEVEL(from)}: ${named(inherited)}` }),
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
  const madePublic = async (r, sp) => (await ctx.background("app-settings: r.publish", r.publish()), await (await ctx.require("index")).listSpace(sp));
  // A SIGNAL's own rule at a path (`signals`: react, save, share, tag — each, unset, as its parent): every settings
  // section lists them, so a new signal shows up here by itself.
  const SIGNAL_FIELDS = path => S.interactions().map(a => ({ action: a.action, path, label: a.label.replace(/^Who may /, "").replace(/^./, c => c.toUpperCase()) }));
  const SECTIONS = {
    "": {
      title: "The space",
      fields: [
        { key: "about", app: "space", label: "About" },
        { key: "history", app: "space", label: "History", choices: [["", "7 days · 20 versions"], ["3", "3 days"], ["1", "1 day"], ["0", "Off"]] },
        { action: "join", path: "", label: "Join" },
        { action: "invite", path: "", label: "Invite" },
        { action: "post", path: "", label: "Post" },
        ...SIGNAL_FIELDS(""),
        { action: "edit", path: "", label: "Edit" },
      ],
      saved: async (changed, r, sp) => {
        if (changed["|join"] !== "anyone") return;
        await (await ctx.require("index")).openRequests(`open ${sp.id}`);
        await madePublic(r, sp);
      },
    },
    // AN APP's OWN SETTINGS only (ARCHITECTURE §1): what the app is and does — Board's rules, Blog's approval, Chat's
    // channels. WHO MAY read, post, edit, comment … in a domain is the CAPABILITY's rule, set in the Spaces app
    // (`capabilityPage`), never here.
    board: { title: "Board", fields: [{ key: "rules", app: "board", label: "Rules" }] },
    // BLOG: a PUBLICATION whose editors (who may moderate) approve members' articles before they are public (`blog-press`).
    blog: { title: "Blog", fields: [{ key: "approval", app: "blog", label: "Approval", choices: [["", "Members publish directly"], ["editors", "Editors approve members' articles"]] }] },
    chat: { title: "Chat", fields: [], extra: chatChannels },
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
        h("div", { style: "display:grid;grid-template-columns:minmax(0,1fr) auto auto auto;gap:var(--cw-space-2);align-items:center;overflow-x:auto" }, ...["Channel", "Post", "Read", ""].map(t => h("b", { textContent: t })), ...channels.list().flatMap(c => {
          const name = h("input", { value: c.name, ariaLabel: "Channel name" });
          const acts = h("span", { className: "row" }, h("button", { type: "button", textContent: "Rename", onclick: () => channels.rename(c, name.value.trim()).then(draw, e => say(e.message)) }), h("button", { type: "button", textContent: "Delete", onclick: () => channels.remove(c).then(draw, e => say(e.message)) }));
          // WHO MAY POST in it: the channel item's own rule (none: as Chat's), saved as chosen.
          const own = c.item?.meta?.write?.post ?? "";
          const sel = h("select", { ariaLabel: `Who may post in ${c.name}` }, h("option", { value: "", textContent: `As Chat: ${NAMES[r.policyIn("chat", "post")] ?? r.policyIn("chat", "post")}` }), ...["anyone", "members", "admins", "owner"].map(w => h("option", { value: w, textContent: NAMES[w] })));
          sel.value = own;
          sel.onchange = () => channels.setPost(c, sel.value || null).then(draw, e => say(e.message));
          // WHO MAY READ it: its space's members, or fewer (a role, the admins, the owner) — then it is sealed to them
          // alone (its own group, kept in step).
          const readSel = h("select", { ariaLabel: `Who may read ${c.name}` }, h("option", { value: "", textContent: "Members" }), ...["admins", "owner", ...(r.roles?.() ?? []).map(ro => `role:${ro.id}`), ...Object.keys(r.lists?.() ?? {}).map(id => `list:${id}`)].map(w => h("option", { value: w, textContent: `${w.startsWith("role:") ? `Role: ${(r.roles?.() ?? []).find(ro => `role:${ro.id}` === w)?.name}` : w.startsWith("list:") ? `${(r.lists?.()[w.slice(5)]?.people ?? []).length} chosen people` : NAMES[w]}` })));
          readSel.value = c.item?.meta?.read ?? "";
          readSel.onchange = () => channels.setRead(c, readSel.value || null).then(draw, e => say(e.message));
          return [name, sel, readSel, acts];
        })),
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
      if (f.choices) return { f, input: h("select", {}, ...f.choices.map(([v, t]) => h("option", { value: v, textContent: t, selected: String(now ?? "") === v }))), now: String(now ?? "") };
      return { f, input: h("textarea", { value: now ?? "", maxLength: 2000 }), now };
    });
    const btn = h("button", { type: "submit", className: "main", textContent: "Save" });
    const f0 = S.fields.some(f => f.action) ? "Who" : "Setting";
    const more = S.extra ? h("div", {}) : null;
    const form = h("form", { className: "cw-appset", id: `settings-${(key || "space").replace("me:", "") || "everything"}` }, h("h3", { textContent: S.title }), h("div", { className: "set-grid", style: "display:grid;grid-template-columns:max-content minmax(0,24rem);gap:var(--cw-space-2) var(--cw-space-3);align-items:center" }, h("b", { textContent: "Action" }), h("b", { textContent: f0 }), ...inputs.flatMap(({ f, input }) => [h("label", { textContent: f.label, style: "margin:0", htmlFor: (input.id ||= `set-${key || "space"}-${f.action ?? f.key}-${f.path ?? ""}`) }), input])), more, h("div", { className: "row" }, said, btn));
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
  // THE SPACE's PAGE (the Settings app): the space's own section only — its apps' settings are in each app, its
  // capabilities' rules in the Spaces app.
  async function page(sp) {
    const r = await roles.of.strict(sp);
    const me = (await (await ctx.require("space")).account()).id;
    return h("div", { className: "cw-settings", style: "display:grid;gap:var(--cw-space-3)" }, await section(sp, "", r, me));
  }
  // AN APP's settings, drawn INSIDE the app (its own Settings page): null when it has none.
  const has = key => !!SECTIONS[key];
  async function appPage(sp, key) {
    if (!SECTIONS[key]) return null;
    const r = await roles.of.strict(sp);
    const me = (await (await ctx.require("space")).account()).id;
    return section(sp, key, r, me);
  }
  // A CAPABILITY's RULES (the Spaces app, beside its switch): who may read, post, edit, and each signal, in its domain.
  // Read by anyone: the space listed in public.
  async function capabilityPage(sp, cap, title = cap) {
    const r = await roles.of.strict(sp);
    const me = (await (await ctx.require("space")).account()).id;
    SECTIONS[`cap:${cap}`] ??= {
      title,
      fields: [...["read", "post", "edit"].map(action => ({ action, path: cap, label: action[0].toUpperCase() + action.slice(1) })), ...SIGNAL_FIELDS(cap)],
      saved: async (c, rr, s_) => c[`${cap}|read`] === "anyone" && madePublic(rr, s_),
    };
    return section(sp, `cap:${cap}`, r, me);
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
    SECTIONS["me:profile"] ??= { title: "You", fields: [{ action: "follow", path: "profile", label: "Follow you" }] };
    // EVERYTHING you post (the space's own level: every app inherits it unless it says otherwise).
    SECTIONS["me:"] ??= { title: "Everything you post", fields: SIGNAL_FIELDS("") };
    const forms = await Promise.all([
      section(null, "me:profile", r, me),
      section(null, "me:", r, me),
      ...Object.entries(PERSONAL).map(([key, title]) => {
        SECTIONS[`me:${key}`] ??= { title, fields: SIGNAL_FIELDS(key) };
        return section(null, `me:${key}`, r, me);
      }),
    ]);
    return h("div", { className: "cw-settings", style: "display:grid;gap:var(--cw-space-3)" }, ...forms);
  }


  return { page, personalPage, appPage, capabilityPage, has, href, who: (r, path, action) => who(r, path, action) };
}
