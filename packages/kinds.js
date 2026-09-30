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
export async function start() {
  const CATALOG = [
    // Video
    ["video", "video", "Video", []],
    ["movie", "video", "Movie", ["year", "director", "cast", "genre"]],
    ["episode", "video", "TV episode", ["show", "season", "episode", "year"]],
    ["music-video", "video", "Music video", ["artist", "album", "year"]],
    ["short", "video", "Short", []],
    // Audio
    ["music", "audio", "Music", ["artist", "album", "year", "genre"]],
    ["podcast", "audio", "Podcast", ["show", "episode", "host"]],
    ["audiobook", "audio", "Audiobook", ["author", "narrator", "chapter"]],
    // Image
    ["photo", "image", "Photo", ["location", "taken", "camera"]],
    ["artwork", "image", "Artwork", ["artist", "medium"]],
    // Document
    ["book", "document", "Book", ["author", "publisher", "year", "isbn"]],
    ["comic", "document", "Comic", ["writer", "artist", "issue", "publisher"]],
    // File
    ["asset", "file", "Asset", ["format", "software"]],
    ["game", "file", "Game", ["platform", "genre", "version"]],
    ["software", "file", "Software", ["platform", "version", "license"]],
    ["dataset", "file", "Dataset", ["format", "schema"]],
    // Text
    ["post", "text", "Post", []],
  ];
  const byKind = new Map(CATALOG.map(([kind, domain, label, fields]) => [kind, Object.freeze({ kind, domain, label, fields: Object.freeze(fields) })]));
  const LABELS = { year: "Year", director: "Director", cast: "Cast", genre: "Genre", show: "Show", season: "Season", episode: "Episode", artist: "Artist", album: "Album", host: "Host", author: "Author", narrator: "Narrator", chapter: "Chapter", location: "Location", taken: "Taken", camera: "Camera", medium: "Medium", publisher: "Publisher", isbn: "ISBN", writer: "Writer", issue: "Issue", format: "Format", software: "Software", platform: "Platform", version: "Version", license: "License", schema: "Schema" };
  return {
    of: kind => byKind.get(kind) ?? null,
    domain: kind => byKind.get(kind)?.domain ?? null,
    inDomain: domain => CATALOG.filter(c => c[1] === domain).map(c => c[0]),
    all: () => [...byKind.keys()],
    fieldLabel: f => LABELS[f] ?? f,
  };
}
