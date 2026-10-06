// DRIVE, an app (`#/drive`: your files; `#/s/<space>/drive`: a space's): every file uploaded — attached to a message, a
// post, a mail, or uploaded here — listed, in FOLDERS (`#/drive/f/<path>`). Upload (📤, into the folder open), New
// folder, open (an image or a video full, anything else downloaded), Move, Remove from Drive. Images show by their
// thumbnail. The files are `files`' (sealed, coded); the catalogue is `drive-store`'s. A PRIVATE page.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [drive, space, attachments, theme, kinds] = await Promise.all(["drive-store", "space", "attachments", "theme", "kinds"].map(n => ctx.require(n)));
  el.innerHTML = `
    <style>@layer apps {
      .dv { display: grid; gap: var(--cw-space-3); }
      .dv .top { display: flex; align-items: center; gap: var(--cw-space-2); flex-wrap: wrap; }
      .dv .top h2 { margin: 0; flex: 1; min-width: 0; }
      .dv .crumbs { display: flex; flex-wrap: wrap; gap: 4px; align-items: center; color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .dv .crumbs a { color: var(--cw-fg); text-decoration: none; }
      .dv .crumbs a:hover { text-decoration: underline; }
      .dv-who { border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-bg); color: var(--cw-fg); min-width: min(420px, 92vw); }
      .dv-who form { display: grid; gap: var(--cw-space-3); }
      .dv-who h3 { margin: 0; overflow-wrap: anywhere; }
      .dv-who .row { display: flex; gap: var(--cw-space-2); justify-content: flex-end; }
      .dv select { font: inherit; padding: 5px 8px; border-radius: var(--cw-radius-sm); max-width: 260px; }
      .dv label.up { font: inherit; cursor: pointer; border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg);
        border-radius: var(--cw-radius-sm); padding: 6px var(--cw-space-3); }
      .dv label.up { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
      .dv .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: var(--cw-space-3); }
      .dv .folder .pic { font-size: 2.8rem; }
      .dv .folder { text-decoration: none; }
      .dv .ups { display: grid; gap: 4px; font-size: var(--cw-text-sm); color: var(--cw-muted); }
      .dv .none { color: var(--cw-muted); text-align: center; padding: var(--cw-space-5); }
      .dv .said { color: var(--cw-danger); font-size: var(--cw-text-sm); margin: 0; }
      .dv-ask { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-4); width: min(360px, calc(100vw - 32px)); box-shadow: var(--cw-shadow-lg);
        background: var(--cw-surface); color: var(--cw-fg); }
      .dv-ask h3 { margin: 0 0 var(--cw-space-2); }
      .dv-ask input { width: 100%; box-sizing: border-box; font: inherit; padding: 6px 8px; border-radius: var(--cw-radius-sm); margin-bottom: var(--cw-space-3); }
      .dv-ask .row { display: flex; justify-content: flex-end; gap: var(--cw-space-2); }
    }</style>
    <div class="dv"></div>`;
  const root = el.querySelector(".dv");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  // Where Drive is: yours, or the space open; and the folder (`f/<path>`).
  // WHERE (`where`: yours, a space's, a person's, Discover — the last two read only).
  const where = await ctx.require("where");
  const place = await where.of({ kind: "file", app: "drive", yours: "Your Drive" });
  const sp = place.space;
  const them = place.person;
  const others = place.others;
  const discover = place.discover;
  const opts = { discover, person: them };
  const base = () => place.base;
  const folder = () => (place.saved ? "/" : `/${decodeURIComponent(place.sub.replace(/^f\/?/, ""))}`.replace(/\/+$/, "") || "/");
  const hrefOf = f => (f === "/" ? base() : `${base()}/f/${f.slice(1).split("/").map(encodeURIComponent).join("/")}`);
  const said = h("p", { className: "said", hidden: true });
  // ASK for a name or a path, in the app (not the browser's prompt): the text, or null.
  const ask = (title, value = "") =>
    new Promise(resolve => {
      const input = h("input", { value, autocomplete: "off", required: true, ariaLabel: title });
      const f = h("form", { method: "dialog" }, h("h3", { textContent: title }), input, h("div", { className: "row" }, h("button", { type: "button", textContent: "Cancel", onclick: () => d.close() }), h("button", { className: "go", textContent: "OK" })));
      const d = h("dialog", { className: "dv-ask" }, f);
      f.onsubmit = e => (e.preventDefault(), (d.returnValue = "ok"), d.close());
      d.addEventListener("close", () => (resolve(d.returnValue === "ok" ? input.value.trim() || null : null), d.remove()));
      document.body.append(d);
      d.showModal();
      input.select();
    });
  const fail = e => ((said.textContent = e?.message ?? String(e)), (said.hidden = false));
  const ups = h("div", { className: "ups" });
  // WHO SEES IT: asked per upload — what is chosen is each file's own (an item's audience), never the page's. The one
  // picker (`audience`): yours start Only you, a space's its members.
  const audience = await ctx.require("audience");
  function askWho(list) {
    return new Promise(async resolve => {
      const who = await audience.picker({ space: sp, kind: "file", initial: sp ? "members" : "private" });
      const d = h("dialog", { className: "dv-who" });
      const go = h("button", { type: "submit", className: "go", textContent: "Upload" });
      const no = h("button", { type: "button", textContent: "Cancel", onclick: () => d.close() });
      const f = h("form", { method: "dialog" }, h("h3", { textContent: list.length === 1 ? list[0].name : `${list.length} files` }), who.el, h("div", { className: "row" }, no, go));
      let chosen = null;
      f.onsubmit = e => ((e.preventDefault()), (chosen = { public: who.isPublic(), audience: who.value(), write: who.write() }), d.close());
      d.append(f);
      d.onclose = () => (d.remove(), resolve(chosen));
      document.body.append(d);
      d.showModal();
    });
  }

  // THE TABS (`where`'s, in every app's order): Your Drive · Saved · Discover · ＋ Upload (into the folder open; from
  // Saved or Discover: your Drive first).
  const input = h("input", { type: "file", multiple: true, hidden: true, onchange: () => (upload([...input.files]), (input.value = "")) });
  el.append(input);
  place.tabs([], { create: { label: "Upload", run: () => (place.others ? (location.hash = "#/drive") : input.click()) } });
  const directory = await ctx.require("directory");
  async function draw() {
    // A FILE's PAGE (`…/drive/p/<ref>`): the one item page — its preview, its comments — framed here.
    if (place.sub.startsWith("p/")) {
      const id = decodeURIComponent(place.sub.slice(2));
      const page = await (await ctx.require("item-page")).show(id, { app: "drive", back: base(), discover });
      // Drive's own, beside the page's action row: Open (show or download it) and, for one listed here, Move…
      const row = (await drive.list(sp, opts).catch(() => [])).find(r => r.id === id);
      const note = h("span", { className: "s" });
      const tools = row
        ? [
            h("button", { type: "button", className: "cw-btn", textContent: "Open", onclick: () => attachments.open(row.ref, note) }),
            row.readOnly
              ? null
              : h("button", {
                  type: "button",
                  className: "cw-btn",
                  textContent: "Move…",
                  onclick: async () => {
                    const to = await ask("Move to folder (e.g. /Photos)", row.folder);
                    if (to != null) await drive.move(sp, row.id, to).then(() => (location.hash = base()), fail);
                  },
                }),
            note,
          ]
        : [];
      return root.replaceChildren(h("nav", { className: "crumbs" }, h("a", { href: base(), textContent: "← Drive" }), " ", ...tools.filter(Boolean)), page);
    }
    const at = folder();
    // SAVED: `where`'s (the files you saved, of anyone's) — one folder.
    const savedRows = () => place.read().then(list => list.filter(it => it.files?.[0]).map(it => ({ id: it.ref, ref: it.files[0], at: it.at, folder: "/", by: it.by, readOnly: true, others: true })));
    const [rows, folders] = place.saved ? [await savedRows(), ["/"]] : await Promise.all([drive.list(sp, opts), drive.folders(sp, opts)]);
    const here = rows.filter(r => r.folder === at);
    const subs = folders.filter(f => f !== at && f.startsWith(at === "/" ? "/" : `${at}/`) && !f.slice(at === "/" ? 1 : at.length + 1).includes("/"));
    // The path, each step a link.
    const crumbs = h("nav", { className: "crumbs", ariaLabel: "Folder" }, h("a", { href: hrefOf("/"), textContent: sp ? `${space.shown(sp)} Drive` : discover ? "Public files" : place.saved ? "Saved" : them ? `${directory.shown(them)}'s Drive` : "Your Drive" }));
    at.split("/").filter(Boolean).reduce((path, part) => {
      const p = `${path}/${part}`;
      crumbs.append(" / ", h("a", { href: hrefOf(p), textContent: part }));
      return p;
    }, "");
    const top = h(
      "div",
      { className: "top" },
      h("h2", { textContent: "🗂️ Drive" }),
      others ? null : h("button", {
        type: "button",
        textContent: "New folder",
        onclick: async () => {
          const name = await ask("New folder");
          if (name) await drive.mkdir(sp, `${at === "/" ? "" : at}/${name}`).then(draw, fail);
        },
      }),
    );
    const tiles = [
      ...subs.map(f => h("a", { className: "cw-card cw-file folder", href: hrefOf(f) }, h("div", { className: "pic", textContent: "📁" }), h("div", { className: "cap" }, h("span", { className: "n", textContent: f.slice(f.lastIndexOf("/") + 1) }), h("span", { className: "s", textContent: "Folder" })))),
      ...here.map(tile),
    ];
    root.replaceChildren(
      top,
      crumbs,
      ups,
      said,
      tiles.length ? h("div", { className: "grid" }, ...tiles) : h("p", { className: "none", textContent: at === "/" ? "Nothing here yet: files you upload, or attach to a message, a post or a mail, are kept here." : "This folder is empty." }),
    );
  }

  // A FILE's tile: the file look (`cards`: the same wherever a file shows), linked to its page — the action row and
  // Drive's Open and Move… are there, not on the tile (owner 10-06: feeds show items; their page acts on one). A file
  // of an item made in another app opens that item's page.
  const cards = await ctx.require("cards");
  function tile(r) {
    const note = h("span", { className: "s" });
    const href = r.page ?? (String(r.id).includes("#") ? null : `${base()}/p/${encodeURIComponent(r.id)}`);
    return cards.card({ kind: "file", files: [r.ref], title: r.ref.name, at: r.at, by: r.by }, { href, actions: [], by: !!r.others, open: href ? () => (location.hash = href) : () => attachments.open(r.ref, note), below: note });
  }

  // UPLOAD into the folder open, seen by whom the picker says (public: put in the clear; else sealed).
  async function upload(list) {
    if (!list.length) return;
    const who = await askWho(list);
    if (!who) return;
    for (const file of list) {
      const line = h("div", { textContent: `${file.name}: starting…` });
      ups.append(line);
      drive
        .upload(file, { space: sp, public: who.public, audience: who.audience, write: who.write, folder: folder(), from: { app: "drive" }, onProgress: e => (line.textContent = `${file.name}: ${e.phase === "reading" ? "reading" : `${Math.round((100 * e.done) / Math.max(1, e.size))}%`}`) })
        .then(
          () => (line.remove(), draw()),
          e => (line.textContent = `${file.name}: not uploaded — ${e.message ?? e}`),
        );
    }
  }

  root.replaceChildren(theme.loading("Opening Drive…"));
  await draw();
  drive.onChange(sp, () => el.isConnected && draw());
  // Another folder: drawn again. Another DRIVE (a space's, a person's, Discover): opened afresh — what it is, read once
  // above, is its own.
  let drawnFor = where.key();
  const whose = where.placeKey;
  const opened = whose();
  const moved = () => {
    if (!el.isConnected || ctx.route !== "/drive") return removeEventListener("craftworks:route", moved);
    const k = where.key();
    if (k === drawnFor) return;
    drawnFor = k;
    if (whose() === opened) return draw();
    removeEventListener("craftworks:route", moved);
    el.replaceChildren();
    mount(ctx, el);
  };
  addEventListener("craftworks:route", moved);
}
