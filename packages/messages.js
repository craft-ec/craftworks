// MESSAGES, a page: DIRECT conversations — the people you talk with, and the conversation open (the `room` component).
// No member list: a direct conversation is you and them. UI only: a direct conversation is `conversation`'s (a
// two-person space, begun by a welcome in the other's inbox), its messages `content`'s, names `directory`'s. Opening
// the page joins every conversation someone started with you while you were away.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [conversation, directory, roomUI, theme, activity] = await Promise.all(["conversation", "directory", "room", "theme", "activity"].map(n => ctx.require(n)));
  el.classList.add("cw-fill");
  el.innerHTML = `
    <style>
      .dm { display: grid; grid-template-columns: 260px 1fr; min-height: 420px;
        overflow: hidden; background: var(--cw-surface); }
      .dm button { font: inherit; cursor: pointer; }
      .dm .list { background: var(--cw-bg); border-right: 1px solid var(--cw-line); display: flex; flex-direction: column; min-width: 0; }
      .dm .list h2 { margin: 0; font-size: 1rem; padding: 12px var(--cw-space-4); border-bottom: 1px solid var(--cw-line); }
      .dm .people { flex: 1; overflow-y: auto; padding: var(--cw-space-2); display: grid; align-content: start; gap: 2px; }
      .dm .people button { text-align: left; border: 0; background: none; color: var(--cw-fg); padding: 8px var(--cw-space-2);
        border-radius: var(--cw-radius-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .dm .people button:hover { background: var(--cw-hover); }
      .dm .people button[aria-current="true"] { background: var(--cw-pressed); }
      .dm .new { margin: var(--cw-space-2); border: 1px solid var(--cw-line); background: none; color: var(--cw-accent);
        border-radius: var(--cw-radius-sm); padding: 8px; }
      .dm .room { min-width: 0; min-height: 0; }
      .dm .empty { color: var(--cw-muted); text-align: center; margin: auto; padding: var(--cw-space-5); }
      .dm .said { color: var(--cw-danger); font-size: var(--cw-text-sm); padding: 0 var(--cw-space-3); margin: 0; }
      .dm dialog.ask { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-4); width: min(420px, calc(100vw - 32px));
        box-shadow: var(--cw-shadow-lg); }
      .dm dialog.ask form { display: grid; gap: var(--cw-space-3); }
      .dm dialog.ask .row { display: flex; gap: var(--cw-space-2); justify-content: flex-end; }
      .dm dialog.ask button { border: 1px solid var(--cw-line); background: none; color: inherit; border-radius: var(--cw-radius-sm);
        padding: var(--cw-space-1) var(--cw-space-3); }
      .dm dialog.ask button[value="ok"] { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
      @media (max-width: 700px) { .dm { grid-template-columns: 1fr; } .dm .list { display: none; } .dm.listing .list { display: flex; }
        .dm.listing .room { display: none; } }
    </style>
    <div class="dm listing">
      <aside class="list"><h2>Messages</h2><button class="new" type="button">+ New message</button><div class="people"></div><p class="said" hidden></p></aside>
      <section class="room"><p class="empty">Pick a conversation, or start one.</p></section>
      <dialog class="ask"><form method="dialog"><label>Who? One person, or several separated by commas for a group — name#abc123 or did:craftec:… <input name="answer" autocomplete="off" required></label>
        <label>Group name (for several) <input name="group" autocomplete="off"></label>
        <div class="row"><button value="cancel" formnovalidate>Cancel</button><button value="ok">Start</button></div></form></dialog>
    </div>`;
  const $ = s => el.querySelector(s);
  // A click outside a question (on its backdrop) cancels it, as Esc does.
  $(".ask").addEventListener("click", e => e.target === e.currentTarget && e.currentTarget.close("cancel"));
  const box = $(".dm"), people = $(".people"), roomEl = $(".room"), said = $(".said");
  const say = m => ((said.textContent = m), (said.hidden = !m));
  let open = null, shown = null;
  // No conversation open: the personal space's apps (Messages is its messages).
  const spaceApps = await ctx.require("space-apps");
  ctx.actions["/messages"] = await spaceApps.menu(await (await ctx.require("space")).account(), "messages");
  dispatchEvent(new CustomEvent("craftworks:actions"));
  // While the inbox is read (conversations begun while this account was away), the list says so at its end.
  let checking = true;
  people.replaceChildren(theme.loading("Loading conversations…"));

  async function drawList() {
    const list = await conversation.list();
    const names = await Promise.all(list.map(sp => (sp.with ? directory.name(sp.with) : sp.name)));
    people.replaceChildren(
      ...list.map((sp, i) => {
        const b = Object.assign(document.createElement("button"), { type: "button", textContent: `${sp.kind === "group" ? "👥 " : ""}${names[i]}`, title: sp.with ?? "" });
        const n = activity.unread(sp.id);
        if (n) b.append(Object.assign(document.createElement("span"), { className: "cw-badge", textContent: String(n) }));
        b.setAttribute("aria-current", String(open?.id === sp.id));
        b.onclick = () => show(sp, names[i]);
        return b;
      }),
    );
    if (checking) people.append(theme.loading("Checking your inbox…", 1));
    else if (!list.length) people.replaceChildren(Object.assign(document.createElement("p"), { className: "empty", textContent: "No conversations yet." }));
  }

  async function show(sp, name) {
    open = sp;
    box.classList.remove("listing");
    drawList();
    shown?.close();
    shown = await roomUI.show(roomEl, sp, sp.kind === "group" ? name : `@${name}`);
    // The conversation's apps (the same space): its messages, and what was added.
    ctx.actions["/messages"] = await spaceApps.menu(sp, "messages");
    dispatchEvent(new CustomEvent("craftworks:actions"));
  }

  $(".new").onclick = () => {
    const d = $(".ask");
    const field = d.querySelector("input");
    field.value = "";
    d.onclose = async () => {
      const who = d.returnValue === "ok" ? field.value.trim() : "";
      if (!who) return;
      say("");
      try {
        const dids = await Promise.all(who.split(",").map(x => x.trim()).filter(Boolean).map(x => conversation.person(x)));
        if (dids.length > 1) {
          const sp = await conversation.group(dids, d.querySelector('input[name="group"]').value);
          await drawList();
          return show(sp, sp.name);
        }
        const did = dids[0];
        const sp = await conversation.direct(did);
        await show(sp, await directory.name(did));
      } catch (e) {
        say(`Could not start it: ${e?.message ?? e}`);
      }
    };
    d.showModal();
    field.focus();
  };

  await drawList();
  activity.onChange(() => el.isConnected && drawList());
  // Opened at a conversation (`#/messages/<id>`: from a person's Message): shown at once.
  const at = async () => {
    if (!ctx.sub) return;
    const sp = (await conversation.list()).find(c => c.id === ctx.sub);
    if (sp && open?.id !== sp.id) show(sp, sp.with ? await directory.name(sp.with) : sp.name);
  };
  at();
  addEventListener("craftworks:route", () => el.isConnected && at());
  // Conversations started with this person while they were away: joined now.
  conversation
    .accept()
    .catch(e => (ctx.log("conversation", { what: e?.message ?? String(e) }), []))
    .then(() => {
      checking = false;
      return drawList().then(at);
    });
}
