// CHAT, a page: DISCORD-style SERVERS — a rail of the servers you belong to, a server's CHANNELS, the channel open (the
// `room` component), and the server's MEMBERS. UI only: a server is a `space`, a channel a sub-space inheriting its
// access, its keys `keys`' (the server's group), its messages `content`'s; a server's channel list is a table of the
// server (`channels`: `<id>` → `{ name, at }`). (Direct messages are their own page: Messages.)
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [space, storage, keys, directory, roomUI, conversation, theme, roles, moderation, settings] = await Promise.all(
    ["space", "storage", "keys", "directory", "room", "conversation", "theme", "roles", "moderation", "server-settings"].map(n => ctx.require(n)),
  );
  const account = await space.account();
  el.classList.add("cw-fill");
  el.innerHTML = `
    <style>
      .dc { display: grid; grid-template-columns: 72px 240px 1fr 220px; min-height: 420px;
        overflow: hidden; background: var(--cw-surface); }
      .dc button { font: inherit; cursor: pointer; }
      .dc .rail { background: var(--cw-bg); border-right: 1px solid var(--cw-line); display: flex; flex-direction: column;
        align-items: center; gap: var(--cw-space-2); padding: var(--cw-space-3) 0; overflow-y: auto; }
      .dc .rail button { width: 48px; height: 48px; border-radius: 50%; border: 0; background: var(--cw-hover); color: var(--cw-fg);
        font-weight: 600; transition: border-radius .15s; }
      .dc .rail button:hover, .dc .rail button[aria-current="true"] { border-radius: 16px; background: var(--cw-accent); color: var(--cw-accent-fg); }
      .dc .rail .add { color: var(--cw-accent); font-size: 1.5rem; font-weight: 400; }
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
      @media (max-width: 800px) { .dc { grid-template-columns: 64px 180px 1fr; } .dc .people { display: none; } }
    </style>
    <div class="dc">
      <nav class="rail" aria-label="Servers"></nav>
      <aside class="side"><h2>—</h2><div class="chans"></div><p class="said" hidden></p></aside>
      <section class="room"><p class="empty">Pick or make a server</p></section>
      <aside class="people"><h3>MEMBERS</h3><ul></ul></aside>
      <dialog class="ask"><form method="dialog"><label><span></span><input name="answer" autocomplete="off" required></label>
        <div class="row"><button value="cancel" formnovalidate>Cancel</button><button value="ok">Create</button></div></form></dialog>
    </div>`;
  const $ = s => el.querySelector(s);
  const rail = $(".rail"), sideName = $(".side h2"), chans = $(".chans"), roomEl = $(".room"), people = $(".people ul"), said = $(".said");
  const say = m => ((said.textContent = m), (said.hidden = !m));
  const newId = n => [...crypto.getRandomValues(new Uint8Array(n))].map(x => x.toString(16).padStart(2, "0")).join("");
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

  let server = null, channelsT = null, channel = null, shown = null, rs = null, mod = null;
  const may = what => !!rs?.can(account.id, what);

  async function drawRail() {
    const servers = (await space.mine()).filter(s => s.kind === "server");
    rail.replaceChildren(
      ...servers.map(s => {
        const b = Object.assign(document.createElement("button"), { type: "button", title: s.name, textContent: s.name.slice(0, 2).toUpperCase() });
        b.setAttribute("aria-current", String(server?.id === s.id));
        b.onclick = () => openServer(s);
        return b;
      }),
      Object.assign(document.createElement("button"), { type: "button", className: "add", title: "Make a server, or join one with a code", textContent: "+", onclick: plus }),
    );
    return servers;
  }

  // +: make a server, or join one with an invite code.
  async function plus() {
    const answer = await ask("Name a new server — or paste an invite code (xxxx-xxxx-xxxx-xxxx) to join one", "Go");
    if (!answer) return;
    if (/^[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{4}$/i.test(answer.trim())) return joinWith(answer);
    return makeServer(answer);
  }
  async function joinWith(code) {
    say("");
    try {
      await conversation.join(code);
      say("Asked to join. You are in as soon as a member who may invite is online — this page looks every 30 s.");
    } catch (e) {
      say(`Could not ask to join: ${e?.message ?? e}`);
    }
  }

  async function makeServer(name) {
    if (!name) return;
    say("");
    try {
      const s = await space.create("server", name);
      await (await storage.table(space.tableOf(s, "channels"), s)).put(newId(4), JSON.stringify({ name: "general", at: Date.now() }));
      await openServer(s);
    } catch (e) {
      say(`Could not make the server: ${e?.message ?? e}`);
    }
  }

  // The top bar, once a server is open: Invite (by name or code) and the server's settings.
  const menu = () => {
    ctx.actions["/chat"] = server
      ? [
          ...(may("invite") ? [{ label: "Invite", run: () => openSettings("invites") }] : []),
          { label: "Server settings", run: () => openSettings("overview") },
        ]
      : [];
    dispatchEvent(new CustomEvent("craftworks:actions"));
  };
  // The server's CHANNELS, as settings changes them (one set of operations for the list here and the settings page).
  const channelOps = {
    list: () => list(),
    add: async name => {
      name = String(name ?? "").trim().toLowerCase().replace(/\s+/g, "-");
      if (!name) throw new Error("name it first");
      await channelsT.put(newId(4), JSON.stringify({ name, at: Date.now() }));
    },
    rename: async (c, name) => {
      name = String(name ?? "").trim().toLowerCase().replace(/\s+/g, "-");
      if (!name) throw new Error("name it first");
      await channelsT.put(c.id.split("/").pop(), JSON.stringify({ name, at: Date.now() }));
    },
    remove: async c => {
      await mod.hide(channelsT.app, c.id.split("/").pop());
      if (channel?.id === c.id) {
        channel = null;
        shown?.close();
        roomEl.replaceChildren();
      }
    },
  };
  const openSettings = (tab, focus = null) =>
    server &&
    settings.open(server, {
      tab,
      focus,
      channels: channelOps,
      // Left: off the rail, and the next server open.
      left: async () => {
        server = null;
        shown?.close();
        const next = (await drawRail())[0];
        if (next) openServer(next);
        else {
          menu();
          sideName.textContent = "—";
          chans.replaceChildren();
          people.replaceChildren();
          roomEl.replaceChildren(Object.assign(document.createElement("p"), { className: "empty", textContent: "No servers yet: make one with +" }));
        }
      },
    });

  async function openServer(s) {
    server = s;
    channel = null;
    channelsT = null;
    menu();
    sideName.textContent = s.name;
    drawRail();
    chans.replaceChildren(theme.loading("Loading channels…"));
    people.replaceChildren(theme.loading("Loading members…", 2));
    roomEl.replaceChildren(theme.loading(`Opening ${s.name}…`));
    try {
      await keys.group(s).ready();
      [rs, mod] = await Promise.all([roles.of(s), moderation.of(s)]);
      await rs.refresh();
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
        menu();
      });
      const t = (channelsT = await storage.table(space.tableOf(s, "channels"), s));
      t.onChange(() => server === s && drawChannels());
      t.settled.finally(() => {
        t.done = true;
        if (server !== s) return;
        drawChannels();
        if (!channel && list()[0]) openChannel(list()[0]);
      });
      drawChannels();
      drawMembers(s);
      const first = list()[0];
      if (first) openChannel(first);
      else roomEl.replaceChildren();
    } catch (e) {
      say(`Could not open the server: ${e?.message ?? e}`);
    }
  }

  // The server's channels: those made by someone who may make channels, less the ones hidden (deleted).
  const list = () => {
    const hidden = mod?.hidden(channelsT.app) ?? new Set();
    return channelsT
      .rows()
      .filter(r => !hidden.has(r.key) && rs?.can(rs.author(r), "channels"))
      .map(r => {
        let v = {};
        try {
          v = JSON.parse(r.value);
        } catch {}
        return space.channel(server, r.key, v.name ?? r.key);
      })
      .sort((a, b) => a.name.localeCompare(b.name));
  };

  function drawChannels() {
    if (!channelsT) return;
    // A server's channels come from its members' feeds: none yet is not "none" until every one has been tried.
    if (!list().length && !channelsT.done) return chans.replaceChildren(theme.loading("Loading channels…"));
    chans.replaceChildren(
      ...list().map(c => {
        const row = Object.assign(document.createElement("div"), { className: "ch" });
        const b = Object.assign(document.createElement("button"), { type: "button", textContent: `# ${c.name}` });
        b.setAttribute("aria-current", String(channel?.id === c.id));
        b.onclick = () => openChannel(c);
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

  // The server's members: the accounts in its group, with their roles. A click opens that person in the settings.
  async function drawMembers(s) {
    const dids = await conversation.members(s);
    if (server !== s || !rs) return;
    if (!dids.length) return people.replaceChildren();
    const draw = names =>
      people.replaceChildren(
        ...dids.map((d, i) => {
          const li = Object.assign(document.createElement("li"), { title: d, className: "who", onclick: () => openSettings("members", d) });
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
    drawChannels();
    shown?.close();
    shown = await roomUI.show(roomEl, c, `#${c.name}`);
  }

  // Welcomes waiting in the inbox: joined now, and every 30 s while Chat is open (a request by code answered). And
  // this person, where they may invite, lets in who asked by a code of the open server.
  const tick = async () => {
    const joined = await conversation.accept().catch(e => (ctx.log("conversation", { what: e?.message ?? String(e) }), []));
    if (joined.some(s => s.kind === "server")) {
      say("");
      drawRail();
    }
    if (server && may("invite")) {
      const s = server;
      const admitted = await conversation.admit(s).catch(e => (ctx.log("conversation", { what: e?.message ?? String(e) }), []));
      if (admitted.length && server === s) drawMembers(s);
    }
  };
  tick();
  const every = setInterval(() => (el.isConnected ? tick() : clearInterval(every)), 30000);
  rail.replaceChildren(theme.loading("", 2));
  chans.replaceChildren(theme.loading("Loading your servers…"));
  const first = (await drawRail())[0];
  if (first) openServer(first);
  else chans.replaceChildren(Object.assign(document.createElement("p"), { className: "empty", textContent: "No servers yet: make one with +" }));
}
