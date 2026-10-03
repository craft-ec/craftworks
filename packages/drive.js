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
    <style>
      .dv { max-width: 1000px; margin: 0 auto; display: grid; gap: var(--cw-space-3); }
      .dv .top { display: flex; align-items: center; gap: var(--cw-space-2); flex-wrap: wrap; }
      .dv .top h2 { margin: 0; font-size: 1.3rem; flex: 1; min-width: 0; }
      .dv .crumbs { display: flex; flex-wrap: wrap; gap: 4px; align-items: center; color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .dv .crumbs a { color: var(--cw-fg); text-decoration: none; }
      .dv .crumbs a:hover { text-decoration: underline; }
      .dv select { font: inherit; padding: 5px 8px; border-radius: var(--cw-radius-sm); max-width: 260px; }
      .dv button, .dv label.up { font: inherit; cursor: pointer; border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg);
        border-radius: var(--cw-radius-sm); padding: 6px var(--cw-space-3); }
      .dv label.up { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
      .dv .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: var(--cw-space-3); }
      .dv .tile { border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); overflow: hidden; display: grid;
        grid-template-rows: 120px auto; cursor: pointer; position: relative; }
      .dv .tile:hover { border-color: var(--cw-muted); }
      .dv .tile .pic { display: grid; place-items: center; background: var(--cw-hover); font-size: 2.4rem; overflow: hidden; }
      .dv .tile .pic img { width: 100%; height: 100%; object-fit: cover; }
      .dv .tile .cap { padding: 6px 8px; display: grid; gap: 2px; min-width: 0; }
      .dv .tile .n { font-size: var(--cw-text-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .dv .tile .s { font-size: var(--cw-text-xs); color: var(--cw-muted); }
      .dv .tile .more { position: absolute; top: 4px; right: 4px; padding: 0 6px; border-radius: 50%; opacity: 0; background: var(--cw-surface); }
      .dv .tile:hover .more, .dv .tile .more:focus-visible { opacity: 1; }
      .dv .menu { position: absolute; top: 28px; right: 4px; z-index: 5; display: grid; background: var(--cw-surface); border: 1px solid var(--cw-line);
        border-radius: var(--cw-radius-sm); box-shadow: var(--cw-shadow-lg); padding: 4px; }
      .dv .menu button { border: 0; text-align: left; padding: 6px 10px; }
      .dv .menu button:hover { background: var(--cw-hover); }
      .dv .folder .pic { font-size: 2.8rem; }
      .dv .ups { display: grid; gap: 4px; font-size: var(--cw-text-sm); color: var(--cw-muted); }
      .dv .none { color: var(--cw-muted); text-align: center; padding: var(--cw-space-5); }
      .dv .said { color: var(--cw-danger); font-size: var(--cw-text-sm); margin: 0; }
      .dv-ask { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-4); width: min(360px, calc(100vw - 32px)); box-shadow: var(--cw-shadow-lg);
        background: var(--cw-surface); color: var(--cw-fg); }
      .dv-ask h3 { margin: 0 0 var(--cw-space-2); font-size: 1rem; }
      .dv-ask input { width: 100%; box-sizing: border-box; font: inherit; padding: 6px 8px; border-radius: var(--cw-radius-sm); margin-bottom: var(--cw-space-3); }
      .dv-ask .row { display: flex; justify-content: flex-end; gap: var(--cw-space-2); }
      .dv-ask button { font: inherit; cursor: pointer; border: 1px solid var(--cw-line); background: none; color: var(--cw-fg); border-radius: var(--cw-radius-sm); padding: 4px 12px; }
      .dv-ask button.go { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
    </style>
    <div class="dv"></div>`;
  const root = el.querySelector(".dv");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  // Where Drive is: yours, or the space open; and the folder (`f/<path>`).
  const sp = ctx.space && ctx.space !== "discover" ? (await space.mine()).find(s => s.id === ctx.space) ?? null : null;
  const base = () => (sp ? `#/s/${sp.id}/drive` : "#/drive");
  const folder = () => `/${decodeURIComponent((ctx.sub || "").replace(/^f\/?/, ""))}`.replace(/\/+$/, "") || "/";
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
  // WHO SEES what is uploaded here: the one picker (`audience`) — yours start Only you, a space's its members.
  const who = await (await ctx.require("audience")).picker({ space: sp, kind: "file", initial: sp ? "members" : "private" });

  const drivesKnown = drive.drives().catch(() => []);
  async function draw() {
    const at = folder();
    const [rows, folders] = await Promise.all([drive.list(sp), drive.folders(sp)]);
    const here = rows.filter(r => r.folder === at);
    const subs = folders.filter(f => f !== at && f.startsWith(at === "/" ? "/" : `${at}/`) && !f.slice(at === "/" ? 1 : at.length + 1).includes("/"));
    // The path, each step a link.
    const crumbs = h("nav", { className: "crumbs", ariaLabel: "Folder" }, h("a", { href: hrefOf("/"), textContent: sp ? `${space.shown(sp)} Drive` : "Your Drive" }));
    at.split("/").filter(Boolean).reduce((path, part) => {
      const p = `${path}/${part}`;
      crumbs.append(" / ", h("a", { href: hrefOf(p), textContent: part }));
      return p;
    }, "");
    const input = h("input", { type: "file", multiple: true, hidden: true, onchange: () => (upload([...input.files]), (input.value = "")) });
    // WHICH DRIVE: yours, or any space's you are in — the spaces filled in when known (each space's apps read), never
    // waited on before the Drive shows.
    const which = h("select", { ariaLabel: "Drive", onchange: () => (location.hash = which.value ? `#/s/${which.value}/drive` : "#/drive") }, h("option", { value: "", textContent: "Your Drive" }), ...(sp ? [h("option", { value: sp.id, textContent: `${space.shown(sp)} Drive` })] : []));
    drivesKnown.then(all => {
      for (const s of all) if (s.id !== sp?.id) which.append(h("option", { value: s.id, textContent: `${space.shown(s)} Drive` }));
      which.value = sp?.id ?? "";
    });
    which.value = sp?.id ?? "";
    const top = h(
      "div",
      { className: "top" },
      h("h2", { textContent: "🗂️ Drive" }),
      which,
      h("label", { className: "up" }, "📤 Upload", input),
      who.el,
      h("button", {
        type: "button",
        textContent: "New folder",
        onclick: async () => {
          const name = await ask("New folder");
          if (name) await drive.mkdir(sp, `${at === "/" ? "" : at}/${name}`).then(draw, fail);
        },
      }),
    );
    const tiles = [
      ...subs.map(f => h("a", { className: "tile folder", href: hrefOf(f), style: "text-decoration:none;color:inherit" }, h("div", { className: "pic", textContent: "📁" }), h("div", { className: "cap" }, h("span", { className: "n", textContent: f.slice(f.lastIndexOf("/") + 1) }), h("span", { className: "s", textContent: "Folder" })))),
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

  function tile(r) {
    const src = r.ref.preview ?? (attachments.isImage(r.ref) && r.ref.inline ? `data:${r.ref.type};base64,${r.ref.inline}` : null);
    const note = h("span", { className: "s" });
    const menu = h("div", { className: "menu", hidden: true });
    const t = h(
      "div",
      { className: "tile", title: r.ref.name, onclick: e => !e.target.closest(".more, .menu") && attachments.open(r.ref, note) },
      h("div", { className: "pic" }, src ? h("img", { src, alt: "" }) : (kinds.mediaOf(r.ref)?.icon ?? "📄")),
      h("div", { className: "cap" }, h("span", { className: "n", textContent: r.ref.name }), h("span", { className: "s", textContent: `${attachments.sizeOf(r.ref.size)} · ${new Date(r.at).toLocaleDateString()}` }), note),
      h("button", { type: "button", className: "more", title: "More", textContent: "⋯", onclick: () => (menu.hidden = !menu.hidden) }),
      menu,
    );
    // A file of an item made in another app: shown here, changed there (its page).
    if (r.readOnly) {
      menu.append(
        h("button", { type: "button", textContent: "Open", onclick: () => ((menu.hidden = true), attachments.open(r.ref, note)) }),
        h("a", { href: r.page, textContent: `Open in ${r.from?.app === "text" ? "Board" : kinds.domainName(r.from?.app)}` }),
      );
      return t;
    }
    menu.append(
      h("button", { type: "button", textContent: "Open", onclick: () => ((menu.hidden = true), attachments.open(r.ref, note)) }),
      h("button", {
        type: "button",
        textContent: "Move…",
        onclick: async () => {
          menu.hidden = true;
          const to = await ask("Move to folder (e.g. /Photos)", r.folder);
          if (to != null) await drive.move(sp, r.id, to).then(draw, fail);
        },
      }),
      h("button", { type: "button", textContent: "Remove from Drive", onclick: () => ((menu.hidden = true), drive.remove(sp, r.id).then(draw, fail)) }),
    );
    return t;
  }

  // UPLOAD into the folder open, seen by whom the picker says (public: put in the clear; else sealed).
  async function upload(list) {
    for (const file of list) {
      const line = h("div", { textContent: `${file.name}: starting…` });
      ups.append(line);
      drive
        .upload(file, { space: sp, public: who.isPublic(), folder: folder(), from: { app: "drive" }, onProgress: e => (line.textContent = `${file.name}: ${e.phase === "reading" ? "reading" : `${Math.round((100 * e.done) / Math.max(1, e.size))}%`}`) })
        .then(
          () => (line.remove(), draw()),
          e => (line.textContent = `${file.name}: not uploaded — ${e.message ?? e}`),
        );
    }
  }

  root.replaceChildren(theme.loading("Opening Drive…"));
  await draw();
  drive.onChange(sp, () => el.isConnected && draw());
  let drawnFor = `${ctx.space ?? ""}|${ctx.sub ?? ""}`;
  addEventListener("craftworks:route", () => {
    const k = `${ctx.space ?? ""}|${ctx.sub ?? ""}`;
    if (el.isConnected && ctx.route === "/drive" && k !== drawnFor) (drawnFor = k), draw();
  });
}
