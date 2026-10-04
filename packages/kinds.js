// KINDS, a capability: what a piece of CONTENT is — the one catalog every app reads (after handcraft's content
// architecture). Four layers:
// - DOMAIN: video, audio, image, document, file, text — DERIVED from the kind, never stored.
// - KIND (the atomic type): what the item IS — a Movie, a Podcast, a Post — each with its own fields (`meta`). A
//   video tutorial is a `video` tagged "Tutorial", not a kind of its own: kinds do not overlap.
// - CONTEXT: tags (genre, theme, purpose) — the `tags` capability, free to grow.
// - BUNDLE: a collection (a series, an album, a course) — later, its own item naming its members.
// An APP is a lens on a domain: Videos shows the video domain, Board the text domain (posts).
//
//   const kinds = await ctx.require("kinds");
//   kinds.of("movie")        // { kind, domain: "video", label: "Movie", fields: ["year", …] } or null
//   kinds.domain("short")    // "video"
//   kinds.inDomain("video")  // ["video", "movie", "episode", "music-video", "short"]
//   kinds.all()              // every kind that stands on its own (a comment answers one; a message is a conversation's)
//   kinds.attaching()        // kinds that ATTACH to another item (`in`): a subtitle to a video or an audio item
//   kinds.attachesTo("caption")   // the domains it attaches to: ["video", "audio"]
//   kinds.attachLabel("caption", "music")   // "Lyrics" ("Transcript" on a podcast, "Subtitles" on a video)
//   kinds.parts("movie")     // its PAGE's parts (`item-page`): { look: "player"|"card", votes, comments, about } — what
//                            // shows around the item, declared once per domain, never per app
//   kinds.policyDomain("movie") // the DOMAIN whose settings govern it in a space ("video"; a post: "text"; a subtitle:
//                             // "subtitle") — content decides, not the app showing it (one app may show every domain)
export async function start() {
  const CATALOG = [
    // Video
    ["video", "video", "Video", []],
    ["movie", "video", "Movie", ["year", "director", "cast", "genre"]],
    ["episode", "video", "TV episode", ["show", "season", "episode", "year"]],
    ["music-video", "video", "Music video", ["artist", "album", "year"]],
    ["short", "video", "Short", []],
    // Audio
    ["audio", "audio", "Audio", []],
    ["music", "audio", "Music", ["artist", "album", "year", "genre"]],
    ["podcast", "audio", "Podcast", ["show", "episode", "host"]],
    ["audiobook", "audio", "Audiobook", ["author", "narrator", "chapter"]],
    // Image
    ["image", "image", "Image", []],
    ["photo", "image", "Photo", ["location", "taken", "camera"]],
    ["artwork", "image", "Artwork", ["artist", "medium"]],
    // Document
    ["book", "document", "Book", ["author", "publisher", "year", "isbn"]],
    ["comic", "document", "Comic", ["writer", "artist", "issue", "publisher"]],
    // File (Drive: what was uploaded, any type — `file`, the general one)
    ["file", "file", "File", []],
    ["folder", "file", "Folder", []],
    ["asset", "file", "Asset", ["format", "software"]],
    ["game", "file", "Game", ["platform", "genre", "version"]],
    ["software", "file", "Software", ["platform", "version", "license"]],
    ["dataset", "file", "Dataset", ["format", "schema"]],
    // Text
    ["post", "text", "Post", []],
    // Note (Notes: a card of text, its own domain — its own policy)
    ["note", "note", "Note", []],
    // Channel (Chat: a place for messages — its title its name; `meta.cid` names its messages' table)
    ["channel", "chat", "Channel", []],
  ];
  // COLLABORATIVE kinds: in a space, whoever its `edit` policy allows edits one (a shared note, a shared file's
  // entry), the creator kept; every other kind its author's alone (an app's own enforcement over the policy).
  // UNTITLED kinds: a title is optional.
  const COLLABORATIVE = new Set(["note", "file", "folder"]);
  // THE MEDIA DOMAINS — each declared ONCE: which files are it (`is`: by type; a streaming manifest by its own flag),
  // its name and icon, the capability that MAKES an upload of it (`maker`), the component that SHOWS it inline
  // (`view`: "image" drawn as an <img>; otherwise a component with `create({ file, item, cover }).el`), its general
  // KIND (what a new upload of it is published as, unless one is chosen: `publisher`). The editor's
  // menu, every upload (`attachments`), the inline renderer (`markdown`) and Drive's folders read it — a new domain
  // (books, comics) is one entry here, its maker and its viewer.
  const MANIFEST = "application/vnd.craftworks.video+json";
  const MEDIA = [
    { domain: "image", label: "Image", plural: "Images", icon: "🖼", accept: "image/*", is: t => /^image\//.test(t), maker: "image-studio", view: "image", kind: "image" },
    { domain: "video", label: "Video", plural: "Videos", icon: "🎬", accept: "video/*", is: (t, ref) => /^video\//.test(t) || (t === MANIFEST && !ref?.audio), maker: "video-studio", view: "media-view", kind: "video" },
    { domain: "audio", label: "Audio", plural: "Audio", icon: "🎵", accept: "audio/*", is: (t, ref) => /^audio\//.test(t) || (t === MANIFEST && !!ref?.audio), maker: "video-studio", view: "media-view", kind: "audio" },
  ];
  const mediaOf = x => {
    const ref = typeof x === "string" ? null : x;
    const t = String((typeof x === "string" ? x : x?.type) ?? "").toLowerCase();
    return MEDIA.find(m => m.is(t, ref)) ?? null;
  };
  const UNTITLED = new Set(["note", "file"]);
  // ATTACHING kinds: their own data, about another item (`in`) — contributed like a comment, listed with what they are
  // about, and a lens of their own (the Caption app). A CAPTION — subtitles, lyrics, a transcript: WebVTT (its file),
  // its language and label.
  const ATTACHING = [["caption", "text", "Caption", ["lang", "label"], ["video", "audio"]]];
  const byKind = new Map(CATALOG.map(([kind, domain, label, fields]) => [kind, Object.freeze({ kind, domain, label, fields: Object.freeze(fields) })]));
  const onTo = new Map(ATTACHING.map(([kind, domain, label, fields, to]) => [kind, Object.freeze({ kind, domain, label, fields: Object.freeze(fields), to: Object.freeze(to), attaches: true })]));
  const LABELS = { lang: "Language", label: "Label",  year: "Year", director: "Director", cast: "Cast", genre: "Genre", show: "Show", season: "Season", episode: "Episode", artist: "Artist", album: "Album", host: "Host", author: "Author", narrator: "Narrator", chapter: "Chapter", location: "Location", taken: "Taken", camera: "Camera", medium: "Medium", publisher: "Publisher", isbn: "ISBN", writer: "Writer", issue: "Issue", format: "Format", software: "Software", platform: "Platform", version: "Version", license: "License", schema: "Schema" };
  // A PAGE's PARTS by domain (`item-page`): its look whole — the PLAYER (video, audio), the PICTURE (an image) or its
  // card's look — and what
  // shows around it. Composed here, never an app's own page.
  const PARTS = {
    video: { look: "player", votes: true, comments: true, about: true },
    audio: { look: "player", votes: true, comments: true, about: true },
    text: { look: "card", votes: true, comments: true, about: false },
    image: { look: "picture", votes: true, comments: true, about: true },
    note: { look: "card", votes: false, comments: true, about: false },
    file: { look: "card", votes: false, comments: true, about: true },
    document: { look: "card", votes: false, comments: true, about: true },
  };
  return {
    parts: kind => PARTS[byKind.get(kind)?.domain] ?? { look: "card", votes: false, comments: false, about: false },
    of: kind => byKind.get(kind) ?? onTo.get(kind) ?? null,
    attaching: () => [...onTo.keys()],
    attachesTo: kind => [...(onTo.get(kind)?.to ?? [])],
    domain: kind => byKind.get(kind)?.domain ?? null,
    inDomain: domain => CATALOG.filter(c => c[1] === domain).map(c => c[0]),
    all: () => [...byKind.keys()],
    fieldLabel: f => LABELS[f] ?? f,
    collaborative: kind => COLLABORATIVE.has(kind),
    titled: kind => !UNTITLED.has(kind),
    // The media domains (`MEDIA`), and the one a file (a reference, or a type) is — or null.
    media: () => MEDIA,
    mediaOf,
    // A FILE's domain, from its type (MIME): what an upload IS, whichever app it came through.
    ofType: type => mediaOf(type)?.domain ?? (/^text\/|pdf|epub|msword|officedocument|opendocument|rtf|comicbook/.test(String(type ?? "").toLowerCase()) ? "document" : "file"),
    // A domain's NAME as a place (a Drive folder; its app's): Videos, Audio, Images, Documents, Files.
    domainName: d => MEDIA.find(m => m.domain === d)?.plural ?? ({ document: "Documents" })[d] ?? "Files",
    policyDomain: kind => (onTo.has(kind) ? kind : byKind.get(kind)?.domain ?? "text"),
    // What an attaching kind is CALLED on an item of `target` kind: a caption on a song is its Lyrics, on a podcast
    // or an audiobook its Transcript — one capability, named for what it is on.
    attachLabel: (kind, target) =>
      kind !== "caption" ? (onTo.get(kind)?.label ?? kind) : ["music", "music-video"].includes(target) ? "Lyrics" : ["podcast", "audiobook"].includes(target) ? "Transcript" : "Subtitles",
  };
}
