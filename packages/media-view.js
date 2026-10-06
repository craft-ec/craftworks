// MEDIA VIEW, a component: the ONE way a video or an audio is shown — on its Videos or Audio page, and written inline
// in a post or a comment. A COVER until played (its poster or album cover, ▶, its length: nothing loads before),
// or playing at once; the player (`video-player`: a manifest adaptively, a plain file by its byte ranges); its
// SUBTITLES, lyrics or transcript as tracks (`caption-store.tracksFor`: the one lookup, by the file's video id — the
// same tracks wherever the video shows), named with a way to add or edit them (the Caption app, for THIS file of
// this item); an audio's first track shown IN TIME beside it (the line playing lit; a click plays from it).
//
//   const mv = await ctx.require("media-view");
//   const v = mv.create({ file, item, kind, cover, outside, label, onNote, onLevel, onLevels })
//   host.append(v.el)      // cover, or the player (with an audio's cover above it)
//   v.media                // the <video>/<audio> (once playing)      v.line   v.timed   (placed by the caller, or in v.el)
//   v.play()               // start (a cover: its click does)      v.prepare()   // its tracks' line now, nothing played
export async function start(ctx) {
  const [player, subs, kindsCap] = await Promise.all(["video-player", "caption-store", "kinds"].map(n => ctx.require(n)));
  const markdown = await ctx.require("markdown");
  const style = document.createElement("style");
  style.textContent = `
    .cw-mv { display: grid; gap: 4px; max-width: 100%; }
    .cw-mv video { width: 100%; max-height: 70vh; background: #000; border-radius: var(--cw-radius-sm); }
    .cw-mv audio { width: min(560px, 100%); }
    .cw-mv .tags { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-mv .album { width: min(240px, 100%); aspect-ratio: 1; object-fit: cover; border-radius: var(--cw-radius-sm); border: 1px solid var(--cw-line); }
    .cw-mv .cover { position: relative; padding: 0; cursor: pointer; aspect-ratio: 16 / 9; width: min(560px, 100%); overflow: hidden; color: #fff; background: #111;
      border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); }
    .cw-mv .cover.audio { aspect-ratio: 1; width: min(240px, 100%); }
    .cw-mv .cover img { width: 100%; height: 100%; object-fit: cover; display: block; }
    .cw-mv .cover .ic { font-size: 2rem; position: absolute; inset: 0; display: grid; place-items: center; opacity: 0.6; }
    .cw-mv .cover .play { position: absolute; left: 50%; top: 50%; transform: translate(-50%, -50%); width: 56px; height: 40px; border-radius: 12px;
      background: rgba(0, 0, 0, 0.7); display: grid; place-items: center; font-size: 18px; }
    .cw-mv .cover:hover .play { background: #e00; }
    .cw-mv .cover .cap { position: absolute; left: 0; right: 0; bottom: 0; padding: 4px 8px; font-size: 12px; text-align: left;
      background: linear-gradient(transparent, rgba(0, 0, 0, 0.7)); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .cw-mv .line { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-mv .note { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-mv .timed { max-height: 240px; overflow-y: auto; border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); padding: 6px 10px; }
    .cw-mv .timed[hidden] { display: none; }
    .cw-mv .timed p { margin: 2px 0; cursor: pointer; color: var(--cw-muted); }
    .cw-mv .timed p.on { color: var(--cw-fg); font-weight: 600; }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const clock = s => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;

  function create({ file, item = null, kind = null, cover = false, outside = null, label = null, itemKind = null, onNote = null, onLevel = null, onLevels = null, place = true } = {}) {
    kind ??= markdown.kindOf(file) === "audio" ? "audio" : "video";
    // What its tracks are called: Subtitles, Lyrics (a song), Transcript (a podcast, an audiobook).
    const NAME = label ?? kindsCap.attachLabel("caption", itemKind ?? (kind === "audio" ? "music" : "video"));
    const el = h("div", { className: "cw-mv" });
    const note = h("span", { className: "note" });
    const line = h("div", { className: "line" });
    const timed = h("div", { className: "timed", hidden: true });
    // Its own TAGS (`video-studio`: title · artist · album), shown wherever it plays.
    const t = file?.tags ?? {};
    const caption = [t.title, t.artist, t.album].filter(Boolean).length ? h("div", { className: "tags", textContent: [t.title, t.artist, t.album].filter(Boolean).join(" · ") }) : null;
    const media = h(kind, { controls: true, playsInline: true, preload: "none" });
    if (kind === "video" && file?.preview) media.poster = file.preview;
    // Where its tracks are added: the Caption app, for the item's own video, or THIS file in the item.
    const hasId = () => file?.type === "application/vnd.craftworks.video+json"; // what tracks are made for (its video id)
    const subsHref = () => (item ? `#/caption/for/${encodeURIComponent(item)}${cover ? `~${markdown.keyOf(file)}` : ""}` : null);
    let started = false;
    // Its TRACKS read (a few rows, not the media): shown in its line at once, the player's once it plays.
    let prepared = null;
    const prepare = () => (prepared ??= tracks().catch(e => (line.textContent = e.message ?? String(e))));
    async function play({ autoplay = false } = {}) {
      if (started) return;
      started = true;
      if (place) {
        el.replaceChildren(...[kind === "audio" && file?.preview ? h("img", { className: "album", src: file.preview, alt: file?.name ?? "" }) : null, caption, media, note, line, timed].filter(Boolean));
      }
      media.autoplay = cover || autoplay; // a click on a cover (or the player's ▶) asked for it to play
      try {
        await player.play(media, file, { onNote: t => ((note.textContent = t), onNote?.(t)), onLevel: onLevel ?? undefined, onLevels: onLevels ?? undefined });
      } catch (e) {
        note.textContent = `${file?.name ?? "it"}: ${e.message ?? e}`;
      }
      await prepare();
    }
    async function tracks() {
      const ts = item ? await subs.tracksFor(item, cover ? file : undefined, { outside }) : [];
      for (const [i, t] of ts.entries()) {
        const text = await subs.text(t).catch(() => null);
        if (text) media.append(h("track", { kind: "subtitles", label: t.label || t.lang || NAME, srclang: t.lang || "und", default: i === 0 && kind === "video", src: URL.createObjectURL(new Blob([text], { type: "text/vtt" })) }));
      }
      // An audio with no track: the lyrics its tags carried.
      if (!ts.length && kind === "audio") for (const t of await subs.forFile(file).catch(() => [])) media.append(h("track", { kind: "subtitles", label: t.label, srclang: "und", src: URL.createObjectURL(new Blob([t.text], { type: "text/vtt" })) }));
      const href = subsHref();
      line.replaceChildren(
        `💬 ${NAME}: ${ts.length ? ts.map(t => `${t.label || t.lang}${t.lang ? ` (${t.lang})` : ""}`).join(", ") : "none yet"}`,
        !outside && href && hasId() ? " · " : "",
        !outside && href && hasId() ? h("a", { href, textContent: ts.length ? "Add or edit" : `Add ${NAME.toLowerCase()}` }) : "",
      );
      // AUDIO: the first track shown in time, the line playing lit; a click plays from it.
      if (kind === "audio" && media.textTracks.length) {
        const tt = media.textTracks[0];
        tt.mode = "hidden";
        const drawCues = () => {
          const cues = [...(tt.cues ?? [])];
          if (!cues.length) return;
          timed.hidden = false;
          timed.replaceChildren(
            h("strong", { textContent: NAME }),
            ...cues.map(c => {
              const p = h("p", { textContent: c.text, onclick: () => ((media.currentTime = c.startTime), media.play()) });
              p.setAttribute("data-s", String(c.startTime));
              return p;
            }),
          );
        };
        for (const t of media.querySelectorAll("track")) t.addEventListener("load", drawCues);
        setTimeout(drawCues, 500);
        tt.oncuechange = () => {
          const on = new Set([...(tt.activeCues ?? [])].map(c => String(c.startTime)));
          for (const p of timed.querySelectorAll("p")) p.classList.toggle("on", on.has(p.dataset.s));
          timed.querySelector("p.on")?.scrollIntoView({ block: "nearest" });
        };
      }
    }
    if (cover) {
      // A COVER until played: nothing loads before.
      const c = h("button", { type: "button", className: `cover${kind === "audio" ? " audio" : ""}`, title: `Play ${file?.name ?? kind}` });
      if (file?.preview) c.append(h("img", { src: file.preview, alt: file?.name ?? "" }));
      else c.append(h("span", { className: "ic", textContent: kind === "audio" ? "🎵" : "🎬" }));
      c.append(h("span", { className: "play", textContent: "▶" }), h("span", { className: "cap", textContent: [file?.name, file?.duration ? clock(file.duration) : ""].filter(Boolean).join(" · ") }));
      c.onclick = e => (e.stopPropagation(), play());
      el.append(...[c, caption].filter(Boolean));
    } else if (place) el.append(...[caption, media, note, line, timed].filter(Boolean));
    el.addEventListener("click", e => e.stopPropagation());
    return { el, media, line, timed, note, play, prepare };
  }
  return { create };
}
