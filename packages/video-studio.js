// VIDEO STUDIO, a capability (ARCHITECTURE §6 Video): a video made ready to stream, in the browser, at upload —
// Mediabunny (the `mediabunny` package: any common file read, WebCodecs decode and encode) writes each RENDITION as a
// FRAGMENTED MP4 (CMAF: fragments of ~4 s, a key frame every 2 s), one file each (`files`: coded, sealed, raced), its
// fragments INDEXED by byte range and time as they are written. Renditions, per codec FAMILY: H.264 + AAC always (every
// browser), AV1 + Opus where this browser can encode it (smaller, sharper); a ladder of heights up to the source's. A
// POSTER, a SCRUB STRIP (a sprite of frames across the video), SUBTITLES (WebVTT; SRT converted). All of it named by one
// MANIFEST (JSON, a file of its own): what the Videos item carries.
//
//   const studio = await ctx.require("video-studio");
//   const ref = await studio.make(file, { space, public, subtitles: [file…], onProgress })   // the manifest's reference
//   studio.MANIFEST   // its type: "application/vnd.craftworks.video+json"
export async function start(ctx) {
  const files = await ctx.require("files");
  const MANIFEST = "application/vnd.craftworks.video+json";

  let loading = null;
  const mb = () =>
    (loading ??= (async () => {
      const url = URL.createObjectURL(new Blob([await ctx.require("mediabunny")], { type: "text/javascript" }));
      try {
        return await import(url);
      } finally {
        URL.revokeObjectURL(url);
      }
    })().catch(e => ((loading = null), Promise.reject(e))));

  // THE LADDER: heights (up to the source's), each family's bitrate at that height (bits/s).
  const HEIGHTS = [1080, 720, 480, 240];
  const AVC = { 1080: 5_000_000, 720: 2_800_000, 480: 1_400_000, 240: 400_000 };
  const AV1 = { 1080: 3_000_000, 720: 1_700_000, 480: 850_000, 240: 250_000 };
  const AV1_MAX = 720; // AV1 is encoded in software here: its ladder stops at 720p to keep an upload in reason

  // ONE RENDITION: the source converted to `codec` at `height`, a fragmented MP4 — its bytes, MIME, and fragment index.
  async function rendition(M, file, { codec, height, width, bitrate, audio, onProgress }) {
    const input = new M.Input({ source: new M.BlobSource(file), formats: M.ALL_FORMATS });
    const index = { init: 0, segments: [] };
    const output = new M.Output({
      format: new M.Mp4OutputFormat({
        fastStart: "fragmented",
        minimumFragmentDuration: 4,
        onMoov: (data, position) => (index.init = position + data.byteLength),
        onMoof: (data, position, timestamp) => index.segments.push({ t: timestamp, start: position }),
      }),
      target: new M.BufferTarget(),
    });
    const conv = await M.Conversion.init({
      input,
      output,
      tracks: "primary",
      // H.264 on the hardware encoder where there is one; AV1 as this browser has it (often software only).
      video: { codec, height, bitrate, keyFrameInterval: 2, forceTranscode: true, hardwareAcceleration: codec === "avc" ? "prefer-hardware" : "no-preference" },
      audio: audio ? { codec: audio.codec, bitrate: audio.bitrate, forceTranscode: true } : { discard: true },
      showWarnings: false,
    });
    if (!conv.isValid) throw new Error(`${codec} ${height}p: ${conv.discardedTracks.map(d => d.reason).join(", ") || "not possible here"}`);
    conv.onProgress = p => onProgress(p);
    await conv.execute();
    const bytes = output.target.buffer;
    // Each fragment runs to the next one's start (the last to the end).
    index.segments.forEach((s, i) => (s.end = index.segments[i + 1]?.start ?? bytes.byteLength));
    return { bytes, mime: await output.getMimeType(), index };
  }

  // THE SCRUB STRIP: `n` frames across the video, `w`×`h` each, in one JPEG sprite (`cols` wide).
  async function strip(M, track, duration) {
    const n = Math.max(1, Math.min(100, Math.round(duration / 2)));
    const w = 160;
    const h = Math.max(1, Math.round((w * track.displayHeight) / Math.max(1, track.displayWidth)));
    const cols = 10;
    const every = duration / n;
    const sheet = new OffscreenCanvas(w * cols, h * Math.ceil(n / cols));
    const g = sheet.getContext("2d");
    const sink = new M.CanvasSink(track, { width: w, height: h, fit: "cover" });
    let i = 0;
    for await (const c of sink.canvasesAtTimestamps(Array.from({ length: n }, (_, k) => k * every))) {
      if (c) g.drawImage(c.canvas, (i % cols) * w, Math.floor(i / cols) * h);
      i++;
    }
    return { blob: await sheet.convertToBlob({ type: "image/jpeg", quality: 0.7 }), n, cols, w, h, every };
  }
  async function poster(M, track, duration) {
    const c = await new M.CanvasSink(track, { width: 640, fit: "contain" }).getCanvas(Math.min(duration / 3, 3)).catch(() => null);
    if (!c) return null;
    const small = new OffscreenCanvas(320, Math.round((320 * c.canvas.height) / c.canvas.width));
    small.getContext("2d").drawImage(c.canvas, 0, 0, small.width, small.height);
    const b = await small.convertToBlob({ type: "image/jpeg", quality: 0.7 });
    return await new Promise(r => {
      const fr = new FileReader();
      fr.onload = () => r(fr.result);
      fr.readAsDataURL(b);
    });
  }
  // SUBTITLES: WebVTT kept; SRT made WebVTT (its commas become points, a header added).
  async function vtt(file) {
    const text = await file.text();
    if (/^\uFEFF?WEBVTT/.test(text)) return text;
    return `WEBVTT\n\n${text.replace(/\r/g, "").replace(/(\d\d:\d\d:\d\d),(\d\d\d)/g, "$1.$2")}`;
  }

  async function make(file, { space = null, public: pub = false, subtitles = [], onProgress = () => {} } = {}) {
    const M = await mb();
    const say = (stage, p = 0) => onProgress({ stage, p });
    const input = new M.Input({ source: new M.BlobSource(file), formats: M.ALL_FORMATS });
    const track = await input.getPrimaryVideoTrack();
    if (!track) throw new Error("no video in this file");
    const audioTrack = await input.getPrimaryAudioTrack();
    const duration = await input.computeDuration();
    const srcH = track.displayHeight;
    const heights = HEIGHTS.filter(x => x <= srcH);
    if (!heights.length) heights.push(Math.max(2, srcH - (srcH % 2)));
    const widthOf = hh => Math.max(2, Math.round((hh * track.displayWidth) / track.displayHeight / 2) * 2);
    // Which families this browser encodes.
    const can = async (codec, hh) => M.canEncodeVideo(codec, { width: widthOf(hh), height: hh }).catch(() => false);
    const aac = await M.canEncodeAudio("aac").catch(() => false);
    const plan = [];
    for (const hh of heights) if (await can("avc", hh)) plan.push({ codec: "avc", height: hh, bitrate: AVC[hh] ?? 400_000, audio: audioTrack ? { codec: aac ? "aac" : "opus", bitrate: 128_000 } : null });
    for (const hh of heights.filter(x => x <= AV1_MAX)) if (await can("av1", hh)) plan.push({ codec: "av1", height: hh, bitrate: AV1[hh] ?? 250_000, audio: audioTrack ? { codec: "opus", bitrate: 96_000 } : null });
    if (!plan.length) throw new Error("this browser cannot encode video (no WebCodecs encoder for H.264 or AV1)");

    say("poster");
    const posterUrl = await poster(M, track, duration);
    say("scrub strip");
    const sp = await strip(M, track, duration).catch(() => null);
    const renditions = [];
    for (const [i, r] of plan.entries()) {
      const label = `${r.codec === "av1" ? "AV1" : "H.264"} ${r.height}p (${i + 1} of ${plan.length})`;
      // A rendition this browser turns out not to make (a configuration its encoder refuses) is left out; H.264 must stay.
      const out = await rendition(M, file, { ...r, width: widthOf(r.height), onProgress: p => say(`encoding ${label}`, p) }).catch(e => {
        ctx.log("video", { what: `${label} left out: ${e.message ?? e}` });
        return null;
      });
      if (!out) continue;
      say(`storing ${label}`);
      const ref = await files.put(new File([out.bytes], `${file.name}.${r.codec}.${r.height}p.mp4`, { type: "video/mp4" }), {
        space,
        public: pub,
        app: "videos",
        onProgress: e => say(`storing ${label}`, e.done / Math.max(1, e.size)),
      });
      renditions.push({ codec: r.codec, mime: out.mime, width: widthOf(r.height), height: r.height, bitrate: r.bitrate, size: out.bytes.byteLength, ref, index: out.index });
    }
    if (!renditions.some(r => r.codec === "avc")) throw new Error("no H.264 rendition could be made here");
    const stripRef = sp ? await files.put(new File([sp.blob], `${file.name}.strip.jpg`, { type: "image/jpeg" }), { space, public: pub, app: "videos" }) : null;
    const subs = [];
    for (const s of subtitles) {
      const text = await vtt(s);
      const ref = await files.put(new File([text], s.name.replace(/\.\w+$/, ".vtt"), { type: "text/vtt" }), { space, public: pub, app: "videos" });
      subs.push({ label: s.name.replace(/\.\w+$/, ""), lang: (s.name.match(/\.([a-z]{2,3})\.\w+$/i)?.[1] ?? "").toLowerCase(), ref });
    }
    say("manifest");
    const manifest = {
      v: 1,
      duration,
      width: track.displayWidth,
      height: track.displayHeight,
      renditions,
      ...(sp ? { strip: { ref: stripRef, n: sp.n, cols: sp.cols, w: sp.w, h: sp.h, every: sp.every } } : {}),
      subtitles: subs,
    };
    const ref = await files.put(new File([JSON.stringify(manifest)], `${file.name}.video.json`, { type: MANIFEST }), { space, public: pub, app: "videos" });
    say("done", 1);
    return { ...ref, ...(posterUrl ? { preview: posterUrl } : {}), duration, width: track.displayWidth, height: track.displayHeight };
  }

  return { make, MANIFEST };
}
