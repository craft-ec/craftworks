// SUBTITLE STORE, a capability: SUBTITLES as data of their own — `content` of the attaching kind "subtitle" (`kinds`):
// about a media item (`in`: a video, an audio item), its language and label (`meta`), its cues as a WebVTT file (a
// film's run to 100 KB: never in the row). Contributed like a comment — where the item is (a space's board), or on a
// profile in the contributor's own tail, pointed to from the item — so anyone who may comment may add a track or a
// translation. Any player composes them (Videos; audio next); the Subtitles app is their lens. Portable: WebVTT in,
// WebVTT or SRT out.
//
//   const subs = await ctx.require("subtitle-store");
//   await subs.of(mediaRef, { outside })          // [{ ref, lang, label, by, at, file, mayRemove }], oldest first
//   await subs.add(mediaRef, fileOrText, { lang, label })   // its ref (SRT is made WebVTT)
//   await subs.update(ref, { lang, label, text })  await subs.remove(ref)
//   await subs.text(sub)   subs.toSrt(vttText)      // its WebVTT; the same as SRT
//   await subs.mine()                              // this person's subtitles, everywhere they are (the app's list)
export async function start(ctx) {
  const [items, files, space] = await Promise.all(["items", "files", "space"].map(n => ctx.require(n)));
  const KIND = "subtitle";

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

  // Where the media item is, and whether it is public: its subtitles are keyed and sealed as it is.
  async function placeOf(mediaRef) {
    const item = await items.get(mediaRef).catch(() => null);
    const sp = mediaRef.startsWith("space:") ? (await space.mine()).find(s => s.id === mediaRef.slice(6, mediaRef.indexOf("/"))) ?? null : null;
    // Public exactly as the item is: in a space by its own app's setting (`items`); a profile item not "only you".
    const pub = sp ? !!item?.pub : !item?.private;
    return { sp, pub };
  }
  // `in` as a full reference (on a board an item names what it is about by id alone).
  const shape = x => ({ ref: x.ref, in: x.board && x.in && !x.in.startsWith("space:") ? `space:${x.board.id}/${x.in}` : x.in, lang: x.meta?.lang ?? "", label: x.meta?.label ?? x.body ?? "", by: x.by, at: x.at, edited: x.edited, file: x.files?.[0] ?? null, mayRemove: !!x.mayRemove, board: x.board });

  async function of(mediaRef, { outside = null } = {}) {
    return (await items.attached(mediaRef, KIND, { outside })).map(shape);
  }
  async function store(mediaRef, vtt, name) {
    const { sp, pub } = await placeOf(mediaRef);
    // Never inline: a long film's cues outgrow a row.
    return files.put(new File([vtt], name, { type: "text/vtt" }), { space: sp, public: pub, app: "subtitles", inline: false });
  }
  async function add(mediaRef, source, { lang = "", label = "" } = {}) {
    const text = typeof source === "string" ? source : await source.text();
    const name = typeof source === "string" ? "subtitles.vtt" : source.name.replace(/\.\w+$/, ".vtt");
    lang = (lang || (typeof source === "string" ? "" : (source.name.match(/\.([a-z]{2,3})\.\w+$/i)?.[1] ?? ""))).toLowerCase().slice(0, 8);
    label = (label || (typeof source === "string" ? lang || "Subtitles" : source.name.replace(/\.\w+$/, ""))).slice(0, 60);
    const ref = await store(mediaRef, toVtt(text), name);
    return items.attach(mediaRef, KIND, label, { meta: { lang, label }, files: [ref] });
  }
  async function update(ref, { lang = null, label = null, text = null } = {}) {
    const cur = (await items.get(ref).catch(() => null)) ?? (await mine()).find(s => s.ref === ref);
    const was = cur ? shape(cur) : null;
    if (!was) throw new Error("that subtitle is not yours here");
    const meta = { lang: lang ?? was.lang, label: label ?? was.label };
    const files_ = text != null ? [await store(was.in, toVtt(text), `${meta.label || "subtitles"}.vtt`)] : null;
    await items.editItem(ref, meta.label, { meta, ...(files_ ? { files: files_ } : {}) });
  }
  const remove = ref => items.remove(ref);
  const text = async sub => toVtt(await (await files.get(sub.file)).text());
  async function mine() {
    const me = (await space.account()).id;
    return (await items.list({ by: me }, "new", KIND)).map(shape);
  }
  return { of, add, update, remove, text, toSrt, toVtt, mine, KIND };
}
