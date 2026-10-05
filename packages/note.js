// NOTES, an app in the manner of Google Keep: a "Take a note…" composer, notes as coloured cards in a masonry grid,
// pinned notes first, archive, search, and grid or list. A PRIVATE page: nothing shows until someone is logged in.
// YOUR notes (`#/note`: the personal space's), or a SHARED space's (`#/s/<space>/note`, the space open,
// when it uses Notes — its table `notes`, written and read by its members).
//
// The notes are one table of the ACCOUNT (the `data` service), the same on every node of the account. A note is one
// row: its key an id that sorts by creation, its value JSON { title, body, color, archived, edited }. A row that is
// not JSON (the first notes were plain text) is read as a body. PINS are not a note's field: they are the account's
// (the `edge` capability's pins, ref `notes:<id>`), shown with the one `pin-button`. A note saved with the old `pinned` field is moved
// into the pins once, when Notes opens. LABELS are the account's private tags (the `edge` capability; Keep's word here), shown with `label-menu`: a note's labels show
// as chips, 🏷️ opens the label menu, the label bar shows one label's notes, and "Edit labels" makes, renames and
// deletes them.
const COLORS = [
  ["", "Default"], ["#faafa8", "Coral"], ["#f39f76", "Peach"], ["#fff8b8", "Sand"], ["#e2f6d3", "Mint"],
  ["#b4ddd3", "Sage"], ["#d4e4ed", "Fog"], ["#aeccdc", "Storm"], ["#d3bfdb", "Dusk"], ["#f6e2dd", "Blossom"],
  ["#e9e3d4", "Clay"], ["#efeff1", "Chalk"],
];

export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  el.innerHTML = `
    <style>
      .keep { position: relative; }
            .keep .composer { max-width: 600px; margin: 8px auto 28px; border: 1px solid var(--cw-line); border-radius: var(--cw-radius);
        box-shadow: var(--cw-shadow); padding: 10px var(--cw-space-4); display: grid; gap: var(--cw-space-2); background: var(--cw-surface); }
      .keep .composer input, .keep .composer textarea, .keep .editor input, .keep .editor textarea { font: inherit;
        border: 0; outline: 0; background: transparent; color: inherit; resize: none; width: 100%; }
      .keep .composer .title, .keep .editor .title { font-weight: 600; }
      .keep .composer .row, .keep .editor .row { display: flex; align-items: center; gap: 6px; }
      .keep .row .end { margin-left: auto; }
      .keep .section { font-size: var(--cw-text-xs); letter-spacing: .08em; color: var(--cw-muted); margin: 18px 0 var(--cw-space-2); }
      .keep .cards { columns: 240px; column-gap: 12px; }
      .keep.list .cards { columns: 1; max-width: 600px; margin: 0 auto; }
      .keep .tools button, .keep .row button, .keep .pin { border: 0; background: none; cursor: pointer; font-size: 15px;
        padding: var(--cw-space-1) 6px; border-radius: 50%; color: inherit; }
      .keep .tools button:hover, .keep .row button:hover { background: var(--cw-hover); }
      .keep .swatches { display: flex; flex-wrap: wrap; gap: var(--cw-space-1); padding: 6px; border: 1px solid var(--cw-line);
        border-radius: var(--cw-radius); background: var(--cw-surface); position: absolute; z-index: 5; width: max-content; max-width: 90vw;
        box-shadow: var(--cw-shadow-lg); }
      .keep .swatches button { width: 26px; height: 26px; border-radius: 50%; border: 1px solid var(--cw-line); }
      .keep .empty { text-align: center; color: var(--cw-muted); margin-top: 40px; }
      .keep .said { text-align: center; font-size: var(--cw-text-sm); }
      .keep dialog.editor { position: fixed; overflow: visible; width: min(600px, calc(100vw - 32px)); border: 0; border-radius: var(--cw-radius); padding: 14px var(--cw-space-4);
        box-shadow: var(--cw-shadow-lg); display: grid; gap: var(--cw-space-2); }
      .keep dialog.editor[style*="background"] { color: var(--cw-on-pastel); }
      .keep dialog.editor:not([open]) { display: none; }
      .keep .labelbar { display: flex; flex-wrap: wrap; gap: 6px; justify-content: center; margin: 0 auto 12px; max-width: 800px; }
      .keep .labelbar:has(> :only-child) { display: none; }
      .keep .labelbar button { font-size: var(--cw-text-sm); border: 1px solid var(--cw-line); background: none;
        border-radius: var(--cw-radius-pill); padding: 3px var(--cw-space-3); cursor: pointer; }
      .keep .labelbar button:hover { background: var(--cw-hover); }
      .keep .labelbar button[aria-pressed="true"] { background: var(--cw-selected); color: var(--cw-on-selected); border-color: transparent; }
      .keep .labelbar .edit-labels { border-style: dashed; }
      .keep dialog.labels-editor { border: 0; border-radius: var(--cw-radius); padding: 14px var(--cw-space-4); width: min(320px, calc(100vw - 32px));
        box-shadow: var(--cw-shadow-lg); }
      .keep dialog.labels-editor:not([open]) { display: none; }
      .keep .labels-editor h3 { margin: 0 0 10px; font-size: 1rem; }
      .keep .labels-editor .line { display: flex; gap: 6px; align-items: center; margin: 4px 0; }
      .keep .labels-editor input { flex: 1; font: inherit; border: 0; border-bottom: 1px solid var(--cw-line); border-radius: 0; background: transparent;
        color: inherit; outline: 0; padding: 3px 0; }
      .keep .labels-editor button { border: 0; background: none; cursor: pointer; font: inherit; color: inherit; }
      .keep .labels-editor .done-l { display: block; margin: 12px 0 0 auto; }
    </style>
    <div class="keep">
      <div class="labelbar"></div>
      <form class="composer">
        <input class="title" name="title" placeholder="Title" hidden>
        <div class="body"></div>
        <div class="row" hidden><button type="button" class="palette" title="Background">🎨</button>
          <span class="audience-slot"></span>
          <button type="button" class="end close">Close</button></div>
      </form>
      <p class="said"></p>
      <div class="pinned-section" hidden><div class="section">PINNED</div><div class="cards pinned"></div></div>
      <div class="others-section"><div class="section others-label" hidden>OTHERS</div><div class="cards others"></div></div>
      <p class="empty" hidden></p>
      <dialog class="editor">
        <input class="title" name="title" placeholder="Title">
        <div class="body"></div>
        <div class="label-slot"></div>
        <div class="row"><span class="pin-slot"></span>
          <button type="button" class="palette" title="Background">🎨</button>
          <button type="button" class="label-e" title="Labels">🏷️</button>
          <button type="button" class="archive-e" title="Archive">🗃️</button>
          <button type="button" class="delete-e" title="Delete">🗑️</button>
          <button type="button" class="end done">Close</button></div>
      </dialog>
      <dialog class="labels-editor">
        <h3>Edit labels</h3>
        <form class="line new-label"><input name="name" placeholder="Create new label" maxlength="50"><button title="Create">✔️</button></form>
        <div class="label-lines"></div>
        <button type="button" class="done-l">Done</button>
      </dialog>
    </div>`;
  const root = el.querySelector(".keep");
  const said = t => (root.querySelector(".said").textContent = t);
  let items, who, edge, pins, labels, pinUI, labelUI, sp = null, rs = null, at = null;
  const meId = (await (await ctx.require("space")).account()).id;
  try {
    edge = await ctx.require("edge");
    // WHERE (`where`: yours, a space's, a person's, Discover) — a space's notes by its policies.
    at = await (await ctx.require("where")).of({ kind: "note", app: "note", yours: "Your notes" });
    sp = at.space;
    if (sp) {
      rs = await (await ctx.require("roles")).of(sp);
      // The space's log read first (its settings are acts in it; whether it uses Note: `where`'s).
      await rs.settled;
    }
    // NOTES ARE ITEMS (kind `note`, `items`): yours in your space (private, or public), a space's in its place —
    // who sees, edits and comments by the same access control as every app.
    [items, pins, labels, pinUI, labelUI] = await Promise.all([
      ctx.require("items"),
      edge.pins(),
      edge.labels(),
      ctx.require("pin-button"),
      ctx.require("label-menu"),
    ]);
  } catch (e) {
    return said(`Could not open the notes: ${e?.message ?? e}`);
  }
  // A NOTE's PAGE (`…/note/p/<ref>`): the one item page — the note, its comments — framed here.
  if (at.sub.startsWith("p/")) {
    const page = await (await ctx.require("item-page")).show(decodeURIComponent(at.sub.slice(2)), { app: "note", back: at.base, discover: at.who === "discover" });
    root.replaceChildren(Object.assign(document.createElement("a"), { href: at.base, textContent: "← Notes" }), page);
    const here = (await ctx.require("where")).key();
    const away = async () => {
      if (!root.isConnected || ctx.route !== "/note") return removeEventListener("craftworks:route", away);
      if ((await ctx.require("where")).key() === here) return;
      removeEventListener("craftworks:route", away);
      el.replaceChildren();
      mount(ctx, el);
    };
    addEventListener("craftworks:route", away);
    return;
  }
  // WHO SEES a new note: the one picker — yours start "Only you", a space's its members.
  who = await (await ctx.require("audience")).picker({ space: sp, kind: "note", initial: sp ? "members" : "private" });
  root.querySelector(".audience-slot").append(who.el);
  // Someone else's (a person's, Discover's): read only, as every app shows what is not yours (`where`).
  const others = at.others;
  let held = [];
  const reload = () =>
    at
      .read({ whole: true })
      .then(x => ((held = x), root.isConnected && render()))
      .catch(e => said(`Could not read the notes: ${e?.message ?? e}`));

  // A note: an item (its key its ref). `pinned` comes from the account's pins; a pin or a label names it `notes:<ref>`.
  // Its LABEL key: an item's, as every kind's (`label-menu`).
  const ref = key => labelUI.key(key);
  const note = it => ({
    key: it.ref,
    title: it.title ?? "",
    body: it.body ?? "",
    color: it.meta?.color ?? "",
    archived: !!it.meta?.archived,
    edited: it.edited || it.at || 0,
    mayEdit: !others && it.mayEdit !== false,
    by: it.by,
    item: it,
    meta: it.meta ?? {},
    pinned: pins.has(ref(it.ref)),
  });
  // SAVE: a change to a note held (its editors: `items.editItem`), or a new one made (`items.submit`): its ref.
  const save = async (key, n) => {
    const { title, body, color, archived, files } = n;
    try {
      const was = key && held.find(x => x.ref === key);
      if (was) await items.editItem(key, body, { title, meta: { ...(was.meta ?? {}), color, archived }, ...(files ? { files } : {}) });
      else key = await items.submit({ board: sp?.id ?? null, title, body, kind: "note", meta: { color, archived }, files: files ?? [], audience: n.audience ?? who.value(), write: who.write() });
      reload();
      return key;
    } catch (e) {
      said(`Could not save: ${e?.message ?? e}`);
      return null;
    }
  };
  const remove = key =>
    items
      .remove(key)
      .then(() => labels.clear(ref(key)))
      .then(reload)
      .catch(e => said(`Could not delete: ${e?.message ?? e}`));

  // The view: Notes or Archive, grid or list, a search.
  let archive = false;
  let list = false;
  let query = "";
  let label = null; // showing one label's notes: its id

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
      b.style.background = c || "var(--cw-surface)";
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

  // THE ONE EDITOR (`md-editor`): a note is written as everything is — formatting, media inline, any item inserted,
  // files attached (kept where the note is: its space's, or yours; public when the note is).
  const [mdEditor, attachments] = await Promise.all(["md-editor", "attachments"].map(n => ctx.require(n)));
  const editorOf = (placeholder, files = []) => {
    const pick = attachments.picker({ space: sp, from: { app: "note" }, public: () => who.value() === "public", media: true });
    pick.preset?.(files);
    return mdEditor.create({ pick, placeholder, label: "Note" });
  };
  // THE COMPOSER: a line until clicked, then title and body; a note is made when it closes with something in it.
  const composer = root.querySelector(".composer");
  const [cTitle, cHost, cRow] = [".title", ".body", ".row"].map(q => composer.querySelector(q));
  let cEd = editorOf("Take a note…");
  cHost.replaceChildren(cEd.el);
  const cBar = () => cEd.el.querySelector(".bar");
  cBar() && (cBar().hidden = true);
  let composing = { color: "" };
  const open = () => {
    cTitle.hidden = false;
    cRow.hidden = false;
    cBar() && (cBar().hidden = false);
  };
  const shut = async () => {
    if (cEd.busy()) return said("Still sending the files: a moment…");
    const title = cTitle.value.trim();
    const body = cEd.value().trim();
    const files = cEd.files();
    const audience = who.value(); // read before the form resets
    composer.reset();
    cTitle.hidden = true;
    cRow.hidden = true;
    cEd = editorOf("Take a note…");
    cHost.replaceChildren(cEd.el);
    cBar() && (cBar().hidden = true);
    const color = composing.color;
    composing = { color: "" };
    tint(composer, "");
    if (!(title || body || files.length)) return;
    // Made while one label is shown: it has that label, as in Keep.
    const key = await save(null, { title, body, color, archived: false, audience, files });
    if (key && label) await labels.set(ref(key), label, true).catch(e => said(`Could not label: ${e?.message ?? e}`));
  };
  cHost.addEventListener("focusin", open);
  composer.querySelector(".close").onclick = shut;
  composer.querySelector(".palette").onclick = e => palette(e.currentTarget, c => ((composing.color = c), tint(composer, c)));
  addEventListener("click", e => {
    if (!composer.isConnected) return;
    if (!composer.contains(e.target) && !cTitle.hidden && !e.target.closest(".swatches")) shut();
  });

  // THE EDITOR: a note opened from its card; saved when it closes, if anything changed.
  const editor = root.querySelector("dialog.editor");
  const [eTitle, eHost] = [".title", ".body"].map(q => editor.querySelector(q));
  let editing = null;
  let eEd = null;
  const edit = n => {
    editing = { ...n };
    eTitle.value = n.title;
    eEd = editorOf("Note", n.item?.files ?? []);
    eEd.set(n.body);
    eHost.replaceChildren(eEd.el);
    tint(editor, n.color);
    editor.querySelector(".pin-slot").replaceChildren(pinUI.button(ref(n.key)));
    editor.querySelector(".label-slot").replaceChildren(labelUI.chips(ref(n.key), { onPick: show }));
    editor.querySelector(".archive-e").title = n.archived ? "Unarchive" : "Archive";
    editor.showModal();
  };
  const finish = async () => {
    if (!editing) return;
    if (eEd?.busy()) return said("Still sending the files: a moment…");
    const files = eEd?.files() ?? [];
    const n = { ...editing, title: eTitle.value.trim(), body: eEd?.value().trim() ?? editing.body, files };
    const was = held.find(x => x.ref === n.key);
    const before = was ? note(was) : { title: "", body: "", color: "", archived: false };
    editing = null;
    editor.close();
    const sameFiles = JSON.stringify(files) === JSON.stringify(was?.files ?? []);
    const changed = !sameFiles || ["title", "body", "color", "archived"].some(k => n[k] !== before[k]);
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
  const live = () => save(editing.key, { ...editing, title: eTitle.value.trim(), body: eEd?.value().trim() ?? editing.body });
  editor.querySelector(".palette").onclick = e =>
    palette(e.currentTarget, c => {
      editing.color = c;
      tint(editor, c);
      live();
    });
  editor.querySelector(".label-e").onclick = e => labelUI.menu(e.currentTarget, ref(editing.key));
  editor.querySelector(".archive-e").onclick = () => ((editing.archived = !editing.archived), finish());
  editor.querySelector(".delete-e").onclick = async () => {
    const key = editing.key;
    editing = null;
    editor.close();
    await remove(key);
  };

  // A CARD: the note's look (`cards`: the same wherever a note shows), with Note's tools — someone else's (Discover,
  // their space) its author named and read only (your pin and labels on it are yours).
  const cards = await ctx.require("cards");
  const actionsCap = await ctx.require("actions");
  const card = n => {
    const tools = [];
    const tool = (icon, title, run) => tools.push(Object.assign(document.createElement("button"), { type: "button", textContent: icon, title, onclick: e => (e.stopPropagation(), run(e)) }));
    // Someone else's: Save (`actions`: kept in your Saved, as anything is).
    if (!n.mayEdit) tools.push(actionsCap.save(n.item));
    if (n.mayEdit) tool("🎨", "Background", e => palette(e.currentTarget, color => save(n.key, { ...n, color })));
    tool("🏷️", "Labels", e => labelUI.menu(e.currentTarget, ref(n.key)));
    if (n.mayEdit) {
      tool(n.archived ? "📤" : "🗃️", n.archived ? "Unarchive" : "Archive", () => {
        save(n.key, { ...n, archived: !n.archived });
        if (!n.archived && n.pinned) pins.set(ref(n.key), false).catch(e => said(`Could not unpin: ${e?.message ?? e}`)); // an archived note is not pinned, as in Keep
      });
      tool("🗑️", "Delete", () => remove(n.key));
    }
    return cards.card(
      { ...n.item, meta: { ...(n.item.meta ?? {}), color: n.color } },
      { href: null, actions: tools, by: others, corner: pinUI.button(ref(n.key)), below: labelUI.chips(ref(n.key), { onPick: show }), open: n.mayEdit ? () => edit(n) : null },
    );
  };

  const R = await ctx.require("roles");
  const render = () => {
    root.classList.toggle("list", list);
    // The composer: not in the archive, nor for who may not edit here.
    composer.hidden = others || archive || (!!rs && R.mayWrite({ action: "post", item: { kind: "note", by: null, meta: {} }, writer: meId, r: rs }) !== true);
    if (label && !labels.list().some(l => l.id === label)) label = null; // deleted meanwhile
    bar();
    const q = query.toLowerCase();
    const inLabel = label ? new Set(labels.refs(label, "item:")) : null;
    const all = held
      .map(note)
      .filter(n => n.archived === archive)
      .filter(n => !inLabel || inLabel.has(ref(n.key)))
      .filter(n => !q || `${n.title}\n${n.body}\n${labels.of(ref(n.key)).map(l => l.name).join("\n")}`.toLowerCase().includes(q))
      .sort((a, b) => (b.edited || 0) - (a.edited || 0) || (a.key < b.key ? 1 : -1));
    const pinned = all.filter(n => n.pinned && !archive);
    const rest = all.filter(n => !n.pinned || archive);
    root.querySelector(".pinned-section").hidden = !pinned.length;
    root.querySelector(".others-label").hidden = !pinned.length || !rest.length;
    root.querySelector(".pinned").replaceChildren(...pinned.map(card));
    root.querySelector(".others").replaceChildren(...rest.map(card));
    const empty = root.querySelector(".empty");
    empty.hidden = all.length > 0;
    empty.textContent = q
      ? "No matching notes."
      : archive
        ? "Your archived notes appear here."
        : label
          ? "No notes with this label yet."
          : "Notes you add appear here.";
  };

  // THE LABEL BAR: all notes, or one label's; and Edit labels.
  function show(id) {
    label = id;
    archive = false;
    render();
    actions();
  }
  function bar() {
    const b = root.querySelector(".labelbar");
    const chip = (text, on, run, cls = "") => {
      const x = Object.assign(document.createElement("button"), { type: "button", textContent: text, className: cls });
      if (cls !== "edit-labels") x.setAttribute("aria-pressed", String(on));
      x.onclick = run;
      return x;
    };
    b.replaceChildren(
      chip("All notes", !label, () => show(null)),
      ...labels.list().map(l => chip(`🏷️ ${l.name}`, label === l.id, () => show(label === l.id ? null : l.id))),
      chip("✏️ Edit labels", false, editLabels, "edit-labels"),
    );
  }

  // EDIT LABELS: make, rename (on leaving the field) and delete, as in Keep.
  const lEditor = root.querySelector("dialog.labels-editor");
  function lines() {
    lEditor.querySelector(".label-lines").replaceChildren(
      ...labels.list().map(l => {
        const line = Object.assign(document.createElement("div"), { className: "line" });
        const del = Object.assign(document.createElement("button"), { type: "button", textContent: "🗑️", title: "Delete label" });
        del.onclick = () => {
          if (confirm(`Delete the label “${l.name}”? It is removed from every note; the notes stay.`)) labels.remove(l.id).catch(e => said(`Could not delete the label: ${e?.message ?? e}`));
        };
        const name = Object.assign(document.createElement("input"), { value: l.name, maxLength: 50 });
        name.onchange = () => labels.rename(l.id, name.value).catch(e => ((name.value = l.name), said(`Could not rename: ${e?.message ?? e}`)));
        line.append(del, name);
        return line;
      }),
    );
  }
  function editLabels() {
    lines();
    lEditor.showModal();
  }
  lEditor.querySelector(".new-label").onsubmit = e => {
    e.preventDefault();
    const f = e.currentTarget;
    const v = f.name.value;
    f.reset();
    if (v.trim()) labels.create(v).catch(err => said(`Could not create the label: ${err?.message ?? err}`));
  };
  lEditor.querySelector(".done-l").onclick = () => lEditor.close();
  labels.onChange(() => lEditor.open && lines());

  // THE TOP BAR while Notes is open: search, grid or list, and Notes or Archive.
  // Who may edit here: the composer says so when this person may not.
  const gate = () => {
    const ok = !others && (!rs || rs.allows("edit", meId, "note"));
    composer.hidden = !ok;
  };
  gate();
  rs?.onChange(() => root.isConnected && (gate(), render()));
  // THE TABS (`where`'s, in every app's order): Your notes · the app's own · Discover.
  const actions = () => {
    at.tabs([
      { search: v => ((query = v), render()), placeholder: sp ? `Search ${sp.name}'s notes` : others ? "Search these notes" : "Search your notes", value: query },
      { label: list ? "Grid view" : "List view", run: () => ((list = !list), render(), actions()) },
      ...(others ? [] : [{ label: "Archive", on: archive, run: () => ((archive = !archive), render(), actions()) }]),
    ], { yoursOn: !archive });
  };
  actions();
  items.onChange(() => root.isConnected && reload());
  reload();
  pins.onChange(() => root.isConnected && render());
  labels.onChange(() => root.isConnected && render());
  render();
  // Another space's notes (or yours): opened afresh.
  const wherever = (await ctx.require("where")).key();
  const moved = async () => {
    if (!root.isConnected || ctx.route !== "/note") return removeEventListener("craftworks:route", moved);
    if ((await ctx.require("where")).key() === wherever) return;
    removeEventListener("craftworks:route", moved);
    el.replaceChildren();
    mount(ctx, el);
  };
  addEventListener("craftworks:route", moved);
}
