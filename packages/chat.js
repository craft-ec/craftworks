// CHAT, a page: a DISCORD-style group chat — a rail of the SERVERS you belong to, a server's CHANNELS, a channel's
// MESSAGES and composer, and the server's MEMBERS. UI only: what a server and a channel ARE is `space`'s (a server is a
// space; a channel is a sub-space inheriting its access), its keys are `keys`' (the server's group), a message is
// `content`'s (an authored item in its channel, in its author's own feed), and a server's channel list is a table of
// the server (`channels`: `<id>` → `{ name, at }`).
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [space, storage, keys, node, content] = await Promise.all(["space", "storage", "keys", "node", "content"].map(n => ctx.require(n)));
  const account = await space.account();
  el.innerHTML = `
    <style>
      .dc { display: grid; grid-template-columns: 72px 240px 1fr 220px; height: calc(100vh - 120px); min-height: 420px;
        border: 1px solid var(--cw-line); border-radius: var(--cw-radius); overflow: hidden; background: var(--cw-surface); }
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
      .dc h2, .dc h3 { margin: 0; font-size: 1rem; }
      .dc .side h2, .dc .room h2 { padding: 12px var(--cw-space-4); border-bottom: 1px solid var(--cw-line); overflow: hidden;
        text-overflow: ellipsis; white-space: nowrap; }
      .dc .chans { flex: 1; overflow-y: auto; padding: var(--cw-space-2); display: grid; align-content: start; gap: 2px; }
      .dc .chans button { text-align: left; border: 0; background: none; color: var(--cw-muted); padding: 6px var(--cw-space-2);
        border-radius: var(--cw-radius-sm); }
      .dc .chans button:hover { background: var(--cw-hover); color: var(--cw-fg); }
      .dc .chans button[aria-current="true"] { background: var(--cw-pressed); color: var(--cw-fg); }
      .dc .chans .new { color: var(--cw-accent); }
      .dc .room { display: flex; flex-direction: column; min-width: 0; }
      .dc .msgs { flex: 1; overflow-y: auto; margin: 0; padding: var(--cw-space-3) var(--cw-space-4); list-style: none; display: grid;
        align-content: end; gap: var(--cw-space-3); }
      .dc .msg .who { font-weight: 600; margin-right: var(--cw-space-2); }
      .dc .msg time { color: var(--cw-muted); font-size: var(--cw-text-xs); }
      .dc .msg .text { white-space: pre-wrap; overflow-wrap: anywhere; }
      .dc .compose { padding: var(--cw-space-3) var(--cw-space-4) var(--cw-space-4); }
      .dc .compose input { width: 100%; box-sizing: border-box; padding: 10px var(--cw-space-3); border-radius: var(--cw-radius); }
      .dc .people h3 { font-size: var(--cw-text-xs); letter-spacing: .08em; color: var(--cw-muted); margin-bottom: var(--cw-space-2); }
      .dc .people li { list-style: none; padding: var(--cw-space-1) 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .dc .people ul { margin: 0; padding: 0; }
      .dc .empty { color: var(--cw-muted); text-align: center; margin: auto; padding: var(--cw-space-5); }
      .dc .said { color: var(--cw-danger); font-size: var(--cw-text-sm); padding: 0 var(--cw-space-4); }
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
      <aside class="side"><h2>—</h2><div class="chans"></div></aside>
      <section class="room"><h2>—</h2><ol class="msgs"></ol><p class="said" hidden></p>
        <form class="compose"><input name="text" autocomplete="off" disabled placeholder="Pick or make a server"></form></section>
      <aside class="people"><h3>MEMBERS</h3><ul></ul></aside>
      <dialog class="ask"><form method="dialog"><label><span></span><input name="answer" autocomplete="off" required></label>
        <div class="row"><button value="cancel" formnovalidate>Cancel</button><button value="ok">Create</button></div></form></dialog>
    </div>`;
  const $ = s => el.querySelector(s);
  const rail = $(".rail"), sideName = $(".side h2"), chans = $(".chans"), roomName = $(".room h2");
  const msgs = $(".msgs"), compose = $(".compose"), input = compose.elements.text, people = $(".people ul"), said = $(".said");
  const say = m => ((said.textContent = m), (said.hidden = !m));
  const short = did => `${did.replace(/^did:craftec:/, "").slice(0, 8)}…`;
  const didOfCred = hex => {
    // A credential: `CWMB ‖ did (32) ‖ …`: its account.
    return node.glue.did_of(new Uint8Array(hex.match(/../g).slice(4, 36).map(x => parseInt(x, 16))));
  };
  // One question, in the page's own dialog: the answer, or null.
  const ask = question =>
    new Promise(resolve => {
      const d = $(".ask");
      d.querySelector("span").textContent = question;
      const field = d.querySelector("input");
      field.value = "";
      d.onclose = () => resolve(d.returnValue === "ok" ? field.value.trim() || null : null);
      d.showModal();
      field.focus();
    });
  const newId = n => [...crypto.getRandomValues(new Uint8Array(n))].map(x => x.toString(16).padStart(2, "0")).join("");

  let servers = [];
  let server = null, channelsT = null, channel = null, room = null;

  async function drawRail() {
    servers = await space.mine();
    rail.replaceChildren(
      ...servers.filter(s => s.kind === "server").map(s => {
        const b = Object.assign(document.createElement("button"), { type: "button", title: s.name, textContent: s.name.slice(0, 2).toUpperCase() });
        b.setAttribute("aria-current", String(server?.id === s.id));
        b.onclick = () => openServer(s);
        return b;
      }),
      Object.assign(document.createElement("button"), { type: "button", className: "add", title: "Make a server", textContent: "+", onclick: makeServer }),
    );
  }

  async function makeServer() {
    const name = await ask("Name your server");
    if (!name) return;
    say("");
    try {
      const s = await space.create("server", name);
      const t = await storage.table(space.tableOf(s, "channels"), s);
      await t.put(newId(4), JSON.stringify({ name: "general", at: Date.now() }));
      await drawRail();
      await openServer(s);
    } catch (e) {
      say(`Could not make the server: ${e?.message ?? e}`);
    }
  }

  async function openServer(s) {
    server = s;
    sideName.textContent = s.name;
    drawRail();
    chans.replaceChildren(Object.assign(document.createElement("p"), { className: "empty", textContent: "Opening…" }));
    try {
      await keys.group(s).ready();
      channelsT = await storage.table(space.tableOf(s, "channels"), s);
      channelsT.onChange(() => server === s && drawChannels());
      drawChannels();
      drawMembers();
      const first = list()[0];
      if (first) openChannel(first);
    } catch (e) {
      say(`Could not open the server: ${e?.message ?? e}`);
    }
  }

  const list = () =>
    channelsT.rows().map(r => {
      let v = {};
      try { v = JSON.parse(r.value); } catch {}
      return space.channel(server, r.key, v.name ?? r.key);
    }).sort((a, b) => a.name.localeCompare(b.name));

  function drawChannels() {
    chans.replaceChildren(
      ...list().map(c => {
        const b = Object.assign(document.createElement("button"), { type: "button", textContent: `# ${c.name}` });
        b.setAttribute("aria-current", String(channel?.id === c.id));
        b.onclick = () => openChannel(c);
        return b;
      }),
      Object.assign(document.createElement("button"), { type: "button", className: "new", textContent: "+ Add a channel", onclick: addChannel }),
    );
  }

  async function addChannel() {
    const name = (await ask("Channel name"))?.toLowerCase().replace(/\s+/g, "-");
    if (!name) return;
    await channelsT.put(newId(4), JSON.stringify({ name, at: Date.now() })).catch(e => say(`Could not add it: ${e?.message ?? e}`));
  }

  async function drawMembers() {
    const st = await keys.group(server).ready().catch(() => null);
    const dids = [...new Set((st?.members ?? []).map(m => (m.cred ? didOfCred(m.cred) : m.key)))];
    people.replaceChildren(
      ...dids.map(d => Object.assign(document.createElement("li"), { textContent: d === account.id ? `${short(d)} (you)` : short(d), title: d })),
    );
  }

  async function openChannel(c) {
    channel = c;
    drawChannels();
    roomName.textContent = `# ${c.name}`;
    input.placeholder = `Message #${c.name}`;
    input.disabled = true;
    msgs.replaceChildren();
    room = await content.in(c);
    room.onChange(() => channel === c && drawMessages());
    drawMessages();
    input.disabled = false;
    input.focus();
  }

  function drawMessages() {
    const rows = room.list();
    if (!rows.length) {
      msgs.replaceChildren(Object.assign(document.createElement("li"), { className: "empty", textContent: `This is the start of #${channel.name}.` }));
      return;
    }
    msgs.replaceChildren(
      ...rows.map(m => {
        const li = Object.assign(document.createElement("li"), { className: "msg" });
        const who = Object.assign(document.createElement("span"), { className: "who", textContent: m.by === account.id ? "you" : short(m.by ?? "?"), title: m.by ?? "" });
        const time = Object.assign(document.createElement("time"), { textContent: new Date(m.at).toLocaleString() });
        const text = Object.assign(document.createElement("div"), { className: "text", textContent: m.body });
        li.append(who, time, text);
        return li;
      }),
    );
    msgs.scrollTop = msgs.scrollHeight;
  }

  compose.onsubmit = async e => {
    e.preventDefault();
    const text = input.value.trim();
    if (!text || !room) return;
    input.value = "";
    say("");
    await room.post("message", text).catch(err => {
      input.value = text;
      say(`Not sent: ${err?.message ?? err}`);
    });
  };

  await drawRail();
  const first = servers.find(s => s.kind === "server");
  if (first) openServer(first);
  else chans.replaceChildren(Object.assign(document.createElement("p"), { className: "empty", textContent: "No servers yet: make one with +" }));
}

