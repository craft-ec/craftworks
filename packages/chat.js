// CHAT, a page: a SHARED SPACE's chat, Discord-style — the space chosen on the rail (`#/s/<space>/chat[/<channel>]`):
// its CHANNELS, the channel open (the `room` component), and its MEMBERS. Chat is a shared space's app only (direct and
// group conversations are Messages, in the personal space); the space itself — made, joined, its settings — is the rail's
// and its Home's. UI only: a space is `space`'s, a channel a sub-space inheriting its access, its keys `keys`' (the
// space's group), its messages `content`'s; its channel list is a table of the space (`channels`: `<id>` → `{ name, at }`).
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [space, keys, directory, roomUI, conversation, theme, roles, moderation, person, activity, appSettings] = await Promise.all(
    ["space", "keys", "directory", "room", "conversation", "theme", "roles", "moderation", "person", "activity", "app-settings"].map(n => ctx.require(n)),
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
      .dc dialog.ask { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-4); width: min(360px, calc(100vw - 32px));
        box-shadow: var(--cw-shadow-lg); }
      .dc dialog.ask form { display: grid; gap: var(--cw-space-3); }
      .dc dialog.ask .row { display: flex; gap: var(--cw-space-2); justify-content: flex-end; }
      .dc dialog.ask button { border: 1px solid var(--cw-line); background: none; color: inherit; border-radius: var(--cw-radius-sm);
        padding: var(--cw-space-1) var(--cw-space-3); }
      .dc dialog.ask button[value="ok"] { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
      @media (max-width: 800px) { .dc { grid-template-columns: 180px 1fr; } .dc .people { display: none; } }
    </style>
    <div class="dc">
      <aside class="side"><h2>—</h2><div class="chans"></div><p class="said" hidden></p></aside>
      <section class="room"></section>
      <aside class="people"><h3>MEMBERS</h3><ul></ul></aside>
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
      await keys.group(s).ready();
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
      const c = (chs = await conversation.channels(s));
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

  function drawChannels() {
    if (!chs) return;
    // A server's channels come from its members' feeds: none yet is not "none" until every one has been tried.
    if (!list().length && !chs.done) return chans.replaceChildren(theme.loading("Loading channels…"));
    chans.replaceChildren(
      ...list().map(c => {
        const row = Object.assign(document.createElement("div"), { className: "ch" });
        const b = Object.assign(document.createElement("button"), { type: "button", textContent: `# ${c.name}` });
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
    const dids = await conversation.members(s);
    if (server !== s || !rs) return;
    if (!dids.length) return people.replaceChildren();
    const draw = names =>
      people.replaceChildren(
        ...dids.map((d, i) => {
          const li = Object.assign(document.createElement("li"), { title: d, className: "who", onclick: e => person.open(e.currentTarget, d, { space: s }) });
          const role = rs.role(d);
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
  activity.onChange(() => el.isConnected && drawChannels());
  // THE SPACE on the rail (`ctx.space`), at a channel (`ctx.sub`: a notification clicked, a link) or its first.
  const at = async () => {
    const s = ctx.space && (await space.mine()).find(x => x.id === ctx.space && x.kind === "server");
    if (!s) {
      server = null;
      sideName.textContent = "—";
      chans.replaceChildren();
      people.replaceChildren();
      roomEl.replaceChildren(
        Object.assign(document.createElement("p"), { className: "empty", textContent: "Chat is a shared space's: pick a space on the left, or make one with +." }),
      );
      return;
    }
    const cid = ctx.sub || null;
    if (server?.id !== s.id) return openServer(s, cid);
    const c = cid && list().find(x => x.id.endsWith(`/${cid}`));
    if (c && channel?.id !== c.id) openChannel(c);
  };
  // CHAT'S OWN SETTINGS (its owner and admins): who may post, and its channels. The space's settings are its Home's.
  const CHANNEL_ROW = c => {
    const name = Object.assign(document.createElement("input"), { value: c.name, ariaLabel: "Channel name" });
    const row = Object.assign(document.createElement("div"), { className: "row" });
    const b = (t, f) => Object.assign(document.createElement("button"), { type: "button", textContent: t, onclick: f });
    row.append(name, b("Rename", () => channelOps.rename(c, name.value.trim()).catch(e => say(e.message))), b("Delete", () => channelOps.remove(c).then(() => row.remove(), e => say(e.message))));
    return row;
  };
  const chatSettings = () =>
    appSettings.open(server, "Chat settings", [{ action: "post", path: "chat", label: "Who may post (in every channel that does not say otherwise)" }], {
      extra: async (host, r) => {
        const draw = () => {
          const add = Object.assign(document.createElement("input"), { placeholder: "New channel", ariaLabel: "New channel" });
          const go = Object.assign(document.createElement("button"), { type: "button", textContent: "Add", onclick: () => add.value.trim() && channelOps.add(add.value.trim()).then(draw, e => say(e.message)) });
          const row = Object.assign(document.createElement("div"), { className: "row" });
          row.append(add, go);
          // Each channel: its name, and who may post in it (inherited from Chat's, or its own).
          host.replaceChildren(
            Object.assign(document.createElement("h4"), { textContent: "Channels" }),
            ...list().map(c => {
              const line = CHANNEL_ROW(c);
              line.append(appSettings.who(r, `chat/${c.id.split("/").pop()}`, "post"));
              return line;
            }),
            row,
          );
        };
        draw();
      },
    });
  const menu = () => {
    ctx.actions["/chat"] = server && rs?.can(account.id, "apps") ? [{ label: "Chat settings", run: chatSettings }] : [];
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
