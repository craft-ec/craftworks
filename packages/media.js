// MEDIA, a page: ONE page for every MEDIA app — a LENS on a content DOMAIN (`kinds`) chosen by its route: VIDEOS
// (`#/videos`: video, movie, TV episode, music video, short) and AUDIO (`#/audio`: music, podcast, audiobook), YouTube-
// and Spotify-shaped; a composed app shows several the same way. Its SUB-TYPES filter the list. CHANNEL-FIRST: yours
// (each public, or only you) and those you follow (`…`: the feed; `…/mine`; `…/c/<did>`: someone's); SAVED (the pin
// edge); DISCOVER. In a SPACE (`#/s/<space>/…`): its members' items, public while its domain reads in public. WATCH/
// LISTEN: `…/w/<ref>` — streamed by byte range (`video-player`), a like (▲), comments, and its timed text
// (`subtitle-store`: Subtitles on a video, Lyrics on a song, a Transcript on a podcast — shown in time for audio).
// UPLOAD: `…/up` (tags and cover from the file). UI only.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [items, directory, person, theme, space, roles, drive, player, kinds, edge] = await Promise.all(["items", "directory", "person", "theme", "space", "roles", "drive-store", "video-player", "kinds", "edge"].map(n => ctx.require(n)));
  const [studio, files, subs, mediaView] = await Promise.all(["video-studio", "files", "subtitle-store", "media-view"].map(n => ctx.require(n)));
  const [pins, people] = await Promise.all([edge.pins(), edge.people()]);
  // WHICH APP this page is (its route): its domain, its words.
  const APPS = {
    "/videos": { app: "videos", domain: "video", icon: "▶️", name: "Videos", one: "video", ones: "videos", accept: "video/*", mine: "Your channel", audio: false },
    "/audio": { app: "audio", domain: "audio", icon: "🎧", name: "Audio", one: "track", ones: "tracks", accept: "audio/*", mine: "Your library", audio: true },
  };
  const C_ROUTE = APPS[ctx.route] ? ctx.route : "/videos";
  const C = APPS[C_ROUTE];
  const VIDEO = kinds.inDomain(C.domain);
  let only = null; // a SUB-TYPE the list is filtered to (null: all)
  // The FEED BAR (shared by every content app, as Grid's): the feed, its window, and a sort of what is shown.
  const bar = (await ctx.require("feed-bar")).create({ start: "new", onChange: () => draw() });
  const me = (await space.account()).id;
  el.innerHTML = `
    <style>
      .vd { max-width: 1200px; margin: 0 auto; display: grid; gap: var(--cw-space-3); }
      .vd .top { display: flex; align-items: center; gap: var(--cw-space-2); flex-wrap: wrap; }
      .vd .top h2 { margin: 0; font-size: 1.3rem; }
      .vd .tabs { display: flex; gap: var(--cw-space-2); flex: 1; }
      .vd .tabs a { color: var(--cw-muted); text-decoration: none; padding: 4px 10px; border-radius: 999px; }
      .vd .tabs a.on { color: var(--cw-fg); background: var(--cw-hover); }
      .vd .up { background: var(--cw-accent); color: var(--cw-accent-fg); border-radius: var(--cw-radius-sm); padding: 6px 12px; text-decoration: none; }
      .vd .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: var(--cw-space-4) var(--cw-space-3); }
      .vd .card { text-decoration: none; color: inherit; display: grid; gap: 6px; }
      .vd .thumb { position: relative; aspect-ratio: 16 / 9; background: #000; border-radius: var(--cw-radius); overflow: hidden; display: grid; place-items: center; font-size: 2rem; }
      .vd .thumb img { width: 100%; height: 100%; object-fit: cover; }
      .vd .dur { position: absolute; right: 6px; bottom: 6px; background: rgba(0,0,0,.8); color: #fff; font-size: var(--cw-text-xs); padding: 1px 5px; border-radius: 4px; }
      .vd .kind { position: absolute; left: 6px; top: 6px; background: rgba(0,0,0,.7); color: #fff; font-size: var(--cw-text-xs); padding: 1px 6px; border-radius: 4px; }
      .vd .t { font-weight: 600; line-height: 1.3; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
      .vd .s { color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .vd .by { cursor: pointer; }
      .vd .by:hover { color: var(--cw-fg); }
      .vd .none { color: var(--cw-muted); text-align: center; padding: var(--cw-space-5); }
      .vd .watch { display: grid; gap: var(--cw-space-3); max-width: 1000px; }
      .vd .watch video { width: 100%; max-height: 70vh; background: #000; border-radius: var(--cw-radius); }
      .vd .watch h1 { font-size: 1.25rem; margin: 0; }
      .vd .row { display: flex; gap: var(--cw-space-3); align-items: center; flex-wrap: wrap; }
      .vd button { font: inherit; cursor: pointer; border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); border-radius: 999px; padding: 5px 14px; }
      .vd button.on { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
      .vd .about { background: var(--cw-hover); border-radius: var(--cw-radius); padding: var(--cw-space-3); white-space: pre-wrap; }
      .vd .meta { display: flex; gap: var(--cw-space-3); flex-wrap: wrap; font-size: var(--cw-text-sm); color: var(--cw-muted); }
      .vd form { display: grid; gap: var(--cw-space-2); max-width: 640px; }
      .vd input, .vd textarea, .vd select { font: inherit; padding: 6px 8px; border-radius: var(--cw-radius-sm); border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); }
      .vd .said { color: var(--cw-danger); margin: 0; }
      .vd .scrub { position: relative; height: 10px; background: var(--cw-hover); border-radius: 5px; cursor: pointer; }
      .vd .scrub .done { position: absolute; inset: 0 auto 0 0; background: var(--cw-accent); border-radius: 5px; pointer-events: none; }
      .vd .scrub .tip { position: absolute; bottom: 16px; transform: translateX(-50%); border: 2px solid #fff; border-radius: 6px; box-shadow: var(--cw-shadow-lg);
        background-repeat: no-repeat; pointer-events: none; display: none; }
      .vd .scrub .tip span { position: absolute; bottom: 2px; left: 0; right: 0; text-align: center; color: #fff; font-size: 11px; text-shadow: 0 0 3px #000; }
      .vd .level { font-size: var(--cw-text-xs); color: var(--cw-muted); }
      .vd .chips { flex-basis: 100%; display: flex; gap: 6px; flex-wrap: wrap; }
      .vd .chips button { padding: 3px 12px; font-size: var(--cw-text-sm); }
      .vd .grid.sq { grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); }
      .vd .thumb.sq { aspect-ratio: 1; }
      .vd .cover { width: min(320px, 70vw); aspect-ratio: 1; border-radius: var(--cw-radius); overflow: hidden; background: var(--cw-hover); display: grid; place-items: center; font-size: 4rem; }
      .vd .cover img { width: 100%; height: 100%; object-fit: cover; }
      .vd .watch audio { width: 100%; }
      .vd .timed { max-height: 320px; overflow: auto; border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); }
      .vd .timed p { margin: 4px 0; cursor: pointer; color: var(--cw-muted); }
      .vd .timed p.on { color: var(--cw-fg); font-weight: 600; }
      .vd .subs { display: grid; gap: 6px; border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); }
      .vd .subs .row input[name=label] { width: 12em; }
      .vd .subs .row input[name=lang] { width: 4em; }
    </style>
    <div class="vd"></div>`;
  const root = el.querySelector(".vd");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const discovering = () => ctx.space === "discover";
  const sp = () => (ctx.space && !discovering() ? ctx.space : null);
  const base = () => (discovering() ? `#/discover/${C.app}` : sp() ? `#/s/${sp()}/${C.app}` : `#/${C.app}`);
  const route = () => {
    const s = ctx.sub || "";
    if (s.startsWith("w/")) return { watch: decodeURIComponent(s.slice(2)) };
    if (s === "up") return { up: true };
    if (s.startsWith("c/")) return { by: decodeURIComponent(s.slice(2)) };
    if (discovering()) return { discover: true };
    if (sp()) return { board: sp() };
    return s === "mine" ? { by: me } : s === "saved" ? { saved: true } : { feed: true };
  };
  // A public space's video seen from OUTSIDE (Discover, a followed space): its public description.
  const outsideOf = async ref => {
    if (!ref.startsWith("space:")) return null;
    const id = ref.slice(6, ref.indexOf("/"));
    if ((await space.mine()).some(x => x.id === id)) return null;
    return (await items.publicSpaces().catch(() => [])).find(d => d.id === id) ?? people.about("follow", id);
  };
  const SAVED = ref => `${C.app}:${ref}`;
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
  const who = did => {
    const n = directory.nameEl(did, "span", { className: "by", onclick: e => (e.preventDefault(), e.stopPropagation(), (location.hash = `#/${C.app}/c/${encodeURIComponent(did)}`)) });
    return n;
  };
  const fileOf = v => v.files?.find(f => f.type === studio.MANIFEST || /^(video|audio)\//.test(f.type ?? "")) ?? v.files?.[0] ?? null;

  let spaceName = null;
  function top(w) {
    const tabs = discovering()
      ? [{ label: "Discover", href: base(), on: !w.watch }]
      : sp()
        ? [{ label: C.name, href: base(), on: !w.watch && !w.up }]
        : [
            { label: "Following", href: `#/${C.app}`, on: !!w.feed },
            { label: C.mine, href: `#/${C.app}/mine`, on: w.by === me },
            { label: "Saved", href: `#/${C.app}/saved`, on: !!w.saved },
            { label: "Discover", href: `#/discover/${C.app}`, on: false },
          ];
    // In a space, it is a CHANNEL (the space named in this app's own words).
    const title = sp() ? `${C.icon} ${spaceName ?? "Channel"} · channel` : `${C.icon} ${C.name}`;
    // The SUB-TYPES (a filter): all, or one kind of the domain.
    const chips = w.watch || w.up ? null : h("div", { className: "chips" }, ...[null, ...VIDEO].map(k => h("button", { type: "button", className: only === k ? "on" : "", textContent: k ? kinds.of(k).label : "All", onclick: () => ((only = k), draw()) })));
    const feedBar = w.watch || w.up || w.saved ? null : h("div", { className: "chips" }, bar.el());
    return h("div", { className: "top" }, h("h2", { textContent: title }), h("nav", { className: "tabs" }, ...tabs.map(t => h("a", { href: t.href, textContent: t.label, className: t.on ? "on" : "" }))), discovering() ? null : h("a", { className: "up", href: `${base()}/up`, textContent: "⬆ Upload" }), chips, feedBar);
  }

  function card(v) {
    const f = fileOf(v);
    return h(
      "a",
      { className: "card", href: `${base()}/w/${encodeURIComponent(v.ref)}` },
      h("div", { className: `thumb${C.audio ? " sq" : ""}` }, f?.preview ? h("img", { src: f.preview, alt: "" }) : C.audio ? "🎵" : "🎬", f?.duration ? h("span", { className: "dur", textContent: clock(f.duration) }) : null, v.kind !== VIDEO[0] ? h("span", { className: "kind", textContent: kinds.of(v.kind)?.label ?? v.kind }) : null),
      C.audio && (v.meta?.artist || v.meta?.show || v.meta?.author) ? h("div", { className: "s", textContent: v.meta.artist ?? v.meta.show ?? v.meta.author }) : null,
      h("div", { className: "t", textContent: v.title }),
      h("div", { className: "s" }, who(v.by), ` · ${ago(v.at)}${v.private ? " · only you" : ""}`),
    );
  }

  async function list(w) {
    // SAVED: the videos this person pinned (the pin edge), newest saved first.
    const vs = w.saved
      ? (await Promise.all(pins.refs("videos:").map(async k => {
          const ref = k.slice(7);
          return items.get(ref, { outside: await outsideOf(ref) }).catch(() => null);
        }))).filter(Boolean)
      : bar.reorder(await items.list(w, bar.sort(), VIDEO, bar.options()));
    const shown = only ? vs.filter(v => v.kind === only) : vs;
    const when = bar.span();
    const none = w.saved ? `Nothing saved: “Save” on a ${C.one} keeps it here.` : w.feed ? `No ${C.ones} ${when} from you or what you follow.` : w.discover ? `No public ${C.ones} ${when}.` : w.by === me ? `Nothing ${when}: upload a ${C.one}.` : `No ${C.ones} ${when}.`;
    const older = w.saved ? null : bar.older("Older", shown.length);
    return shown.length ? h("div", {}, h("div", { className: `grid${C.audio ? " sq" : ""}` }, ...shown.map(card)), older) : h("div", {}, h("p", { className: "none", textContent: none }), older);
  }

  async function watch(ref) {
    const outside = await outsideOf(ref);
    const v = await items.get(ref, { outside });
    if (!v) return h("p", { className: "none", textContent: `This ${C.one} is not here (removed, or not shared with you).` });
    const f = fileOf(v);
    const level = h("span", { className: "level" });
    // THE PLAYER (`media-view`: the one way a video or an audio shows — its tracks by its video id, the same as
    // wherever else it shows); audio: its cover above it, and (a podcast, an audiobook) a speed.
    // An AUDIO starts as its album cover with ▶ (nothing read before it is played, as a video's pieces are only read
    // as it plays); its lyrics or transcript line shows at once.
    const view = mediaView.create({ file: f, item: ref, kind: C.audio ? "audio" : "video", outside, itemKind: v.kind, onLevel: l => (level.textContent = l), cover: C.audio, place: C.audio });
    const video = view.media;
    const note = view.note;
    const timed = view.timed;
    const coverBox = C.audio ? view.el : null;
    const speed = C.audio && ["podcast", "audiobook"].includes(v.kind) ? h("select", { title: "Speed", onchange: e => (video.playbackRate = Number(e.target.value)) }, ...[0.75, 1, 1.25, 1.5, 2].map(x => h("option", { value: x, textContent: `${x}×`, selected: x === 1 }))) : null;
    const like = h("button", { type: "button", className: v.mine === 1 ? "on" : "", disabled: !!outside, title: outside ? "Join to like" : "", textContent: `▲ ${v.score ?? 0}`, onclick: async () => ((like.disabled = true), await items.vote(ref, v.mine === 1 ? 0 : 1).catch(() => {}), draw()) });
    const saved = () => pins.has(SAVED(ref));
    const save = h("button", { type: "button", className: saved() ? "on" : "", textContent: saved() ? "Saved ✓" : "Save", onclick: async () => (await pins.set(SAVED(ref), !saved()), (save.className = saved() ? "on" : ""), (save.textContent = saved() ? "Saved ✓" : "Save")) });
    const k = kinds.of(v.kind);
    const fields = (k?.fields ?? []).filter(x => v.meta?.[x]).map(x => h("span", { textContent: `${kinds.fieldLabel(x)}: ${v.meta[x]}` }));
    // Its COMMENTS: the one thread (`comments`), as under a Board post.
    const thread = await (await ctx.require("comments")).create({ item: v, outside, app: C.app });
    const out = h(
      "div",
      { className: "watch" },
      coverBox,
      C.audio ? null : video,
      note,
      h("h1", { textContent: v.title }),
      h("div", { className: "row" }, h("span", { className: "s" }, who(v.by), ` · ${ago(v.at)}${k && v.kind !== VIDEO[0] ? ` · ${k.label}` : ""}${v.private ? " · only you" : ""}`), like, save, speed),
      fields.length ? h("div", { className: "meta" }, ...fields) : null,
      v.body ? h("div", { className: "about", textContent: v.body }) : null,
      view.line,
      timed,
      h("h3", { textContent: `Comments` }),
      thread.el,
    );
    // THE SCRUB BAR: the video's strip of frames, shown where the pointer is; a click seeks.
    const scrub = h("div", { className: "scrub" });
    const doneBar = h("div", { className: "done" });
    const tip = h("div", { className: "tip" }, h("span"));
    scrub.append(doneBar, tip);
    video.addEventListener("timeupdate", () => video.duration && (doneBar.style.width = `${(100 * video.currentTime) / video.duration}%`));
    const tAt = e => Math.max(0, Math.min(1, (e.clientX - scrub.getBoundingClientRect().left) / scrub.clientWidth)) * (video.duration || f?.duration || 0);
    scrub.onclick = e => (video.currentTime = tAt(e));
    const scrubStrip = async () => {
      if (f?.type !== studio.MANIFEST) return;
      const m = await player.manifest(f);
      if (!m.strip) return;
      const url = URL.createObjectURL(await files.get(m.strip.ref));
      const st = m.strip;
      Object.assign(tip.style, { width: `${st.w}px`, height: `${st.h}px`, backgroundImage: `url(${url})` });
      scrub.onmousemove = e => {
        const t = tAt(e);
        const i = Math.min(st.n - 1, Math.floor(t / st.every));
        tip.style.display = "block";
        tip.style.left = `${e.clientX - scrub.getBoundingClientRect().left}px`;
        tip.style.backgroundPosition = `-${(i % st.cols) * st.w}px -${Math.floor(i / st.cols) * st.h}px`;
        tip.firstChild.textContent = clock(t);
      };
      scrub.onmouseleave = () => (tip.style.display = "none");
    };
    // STILL BEING MADE (the uploader's devices, in the background): what is done, and what now.
    const making = h("span", { className: "level" });
    let quiet = 0; // checks in a row with nothing being made (the work may not have started yet)
    const showMaking = async () => {
      if (!el.isConnected || !out.isConnected || v.by !== me) return;
      const pr = await studio.progress(ref);
      making.textContent = pr ? ` · processing ${pr.done} of ${pr.of}: ${pr.stage}${pr.p ? ` ${Math.round(pr.p * 100)}%` : ""}` : "";
      quiet = pr ? 0 : quiet + 1;
      if (quiet < 6) setTimeout(showMaking, 3000);
    };
    out.insertBefore(h("div", {}, C.audio ? null : scrub, level, making), note);
    setTimeout(showMaking, 500); // once the view is in the page
    queueMicrotask(() => {
      // The author's subtitles from before (in the manifest) moved over once, then it plays with its tracks.
      if (f)
        (async () => {
          if (v.by === me && f.type === studio.MANIFEST)
            for (const old of await studio.takeLegacySubtitles(ref).catch(() => [])) {
              const text = await (await files.get(old.ref)).text();
              await subs.add(ref, text, { lang: old.lang, label: old.label }).catch(() => {});
            }
          if (C.audio) await view.prepare();
          else await view.play();
        })().catch(e => (note.textContent = e.message ?? String(e)));
      scrubStrip().catch(() => {});
    });
    return out;
  }

  // A cover image made small (320 px wide, JPEG): what rides on the item as its preview.
  async function coverOf(file) {
    const bmp = await createImageBitmap(file);
    const c = Object.assign(document.createElement("canvas"), { width: 320, height: Math.round((320 * bmp.height) / bmp.width) });
    c.getContext("2d").drawImage(bmp, 0, 0, c.width, c.height);
    return c.toDataURL("image/jpeg", 0.75);
  }

  async function upload() {
    const said = h("p", { className: "said", hidden: true });
    const progress = h("span", { className: "s" });
    const kindSel = h("select", { name: "kind" }, ...VIDEO.map(k => h("option", { value: k, textContent: kinds.of(k).label })));
    const fieldsBox = h("div", { style: "display:grid;gap:6px" });
    const drawFields = () => fieldsBox.replaceChildren(...kinds.of(kindSel.value).fields.map(x => h("input", { name: `meta.${x}`, placeholder: kinds.fieldLabel(x) })));
    kindSel.onchange = drawFields;
    drawFields();
    const inSpace = sp() ? (await space.mine()).find(s => s.id === sp()) : null;
    const f = h(
      "form",
      {},
      h("input", { type: "file", name: "file", accept: C.accept, required: true, onchange: e => prefill(e.target.files[0]) }),
      C.audio ? h("label", { className: "s" }, "Cover (an image, optional: else the file's own) ", h("input", { type: "file", name: "cover", accept: "image/*" })) : null,
      h("label", { className: "s" }, `${C.audio ? "Lyrics or transcript" : "Subtitles"} (.vtt or .srt, optional) `, h("input", { type: "file", name: "subs", accept: ".vtt,.srt,text/vtt", multiple: true })),
      h("label", { className: "s" }, h("input", { type: "checkbox", name: "keep" }), " Keep the original file too (as large as all the versions together; lets a newer format be made later)"),
      h("input", { name: "title", placeholder: "Title", required: true, maxLength: 300 }),
      h("textarea", { name: "body", rows: 4, placeholder: "Description" }),
      h("label", { className: "s" }, "What it is ", kindSel),
      fieldsBox,
      inSpace ? h("p", { className: "s", textContent: `For ${space.shown(inSpace)}: its members (and everyone, while it reads its ${C.ones} in public).` }) : h("select", { name: "audience" }, h("option", { value: "public", textContent: "🌐 Everyone (on your channel)" }), h("option", { value: "private", textContent: "🔒 Only you" })),
      h("div", { className: "row" }, h("button", { className: "on", textContent: "Upload" }), progress),
      said,
    );
    // The file's own TAGS fill the form (title; artist, album, year, genre where its kind has them).
    let tags = null;
    const prefill = async file => {
      if (!file) return;
      tags = await studio.probe(file).catch(() => null);
      if (!tags) return;
      if (tags.title && !f.elements.title.value) f.elements.title.value = tags.title;
      for (const [x, val] of Object.entries({ artist: tags.artist, album: tags.album, year: tags.year, genre: tags.genre })) {
        const input = f.elements[`meta.${x}`];
        if (val && input && !input.value) input.value = val;
      }
    };
    f.onsubmit = async e => {
      e.preventDefault();
      said.hidden = true;
      const file = f.elements.file.files[0];
      if (!file) return;
      const btn = f.querySelector("button");
      btn.disabled = true;
      try {
        progress.textContent = `Reading the ${C.one}…`;
        const pub = inSpace ? await items.publicIn(inSpace, kindSel.value) : f.elements.audience.value !== "private";
        // MADE READY TO STREAM (renditions, strip, subtitles, a manifest); a browser that cannot encode sends the file as it is.
        const ref = await studio
          .make(file, { space: inSpace, public: pub, keepOriginal: f.elements.keep.checked, onProgress: p => (progress.textContent = `${p.stage[0].toUpperCase()}${p.stage.slice(1)}${p.p ? ` ${Math.round(100 * p.p)}%` : "…"}`) })
          .catch(async err => {
            ctx.log(C.app, { what: `not encoded here (${err.message ?? err}): the file as it is` });
            const m = C.audio ? {} : await player.meta(file);
            const up = await files.put(file, { space: inSpace, public: pub, app: C.app, onProgress: p => (progress.textContent = `Uploading ${Math.round((100 * p.done) / Math.max(1, p.size))}%`) });
            return { ...up, ...(m.poster ? { preview: m.poster } : {}), ...(m.duration ? { duration: m.duration } : {}) };
          });
        // Drive lists what was uploaded: the original when it is kept, else the video (its manifest) or the file itself.
        const kept = ref.type === studio.MANIFEST && f.elements.keep.checked ? (await player.manifest(ref).catch(() => null))?.source : ref;
        if (kept) await drive.add(kept, { space: inSpace, from: { app: C.app } }).catch(() => {});
        // A COVER chosen here: over the file's own.
        const coverFile = f.elements.cover?.files?.[0];
        if (coverFile) ref.preview = await coverOf(coverFile).catch(() => ref.preview);
        const meta = Object.fromEntries(kinds.of(kindSel.value).fields.map(x => [x, String(f.elements[`meta.${x}`]?.value ?? "").trim()]).filter(([, v]) => v));
        // A file sent as it is (not encoded here) carries its video id on the item (a manifest carries its own).
        if (ref.type !== studio.MANIFEST && ref.key) meta.vid = await studio.videoId(ref.key);
        const posted = await items.submit({ board: inSpace?.id ?? null, title: f.elements.title.value, body: f.elements.body.value, kind: kindSel.value, meta, private: !inSpace && !pub, files: [ref] });
        // Its SUBTITLES: items of their own, about it.
        for (const s of f.elements.subs.files ?? []) await subs.add(posted, s).catch(e => ctx.log(C.app, { what: `timed text ${s.name}: ${e.message}` }));
        // LYRICS the file carries (untimed): one cue over the whole, when none was given.
        if (C.audio && tags?.lyrics && !(f.elements.subs.files ?? []).length)
          await subs.add(posted, `WEBVTT\n\n00:00:00.000 --> ${new Date(Math.max(1, ref.duration || 3600) * 1000).toISOString().slice(11, 23)}\n${tags.lyrics.trim()}\n`, { label: kinds.attachLabel("subtitle", kindSel.value) }).catch(() => {});
        location.hash = `${base()}/w/${encodeURIComponent(posted)}`;
      } catch (err) {
        said.textContent = err.message ?? String(err);
        said.hidden = false;
        btn.disabled = false;
        progress.textContent = "";
      }
    };
    return h("div", {}, h("h3", { textContent: `Upload a ${C.one}` }), f);
  }

  let drawn = "";
  async function draw() {
    const w = route();
    spaceName = sp() ? space.shown((await space.mine()).find(x => x.id === sp()) ?? { id: sp(), name: "" }) : null;
    drawn = `${ctx.space ?? ""}|${ctx.sub ?? ""}`;
    root.replaceChildren(top(w), theme.loading(w.watch ? `Opening the ${C.one}…` : `Reading the ${C.ones}…`));
    const body = await (w.watch ? watch(w.watch) : w.up ? upload() : list(w)).catch(e => h("p", { className: "said", textContent: e.message ?? String(e) }));
    if (drawn === `${ctx.space ?? ""}|${ctx.sub ?? ""}`) root.replaceChildren(top(w), ...(w.by && w.by !== me && !w.watch ? [h("div", { className: "row" }, h("h3", {}, who(w.by)), h("button", { type: "button", textContent: "Follow…", onclick: e => person.open(e.currentTarget, w.by) }))] : []), body);
  }
  await draw();
  items.onChange(() => el.isConnected && !route().watch && !route().up && draw());
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === C_ROUTE && `${ctx.space ?? ""}|${ctx.sub ?? ""}` !== drawn && draw());
}
