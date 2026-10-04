// MEDIA LOOK, a component: a video's or an audio's look WHOLE — on its own page (`item-page`), the same wherever it
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
    .cw-media .cover { width: min(320px, 70vw); aspect-ratio: 1; border-radius: var(--cw-radius); overflow: hidden; background: var(--cw-hover); display: grid; place-items: center; font-size: 4rem; }
    .cw-media .cover img { width: 100%; height: 100%; object-fit: cover; }
    .cw-media .timed { max-height: 320px; overflow: auto; border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); }
    .cw-media .timed p { margin: 4px 0; cursor: pointer; color: var(--cw-muted); }
    .cw-media .timed p.on { color: var(--cw-fg); font-weight: 600; }
    .cw-media select { font: inherit; padding: 4px 8px; border-radius: var(--cw-radius-sm); border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const fileOf = v => v.files?.find(f => f.type === studio.MANIFEST || /^(video|audio)\//.test(f.type ?? "")) ?? v.files?.[0] ?? null;

  function full(v, { outside = null } = {}) {
    const audio = kinds.domain(v.kind) === "audio";
    const f = fileOf(v);
    const level = h("span", { className: "level" });
    const view = mediaView.create({ file: f, item: v.ref, kind: audio ? "audio" : "video", outside, itemKind: v.kind, onLevel: l => (level.textContent = l), cover: audio, place: audio });
    const video = view.media;
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
    const el = h("div", { className: "cw-media" }, audio ? view.el : video, h("div", {}, audio ? null : scrub, level, making), view.note, speed, view.line, view.timed);
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
      if (f) (audio ? view.prepare() : view.play()).catch(e => (view.note.textContent = e.message ?? String(e)));
      scrubStrip().catch(() => {});
    };
    return { el, start: begin, media: video };
  }

  return { full, fileOf };
}
