// VIDEO PLAYER, a capability: a video FILE played by BYTE RANGE — MediaSource fed by mp4box.js (the `mp4box` package,
// loaded on first use), which demuxes the MP4 and fragments it on the fly — the range read as the file's chunks
// (`files.range`: a seek reads only the chunks it lands in). Works in the page's sandboxed frame (no service worker).
// Anything that does not stream (not an MP4, a codec this browser lacks, no MediaSource) is loaded WHOLE instead.
// From the archived craftworks-video.js (the grid app's), fed by `files` here.
//
//   const player = await ctx.require("video-player");
//   await player.play(videoEl, ref, { onNote })   // "stream" | "whole"; videoEl in the document first
//   player.meta(file) -> { poster (a data URL), duration (s), width, height }   // at upload, from the file itself
export async function start(ctx) {
  const files = await ctx.require("files");

  // mp4box.js: its build as bytes, run once as a script (it sets `window.MP4Box`).
  let loading = null;
  const mp4box = () =>
    (loading ??= (async () => {
      if (window.MP4Box) return window.MP4Box;
      const code = await ctx.require("mp4box");
      const url = URL.createObjectURL(new Blob([code], { type: "text/javascript" }));
      await new Promise((ok, no) => {
        const s = Object.assign(document.createElement("script"), { src: url, onload: ok, onerror: () => no(new Error("mp4box did not load")) });
        document.head.append(s);
      }).finally(() => URL.revokeObjectURL(url));
      if (!window.MP4Box) throw new Error("mp4box did not load");
      // Its one benign self-correcting parse message (a `colr` box over-read) is logged as an error: kept quiet.
      const L = window.Log;
      if (L && !L.__quiet) {
        const err = L.error.bind(L);
        L.error = (tag, msg) => (tag === "BoxParser" && String(msg).includes("seeking backwards") ? undefined : err(tag, msg));
        L.__quiet = true;
      }
      return window.MP4Box;
    })().catch(e => ((loading = null), Promise.reject(e))));

  const canStream = () => "MediaSource" in window && typeof MediaSource.isTypeSupported === "function";
  const KEEP_AHEAD = 30; // seconds buffered ahead before fetching pauses
  const STEP = 512 * 1024; // bytes per range fed to mp4box

  // One SourceBuffer's appends, one at a time (appendBuffer throws while updating).
  const appender = sb => {
    const queue = [];
    const pump = () => {
      if (sb.updating || !queue.length) return;
      try {
        sb.appendBuffer(queue.shift());
      } catch {}
    };
    sb.addEventListener("updateend", pump);
    return buf => (queue.push(buf), pump());
  };
  const anyUpdating = ms => [...ms.sourceBuffers].some(b => b.updating);

  // STREAM: true once a playable track started; false (nothing played) to load it whole.
  async function stream(video, ref) {
    if (!canStream() || !/mp4|quicktime|m4v/.test(ref.type ?? "")) return false;
    const MP4Box = await mp4box().catch(() => null);
    if (!MP4Box) return false;
    const size = ref.size;
    const mp4 = MP4Box.createFile();
    const ms = new MediaSource();
    const apps = {};
    let started = false;
    let ended = false;
    let feeding = false;
    let offset = 0;
    let eof = false;
    video.src = URL.createObjectURL(ms);
    const opened = await Promise.race([new Promise(r => ms.addEventListener("sourceopen", () => r(true), { once: true })), new Promise(r => setTimeout(() => r(false), 8000))]);
    if (!opened) return false;
    const ready = new Promise(resolve => {
      mp4.onError = () => resolve(started);
      mp4.onReady = moov => {
        try {
          ms.duration = moov.duration / moov.timescale;
        } catch {}
        for (const t of moov.tracks ?? []) {
          const type = `${t.type === "audio" ? "audio" : "video"}/mp4; codecs="${t.codec}"`;
          if (!MediaSource.isTypeSupported(type)) continue;
          let sb;
          try {
            sb = ms.addSourceBuffer(type);
          } catch {
            continue;
          }
          apps[t.id] = appender(sb);
          mp4.setSegmentOptions(t.id, t.id, { nbSamples: 200 });
        }
        if (!Object.keys(apps).length) return resolve(false);
        for (const seg of mp4.initializeSegmentation()) apps[seg.id](seg.buffer);
        mp4.start();
        started = true;
        resolve(true);
      };
      mp4.onSegment = (id, _u, buffer) => apps[id]?.(buffer);
    });
    const enough = () => {
      const t = video.currentTime;
      const b = video.buffered;
      for (let i = 0; i < b.length; i++) if (t >= b.start(i) - 0.5 && b.end(i) - t >= KEEP_AHEAD) return true;
      return false;
    };
    const feed = async () => {
      if (feeding || ended) return;
      feeding = true;
      try {
        while (!ended && offset < size) {
          if (started && enough()) break;
          const bytes = await files.range(ref, offset, Math.min(STEP, size - offset));
          const ab = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
          ab.fileStart = offset;
          offset = mp4.appendBuffer(ab);
          if (offset >= size) eof = true;
        }
        if (eof && !ended) mp4.flush();
      } catch (e) {
        ctx.log("video", { what: `${ref.name}: ${e.message ?? e}` });
      } finally {
        feeding = false;
      }
    };
    feed();
    // The whole moov is needed first: a file with it at the end reads through before playing.
    const ok = await Promise.race([ready, new Promise(r => setTimeout(() => r(false), 120000))]);
    if (!ok) {
      try {
        ms.readyState === "open" && ms.endOfStream("decode");
      } catch {}
      return false;
    }
    const tick = setInterval(() => {
      if (!video.isConnected) return clearInterval(tick);
      if (ms.readyState !== "open") return;
      if (offset < size && !enough()) feed();
      else if (eof && !ended && !anyUpdating(ms)) {
        ended = true;
        try {
          ms.endOfStream();
        } catch {}
        clearInterval(tick);
      }
    }, 500);
    // A SEEK: mp4box names the byte where that time's samples start; reading goes on from there.
    video.addEventListener("seeking", () => {
      if (ended) return;
      try {
        const s = mp4.seek(video.currentTime, true);
        if (typeof s.offset === "number") (offset = Math.min(s.offset, size)), (eof = false), feed();
      } catch {}
    });
    return true;
  }

  async function play(video, ref, { onNote = () => {} } = {}) {
    onNote("Loading…");
    if (await stream(video, ref).catch(() => false)) {
      onNote("");
      return "stream";
    }
    // WHOLE: the file read through, then played.
    const blob = await files.get(ref, { onProgress: e => onNote(`Loading ${Math.round((100 * e.done) / Math.max(1, e.size))}%`) });
    video.src = URL.createObjectURL(blob);
    onNote("");
    return "whole";
  }

  // META at upload: a poster (a frame a little way in), the duration and size — from the file, in the browser.
  async function meta(file) {
    const v = Object.assign(document.createElement("video"), { muted: true, playsInline: true, preload: "metadata" });
    const url = URL.createObjectURL(file);
    try {
      v.src = url;
      await new Promise((ok, no) => ((v.onloadedmetadata = ok), (v.onerror = () => no(new Error("this video does not play here")))));
      const duration = Number.isFinite(v.duration) ? v.duration : 0;
      v.currentTime = Math.min(1, duration / 3 || 0);
      await new Promise(ok => ((v.onseeked = ok), setTimeout(ok, 3000)));
      const w = 320;
      const h = Math.round((w * (v.videoHeight || 180)) / (v.videoWidth || 320));
      const c = Object.assign(document.createElement("canvas"), { width: w, height: h });
      c.getContext("2d").drawImage(v, 0, 0, w, h);
      return { poster: c.toDataURL("image/jpeg", 0.7), duration, width: v.videoWidth, height: v.videoHeight };
    } catch {
      return { poster: null, duration: 0, width: 0, height: 0 };
    } finally {
      URL.revokeObjectURL(url);
    }
  }

  return { play, meta, canStream };
}
