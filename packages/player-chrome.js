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
  style.textContent = `@layer components {

    /* ONE SHAPE, the video's own (--cw-pc-w / --cw-pc-h), never taller than 70% of the screen: the box never resizes
       as renditions change; the picture fits inside it. */
    .cw-pc { position: relative; width: 100%; aspect-ratio: var(--cw-pc-w, 16) / var(--cw-pc-h, 9); max-height: 70vh;
      max-width: calc(70vh * var(--cw-pc-w, 16) / var(--cw-pc-h, 9)); margin-inline: auto; background: #000; border-radius: var(--cw-radius, 8px);
      overflow: hidden; user-select: none; }
    .cw-pc.cw-pc > video { position: absolute; inset: 0; display: block; width: 100%; height: 100%; max-height: none; object-fit: contain; background: #000; border-radius: 0; }
    .cw-pc:fullscreen { max-width: none; max-height: none; aspect-ratio: auto; width: 100%; height: 100%; border-radius: 0; }
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
    .cw-pc.cw-pc button { font: inherit; color: #fff; background: none; border: 0; box-shadow: none; padding: 4px 6px; border-radius: 4px; cursor: pointer; min-height: 0; }
    .cw-pc.cw-pc button:focus-visible { outline: 2px solid #fff; }
    .cw-pc .cw-pc-menu { position: absolute; right: 10px; bottom: 52px; z-index: 3; min-width: 220px; max-height: calc(100% - 64px); overflow: auto; padding: 8px 0;
      background: rgba(24, 24, 24, 0.94); color: #fff; border-radius: 10px; font: 13px/1.3 system-ui, sans-serif; box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5); }
    .cw-pc .cw-pc-menu[hidden] { display: none; }
    .cw-pc .cw-pc-menu h4 { margin: 6px 14px 4px; font-size: 11px; font-weight: 600; letter-spacing: 0.6px; text-transform: uppercase; color: rgba(255, 255, 255, 0.6); }
    .cw-pc.cw-pc .cw-pc-menu button { display: flex; width: 100%; gap: 8px; padding: 7px 14px; border-radius: 0; text-align: left; }
    .cw-pc.cw-pc .cw-pc-menu button:hover, .cw-pc.cw-pc .cw-pc-menu button:focus-visible { background: rgba(255, 255, 255, 0.12); outline: 0; }
    .cw-pc .cw-pc-menu button i { width: 14px; font-style: normal; }
    .cw-pc .cw-pc-menu button small { margin-left: auto; color: rgba(255, 255, 255, 0.6); }
    .cw-pc .cw-pc-menu .speeds { display: flex; flex-wrap: wrap; gap: 4px; padding: 2px 10px 6px; }
    .cw-pc.cw-pc .cw-pc-menu .speeds button { width: auto; padding: 5px 9px; border-radius: 14px; background: rgba(255, 255, 255, 0.1); }
    .cw-pc.cw-pc .cw-pc-menu .speeds button.on { background: #fff; color: #000; }
    .cw-pc .cw-pc-row button[aria-pressed="true"] { background: rgba(255, 255, 255, 0.25); }
    .cw-pc .cw-pc-row [hidden] { display: none; }
    .cw-pc .cw-pc-row .time { font-variant-numeric: tabular-nums; }
    .cw-pc .cw-pc-row .fill { flex: 1; }
    .cw-pc.cw-pc .cw-pc-row button { display: inline-grid; place-items: center; width: 34px; height: 30px; padding: 0; }
    .cw-pc .cw-pc-row button svg { width: 22px; height: 22px; fill: currentColor; }
    .cw-pc.cw-pc .cw-pc-row button.cc { width: auto; padding: 0 6px; font-weight: 700; font-size: 12px; letter-spacing: 0.5px; }
    .cw-pc .cw-pc-row .sound { display: inline-flex; align-items: center; }
    .cw-pc.cw-pc .cw-pc-row input[type=range] { -webkit-appearance: none; appearance: none; box-sizing: content-box; width: 64px; height: 4px; min-height: 0; padding: 0; border: 0; box-shadow: none; outline: 0; margin: 0 6px 0 2px; border-radius: 2px; cursor: pointer;
      background: linear-gradient(#fff, #fff) 0 / var(--v, 100%) 100% no-repeat, rgba(255, 255, 255, 0.3); }
    .cw-pc .cw-pc-row input[type=range]::-webkit-slider-thumb { -webkit-appearance: none; width: 12px; height: 12px; border-radius: 50%; background: #fff; border: 0; }
    .cw-pc .cw-pc-row input[type=range]::-moz-range-thumb { width: 12px; height: 12px; border-radius: 50%; background: #fff; border: 0; }
}`;
  // ONE ICON SET (drawn, so every control looks alike — an emoji's look is the system's).
  const ICON = {
    play: '<svg viewBox="0 0 24 24"><path d="M8 5v14l11-7z"/></svg>',
    pause: '<svg viewBox="0 0 24 24"><path d="M6 5h4v14H6zM14 5h4v14h-4z"/></svg>',
    sound: '<svg viewBox="0 0 24 24"><path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8v8a4.5 4.5 0 0 0 2.5-4zM14 3.2v2.1a7 7 0 0 1 0 13.4v2.1a9 9 0 0 0 0-17.6z"/></svg>',
    muted: '<svg viewBox="0 0 24 24"><path d="M3 9v6h4l5 5V4L7 9H3zm13.6 3 2.7-2.7-1.4-1.4-2.7 2.7-2.7-2.7-1.4 1.4 2.7 2.7-2.7 2.7 1.4 1.4 2.7-2.7 2.7 2.7 1.4-1.4z"/></svg>',
    gear: '<svg viewBox="0 0 24 24"><path d="M19.4 13a7.6 7.6 0 0 0 0-2l2.1-1.6-2-3.5-2.5 1a7.4 7.4 0 0 0-1.7-1L14.9 3h-4l-.4 2.9c-.6.3-1.2.6-1.7 1l-2.5-1-2 3.5L6.4 11a7.6 7.6 0 0 0 0 2l-2.1 1.6 2 3.5 2.5-1c.5.4 1.1.7 1.7 1l.4 2.9h4l.4-2.9c.6-.3 1.2-.6 1.7-1l2.5 1 2-3.5zM12.9 15.5a3.5 3.5 0 1 1 0-7 3.5 3.5 0 0 1 0 7z"/></svg>',
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

  // THE BOX: its shape from the video's width and height, before anything plays (the poster) and after.
  function frame(stage, { width = 16, height = 9 } = {}) {
    stage.classList.add("cw-pc");
    stage.style.setProperty("--cw-pc-w", String(width || 16));
    stage.style.setProperty("--cw-pc-h", String(height || 9));
    return stage;
  }

  function attach(stage, video, { duration = 0, width, height } = {}) {
    if (width && height) frame(stage, { width, height });
    else stage.classList.add("cw-pc");
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
    const cc = h("button", { type: "button", className: "cc", title: "Subtitles (c)", textContent: "CC", hidden: true });
    const gear = h("button", { type: "button", title: "Settings: quality, speed", innerHTML: ICON.gear, "aria-haspopup": "true" });
    const full = h("button", { type: "button", title: "Full screen (f)", innerHTML: ICON.full });
    const bar = h("div", { className: "cw-pc-bar" }, line, h("div", { className: "cw-pc-row" }, play, h("span", { className: "sound" }, mute, vol), time, h("span", { className: "fill" }), cc, gear, full));
    // SETTINGS: quality (Auto, and what Auto plays now; or one rendition) and playback speed.
    const menu = h("div", { className: "cw-pc-menu", hidden: true, role: "menu" });
    stage.append(bar, menu);
    let labels = [];
    let pick = null;
    let chosen = null; // a rendition's index, or null: Auto
    let playing = "";
    const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2];
    const drawMenu = () => {
      const q = labels.length
        ? [
            h("h4", { textContent: "Quality" }),
            h("button", { type: "button", onclick: () => choose(null) }, h("i", { textContent: chosen == null ? "✓" : "" }), "Auto", h("small", { textContent: chosen == null ? playing : "" })),
            ...labels.map((l, i) => h("button", { type: "button", onclick: () => choose(i) }, h("i", { textContent: chosen === i ? "✓" : "" }), l)).reverse(),
          ]
        : [];
      menu.replaceChildren(
        ...q,
        h("h4", { textContent: "Speed" }),
        h("div", { className: "speeds" }, ...SPEEDS.map(x => h("button", { type: "button", className: video.playbackRate === x ? "on" : "", textContent: x === 1 ? "Normal" : `${x}×`, onclick: () => ((video.playbackRate = x), drawMenu()) }))),
      );
    };
    const choose = i => {
      chosen = i;
      pick?.(i);
      menu.hidden = true;
    };
    gear.onclick = e => (e.stopPropagation(), (menu.hidden = !menu.hidden), menu.hidden || drawMenu(), wake());
    stage.addEventListener("pointerdown", e => !menu.hidden && !menu.contains(e.target) && e.target !== gear && !gear.contains(e.target) && (menu.hidden = true));

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
      idle = setTimeout(() => !video.paused && !line.dragging && menu.hidden && stage.classList.add("idle"), 3000);
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
      levels(ls, p) {
        labels = ls;
        pick = p;
        if (!menu.hidden) drawMenu();
      },
      level(label) {
        playing = label.replace(/^Auto · /, "");
        if (!menu.hidden) drawMenu();
      },
    };
  }

  return { attach, frame, clock };
}
