// PLAYER CHROME, a component: a video's controls ON the picture — play/pause, the TIMELINE (what is buffered, what is
// played, a frame from the scrub strip where the pointer or finger is; a click or a drag seeks), the time, QUALITY
// (Auto or one rendition), subtitles, sound and full screen. Shown while paused; while playing, hidden 3 s after the
// last touch, and back on any touch, tap or key (nothing is hover-only).
//
//   const chrome = await ctx.require("player-chrome");
//   const c = chrome.attach(stage, video)      // stage: the element around the <video> (it goes full screen)
//   c.strip({ url, w, h, n, cols, every })     // the frames sprite, once read
//   c.levels(labels, pick)                     // the renditions (`video-player`'s onLevels); pick(i | null)
//   c.level(label)                             // the one playing (`video-player`'s onLevel)
export async function start() {
  const style = document.createElement("style");
  style.textContent = `
    .cw-pc { position: relative; max-width: calc(70vh * 16 / 9); margin-inline: auto; background: #000; border-radius: var(--cw-radius, 8px); overflow: hidden; user-select: none; }
    .cw-pc video { display: block; width: 100%; max-height: 70vh; background: #000; }
    .cw-pc:fullscreen { max-width: none; }
    .cw-pc:fullscreen video { max-height: 100vh; height: 100%; }
    .cw-pc .cw-pc-bar { position: absolute; left: 0; right: 0; bottom: 0; padding: 18px 10px 6px; color: #fff; font: 13px/1.2 system-ui, sans-serif;
      background: linear-gradient(transparent, rgba(0, 0, 0, 0.75)); transition: opacity 0.25s; }
    .cw-pc.idle .cw-pc-bar { opacity: 0; pointer-events: none; }
    .cw-pc.idle { cursor: none; }
    .cw-pc .cw-pc-line { position: relative; height: 16px; cursor: pointer; touch-action: none; }
    .cw-pc .cw-pc-line > i { position: absolute; left: 0; top: 6px; height: 4px; border-radius: 2px; pointer-events: none; }
    .cw-pc .cw-pc-line .all { right: 0; background: rgba(255, 255, 255, 0.25); }
    .cw-pc .cw-pc-line .got { background: rgba(255, 255, 255, 0.45); }
    .cw-pc .cw-pc-line .seen { background: #f03; }
    .cw-pc .cw-pc-line .knob { width: 12px; height: 12px; top: 2px; margin-left: -6px; border-radius: 50%; background: #f03; }
    .cw-pc .cw-pc-tip { position: absolute; bottom: 22px; transform: translateX(-50%); border: 2px solid #fff; border-radius: 6px; background: #000 no-repeat;
      box-shadow: 0 4px 16px rgba(0, 0, 0, 0.5); pointer-events: none; }
    .cw-pc .cw-pc-tip[hidden] { display: none; }
    .cw-pc .cw-pc-tip span { position: absolute; left: 0; right: 0; bottom: -20px; text-align: center; text-shadow: 0 0 3px #000; }
    .cw-pc .cw-pc-row { display: flex; align-items: center; gap: 6px; }
    .cw-pc .cw-pc-row button, .cw-pc .cw-pc-row select { font: inherit; color: #fff; background: none; border: 0; padding: 4px 6px; border-radius: 4px; cursor: pointer; }
    .cw-pc .cw-pc-row select { background: rgba(0, 0, 0, 0.4); }
    .cw-pc .cw-pc-row select option { color: #000; }
    .cw-pc .cw-pc-row button:focus-visible, .cw-pc .cw-pc-row select:focus-visible { outline: 2px solid #fff; }
    .cw-pc .cw-pc-row button[aria-pressed="true"] { background: rgba(255, 255, 255, 0.25); }
    .cw-pc .cw-pc-row [hidden] { display: none; }
    .cw-pc .cw-pc-row .time { font-variant-numeric: tabular-nums; }
    .cw-pc .cw-pc-row .fill { flex: 1; }
    .cw-pc .cw-pc-row .lvl { opacity: 0.8; font-size: 12px; }
    .cw-pc .cw-pc-row button { display: inline-grid; place-items: center; width: 34px; height: 30px; padding: 0; }
    .cw-pc .cw-pc-row button svg { width: 22px; height: 22px; fill: currentColor; }
    .cw-pc .cw-pc-row button.cc { width: auto; padding: 0 6px; font-weight: 700; font-size: 12px; letter-spacing: 0.5px; }
    .cw-pc .cw-pc-row .sound { display: inline-flex; align-items: center; }
    .cw-pc .cw-pc-row input[type=range] { -webkit-appearance: none; appearance: none; width: 64px; height: 4px; margin: 0 6px 0 2px; border-radius: 2px; cursor: pointer;
      background: linear-gradient(#fff, #fff) 0 / var(--v, 100%) 100% no-repeat, rgba(255, 255, 255, 0.3); }
    .cw-pc .cw-pc-row input[type=range]::-webkit-slider-thumb { -webkit-appearance: none; width: 12px; height: 12px; border-radius: 50%; background: #fff; border: 0; }
    .cw-pc .cw-pc-row input[type=range]::-moz-range-thumb { width: 12px; height: 12px; border-radius: 50%; background: #fff; border: 0; }`;
  // ONE ICON SET (drawn, so every control looks alike — an emoji's look is the system's).
  const ICON = {
    play: '<svg viewBox="0 0 24 24"><path d="M8 5v14l11-7z"/></svg>',
    pause: '<svg viewBox="0 0 24 24"><path d="M6 5h4v14H6zM14 5h4v14h-4z"/></svg>',
    sound: '<svg viewBox="0 0 24 24"><path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8v8a4.5 4.5 0 0 0 2.5-4zM14 3.2v2.1a7 7 0 0 1 0 13.4v2.1a9 9 0 0 0 0-17.6z"/></svg>',
    muted: '<svg viewBox="0 0 24 24"><path d="M3 9v6h4l5 5V4L7 9H3zm13.6 3 2.7-2.7-1.4-1.4-2.7 2.7-2.7-2.7-1.4 1.4 2.7 2.7-2.7 2.7 1.4 1.4 2.7-2.7 2.7 2.7 1.4-1.4z"/></svg>',
    full: '<svg viewBox="0 0 24 24"><path d="M5 5h5v2H7v3H5zm9 0h5v5h-2V7h-3zM5 14h2v3h3v2H5zm12 0h2v5h-5v-2h3z"/></svg>',
  };
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const clock = s => {
    s = Math.max(0, Math.floor(s || 0));
    const hh = Math.floor(s / 3600);
    const mm = Math.floor((s % 3600) / 60);
    const ss = String(s % 60).padStart(2, "0");
    return hh ? `${hh}:${String(mm).padStart(2, "0")}:${ss}` : `${mm}:${ss}`;
  };

  function attach(stage, video, { duration = 0 } = {}) {
    stage.classList.add("cw-pc");
    video.controls = false;
    const all = h("i", { className: "all" });
    const got = h("i", { className: "got" });
    const seen = h("i", { className: "seen" });
    const knob = h("i", { className: "knob" });
    const tip = h("div", { className: "cw-pc-tip", hidden: true }, h("span"));
    const line = h("div", { className: "cw-pc-line", title: "Seek" }, all, got, seen, knob, tip);
    const play = h("button", { type: "button", title: "Play (k)", innerHTML: ICON.play });
    const time = h("span", { className: "time", textContent: "0:00 / 0:00" });
    const mute = h("button", { type: "button", title: "Mute (m)", innerHTML: ICON.sound });
    const vol = h("input", { type: "range", min: 0, max: 1, step: 0.05, value: video.volume, title: "Volume" });
    const lvl = h("span", { className: "lvl" });
    const cc = h("button", { type: "button", className: "cc", title: "Subtitles (c)", textContent: "CC", hidden: true });
    const quality = h("select", { title: "Quality", hidden: true });
    const full = h("button", { type: "button", title: "Full screen (f)", innerHTML: ICON.full });
    const bar = h("div", { className: "cw-pc-bar" }, line, h("div", { className: "cw-pc-row" }, play, h("span", { className: "sound" }, mute, vol), time, h("span", { className: "fill" }), lvl, cc, quality, full));
    stage.append(bar);

    const len = () => (Number.isFinite(video.duration) && video.duration) || duration || 0;
    const draw = () => {
      const d = len();
      const p = d ? video.currentTime / d : 0;
      seen.style.width = `${100 * p}%`;
      knob.style.left = `${100 * p}%`;
      const b = video.buffered;
      let end = 0;
      for (let i = 0; i < b.length; i++) if (b.start(i) <= video.currentTime + 0.5) end = Math.max(end, b.end(i));
      got.style.width = d ? `${(100 * end) / d}%` : "0";
      time.textContent = `${clock(video.currentTime)} / ${clock(d)}`;
      const icon = (b, k) => b.dataset.icon !== k && ((b.dataset.icon = k), (b.innerHTML = ICON[k]));
      icon(play, video.paused ? "play" : "pause");
      icon(mute, video.muted || video.volume === 0 ? "muted" : "sound");
      vol.style.setProperty("--v", `${100 * (video.muted ? 0 : video.volume)}%`);
    };
    for (const ev of ["timeupdate", "progress", "play", "pause", "durationchange", "volumechange", "seeking"]) video.addEventListener(ev, draw);
    draw();

    // SHOWN while paused; while playing, hidden 3 s after the last touch.
    let idle = 0;
    const wake = () => {
      stage.classList.remove("idle");
      clearTimeout(idle);
      idle = setTimeout(() => !video.paused && !line.dragging && stage.classList.add("idle"), 3000);
    };
    for (const ev of ["pointermove", "pointerdown", "keydown", "focusin"]) stage.addEventListener(ev, wake);
    video.addEventListener("play", wake);
    video.addEventListener("pause", () => (clearTimeout(idle), stage.classList.remove("idle")));

    const toggle = () => (video.paused ? video.play().catch(() => {}) : video.pause());
    play.onclick = toggle;
    // A tap on the picture: when the controls are hidden it only shows them (a phone's first tap); else play/pause.
    video.addEventListener("click", () => (stage.classList.contains("idle") ? wake() : toggle()));
    mute.onclick = () => (video.muted = !video.muted);
    vol.oninput = () => ((video.volume = Number(vol.value)), (video.muted = video.volume === 0));
    full.onclick = () => (document.fullscreenElement ? document.exitFullscreen() : stage.requestFullscreen?.())?.catch?.(() => {});
    stage.tabIndex = 0;
    stage.addEventListener("keydown", e => {
      if (e.target.closest?.("select, input")) return;
      const k = e.key.toLowerCase();
      if (k === " " || k === "k") toggle();
      else if (k === "arrowright") video.currentTime = Math.min(len(), video.currentTime + 5);
      else if (k === "arrowleft") video.currentTime = Math.max(0, video.currentTime - 5);
      else if (k === "m") mute.onclick();
      else if (k === "f") full.onclick();
      else if (k === "c" && !cc.hidden) cc.onclick();
      else return;
      e.preventDefault();
    });

    // SUBTITLES: the element's tracks (added by whoever reads them), the first one on or off.
    const tracks = () => [...video.textTracks].filter(t => t.kind === "subtitles" || t.kind === "captions");
    const ccDraw = () => {
      const ts = tracks();
      cc.hidden = !ts.length;
      cc.setAttribute("aria-pressed", String(ts.some(t => t.mode === "showing")));
    };
    cc.onclick = () => {
      const ts = tracks();
      const on = ts.some(t => t.mode === "showing");
      ts.forEach((t, i) => (t.mode = !on && i === 0 ? "showing" : "disabled"));
      ccDraw();
    };
    video.textTracks.addEventListener?.("addtrack", ccDraw);
    video.textTracks.addEventListener?.("change", ccDraw);

    // THE TIMELINE: a frame and its time where the pointer is (a finger too); a click or a drag seeks.
    let st = null;
    const at = e => Math.max(0, Math.min(1, (e.clientX - line.getBoundingClientRect().left) / line.clientWidth));
    const show = e => {
      const p = at(e);
      const t = p * len();
      tip.hidden = false;
      tip.style.left = `${Math.min(Math.max(p * line.clientWidth, (st?.w ?? 40) / 2), line.clientWidth - (st?.w ?? 40) / 2)}px`;
      tip.firstChild.textContent = clock(t);
      if (st) {
        const i = Math.min(st.n - 1, Math.floor(t / st.every));
        tip.style.backgroundPosition = `-${(i % st.cols) * st.w}px -${Math.floor(i / st.cols) * st.h}px`;
      }
    };
    line.addEventListener("pointermove", show);
    line.addEventListener("pointerleave", () => !line.dragging && (tip.hidden = true));
    line.addEventListener("pointerdown", e => {
      line.dragging = true;
      line.setPointerCapture(e.pointerId);
      show(e);
    });
    line.addEventListener("pointerup", e => {
      if (!line.dragging) return;
      line.dragging = false;
      video.currentTime = at(e) * len();
      if (e.pointerType !== "mouse") tip.hidden = true;
      wake();
    });

    return {
      el: stage,
      strip(s) {
        st = s;
        Object.assign(tip.style, { width: `${s.w}px`, height: `${s.h}px`, backgroundImage: `url(${s.url})` });
      },
      levels(labels, pick) {
        quality.replaceChildren(h("option", { value: "", textContent: "Auto" }), ...labels.map((l, i) => h("option", { value: i, textContent: l })).reverse());
        quality.onchange = () => pick(quality.value === "" ? null : Number(quality.value));
        quality.hidden = labels.length < 2;
      },
      level(label) {
        lvl.textContent = label;
      },
    };
  }

  return { attach, clock };
}
