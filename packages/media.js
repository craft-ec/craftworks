// MEDIA, a page: ONE page for every MEDIA app — a LENS on a content DOMAIN (`kinds`) chosen by its route: VIDEOS
// (`#/video`: video, movie, TV episode, music video, short) and AUDIO (`#/audio`: music, podcast, audiobook), YouTube-
// and Spotify-shaped; a composed app shows several the same way. Its SUB-TYPES filter the list. CHANNEL-FIRST: yours
// (each public, or only you) and those you follow (`…`: the feed; `…/mine`; `…/c/<did>`: someone's); SAVED (the pin
// edge); DISCOVER. In a SPACE (`#/s/<space>/…`): its members' items, public while its domain reads in public. WATCH/
// LISTEN: `…/w/<ref>` — streamed by byte range (`video-player`), a like (▲), comments, and its timed text
// (`caption-store`: Subtitles on a video, Lyrics on a song, a Transcript on a podcast — shown in time for audio).
// UPLOAD: `…/up` (tags and cover from the file). UI only.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [items, directory, person, theme, space, roles, drive, player, kinds, edge] = await Promise.all(["items", "directory", "person", "theme", "space", "roles", "drive-store", "video-player", "kinds", "edge"].map(n => ctx.require(n)));
  const [studio, files, subs, mediaView] = await Promise.all(["video-studio", "files", "caption-store", "media-view"].map(n => ctx.require(n)));
  const [pins, people] = await Promise.all([edge.pins(), edge.people()]);
  // WHICH APP this page is (its route): its domain, its words.
  const APPS = {
    "/video": { app: "video", domain: "video", icon: "▶️", name: "Video", one: "video", ones: "videos", accept: "video/*", mine: "Your channel", audio: false },
    "/audio": { app: "audio", domain: "audio", icon: "🎧", name: "Audio", one: "track", ones: "tracks", accept: "audio/*", mine: "Your library", audio: true },
  };
  const C_ROUTE = APPS[ctx.route] ? ctx.route : "/video";
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
      .vd .up { background: var(--cw-accent); color: var(--cw-accent-fg); border-radius: var(--cw-radius-sm); padding: 6px 12px; text-decoration: none; }
      .vd .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: var(--cw-space-4) var(--cw-space-3); }
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
  // WHERE it is (`where`: the one reading of an address — yours, a space's, a person's, Discover), read at each draw.
  const where = await ctx.require("where");
  let at = await where.of({ kinds: VIDEO, app: C.app, yours: "Following" });
  const base = () => at.base;
  const route = () => {
    const s = at.sub;
    if (s.startsWith("w/")) return { watch: decodeURIComponent(s.slice(2)) };
    if (s === "up") return { up: true };
    // `c/<did>` from before: that person's space.
    if (s.startsWith("c/")) return { by: decodeURIComponent(s.slice(2)) };
    if (at.who === "person") return { by: at.person };
    if (at.who === "discover") return { discover: true };
    if (at.who === "space") return { board: at.space.id };
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
  // One wording of time everywhere (`cards`).
  const { ago, clock } = await ctx.require("cards");
  const who = did => {
    const n = directory.nameEl(did, "span", { className: "by", onclick: e => (e.preventDefault(), e.stopPropagation(), (location.hash = `#/${C.app}/u/${did}`)) });
    return n;
  };
  const fileOf = v => v.files?.find(f => f.type === studio.MANIFEST || /^(video|audio)\//.test(f.type ?? "")) ?? v.files?.[0] ?? null;

  function top(w) {
    // THE TABS (`where`'s, in every app's order): Following · yours · Saved · Discover; Upload where you may.
    at.tabs([
      ...(at.who === "mine" || at.who === "discover" ? [{ label: C.mine, href: `#/${C.app}/mine`, on: w.by === me }, { label: "Saved", href: `#/${C.app}/saved`, on: !!w.saved }] : []),
      ...(at.others ? [] : [{ label: "⬆ Upload", href: `${base()}/up`, on: !!w.up }]),
    ], { yoursOn: !!w.feed });
    // In a space, it is a CHANNEL (the space named in this app's own words).
    const title = at.space ? `${C.icon} ${space.shown(at.space)} · channel` : `${C.icon} ${C.name}`;
    // The SUB-TYPES (a filter): all, or one kind of the domain.
    const chips = w.watch || w.up ? null : h("div", { className: "chips" }, ...[null, ...VIDEO].map(k => h("button", { type: "button", className: only === k ? "on" : "", textContent: k ? kinds.of(k).label : "All", onclick: () => ((only = k), draw()) })));
    const feedBar = w.watch || w.up || w.saved ? null : h("div", { className: "chips" }, bar.el());
    return h("div", { className: "top" }, h("h2", { textContent: title }), chips, feedBar);
  }

  // A video's or a track's CARD: its kind's look (`cards`), opened here (in Discover: from outside).
  const cards = await ctx.require("cards");
  const card = v => cards.card(v, { href: `${base()}/w/${encodeURIComponent(v.ref)}` });

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
    // A like: where the one check says this person may (from outside: as the space's public policy says).
    const mayLike = !outside || (await items.mayWriteOn(v, "vote", { outside }).catch(() => false));
    const like = h("button", { type: "button", className: v.mine === 1 ? "on" : "", disabled: !mayLike, title: mayLike ? "" : "Join to like", textContent: `▲ ${v.score ?? 0}`, onclick: async () => ((like.disabled = true), await items.vote(ref, v.mine === 1 ? 0 : 1, ref, { outside }).catch(() => {}), draw()) });
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


  // UPLOAD: the one upload form (`publisher.form`: the same as over an editor) — the item opened once published.
  async function upload() {
    const f = await (await ctx.require("publisher")).form({ domain: C.audio ? "audio" : "video", space: at.space, app: C.app, onPublished: ({ item }) => (location.hash = `${base()}/w/${encodeURIComponent(item)}`) });
    return h("div", {}, h("h3", { textContent: `Upload a ${C.one}` }), f);
  }

  let drawn = "";
  async function draw() {
    at = await where.of({ kinds: VIDEO, app: C.app, yours: "Following" });
    const w = route();
    drawn = where.key();
    root.replaceChildren(top(w), theme.loading(w.watch ? `Opening the ${C.one}…` : `Reading the ${C.ones}…`));
    const body = await (w.watch ? watch(w.watch) : w.up ? upload() : list(w)).catch(e => h("p", { className: "said", textContent: e.message ?? String(e) }));
    if (drawn === where.key()) root.replaceChildren(top(w), ...(w.by && w.by !== me && !w.watch ? [h("div", { className: "row" }, h("h3", {}, who(w.by)), h("button", { type: "button", textContent: "Follow…", onclick: e => person.open(e.currentTarget, w.by) }))] : []), body);
  }
  await draw();
  items.onChange(() => el.isConnected && !route().watch && !route().up && draw());
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === C_ROUTE && where.key() !== drawn && draw());
}
