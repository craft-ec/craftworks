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
  const [player, kinds, studio, files, mediaView, cards, space] = await Promise.all(["video-player", "kinds", "video-studio", "files", "media-view", "cards", "space"].map(n => ctx.require(n)));
  const style = document.createElement("style");
  style.textContent = `
    .cw-media { display: grid; gap: var(--cw-space-3); }
    .cw-media video { width: 100%; max-height: 70vh; background: #000; border-radius: var(--cw-radius); }
    .cw-media audio { width: 100%; }
    .cw-media .scrub { position: relative; height: 10px; background: var(--cw-hover); border-radius: 5px; cursor: pointer; }
    .cw-media .scrub .done { position: absolute; inset: 0 auto 0 0; background: var(--cw-accent); border-radius: 5px; pointer-events: none; }
    .cw-media .scrub .tip { position: absolute; bottom: 16px; transform: translateX(-50%); border: 2px solid #fff; border-radius: 6px; box-shadow: var(--cw-shadow-lg);
      background-repeat: no-repeat; pointer-events: none; display: none; }
    .cw-media .scrub .tip span { position: absolute; bottom: 2px; left: 0; right: 0; text-align: center; color: #fff; font-size: 11px; text-shadow: 0 0 3px #000; }
    .cw-media .level { font-size: var(--cw-text-xs); color: var(--cw-muted); }
    .cw-media .stage { position: relative; }
    .cw-media .stage .start { position: absolute; inset: 0; margin: auto; width: 72px; height: 52px; border: 0; border-radius: 14px; background: rgba(0, 0, 0, 0.65); color: #fff; font-size: 24px; cursor: pointer; }
    .cw-media .stage .start:focus-visible { outline: 2px solid var(--cw-accent); }
    .cw-media .quality { font-size: var(--cw-text-xs); margin-right: var(--cw-space-2); }
    .cw-media .cover { width: min(320px, 70vw); aspect-ratio: 1; border-radius: var(--cw-radius); overflow: hidden; background: var(--cw-hover); display: grid; place-items: center; font-size: 4rem; }
    .cw-media .cover img { width: 100%; height: 100%; object-fit: cover; }
    .cw-media .timed { max-height: 320px; overflow: auto; border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); }
    .cw-media .timed p { margin: 4px 0; cursor: pointer; color: var(--cw-muted); }
    .cw-media .timed p.on { color: var(--cw-fg); font-weight: 600; }
    .cw-media.pic .frame { display: grid; place-items: center; background: #111; border-radius: var(--cw-radius); overflow: auto; max-height: 80vh; cursor: zoom-in; }
    .cw-media.pic .frame img { max-width: 100%; max-height: 80vh; object-fit: contain; }
    .cw-media.pic.actual .frame { cursor: zoom-out; place-items: start; }
    .cw-media.pic.actual .frame img { max-width: none; max-height: none; }
    .cw-media select { font: inherit; padding: 4px 8px; border-radius: var(--cw-radius-sm); border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); }`;
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
    const level = h("span", { className: "level" });
    // QUALITY: Auto, or one rendition (the player's list, once it knows the ladder this browser plays).
    const quality = h("select", { className: "quality", title: "Quality", hidden: true });
    const onLevels = (labels, pick) => {
      quality.replaceChildren(h("option", { value: "", textContent: "Auto" }), ...labels.map((l, i) => h("option", { value: i, textContent: l })).reverse());
      quality.onchange = () => pick(quality.value === "" ? null : Number(quality.value));
      quality.hidden = labels.length < 2;
    };
    const view = mediaView.create({ file: f, item: v.ref, kind: audio ? "audio" : "video", outside, itemKind: v.kind, onLevel: l => (level.textContent = l), onLevels, cover: audio, place: audio });
    const video = view.media;
    // NOTHING LOADS until ▶: the poster shows with a ▶ over it (the element's own controls do nothing with no source);
    // its click starts the player, playing.
    if (!audio && f?.preview) video.poster = f.preview;
    const start = h("button", { type: "button", className: "start", title: "Play", textContent: "▶" });
    const stage = h("div", { className: "stage" }, video, start);
    if (!audio) video.controls = false;
    const speed = audio && ["podcast", "audiobook"].includes(v.kind) ? h("select", { title: "Speed", onchange: e => (video.playbackRate = Number(e.target.value)) }, ...[0.75, 1, 1.25, 1.5, 2].map(x => h("option", { value: x, textContent: `${x}×`, selected: x === 1 }))) : null;
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
        tip.firstChild.textContent = cards.clock(t);
      };
      scrub.onmouseleave = () => (tip.style.display = "none");
    };
    // STILL BEING MADE (the uploader's devices, in the background): what is done, and what now.
    const making = h("span", { className: "level" });
    const el = h("div", { className: "cw-media" }, audio ? view.el : stage, h("div", {}, audio ? null : scrub, quality, level, making), view.note, speed, view.line, view.timed);
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
      if (f && audio) view.prepare().catch(e => (view.note.textContent = e.message ?? String(e)));
      else if (f)
        start.onclick = () => {
          start.remove();
          video.controls = true;
          view.play({ autoplay: true }).catch(e => (view.note.textContent = e.message ?? String(e)));
        };
      scrubStrip().catch(() => {});
    };
    return { el, start: begin, media: video };
  }

  return { full, fileOf };
}
