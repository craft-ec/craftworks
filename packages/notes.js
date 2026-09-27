// NOTES, an app in the manner of Google Keep: a "Take a note…" composer, notes as coloured cards in a masonry grid,
// pinned notes first, archive, search, and grid or list. A PRIVATE page: nothing shows until someone is logged in.
//
// The notes are one table of the ACCOUNT (the `data` service), the same on every node of the account. A note is one
// row: its key an id that sorts by creation, its value JSON { title, body, color, archived, edited }. A row that is
// not JSON (the first notes were plain text) is read as a body. PINS are not a note's field: they are the account's
// (the `pins` package, ref `notes:<id>`), with its one pin button. A note saved with the old `pinned` field is moved
// into the pins once, when Notes opens.
const COLORS = [
  ["", "Default"], ["#faafa8", "Coral"], ["#f39f76", "Peach"], ["#fff8b8", "Sand"], ["#e2f6d3", "Mint"],
  ["#b4ddd3", "Sage"], ["#d4e4ed", "Fog"], ["#aeccdc", "Storm"], ["#d3bfdb", "Dusk"], ["#f6e2dd", "Blossom"],
  ["#e9e3d4", "Clay"], ["#efeff1", "Chalk"],
];

export async function mount(ctx, el) {
  const auth = await ctx.require("auth");
  if (!(await auth.session())) {
    location.hash = "#/";
    return;
  }
  el.innerHTML = `
    <style>
      .keep { --line: #8884; position: relative; }
      .keep [hidden] { display: none !important; }
      .keep .composer { max-width: 600px; margin: 8px auto 28px; border: 1px solid var(--line); border-radius: 8px;
        box-shadow: 0 1px 3px #0002; padding: 10px 16px; display: grid; gap: 8px; }
      .keep .composer input, .keep .composer textarea, .keep .editor input, .keep .editor textarea { font: inherit;
        border: 0; outline: 0; background: transparent; color: inherit; resize: none; width: 100%; }
      .keep .composer .title, .keep .editor .title { font-weight: 600; }
      .keep .composer .row, .keep .editor .row { display: flex; align-items: center; gap: 6px; }
      .keep .row .end { margin-left: auto; }
      .keep .section { font-size: .72rem; letter-spacing: .08em; opacity: .7; margin: 18px 0 8px; }
      .keep .cards { columns: 240px; column-gap: 12px; }
      .keep.list .cards { columns: 1; max-width: 600px; margin: 0 auto; }
      .keep .card { break-inside: avoid; margin: 0 0 12px; border: 1px solid var(--line); border-radius: 8px;
        padding: 12px 14px 6px; cursor: default; position: relative; }
      .keep .card[style*="background"] { color: #202124; border-color: transparent; }
      .keep .card .t { font-weight: 600; margin-bottom: 6px; overflow-wrap: anywhere; }
      .keep .card .b { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 18em; overflow: hidden; }
      .keep .card .tools { display: flex; gap: 2px; opacity: 0; transition: opacity .15s; margin-top: 6px; }
      .keep .card:hover .tools, .keep .card:focus-visible .tools, .keep .card:has(:focus-visible) .tools { opacity: 1; }
      .keep .tools button, .keep .row button, .keep .pin { border: 0; background: none; cursor: pointer; font-size: 15px;
        padding: 4px 6px; border-radius: 50%; color: inherit; }
      .keep .tools button:hover, .keep .row button:hover { background: #0001; }
      /* The pin's look is the pins package's; here only where a card's sits, and that it shows on hover. */
      .keep .card .cw-pin { position: absolute; top: 6px; right: 6px; }
      .keep .card .cw-pin[aria-pressed="false"] { opacity: 0; }
      .keep .card:hover .cw-pin[aria-pressed="false"] { opacity: .45; }
      .keep .swatches { display: flex; flex-wrap: wrap; gap: 4px; padding: 6px; border: 1px solid var(--line);
        border-radius: 8px; background: Canvas; position: absolute; z-index: 5; width: max-content; max-width: 90vw;
        box-shadow: 0 2px 10px #0003; }
      .keep .swatches button { width: 26px; height: 26px; border-radius: 50%; border: 1px solid var(--line); }
      .keep .empty { text-align: center; opacity: .6; margin-top: 40px; }
      .keep .said { text-align: center; font-size: .9em; }
      .keep dialog.editor { position: fixed; overflow: visible; width: min(600px, calc(100vw - 32px)); border: 0; border-radius: 8px; padding: 14px 16px;
        box-shadow: 0 4px 24px #0006; display: grid; gap: 8px; }
      .keep dialog.editor[style*="background"] { color: #202124; }
      .keep dialog.editor:not([open]) { display: none; }
    </style>
    <div class="keep">
      <form class="composer">
        <input class="title" name="title" placeholder="Title" hidden>
        <textarea class="body" name="body" rows="1" placeholder="Take a note…"></textarea>
        <div class="row" hidden><button type="button" class="palette" title="Background">🎨</button>
          <button type="button" class="end close">Close</button></div>
      </form>
      <p class="said"></p>
      <div class="pinned-section" hidden><div class="section">PINNED</div><div class="cards pinned"></div></div>
      <div class="others-section"><div class="section others-label" hidden>OTHERS</div><div class="cards others"></div></div>
      <p class="empty" hidden></p>
      <dialog class="editor">
        <input class="title" name="title" placeholder="Title">
        <textarea class="body" name="body" rows="6" placeholder="Note"></textarea>
        <div class="row"><span class="pin-slot"></span>
          <button type="button" class="palette" title="Background">🎨</button>
          <button type="button" class="archive-e" title="Archive">🗃️</button>
          <button type="button" class="delete-e" title="Delete">🗑️</button>
          <button type="button" class="end done">Close</button></div>
      </dialog>
    </div>`;
  const root = el.querySelector(".keep");
  const said = t => (root.querySelector(".said").textContent = t);
  let notes, pins;
  try {
    const data = await ctx.require("data");
    [notes, pins] = await Promise.all([data.table("notes"), ctx.require("pins")]);
  } catch (e) {
    return said(`Could not open your notes: ${e?.message ?? e}`);
  }

  // A row as a note: JSON, or (the first notes) plain text as the body. `pinned` comes from the account's pins.
  const ref = key => `notes:${key}`;
  const note = r => {
    let n = { title: "", body: r.value, color: "", archived: false, edited: 0 };
    try {
      const j = JSON.parse(r.value);
      if (j && typeof j === "object") n = { title: "", body: "", color: "", archived: false, edited: 0, ...j };
    } catch {}
    return { ...n, key: r.key, pinned: pins.has(ref(r.key)) };
  };
  const save = (key, n) => {
    const { title, body, color, archived } = n;
    return notes.put(key, JSON.stringify({ title, body, color, archived, edited: Date.now() })).catch(e => said(`Could not save: ${e?.message ?? e}`));
  };
  // Notes saved with the old `pinned` field: into the pins, and saved without it (once; `save` never writes it).
  for (const r of notes.rows()) {
    let j;
    try {
      j = JSON.parse(r.value);
    } catch {
      continue;
    }
    if (j?.pinned !== true) continue;
    const n = note(r);
    (n.pinned ? Promise.resolve() : pins.set(ref(r.key), true))
      .then(() => notes.put(r.key, JSON.stringify({ title: n.title, body: n.body, color: n.color, archived: n.archived, edited: n.edited })))
      .catch(e => said(`Could not move a pin: ${e?.message ?? e}`));
  }
  const remove = key => notes.remove(key).catch(e => said(`Could not delete: ${e?.message ?? e}`));
  const newKey = () =>
    `${Date.now().toString(36).padStart(10, "0")}-${[...crypto.getRandomValues(new Uint8Array(4))].map(b => b.toString(16).padStart(2, "0")).join("")}`;

  // The view: Notes or Archive, grid or list, a search.
  let archive = false;
  let list = false;
  let query = "";

  // A colour picker under `anchor`; `pick(color)` on a choice.
  const palette = (anchor, pick) => {
    el.querySelector(".swatches")?.remove();
    const box = document.createElement("div");
    box.className = "swatches";
    for (const [c, name] of COLORS) {
      const b = document.createElement("button");
      b.type = "button";
      b.title = name;
      b.setAttribute("aria-label", name);
      b.style.background = c || "Canvas";
      b.onclick = e => {
        e.stopPropagation();
        box.remove();
        pick(c);
      };
      box.append(b);
    }
    // Inside the open editor when picked from it: while a dialog is open, everything outside it is inert.
    const host = anchor.closest("dialog") ?? root;
    const r = anchor.getBoundingClientRect();
    const base = host.getBoundingClientRect();
    box.style.left = `${r.left - base.left}px`;
    box.style.top = `${r.bottom - base.top + 4}px`;
    host.append(box);
    setTimeout(() => addEventListener("click", () => box.remove(), { once: true }));
  };
  const tint = (node, color) => (node.style.background = color || "");

  // THE COMPOSER: a line until clicked, then title and body; a note is made when it closes with something in it.
  const composer = root.querySelector(".composer");
  const [cTitle, cBody, cRow] = [".title", ".body", ".row"].map(q => composer.querySelector(q));
  let composing = { color: "" };
  const open = () => {
    cTitle.hidden = false;
    cRow.hidden = false;
    cBody.rows = 3;
  };
  const shut = async () => {
    const title = cTitle.value.trim();
    const body = cBody.value.trim();
    composer.reset();
    cTitle.hidden = true;
    cRow.hidden = true;
    cBody.rows = 1;
    const color = composing.color;
    composing = { color: "" };
    tint(composer, "");
    if (title || body) await save(newKey(), { title, body, color, archived: false });
  };
  cBody.addEventListener("focus", open);
  composer.querySelector(".close").onclick = shut;
  composer.querySelector(".palette").onclick = e => palette(e.currentTarget, c => ((composing.color = c), tint(composer, c)));
  addEventListener("click", e => {
    if (!composer.isConnected) return;
    if (!composer.contains(e.target) && !cTitle.hidden && !e.target.closest(".swatches")) shut();
  });

  // THE EDITOR: a note opened from its card; saved when it closes, if anything changed.
  const editor = root.querySelector("dialog.editor");
  const [eTitle, eBody] = [".title", ".body"].map(q => editor.querySelector(q));
  let editing = null;
  const edit = n => {
    editing = { ...n };
    eTitle.value = n.title;
    eBody.value = n.body;
    tint(editor, n.color);
    editor.querySelector(".pin-slot").replaceChildren(pins.button(ref(n.key)));
    editor.querySelector(".archive-e").title = n.archived ? "Unarchive" : "Archive";
    editor.showModal();
  };
  const finish = async () => {
    if (!editing) return;
    const n = { ...editing, title: eTitle.value.trim(), body: eBody.value.trim() };
    const before = note(notes.rows().find(r => r.key === n.key) ?? { key: n.key, value: "" });
    editing = null;
    editor.close();
    const changed = ["title", "body", "color", "archived"].some(k => n[k] !== before[k]);
    if (changed) await save(n.key, n);
  };
  editor.querySelector(".done").onclick = finish;
  editor.addEventListener("cancel", e => (e.preventDefault(), finish()));
  // A click outside the note (on the backdrop) closes it, as Close does. The backdrop's clicks land on the dialog
  // itself, outside its box.
  editor.addEventListener("click", e => {
    if (e.target !== editor || e.target.closest(".swatches")) return;
    const r = editor.getBoundingClientRect();
    if (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom) finish();
  });
  // Pin and colour take effect at once, as in Keep: the card behind the open note moves or changes while it is open.
  // Title and body are saved when it closes.
  const live = () => save(editing.key, { ...editing, title: eTitle.value.trim(), body: eBody.value.trim() });
  editor.querySelector(".palette").onclick = e =>
    palette(e.currentTarget, c => {
      editing.color = c;
      tint(editor, c);
      live();
    });
  editor.querySelector(".archive-e").onclick = () => ((editing.archived = !editing.archived), finish());
  editor.querySelector(".delete-e").onclick = async () => {
    const key = editing.key;
    editing = null;
    editor.close();
    await remove(key);
  };

  // A CARD.
  const card = n => {
    const c = document.createElement("div");
    c.className = "card";
    c.tabIndex = 0;
    tint(c, n.color);
    if (n.title) c.append(Object.assign(document.createElement("div"), { className: "t", textContent: n.title }));
    if (n.body) c.append(Object.assign(document.createElement("div"), { className: "b", textContent: n.body }));
    const pinButton = pins.button(ref(n.key));
    const tools = document.createElement("div");
    tools.className = "tools";
    const tool = (icon, title, run) => {
      const b = Object.assign(document.createElement("button"), { type: "button", textContent: icon, title });
      b.onclick = e => (e.stopPropagation(), run(e));
      tools.append(b);
    };
    tool("🎨", "Background", e => palette(e.currentTarget, color => save(n.key, { ...n, color })));
    tool(n.archived ? "📤" : "🗃️", n.archived ? "Unarchive" : "Archive", () => {
      save(n.key, { ...n, archived: !n.archived });
      if (!n.archived && n.pinned) pins.set(ref(n.key), false).catch(e => said(`Could not unpin: ${e?.message ?? e}`)); // an archived note is not pinned, as in Keep
    });
    tool("🗑️", "Delete", () => remove(n.key));
    c.append(pinButton, tools);
    c.onclick = () => edit(n);
    c.onkeydown = e => e.key === "Enter" && edit(n);
    return c;
  };

  const render = () => {
    root.classList.toggle("list", list);
    composer.hidden = archive;
    const q = query.toLowerCase();
    const all = notes
      .rows()
      .map(note)
      .filter(n => n.archived === archive)
      .filter(n => !q || `${n.title}\n${n.body}`.toLowerCase().includes(q))
      .sort((a, b) => (b.edited || 0) - (a.edited || 0) || (a.key < b.key ? 1 : -1));
    const pinned = all.filter(n => n.pinned && !archive);
    const others = all.filter(n => !n.pinned || archive);
    root.querySelector(".pinned-section").hidden = !pinned.length;
    root.querySelector(".others-label").hidden = !pinned.length || !others.length;
    root.querySelector(".pinned").replaceChildren(...pinned.map(card));
    root.querySelector(".others").replaceChildren(...others.map(card));
    const empty = root.querySelector(".empty");
    empty.hidden = all.length > 0;
    empty.textContent = q ? "No matching notes." : archive ? "Your archived notes appear here." : "Notes you add appear here.";
  };

  // THE TOP BAR while Notes is open: search, grid or list, and Notes or Archive.
  const actions = () => {
    ctx.actions["/notes"] = [
      { search: v => ((query = v), render()), placeholder: "Search your notes", value: query },
      { label: list ? "Grid view" : "List view", run: () => ((list = !list), render(), actions()) },
      { label: "Archive", on: archive, run: () => ((archive = !archive), render(), actions()) },
    ];
    dispatchEvent(new CustomEvent("craftworks:actions"));
  };
  actions();
  notes.onChange(() => root.isConnected && render());
  pins.onChange(() => root.isConnected && render());
  render();
}
