// ATTACHMENTS, a component: FILES on an item (a message, a post, a mail) — picked with 📎: a file from this device
// (sent at once, with its progress, and listed in Drive: every upload is), or one ALREADY in Drive (yours, or the
// space's: its reference given, nothing sent again); the item waits for them. And SHOWN: an image as its thumbnail (made by `image-studio` when picked, kept in the
// reference: a list never downloads the image), opened full on a click; any other file as its name and size, with
// Download. The bytes are `files`' (sealed, coded, raced); the reference rides in the item, so who reads the item
// reads its files — and nobody else.
//
//   const att = await ctx.require("attachments");
//   const pick = att.picker({ space, public })   // { el, files(): [ref], busy(): bool, clear(), onChange(fn),
//                                                //   addFiles([File]), preset([ref]), onReady(fn(ref, File|null)) }
//                                                // public: a boolean, or a function asked when each file is picked
//   host.append(att.show(item.files))            // nothing for none
export async function start(ctx) {
  const [files, drive, spaces, kinds] = await Promise.all(["files", "drive-store", "space", "kinds"].map(n => ctx.require(n)));
  const style = document.createElement("style");
  style.textContent = `@layer components {

    .cw-att-pick { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
    .cw-att-pick > button.clip { border: 0; background: none; font-size: 1.1rem; cursor: pointer; padding: 2px 4px; color: var(--cw-muted); }
    .cw-att-pick > button.clip:hover { color: var(--cw-fg); }
    .cw-att-chip { display: inline-flex; align-items: center; gap: 6px; border: 1px solid var(--cw-line); border-radius: var(--cw-radius-pill);
      padding: 2px 4px 2px 8px; font-size: var(--cw-text-xs); max-width: 260px; background: var(--cw-surface); }
    .cw-att-chip .n { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cw-att-chip .p { color: var(--cw-muted); }
    .cw-att-chip.err { border-color: var(--cw-danger); color: var(--cw-danger); }
    .cw-att-chip button { border: 0; background: none; cursor: pointer; color: var(--cw-muted); padding: 0 2px; }
    .cw-att { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 180px)); gap: var(--cw-space-2); margin-top: 6px; }
    .cw-att-pick { position: relative; }
    .cw-att-menu { position: absolute; bottom: 100%; left: 0; z-index: 5; display: grid; background: var(--cw-surface); border: 1px solid var(--cw-line);
      border-radius: var(--cw-radius-sm); box-shadow: var(--cw-shadow-lg); padding: 4px; min-width: 160px; }
    .cw-att-menu[hidden] { display: none; }
    .cw-att-menu button, .cw-att-menu .cw-att-file { font: inherit; text-align: left; border: 0; background: none; color: var(--cw-fg); padding: 6px 10px; cursor: pointer; border-radius: var(--cw-radius-sm); white-space: nowrap; }
    .cw-att-menu .cw-att-file:hover { background: var(--cw-hover); }
    .cw-att-file { position: relative; display: block; }
    .cw-att-file input { position: absolute; width: 1px; height: 1px; opacity: 0; overflow: hidden; pointer-events: none; }
    .cw-att-menu button:hover { background: var(--cw-hover); }
    .cw-att-drive { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-4); width: min(520px, calc(100vw - 32px)); box-shadow: var(--cw-shadow-lg);
      background: var(--cw-surface); color: var(--cw-fg); }
    .cw-att-drive h3 { margin: 0 0 var(--cw-space-2); font-size: 1rem; }
    .cw-att-drive .who { margin: 0 0 var(--cw-space-2); color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-att-drive select { font: inherit; padding: 4px 8px; border-radius: var(--cw-radius-sm); margin-bottom: var(--cw-space-2); max-width: 100%; }
    .cw-att-drive ul { list-style: none; margin: 0 0 var(--cw-space-3); padding: 0; max-height: 60vh; overflow: auto; display: grid; gap: 4px; }
    .cw-att-drive li { display: flex; align-items: center; gap: var(--cw-space-2); padding: 6px; border-radius: var(--cw-radius-sm); cursor: pointer; }
    .cw-att-drive li:hover { background: var(--cw-hover); }
    .cw-att-drive li img { width: 40px; height: 40px; object-fit: cover; border-radius: 4px; }
    .cw-att-drive li .ic { width: 40px; text-align: center; font-size: 1.4rem; }
    .cw-att-drive li .n { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cw-att-drive li .s { color: var(--cw-muted); font-size: var(--cw-text-xs); }
    .cw-att-drive > button { font: inherit; border: 1px solid var(--cw-line); background: none; color: var(--cw-fg); border-radius: var(--cw-radius-sm); padding: 4px 12px; cursor: pointer; }
    .cw-att-full { border: 0; padding: 0; background: transparent; max-width: 96vw; max-height: 96vh; }
    .cw-att-full::backdrop { background: rgba(0, 0, 0, .8); }
    .cw-att-full img, .cw-att-full video { max-width: 96vw; max-height: 92vh; display: block; }
    .cw-att-full p { color: #fff; margin: 8px 0 0; font-size: var(--cw-text-sm); }
}`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const sizeOf = n => (n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(0)} KB` : n < 1073741824 ? `${(n / 1048576).toFixed(1)} MB` : `${(n / 1073741824).toFixed(2)} GB`);
  const isImage = r => kinds.mediaOf(r)?.domain === "image";

  // `media`: a video or an audio goes through the media pipeline (`video-studio`: streamed, its poster or album cover,
  // its length, its video id — what its subtitles, lyrics and transcripts are found by), as in Videos and Audio.
  // `publish`: a NEW image, video or audio uploaded here becomes an ITEM of its kind by the one path (`publisher`, the
  // same as its app's upload page) — for whom the item being written is; one taken from Drive is an item already.
  // (A conversation's files — a message, a mail — stay its own: no `publish`.)
  // A LINE THAT PICKS FILES: a <label> around its own file input — the browser opens its chooser on the click itself
  // (no script calling `input.click()` on a hidden input, which Safari ignores, in a sandboxed frame above all).
  // `onFiles(files)` when some are chosen; `after()` once clicked (a menu closing).
  function fileButton(text, { accept = "", onFiles, after = () => {} } = {}) {
    const input = h("input", { type: "file", multiple: true, accept, tabIndex: -1 });
    input.onchange = () => {
      const fs = [...input.files];
      input.value = "";
      if (fs.length) onFiles(fs);
    };
    return h("label", { className: "cw-att-file", onclick: () => setTimeout(after) }, text, input);
  }

  // A MENU opened beside its button: on the side with more room (above or below), never past the screen — as tall as
  // that side allows, scrolling within. Every attach and insert menu opens through it.
  // A MENU placed in the WINDOW by its anchor (fixed: never clipped by a card or a list that hides what spills out),
  // above or below it — wherever there is more room under the top bar — and within the window's width.
  function fitMenu(menu, anchor = menu.parentElement) {
    const r = anchor.getBoundingClientRect();
    const top = Math.max(0, document.querySelector(".bar")?.getBoundingClientRect().bottom ?? 0);
    const above = r.top - top - 8;
    const below = innerHeight - r.bottom - 8;
    const up = above > below;
    Object.assign(menu.style, { position: "fixed", top: up ? "auto" : `${r.bottom + 4}px`, bottom: up ? `${innerHeight - r.top + 4}px` : "auto", maxHeight: `${Math.max(120, up ? above : below)}px`, overflowY: "auto", boxSizing: "border-box", zIndex: "80" });
    // Within the window's width: its left by the anchor, moved in when it would spill out.
    menu.style.left = "0px";
    menu.style.right = "auto";
    const w = menu.offsetWidth;
    menu.style.left = `${Math.max(8, Math.min(r.left, innerWidth - w - 8))}px`;
  }
  // A placed menu follows nothing: closed when the page scrolls under it.
  addEventListener("scroll", e => document.querySelectorAll(".cw-att-menu, .cw-reacts .pick").forEach(m => m.style.position === "fixed" && !m.hidden && !m.contains(e.target) && (m.hidden = true)), true);
  function picker({ space = null, public: pub = false, from = null, media = false, publish = false } = {}) {
    const chips = h("span", { className: "cw-att-pick" });
    // 📎: this device, or Drive.
    const menu = h("span", { className: "cw-att-menu", hidden: true },
      fileButton("From this device", { onFiles: fs => fs.forEach(f => add(f)), after: () => (menu.hidden = true) }),
      h("button", { type: "button", textContent: "From Drive", onclick: () => ((menu.hidden = true), fromDrive()) }),
    );
    const el = h("span", { className: "cw-att-pick" }, h("button", { type: "button", className: "clip", title: "Attach files", ariaLabel: "Attach files", textContent: "📎", onclick: () => ((menu.hidden = !menu.hidden), menu.hidden || fitMenu(menu)) }), menu, chips);
    // The menu closes on a click anywhere outside it (and its 📎).
    document.addEventListener("pointerdown", e => {
      if (el.isConnected && !menu.hidden && !el.contains(e.target)) menu.hidden = true;
    });
    const items = []; // { file, ref, busy, error, chip }
    const changed = [];
    const tell = () => changed.forEach(f => f());
    const readied = []; // told each file once it is ready: (ref, the File it came from, or null from Drive)
    const told = (ref, file) => readied.forEach(f => f(ref, file));
    // FROM DRIVE: yours, or ANY space's you are in (chosen at the top), each file attached as its reference (ready at
    // once): the item's readers read it — as a file forwarded.
    // `media`: only images, videos and audio (the editor's 🖼). WHO READS IT is said: a file taken from Drive is
    // adopted into this item's space, readable by exactly the item's readers.
    async function fromDrive({ media = false } = {}) {
      const all = await drive.drives();
      const choose = h("select", { ariaLabel: "Drive" }, h("option", { value: "", textContent: "Your Drive" }), ...all.map(s => h("option", { value: s.id, textContent: `${spaces.shown(s)} Drive` })));
      if (space && all.some(s => s.id === space.id)) choose.value = space.id;
      const d = h("dialog", { className: "cw-att-drive" });
      const listEl = h("ul", {});
      const drawList = async () => {
        const sp = all.find(s => s.id === choose.value) ?? null;
        listEl.replaceChildren(h("li", { textContent: "Loading…" }));
        const rows = (await drive.list(sp).catch(() => [])).filter(r => !media || !!kinds.mediaOf(r.ref));
        listEl.replaceChildren(
          ...(rows.length
            ? rows.map(r => {
                const src = r.ref.preview ?? (isImage(r.ref) && r.ref.inline ? `data:${r.ref.type};base64,${r.ref.inline}` : null);
                return h("li", { onclick: () => (take(r.ref), d.close()) }, src ? h("img", { src, alt: "" }) : h("span", { className: "ic", textContent: /^video\//.test(r.ref.type) ? "🎬" : /^image\//.test(r.ref.type) ? "🖼️" : "📄" }), h("span", { className: "n", textContent: r.ref.name }), h("span", { className: "s", textContent: `${sizeOf(r.ref.size)} · ${r.folder}` }));
              })
            : [h("li", { textContent: "Nothing in this Drive yet." })]),
        );
      };
      choose.onchange = drawList;
      const pubNow = typeof pub === "function" ? !!pub() : pub;
      const who = h("p", { className: "who", textContent: pubNow ? "Attached here, it is PUBLIC: anyone who reads this can open it." : space ? `Attached here, it is read by ${spaces.shown(space)}'s members — whoever reads this.` : "Attached here, it is read by whoever reads this." });
      d.append(h("h3", { textContent: media ? "Media from Drive" : "Attach from Drive" }), choose, who, listEl, h("button", { type: "button", textContent: "Close", onclick: () => d.close() }));
      drawList();
      d.addEventListener("click", e => e.target === d && d.close());
      d.addEventListener("close", () => d.remove());
      document.body.append(d);
      d.showModal();
    }
    // A file from ANOTHER space's Drive is adopted into this item's space (listed there, then copied under its key):
    // who reads it is who reads this space.
    function take(ref) {
      const pubNow = typeof pub === "function" ? !!pub() : pub;
      files.adopt(ref, space, { app: from?.app ?? null, pub: pubNow }).then(ready, e => ctx.log("attachments", { what: `${ref.name}: ${e.message ?? e}` }));
    }
    function ready(ref, { quiet = false } = {}) {
      if (items.some(i => i.ref && (i.ref.root ?? i.ref.inline) === (ref.root ?? ref.inline))) return;
      const it = { file: null, ref, busy: false, error: null };
      it.chip = h("span", { className: "cw-att-chip" }, h("span", { className: "n", textContent: ref.name, title: ref.name }), h("span", { className: "p", textContent: sizeOf(ref.size) }), h("button", { type: "button", title: "Remove", textContent: "✕", onclick: () => (items.splice(items.indexOf(it), 1), it.chip.remove(), tell()) }));
      chips.append(it.chip);
      items.push(it);
      tell();
      if (!quiet) told(ref, null);
    }
    function add(file) {
      const it = { file, ref: null, busy: true, error: null };
      const pct = h("span", { className: "p", textContent: "0%" });
      it.chip = h("span", { className: "cw-att-chip" }, h("span", { className: "n", textContent: file.name, title: file.name }), pct, h("button", { type: "button", title: "Remove", textContent: "✕", onclick: () => (items.splice(items.indexOf(it), 1), it.chip.remove(), tell()) }));
      chips.append(it.chip);
      items.push(it);
      tell();
      (async () => {
        // A MEDIA file goes to its domain's MAKER (`kinds`: an image's thumbnail, a video's or an audio's renditions —
        // the same as its app's); anything else as it is.
        const m = kinds.mediaOf(file.type);
        const opts = { space, app: from?.app, public: typeof pub === "function" ? !!pub() : pub, onProgress: e => (pct.textContent = e.stage === "uploading" ? `${Math.round((e.p || 0) * 100)}%` : `${e.stage} ${Math.round((e.p || 0) * 100)}%`) };
        // PUBLISHED: the one upload form over the editor (`publisher.dialog`: kind, title, fields, who sees it — for whom
        // the item being written is, to start) — cancelled, nothing is sent and the file leaves the editor.
        if (m?.maker && publish) {
          pct.textContent = "publishing…";
          const done = await (await ctx.require("publisher")).dialog(file, { space: space && space.kind !== "account" ? space : null, initial: opts.public ? "public" : space && space.kind !== "account" ? "members" : "private", app: from?.app });
          if (!done) {
            items.splice(items.indexOf(it), 1);
            it.chip.remove();
            return;
          }
          it.ref = { ...done.ref, name: file.name };
          pct.textContent = sizeOf(file.size);
          told(it.ref, file);
          return;
        }
        const ref = m?.maker
          ? await (await ctx.require(m.maker)).make(file, opts)
          : await drive.upload(file, { space, from, public: opts.public, onProgress: e => (pct.textContent = e.phase === "reading" ? "reading…" : `${Math.round((100 * e.done) / e.size)}%`) });
        it.ref = { ...ref, name: file.name };
        pct.textContent = sizeOf(file.size);
        told(it.ref, file);
      })()
        .catch(e => {
          it.error = e.message ?? String(e);
          it.chip.classList.add("err");
          pct.textContent = "not sent";
          it.chip.title = it.error;
        })
        .finally(() => ((it.busy = false), tell()));
    }
    return {
      el,
      files: () => items.filter(i => i.ref).map(i => i.ref),
      busy: () => items.some(i => i.busy),
      failed: () => items.some(i => i.error),
      clear: () => (items.splice(0), chips.replaceChildren(), tell()),
      onChange: f => changed.push(f),
      // Files sent from elsewhere (the editor's 🖼); files already on the item (an edit: listed, nothing sent again);
      // told when each file is ready.
      addFiles: list => list.forEach(add),
      fromDrive,
      // Where what is attached goes: the space, and whether it is public now (a new item made from the editor goes there).
      where: () => ({ space, public: typeof pub === "function" ? !!pub() : pub }),
      preset: refs => (refs ?? []).forEach(r => ready(r, { quiet: true })),
      onReady: f => readied.push(f),
    };
  }

  // Open a file full: a MEDIA file in a dialog by its domain's VIEWER (`kinds`: an image drawn, a video or an audio
  // by `media-view` — the one player, streamed, as in its app); anything else saved.
  async function openFull(ref, note) {
    const m = kinds.mediaOf(ref);
    const show = (media, url = null) => {
      const d = h("dialog", { className: "cw-att-full" }, media, h("p", { textContent: `${ref.name} · ${sizeOf(ref.size)}` }));
      d.addEventListener("click", e => e.target === d && d.close());
      d.addEventListener("close", () => (d.remove(), url && URL.revokeObjectURL(url)));
      document.body.append(d);
      d.showModal();
    };
    if (m && m.view !== "image") return show((await ctx.require(m.view)).create({ file: ref, cover: false }).el);
    note.textContent = "Loading…";
    try {
      const blob = await files.get(ref, { onProgress: e => (note.textContent = `Loading ${Math.round((100 * e.done) / Math.max(1, e.size))}%`) });
      const url = URL.createObjectURL(blob);
      note.textContent = "";
      if (m) {
        show(h("img", { src: url, alt: ref.name }), url);
      } else {
        const a = h("a", { href: url, download: ref.name });
        document.body.append(a);
        a.click();
        a.remove();
        setTimeout(() => URL.revokeObjectURL(url), 60000);
      }
    } catch (e) {
      note.textContent = e.message ?? String(e);
    }
  }

  // SAVE TO DRIVE: a file someone shared, made yours — listed at once, copied into your space in the background (so
  // it stays yours whoever leaves where it came from).
  const saveButton = (r, note) =>
    h("button", {
      type: "button",
      className: "save",
      title: "Save to your Drive",
      textContent: "Save to Drive",
      onclick: e => files.adopt(r, null, { app: "drive" }).then(ref => drive.add(ref, { from: { saved: true } })).then(() => ((e.target.textContent = "In Drive ✓"), (e.target.disabled = true)), err => (note.textContent = err.message)),
    });

  // THE FILES of an item (what its text does not show inline): each the file look (`cards`: the same as in Drive, as
  // anywhere a file shows) — opened on a click, saved to your Drive from its menu.
  const cards = await ctx.require("cards");
  function show(refs) {
    const list = (Array.isArray(refs) ? refs : []).filter(r => r && typeof r === "object" && r.name && (r.inline || (r.key && r.root)));
    if (!list.length) return null;
    return h(
      "div",
      { className: "cw-att" },
      ...list.map(r => {
        const note = h("span", { className: "s" });
        const open = () => openFull(r, note);
        return cards.card({ kind: "file", files: [r], title: r.name }, { href: null, by: false, open, below: note, actions: [h("button", { type: "button", textContent: "Open", onclick: open }), saveButton(r, note)] });
      }),
    );
  }

  return { picker, fileButton, fitMenu, show, open: (ref, note = document.createElement("span")) => openFull(ref, note), sizeOf, isImage };
}
