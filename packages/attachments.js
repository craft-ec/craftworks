// ATTACHMENTS, a component: FILES on an item (a message, a post, a mail) — picked with 📎 (each sent at once, with its
// progress; the item waits for them), and SHOWN: an image as its thumbnail (made here when picked, kept in the
// reference: a list never downloads the image), opened full on a click; any other file as its name and size, with
// Download. The bytes are `files`' (sealed, coded, raced); the reference rides in the item, so who reads the item
// reads its files — and nobody else.
//
//   const att = await ctx.require("attachments");
//   const pick = att.picker({ space, public })   // { el, files(): [ref], busy(): bool, clear(), onChange(fn) }
//                                                // public: a boolean, or a function asked when each file is picked
//   host.append(att.show(item.files))            // nothing for none
export async function start(ctx) {
  const files = await ctx.require("files");
  const style = document.createElement("style");
  style.textContent = `
    .cw-att-pick { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
    .cw-att-pick > button.clip { border: 0; background: none; font-size: 1.1rem; cursor: pointer; padding: 2px 4px; color: var(--cw-muted); }
    .cw-att-pick > button.clip:hover { color: var(--cw-fg); }
    .cw-att-chip { display: inline-flex; align-items: center; gap: 6px; border: 1px solid var(--cw-line); border-radius: var(--cw-radius-pill);
      padding: 2px 4px 2px 8px; font-size: var(--cw-text-xs); max-width: 260px; background: var(--cw-surface); }
    .cw-att-chip .n { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cw-att-chip .p { color: var(--cw-muted); }
    .cw-att-chip.err { border-color: var(--cw-danger); color: var(--cw-danger); }
    .cw-att-chip button { border: 0; background: none; cursor: pointer; color: var(--cw-muted); padding: 0 2px; }
    .cw-att { display: flex; flex-wrap: wrap; gap: var(--cw-space-2); margin-top: 6px; }
    .cw-att img { max-width: min(320px, 100%); max-height: 240px; border-radius: var(--cw-radius-sm); cursor: zoom-in; display: block;
      border: 1px solid var(--cw-line); background: var(--cw-surface); }
    .cw-att .file { display: inline-flex; align-items: center; gap: var(--cw-space-2); border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm);
      padding: 6px var(--cw-space-2); font-size: var(--cw-text-sm); background: var(--cw-surface); max-width: 100%; }
    .cw-att .file .n { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 240px; }
    .cw-att .file .s { color: var(--cw-muted); font-size: var(--cw-text-xs); }
    .cw-att .file button { font: inherit; border: 1px solid var(--cw-line); background: none; color: var(--cw-fg); border-radius: var(--cw-radius-sm);
      padding: 2px 8px; cursor: pointer; }
    .cw-att-full { border: 0; padding: 0; background: transparent; max-width: 96vw; max-height: 96vh; }
    .cw-att-full::backdrop { background: rgba(0, 0, 0, .8); }
    .cw-att-full img, .cw-att-full video { max-width: 96vw; max-height: 92vh; display: block; }
    .cw-att-full p { color: #fff; margin: 8px 0 0; font-size: var(--cw-text-sm); }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const sizeOf = n => (n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(0)} KB` : n < 1073741824 ? `${(n / 1048576).toFixed(1)} MB` : `${(n / 1073741824).toFixed(2)} GB`);
  const isImage = r => /^image\//.test(r?.type ?? "");

  // An image's THUMBNAIL (at most 320 px, WebP): kept in the reference, what every list shows.
  async function thumbnail(file) {
    try {
      const bmp = await createImageBitmap(file);
      const s = Math.min(1, 320 / Math.max(bmp.width, bmp.height));
      const c = Object.assign(document.createElement("canvas"), { width: Math.max(1, Math.round(bmp.width * s)), height: Math.max(1, Math.round(bmp.height * s)) });
      c.getContext("2d").drawImage(bmp, 0, 0, c.width, c.height);
      const blob = await new Promise(r => c.toBlob(r, "image/webp", 0.75));
      if (!blob || blob.size > 48 * 1024) return null;
      return await new Promise(r => {
        const fr = new FileReader();
        fr.onload = () => r(fr.result);
        fr.readAsDataURL(blob);
      });
    } catch {
      return null;
    }
  }

  function picker({ space = null, public: pub = false } = {}) {
    const input = h("input", { type: "file", multiple: true, hidden: true });
    const chips = h("span", { className: "cw-att-pick" });
    const el = h("span", { className: "cw-att-pick" }, h("button", { type: "button", className: "clip", title: "Attach files", ariaLabel: "Attach files", textContent: "📎", onclick: () => input.click() }), chips, input);
    const items = []; // { file, ref, busy, error, chip }
    const changed = [];
    const tell = () => changed.forEach(f => f());
    input.onchange = () => {
      for (const file of input.files) add(file);
      input.value = "";
    };
    function add(file) {
      const it = { file, ref: null, busy: true, error: null };
      const pct = h("span", { className: "p", textContent: "0%" });
      it.chip = h("span", { className: "cw-att-chip" }, h("span", { className: "n", textContent: file.name, title: file.name }), pct, h("button", { type: "button", title: "Remove", textContent: "✕", onclick: () => (items.splice(items.indexOf(it), 1), it.chip.remove(), tell()) }));
      chips.append(it.chip);
      items.push(it);
      tell();
      (async () => {
        const preview = isImage(file) ? await thumbnail(file) : null;
        const ref = await files.put(file, { space, public: typeof pub === "function" ? !!pub() : pub, onProgress: e => (pct.textContent = e.phase === "reading" ? "reading…" : `${Math.round((100 * e.done) / e.size)}%`) });
        it.ref = preview ? { ...ref, preview } : ref;
        pct.textContent = sizeOf(file.size);
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
    };
  }

  // Open a file full: an image or a video in a dialog, anything else saved.
  async function openFull(ref, note) {
    note.textContent = "Loading…";
    try {
      const blob = await files.get(ref, { onProgress: e => (note.textContent = `Loading ${Math.round((100 * e.done) / Math.max(1, e.size))}%`) });
      const url = URL.createObjectURL(blob);
      note.textContent = "";
      if (isImage(ref) || /^video\//.test(ref.type)) {
        const media = isImage(ref) ? h("img", { src: url, alt: ref.name }) : h("video", { src: url, controls: true, autoplay: true });
        const d = h("dialog", { className: "cw-att-full" }, media, h("p", { textContent: `${ref.name} · ${sizeOf(ref.size)}` }));
        d.addEventListener("click", () => d.close());
        d.addEventListener("close", () => (d.remove(), URL.revokeObjectURL(url)));
        document.body.append(d);
        d.showModal();
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

  function show(refs) {
    const list = (Array.isArray(refs) ? refs : []).filter(r => r && typeof r === "object" && r.name && (r.inline || (r.key && r.root)));
    if (!list.length) return null;
    return h(
      "div",
      { className: "cw-att" },
      ...list.map(r => {
        const note = h("span", { className: "s" });
        const src = r.preview ?? (isImage(r) && r.inline ? `data:${r.type};base64,${r.inline}` : null);
        if (src)
          return h("figure", { style: "margin:0" }, h("img", { src, alt: r.name, title: `${r.name} · ${sizeOf(r.size)}`, onclick: () => openFull(r, note) }), note);
        return h("span", { className: "file" }, h("span", { textContent: /^video\//.test(r.type) ? "🎬" : /^audio\//.test(r.type) ? "🎵" : "📄" }), h("span", { className: "n", textContent: r.name, title: r.name }), h("span", { className: "s", textContent: sizeOf(r.size) }), h("button", { type: "button", textContent: /^video\//.test(r.type) ? "Play" : "Download", onclick: () => openFull(r, note) }), note);
      }),
    );
  }

  return { picker, show };
}
