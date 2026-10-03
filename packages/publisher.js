// PUBLISHER, a capability: the ONE way a new FILE becomes an ITEM of its kind — from its app's upload page (Videos,
// Audio, Images) or uploaded inline in an editor (a post, a comment): the same path either way. Its MAKER (`kinds`:
// a video's or an audio's renditions, an image's thumbnail) makes it; its own TAGS name it (title; artist, album, year,
// genre where its kind has those fields); a COVER chosen replaces its own; an item of its kind is made with whom it is
// for (`audience`: so policy, Discover and feeds treat it as they treat its kind); its SUBTITLES and the LYRICS it
// carries become its timed text. A file taken from Drive is never published again: it is an item already.
//
//   const publisher = await ctx.require("publisher");
//   const { item, ref } = await publisher.publish(file, { space, audience, kind, title, body, meta, cover, subtitles,
//                                                         keepOriginal, app, onProgress })
export async function start(ctx) {
  const [kinds, items] = await Promise.all(["kinds", "items"].map(n => ctx.require(n)));

  async function publish(file, { space = null, audience = space ? "members" : "private", kind = null, title = "", body = "", meta = {}, cover = null, subtitles = [], keepOriginal = false, app = null, onProgress = () => {} } = {}) {
    const spec = kinds.mediaOf(file.type);
    if (!spec) throw new Error(`${file.name}: not a kind that is published (an image, a video, an audio)`);
    kind ??= spec.kind;
    const maker = await ctx.require(spec.maker);
    const ref = await maker.make(file, { space, public: audience === "public", app: app ?? spec.domain, keepOriginal, onProgress });
    // A COVER chosen: over the file's own.
    if (cover) ref.preview = (await (await ctx.require("image-studio")).thumbnail(cover)) ?? ref.preview;
    // Its own TAGS: the fields its kind has, unless given.
    const tags = ref.tags ?? {};
    const fields = Object.fromEntries((kinds.of(kind)?.fields ?? []).map(f => [f, tags[f]]).filter(([, v]) => v != null && v !== ""));
    const m = { ...fields, ...Object.fromEntries(Object.entries(meta).filter(([, v]) => v != null && v !== "")) };
    // A video or an audio sent as it is (not encoded here) carries its video id on the item (a manifest has its own).
    if (spec.maker === "video-studio" && ref.type !== maker.MANIFEST && ref.key) m.vid = await maker.videoId(ref.key);
    const name = String(title || tags.title || file.name.replace(/\.[^.]+$/, "")).trim().slice(0, 300);
    const item = await items.submit({ board: space?.id ?? null, title: name, body, kind, meta: m, audience, files: [ref] });
    // Its TIMED TEXT: the subtitles given, else the lyrics the file carries (untimed: one cue over the whole).
    if (spec.maker === "video-studio") {
      const subs = await ctx.require("subtitle-store");
      for (const s of subtitles) await subs.add(item, s).catch(e => ctx.log("publish", { what: `timed text ${s.name}: ${e.message}` }));
      const lyrics = !subtitles.length && spec.domain === "audio" ? (await maker.probe(file).catch(() => null))?.lyrics : null;
      if (lyrics)
        await subs
          .add(item, `WEBVTT\n\n00:00:00.000 --> ${new Date(Math.max(1, ref.duration || 3600) * 1000).toISOString().slice(11, 23)}\n${lyrics.trim()}\n`, { label: kinds.attachLabel("subtitle", kind) })
          .catch(() => {});
    }
    return { item, ref };
  }
  return { publish };
}
