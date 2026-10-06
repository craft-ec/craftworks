// CARDS, a capability: each KIND's LOOK — one per domain, whatever app shows it (a video looks the same in Video, in
// Discover, in someone's space, in Drive). An app picks its kinds and its layout; the look is the kind's. An item's
// ACTIONS are given (`actions`: elements beside it), never part of the look; its link is the one page (`items.pageOf`).
//
//   const cards = await ctx.require("cards");
//   cards.card(item, { href, actions, by, corner, below, open, lead, body })   // its card, by its kind (else its
//         // domain) — `actions`: its tools; `corner`: one element in its corner (a pin); `below`: under it (labels);
//         // `by`: false hides who made it; `open`: what a click does where it is not a link (`href` null: a note edited
//         // in place); `lead`: beside it, first (a post's votes); `body`: its content whole (its own page), instead of
//         // the list's preview
//   cards.embed(ref)          // ANY item embedded in text (`![title](item:REF)`: a post, a note, a file, a video…) —
//                             // read with its own access; a video or an audio plays where it is, the rest is its card
//   cards.ago(at)   cards.clock(seconds)   cards.counted(item)   // "3 hours ago", "1:02:03", " · 3 views · 1 save" — one wording everywhere
export async function start(ctx) {
  const [kinds, items, directory, signals, labelUI] = await Promise.all(["kinds", "items", "directory", "signals", "label-menu"].map(n => ctx.require(n)));
  const MANIFEST = "application/vnd.craftworks.video+json";
  const style = document.createElement("style");
  style.textContent = `@layer components {

    .cw-card { text-decoration: none; color: inherit; display: grid; gap: 6px; min-width: 0; }
    .cw-book .cover { position: relative; aspect-ratio: 2 / 3; border-radius: var(--cw-radius-sm); overflow: hidden; background: var(--cw-hover); display: grid; place-items: center; font-size: 2.5rem; box-shadow: 0 1px 4px rgba(0, 0, 0, .25); }
    .cw-book .cover img { width: 100%; height: 100%; object-fit: cover; }
    .cw-book .cover .kind { position: absolute; left: 6px; top: 6px; font-size: 11px; padding: 1px 6px; border-radius: 4px; background: rgba(0, 0, 0, .7); color: #fff; }
    .cw-pic { position: relative; display: block; border-radius: var(--cw-radius-sm); overflow: hidden; background: var(--cw-surface); color: var(--cw-fg); text-decoration: none; border: 1px solid var(--cw-line); }
    .cw-pic img { display: block; width: 100%; height: auto; }
    .cw-pic .none { aspect-ratio: 4 / 3; display: grid; place-items: center; font-size: 2.5rem; }
    /* Its title and who and when UNDER the picture (never over it: nothing hides until hover), then its row. */
    .cw-pic .over { padding: 6px 8px 2px; }
    .cw-pic .over .t { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .cw-pic .over .s { font-size: var(--cw-text-sm); color: var(--cw-muted); }
    .cw-pic .over .by { cursor: pointer; }
    .cw-card .thumb { position: relative; aspect-ratio: 16 / 9; background: #000; border-radius: var(--cw-radius); overflow: hidden;
      display: grid; place-items: center; font-size: 2rem; }
    .cw-card .thumb.sq { aspect-ratio: 1; }
    .cw-card .thumb img { width: 100%; height: 100%; object-fit: cover; }
    .cw-card .dur { position: absolute; right: 6px; bottom: 6px; background: rgba(0,0,0,.8); color: #fff; font-size: var(--cw-text-xs);
      padding: 1px 5px; border-radius: 4px; }
    .cw-card .kind { position: absolute; left: 6px; top: 6px; background: rgba(0,0,0,.7); color: #fff; font-size: var(--cw-text-xs);
      padding: 1px 6px; border-radius: 4px; }
    .cw-card .t { font-weight: 600; overflow-wrap: anywhere; }
    .cw-card .s { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-card .by { cursor: pointer; }
    .cw-card .by:hover { text-decoration: underline; }
    .cw-card .acts { display: flex; gap: 6px; flex-wrap: wrap; }
    .cw-note { break-inside: avoid; margin: 0 0 var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius);
      background: var(--cw-surface); padding: 12px 14px 6px; cursor: default; position: relative; display: block; gap: 0; }
    .cw-note[style*="background"] { color: var(--cw-on-pastel); border-color: transparent; }
    .cw-note .t { margin-bottom: 6px; }
    .cw-note .b { overflow-wrap: anywhere; max-height: 18em; overflow: hidden; }
    .cw-paste { display: grid; gap: 6px; padding: var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); color: inherit; text-decoration: none; min-width: 0; }
    .cw-paste .t { font-weight: 600; }
    .cw-paste .s { font-size: var(--cw-text-xs); color: var(--cw-muted); margin: 0; }
    .cw-paste pre.code { margin: 0; padding: var(--cw-space-2); max-height: 14em; overflow: auto; background: var(--cw-hover); border-radius: var(--cw-radius-sm);
      font: 12.5px/1.5 ui-monospace, Menlo, Consolas, monospace; white-space: pre; tab-size: 2; }
    .cw-page .cw-paste pre.code { max-height: none; font-size: 13px; }
    .cw-note .s { margin-top: 6px; font-size: var(--cw-text-xs); }
    .cw-note .acts { gap: 2px; margin-top: 6px; }
    .cw-note .acts button { border: 0; background: none; cursor: pointer; font-size: 15px; padding: var(--cw-space-1) 6px; border-radius: 50%; color: inherit; }
    .cw-note .acts button:hover { background: var(--cw-hover); }
    .cw-note .corner { position: absolute; top: 6px; right: 6px; }
    .cw-note .below { margin-top: var(--cw-space-2); }
    .cw-marks { display: flex; flex-wrap: wrap; gap: 4px; align-items: center; }
    .cw-marks:not(:has(.nsfw, .tag, .cw-chip)) { display: none; }
    .cw-marks .tag { border: 0; background: none; color: var(--cw-accent); font: inherit; font-size: var(--cw-text-xs); padding: 0 2px; cursor: pointer; }
    .cw-marks .tag:hover { text-decoration: underline; }
    .cw-marks .nsfw { font-size: var(--cw-text-xs); }
    .cw-embed { display: grid; gap: 6px; max-width: 420px; margin: var(--cw-space-2) 0; }
    .cw-file { border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); overflow: hidden;
      grid-template-rows: 120px auto; cursor: pointer; position: relative; gap: 0; }
    .cw-file:hover { border-color: var(--cw-muted); }
    .cw-file .pic { display: grid; place-items: center; background: var(--cw-hover); font-size: 2.4rem; overflow: hidden; }
    .cw-file .pic img { width: 100%; height: 100%; object-fit: cover; }
    .cw-file .cap { padding: 6px 8px; display: grid; gap: 2px; min-width: 0; }
    .cw-file .n { font-size: var(--cw-text-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cw-file .s { font-size: var(--cw-text-xs); }
    .cw-file .more { position: absolute; top: 4px; right: 4px; padding: 0 6px; border-radius: 50%; background: var(--cw-surface);
      border: 1px solid var(--cw-line); cursor: pointer; font: inherit; color: inherit; }
    .cw-file .menu { position: absolute; top: 28px; right: 4px; z-index: 5; display: grid; background: var(--cw-surface); border: 1px solid var(--cw-line);
      border-radius: var(--cw-radius-sm); box-shadow: var(--cw-shadow-lg); padding: 4px; }
    .cw-file .menu button, .cw-file .menu a { border: 0; background: none; text-align: left; padding: 6px 10px; font: inherit; color: inherit; text-decoration: none; cursor: pointer; }
    .cw-file .menu button:hover, .cw-file .menu a:hover { background: var(--cw-hover); }
}`;
  style.textContent += `
    .cw-post { display: block; color: inherit; text-decoration: none; min-width: 0; }
    .cw-post.link { border-bottom: 1px solid var(--cw-line); }
    .cw-post.link:hover { background: var(--cw-hover); cursor: pointer; }
    .cw-post .in { padding: var(--cw-space-2) var(--cw-space-1); display: grid; gap: 4px; min-width: 0; }
    .cw-post h1 { margin: 0; font-size: 1.45rem; line-height: 1.3; font-weight: 700; overflow-wrap: anywhere; }
    .cw-post .acts { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; margin-top: 2px; }
    .cw-post .meta { display: flex; flex-wrap: wrap; gap: 4px; align-items: baseline; color: var(--cw-muted); font-size: var(--cw-text-xs); }
    .cw-post .meta .b { color: var(--cw-fg); font-weight: 700; }
    .cw-post .meta .by:hover { text-decoration: underline; cursor: pointer; }
    .cw-post h3 { margin: 0; font-size: 1.1rem; font-weight: 600; overflow-wrap: anywhere; }
    .cw-post .text { overflow-wrap: anywhere; line-height: 1.5; margin: 0; font-size: var(--cw-text-sm); }
    .cw-post.link .text { color: var(--cw-muted); }
    /* NOTHING HIDES UNTIL HOVER (a touch screen has none): what is there is shown. */
    .cw-stack { position: relative; margin: 6px 0; --stack-h: 340px; }
    @media (max-width: 480px) { .cw-stack { --stack-h: 240px; } }
    .cw-stack .vp { height: var(--stack-h); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); overflow: hidden; background: var(--cw-bg);
      display: flex; align-items: center; justify-content: center; }
    .cw-stack .vp > * { width: 100%; height: 100%; max-width: none !important; max-height: none !important; margin: 0 !important; border: 0 !important; border-radius: 0 !important; }
    .cw-stack .vp img { object-fit: cover; }
    .cw-stack .vp video { object-fit: contain; background: #000; }
    .cw-stack .nav { position: absolute; top: calc(var(--stack-h) / 2); transform: translateY(-50%); background: rgba(0,0,0,.45); color: #fff; border: 0;
      border-radius: 50%; width: 34px; height: 34px; font-size: 22px; line-height: 1; cursor: pointer; z-index: 3; }
    .cw-stack .nav.prev { left: 8px; } .cw-stack .nav.next { right: 8px; }
    .cw-stack .dots { display: flex; gap: 6px; justify-content: center; margin-top: 6px; }
    .cw-stack .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--cw-line); cursor: pointer; }
    .cw-stack .dot.on { background: var(--cw-accent); }
    .cw-post.link .text .preview { display: -webkit-box; -webkit-line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; margin: 0; }
    .cw-track { display: flex; flex-wrap: wrap; gap: 6px 12px; align-items: baseline; padding: 6px 0; border-bottom: 1px solid var(--cw-line); }
    .cw-track .n { font-weight: 600; }
    .cw-track .acts { display: flex; gap: 6px; margin-left: auto; }`;
  document.head.append(style);
  // Read LATER (they use cards themselves: `attachments` shows a file as its card): taken as they arrive.
  const later = {};
  for (const n of ["markdown", "attachments", "space"]) ctx.require(n).then(m => (later[n] = m), () => {});
  const when = (n, f) => {
    if (later[n]) return f(later[n]);
    const ph = document.createElement("span");
    ctx.require(n).then(m => ph.replaceWith(f(m) ?? ""), () => ph.remove());
    return ph;
  };
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const ago = at => {
    const s = Math.max(0, (Date.now() - at) / 1000);
    if (s < 60) return "just now";
    for (const [n, u] of [[31536000, "year"], [2592000, "month"], [86400, "day"], [3600, "hour"], [60, "minute"]]) if (s >= n) return `${Math.floor(s / n)} ${u}${Math.floor(s / n) > 1 ? "s" : ""} ago`;
  };
  // Its COUNTED SIGNALS (`signals`: views, saves, shares — over the list's window): " · 3 views · 1 save".
  const counted = it => signals.summary(it);
  const clock = d => {
    d = Math.round(d || 0);
    const hh = Math.floor(d / 3600);
    const mm = Math.floor((d % 3600) / 60);
    const ss = String(d % 60).padStart(2, "0");
    return hh ? `${hh}:${String(mm).padStart(2, "0")}:${ss}` : `${mm}:${ss}`;
  };
  // The author: their name, opening their CARD (`person`: and Open, their home).
  const author = (did, app) =>
    // Their name: their CARD (what can be done with them, and Open: their home).
    directory.nameEl(did, "span", { className: "by", onclick: e => (e.preventDefault(), e.stopPropagation(), ctx.require("person").then(p => p.open(e.target, did))) });

  // VIDEO and AUDIO: a thumbnail (its preview, its length, its kind when not the plain one), its title, who and when.
  function media(it, { href, actions, by }) {
    const domain = kinds.domain(it.kind);
    const audio = domain === "audio";
    const f = it.files?.find(x => x.type === MANIFEST || /^(video|audio)\//.test(x.type ?? "")) ?? it.files?.[0] ?? null;
    return h(
      "a",
      { className: "cw-card", href },
      h(
        "div",
        { className: `thumb${audio ? " sq" : ""}` },
        f?.preview ? h("img", { src: f.preview, alt: "" }) : audio ? "🎵" : "🎬",
        f?.duration ? h("span", { className: "dur", textContent: clock(f.duration) }) : null,
        it.kind !== domain ? h("span", { className: "kind", textContent: kinds.of(it.kind)?.label ?? it.kind }) : null,
      ),
      audio && (it.meta?.artist || it.meta?.show || it.meta?.author) ? h("div", { className: "s", textContent: it.meta.artist ?? it.meta.show ?? it.meta.author }) : null,
      h("div", { className: "t", textContent: it.title }),
      h("div", { className: "s" }, by ? author(it.by, items.appOf(it.kind)) : null, `${by ? " · " : ""}${ago(it.at)}${counted(it)}${it.private ? " · only you" : ""}`),
      actions.length ? h("div", { className: "acts", onclick: e => e.preventDefault() }, ...actions) : null,
    );
  }

  // A NOTE: its colour, title and text (cut short), who wrote it (someone else's), its tools on hover.
  function note(it, { href, actions, by, corner, below, open }) {
    const c = h(
      href ? "a" : "div",
      { className: "cw-card cw-note", ...(href ? { href } : { tabIndex: 0 }) },
      it.title ? h("div", { className: "t", textContent: it.title }) : null,
      it.body ? h("div", { className: "b" }, when("markdown", m => m.render(it.body, it.files ?? [], { item: it.ref ?? null }))) : null,
      below ? h("div", { className: "below" }, below) : null,
      by ? h("div", { className: "s" }, author(it.by, "note")) : null,
      corner ? h("div", { className: "corner" }, corner) : null,
      actions.length ? h("div", { className: "acts" }, ...actions) : null,
    );
    if (it.meta?.color) c.style.background = it.meta.color;
    if (open) (c.onclick = e => !e.target.closest("button, .corner, .below, .by") && open()), (c.onkeydown = e => e.key === "Enter" && open());
    return c;
  }

  // A FILE (an image, a document, any file): its picture (an image's preview) or its kind's icon, its name, who, its
  // size and date; its actions in a ⋯ menu; a click opens it (`open`).
  const sizeOf = n => (n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(0)} KB` : n < 1073741824 ? `${(n / 1048576).toFixed(1)} MB` : `${(n / 1073741824).toFixed(2)} GB`);
  function file(it, { actions, by, open, below }) {
    const ref = it.files?.[0] ?? {};
    const image = kinds.mediaOf(ref)?.domain === "image";
    const src = ref.preview ?? (image && ref.inline ? `data:${ref.type};base64,${ref.inline}` : null);
    const menu = h("div", { className: "menu", hidden: true }, ...actions);
    menu.onclick = () => (menu.hidden = true);
    return h(
      "div",
      { className: "cw-card cw-file", title: ref.name ?? it.title, onclick: e => !e.target.closest(".more, .menu, .by") && open?.() },
      h("div", { className: "pic" }, src ? h("img", { src, alt: "" }) : (kinds.mediaOf(ref)?.icon ?? "📄")),
      h("div", { className: "cap" }, h("span", { className: "n", textContent: ref.name ?? it.title }), h("span", { className: "s" }, by ? author(it.by, "drive") : null, `${by ? " · " : ""}${sizeOf(ref.size ?? 0)}${it.at ? ` · ${new Date(it.at).toLocaleDateString()}` : ""}`), below ?? null),
      actions.length ? h("button", { type: "button", className: "more", title: "More", textContent: "⋯", onclick: () => (menu.hidden = !menu.hidden) }) : null,
      menu,
    );
  }

  // A CAROUSEL (Grid's media stack): a post's photos and videos, one at a time at one height — ‹ › and its dots, a
  // swipe on a phone; one alone as it is; none: nothing. At most 8.
  const VISUAL = new Set(["image", "video"]);
  function carousel(slides) {
    slides = slides.filter(Boolean).slice(0, 8);
    if (!slides.length) return null;
    const vp = h("div", { className: "vp" });
    const wrap = h("div", { className: `cw-stack${slides.length > 1 ? " multi" : ""}`, onclick: e => e.stopPropagation() }, vp);
    if (slides.length === 1) return vp.append(slides[0]), wrap;
    let at = 0;
    const dots = slides.map((_, i) => h("span", { className: "dot", onclick: e => (e.stopPropagation(), show(i)) }));
    const show = i => {
      at = (i + slides.length) % slides.length;
      vp.replaceChildren(slides[at]);
      dots.forEach((d, j) => d.classList.toggle("on", j === at));
    };
    wrap.append(
      h("button", { type: "button", className: "nav prev", textContent: "‹", title: "Previous", onclick: e => (e.stopPropagation(), show(at - 1)) }),
      h("button", { type: "button", className: "nav next", textContent: "›", title: "Next", onclick: e => (e.stopPropagation(), show(at + 1)) }),
      h("div", { className: "dots" }, ...dots),
    );
    let x0 = null;
    vp.addEventListener("touchstart", e => (x0 = e.touches[0].clientX), { passive: true });
    vp.addEventListener("touchend", e => {
      if (x0 == null) return;
      const dx = e.changedTouches[0].clientX - x0;
      if (Math.abs(dx) > 40) show(at + (dx < 0 ? 1 : -1));
      x0 = null;
    });
    show(0);
    return wrap;
  }

  // A POST (any text item: a post, a link, a question…): where it is, who and when, its title; in a list its text cut
  // short and its files' thumbnails (a list never plays anything) — on its own page (`body`) its content whole.
  function text(it, { href, actions, by, open, lead, body }) {
    const list = !body;
    const where = it.board ? h("span", { className: "b", textContent: `b/${it.board.name ?? it.board.id?.slice(0, 8)}` }) : h("span", { textContent: "profile" });
    if (it.board) when("space", s => void (where.textContent = `b/${s.shown(it.board)}`));
    // AS GRID'S: the title first, where and who and when under it, the text (cut short in a list; whole on its page),
    // then ONE row of what can be done — its votes (`lead`) first — the same in a list and on its page.
    const c = h(
      list && href ? "a" : "article",
      { className: `cw-post${list ? " link" : " full"}`, ...(list && href ? { href } : {}) },
      h(
        "div",
        { className: "in" },
        it.title ? h(list ? "h3" : "h1", { textContent: it.title }) : null,
        h(
          "div",
          { className: "meta" },
          where,
          it.pub ? h("span", { textContent: "· 🌐 public" }) : null,
          it.private ? h("span", { textContent: "· 🔒 only you" }) : null,
          by ? h("span", { textContent: "· Posted by" }) : null,
          by ? author(it.by, "board") : null,
          h("time", { textContent: ago(it.at), title: new Date(it.at).toLocaleString() }),
          it.edited ? h("span", { textContent: "(edited)" }) : null,
          counted(it) ? h("span", { textContent: counted(it).slice(1) }) : null,
        ),
        // In a list (Grid's): its photos and videos first, as ONE carousel; then its text cut short; then any other file.
        body ??
          h(
            "div",
            { className: "text" },
            it.files?.length ? when("markdown", m => carousel(it.files.filter(f => VISUAL.has(m.kindOf(f))).map(f => m.fileView(f, { item: it.ref, alt: it.title })))) : null,
            it.body ? when("markdown", m => h("p", { className: "preview", textContent: m.plain(it.body) })) : null,
            it.files?.length ? when("markdown", m => when("attachments", a => (it.files.some(f => !VISUAL.has(m.kindOf(f))) ? a.show(it.files.filter(f => !VISUAL.has(m.kindOf(f)))) : null))) : null,
          ),
        lead || actions.length ? h("div", { className: "acts", onclick: e => e.stopPropagation() }, lead, ...actions) : null,
      ),
    );
    if (list && open && !href) c.onclick = e => !e.target.closest("button, a, .by, .cw-votes") && open();
    return c;
  }

  // A CAPTION track (subtitles, lyrics, a transcript): its label and language, who made it and where it is kept, the
  // item it is for (`about`: its link, given), its tools.
  function caption(it, { actions, by, below }) {
    return h(
      "div",
      { className: "cw-track" },
      h("span", { className: "n", textContent: it.label || it.lang || "Subtitles" }),
      h("span", { className: "s", textContent: it.lang || "—" }),
      by ? h("span", { className: "s" }, "by ", author(it.by, "caption"), it.place ? ` · in ${it.place.name ?? "a space"}` : "") : null,
      below,
      actions.length ? h("span", { className: "acts" }, ...actions) : null,
    );
  }

  // A PICTURE on a wall (Images): the picture itself (its preview), its title and who over it on hover.
  function picture(it, { href = items.pageOf(it.ref, it.kind), by = true, actions = [] } = {}) {
    const ref = it.files?.find(x => kinds.mediaOf(x)?.domain === "image") ?? it.files?.[0] ?? {};
    const src = ref.preview ?? (ref.inline ? `data:${ref.type};base64,${ref.inline}` : null);
    return h(
      "a",
      { className: "cw-pic", href },
      src ? h("img", { src, alt: it.title ?? "", loading: "lazy" }) : h("div", { className: "none", textContent: "🖼" }),
      h("div", { className: "over" }, h("div", { className: "t", textContent: it.title }), h("div", { className: "s" }, by ? author(it.by, "image") : null, `${by ? " · " : ""}${ago(it.at)}${it.kind !== "image" ? ` · ${kinds.of(it.kind)?.label ?? it.kind}` : ""}${counted(it)}${it.private ? " · only you" : ""}`), marks(it)),
      actions.length ? h("div", { className: "acts", onclick: e => (e.preventDefault(), e.stopPropagation()) }, ...actions) : null,
    );
  }

  // A BOOK on a shelf: its cover (its preview), its title, its author (or a comic's writer), who and when.
  function book(it, { href, actions, by }) {
    const f = it.files?.find(x => kinds.mediaOf(x)?.domain === "book") ?? it.files?.[0] ?? null;
    return h(
      "a",
      { className: "cw-card cw-book", href },
      h("div", { className: "cover" }, f?.preview ? h("img", { src: f.preview, alt: "" }) : it.kind === "comic" ? "💬" : "📚", it.kind !== "book" ? h("span", { className: "kind", textContent: kinds.of(it.kind)?.label ?? it.kind }) : null),
      h("div", { className: "t", textContent: it.title }),
      it.meta?.author || it.meta?.writer ? h("div", { className: "s", textContent: it.meta.author ?? it.meta.writer }) : null,
      h("div", { className: "s" }, by ? author(it.by, "book") : null, `${by ? " · " : ""}${ago(it.at)}${f?.pages ? ` · ${f.pages} pages` : ""}${counted(it)}${it.private ? " · only you" : ""}`),
      actions.length ? h("div", { className: "acts", onclick: e => e.preventDefault() }, ...actions) : null,
    );
  }

  // ITS MARKS, the same on every look: 🔞 (tagged NSFW — shown because this person chose to see it, or theirs), its
  // TAGS (public, its author's: each opens every item with it, `#/tag/<tag>`) and this person's LABELS on it (private:
  // `label-menu`, one key for every kind). `labels: false`: the look shows them already (Notes' own chips).
  function marks(it, { labels = true } = {}) {
    const tags = items.tagsOf(it);
    const go = t => e => (e.preventDefault(), e.stopPropagation(), (location.hash = `#/tag/${encodeURIComponent(t)}`));
    return h(
      "div",
      // A click on its marks never follows the card's own link (a video's, a book's).
      { className: "cw-marks", onclick: e => (e.stopPropagation(), e.preventDefault()) },
      items.isNsfw(it) ? h("span", { className: "nsfw", title: "Adult content (NSFW)", textContent: "🔞" }) : null,
      ...tags.filter(t => t !== "nsfw").map(t => h("button", { type: "button", className: "tag", textContent: `#${t}`, title: `Everything tagged #${t}`, onclick: go(t) })),
      // Tags OTHERS put on it (the tag signal: each with how many put it), beside its author's.
      ...Object.entries(it.counts?.tag ?? {})
        .filter(([t]) => !tags.includes(t))
        .sort((a, b) => b[1] - a[1])
        .map(([t, n]) => h("button", { type: "button", className: "tag", textContent: `#${t} ${n}`, title: `${n} ${n === 1 ? "person" : "people"} tagged it #${t}`, onclick: go(t) })),
      labels && it.ref ? labelUI.chips(labelUI.key(it.ref)) : null,
    );
  }

  // Each kind's look: its own (a caption), else its domain's.
  // A PASTE: its title, its language, its code coloured (`highlight`) — whole on its page, its first lines in a list;
  // an expired one says so (the text not shown).
  function paste(it, { href, actions, by, open }) {
    const expired = !!it.meta?.expires && it.meta.expires < Date.now();
    const pre = h("pre", { className: "code" }, when("highlight", hl => hl.code(String(it.body ?? ""), it.meta?.language)));
    const lang = it.meta?.language && it.meta.language !== "plain" ? it.meta.language : "text";
    const c = h(
      href ? "a" : "div",
      { className: "cw-card cw-paste", ...(href ? { href } : { tabIndex: 0 }) },
      h("div", { className: "t", textContent: it.title || "Untitled paste" }),
      h("div", { className: "s" }, `${lang}`, it.meta?.expires ? ` · ${expired ? "expired" : `expires ${new Date(it.meta.expires).toLocaleString()}`}` : "", it.meta?.unlisted ? " · unlisted" : "", by ? h("span", {}, " · ", author(it.by, "paste")) : null),
      expired ? h("p", { className: "s", textContent: "This paste has expired." }) : pre,
      actions.length ? h("div", { className: "acts" }, ...actions) : null,
    );
    if (open) c.onclick = e => !e.target.closest("button, .by") && open();
    return c;
  }
  const LOOKS = { video: media, audio: media, book, note, file, image: file, document: file, text, caption, paste };
  function card(it, { href = items.pageOf(it.ref, it.kind), actions = [], by = true, corner = null, below = null, open = null, lead = null, body = null } = {}) {
    const look = LOOKS[it.kind] ?? LOOKS[kinds.domain(it.kind)];
    if (!look) throw new Error(`no look for ${it.kind} yet`);
    // Its marks: in the look's own place below it (a note, a file), else after it.
    const m = marks(it, { labels: !below });
    if (look === note || look === file) return look(it, { href, actions, by, corner, below: below ? h("div", {}, below, m) : m, open, lead, body });
    const el = look(it, { href, actions, by, corner, below, open, lead, body });
    (el.querySelector(".in") ?? el).append(m);
    return el;
  }
  // AN ITEM EMBEDDED — in a post, a comment, a message, a note, a mail: by its reference, read as its reader may (what
  // they may not read is said, never shown); its kind's look: a video or an audio its player, the rest its card.
  // Not there YET (its place still arriving on a page just opened): drawn when it arrives — as every list is, on
  // `items`' changes — until then said not to be available.
  function embed(ref) {
    const ph = h("div", { className: "cw-embed" }, h("span", { className: "s", textContent: "…" }));
    let shown = false;
    const none = () => !shown && ph.replaceChildren(h("span", { className: "s", textContent: "(not available to you)" }));
    const draw = () =>
      items
        .get(ref)
        .then(async it => {
          if (shown) return;
          if (!it) return none();
          shown = true;
          // ADULT CONTENT someone chose not to see: said, shown on asking (as its page asks).
          if (items.isNsfw(it) && !(await items.nsfwShown()) && !(await items.visible([it])).length)
            return ph.replaceChildren(h("button", { type: "button", className: "s", textContent: "🔞 Adult content (NSFW) — show", onclick: e => (e.preventDefault(), e.stopPropagation(), show(it)) }));
          await show(it);
        })
        .catch(none);
    const show = async it => {
      const main = MEDIA.has(kinds.domain(it.kind)) ? (it.files?.find(x => x.type === MANIFEST || /^(video|audio)\//.test(x.type ?? "")) ?? null) : null;
      if (main) return ph.replaceChildren((await ctx.require("markdown")).fileView(main, { item: it.ref, alt: it.title }), card(it, { by: true }));
      ph.replaceChildren(card(it, { by: true }));
    };
    draw();
    items.onChange(() => !shown && draw());
    return ph;
  }
  const MEDIA = new Set(["video", "audio"]);

  return { card, picture, embed, ago, clock, counted, marks, author, sizeOf };
}
