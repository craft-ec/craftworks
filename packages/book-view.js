// BOOK VIEW, a component: the ONE way a book or a comic is read — on its Books page (`media-look`), and written inline
// in a post or a comment (a COVER until opened: nothing loads before). The READER (`book-studio`): PAGES (a PDF, a
// comic) one at a time or as a SPREAD of two, a comic right to left when asked (manga); CHAPTERS (an EPUB) flowing,
// its text larger or smaller. ◀ ▶ and the arrow keys turn; where the reader was is kept on this device, per book.
//
//   const bv = await ctx.require("book-view");
//   const v = bv.create({ file, item, cover })   // file: the book's reference; cover: true → its cover until opened
//   host.append(v.el)
//   v.open()                                     // open the reader now (a cover: its click does)
export async function start(ctx) {
  const [files, studio, kinds] = await Promise.all(["files", "book-studio", "kinds"].map(n => ctx.require(n)));
  const style = document.createElement("style");
  style.textContent = `@layer components {

    .cw-bv { display: grid; gap: 6px; max-width: 100%; }
    .cw-bv .cover { position: relative; padding: 0; cursor: pointer; width: min(200px, 100%); aspect-ratio: 2 / 3; overflow: hidden; color: #fff; background: #222;
      border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); }
    .cw-bv .cover img { width: 100%; height: 100%; object-fit: cover; display: block; }
    .cw-bv .cover .ic { position: absolute; inset: 0; display: grid; place-items: center; font-size: 2.5rem; opacity: .6; }
    .cw-bv .cover .cap { position: absolute; left: 0; right: 0; bottom: 0; padding: 6px 8px; font-size: 12px; text-align: left;
      background: linear-gradient(transparent, rgba(0, 0, 0, .75)); }
    .cw-bv .bar { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-bv .bar button { font: inherit; cursor: pointer; border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); border-radius: var(--cw-radius-sm); padding: 3px 10px; }
    .cw-bv .bar button.on { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
    .cw-bv .bar .grow { flex: 1; }
    .cw-bv .pages { display: flex; justify-content: center; gap: 4px; background: #1a1a1a; border-radius: var(--cw-radius); padding: 8px; min-height: 240px; outline: none; overflow: auto; }
    .cw-bv .pages.rtl { flex-direction: row-reverse; }
    .cw-bv .pages > * { max-width: 100%; height: auto; box-shadow: 0 1px 6px rgba(0, 0, 0, .5); background: #fff; }
    .cw-bv .pages.two > * { max-width: calc(50% - 2px); }
    .cw-bv .pages img { max-height: 85vh; object-fit: contain; }
    .cw-bv .flow { width: 100%; height: 75vh; border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: #fff; }
    .cw-bv .note { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-bv:fullscreen { background: var(--cw-bg); padding: 12px; overflow: auto; }
    .cw-bv:fullscreen .flow { height: calc(100vh - 80px); }
}`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  // WHERE THE READER WAS, per book, on this device (a convenience: lost with the browser's storage, nothing else).
  const placeKey = (item, file) => `cw-read:${item ?? file?.key ?? file?.name}`;
  const placeOf = k => {
    try {
      return JSON.parse(localStorage.getItem(k) ?? "null") ?? {};
    } catch {
      return {};
    }
  };
  const keepPlace = (k, v) => {
    try {
      localStorage.setItem(k, JSON.stringify(v));
    } catch {}
  };

  function create({ file, item = null, cover = true, kind = null } = {}) {
    const el = h("div", { className: "cw-bv" });
    const comic = kind === "comic" || studio.formatOf(file) === "cbz";
    const note = h("span", { className: "note" });
    let opened = false;

    async function open() {
      if (opened) return;
      opened = true;
      el.replaceChildren(note);
      note.textContent = "Loading the book…";
      let book;
      try {
        const blob = await files.get(file, { onProgress: e => (note.textContent = `Loading ${Math.round((100 * e.done) / Math.max(1, e.size))}%`) });
        note.textContent = "Opening…";
        book = await studio.open(blob, studio.formatOf(file) ?? undefined);
      } catch (e) {
        note.textContent = e.message ?? String(e);
        opened = false;
        return;
      }
      note.textContent = "";
      const key = placeKey(item, file);
      const place = { at: 0, two: false, rtl: false, size: 100, ...placeOf(key) };
      const count = book.pages ?? book.chapters ?? 0;
      const flow = !book.pages;
      const where = h("span");
      const pages = h("div", { className: "pages", tabIndex: 0 });
      const frame = flow ? h("iframe", { className: "flow", sandbox: "", title: "The book" }) : null;
      const btn = (textContent, onclick, title = "") => h("button", { type: "button", textContent, onclick, title });
      const prev = btn("◀", () => turn(place.rtl && !flow ? 1 : -1), "Back");
      const next = btn("▶", () => turn(place.rtl && !flow ? -1 : 1), "On");
      const twoBtn = flow ? null : btn("Spread", () => ((place.two = !place.two), draw()), "Two pages side by side");
      const rtlBtn = flow || !comic ? null : btn("Right to left", () => ((place.rtl = !place.rtl), draw()), "Read right to left (manga)");
      const smaller = flow ? btn("A−", () => ((place.size = Math.max(60, place.size - 10)), draw())) : null;
      const larger = flow ? btn("A+", () => ((place.size = Math.min(220, place.size + 10)), draw())) : null;
      const whole = btn("⛶", () => (document.fullscreenElement ? document.exitFullscreen() : el.requestFullscreen?.()), "Full screen");
      el.addEventListener("fullscreenchange", () => draw());
      const bar = h("div", { className: "bar" }, prev, where, next, h("span", { className: "grow" }), twoBtn, rtlBtn, smaller, larger, whole);
      const step = () => (!flow && place.two ? 2 : 1);
      const turn = d => {
        const at = Math.max(0, Math.min(count - 1, place.at + d * step()));
        if (at !== place.at) (place.at = at), draw();
      };
      pages.onkeydown = e => (e.key === "ArrowRight" ? (turn(place.rtl && !flow ? -1 : 1), e.preventDefault()) : e.key === "ArrowLeft" ? (turn(place.rtl && !flow ? 1 : -1), e.preventDefault()) : null);
      let drawing = 0;
      async function draw() {
        const mine = ++drawing;
        keepPlace(key, place);
        twoBtn?.classList.toggle("on", place.two);
        rtlBtn?.classList.toggle("on", place.rtl);
        const last = !flow && place.two ? Math.min(count, place.at + 2) : place.at + 1; // the last page in view
        const shown = flow ? `Chapter ${place.at + 1} of ${count}` : last > place.at + 1 ? `Pages ${place.at + 1}–${last} of ${count}` : `Page ${place.at + 1} of ${count}`;
        where.textContent = `${shown} · ${Math.round((100 * last) / Math.max(1, count))}%`;
        try {
          if (flow) {
            // The reader's look goes IN the chapter (its frame is of no origin the reader can reach into).
            const style = `html { font-size: ${place.size}%; } body { max-width: 42em; margin: 1.5em auto; padding: 0 1.2em; line-height: 1.6; font-family: Georgia, serif; color: #222; background: #fff; } img, svg { max-width: 100%; height: auto; }`;
            const { html } = await book.chapter(place.at, { style });
            if (mine !== drawing) return;
            frame.srcdoc = html;
            return;
          }
          pages.classList.toggle("two", place.two);
          pages.classList.toggle("rtl", place.rtl);
          // A page fits the reader: its share of the width, and the screen's height.
          const width = Math.max(320, (pages.clientWidth || 800) - 16) / (place.two ? 2 : 1);
          const height = Math.max(320, (document.fullscreenElement ? innerHeight - 80 : innerHeight * 0.85) - 16);
          const shownPages = await Promise.all([place.at, ...(place.two && place.at + 1 < count ? [place.at + 1] : [])].map(i => book.page(i, width, height)));
          if (mine === drawing) pages.replaceChildren(...shownPages);
        } catch (e) {
          if (mine === drawing) note.textContent = e.message ?? String(e);
        }
      }
      if (!count) {
        note.textContent = "This book has no pages that can be read here.";
        return;
      }
      place.at = Math.min(place.at, count - 1);
      el.replaceChildren(bar, flow ? frame : pages, note);
      await draw();
    }

    if (cover) {
      const k = kinds.mediaOf(file);
      el.append(
        h(
          "button",
          { type: "button", className: "cover", title: `Read ${file?.name ?? "the book"}`, onclick: () => open() },
          file?.preview ? h("img", { src: file.preview, alt: "" }) : h("span", { className: "ic", textContent: k?.icon ?? "📚" }),
          h("span", { className: "cap", textContent: `📖 Read${file?.pages ? ` · ${file.pages} pages` : ""}` }),
        ),
      );
    } else queueMicrotask(open);
    return { el, open };
  }

  return { create };
}
