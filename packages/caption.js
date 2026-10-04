// CAPTION, a page (the Caption app): the lens on captions — subtitles, lyrics, transcripts (`caption-store`) — YOURS (`#/caption`: every
// track you made, on any video), a SPACE's (`#/s/<space>/subtitles`: a group working on its videos' subtitles — every
// track on them, by any member, and each of its videos to add one to), a media item's (`#/caption/for/<ref>`: its tracks, by anyone, and one added — a
// file or pasted text), and one track EDITED (`#/caption/e/<ref>`: its label, language and cues as WebVTT; exported
// as WebVTT or SRT: the data is portable). UI only.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [subs, items, directory, theme, space] = await Promise.all(["caption-store", "items", "directory", "theme", "space"].map(n => ctx.require(n)));
  // The FEED BAR (every content app's): which tracks, over which window, in which order.
  const bar = (await ctx.require("feed-bar")).create({ start: "new", onChange: () => draw() });
  const me = (await space.account()).id;
  el.innerHTML = `
    <style>
      .sb { max-width: 900px; margin: 0 auto; display: grid; gap: var(--cw-space-3); }
      .sb h2 { margin: 0; font-size: 1.3rem; }
      .sb .top { display: flex; gap: var(--cw-space-3); align-items: center; flex-wrap: wrap; }
      .sb .top a { color: var(--cw-muted); text-decoration: none; }
      .sb ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
      .sb li { display: flex; gap: var(--cw-space-3); align-items: center; flex-wrap: wrap; padding: 8px 10px; border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); }
      .sb li .n { font-weight: 600; }
      .sb .s { color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .sb form, .sb .ed { display: grid; gap: var(--cw-space-2); }
      .sb input, .sb textarea { font: inherit; padding: 6px 8px; border-radius: var(--cw-radius-sm); border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); }
      .sb textarea { font-family: var(--cw-mono, monospace); font-size: 13px; min-height: 360px; }
      .sb .row { display: flex; gap: var(--cw-space-2); flex-wrap: wrap; align-items: center; }
      .sb button { font: inherit; cursor: pointer; border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); border-radius: 999px; padding: 5px 14px; }
      .sb button.go { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
      .sb .said { color: var(--cw-danger); margin: 0; }
      .sb .none { color: var(--cw-muted); }
    </style>
    <div class="sb"></div>`;
  const root = el.querySelector(".sb");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const who = did => {
    const n = directory.nameEl(did);
    return n;
  };
  // WHERE (`where`: yours, a space's, a person's — their tracks —, Discover — public videos and audio, each with
  // its tracks), read at each draw.
  const where = await ctx.require("where");
  const actionsCap = await ctx.require("actions");
  let at = await where.of({ kind: "caption", app: "caption", yours: "Your captions" });
  const sp = () => at.space?.id ?? null;
  const route = () => {
    const s = at.sub;
    if (sp() && !s) return { space: sp() };
    if (at.who === "discover" && !s) return { discover: true };
    if (at.who === "person") return { person: at.person };
    if (at.saved) return { saved: true };
    // `for/<item>~<file key>`: a media file inside an item (inline in a post).
    if (s.startsWith("for/")) {
      const [item, key] = s.slice(4).split("~");
      return { for: decodeURIComponent(item), file: key ?? null };
    }
    if (s.startsWith("e/")) return { edit: decodeURIComponent(s.slice(2)) };
    return { mine: true };
  };
  // Where an item is shown: the one link (`items.pageOf`) — a video or a track where it plays, a post on its board.
  const itemsCap0 = await ctx.require("items");
  const watchHref = (ref, kind = null) => itemsCap0.pageOf(ref, kind ?? "video");
  const postHref = ref => itemsCap0.pageOf(ref, "post");
  const download = (text, name, type) => {
    const a = h("a", { href: URL.createObjectURL(new Blob([text], { type })), download: name });
    document.body.append(a);
    a.click();
    a.remove();
  };
  const itemOf = async ref => await items.get(ref).catch(() => null);
  const title = async ref => (await itemOf(ref))?.title ?? "an item";
  const row = async t =>
    h(
      "li",
      {},
      h("span", { className: "n", textContent: t.label || t.lang || "Subtitles" }),
      h("span", { className: "s", textContent: t.lang || "—" }),
      h("span", { className: "s" }, "by ", who(t.by), t.place ? ` · in ${t.place.name ?? "a space"}` : ""),
      await (async () => { const it = await itemOf(t.in); return h("a", { href: watchHref(t.in, it?.kind), textContent: `▶ ${it?.title ?? "an item"}` }); })(),
      t.by === me ? h("a", { href: `#/caption/e/${encodeURIComponent(t.ref)}`, textContent: "Edit" }) : actionsCap.save(t),
      h("button", { type: "button", textContent: ".vtt", onclick: async () => download(await subs.text(t), `${t.label || "subtitles"}.vtt`, "text/vtt") }),
      h("button", { type: "button", textContent: ".srt", onclick: async () => download(subs.toSrt(await subs.text(t)), `${t.label || "subtitles"}.srt`, "application/x-subrip") }),
    );

  async function mine(by = null, saved = false) {
    // SAVED: `where`'s (the tracks you saved).
    const list = saved ? (await at.read()).map(subs.shape) : bar.reorder(await subs.mine({ sort: bar.sort(), ...bar.options(), ...(by ? { by } : {}) }));
    return h(
      "div",
      {},
      h("p", { className: "s", textContent: by ? "Their subtitle, lyrics and transcript tracks, in their space." : "Every subtitle, lyrics or transcript track you made, on any video or audio. Add one from its page." }),
      bar.el(),
      list.length ? h("ul", {}, ...(await Promise.all(list.map(row)))) : h("p", { className: "none", textContent: `None ${bar.span()}.` }),
      bar.older("Older", list.length),
    );
  }

  // A SPACE's subtitle work — or DISCOVER's (the public network's): its videos and audio, each with its tracks (any
  // reader's) and a way to add one.
  async function inSpace(id) {
    const kindsCap = await ctx.require("kinds");
    const media = [...kindsCap.inDomain("video"), ...kindsCap.inDomain("audio")];
    const videos = await items.list(id ? { board: id } : { discover: true }, "new", media, id ? {} : bar.options());
    if (!videos.length) return h("p", { className: "none", textContent: id ? "No videos or audio in this space yet: subtitles, lyrics and transcripts go with them." : "No public videos or audio yet." });
    const blocks = await Promise.all(
      videos.map(async v => {
        const tracks = await subs.of(v.ref);
        return h("div", {}, h("h3", {}, h("a", { href: watchHref(v.ref, v.kind), textContent: `▶ ${v.title}` }), " ", h("a", { className: "s", href: `#/caption/for/${encodeURIComponent(v.ref)}`, textContent: "Add a track" })), tracks.length ? h("ul", {}, ...(await Promise.all(tracks.map(row)))) : h("p", { className: "none", textContent: "No subtitles yet." }));
      }),
    );
    return h("div", {}, h("p", { className: "s", textContent: id ? "This space's videos and their subtitles — any member adds a track or a translation; each edits their own." : "Public videos and audio, and their subtitles, lyrics and transcripts — anyone adds a track or a translation." }), ...(id ? [] : [bar.el()]), ...blocks);
  }

  async function forItem(ref, fileKey = null) {
    // A FILE inside the item: its own tracks (by its video id), made for it.
    const itemNow = fileKey ? await itemOf(ref) : null;
    const markdownCap = fileKey ? await ctx.require("markdown") : null;
    const file = fileKey ? (itemNow?.files ?? []).find(f => markdownCap.keyOf(f) === fileKey) ?? null : null;
    if (fileKey && !file) return h("p", { className: "none", textContent: "That file is not in this item (any more)." });
    const list = file ? await subs.ofFile(ref, file) : await subs.of(ref);
    // The places a track can be kept: with the video (where this person may add), their own, their spaces with Subtitles.
    const roles = await ctx.require("roles");
    const mine = await space.mine();
    const teams = [];
    for (const s of mine.filter(x => x.kind === "server")) if ((await roles.of(s).catch(() => null))?.apps().includes("caption")) teams.push(s);
    const withVideo = ref.startsWith("space:") ? mine.some(s => ref.startsWith(`space:${s.id}/`)) : true;
    const placeSel = h(
      "select",
      { name: "place" },
      ...(withVideo ? [h("option", { value: "with", textContent: "with the video" })] : []),
      h("option", { value: "own", textContent: "as your own" }),
      ...teams.filter(s => !ref.startsWith(`space:${s.id}/`)).map(s => h("option", { value: s.id, textContent: `in ${space.shown(s)}` })),
    );
    const said = h("p", { className: "said", hidden: true });
    const f = h(
      "form",
      {},
      h("h3", { textContent: "Add a track" }),
      h("input", { type: "file", name: "file", accept: ".vtt,.srt,text/vtt" }),
      h("textarea", { name: "text", rows: 6, placeholder: "…or paste WebVTT or SRT here", style: "min-height:120px" }),
      h("div", { className: "row" }, h("input", { name: "label", placeholder: "Label (e.g. English)" }), h("input", { name: "lang", placeholder: "Language (en)", style: "width:9em" })),
      // WHERE it is kept (like a Drive file): with the video, your own, or a team's space — found by the video's id.
      h("div", { className: "row" }, h("span", { className: "s", textContent: "Keep it " }), placeSel, h("button", { className: "go", textContent: "Add" })),
      said,
    );
    f.onsubmit = async e => {
      e.preventDefault();
      const src = f.elements.file.files[0] ?? (f.elements.text.value.trim() || null);
      if (!src) return;
      said.hidden = true;
      const v = placeSel.value;
      const place = v === "with" ? undefined : v === "own" ? null : mine.find(s => s.id === v);
      await subs.add(ref, src, { label: f.elements.label.value.trim(), lang: f.elements.lang.value.trim(), ...(file ? { file } : {}), ...(place !== undefined ? { place } : {}) }).then(draw, err => ((said.textContent = err.message ?? String(err)), (said.hidden = false)));
    };
    const it = itemNow ?? (await itemOf(ref));
    return h("div", {}, h("h3", {}, "For ", file ? `${file.name} in ` : "", h("a", { href: file ? postHref(ref) : watchHref(ref, it?.kind), textContent: `${file ? "" : "▶ "}${it?.title ?? "an item"}` })), list.length ? h("ul", {}, ...(await Promise.all(list.map(row)))) : h("p", { className: "none", textContent: "No subtitles yet." }), f);
  }

  async function editor(ref) {
    const t = (await subs.mine()).find(x => x.ref === ref);
    if (!t) return h("p", { className: "none", textContent: "Only its author edits a track." });
    const text = h("textarea", { value: await subs.text(t) });
    const label = h("input", { value: t.label, placeholder: "Label" });
    const lang = h("input", { value: t.lang, placeholder: "Language", style: "width:9em" });
    const said = h("p", { className: "s" });
    return h(
      "div",
      { className: "ed" },
      await (async () => { const it = await itemOf(t.in); return h("div", { className: "row" }, "For ", h("a", { href: watchHref(t.in, it?.kind), textContent: `▶ ${it?.title ?? "an item"}` })); })(),
      h("div", { className: "row" }, label, lang),
      text,
      h(
        "div",
        { className: "row" },
        h("button", { className: "go", type: "button", textContent: "Save", onclick: async () => ((said.textContent = "Saving…"), await subs.update(ref, { label: label.value.trim(), lang: lang.value.trim(), text: text.value }).then(() => (said.textContent = "Saved."), e => (said.textContent = e.message))) }),
        h("button", { type: "button", textContent: "Export .vtt", onclick: () => download(subs.toVtt(text.value), `${label.value || "subtitles"}.vtt`, "text/vtt") }),
        h("button", { type: "button", textContent: "Export .srt", onclick: () => download(subs.toSrt(text.value), `${label.value || "subtitles"}.srt`, "application/x-subrip") }),
        h("button", { type: "button", textContent: "Delete", onclick: async () => (await subs.remove(ref), (location.hash = "#/caption")) }),
        said,
      ),
    );
  }

  let drawn = "";
  async function draw() {
    at = await where.of({ kind: "caption", app: "caption", yours: "Your captions" });
    const w = route();
    drawn = where.key();
    const top = h("div", { className: "top" }, h("h2", { textContent: w.person ? `🔤 ${directory.shown(w.person)}'s captions` : "🔤 Caption" }));
    // THE TABS (`where`'s, in every app's order): Your captions · Discover.
    at.tabs([], { yoursOn: !!w.mine && !w.saved });
    root.replaceChildren(top, theme.loading("Reading…"));
    const body = await (w.space ? inSpace(w.space) : w.discover ? inSpace(null) : w.for ? forItem(w.for, w.file) : w.edit ? editor(w.edit) : w.person ? mine(w.person) : w.saved ? mine(null, true) : mine()).catch(e => h("p", { className: "said", textContent: e.message ?? String(e) }));
    if (drawn === where.key()) root.replaceChildren(top, body);
  }
  await draw();
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/caption" && where.key() !== drawn && draw());
}
