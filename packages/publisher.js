// PUBLISHER, a capability: the ONE way a new FILE becomes an ITEM of its kind — from its app's upload page (Videos,
// Audio, Images) or uploaded inline in an editor (a post, a comment): the same path either way. Its MAKER (`kinds`:
// a video's or an audio's renditions, an image's thumbnail) makes it; its own TAGS name it (title; artist, album, year,
// genre where its kind has those fields); a COVER chosen replaces its own; an item of its kind is made with whom it is
// for (`audience`: so policy, Discover and feeds treat it as they treat its kind); its SUBTITLES and the LYRICS it
// carries become its timed text. A file taken from Drive is never published again: it is an item already.
//
//   const publisher = await ctx.require("publisher");
//   const { item, ref } = await publisher.publish(file, { space, audience, kind, title, body, meta, cover, subtitles,
//                                                         keepOriginal, app, onProgress })
//   host.append(await publisher.form({ domain, space, app, onPublished }))   // the upload form (its app's page)
//   const done = await publisher.dialog(file, { space, initial, app })      // the same form over an editor (or null)
export async function start(ctx) {
  const [kinds, items] = await Promise.all(["kinds", "items"].map(n => ctx.require(n)));

  async function publish(file, { space = null, audience = space ? "members" : "private", kind = null, title = "", body = "", meta = {}, cover = null, subtitles = [], keepOriginal = false, app = null, onProgress = () => {} } = {}) {
    const spec = kinds.mediaOf(file.type);
    if (!spec) throw new Error(`${file.name}: not a kind that is published (an image, a video, an audio)`);
    kind ??= spec.kind;
    const maker = await ctx.require(spec.maker);
    const ref = await maker.make(file, { space, public: audience === "public", app: app ?? spec.domain, keepOriginal, onProgress });
    // A COVER chosen: over the file's own.
    if (cover) ref.preview = (await (await ctx.require("image-studio")).thumbnail(cover)) ?? ref.preview;
    // Its own TAGS: the fields its kind has, unless given.
    const tags = ref.tags ?? {};
    const fields = Object.fromEntries((kinds.of(kind)?.fields ?? []).map(f => [f, tags[f]]).filter(([, v]) => v != null && v !== ""));
    const m = { ...fields, ...Object.fromEntries(Object.entries(meta).filter(([, v]) => v != null && v !== "")) };
    // A video or an audio sent as it is (not encoded here) carries its video id on the item (a manifest has its own).
    if (spec.maker === "video-studio" && ref.type !== maker.MANIFEST && ref.key) m.vid = await maker.videoId(ref.key);
    const name = String(title || tags.title || file.name.replace(/\.[^.]+$/, "")).trim().slice(0, 300);
    const item = await items.submit({ board: space?.id ?? null, title: name, body, kind, meta: m, audience, files: [ref] });
    // Its TIMED TEXT: the subtitles given, else the lyrics the file carries (untimed: one cue over the whole).
    if (spec.maker === "video-studio") {
      const subs = await ctx.require("subtitle-store");
      for (const s of subtitles) await subs.add(item, s).catch(e => ctx.log("publish", { what: `timed text ${s.name}: ${e.message}` }));
      const lyrics = !subtitles.length && spec.domain === "audio" ? (await maker.probe(file).catch(() => null))?.lyrics : null;
      if (lyrics)
        await subs
          .add(item, `WEBVTT\n\n00:00:00.000 --> ${new Date(Math.max(1, ref.duration || 3600) * 1000).toISOString().slice(11, 23)}\n${lyrics.trim()}\n`, { label: kinds.attachLabel("subtitle", kind) })
          .catch(() => {});
    }
    return { item, ref };
  }
  // THE FORM — the ONE upload form for a media file, wherever it is made: its app's upload page (`form`, in place) or an
  // editor (`dialog`, over the post being written). Its kinds (the domain's: Video · Movie · Episode …), the fields of
  // the kind chosen, a cover (an audio's), subtitles or lyrics, keep the original, a title and a description — filled
  // from the file's own tags — and whom it is for (`audience`). `file`: given (an editor's), else picked here.
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const style = document.createElement("style");
  style.textContent = `
    .cw-pub { display: grid; gap: 8px; }
    .cw-pub .s { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-pub input:not([type=checkbox]):not([type=file]), .cw-pub textarea, .cw-pub select { font: inherit; padding: 6px 8px; border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); background: var(--cw-surface); color: var(--cw-fg); }
    .cw-pub .row { display: flex; gap: 8px; align-items: center; }
    .cw-pub .said { color: var(--cw-danger); margin: 0; }
    .cw-pub .go { border: 0; border-radius: var(--cw-radius-pill); padding: 6px 16px; background: var(--cw-accent); color: var(--cw-accent-fg); font-weight: 600; cursor: pointer; }
    .cw-pub .ghost { border: 1px solid var(--cw-line); border-radius: var(--cw-radius-pill); padding: 6px 16px; background: none; color: var(--cw-fg); cursor: pointer; }
    dialog.cw-pub-dlg { max-width: min(560px, 92vw); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-bg); color: var(--cw-fg); }`;
  document.head.append(style);

  async function form({ domain, file = null, space = null, initial = null, app = null, onPublished = () => {}, onCancel = null } = {}) {
    const spec = kinds.media().find(m => m.domain === (domain ?? kinds.mediaOf(file?.type)?.domain));
    if (!spec) throw new Error("not a kind that is published");
    const av = spec.maker === "video-studio";
    const said = h("p", { className: "said", hidden: true });
    const progress = h("span", { className: "s" });
    const kindSel = h("select", { name: "kind" }, ...kinds.inDomain(spec.domain).map(k => h("option", { value: k, textContent: kinds.of(k).label })));
    const fieldsBox = h("div", { style: "display:grid;gap:6px" });
    const drawFields = () => fieldsBox.replaceChildren(...kinds.of(kindSel.value).fields.map(x => h("input", { name: `meta.${x}`, placeholder: kinds.fieldLabel(x) })));
    kindSel.onchange = drawFields;
    drawFields();
    const who = await (await ctx.require("audience")).picker({ space, kind: spec.kind, initial });
    const f = h(
      "form",
      { className: "cw-pub" },
      file ? h("p", { className: "s", textContent: `${spec.icon} ${file.name}` }) : h("input", { type: "file", name: "file", accept: spec.accept, required: true, onchange: e => prefill(e.target.files[0]) }),
      spec.domain === "audio" ? h("label", { className: "s" }, "Cover (an image, optional: else the file's own) ", h("input", { type: "file", name: "cover", accept: "image/*" })) : null,
      av ? h("label", { className: "s" }, `${spec.domain === "audio" ? "Lyrics or transcript" : "Subtitles"} (.vtt or .srt, optional) `, h("input", { type: "file", name: "subs", accept: ".vtt,.srt,text/vtt", multiple: true })) : null,
      av ? h("label", { className: "s" }, h("input", { type: "checkbox", name: "keep" }), " Keep the original file too (as large as all the versions together; lets a newer format be made later)") : null,
      h("input", { name: "title", placeholder: "Title", required: true, maxLength: 300 }),
      h("textarea", { name: "body", rows: 3, placeholder: "Description" }),
      h("label", { className: "s" }, "What it is ", kindSel),
      fieldsBox,
      who.el,
      h("div", { className: "row" }, h("button", { className: "go", textContent: "Publish" }), onCancel ? h("button", { type: "button", className: "ghost", textContent: "Cancel", onclick: () => onCancel() }) : null, progress),
      said,
    );
    // The file's own TAGS fill the form (title; artist, album, year, genre where its kind has them).
    const prefill = async x => {
      if (!x) return;
      const t = av ? await (await ctx.require("video-studio")).probe(x).catch(() => null) : null;
      if (!f.elements.title.value) f.elements.title.value = t?.title || x.name.replace(/\.[^.]+$/, "");
      for (const [k, val] of Object.entries({ artist: t?.artist, album: t?.album, year: t?.year, genre: t?.genre })) {
        const input = f.elements[`meta.${k}`];
        if (val && input && !input.value) input.value = val;
      }
    };
    if (file) prefill(file);
    f.onsubmit = async e => {
      e.preventDefault();
      said.hidden = true;
      const x = file ?? f.elements.file?.files?.[0];
      if (!x) return;
      const btn = f.querySelector("button.go");
      btn.disabled = true;
      try {
        progress.textContent = "Reading…";
        const meta = Object.fromEntries(kinds.of(kindSel.value).fields.map(k => [k, String(f.elements[`meta.${k}`]?.value ?? "").trim()]).filter(([, v]) => v));
        const done = await publish(x, {
          space,
          audience: who.value(),
          kind: kindSel.value,
          title: f.elements.title.value,
          body: f.elements.body.value,
          meta,
          cover: f.elements.cover?.files?.[0] ?? null,
          subtitles: [...(f.elements.subs?.files ?? [])],
          keepOriginal: !!f.elements.keep?.checked,
          app,
          onProgress: p => (progress.textContent = `${p.stage[0].toUpperCase()}${p.stage.slice(1)}${p.p ? ` ${Math.round(100 * p.p)}%` : "…"}`),
        });
        onPublished(done);
      } catch (err) {
        said.textContent = err.message ?? String(err);
        said.hidden = false;
        btn.disabled = false;
        progress.textContent = "";
      }
    };
    return f;
  }
  // THE FORM OVER AN EDITOR: published ({ item, ref }) or cancelled (null).
  function dialog(file, { space = null, initial = null, app = null } = {}) {
    return new Promise(async resolve => {
      const d = h("dialog", { className: "cw-pub-dlg" });
      const end = v => (d.close(), d.remove(), resolve(v));
      d.addEventListener("cancel", e => (e.preventDefault(), end(null)));
      try {
        d.append(h("h3", { textContent: `Publish ${file.name}` }), await form({ file, space, initial, app, onPublished: end, onCancel: () => end(null) }));
      } catch (e) {
        return resolve(Promise.reject(e));
      }
      document.body.append(d);
      d.showModal();
    });
  }
  return { publish, form, dialog };
}
