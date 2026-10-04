// CAPTION STORE, a capability: CAPTIONS — subtitles, lyrics, transcripts — as data of their own, like Drive's files —
// `content` of the attaching kind "caption" (`kinds`): its language and label (`meta`), its cues a WebVTT file (a film's run to 100 KB: never in the
// row), and what it is FOR — the VIDEO's ID (`meta.for`: fixed at upload from the original's key: public — the content
// alone, the same video anywhere; private — salted by its space, so only its readers can name it), with the item it
// was made on (`in`). KEPT where its author chooses: with the video (its place), their own, or a team's space. FOUND by
// the video's id in the places this person can READ — the video's own, their own, the spaces they are in, the people
// and spaces they follow (public ones from outside): access control and privacy decide what loads, nothing else.
// Any player composes them; the Caption app is their lens. Portable: WebVTT in, WebVTT or SRT out.
//
//   const subs = await ctx.require("caption-store");
//   await subs.of(mediaRef, { outside })          // [{ ref, lang, label, by, at, file, place, mayRemove }], oldest first
//   await subs.add(mediaRef, fileOrText, { lang, label, place })   // its ref (SRT made WebVTT); `place`: a space, null
//   await subs.mine({ sort, by, window })        // a person's tracks (`by`: theirs; none: yours) — their space's
//                                                  // (your own), or none: with the video
//   await subs.videoId(mediaRef)                   // the id tracks are matched by (null: a video from before ids)
//   await subs.update(ref, { lang, label, text })  await subs.remove(ref)
//   await subs.text(sub)   subs.toSrt(vttText)      // its WebVTT; the same as SRT
//   await subs.mine()                              // this person's subtitles, everywhere they are (the app's list)
//   await subs.ofFile(itemRef, fileRef)             // a media FILE's tracks (inline in an item), as `of` gives them
//   await subs.forFile(fileRef, { item })           // the same, for its player: [{ label, lang, text }]
//   await subs.add(itemRef, text, { file })         // a track made for a FILE in the item
export async function start(ctx) {
  const [items, files, space] = await Promise.all(["items", "files", "space"].map(n => ctx.require(n)));
  const KIND = "caption";

  // WebVTT from what was given: WebVTT kept; SRT made WebVTT (its commas become points, a header added).
  const toVtt = text => {
    text = String(text ?? "").replace(/^\uFEFF/, "").replace(/\r/g, "");
    if (/^WEBVTT/.test(text)) return text;
    return `WEBVTT\n\n${text.replace(/(\d\d:\d\d:\d\d),(\d\d\d)/g, "$1.$2")}`;
  };
  // SRT from WebVTT: cues numbered, points made commas, the header and settings dropped.
  const toSrt = vtt =>
    toVtt(vtt)
      .replace(/^WEBVTT[^\n]*\n+/, "")
      .split(/\n{2,}/)
      .filter(b => /-->/.test(b))
      .map((b, i) => {
        const lines = b.split("\n").filter(l => l && !/^NOTE/.test(l));
        const at = lines.findIndex(l => l.includes("-->"));
        const time = lines[at].replace(/(\d\d:\d\d:\d\d)\.(\d\d\d)/g, "$1,$2").replace(/(-->\s*[\d:,]+).*$/, "$1");
        return `${i + 1}\n${time}\n${lines.slice(at + 1).join("\n")}`;
      })
      .join("\n\n")
      .concat("\n");

  // THE VIDEO's ID: fixed at upload (its manifest's `vid`, or its item's `meta.vid`); null for a video from before ids.
  async function videoId(mediaRef) {
    const item = await items.get(mediaRef).catch(() => null);
    if (!item) return null;
    if (item.meta?.vid) return item.meta.vid;
    const m = item.files?.find(f => f.type === "application/vnd.craftworks.video+json");
    if (!m) return null;
    return JSON.parse(await (await files.get(m)).text()).vid ?? null;
  }
  // WHERE a track is kept, and whether it is public there: with the video — as the video is; in a space — as that
  // space's subtitles are; this person's own — public, unless it is about a private item of theirs.
  async function keep(mediaRef, place) {
    if (place === undefined) {
      const item = await items.get(mediaRef).catch(() => null);
      const sp = mediaRef.startsWith("space:") ? (await space.mine()).find(s => s.id === mediaRef.slice(6, mediaRef.indexOf("/"))) ?? null : null;
      return { sp, pub: sp ? !!item?.pub : !item?.private };
    }
    if (place) return { sp: place, pub: await items.publicIn(place, KIND) };
    const item = await items.get(mediaRef).catch(() => null);
    return { sp: null, pub: !(item?.private && item.by === (await space.account())?.id) };
  }
  // `in` as a full reference (on a board an item names what it is about by id alone).
  const shape = x => ({ ref: x.ref, in: x.board && x.in && !x.in.startsWith("space:") ? `space:${x.board.id}/${x.in}` : x.in, for: x.meta?.for ?? null, lang: x.meta?.lang ?? "", label: x.meta?.label ?? x.body ?? "", by: x.by, at: x.at, edited: x.edited, file: x.files?.[0] ?? null, mayRemove: !!x.mayRemove, board: x.board, place: x.board ?? null });

  // TRACKS for a media FILE of an ITEM — the one lookup (a Videos or Audio item's own video, a video written inline in a
  // post): those attached to the item made for it (for the item's own main video, those from before ids too), and
  // those anywhere this person can READ made for its video id — the places a track can be kept: their spaces that use
  // Subtitles, their profile and those they follow — each read from when the video was made, never whole.
  const mainOf = item => item?.files?.find(f => f.type === "application/vnd.craftworks.video+json") ?? null;
  async function tracksFor(itemRef, file = undefined, { outside = null } = {}) {
    const item = await items.get(itemRef, outside ? { outside } : {}).catch(() => null);
    const main = mainOf(item);
    file = file === undefined ? main : file;
    const idOf = f => f?.id ?? f?.root ?? null;
    const isMain = !!file && !!main && idOf(file) === idOf(main);
    const m = await manifestOf(file).catch(() => null);
    const vid = m?.vid ?? (isMain ? item?.meta?.vid ?? null : null);
    const withIt = (await items.attached(itemRef, KIND, { outside }).catch(() => [])).map(shape).filter(t => (t.for ? t.for === vid : isMain));
    let elsewhere = [];
    if (!outside && vid) {
      const roles = await ctx.require("roles");
      const teams = [];
      for (const s of (await space.mine()).filter(x => x.kind === "server")) if ((await roles.of(s).catch(() => null))?.apps().includes("caption")) teams.push(s);
      const me = (await space.account()).id;
      const own = itemRef.slice(itemRef.lastIndexOf("/") + 1);
      const itemAt = /^t([0-9a-z]{9})/.test(own) ? parseInt(own.slice(1, 10), 36) : null;
      const from = itemAt == null ? (m?.at ?? null) : Math.min(itemAt, m?.at ?? itemAt);
      const after = from == null ? null : `t${Math.floor(from).toString(36).padStart(9, "0")}`;
      elsewhere = (await items.inPlaces({ spaces: teams, people: [me, ...(await items.following())] }, KIND, { after }).catch(() => [])).map(shape).filter(t => t.for === vid || t.in === itemRef);
    }
    const seen = new Set();
    return [...withIt, ...elsewhere].filter(t => !seen.has(t.ref) && seen.add(t.ref)).sort((a, b) => a.at - b.at);
  }
  // An item's own video's tracks (`tracksFor` with its main file).
  const of = (mediaRef, { outside = null } = {}) => tracksFor(mediaRef, undefined, { outside });

  // A MEDIA FILE inside an item (a video or an audio written inline in a post): its manifest (its video id, when it
  // was made, its tags), or null for a file that is not one.
  const manifestOf = async ref => (ref?.type === "application/vnd.craftworks.video+json" ? JSON.parse(await (await files.get(ref)).text()) : null);
  // A FILE's tracks inside an item (`tracksFor`).
  const ofFile = (item, ref, { outside = null } = {}) => tracksFor(item, ref, { outside });
  // For its PLAYER: `[{ label, lang, text }]` (WebVTT); an audio with none shows the lyrics its tags carried.
  async function forFile(ref, { item = null } = {}) {
    const m = await manifestOf(ref).catch(() => null);
    if (!m) return [];
    const out = [];
    for (const t of await ofFile(item, ref).catch(() => [])) {
      const text = await (async () => toVtt(await (await files.get(t.file)).text()))().catch(() => null);
      if (text) out.push({ label: t.label || t.lang, lang: t.lang, text });
    }
    if (!out.length && m.tags?.lyrics) out.push({ label: "Lyrics", lang: "", text: `WEBVTT\n\n00:00:00.000 --> ${new Date(Math.max(1, m.duration || 3600) * 1000).toISOString().slice(11, 23)}\n${String(m.tags.lyrics).trim()}\n` });
    return out;
  }
  async function store(where, vtt, name) {
    // Never inline: a long film's cues outgrow a row.
    return files.put(new File([vtt], name, { type: "text/vtt" }), { space: where.sp, public: where.pub, app: "caption", inline: false });
  }
  // `file`: a media file inside the item (inline in a post) — the track is made for IT (its video id), attached to
  // the item.
  async function add(mediaRef, source, { lang = "", label = "", place = undefined, file = null } = {}) {
    const text = typeof source === "string" ? source : await source.text();
    const name = typeof source === "string" ? "subtitles.vtt" : source.name.replace(/\.\w+$/, ".vtt");
    lang = (lang || (typeof source === "string" ? "" : (source.name.match(/\.([a-z]{2,3})\.\w+$/i)?.[1] ?? ""))).toLowerCase().slice(0, 8);
    label = (label || (typeof source === "string" ? lang || "Subtitles" : source.name.replace(/\.\w+$/, ""))).slice(0, 60);
    const vid = file ? (await manifestOf(file))?.vid ?? null : await videoId(mediaRef);
    const where = await keep(mediaRef, place);
    const ref = await store(where, toVtt(text), name);
    return items.attach(mediaRef, KIND, label, { meta: { lang, label, ...(vid ? { for: vid } : {}) }, files: [ref], ...(place !== undefined ? { place } : {}) });
  }
  async function update(ref, { lang = null, label = null, text = null } = {}) {
    const was = (await mine()).find(s => s.ref === ref);
    if (!was) throw new Error("that subtitle is not yours here");
    const meta = { lang: lang ?? was.lang, label: label ?? was.label, ...(was.for ? { for: was.for } : {}) };
    const sp = was.place ? (await space.mine()).find(s => s.id === was.place.id) ?? null : null;
    const where = sp ? { sp, pub: await items.publicIn(sp, KIND) } : await keep(was.in, null);
    const files_ = text != null ? [await store(where, toVtt(text), `${meta.label || "subtitles"}.vtt`)] : null;
    await items.editItem(ref, meta.label, { meta, ...(files_ ? { files: files_ } : {}) });
  }
  const remove = ref => items.remove(ref);
  const text = async sub => toVtt(await (await files.get(sub.file)).text());
  // THIS PERSON's tracks — within a list's window (`items.list` options: a feed bar's), newest first by default.
  async function mine({ sort = "new", by = null, ...options } = { window: "all" }) {
    const me = by ?? (await space.account()).id;
    return (await items.list({ by: me }, sort, KIND, options)).map(shape);
  }
  return { of, ofFile, tracksFor, forFile, add, update, remove, text, toSrt, toVtt, mine, videoId, KIND, shape };
}
