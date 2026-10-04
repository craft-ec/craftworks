// MEDIA, a page: ONE page for every MEDIA app — a LENS on a content DOMAIN (`kinds`) chosen by its route: VIDEOS
// (`#/video`: video, movie, TV episode, music video, short), AUDIO (`#/audio`: music, podcast, audiobook) and IMAGES
// (`#/image`: image, photo, artwork — a wall of pictures), YouTube-, Spotify- and Flickr-shaped; a composed app shows several the same way. Its SUB-TYPES filter the list. CHANNEL-FIRST: yours
// (each public, or only you) and those you follow (`…`: the feed; `…/mine`; `…/c/<did>`: someone's); SAVED (the pin
// edge); DISCOVER. In a SPACE (`#/s/<space>/…`): its members' items, public while its domain reads in public. WATCH/
// LISTEN: `…/p/<ref>` (its page: `item-page`) — streamed by byte range (`video-player`), a like (▲), comments, and its timed text
// (`caption-store`: Subtitles on a video, Lyrics on a song, a Transcript on a podcast — shown in time for audio).
// UPLOAD: `…/up` (tags and cover from the file). UI only.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [items, directory, person, theme, space, roles, drive, kinds, edge] = await Promise.all(["items", "directory", "person", "theme", "space", "roles", "drive-store", "kinds", "edge"].map(n => ctx.require(n)));
  const people = await edge.people();
  // WHICH APP this page is (its route): its domain, its words.
  const APPS = {
    "/video": { app: "video", domain: "video", icon: "▶️", name: "Video", one: "video", ones: "videos", accept: "video/*", mine: "Your channel", audio: false },
    "/audio": { app: "audio", domain: "audio", icon: "🎧", name: "Audio", one: "track", ones: "tracks", accept: "audio/*", mine: "Your library", audio: true },
    "/image": { app: "image", domain: "image", icon: "🖼", name: "Images", one: "image", ones: "images", accept: "image/*", mine: "Your photostream", picture: true },
  };
  const C_ROUTE = APPS[ctx.route] ? ctx.route : "/video";
  const C = APPS[C_ROUTE];
  const VIDEO = kinds.inDomain(C.domain);
  let only = null; // a SUB-TYPE the list is filtered to (null: all)
  // The FEED BAR (shared by every content app, as Grid's): the feed, its window, and a sort of what is shown.
  const bar = (await ctx.require("feed-bar")).create({ start: "new", onChange: () => draw() });
  const me = (await space.account()).id;
  el.innerHTML = `
    <style>
      .vd { display: grid; gap: var(--cw-space-3); }
      .vd .top { display: flex; align-items: center; gap: var(--cw-space-2); flex-wrap: wrap; }
      .vd .top h2 { margin: 0; font-size: 1.3rem; }
      .vd .up { background: var(--cw-accent); color: var(--cw-accent-fg); border-radius: var(--cw-radius-sm); padding: 6px 12px; text-decoration: none; }
      .vd .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: var(--cw-space-4) var(--cw-space-3); }
      .vd .t { font-weight: 600; line-height: 1.3; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
      .vd .s { color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .vd .by { cursor: pointer; }
      .vd .by:hover { color: var(--cw-fg); }
      .vd .none { color: var(--cw-muted); text-align: center; padding: var(--cw-space-5); }
      .vd .row { display: flex; gap: var(--cw-space-3); align-items: center; flex-wrap: wrap; }
      .vd button { font: inherit; cursor: pointer; border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); border-radius: 999px; padding: 5px 14px; }
      .vd button.on { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
      .vd form { display: grid; gap: var(--cw-space-2); max-width: 640px; }
      .vd input, .vd textarea, .vd select { font: inherit; padding: 6px 8px; border-radius: var(--cw-radius-sm); border: 1px solid var(--cw-line); background: var(--cw-surface); color: var(--cw-fg); }
      .vd .said { color: var(--cw-danger); margin: 0; }
      .vd .chips { flex-basis: 100%; display: flex; gap: 6px; flex-wrap: wrap; }
      .vd .chips button { padding: 3px 12px; font-size: var(--cw-text-sm); }
      .vd .grid.sq { grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); }
      .vd .wall { columns: 240px; column-gap: var(--cw-space-2); }
      .vd .wall > * { break-inside: avoid; margin-bottom: var(--cw-space-2); }
      .vd .subs { display: grid; gap: 6px; border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); }
      .vd .subs .row input[name=label] { width: 12em; }
      .vd .subs .row input[name=lang] { width: 4em; }
    </style>
    <div class="vd"></div>`;
  const root = el.querySelector(".vd");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  // WHERE it is (`where`: the one reading of an address — yours, a space's, a person's, Discover), read at each draw.
  const where = await ctx.require("where");
  let at = await where.of({ kinds: VIDEO, app: C.app, yours: "Following" });
  const base = () => at.base;
  const route = () => {
    const s = at.sub;
    if (s.startsWith("p/")) return { watch: decodeURIComponent(s.slice(2)) };
    if (s === "up") return { up: true };
    if (at.who === "person") return { by: at.person };
    if (at.who === "discover") return { discover: true };
    if (at.who === "space") return { board: at.space.id };
    return s === "mine" ? { by: me } : s === "saved" ? { saved: true } : { feed: true };
  };
  // A public space's video seen from OUTSIDE (Discover, a followed space): its public description.
  const outsideOf = async ref => {
    if (!ref.startsWith("space:")) return null;
    const id = ref.slice(6, ref.indexOf("/"));
    if ((await space.mine()).some(x => x.id === id)) return null;
    return (await items.publicSpaces().catch(() => [])).find(d => d.id === id) ?? people.about("follow", id);
  };
  // One wording of time everywhere (`cards`).
  const who = did => {
    const n = directory.nameEl(did, "span", { className: "by", onclick: e => (e.preventDefault(), e.stopPropagation(), person.open(e.currentTarget, did)) });
    return n;
  };

  function top(w) {
    // THE TABS (`where`'s, in every app's order): Following · yours · Saved · Discover; Upload where you may.
    at.tabs([], {
      yoursOn: !!w.feed && !w.saved,
      // Yours first, as every app's: your channel, then the feed of those you follow.
      before: [{ label: C.mine, href: `#/${C.app}/mine`, on: w.by === me }],
      create: { label: "Upload", href: at.space ? `${base()}/up` : `#/${C.app}/up`, on: !!w.up },
    });
    // In a space, it is a CHANNEL (the space named in this app's own words).
    const title = at.space ? `${C.icon} ${space.shown(at.space)} · channel` : `${C.icon} ${C.name}`;
    // The SUB-TYPES (a filter): all, or one kind of the domain.
    const chips = w.watch || w.up ? null : h("div", { className: "chips" }, ...[null, ...VIDEO].map(k => h("button", { type: "button", className: only === k ? "on" : "", textContent: k ? kinds.of(k).label : "All", onclick: () => ((only = k), draw()) })));
    const feedBar = w.watch || w.up || w.saved ? null : h("div", { className: "chips" }, bar.el());
    return h("div", { className: "top" }, h("h2", { textContent: title }), chips, feedBar);
  }

  // A video's or a track's CARD: its kind's look (`cards`), opened here (in Discover: from outside).
  const cards = await ctx.require("cards");
  const card = v => (C.picture ? cards.picture : cards.card)(v, { href: `${base()}/p/${v.ref}` });

  async function list(w) {
    // SAVED: what this person saved (`actions`: any kind, one key) — of this app's kinds.
    // SAVED: `where`'s (what you saved, of this app's kinds).
    const vs = w.saved
      ? await at.read()
      : bar.reorder(await items.list(w, bar.sort(), VIDEO, bar.options()));
    const shown = only ? vs.filter(v => v.kind === only) : vs;
    const when = bar.span();
    const none = w.saved ? `Nothing saved: “Save” on a ${C.one} keeps it here.` : w.feed ? `No ${C.ones} ${when} from you or what you follow.` : w.discover ? `No public ${C.ones} ${when}.` : w.by === me ? `Nothing ${when}: upload a ${C.one}.` : `No ${C.ones} ${when}.`;
    const older = w.saved ? null : bar.older("Older", shown.length);
    return shown.length ? h("div", {}, h("div", { className: C.picture ? "wall" : `grid${C.audio ? " sq" : ""}` }, ...shown.map(card)), older) : h("div", {}, h("p", { className: "none", textContent: none }), older);
  }

  // ITS PAGE: the one item page (`item-page`) — its player, likes, comments — framed here.
  async function watch(ref) {
    return (await ctx.require("item-page")).show(ref, { outside: await outsideOf(ref), app: C.app, back: base(), discover: at.who === "discover" });
  }

  async function upload() {
    const f = await (await ctx.require("publisher")).form({ domain: C.domain, space: at.space, app: C.app, onPublished: ({ item }) => (location.hash = `${base()}/p/${item}`) });
    return h("div", {}, h("h3", { textContent: `Upload ${/^[aeiou]/.test(C.one) ? "an" : "a"} ${C.one}` }), f);
  }

  let drawn = "";
  async function draw() {
    at = await where.of({ kinds: VIDEO, app: C.app, yours: "Following" });
    const w = route();
    drawn = where.key();
    root.replaceChildren(top(w), theme.loading(w.watch ? `Opening the ${C.one}…` : `Reading the ${C.ones}…`));
    const body = await (w.watch ? watch(w.watch) : w.up ? upload() : list(w)).catch(e => h("p", { className: "said", textContent: e.message ?? String(e) }));
    if (drawn === where.key()) root.replaceChildren(top(w), ...(w.by && w.by !== me && !w.watch ? [h("div", { className: "row" }, h("h3", {}, who(w.by)), h("button", { type: "button", textContent: "Follow…", onclick: e => person.open(e.currentTarget, w.by) }))] : []), body);
  }
  await draw();
  items.onChange(() => el.isConnected && !route().watch && !route().up && draw());
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === C_ROUTE && where.key() !== drawn && draw());
}
