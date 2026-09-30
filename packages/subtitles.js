// SUBTITLES, a page (the Subtitles app): the lens on subtitle data (`subtitle-store`) — YOURS (`#/subtitles`: every
// track you made, on any video), a SPACE's (`#/s/<space>/subtitles`: a group working on its videos' subtitles — every
// track on them, by any member, and each of its videos to add one to), a media item's (`#/subtitles/for/<ref>`: its tracks, by anyone, and one added — a
// file or pasted text), and one track EDITED (`#/subtitles/e/<ref>`: its label, language and cues as WebVTT; exported
// as WebVTT or SRT: the data is portable). UI only.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [subs, items, directory, theme, space] = await Promise.all(["subtitle-store", "items", "directory", "theme", "space"].map(n => ctx.require(n)));
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
    const n = h("span", { textContent: directory.shown(did) });
    directory.name(did).then(t => (n.textContent = t), () => {});
    return n;
  };
  const sp = () => (ctx.space && ctx.space !== "discover" ? ctx.space : null);
  const route = () => {
    const s = ctx.sub || "";
    if (sp() && !s) return { space: sp() };
    if (s.startsWith("for/")) return { for: decodeURIComponent(s.slice(4)) };
    if (s.startsWith("e/")) return { edit: decodeURIComponent(s.slice(2)) };
    return { mine: true };
  };
  const watchHref = ref => (ref.startsWith("space:") ? `#/s/${ref.slice(6, ref.indexOf("/"))}/videos/w/${encodeURIComponent(ref)}` : `#/videos/w/${encodeURIComponent(ref)}`);
  const download = (text, name, type) => {
    const a = h("a", { href: URL.createObjectURL(new Blob([text], { type })), download: name });
    document.body.append(a);
    a.click();
    a.remove();
  };
  const title = async ref => (await items.get(ref).catch(() => null))?.title ?? "a video";
  const row = async t =>
    h(
      "li",
      {},
      h("span", { className: "n", textContent: t.label || t.lang || "Subtitles" }),
      h("span", { className: "s", textContent: t.lang || "—" }),
      h("span", { className: "s" }, "by ", who(t.by)),
      h("a", { href: watchHref(t.in), textContent: `▶ ${await title(t.in)}` }),
      t.by === me ? h("a", { href: `#/subtitles/e/${encodeURIComponent(t.ref)}`, textContent: "Edit" }) : null,
      h("button", { type: "button", textContent: ".vtt", onclick: async () => download(await subs.text(t), `${t.label || "subtitles"}.vtt`, "text/vtt") }),
      h("button", { type: "button", textContent: ".srt", onclick: async () => download(subs.toSrt(await subs.text(t)), `${t.label || "subtitles"}.srt`, "application/x-subrip") }),
    );

  async function mine() {
    const list = await subs.mine();
    return h("div", {}, h("p", { className: "s", textContent: "Every subtitle track you made, on any video. Add one from a video's page (“Add subtitles”)." }), list.length ? h("ul", {}, ...(await Promise.all(list.map(row)))) : h("p", { className: "none", textContent: "None yet." }));
  }

  // A SPACE's subtitle work: its videos, each with its tracks (any member's) and a way to add one.
  async function inSpace(id) {
    const kindsCap = await ctx.require("kinds");
    const videos = await items.list({ board: id }, "new", kindsCap.inDomain("video"));
    if (!videos.length) return h("p", { className: "none", textContent: "No videos in this space yet: subtitles go with its videos." });
    const blocks = await Promise.all(
      videos.map(async v => {
        const tracks = await subs.of(v.ref);
        return h("div", {}, h("h3", {}, h("a", { href: watchHref(v.ref), textContent: `▶ ${v.title}` }), " ", h("a", { className: "s", href: `#/subtitles/for/${encodeURIComponent(v.ref)}`, textContent: "Add a track" })), tracks.length ? h("ul", {}, ...(await Promise.all(tracks.map(row)))) : h("p", { className: "none", textContent: "No subtitles yet." }));
      }),
    );
    return h("div", {}, h("p", { className: "s", textContent: "This space's videos and their subtitles — any member adds a track or a translation; each edits their own." }), ...blocks);
  }

  async function forItem(ref) {
    const list = await subs.of(ref);
    const said = h("p", { className: "said", hidden: true });
    const f = h(
      "form",
      {},
      h("h3", { textContent: "Add a track" }),
      h("input", { type: "file", name: "file", accept: ".vtt,.srt,text/vtt" }),
      h("textarea", { name: "text", rows: 6, placeholder: "…or paste WebVTT or SRT here", style: "min-height:120px" }),
      h("div", { className: "row" }, h("input", { name: "label", placeholder: "Label (e.g. English)" }), h("input", { name: "lang", placeholder: "Language (en)", style: "width:9em" }), h("button", { className: "go", textContent: "Add" })),
      said,
    );
    f.onsubmit = async e => {
      e.preventDefault();
      const src = f.elements.file.files[0] ?? (f.elements.text.value.trim() || null);
      if (!src) return;
      said.hidden = true;
      await subs.add(ref, src, { label: f.elements.label.value.trim(), lang: f.elements.lang.value.trim() }).then(draw, err => ((said.textContent = err.message ?? String(err)), (said.hidden = false)));
    };
    return h("div", {}, h("h3", {}, "For ", h("a", { href: watchHref(ref), textContent: `▶ ${await title(ref)}` })), list.length ? h("ul", {}, ...(await Promise.all(list.map(row)))) : h("p", { className: "none", textContent: "No subtitles yet." }), f);
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
      h("div", { className: "row" }, "For ", h("a", { href: watchHref(t.in), textContent: `▶ ${await title(t.in)}` })),
      h("div", { className: "row" }, label, lang),
      text,
      h(
        "div",
        { className: "row" },
        h("button", { className: "go", type: "button", textContent: "Save", onclick: async () => ((said.textContent = "Saving…"), await subs.update(ref, { label: label.value.trim(), lang: lang.value.trim(), text: text.value }).then(() => (said.textContent = "Saved."), e => (said.textContent = e.message))) }),
        h("button", { type: "button", textContent: "Export .vtt", onclick: () => download(subs.toVtt(text.value), `${label.value || "subtitles"}.vtt`, "text/vtt") }),
        h("button", { type: "button", textContent: "Export .srt", onclick: () => download(subs.toSrt(text.value), `${label.value || "subtitles"}.srt`, "application/x-subrip") }),
        h("button", { type: "button", textContent: "Delete", onclick: async () => (await subs.remove(ref), (location.hash = "#/subtitles")) }),
        said,
      ),
    );
  }

  let drawn = "";
  async function draw() {
    const w = route();
    drawn = ctx.sub ?? "";
    const top = h("div", { className: "top" }, h("h2", { textContent: "🔤 Subtitles" }), h("a", { href: "#/subtitles", textContent: "Yours" }));
    root.replaceChildren(top, theme.loading("Reading…"));
    const body = await (w.space ? inSpace(w.space) : w.for ? forItem(w.for) : w.edit ? editor(w.edit) : mine()).catch(e => h("p", { className: "said", textContent: e.message ?? String(e) }));
    if (drawn === (ctx.sub ?? "")) root.replaceChildren(top, body);
  }
  await draw();
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/subtitles" && (ctx.sub ?? "") !== drawn && draw());
}
