// CHAT, a page: a SHARED SPACE's chat, Discord-style — the space chosen in the spaces panel (`#/s/<space>/chat[/<channel>]`):
// its CHANNELS, the channel open (the `room` component), and its MEMBERS. Chat is a shared space's app only (direct and
// group conversations are Messages, in the personal space); the space itself — made, joined, its settings — is the spaces panel's
// and its Home's. UI only: a space is `space`'s, a channel a sub-space inheriting its access, its keys `keys`' (the
// space's group), its messages `content`'s; its channel list is a table of the space (`channels`: `<id>` → `{ name, at }`).
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [space, keys, directory, roomUI, conversation, theme, roles, moderation, person, activity] = await Promise.all(
    ["space", "keys", "directory", "room", "conversation", "theme", "roles", "moderation", "person", "activity"].map(n => ctx.require(n)),
  );
  const account = await space.account();
  // DISCOVER (Chat's public view): the directory of open spaces with Chat — to join (chat itself stays its members').
  if (ctx.space === "discover") return openSpaces(ctx, el, { space, roles, conversation, theme });
  el.classList.add("cw-fill");
  el.innerHTML = `
    <style>
      .dc { display: grid; grid-template-columns: 240px 1fr 220px; min-height: 420px;
        overflow: hidden; background: var(--cw-surface); }
      .dc button { font: inherit; cursor: pointer; }
      .dc .side, .dc .people { background: var(--cw-bg); display: flex; flex-direction: column; min-width: 0; }
      .dc .side { border-right: 1px solid var(--cw-line); }
      .dc .people { border-left: 1px solid var(--cw-line); padding: var(--cw-space-3); overflow-y: auto; }
      .dc .side h2 { margin: 0; font-size: 1rem; padding: 12px var(--cw-space-4); border-bottom: 1px solid var(--cw-line);
        overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .dc .chans { flex: 1; overflow-y: auto; padding: var(--cw-space-2); display: grid; align-content: start; gap: 2px; }
      .dc .chans button { text-align: left; border: 0; background: none; color: var(--cw-muted); padding: 6px var(--cw-space-2);
        border-radius: var(--cw-radius-sm); }
      .dc .chans button:hover { background: var(--cw-hover); color: var(--cw-fg); }
      .dc .chans button[aria-current="true"] { background: var(--cw-pressed); color: var(--cw-fg); }
      .dc .chans .new { color: var(--cw-accent); }
      .dc .room { min-width: 0; min-height: 0; }
      .dc .people h3 { font-size: var(--cw-text-xs); letter-spacing: .08em; color: var(--cw-muted); margin: 0 0 var(--cw-space-2); }
      .dc .people ul { margin: 0; padding: 0; }
      .dc .people li { list-style: none; padding: var(--cw-space-1) 0; display: flex; align-items: center; gap: var(--cw-space-1); }
      .dc .people li .n { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .dc .people li .r { color: var(--cw-muted); font-size: var(--cw-text-xs); }
      .dc .people li.who { cursor: pointer; border-radius: var(--cw-radius-sm); padding-left: var(--cw-space-1); }
      .dc .people li.who:hover { background: var(--cw-hover); }
      .dc .chans .ch { display: flex; align-items: center; }
      .dc .chans .ch > button:first-child { flex: 1; min-width: 0; }
      .dc .empty { color: var(--cw-muted); text-align: center; margin: auto; padding: var(--cw-space-5); }
      .dc .said { color: var(--cw-danger); font-size: var(--cw-text-sm); padding: var(--cw-space-2) var(--cw-space-4); margin: 0; }
      .dc dialog.ask, .dc dialog.start { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-4); width: min(360px, calc(100vw - 32px));
        box-shadow: var(--cw-shadow-lg); }
      .dc dialog.ask form, .dc dialog.start form { display: grid; gap: var(--cw-space-3); }
      .dc dialog.ask .row, .dc dialog.start .row { display: flex; gap: var(--cw-space-2); justify-content: flex-end; }
      .dc dialog.ask button, .dc dialog.start button { border: 1px solid var(--cw-line); background: none; color: inherit; border-radius: var(--cw-radius-sm);
        padding: var(--cw-space-1) var(--cw-space-3); }
      .dc dialog.ask button[value="ok"], .dc dialog.start button[value="ok"] { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
      @media (max-width: 800px) { .dc { grid-template-columns: 180px 1fr; } .dc .people { display: none; } }
    </style>
    <div class="dc">
      <aside class="side"><h2>—</h2><div class="chans"></div><p class="said" hidden></p></aside>
      <section class="room"></section>
      <aside class="people"><h3>MEMBERS</h3><ul></ul></aside>
      <dialog class="start"><form method="dialog"><label>Who? One person, or several separated by commas for a group — name#abc123 or did:craftec:… <input name="who" autocomplete="off" required></label>
        <label>Group name (for several) <input name="group" autocomplete="off"></label>
        <div class="row"><button value="cancel" formnovalidate>Cancel</button><button value="ok">Start</button></div></form></dialog>
      <dialog class="ask"><form method="dialog"><label><span></span><input name="answer" autocomplete="off" required></label>
        <div class="row"><button value="cancel" formnovalidate>Cancel</button><button value="ok">Create</button></div></form></dialog>
    </div>`;
  const $ = s => el.querySelector(s);
  // A click outside a question (on its backdrop) cancels it, as Esc does.
  $(".ask").addEventListener("click", e => e.target === e.currentTarget && e.currentTarget.close("cancel"));
  const sideName = $(".side h2"), chans = $(".chans"), roomEl = $(".room"), people = $(".people ul"), said = $(".said");
  const say = m => ((said.textContent = m), (said.hidden = !m));
  // One question, in the page's own dialog: the answer, or null.
  const ask = (question, button = "Create") =>
    new Promise(resolve => {
      const d = $(".ask");
      d.querySelector("span").textContent = question;
      d.querySelector('button[value="ok"]').textContent = button;
      const field = d.querySelector("input");
      field.value = "";
      d.onclose = () => resolve(d.returnValue === "ok" ? field.value.trim() || null : null);
      d.showModal();
      field.focus();
    });

  let server = null, chs = null, channel = null, shown = null, rs = null, mod = null;
  const may = what => !!rs?.can(account.id, what);

  // The server's CHANNELS, as settings changes them: `conversation.channels`', and the channel open closed when it goes.
  const channelOps = {
    list: () => chs?.list() ?? [],
    add: name => chs.add(name),
    rename: (c, name) => chs.rename(c, name),
    remove: async c => {
      await chs.remove(c);
      if (channel?.id === c.id) {
        channel = null;
        shown?.close();
        roomEl.replaceChildren();
      }
    },
  };
  async function openServer(s, want = null) {
    server = s;
    channel = null;
    chs = null;
    sideName.textContent = s.name;
    chans.replaceChildren(theme.loading("Loading channels…"));
    people.replaceChildren(theme.loading("Loading members…", 2));
    roomEl.replaceChildren(theme.loading(`Opening ${s.name}…`));
    try {
      // At once: its roles, moderation and channels (each opens what it reads; the keys come with the tables).
      const chsP = conversation.channels(s);
      chsP.catch(() => {});
      [rs, mod] = await Promise.all([roles.of(s), moderation.of(s)]);
      if (server !== s) return;
      menu();
      if (rs.left) {
        const gone = `You were removed from ${s.name}.`;
        chans.replaceChildren();
        people.replaceChildren();
        roomEl.replaceChildren(Object.assign(document.createElement("p"), { className: "empty", textContent: gone }));
        return;
      }
      rs.onChange(() => {
        if (server !== s) return;
        drawChannels();
        drawMembers(s);
      });
      const c = (chs = await chsP);
      c.onChange(() => server === s && drawChannels());
      c.settled.finally(() => {
        if (server !== s) return;
        drawChannels();
        const c = (want && list().find(x => x.id.endsWith(`/${want}`))) || list()[0];
        if (c && (!channel || (want && channel.id !== c.id))) openChannel(c);
      });
      drawChannels();
      drawMembers(s);
      const first = (want && list().find(x => x.id.endsWith(`/${want}`))) || list()[0];
      if (first) openChannel(first);
      else roomEl.replaceChildren();
    } catch (e) {
      say(`Could not open the server: ${e?.message ?? e}`);
    }
  }

  const list = () => chs?.list() ?? [];
  // A rule's `who` in words (a role by its name).
  const whoName = w => (w?.startsWith("role:") ? `the ${rs?.roles?.().find(x => `role:${x.id}` === w)?.name ?? "role"}'s holders` : { admins: "the admins", owner: "the owner" }[w] ?? w);

  function drawChannels() {
    if (!chs) return;
    // A server's channels come from its members' feeds: none yet is not "none" until every one has been tried.
    if (!list().length && !chs.done) return chans.replaceChildren(theme.loading("Loading channels…"));
    chans.replaceChildren(
      ...list().map(c => {
        const row = Object.assign(document.createElement("div"), { className: "ch" });
        const b = Object.assign(document.createElement("button"), { type: "button", textContent: `${c.item?.meta?.group ? "🔒" : "#"} ${c.name}` });
        b.setAttribute("aria-current", String(channel?.id === c.id));
        b.onclick = () => openChannel(c);
        const n = activity.unread(c.id);
        if (n) b.append(Object.assign(document.createElement("span"), { className: "cw-badge", textContent: String(n) }));
        row.append(b);
        return row;
      }),
      ...(may("channels") ? [Object.assign(document.createElement("button"), { type: "button", className: "new", textContent: "+ Add a channel", onclick: addChannel })] : []),
    );
  }

  async function addChannel() {
    const name = await ask("Channel name");
    if (name) await channelOps.add(name).catch(e => say(`Could not add it: ${e?.message ?? e}`));
  }

  // The server's members: the accounts in its group, with their roles. A click: what can be done with that person.
  async function drawMembers(s) {
    // A SERVER: the one list of its people (`members-list`), narrowed to who may read the channel open (a channel
    // kept to a role, the admins, the owner: its readers only).
    if (s.kind === "server") {
      const who = channel?.item?.meta?.read ?? null;
      const list = await (await ctx.require("members-list")).show(s, { who });
      if (server === s) people.replaceChildren(list);
      return;
    }
    // (A direct conversation: you and them, whatever its group has listed yet.)
    const listed = await conversation.members(s);
    const dids = s.kind === "direct" ? [...new Set([account.id, s.with, ...listed].filter(Boolean))] : listed;
    if (server !== s) return;
    if (!dids.length) return people.replaceChildren();
    const draw = names =>
      people.replaceChildren(
        ...dids.map((d, i) => {
          const li = Object.assign(document.createElement("li"), { title: d, className: "who", onclick: e => person.open(e.currentTarget, d, { space: s }) });
          const role = rs?.role(d) ?? "member";
          li.append(
            Object.assign(document.createElement("span"), { className: "n", textContent: `${directory.shown(d, names[i])}${d === account.id ? " (you)" : ""}` }),
            Object.assign(document.createElement("span"), { className: "r", textContent: role === "member" ? "" : role }),
          );
          return li;
        }),
      );
    draw([]);
    draw(await Promise.all(dids.map(d => directory.handle(d))));
  }

  async function openChannel(c) {
    channel = c;
    // Its readers in the side panel (a channel kept to a role shows that role's people).
    if (server) drawMembers(server).catch(() => {});
    // A RESTRICTED channel (who may read: a role, admins…): its messages are its GROUP's (`groups`) — open to its
    // readers, locked to everyone else here.
    const g = c.item?.meta?.group;
    if (g) {
      // (Its group left — no longer of its readers —: locked, as for anyone else.)
      const gsp0 = (await space.mine()).find(s => s.id === g);
      const gsp = gsp0 && !(await roles.of(gsp0).then(async r => (await r.settled, r.left), () => true)) ? gsp0 : null;
      const to = `#/s/${server.id}/chat/${c.id.split("/").pop()}`;
      if (location.hash !== to) history.replaceState(null, "", to);
      drawChannels();
      shown?.close();
      if (!gsp) {
        shown = null;
        return roomEl.replaceChildren(Object.assign(document.createElement("p"), { className: "empty", textContent: `🔒 #${c.name} is read by ${whoName(c.item.meta.read)} only.` }));
      }
      shown = await roomUI.show(roomEl, space.channel(gsp, c.id.split("/").pop(), c.name), `🔒 #${c.name}`);
      return;
    }
    // The address follows the channel open (a reload, a link: the same channel).
    const to = `#/s/${server.id}/chat/${c.id.split("/").pop()}`;
    if (location.hash !== to) history.replaceState(null, "", to);
    drawChannels();
    shown?.close();
    shown = await roomUI.show(roomEl, c, `#${c.name}`);
  }

  // Welcomes and askers are `upkeep`'s (every page, every 30 s): asked once now, for whoever is waiting on this one.
  ctx.require("upkeep").then(u => u.tick(), () => {});
  // New since read: the channel list says so.
  activity.onChange(() => el.isConnected && (!ctx.space && personalDraw ? personalDraw() : drawChannels()));
  // YOUR CHAT (your personal space): your CONVERSATIONS — direct and group, each its own sealed space with its own
  // members — as the channels of your space; one opened in the same room as any channel (`#/chat/<conversation>`).
  // Conversations begun with you while you were away: joined when it opens.
  let accepted = null;
  async function personal() {
    // (Run again on every route event: the conversation open stays open — only another one replaces it.)
    server = shownConv;
    rs = null;
    channel = null;
    sideName.textContent = "Your conversations";
    const draw = async () => {
      const list = await conversation.list();
      const name = sp => (sp.kind === "group" ? `👥 ${sp.name}` : sp.with ? directory.nameEl(sp.with) : sp.name);
      chans.replaceChildren(
        ...list.map(sp => {
          const b = Object.assign(document.createElement("button"), { type: "button", title: sp.with ?? "" });
          b.append(name(sp));
          const n = activity.unread(sp.id);
          if (n) b.append(Object.assign(document.createElement("span"), { className: "cw-badge", textContent: String(n) }));
          b.setAttribute("aria-current", String(shownConv?.id === sp.id));
          b.onclick = () => (location.hash = `#/chat/${sp.id}`);
          return b;
        }),
        ...(accepted ? [] : [theme.loading("Checking your inbox…", 1)]),
        Object.assign(document.createElement("button"), { type: "button", className: "new", textContent: "+ New conversation", onclick: start }),
      );
      return list;
    };
    const list = await draw();
    accepted ??= conversation.accept().catch(e => ctx.log("conversation", { what: e?.message ?? String(e) })).then(() => el.isConnected && !ctx.space && draw().then(openAt));
    const openAt = async (l = list) => {
      const sp = (ctx.sub && (await conversation.list()).find(c => c.id === ctx.sub)) || null;
      if (!sp) return shownConv ? null : roomEl.replaceChildren(Object.assign(document.createElement("p"), { className: "empty", textContent: l.length ? "Pick a conversation, or start one." : "No conversations yet: start one." }));
      if (shownConv?.id === sp.id) return;
      shownConv = sp;
      server = sp;
      draw();
      drawMembers(sp);
      shown?.close();
      const title = sp.kind === "group" ? sp.name : document.createElement("span");
      if (typeof title !== "string") title.append("@", directory.nameEl(sp.with));
      shown = await roomUI.show(roomEl, sp, title);
    };
    personalDraw = draw;
    await openAt();
  }
  let shownConv = null, personalDraw = null;
  // A NEW conversation: one person (direct) or several (a group), each welcomed.
  function start() {
    const d = $(".start");
    d.querySelector('input[name="who"]').value = "";
    d.querySelector('input[name="group"]').value = "";
    d.onclose = async () => {
      const who = d.returnValue === "ok" ? d.querySelector('input[name="who"]').value.trim() : "";
      if (!who) return;
      say("");
      try {
        const dids = await Promise.all(who.split(",").map(x => x.trim()).filter(Boolean).map(x => conversation.person(x)));
        const sp = dids.length > 1 ? await conversation.group(dids, d.querySelector('input[name="group"]').value) : await conversation.direct(dids[0]);
        location.hash = `#/chat/${sp.id}`;
      } catch (e) {
        say(`Could not start it: ${e?.message ?? e}`);
      }
    };
    d.showModal();
    d.querySelector("input").focus();
  }
  $(".start").addEventListener("click", e => e.target === e.currentTarget && e.currentTarget.close("cancel"));

  // THE SPACE open (`ctx.space`), at a channel (`ctx.sub`: a notification clicked, a link) or its first.
  const at = async () => {
    if (!ctx.space || ctx.space === "discover") {
      if (ctx.space === "discover") return;
      return personal();
    }
    shownConv = null;
    const s = ctx.space && (await space.mine()).find(x => x.id === ctx.space && x.kind === "server");
    if (!s) return;
    const cid = ctx.sub || null;
    if (server?.id !== s.id) return openServer(s, cid);
    const c = cid && list().find(x => x.id.endsWith(`/${cid}`));
    if (c && channel?.id !== c.id) openChannel(c);
  };
  const menu = () => {
    // (Its settings — who may post, its channels — are the Settings app's.)
    ctx.actions["/chat"] = [];
    dispatchEvent(new CustomEvent("craftworks:actions"));
  };
  menu();
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/chat" && at());
  await at();
}

// THE DIRECTORY (Discover → Chat): every space listed in Discover that is open to join and uses Chat — its name, how
// many are in it, and Join (or Open, when you are in). What is said in them is their members' only: nothing of it here.
async function openSpaces(ctx, el, { space, roles, conversation, theme }) {
  const posts = await ctx.require("items");
  el.innerHTML = `
    <style>
      .dir { max-width: 880px; margin: 0 auto; display: grid; gap: var(--cw-space-3); }
      .dir h2 { margin: 0; font-size: 1.4rem; }
      .dir .note { margin: 0; color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .dir .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: var(--cw-space-3); }
      .dir .card { display: grid; gap: var(--cw-space-2); padding: var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); }
      .dir .card b { font-size: 1.05rem; overflow-wrap: anywhere; }
      .dir .card span { color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .dir .card button, .dir .card a { justify-self: start; font: inherit; border: 0; border-radius: var(--cw-radius-pill); padding: 6px var(--cw-space-4);
        background: var(--cw-accent); color: var(--cw-accent-fg); cursor: pointer; text-decoration: none; font-weight: 600; }
      .dir .card .said { color: var(--cw-muted); font-size: var(--cw-text-sm); margin: 0; }
    </style>
    <div class="dir"><h2>💬 Open spaces</h2><p class="note">Spaces anyone may join. What is said in them is for their members.</p><div class="grid"></div></div>`;
  const grid = el.querySelector(".grid");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  ctx.actions["/chat"] = [];
  dispatchEvent(new CustomEvent("craftworks:actions"));
  grid.replaceChildren(theme.loading("Finding open spaces…"));
  const joins = await ctx.require("join-button");
  const cards = [];
  for (const d of await posts.publicSpaces()) {
    const r = await roles.ofPublic(d).catch(() => null);
    if (!r || r.policy("", "join") !== "anyone" || !r.apps().includes("chat")) continue;
    const n = r.members().length;
    // Join, Requested (while a member lets you in), Open: the one join control.
    const act = await joins.control(d, { open: `#/s/${d.id}/chat` });
    cards.push(h("div", { className: "card" }, h("b", { textContent: space.shown(d) }), h("span", { textContent: `${n} member${n === 1 ? "" : "s"} · ${r.apps().join(", ")}` }), act));
  }
  grid.replaceChildren(...(cards.length ? cards : [h("p", { className: "note", textContent: "No open spaces yet: a space's owner or admins open it (its Home → Permissions → Who may join: Anyone)." })]));
}
