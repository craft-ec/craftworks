// VIDEO STUDIO, a capability (ARCHITECTURE §6 Video): a video made ready to stream, in the browser, at upload —
// Mediabunny (the `mediabunny` package: any common file read, WebCodecs decode and encode) writes each RENDITION as a
// FRAGMENTED MP4 (CMAF: fragments of ~4 s, a key frame every 2 s), one file each (`files`: coded, sealed, raced), its
// fragments INDEXED by byte range and time as they are written. A LEAN ladder (every byte is the uploader's upload and
// the network's keeping): ONE EFFICIENT family at full quality up to the source's height, 4K included — the one this
// device encodes in HARDWARE (measured): AV1 + Opus where it can, else HEVC + AAC (Macs, iPhones, most GPUs) — and
// H.264 + AAC only at 720p and 360p, the safety net for players that cannot decode that family. A device with no
// efficient hardware encoder makes an H.264 ladder alone. A
// POSTER and a SCRUB STRIP (a sprite of frames across the video). All of it named by one MANIFEST (JSON, a file of its
// own): what the Video item carries. (Captions are data of their own: `caption-store`.)
//
// FAST, THEN IN THE BACKGROUND (no server encodes here; the uploader's own devices do): an upload keeps the ORIGINAL,
// makes ONE rendition (H.264, on the hardware encoder) and posts — watchable in seconds. The rest of the ladder is
// PENDING in the manifest: due work, no job list. Any open page of the uploader's devices makes them, one at a time,
// resuming from the original: each done rendition a new manifest, the item's file replaced (viewers get it). One
// device at a time per video — a LEASE in the account's table `encodes` (with its progress, shown on the video), taken
// over when it goes quiet. Keeping the original also lets a newer codec (AV2) be added later without a new upload.
//
//   const studio = await ctx.require("video-studio");
//   const ref = await studio.make(file, { space, public, keepOriginal, onProgress })   // the manifest's reference
//   (captions are `caption-store`'s: attached to the item once posted)
//   studio.progress(ref) -> { stage, p, done, of } | null          // a video still being made (the uploader's devices)
//   studio.MANIFEST   // its type: "application/vnd.craftworks.video+json"
export async function start(ctx) {
  const [files, storage, space, items, kinds] = await Promise.all(["files", "storage", "space", "items", "kinds"].map(n => ctx.require(n)));
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
  // THE LADDER, LEAN (every byte is the uploader's upload and the network's keeping): ONE EFFICIENT family at full
  // quality — the one this device encodes in HARDWARE: AV1 if it can, else HEVC — and H.264 only as the small safety
  // net (720p, 360p) for players that cannot decode that family. A device with no efficient hardware encoder makes an
  // H.264 ladder alone.
  const EFFICIENT_HEIGHTS = [2160, 1080, 720, 480];
  const NET_HEIGHTS = [720, 360];
  const AVC_ONLY_HEIGHTS = [2160, 1080, 720, 480, 360];
  const AVC = { 2160: 16_000_000, 1440: 9_000_000, 1080: 5_000_000, 720: 2_800_000, 480: 1_400_000, 360: 800_000, 240: 400_000 };
  const AV1 = { 2160: 9_000_000, 1440: 5_000_000, 1080: 3_000_000, 720: 1_700_000, 480: 850_000, 360: 500_000, 240: 250_000 };
  // HEVC: close to AV1's size, and encoded in HARDWARE on Macs, iPhones and most GPUs (AV1's encoder is rarer).
  const HEVC = { 2160: 10_000_000, 1440: 5_500_000, 1080: 3_300_000, 720: 1_900_000, 480: 950_000, 360: 550_000, 240: 280_000 };
  const LABEL = { avc: "H.264", hevc: "HEVC", av1: "AV1" };
  // AV1 encoded in SOFTWARE (no AV1 encoder in the hardware — every Mac, most PCs) stops at 1080p to keep it in
  // reason; with a hardware AV1 encoder (NVIDIA RTX 40, Intel Arc, AMD RX 7000) it goes to the top.
  const AV1_SOFTWARE_MAX = 1080;

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
      // H.264 and HEVC on the hardware encoder; AV1 as this browser has it (hardware where there is one).
      video: codec === "aac" ? { discard: true } : { codec, height, bitrate, keyFrameInterval: 2, forceTranscode: true, hardwareAcceleration: codec === "av1" ? "no-preference" : "prefer-hardware" },
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

  // MAKE — the one way a video or an audio is uploaded, from any app (Videos, Audio, an editor's 🖼, a comment, a
  // message): made ready to stream (renditions, strip, cover: a manifest); a browser that cannot encode it, the file
  // AS IT IS (played by its byte ranges), with its poster and length.
  // Its own TAGS (title, artist, album, year, genre: `probe`) kept on its reference whichever way it came — so the
  // file shows the same wherever it is (Videos, Audio, inline in a post). Lyrics, being long, stay with the caller.
  async function make(file, opts = {}) {
    const t = await probe(file).catch(() => null);
    const tags = t ? Object.fromEntries(["title", "artist", "album", "year", "genre"].map(k => [k, t[k]]).filter(([, v]) => v)) : {};
    const ref = await made(file, opts);
    return Object.keys(tags).length ? { ...ref, tags } : ref;
  }
  async function made(file, opts) {
    try {
      return await encode(file, opts);
    } catch (err) {
      ctx.log("video-studio", { what: `${file.name}: not encoded here (${err?.message ?? err}): the file as it is` });
      const isVideo = /^video\//.test(file.type);
      const m = isVideo ? await (await ctx.require("video-player")).meta(file).catch(() => ({})) : {};
      const up = await files.put(file, { space: opts.space ?? null, public: !!opts.public, app: opts.app ?? "video", onProgress: p => opts.onProgress?.({ stage: "uploading", p: p.done / Math.max(1, p.size) }) });
      return { ...up, name: file.name, ...(m.poster ? { preview: m.poster } : {}), ...(m.duration ? { duration: m.duration } : {}) };
    }
  }
  async function encode(file, { space = null, public: pub = false, keepOriginal = false, onProgress = () => {} } = {}) {
    const M = await mb();
    const say = (stage, p = 0) => onProgress({ stage, p });
    const input = new M.Input({ source: new M.BlobSource(file), formats: M.ALL_FORMATS });
    const track = await input.getPrimaryVideoTrack();
    if (!track) return makeAudio(M, file, input, { space, pub, keepOriginal, say });
    const audioTrack = await input.getPrimaryAudioTrack();
    const duration = await input.computeDuration();
    const srcH = track.displayHeight;
    const widthOf = hh => Math.max(2, Math.round((hh * track.displayWidth) / track.displayHeight / 2) * 2);
    // Which families this browser encodes.
    const can = async (codec, hh) => M.canEncodeVideo(codec, { width: widthOf(hh), height: hh }).catch(() => false);
    const aac = await M.canEncodeAudio("aac").catch(() => false);
    // Heights up to the source's (the source's own when it is below the ladder).
    const upTo = hs => {
      const out = hs.filter(x => x <= srcH);
      return out.length ? out : [Math.max(2, srcH - (srcH % 2))];
    };
    const hw = async (codec, hh) => M.canEncodeVideo(codec, { width: widthOf(hh), height: hh, hardwareAcceleration: "prefer-hardware" }).catch(() => false);
    const top = upTo(EFFICIENT_HEIGHTS)[0];
    const efficient = (await hw("av1", top)) ? "av1" : (await hw("hevc", top)) ? "hevc" : null;
    const audioFor = codec => (audioTrack ? (codec === "av1" ? { codec: "opus", bitrate: 96_000 } : { codec: aac ? "aac" : "opus", bitrate: 128_000 }) : null);
    const RATES = { avc: AVC, hevc: HEVC, av1: AV1 };
    const rung = (codec, hh) => ({ codec, height: hh, bitrate: RATES[codec][hh] ?? Math.round((RATES[codec][360] * hh) / 360), audio: audioFor(codec) });
    const plan = efficient ? [...upTo(NET_HEIGHTS).map(hh => rung("avc", hh)), ...upTo(EFFICIENT_HEIGHTS).map(hh => rung(efficient, hh))] : upTo(AVC_ONLY_HEIGHTS).map(hh => rung("avc", hh));
    if (!(await can("avc", plan[0].height))) throw new Error("this browser cannot encode H.264 video");

    say("poster");
    const posterUrl = await poster(M, track, duration);
    say("scrub strip");
    const sp = await strip(M, track, duration).catch(() => null);
    // THE ORIGINAL, kept: what the background makes the rest from (and a newer codec later).
    const source = await files.put(file, { space, public: pub, app: "video", onProgress: e => say("keeping the original", e.done / Math.max(1, e.size)) });
    originals.set(source.id ?? source.root, file);
    // THE VIDEO's ID, fixed now (a re-key later does not change it): from the original's key — public: its content
    // alone (the same video anywhere has one id); otherwise salted by its space (only its readers can name it).
    const vid = await videoId(source.key);
    // ONE rendition now: H.264 at up to 720p (the hardware encoder: seconds) — the rest pending.
    const first = plan.filter(r => r.codec === "avc").sort((a, b) => Math.abs(a.height - 720) - Math.abs(b.height - 720))[0];
    const renditions = [];
    const made = await makeRendition(M, file, first, { space, pub, name: file.name, say: (st, p) => say(st, p), width: widthOf(first.height) });
    if (!made) throw new Error("no H.264 rendition could be made here");
    renditions.push(made);
    const stripRef = sp ? await files.put(new File([sp.blob], `${file.name}.strip.jpg`, { type: "image/jpeg" }), { space, public: pub, app: "video" }) : null;
    // (Captions are data of their own — `caption-store` — attached to the item once it is posted.)
    say("manifest");
    const manifest = {
      v: 1,
      name: file.name,
      duration,
      width: track.displayWidth,
      height: track.displayHeight,
      source,
      vid,
      at: Date.now(), // when the video was made: nothing about it (a subtitle) is older
      renditions,
      pending: plan.filter(r => r !== first).map(({ codec, height, bitrate, audio }) => ({ codec, height, bitrate, audio })),
      ...(sp ? { strip: { ref: stripRef, n: sp.n, cols: sp.cols, w: sp.w, h: sp.h, every: sp.every } } : {}),
      pub: !!pub,
      // The ORIGINAL is kept for good only when asked (a newer codec later); else released once the ladder is made.
      keepOriginal: !!keepOriginal,
    };
    const ref = await putManifest(manifest, space, pub);
    say("done", 1);
    // The rest: in the background, once the item is posted (a moment for its row to be there).
    setTimeout(kick, 3000);
    return { ...ref, ...(posterUrl ? { preview: posterUrl } : {}), duration, width: track.displayWidth, height: track.displayHeight };
  }

  const videoId = async key => [...new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array([...new TextEncoder().encode("craftworks video id"), ...key.match(/../g).map(x => parseInt(x, 16))])))].map(x => x.toString(16).padStart(2, "0")).join("");
  // AUDIO (a song, a podcast, an audiobook): ONE rendition — AAC, which every browser plays — as a fragmented MP4
  // (streamed and seekable as a video is), its COVER (embedded, or none) and its TAGS (title, artist, album …) kept.
  async function makeAudio(M, file, input, { space, pub, keepOriginal, say }) {
    const at = await input.getPrimaryAudioTrack();
    if (!at) throw new Error("no audio or video in this file");
    const duration = await input.computeDuration();
    const tags = await input.getMetadataTags().catch(() => ({}));
    const img = tags.images?.find(i => i.kind === "coverFront") ?? tags.images?.[0];
    const cover = img ? await shrink(new Blob([img.data], { type: img.mimeType }), 320).catch(() => null) : null;
    const source = keepOriginal ? await files.put(file, { space, public: pub, app: "audio", onProgress: e => say("keeping the original", e.done / Math.max(1, e.size)) }) : null;
    // Its ID by the files' rule (public: the content alone; private: salted by its space) — kept or not.
    const vid = await videoId(source?.key ?? (await files.keyOf(file, { space, public: pub })));
    const made = await makeRendition(M, file, { codec: "aac", height: 0, bitrate: 160_000, audio: { codec: "aac", bitrate: 160_000 } }, { space, pub, name: file.name, say, width: 0 });
    if (!made) throw new Error("this browser cannot encode AAC audio");
    const manifest = {
      v: 1,
      audio: true,
      at: Date.now(),
      name: file.name,
      duration,
      vid,
      renditions: [made],
      pending: [],
      tags: { title: tags.title ?? null, artist: tags.artist ?? null, album: tags.album ?? null, genre: tags.genre ?? null, year: tags.date ? new Date(tags.date).getFullYear() : null, lyrics: tags.lyrics ?? null },
      pub: !!pub,
      keepOriginal: !!keepOriginal,
      ...(source ? { source } : {}),
    };
    const ref = await putManifest(manifest, space, pub);
    say("done", 1);
    return { ...ref, ...(cover ? { preview: cover } : {}), duration, audio: true };
  }
  async function shrink(blob, w) {
    const bmp = await createImageBitmap(blob);
    const c = new OffscreenCanvas(w, Math.round((w * bmp.height) / bmp.width));
    c.getContext("2d").drawImage(bmp, 0, 0, c.width, c.height);
    const b = await c.convertToBlob({ type: "image/jpeg", quality: 0.75 });
    return await new Promise(r => {
      const fr = new FileReader();
      fr.onload = () => r(fr.result);
      fr.readAsDataURL(b);
    });
  }
  // The TAGS a file carries (title, artist, album, genre, year; embedded lyrics), to fill the upload form.
  async function probe(file) {
    const M = await mb();
    const input = new M.Input({ source: new M.BlobSource(file), formats: M.ALL_FORMATS });
    const t = await input.getMetadataTags().catch(() => ({}));
    return { title: t.title ?? null, artist: t.artist ?? null, album: t.album ?? null, genre: t.genre ?? null, year: t.date ? new Date(t.date).getFullYear() : null, lyrics: t.lyrics ?? null, video: !!(await input.getPrimaryVideoTrack().catch(() => null)) };
  }
  const widthFor = (w, h, hh) => Math.max(2, Math.round((hh * w) / h / 2) * 2);
  // ONE RENDITION made and stored: its manifest entry (null: this browser's encoder refused it).
  async function makeRendition(M, file, r, { space, pub, name, say, width = null }) {
    const label = `${LABEL[r.codec]} ${r.height}p`;
    const out = await rendition(M, file, { ...r, onProgress: p => say(`encoding ${label}`, p) }).catch(e => {
      ctx.log("video", { what: `${label} left out: ${e.message ?? e}` });
      return null;
    });
    if (!out) return null;
    say(`storing ${label}`);
    const ref = await files.put(new File([out.bytes], `${name}.${r.codec}.${r.height}p.mp4`, { type: "video/mp4" }), { space, public: pub, app: "video", onProgress: e => say(`storing ${label}`, e.done / Math.max(1, e.size)) });
    return { codec: r.codec, mime: out.mime, width, height: r.height, bitrate: r.bitrate, size: out.bytes.byteLength, ref, index: out.index };
  }
  // Never inline: it grows with the video (its fragment index), and the item's row must stay small.
  const putManifest = (m, space, pub) => files.put(new File([JSON.stringify(m)], `${m.name}.video.json`, { type: MANIFEST }), { space, public: pub, app: "video", inline: false });

  // CAN THIS DEVICE make it: H.264 as the browser has it (hardware or not), AV1 in hardware at any height, in
  // software up to 1080p. What it cannot is left PENDING for another of this person's devices that can.
  async function canHere(M, r, width) {
    // HEVC only on a hardware encoder (a software one would take far too long).
    if (r.codec === "hevc") return M.canEncodeVideo("hevc", { width, height: r.height, hardwareAcceleration: "prefer-hardware" }).catch(() => false);
    if (r.codec === "av1") {
      const hw = await M.canEncodeVideo("av1", { width, height: r.height, hardwareAcceleration: "prefer-hardware" }).catch(() => false);
      if (hw) return true;
      if (r.height > AV1_SOFTWARE_MAX) return false;
    }
    return M.canEncodeVideo(r.codec, { width, height: r.height }).catch(() => false);
  }

  // THE ORIGINAL on this page: the file picked at upload (on disk, nothing read), or read from the network ONCE and kept
  // for every rendition after it (reading it per rendition cost minutes each). One read original at a time.
  const originals = new Map(); // source id → File picked at upload (kept: on disk, not in memory)
  let read = null; // { k, file }: the one original read from the network (in memory)
  async function originalOf(m) {
    const k = m.source.id ?? m.source.root;
    if (originals.has(k)) return originals.get(k);
    if (read?.k !== k) read = { k, file: new File([await files.get(m.source)], m.name ?? "video", { type: m.source.type }) };
    return read.file;
  }

  // THE BACKGROUND: this person's videos with renditions pending, made one at a time here.
  const LEASE_QUIET = 2 * 60 * 1000;
  const device = (crypto.randomUUID?.() ?? String(Math.random())).slice(0, 12);
  const leases = () => storage.table("encodes");
  let working = false;
  let again = false;
  async function kick() {
    if (working) return void (again = true);
    working = true;
    try {
      do {
        again = false;
        await work();
      } while (again);
    } catch (e) {
      ctx.log("video", { what: `background: ${e.message ?? e}` });
    } finally {
      working = false;
    }
  }
  async function work() {
    const me = await space.account().catch(() => null);
    if (!me) return;
    const t = await leases();
    await t.settled;
    const mine = await items.list({ by: me.id }, "new", kinds.inDomain("video")).catch(() => []);
    for (const v of mine) {
      const f = v.files?.find(x => x.type === MANIFEST);
      if (!f) continue;
      const m = JSON.parse(await (await files.get(f)).text());
      if (!m.pending?.length || !m.source) {
        if (t.rows().some(r => r.key === v.ref && r.value)) await t.remove(v.ref);
        continue;
      }
      // The LEASE: another device making it, and heard from lately — theirs.
      let held = null;
      try {
        held = JSON.parse(t.rows().find(r => r.key === v.ref)?.value ?? "null");
      } catch {}
      if (held && !held.waiting && held.device !== device && Date.now() - held.at < LEASE_QUIET) continue;
      const lease = (stage, p) => t.put(v.ref, JSON.stringify({ device, at: Date.now(), stage, p: Math.round((p || 0) * 100) / 100, done: m.renditions.length, of: m.renditions.length + m.pending.length }));
      let beat = 0;
      const say = (stage, p) => Date.now() - beat > 5000 && ((beat = Date.now()), lease(stage, p).catch(() => {}));
      await lease("reading the original", 0);
      const M = await mb();
      const original = await originalOf(m);
      const sp = v.board ? (await space.mine()).find(x => x.id === v.board.id) ?? null : null;
      // One rendition — the first pending this device CAN make (the rest wait for a device that can) — then the manifest
      // written again and the item's file replaced.
      let r = null;
      for (const x of m.pending) if (await canHere(M, x, widthFor(m.width, m.height, x.height))) (r = r ?? x);
      if (!r) {
        await t.put(v.ref, JSON.stringify({ device, at: Date.now(), stage: `waiting for a device that can encode ${m.pending.map(x => `${LABEL[x.codec]} ${x.height}p`).join(", ")}`, p: 0, done: m.renditions.length, of: m.renditions.length + m.pending.length, waiting: true }));
        continue;
      }
      const made = await makeRendition(M, original, r, { space: sp, pub: m.pub, name: m.name ?? "video", say, width: widthFor(m.width, m.height, r.height) });
      // Made, or refused by this encoder after all: out of the pending either way (a refusal is not tried again here).
      // Applied to the manifest as it is NOW (subtitles may have changed while this was encoding).
      const cur = (await latest(v.ref)) ?? { item: v, manifest: m, file: f };
      const same = x => x.codec === r.codec && x.height === r.height;
      const next = { ...cur.manifest, renditions: made ? [...cur.manifest.renditions, made] : cur.manifest.renditions, pending: (cur.manifest.pending ?? []).filter(x => !same(x)) };
      // All made: the original RELEASED unless it is to be kept (no longer named: Lifecycle stops keeping it).
      if (!next.pending.length && !next.keepOriginal && next.source) delete next.source;
      const ref = await putManifest(next, sp, m.pub);
      await replaceManifest(cur.item, cur.file, ref);
      if (next.pending.length) await lease("next rendition", 0);
      else await t.remove(v.ref);
      again = true;
      return;
    }
  }
  async function progress(ref) {
    const t = await leases().catch(() => null);
    try {
      return JSON.parse(t?.rows().find(r => r.key === ref)?.value ?? "null");
    } catch {
      return null;
    }
  }
  // Started with the page (the header asks for it), and whenever this person's posts change.
  // And EVERY MINUTE: a lease whose page went away (closed, reloaded) goes quiet, and an open page takes it over then.
  setTimeout(() => (space.account().then(a => a && kick(), () => {}), items.onChange(() => kick())), 5000);
  setInterval(() => space.account().then(a => a && kick(), () => {}), 60000);

  // A video's item and manifest as they are now.
  async function latest(ref) {
    const item = await items.get(ref).catch(() => null);
    const file = item?.files?.find(x => x.type === MANIFEST);
    if (!file) return null;
    return { item, file, manifest: JSON.parse(await (await files.get(file)).text()) };
  }
  // The item's manifest replaced by a new one (its poster and size kept on the reference).
  const replaceManifest = (item, old, ref) =>
    items.setFiles(item.ref, item.files.map(x => (x === old ? { ...ref, ...(old.preview ? { preview: old.preview } : {}), duration: old.duration, width: old.width, height: old.height } : x)));

  return { make, probe, progress, kick, videoId, MANIFEST };
}
