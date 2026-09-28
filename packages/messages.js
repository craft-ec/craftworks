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
  const [conversation, directory, roomUI] = await Promise.all(["conversation", "directory", "room"].map(n => ctx.require(n)));
  el.innerHTML = `
    <style>
      .dm { display: grid; grid-template-columns: 260px 1fr; height: calc(100vh - 120px); min-height: 420px;
        border: 1px solid var(--cw-line); border-radius: var(--cw-radius); overflow: hidden; background: var(--cw-surface); }
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
      <dialog class="ask"><form method="dialog"><label>Their id (did:craftec:…) <input name="answer" autocomplete="off" required></label>
        <div class="row"><button value="cancel" formnovalidate>Cancel</button><button value="ok">Start</button></div></form></dialog>
    </div>`;
  const $ = s => el.querySelector(s);
  const box = $(".dm"), people = $(".people"), roomEl = $(".room"), said = $(".said");
  const say = m => ((said.textContent = m), (said.hidden = !m));
  let open = null, shown = null;

  async function drawList() {
    const list = await conversation.list();
    const names = await Promise.all(list.map(sp => (sp.with ? directory.name(sp.with) : sp.name)));
    people.replaceChildren(
      ...list.map((sp, i) => {
        const b = Object.assign(document.createElement("button"), { type: "button", textContent: names[i], title: sp.with ?? "" });
        b.setAttribute("aria-current", String(open?.id === sp.id));
        b.onclick = () => show(sp, names[i]);
        return b;
      }),
    );
    if (!list.length) people.replaceChildren(Object.assign(document.createElement("p"), { className: "empty", textContent: "No conversations yet." }));
  }

  async function show(sp, name) {
    open = sp;
    box.classList.remove("listing");
    drawList();
    shown?.close();
    shown = await roomUI.show(roomEl, sp, `@${name}`);
  }

  $(".new").onclick = () => {
    const d = $(".ask");
    const field = d.querySelector("input");
    field.value = "";
    d.onclose = async () => {
      const did = d.returnValue === "ok" ? field.value.trim() : "";
      if (!did) return;
      say("");
      try {
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
  // Conversations started with this person while they were away: joined now.
  conversation
    .accept()
    .then(joined => joined.length && drawList())
    .catch(e => ctx.log("conversation", { what: e?.message ?? String(e) }));
}
