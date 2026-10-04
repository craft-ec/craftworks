// CARDS, a capability: each KIND's LOOK — one per domain, whatever app shows it (a video looks the same in Video, in
// Discover, in someone's space, in Drive). An app picks its kinds and its layout; the look is the kind's. An item's
// ACTIONS are given (`actions`: elements beside it), never part of the look; its link is the one page (`items.pageOf`).
//
//   const cards = await ctx.require("cards");
//   cards.card(item, { href, actions, by, corner, below, open })   // its card, by its kind's domain — `actions`: its
//         // tools; `corner`: one element in its corner (a pin); `below`: under it (labels); `by`: false hides who made
//         // it; `open`: what a click does where it is not a link (`href` null: a note edited in place)
//   cards.embed(ref)          // ANY item embedded in text (`![title](item:REF)`: a post, a note, a file, a video…) —
//                             // read with its own access; a video or an audio plays where it is, the rest is its card
//   cards.ago(at)   cards.clock(seconds)      // "3 hours ago", "1:02:03" — one wording everywhere
export async function start(ctx) {
  const [kinds, items, directory] = await Promise.all(["kinds", "items", "directory"].map(n => ctx.require(n)));
  const MANIFEST = "application/vnd.craftworks.video+json";
  const style = document.createElement("style");
  style.textContent = `
    .cw-card { text-decoration: none; color: inherit; display: grid; gap: 6px; min-width: 0; }
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
    .cw-note .b { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 18em; overflow: hidden; }
    .cw-note .s { margin-top: 6px; font-size: var(--cw-text-xs); }
    .cw-note .acts { gap: 2px; opacity: 0; transition: opacity .15s; margin-top: 6px; }
    .cw-note:hover .acts, .cw-note:focus-visible .acts, .cw-note:has(:focus-visible) .acts { opacity: 1; }
    .cw-note .acts button { border: 0; background: none; cursor: pointer; font-size: 15px; padding: var(--cw-space-1) 6px; border-radius: 50%; color: inherit; }
    .cw-note .acts button:hover { background: var(--cw-hover); }
    .cw-note .corner { position: absolute; top: 6px; right: 6px; }
    .cw-note .corner .cw-pin[aria-pressed="false"] { opacity: 0; }
    .cw-note:hover .corner .cw-pin[aria-pressed="false"] { opacity: .45; }
    .cw-note .below { margin-top: var(--cw-space-2); }
    .cw-embed { display: grid; gap: 6px; max-width: 420px; margin: var(--cw-space-2) 0; }
    .cw-file { border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); overflow: hidden;
      grid-template-rows: 120px auto; cursor: pointer; position: relative; gap: 0; }
    .cw-file:hover { border-color: var(--cw-muted); }
    .cw-file .pic { display: grid; place-items: center; background: var(--cw-hover); font-size: 2.4rem; overflow: hidden; }
    .cw-file .pic img { width: 100%; height: 100%; object-fit: cover; }
    .cw-file .cap { padding: 6px 8px; display: grid; gap: 2px; min-width: 0; }
    .cw-file .n { font-size: var(--cw-text-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cw-file .s { font-size: var(--cw-text-xs); }
    .cw-file .more { position: absolute; top: 4px; right: 4px; padding: 0 6px; border-radius: 50%; opacity: 0; background: var(--cw-surface);
      border: 1px solid var(--cw-line); cursor: pointer; font: inherit; color: inherit; }
    .cw-file:hover .more, .cw-file .more:focus-visible { opacity: 1; }
    .cw-file .menu { position: absolute; top: 28px; right: 4px; z-index: 5; display: grid; background: var(--cw-surface); border: 1px solid var(--cw-line);
      border-radius: var(--cw-radius-sm); box-shadow: var(--cw-shadow-lg); padding: 4px; }
    .cw-file .menu button, .cw-file .menu a { border: 0; background: none; text-align: left; padding: 6px 10px; font: inherit; color: inherit; text-decoration: none; cursor: pointer; }
    .cw-file .menu button:hover, .cw-file .menu a:hover { background: var(--cw-hover); }`;
  document.head.append(style);
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
  const clock = d => {
    d = Math.round(d || 0);
    const hh = Math.floor(d / 3600);
    const mm = Math.floor((d % 3600) / 60);
    const ss = String(d % 60).padStart(2, "0");
    return hh ? `${hh}:${String(mm).padStart(2, "0")}:${ss}` : `${mm}:${ss}`;
  };
  // The author: their name, opening their space at the same app (`where`: a person's space).
  const author = (did, app) =>
    directory.nameEl(did, "span", { className: "by", onclick: e => (e.preventDefault(), e.stopPropagation(), (location.hash = app ? `#/${app}/u/${did}` : `#/u/${did}`)) });

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
      h("div", { className: "s" }, by ? author(it.by, items.appOf(it.kind)) : null, `${by ? " · " : ""}${ago(it.at)}${it.private ? " · only you" : ""}`),
      actions.length ? h("div", { className: "acts", onclick: e => e.preventDefault() }, ...actions) : null,
    );
  }

  // A NOTE: its colour, title and text (cut short), who wrote it (someone else's), its tools on hover.
  function note(it, { href, actions, by, corner, below, open }) {
    const c = h(
      href ? "a" : "div",
      { className: "cw-card cw-note", ...(href ? { href } : { tabIndex: 0 }) },
      it.title ? h("div", { className: "t", textContent: it.title }) : null,
      it.body ? h("div", { className: "b", textContent: it.body }) : null,
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

  const LOOKS = { video: media, audio: media, note, file, image: file, document: file };
  function card(it, { href = items.pageOf(it.ref, it.kind), actions = [], by = true, corner = null, below = null, open = null } = {}) {
    const look = LOOKS[kinds.domain(it.kind)];
    if (!look) throw new Error(`no look for ${it.kind} yet`);
    return look(it, { href, actions, by, corner, below, open });
  }
  // AN ITEM EMBEDDED — in a post, a comment, a message, a note, a mail: by its reference, read as its reader may (what
  // they may not read is said, never shown); its kind's look: a video or an audio its player, the rest its card.
  function embed(ref) {
    const ph = h("div", { className: "cw-embed" }, h("span", { className: "s", textContent: "…" }));
    items
      .get(ref)
      .then(async it => {
        if (!it) return ph.replaceChildren(h("span", { className: "s", textContent: "(not available to you)" }));
        const main = MEDIA.has(kinds.domain(it.kind)) ? (it.files?.find(x => x.type === MANIFEST || /^(video|audio)\//.test(x.type ?? "")) ?? null) : null;
        if (main) return ph.replaceChildren((await ctx.require("markdown")).fileView(main, { item: it.ref, alt: it.title }), card(it, { by: true }));
        ph.replaceChildren(card(it, { by: true }));
      })
      .catch(() => ph.replaceChildren(h("span", { className: "s", textContent: "(not available to you)" })));
    return ph;
  }
  const MEDIA = new Set(["video", "audio"]);

  return { card, embed, ago, clock, author, sizeOf };
}
