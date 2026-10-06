// MEDIA LOOK, a component: a video's, an audio's, an image's or a book's look WHOLE — on its own page (`item-page`), the same wherever it
// opens. The player (`media-view`: its tracks by its video id), a video's scrub strip of frames, an audio's cover
// (and a speed for a podcast or an audiobook), its lyrics or transcript line, its timed text; while its uploader's
// devices still make it, what is done.
//
//   const look = await ctx.require("media-look");
//   const m = look.full(item, { outside })   // item: from items.get
//   host.append(m.el)                         // then, once in the page:
//   m.start()                                 // a video plays; an audio shows its cover with ▶
export async function start(ctx) {
  const [player, kinds, studio, files, mediaView, cards, space, chrome] = await Promise.all(["video-player", "kinds", "video-studio", "files", "media-view", "cards", "space", "player-chrome"].map(n => ctx.require(n)));
  const style = document.createElement("style");
  style.textContent = `@layer components {

    .cw-media { display: grid; gap: var(--cw-space-3); }
    .cw-media video:not(.cw-pc > video) { width: 100%; max-height: 70vh; background: #000; border-radius: var(--cw-radius); }
    .cw-media audio { width: 100%; }
    .cw-media .level { font-size: var(--cw-text-xs); color: var(--cw-muted); }
    .cw-media .stage { position: relative; }
    .cw-media .stage .start { position: absolute; inset: 0; z-index: 2; display: grid; place-items: center; width: 100%; height: 100%; padding: 0; border: 0; border-radius: 0; background: transparent; cursor: pointer; }
    .cw-media .stage .start span { display: grid; place-items: center; width: 72px; height: 52px; border-radius: 14px; background: rgba(0, 0, 0, 0.65); color: #fff; font-size: 24px; }
    .cw-media .stage .start:focus-visible span { outline: 2px solid var(--cw-accent); }
    .cw-media .cover { width: min(320px, 70vw); aspect-ratio: 1; border-radius: var(--cw-radius); overflow: hidden; background: var(--cw-hover); display: grid; place-items: center; font-size: 4rem; }
    .cw-media .cover img { width: 100%; height: 100%; object-fit: cover; }
    .cw-media .timed { max-height: 320px; overflow: auto; border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); }
    .cw-media .timed p { margin: 4px 0; cursor: pointer; color: var(--cw-muted); }
    .cw-media .timed p.on { color: var(--cw-fg); font-weight: 600; }
    .cw-media.pic .frame { display: grid; place-items: center; background: #111; border-radius: var(--cw-radius); overflow: auto; max-height: 80vh; cursor: zoom-in; }
    .cw-media.pic .frame img { max-width: 100%; max-height: 80vh; object-fit: contain; }
    .cw-media.pic.actual .frame { cursor: zoom-out; place-items: start; }
    .cw-media.pic.actual .frame img { max-width: none; max-height: none; }
    .cw-media select { font: inherit; padding: 4px 8px; border-radius: var(--cw-radius-sm); border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); }
}`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const fileOf = v => v.files?.find(f => f.type === studio.MANIFEST || /^(video|audio)\//.test(f.type ?? "")) ?? v.files?.[0] ?? null;

  // AN IMAGE whole: its preview at once, the picture itself when read; a click shows it at its own size.
  function picture(v, { outside = null } = {}) {
    const f = v.files?.find(x => kinds.mediaOf(x)?.domain === "image") ?? v.files?.[0] ?? null;
    const img = h("img", { alt: v.title ?? "", src: f?.preview ?? "" });
    const note = h("span", { className: "level" });
    const el = h("div", { className: "cw-media pic" }, h("div", { className: "frame" }, img), note);
    el.querySelector(".frame").onclick = () => el.classList.toggle("actual");
    const start = async () => {
      if (!f) return;
      note.textContent = "Loading the picture…";
      try {
        const blob = await files.get(f, { onProgress: e => (note.textContent = `Loading ${Math.round((100 * e.done) / Math.max(1, e.size))}%`) });
        img.src = URL.createObjectURL(blob);
        note.textContent = "";
      } catch (e) {
        note.textContent = e.message ?? String(e);
      }
    };
    return { el, start, media: null };
  }

  // A BOOK whole: its reader (`book-view`), open at once.
  function reader(v) {
    const f = v.files?.find(x => kinds.mediaOf(x)?.domain === "book") ?? v.files?.[0] ?? null;
    const el = h("div", { className: "cw-media" });
    const start = async () => {
      if (!f) return;
      const bv = await ctx.require("book-view");
      el.replaceChildren(bv.create({ file: f, item: v.ref, cover: false, kind: v.kind }).el);
    };
    return { el, start, media: null };
  }

  function full(v, { outside = null } = {}) {
    if (kinds.domain(v.kind) === "image") return picture(v, { outside });
    if (kinds.domain(v.kind) === "book") return reader(v);
    const audio = kinds.domain(v.kind) === "audio";
    const f = fileOf(v);
    // THE CONTROLS ON THE PICTURE (`player-chrome`): timeline with the scrub strip's frames, quality, subtitles, sound,
    // full screen — attached when ▶ starts the player (before it: the poster and a ▶, nothing loaded).
    let c = null;
    let levels = null;
    let label = "";
    let strip = null;
    const onLevels = (labels, pick) => ((levels = [labels, pick]), c?.levels(labels, pick));
    const onLevel = l => ((label = l), c?.level(l));
    const view = mediaView.create({ file: f, item: v.ref, kind: audio ? "audio" : "video", outside, itemKind: v.kind, onLevel, onLevels, cover: audio, place: audio });
    const video = view.media;
    if (!audio && f?.preview) video.poster = f.preview;
    // The whole poster is the ▶ (a click anywhere on it starts the player).
    const start = h("button", { type: "button", className: "start", title: "Play", "aria-label": "Play" }, h("span", { textContent: "▶" }));
    const stage = chrome.frame(h("div", { className: "stage" }, video, start), { width: f?.width, height: f?.height });
    if (!audio) video.controls = false;
    const speed = audio && ["podcast", "audiobook"].includes(v.kind) ? h("select", { title: "Speed", onchange: e => (video.playbackRate = Number(e.target.value)) }, ...[0.75, 1, 1.25, 1.5, 2].map(x => h("option", { value: x, textContent: `${x}×`, selected: x === 1 }))) : null;
    const scrubStrip = async () => {
      if (f?.type !== studio.MANIFEST) return;
      const m = await player.manifest(f);
      if (!m.strip) return;
      strip = { ...m.strip, url: URL.createObjectURL(await files.get(m.strip.ref)) };
      c?.strip(strip);
    };
    // STILL BEING MADE (the uploader's devices, in the background): what is done, and what now.
    const making = h("span", { className: "level" });
    const el = h("div", { className: "cw-media" }, audio ? view.el : stage, h("div", {}, making), view.note, speed, view.line, view.timed);
    let quiet = 0; // checks in a row with nothing being made (the work may not have started yet)
    const showMaking = async () => {
      if (!el.isConnected || v.by !== (await space.account())?.id) return;
      const pr = await studio.progress(v.ref);
      making.textContent = pr ? ` · processing ${pr.done} of ${pr.of}: ${pr.stage}${pr.p ? ` ${Math.round(pr.p * 100)}%` : ""}` : "";
      quiet = pr ? 0 : quiet + 1;
      if (quiet < 6) setTimeout(showMaking, 3000);
    };
    const begin = () => {
      setTimeout(showMaking, 500);
      // ITS USE counted (`usage`): the time it plays, and the data its files bring — the manifest's, the frames', every
      // rendition's (whichever plays).
      ctx.require("usage").then(async u => {
        const info = { kind: v.kind, title: v.title ?? f?.name ?? null };
        u.watch(video, v.ref, info);
        if (!f) return;
        u.track(v.ref, info, [f.root]);
        if (f.type === studio.MANIFEST) {
          const m = await player.manifest(f).catch(() => null);
          if (m) u.track(v.ref, info, [m.strip?.ref?.root, ...m.renditions.map(r => r.ref?.root)]);
        }
      }, () => {});
      if (f && audio) view.prepare().catch(e => (view.note.textContent = e.message ?? String(e)));
      else if (f)
        start.onclick = () => {
          start.remove();
          c = chrome.attach(stage, video, { duration: f.duration ?? 0, width: f.width, height: f.height });
          if (strip) c.strip(strip);
          if (levels) c.levels(...levels);
          if (label) c.level(label);
          view.play({ autoplay: true }).catch(e => (view.note.textContent = e.message ?? String(e)));
        };
      scrubStrip().catch(() => {});
    };
    return { el, start: begin, media: video };
  }

  return { full, fileOf };
}
