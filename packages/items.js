// ITEMS, a capability: every item that STANDS ON ITS OWN — of any kind in the catalog (`kinds`), siblings: a `post`
// (text), a `video`, a `movie` … — with what attaches to it (COMMENTS, and attaching kinds: a `subtitle`) and VOTES,
// in one of two PLACES. Apps are lenses on it by domain (Board: posts; Video: the video kinds; Caption: captions).
// (Once Board's alone, hence `board` in its places' names and API: a space's place is its "board room".)
// - A BOARD: a SPACE's (`space.board(sp)`), one of its apps (added by its owner or admins) beside its messages — a
//   server's, a group or direct conversation's — the same members, roles, governance and moderation. Its posts
//   are `content` in the space's table `board`: members write, moderators hide, what is hidden is left out, and a
//   person removed or banned there writes no more. Joined by joining the space (an invite, a welcome).
// - A person's PROFILE: `content` in their own public tail `posts` (only their account writes it; anyone reads it) —
//   what their FOLLOWERS see. A comment or a vote on someone's profile post is in the commenter's own tail, with a
//   pointer (`{ from }`) in the post's public bag (`index`) so the post's readers find it.
// A post is `content` of kind "post" (`title`); a VIDEO the same, of kind "video" (its file, with its poster and
// duration, in `files`: the Videos app lists these, Board the posts); a comment of kind "comment" (`re`: what it
// answers, `in`: its post); a vote a reaction ▲ or ▼. REFS: `space:<server id>/<id>` on a board, `<author did>/<id>` on
// a profile.
//
//   const items = await ctx.require("items");   (Board names it `posts`: its lens)
//   await posts.submit({ board, title, body, kind })  // `board`: a space's id, or none (this person's profile): its ref
//                                                     // (`kind`: "post", or "video")
//   await items.feed(source, { kinds, sort, window, tag })   // THE ONE FEED: { home } { space } { person } { yours }
//                                                     // { following } { friends } { saved } { shared } { feed }
//   await items.list({ board } | { by } | { feed }, sort, kind, { window })   // (`kind`: a kind, or kinds — a domain's;
//                                                     // `window`: "day" | "week" | "month" | days | "all" — what is read)
//                                                     // a space's board, a profile, or the FEED (the PERSONAL
//                                                     // view: the boards of every space this person is in, their own
//                                                     // profile, those they follow and their friends'); sort "hot" | "new" | "top"
//   await posts.get(ref)   await posts.thread(ref)    // one post; its comments as a tree (`replies`), best first
//   await posts.comment(post, re, body, { files })   await posts.vote(ref, 1 | -1 | 0, post)   await posts.remove(ref)
//   await posts.syncPublic()      // this person's items on each board, public exactly while the board reads in public
//   await posts.boards()                             // the spaces this person is in: their boards
//   await posts.boardOf(ref | id)                    // the space a board post or board id is in (null: a profile's)
//   posts.onChange(fn)
// A post: { ref, id, by, title, body, board, at, edited, comments, score, mine, mayRemove }; a comment: the same
// without title and board, with `replies`.
// AN ITEM's PURE PARTS — what a list or a card asks at once, never a read: its tags, where its page is, its link, the
// item a text names. One implementation: `start` uses them, and an app's frame has the same (`frame`, FOUNDATION §3).
const NSFW = "nsfw";
const spaceIdOf = x => (String(x).startsWith("space:") ? String(x).slice(6, String(x).indexOf("/")) : null);
const tagsOf = it => (Array.isArray(it?.meta?.tags) ? it.meta.tags : []);
const normalTags = text =>
  [...new Set(String(text ?? "").split(/[,\s]+/).map(t => t.replace(/^#/, "").toLowerCase().replace(/[^\p{L}\p{N}_-]/gu, "").slice(0, 40)).filter(Boolean))].slice(0, 20);
const isNsfw = it => tagsOf(it).includes(NSFW);
// (`site()`: this site's address, before its `#`.)
function pure(kinds, site) {
  const APP_OF = { video: "video", audio: "audio", image: "image", book: "book", note: "note", file: "drive", document: "drive", form: "form", wiki: "wiki", vote: "vote", doc: "docs", sheet: "sheets", slides: "slides", bundle: "bundle" };
  // A kind with an app of its own (a paste: Paste, though of the note family) first; else its domain's.
  const APP_OF_KIND = { paste: "paste" };
  const appOf = kind => APP_OF_KIND[kind] ?? APP_OF[kinds.domain(kind)] ?? "board";
  // In its PLACE (`where`'s addresses): a space's in the space, a person's in their space (`u/<did>`: yours too).
  // AN ITEM'S REFERENCE: a space's (`space:<id>/<item>`) or a person's (`did:craftec:<id>/<item>`).
  const ITEM_REF = /^(space:[0-9a-f]{64}|did:craftec:[1-9A-HJ-NP-Za-km-z]{20,64})\/[A-Za-z0-9_-]{4,80}$/;
  function pageOf(ref, kind) {
    const app = appOf(kind);
    const sp = String(ref).startsWith("space:") ? ref.slice(6, ref.indexOf("/")) : null;
    const base = sp ? `#/s/${sp}/${app}` : `#/${app}/u/${String(ref).slice(0, String(ref).lastIndexOf("/"))}`;
    // THE ONE PAGE (`item-page`), framed by its app: `…/<app>/p/<ref>`.
    return `${base}/p/${ref}`;
  }
  // ITS LINK, to share: the address this page is served at — a node's (`/v1/contract/web/<site>/`) or a server's own
  // domain in front of one — with its page (`pageOf`). Opened in a browser it is the item; pasted anywhere in the app,
  // from ANY address, it opens here (`markdown`): what names the item is its page, never the host.
  const linkOf = (ref, kind) => `${site()}${pageOf(ref, kind)}`;
  // The item a text NAMES: its reference as it is, or any link to its page (`…#/<app>/…/p/<ref>`, whatever the host).
  function refOf(text) {
    const t = String(text ?? "").trim();
    if (ITEM_REF.test(t)) return t;
    const m = /^https?:\/\/\S*#\/\S*?\/p\/(\S+)$/.exec(t);
    const ref = m ? decodeURIComponent(m[1]) : null;
    return ref && ITEM_REF.test(ref) ? ref : null;
  }
  return { appOf, pageOf, linkOf, refOf };
}
// IN AN APP's FRAME: the pure parts beside the host's proxy (the site's address: the host's, `ctx.site`).
export async function frame(cap, ctx) {
  return { ...pure(await ctx.require("kinds"), () => ctx.site), spaceIdOf, tagsOf, normalTags, isNsfw };
}

// AN ITEM's REFERENCE — the one builder: `<writer>/<id>` (its writer's DID or node, its id in their feed). (Not
// `refOf`: that reads a reference out of a link's text — the two shared one name in v399, and the link reader shadowed
// this inside `start`: every profile item's reference came out wrong, so Discover and the media feeds were empty.)
export const itemRef = (by, id) => `${by}/${id}`;

export async function start(ctx) {
  const [content, index, space, edge] = await Promise.all(["content", "index", "space", "edge"].map(n => ctx.require(n)));
  const TAIL = "posts";
  // SIGNALS (`signals`): every interaction with an item — a vote, a view, a save, a share, a tag — kept as a reaction
  // where its giver's vote is, counted, windowed and sealed by the one path below.
  const signals = await ctx.require("signals");
  // What stands on its own (a post, a video, a movie …: `kinds`): comments and votes answer these.
  const kinds = await ctx.require("kinds");
  const TOP = new Set(kinds.all());
  // What a list shows: a kind, or KINDS (a domain's: `kinds.inDomain("video")` — videos, movies, episodes …).
  const kindsFor = k => (Array.isArray(k) ? k : [k]);
  const domainOf = k => kinds.policyDomain(k);
  // DISCOVER's bags as this page has read them (`index`): what is listed there already — so a pointer is dropped once.
  const listedIn = new Map(); // `domain|month` → Promise<Set of refs>
  const indexed = (domain, at) => {
    const k = `${domain}|${index.monthOf(at)}`;
    return ctx.cache(listedIn, k, () => index.discoverPointers.strict(domain, [index.monthOf(at)]).then(ps => new Set(ps.map(p => p?.ref))));
  };
  const answeredIn = new Map(); // an item's ref → Promise<Set of writer keys pointed in its bag>
  const answerers = ref => {
    return ctx.cache(answeredIn, ref, () => index.pointers.strict(ref).then(ps => new Set(ps.map(p => p?.w).filter(Boolean))), () => new Set());
  };
  // A PUBLIC item LISTED: in Discover's bag of its domain and month (a top item), its own bag made; an answer to one (a
  // comment, a vote, a subtitle) pointed in that item's bag. Once each: what the bags hold already is not dropped again.
  async function listPublic(ref, x, where) {
    if (TOP.has(x.kind)) {
      const set = await indexed(domainOf(x.kind), x.at);
      if (set.has(ref)) return;
      await index.discoverPoint(domainOf(x.kind), x.at, { ref, kind: x.kind, at: x.at, ...where });
      set.add(ref);
      await ctx.background("items: index.openPointers", index.openPointers(ref));
      return;
    }
    if (!where.w) return;
    const set = await answerers(ref);
    if (set.has(where.w)) return;
    await index.point(ref, where);
    set.add(where.w);
  }
  const changed = [];
  const fire = () => changed.forEach(f => f());
  // A bag answered (a space's outsiders): what reads it, drawn again.
  index.onChange?.(fire);
  const me = async () => space.me.strict();
  // Whose profiles this person reads: whom they follow, and their friends (friends need no follow as well).
  // A follow names a SPACE: a person's DID (their personal space) or a shared space's id.
  const following = async () => {
    const p = await edge.people();
    return [...new Set([...p.list("follow"), ...p.list("friend")])].filter(id => id.startsWith("did:"));
  };
  // The SHARED spaces followed (not joined): each read from outside, as Discover reads a public one.
  const followedSpaces = async () => {
    const p = await edge.people();
    const inside = new Set((await space.mine.strict()).map(s => s.id));
    return p
      .list("follow")
      .filter(id => !id.startsWith("did:") && !inside.has(id))
      .map(id => p.about("follow", id))
      .filter(d => d?.id && d.governance?.owner);
  };

  // ROOMS, opened once per page: a board's, a person's profile tail.
  const rooms = new Map();
  const opened = (key, make) => {
    if (!rooms.has(key)) {
      const p = make().then(r => (r.onChange(fire), r));
      ctx.optional("items: p", p, () => rooms.delete(key));
      rooms.set(key, p);
    }
    return rooms.get(key);
  };
  // The spaces this person is in that have a Board (an app of the space, added by its owner or admins: `roles`).
  const roles = await ctx.require("roles");
  // A space's board room is its place for every item `posts` keeps — posts, videos, captions — so any app on it opens
  // the place (a space with Video and no Board still takes videos).
  const PLACE_APPS = ["board", "video", "audio", "caption", "note", "drive", "image", "book", "chat", "form", "blog", "wiki", "vote", "docs", "sheets", "slides"];
  const boards = async () => {
    const all = await space.mine.strict();
    // Its apps once its acts are read (before, a space shows the default apps: a Chat-only space would open a board).
    // A space whose rules do not read here is not a board here — said, never dropped silently.
    const on = await Promise.all(all.map(sp => roles.of.strict(sp).then(async r => (await r.settled, r.apps().some(a => PLACE_APPS.includes(a))), e => (ctx.failed("posts", e, `${sp.name ?? sp.id.slice(0, 8)}: its rules do not read (${e?.message ?? e})`), false))));
    return all.filter((_, i) => on[i]);
  };
  // The SPACE ID an item reference names (`space:<id>/…`), null for a person's item.
  const boardOf = async x => {
    const id = spaceIdOf(x) ?? x;
    return (await boards()).find(s => s.id === id) ?? null;
  };
  // WHY a space is not a place here — said, never a bare "not in it": not among this person's spaces, or its apps show
  // none that keeps items.
  const notInBoard = async x => {
    const id = spaceIdOf(x) ?? x;
    const sp = (await space.mine.strict()).find(s => s.id === id);
    if (!sp) return new Error("you are not in that board's space");
    const r = await roles.of.strict(sp);
    await r.settled;
    return new Error(`${space.shown(sp)} shows ${r.apps().join(", ") || "no apps"}: none keeps items here`);
  };
  // A BOARD's room: its two tables as one — the SEALED board (members read) and the PUBLIC board (anyone reads). What
  // is public follows the board's setting NOW, not when something was written: each person's items (posts, comments,
  // votes) live in the sealed table, and while the board reads in public their copy is in the public table too — put
  // there by their own page (`sync`: only an author's own feed can carry their items, so the author's page copies
  // them), taken out again when the board goes private. Outside (`desc`: not a member): the public board alone.
  // A public copy's files carry the key they have NOW (a file re-keyed: the copy follows; readers outside the space read
  // the reference alone).
  async function currentKeys(v) {
    let it;
    try {
      it = JSON.parse(v);
    } catch {
      return v;
    }
    if (!Array.isArray(it?.files) || !it.files.length) return v;
    const files = await ctx.require("files");
    return JSON.stringify({ ...it, files: await Promise.all(it.files.map(f => ctx.optional("items: files.current", files.current(f), () => f))) });
  }
  // A DOMAIN's read setting in a space: its own (`text`, `video`, `caption` …: content decides, whatever app shows it),
  // else the space's.
  const domainReads = (r, domain) => r.readsAnyone(domain);
  // Whether an item of `kind` is public in a space now.
  async function publicIn(sp, kind) {
    const r = await roles.of.strict(sp);
    await r.settled;
    return domainReads(r, kinds.policyDomain(kind));
  }
  // (A space's PLACE: its board room — the same for every kind.)
  const boardRoom = sp =>
    opened(`space:${sp.id}`, async () => {
      // PAGED (phase 3): what a list shows is read by its TIME WINDOW; a thread opened reads the place whole.
      const t0 = performance.now();
      const took = (what, p) => p.then(v => ((t[what] = Math.round(performance.now() - t0)), v));
      const t = {};
      const [a, b, r] = await Promise.all([took("board", content.in(space.board(sp), { paged: true })), took("public board", content.in(space.board(sp, { pub: true }), { paged: true, extra: outsideSource(sp) })), took("roles", roles.of.strict(sp))]);
      if (performance.now() - t0 > 3000) ctx.log("posts", { what: `${sp.name}: its board opened slowly — ${Object.entries(t).map(([k, v]) => `${k} ${v} ms`).join(", ")}` });
      // PUBLIC is per item, by its own APP's setting now (a video: Videos'; a post: Board's) — what attaches to an item
      // (a comment, a vote, a subtitle) as that item is. Not bound to Board: a space's place holds every kind.
      const readsAnyone = domain => domainReads(r, domain);
      const rootsHere = new Map(); // item id → { kind, aud, in } (every author's, as listed)
      const learnKinds = () => [...a.list(), ...b.list()].forEach(x => rootsHere.set(x.id, { kind: x.kind, aud: x.aud ?? null, in: x.in ?? null }));
      // WHAT ATTACHES TO AN ITEM FOLLOWS THAT ITEM (ARCHITECTURE §7): a comment, a vote, a section, a ballot is public
      // exactly when the ROOT item it is about, here, is — the root's own kind's domain AND the root's own audience (10-21:
      // the row's own `aud`, never set on a comment, made comments on a members-only item public). An item kept about
      // something ELSEWHERE (`in` a full reference: a team's subtitle) is its own root. A root not read yet: not public
      // yet (the next sync, once it is read, decides) — never guessed.
      const rootOf = (x, depth = 0) => {
        const about = TOP.has(x.kind) ? null : x.in ?? x.item ?? null;
        if (!about || String(about).includes("/")) return x;
        const up = rootsHere.get(about);
        if (!up || depth >= 8) return null;
        return rootOf(up, depth + 1);
      };
      const isPublic = x => {
        const root = rootOf(x);
        if (!root || root.aud === "members" || x.aud === "members") return false;
        return readsAnyone(kinds.of(root.kind) ? kinds.policyDomain(root.kind) : "text");
      };
      const pubOfValue = v => {
        try {
          return isPublic({ kind: "post", ...JSON.parse(v) });
        } catch {
          return false;
        }
      };
      const pubOfItem = it => isPublic(it);
      // THE SYNC: this person's rows — a public copy of each while ITS APP reads in public, none while it does not.
      let syncing = null;
      const sync = () =>
        (syncing ??= (async () => {
          await Promise.all([a.settled, b.settled]);
          // This person's own rows, whole, in both tables (the others' stay paged).
          await Promise.all([a.ownAll?.(), b.ownAll?.()]);
          const inA = new Map(a.own().map(x => [x.key, x.value]));
          const inB = new Map(b.own().map(x => [x.key, x.value]));
          learnKinds();
          for (const [k, v0] of inA) {
            if (pubOfValue(v0)) {
              const v = await currentKeys(v0);
              if (inB.get(k) !== v) await b.putOwn(k, v);
            } else if (inB.has(k)) await b.dropOwn(k);
          }
          // TAKEN BACK (a vote, a save, a reaction undone; a row deleted): its public copy goes too — the public table
          // mirrors this person's rows, never keeps one they no longer have (a save undone still counted, seen).
          for (const k of inB.keys()) if (!inA.has(k)) await b.dropOwn(k);
          // DISCOVER: each public row listed where readers find it (this device's public board of the space).
          const where = { w: sp.self, t: space.board(sp, { pub: true }).messages, sp: { id: sp.id, name: sp.name } };
          for (const [k, v] of inA) {
            if (!pubOfValue(v)) continue;
            let x;
            try {
              x = JSON.parse(v);
            } catch {
              continue;
            }
            const about = TOP.has(x.kind) ? k : x.kind === "reaction" ? x.item : x.in;
            if (!about || !x.at || x.meta?.unlisted) continue;
            await listPublic(`space:${sp.id}/${about}`, x, where).catch(e => ctx.failed("posts", e, `listing ${k} in Discover: ${e?.message ?? e}`));
          }
        })().finally(() => (syncing = null)));
      const dedupe = (list, key) => {
        const seen = new Set();
        return list.filter(x => !seen.has(key(x)) && seen.add(key(x)));
      };
      const rooms = [a, b];
      return {
        list: () => (learnKinds(), dedupe([...a.list(), ...b.list()], x => x.id).map(x => ({ ...x, pub: pubOfItem(x) }))),
        // A vote in both tables is one vote.
        reactions: () => dedupe([...a.reactions(), ...b.reactions()], x => `${x.item}|${x.emoji}|${x.by}`),
        mayRemove: it => a.mayRemove(it),
        mayEdit: it => a.mayEdit(it),
        mayAbout: (id, action) => a.mayAbout(id, action),
        mayPost: () => a.mayPost(),
        since: ms => Promise.all([a.since?.(ms), b.since?.(ms)]),
        loadAll: () => Promise.all([a.loadAll?.(), b.loadAll?.()]),
        onChange: f => (a.onChange(f), b.onChange(f)),
        settled: Promise.all([a.settled, b.settled]),
        sync,
        post: async (kind, body, opts = {}) => {
          await Promise.all([a.settled, b.settled]);
          const id = await a.post(kind, body, opts);
          await sync();
          return id;
        },
        react: async (id, e, on, o) => (await Promise.all([a.settled, b.settled]), await sync(), await a.react(id, e, on, o), await sync()),
        edit: async (id, body) => (await sync(), await a.edit(id, body), await sync()),
        setFiles: async (id, files) => (await sync(), await a.setFiles(id, files), await sync()),
        editFull: async (id, body, o) => (await sync(), await a.edit(id, body, o), await sync()),
        history: id => a.history(id),
        // Out of both tables (an author's own; a moderator's hide, in each table it is listed in).
        remove: async id => {
          for (const x of rooms) if (x.list().some(it => it.id === id)) await x.remove(id);
        },
      };
    });
  // DISCOVER's POINTERS for these kinds since `since` (ms): only the months shown, only the items listed.
  async function discovered(kinds_, since) {
    const domains = [...new Set(kinds_.map(domainOf))];
    const months = index.monthsSince(since);
    const ps = (await Promise.all(domains.map(d => index.discoverPointers(d, months)))).flat();
    const seen = new Set();
    return ps.filter(p => p?.ref && kinds_.includes(p.kind) && (p.at ?? 0) >= since && !seen.has(p.ref) && seen.add(p.ref));
  }
  // A PUBLIC SPACE read as its pointers name it — never every member: the room over exactly the writers of these items
  // and those who answered them (each item's own bag), and only those the space's roles count.
  async function pointedRoom(desc, ptrs) {
    const answered = (await Promise.all(ptrs.map(p => answerers(p.ref)))).flatMap(s => [...s]);
    const writers = [...new Set([...ptrs.map(p => p.w).filter(Boolean), ...answered])];
    const b = await content.in({ ...space.board(desc, { outside: true }), writers }, { extra: outsideSource(desc) });
    return { ...b, list: () => b.list().map(x => ({ ...x, pub: true })), mayRemove: () => false };
  }
  // The pointer of ONE item seen from outside: its month from its id's time (`t` ‖ ms base 36), its domain's bags.
  async function pointerOf(ref) {
    const m = /\/t([0-9a-z]{9})[0-9a-f]{8}$/.exec(ref);
    if (!m) return null;
    const at = parseInt(m[1], 36);
    for (const d of new Set([...TOP].map(domainOf))) {
      const p = (await index.discoverPointers(d, [index.monthOf(at)])).find(x => x?.ref === ref);
      if (p) return p;
    }
    return null;
  }
  // A public space's ROOM from outside: as its pointers name it (`ptrs`), or — none given — every member (`roles`).
  const outsideRoom = desc =>
    opened(`outside:${desc.id}`, () => content.in(space.board(desc, { outside: true }), { extra: outsideSource(desc) }).then(b => ({ ...b, list: () => b.list().map(x => ({ ...x, pub: true })), mayRemove: () => false })));
  // A PROFILE's room: their public tail. YOURS also holds your PRIVATE posts (per post: "only you"), sealed in your
  // account's table `journal` — with their comments and votes, so nothing of them reaches the public tail.
  const JOURNAL = "journal";
  const profileRoom = did =>
    opened(did, async () => {
      const pub = await content.in({ kind: "public", did, name: TAIL });
      const acc = await space.account.strict();
      // Another's (or anyone's, for a guest): its public posts.
      if (!acc || did !== acc.id) return pub;
      // Its PRIVATE posts (`journal`) opened in the background: the public ones never wait on them; what needs them
      // (a private post, an item of theirs) waits for them.
      let priv = null;
      const privReady = content.in({ kind: "journal", messages: JOURNAL, scope: acc }).then(r => (priv = r));
      const changedHere = [];
      ctx.background("items: privReady.then", privReady.then(r => (r.onChange(() => changedHere.forEach(f => f())), changedHere.forEach(f => f()))));
      const isPrivate = id => !!priv?.list().some(x => x.id === id);
      const roomOf = async id => (await ctx.optional("items: privReady", privReady, () => null), isPrivate(id) ? priv : pub);
      const idIn = ref => String(ref ?? "").slice(String(ref ?? "").lastIndexOf("/") + 1);
      return {
        list: () => [...pub.list(), ...(priv?.list() ?? []).map(x => ({ ...x, private: true }))],
        reactions: () => [...pub.reactions(), ...(priv?.reactions() ?? [])],
        mayRemove: it => it.by === acc.id,
        mayEdit: it => it.by === acc.id,
        onChange: f => (pub.onChange(f), changedHere.push(f)),
        settled: pub.settled,
        isPrivate: ref => isPrivate(idIn(ref)),
        // Its private posts read (before deciding whether something about one is private).
        whenPrivate: () => ctx.background("items: privReady", privReady),
        post: async (kind, body, opts = {}) => (TOP.has(kind) ? (opts.private ? await privReady : pub) : await roomOf(idIn(opts.in))).post(kind, body, opts),
        react: async (item, e, on, o) => (await roomOf(idIn(item))).react(item, e, on, o),
        remove: async id => (await roomOf(id)).remove(id),
        edit: async (id, body) => (await roomOf(id)).edit(id, body),
        setFiles: async (id, files) => (await roomOf(id)).setFiles(id, files),
        editFull: async (id, body, o) => (await roomOf(id)).edit(id, body, o),
        history: async id => (await roomOf(id)).history(id),
      };
    });
  // Each read bounded where it is read (`storage`): a silent profile shows as held, never waited on.
  const profiles = async dids =>
    (await Promise.all([...new Set(dids)].map(d => ctx.optional("items: profileRoom", profileRoom(d), () => null)))).filter(Boolean);
  const pointersTo = async ref => (await index.pointers(ref)).map(p => p.from).filter(d => typeof d === "string" && d.startsWith("did:craftec:"));
  const idOf = ref => ref.slice(ref.lastIndexOf("/") + 1);
  const whereOf = ref => ref.slice(0, ref.lastIndexOf("/"));

  // SIGNALS COUNTED, from reactions — within a WINDOW when one is given (`since`, ms: a list's counts match its window,
  // as Grid's did). VOTES: ref → (did → 1 | -1) (both at once — a change half-made — counts as neither); every other
  // signal beside them (`votes.counts`: ref → signal → (did → its values)).
  function tally(reactions, votes = new Map(), since = 0) {
    votes.counts ??= new Map();
    for (const x of reactions) {
      const s = signals.byMark(x.emoji);
      if (!s || (since && (x.at ?? Infinity) < since)) continue;
      if (s.value === "updown") {
        const on = votes.get(x.item) ?? new Map();
        on.set(x.by, on.has(x.by) ? 0 : signals.valueOf(x.emoji));
        votes.set(x.item, on);
        continue;
      }
      const of = votes.counts.get(x.item) ?? new Map();
      const by = of.get(s.id) ?? new Map();
      by.set(x.by, (by.get(x.by) ?? new Set()).add(signals.valueOf(x.emoji)));
      of.set(s.id, by);
      votes.counts.set(x.item, of);
    }
    return votes;
  }
  // An item's counts: its votes (`score`, `ups`, `downs`, `mine`), and `counts` — per signal, how many people gave it
  // (a many-signal: per value) — and `signaled`: this person's own (a save's state, the tags they put).
  const scored = (key, votes, self) => {
    const on = votes.get(key);
    const vs = on ? [...on.values()] : [];
    const counts = {};
    const signaled = {};
    for (const [id, by] of votes.counts?.get(key) ?? []) {
      const many = signals.of(id)?.value === "many";
      // A SUM: each person's latest (their largest: a replacement half-made counts once), added up.
      if (signals.of(id)?.value === "sum") {
        const latest = values => Math.max(0, ...[...values].map(Number));
        counts[id] = [...by.values()].reduce((n, v) => n + latest(v), 0);
        if (by.has(self)) signaled[id] = latest(by.get(self));
        continue;
      }
      if (many) {
        const t = {};
        for (const values of by.values()) for (const v of values) t[v] = (t[v] ?? 0) + 1;
        counts[id] = t;
      } else counts[id] = by.size;
      if (by.has(self)) signaled[id] = many ? [...by.get(self)] : true;
    }
    return { score: vs.reduce((n, v) => n + v, 0), ups: vs.filter(v => v > 0).length, downs: vs.filter(v => v < 0).length, mine: on?.get(self) ?? 0, counts, signaled };
  };
  // Who may give a signal, by its policy action: a READ one (a view, a save, a share) whoever reads it — they read it.
  const signalPasses = (emoji, check) => {
    const s = signals.byMark(emoji);
    return !!s && (s.action === "read" || check(s.action));
  };
  const postOf = c => c.in; // a comment's post
  const shape = (it, ref, board) => {
    return { ref, id: it.id, kind: it.kind, in: it.in ?? null, by: it.by, editor: it.editor ?? null, aud: it.aud ?? null, title: it.title ?? "", body: it.body, board, at: it.at, edited: it.edited, private: !!it.private, files: it.files ?? [], meta: it.meta ?? {} };
  };

  // A BOARD's posts: everything is in its one room (reactions keyed by the item's id).
  // WINDOWS a list is bounded by (phase 3: Top of today, of the week, of the month; Hot the week; New the month and
  // older on asking): what is read is that span of the place, never all of it. `all`: the place read whole.
  const WINDOW = { day: 86400e3, week: 7 * 86400e3, month: 30 * 86400e3 };
  const sinceOf = w => (w === "all" || w == null ? null : Date.now() - (typeof w === "number" ? w * 86400e3 : (WINDOW[w] ?? WINDOW.month)));
  // No bound here: every read under a list is bounded where it is read (`storage`: shown within WAIT.hint, merged
  // when it answers — the lists drawn again on `onChange`).
  const { WAIT } = await ctx.require("node");
  const boardPosts = (sp, opts = {}) => boardPostsOf(sp, opts);
  async function boardPostsOf(sp, { outside = false, kinds = ["post"], window = "all", room = null } = {}) {
    const r = room ?? (await (outside ? outsideRoom(sp) : boardRoom(sp)));
    // `window: "held"`: what is read already (a caller read its own span: one item and what came after it).
    const since = window === "held" ? 0 : (sinceOf(window) ?? 0);
    if (!outside && window !== "held") {
      if (!since) await r.loadAll?.();
      else await r.since?.(since);
    }
    const self = await me();
    const items = r.list();
    const votes = tally(r.reactions(), new Map(), since);
    const counts = new Map();
    for (const it of items) if (it.kind === "comment" && it.at >= since) counts.set(postOf(it), (counts.get(postOf(it)) ?? 0) + 1);
    return items
      .filter(it => kinds.includes(it.kind))
      .map(it => ({ ...shape(it, `space:${sp.id}/${it.id}`, { id: sp.id, name: sp.name }), pub: it.pub, comments: counts.get(it.id) ?? 0, ...scored(it.id, votes, self), mayRemove: r.mayRemove(it), mayEdit: !!r.mayEdit?.(it) }));
  }
  // OUTSIDERS' PART in a space (public participation): comments and votes by people not in it — kept in their own
  // profiles (only they write there), each pointed ONCE in the space's bag (`space:<id>`). Counted where the space's
  // policy lets them (`roles.mayWrite`, the one check: "anyone") and its acts have not banned them. Members' are its room's.
  // `room`: the room's items as read; returns the outsiders' comments (`post`: the room post's id) and reactions (`item`:
  // a room item's id, or an outsider's comment's ref).
  const spaceBag = sp => `space:${sp.id}`;
  // A SPACE's OUTSIDERS' items, for its public board room (`content`'s `extra`: one more source beside its members'
  // feeds — the room's one filter then applies: its policy, bans, hides). Each from its writer's own PROFILE (only they
  // write there: theirs, whatever it claims), named in the room by `~<did>~<id>`; what it answers named as the room does.
  const sources = new Map();
  function outsideSource(sp) {
    if (sources.has(sp.id)) return sources.get(sp.id);
    const base = `space:${sp.id}/`;
    const local = ref => {
      const s = String(ref ?? "");
      if (s.startsWith(base)) return s.slice(base.length);
      const m = /^(did:craftec:[^/]+)\/(.+)$/.exec(s);
      return m ? `~${m[1]}~${m[2]}` : s;
    };
    let rooms = [];
    const fns = [];
    const fire = () => fns.forEach(f => f());
    const seen = new Set();
    let loads = 0;
    const load = async () => {
      const dids = (await index.pointers(spaceBag(sp), { show: true })).map(p => p?.from).filter(d => typeof d === "string" && d.startsWith("did:craftec:"));
      ctx.log("posts", { what: `${sp.name ?? sp.id.slice(0, 8)}: outsiders read (${++loads}) — ${dids.length} writer(s)` });
      rooms = await profiles(dids);
      for (const r of rooms) if (!seen.has(r)) seen.add(r), r.onChange(fire);
      fire();
    };
    load();
    index.onChange?.(() => ctx.background("items: load", load()));
    // A writer new since: read when the node says the bag changed.
    ctx.background("items: index.watch", index.watch(spaceBag(sp), () => ctx.background("items: load", load())));
    const src = {
      items: () =>
        rooms.flatMap(r => [
          ...r
            .list()
            .filter(it => !it.private && (it.meta?.space === sp.id || String(it.in ?? "").startsWith(base)))
            .map(it => ({ ...it, id: `~${it.by}~${it.id}`, in: it.in ? local(it.in) : null, re: it.re ? local(it.re) : null, outsider: true })),
          ...r
            .reactions()
            .filter(x => String(x.item).startsWith(base))
            .map(x => ({ kind: "reaction", id: `~${x.by}~r~${x.item}~${x.emoji}`, item: local(x.item), emoji: x.emoji, by: x.by, at: x.at, ...(x.cred ? { meta: { cred: x.cred } } : {}), outsider: true })),
        ]),
      onChange: f => fns.push(f),
    };
    sources.set(sp.id, src);
    return src;
  }
  // This person's part in a space they are not in: pointed in its bag, once per page.
  async function pointToSpace(sp) {
    const self = await me();
    if (pointed.has(spaceBag(sp))) return;
    await index.point(spaceBag(sp), { from: self });
    pointed.add(spaceBag(sp));
  }

  // PROFILE posts: from their authors' tails; comments and votes from the tails known here (the reader's, whom they
  // follow, and whoever `readers` names).
  // `withVotes: false`: the authors' profiles alone (a lens that shows no votes or comment counts: Notes, Drive).
  async function profilePosts(authors, readers = [], kinds = ["post"], since = 0, { withVotes = true } = {}) {
    const self = await me();
    // Every profile opened ONCE (each bounded): the authors' among them — never a second wait on a slow one.
    let rs = await profiles(withVotes ? [...authors, self, ...(await following()), ...readers] : authors);
    const authorSet = new Set(authors);
    // EVERYONE WHO ANSWERED the posts listed (their comments, votes and replies live in their own profiles, pointed in
    // each post's bag — as its thread reads them): opened too, so a count is the thread's, not only the followed's.
    if (withVotes) {
      const listed = rs.flatMap(r => r.list().filter(it => kinds.includes(it.kind) && authorSet.has(it.by) && !it.meta?.space && it.at >= since).map(it => itemRef(it.by, it.id)));
      const held = new Set([...authors, self, ...(await following()), ...readers]);
      const more = [...new Set((await Promise.all(listed.map(ref => pointersTo(ref)))).flat())].filter(d => !held.has(d));
      if (more.length) rs = [...rs, ...(await profiles(more))];
    }
    const votes = new Map();
    const counts = new Map();
    // What counts about an item: by its rule (`roles.mayWrite`: the one check, as in a space) — as it was when written.
    const posts = new Map(rs.flatMap(r => r.list().filter(it => TOP.has(it.kind)).map(it => [itemRef(it.by, it.id), it])));
    const counts_ = (ref, action, by, cred, at) => {
      const p = posts.get(ref);
      return !p || roles.mayWrite({ action, item: p, writer: by, cred, at: at ?? Infinity }) === true;
    };
    for (const r of rs) {
      tally(r.reactions().filter(x => signalPasses(x.emoji, action => counts_(x.item, action, x.by, x.cred, x.at))), votes, since);
      for (const it of r.list()) if (it.kind === "comment" && it.at >= since && counts_(postOf(it), "comment", it.by, it.meta?.cred, it.at)) counts.set(postOf(it), (counts.get(postOf(it)) ?? 0) + 1);
    }
    const out = [];
    for (const r of rs)
      for (const it of r.list()) {
        // One made in a SPACE (an outsider's post there) is that space's: listed in its room, not here.
        if (!kinds.includes(it.kind) || !authorSet.has(it.by) || it.meta?.space) continue;
        const ref = itemRef(it.by, it.id);
        out.push({ ...shape(it, ref, null), comments: counts.get(ref) ?? 0, ...scored(ref, votes, self), mayRemove: it.by === self, mayEdit: it.by === self });
      }
    return out;
  }

  // THE RANKS (Grid's), each over its window's counts: HOT — activity (every vote, up or down, and comments); BEST —
  // quality (net votes and comments); RISING — interactions per hour of age; TOP — net votes (or, `by: "comments"`,
  // comments); NEW — newest. Ties go to the newer.
  // ACTIVITY: every vote, comment and signal the catalog counts as activity (`signals`: a save, a share, a tag).
  const activity = p =>
    (p.ups ?? 0) + (p.downs ?? 0) + (p.comments ?? 0) + signals.all().filter(s => s.activity && s.value !== "updown").reduce((n, s) => n + (typeof p.counts?.[s.id] === "object" ? Object.values(p.counts[s.id]).reduce((a, b) => a + b, 0) : (p.counts?.[s.id] ?? 0)), 0);
  const RANK = {
    hot: activity,
    best: p => (p.score ?? 0) + (p.comments ?? 0),
    rising: p => activity(p) / Math.max(1, (Date.now() - p.at) / 3600e3),
    // POPULAR: how many people viewed it (over the window).
    popular: p => p.counts?.view ?? 0,
    top: p => p.score ?? 0,
    comments: p => p.comments ?? 0,
  };
  const sorter = (sort, by) => (sort === "new" ? (a, b) => b.at - a.at : (a, b) => RANK[sort === "top" && by === "comments" ? "comments" : sort in RANK ? sort : "hot"](b) - RANK[sort === "top" && by === "comments" ? "comments" : sort in RANK ? sort : "hot"](a) || b.at - a.at);

  // ONE public space's description (as Discover lists it), or null.
  const publicSpace = async id => (await ctx.optional("items: publicSpaces", publicSpaces(), () => [])).find(d => d.id === id) ?? null;
  // An item SEEN FROM OUTSIDE: its space's public description when this person is not in it (listed, else followed),
  // null when they are (or it is a person's) — what `get`, `list`, votes and comments are given as `outside`.
  async function outsideOf(ref) {
    ref = String(ref?.ref ?? ref);
    const id = spaceIdOf(ref);
    if (!id) return null;
    if (await boardOf(id)) return null;
    return (await publicSpace(id)) ?? (await edge.people()).about("follow", id);
  }
  // DISCOVER: the public spaces listed (their descriptions), each proved by its id (its owner), one per id.
  // LEFTOVERS (FOUNDATION.md §5): a listing whose space does not OPEN — its public acts unread, or none of them counts
  // (a space under keys nobody holds any more) — is left out, never shown as a space that opens nothing; how many,
  // reported once a page (`leftover`).
  // (Unread is left out NOW, never remembered: R4 — asked again, it is read again.)
  // OPENS: its public acts read (its roles from outside — `roles.ofPublic`, kept by roles and LIVE: an author learned
  // later, a departed writer's record arriving after its acts, counts as it comes) and any counted. Asked each time,
  // never kept: a "no" decided before its departed record arrived stayed "no" for the page (fx 10-16, a space whose
  // rules a dropped device wrote).
  const opens = async d => {
    const pr = await (await ctx.require("roles")).ofPublic(d);
    const st = (await pr.decided?.()) ?? pr.state;
    if (st?.s === "unknown") throw new Error(`${d.id.slice(0, 8)}: its public acts not read yet`);
    return pr.acts().length > 0;
  };
  let leftoverSaid = false;
  async function publicSpaces() {
    const seen = new Map();
    const lists = await ctx.optional("items: lists", (await ctx.require("moderation")).lists(), () => null);
    const candidates = [];
    for (const d of await index.spaces()) {
      if (!d?.id || seen.has(d.id) || !d.governance?.owner) continue;
      if ((await space.owner(d)) !== d.governance.owner) continue;
      if (lists?.flagged({ space: d.id, by: d.governance.owner })) continue;
      seen.set(d.id, { id: d.id, name: String(d.name ?? "").slice(0, 100), kind: "server", governance: d.governance });
      candidates.push(seen.get(d.id));
    }
    const ok = await Promise.all(candidates.map(d => ctx.optional("items: opens", opens(d), () => false)));
    const shown = candidates.filter((_, i) => ok[i]);
    const hidden = candidates.length - shown.length;
    if (hidden && !leftoverSaid) {
      leftoverSaid = true;
      ctx.background("items: index.health", index.health({ k: "leftover", s: "spaces", why: `${hidden} of ${candidates.length} listed space(s) do not open: left out of Discover`, d: { hidden, of: candidates.length } }));
    }
    return shown;
  }

  // POSTS AS DISCOVER's BAGS LIST THEM: each public space's (only listed spaces, not flagged) read from its pointed
  // writers, each person's profile items from their own public tail. `space`: one space's only.
  async function pointedPosts(kinds_, since, { space: only = null } = {}) {
    const ps = await discovered(kinds_, since);
    const spaces = new Map((only ? [only] : await publicSpaces()).map(d => [d.id, d]));
    const bySpace = new Map();
    const dids = new Set();
    for (const p of ps) {
      if (p.sp?.id && spaces.has(p.sp.id)) (bySpace.get(p.sp.id) ?? bySpace.set(p.sp.id, []).get(p.sp.id)).push(p);
      else if (!only && typeof p.did === "string" && p.did.startsWith("did:craftec:") && p.ref.startsWith(`${p.did}/`)) dids.add(p.did);
    }
    const listedRefs = new Set(ps.map(p => p.ref));
    const [inSpaces, profile] = await Promise.all([
      Promise.all(
        [...bySpace].map(async ([id, ptrs]) =>
          // A space that does not read: SAID (never a silent empty list).
          (async () => boardPostsOf(spaces.get(id), { outside: true, kinds: kinds_, room: await pointedRoom(spaces.get(id), ptrs) }))().catch(e => (ctx.failed("posts", e, `Discover: ${spaces.get(id).name ?? id.slice(0, 8)} not read: ${e?.message ?? e}`), [])),
        ),
      ).then(x => x.flat()),
      dids.size ? ctx.optional("items: profilePosts", profilePosts([...dids], [], kinds_, since), () => []) : [],
    ]);
    // What a pointer claims is checked: only items there, of what was listed.
    ctx.log("posts", { what: `Discover: ${ps.length} pointer(s) — ${[...bySpace].map(([id, x]) => `${spaces.get(id).name ?? id.slice(0, 8)}: ${x.length}`).join(", ") || "no space"}; ${dids.size} profile(s); read ${inSpaces.length} space item(s), ${profile.length} profile item(s)` });
    return [...inSpaces, ...profile].filter(p => listedRefs.has(p.ref));
  }

  // WINDOW, bounded unless asked for "all": New the last 30 days (older on asking), a rank the last week.
  // TAGS: an item's own (its author's: `meta.tags`, lowercase) — what it is, for everyone; `nsfw` marks adult content,
  // shown only to whoever chose to see it (`edge.prefs` "nsfw"), and always to its author.
  const nsfwShown = async () => !!(await edge.prefs()).get("nsfw");
  // What a list shows of what it read: its TAG (`where.tag`), and NSFW only where chosen.
  async function shownOf(out, where) {
    const self = await me();
    const nsfw = await nsfwShown();
    // A tag its author put, or others did (the tag signal).
    return out.filter(p => (!where.tag || tagsOf(p).includes(where.tag) || !!p.counts?.tag?.[where.tag]) && (nsfw || p.by === self || !isNsfw(p)));
  }
  // THE ONE FEED (owner 10-20: "all app and capability need to use the single feed for all"): a feed is a history of
  // items composed from its SOURCE — what it covers, nothing else; reading, what is shown (the moderation lists this
  // person applies, NSFW, a tag), the window and the order are this one pipeline for every feed of every app.
  //   { home }              the global public items (what public spaces and people list)
  //   { space: id | desc }  a space's: its tables where this person is a member (its audiences' too), else its public
  //   { person: did }       a person's personal space (yours: your private items too)
  //   { yours }             your items: your personal space's, and what you wrote in the spaces you are in
  //   { following }         who and what you follow (people's personal spaces, spaces' public)
  //   { friends }           your friends' personal spaces
  //   { saved }             what you saved     { shared }   what is shared with you (`share`'s groups)
  //   { feed }              your spaces, whom you follow, your friends, yourself (a person's home stream)
  // (Before: each place its own path, and the moderation lists applied to Discover alone — a post hidden still showed
  // in a space; a space's public post on its board and not in the public feed, 10-20.)
  async function fromSource(src, kinds, since, window) {
    // (A board read within the feed's window: what a list shows is that span of the place, never all of it.)
    const boardsOf = sps => Promise.all(sps.map(sp => ctx.optional("items: boardPosts", boardPosts(sp, { kinds, window }), () => []))).then(x => x.flat());
    if (src.home) return pointedPosts(kinds, since);
    if (src.space) {
      const id = typeof src.space === "string" ? src.space : src.space.id;
      const sp = (await space.mine.strict()).find(x => x.id === id);
      if (sp && !src.outside) return boardsOf([sp, ...(await audienceGroups(sp))]);
      const desc = typeof src.space === "object" ? src.space : await publicSpace(id);
      if (!desc) throw new Error("you are not in that space, and it lists nothing in public");
      return pointedPosts(kinds, since, { space: desc });
    }
    if (src.person) return profilePosts([src.person], [], kinds, since);
    const self = await me();
    const people = await edge.people();
    if (src.yours) return [...(await profilePosts([self], [], kinds, since)), ...(await boardsOf(await boards())).filter(p => p.by === self)];
    if (src.following) {
      const dids = people.list("follow").filter(d => d.startsWith("did:"));
      return [...(await profilePosts(dids, [], kinds, since)), ...(await Promise.all((await followedSpaces()).map(d => ctx.optional("items: pointedPosts", pointedPosts(kinds, since, { space: d }), () => [])))).flat()];
    }
    if (src.friends) return profilePosts(people.list("friend").filter(d => d.startsWith("did:")), [], kinds, since);
    if (src.saved) {
      const refs = await (await ctx.require("actions")).saved();
      return (await Promise.all(refs.map(async ref => ctx.optional("items: saved", get(ref, { outside: await outsideOf(ref) }), () => null)))).filter(Boolean);
    }
    if (src.shared) return boardsOf(await (await ctx.require("share")).groups());
    if (src.feed) {
      const [bs, ps, fs] = await Promise.all([boards(), following(), followedSpaces()]);
      return (
        await Promise.all([
          boardsOf(bs),
          Promise.all(fs.map(d => ctx.optional("items: pointedPosts", pointedPosts(kinds, since, { space: d }), () => []))).then(x => x.flat()),
          profilePosts([self, ...ps], [], kinds, since),
        ])
      ).flat();
    }
    throw new Error(`not a feed's source: ${JSON.stringify(src).slice(0, 80)}`);
  }
  async function feed(src = { home: true }, { kinds: kind = "post", sort = "hot", window = sort === "new" ? 30 : "week", by = "votes", tag = null } = {}) {
    const kinds = kindsFor(kind);
    const since = sinceOf(window) ?? 0;
    const t0 = performance.now();
    const read = await fromSource(src, kinds, since, window);
    if (performance.now() - t0 > WAIT.hint) ctx.log("posts", { what: `the feed ${Object.keys(src).join("+")}: read slowly`, ms: Math.round(performance.now() - t0) });
    // ONE PIPELINE: each item once, in its window, as the moderation lists this person applies leave it, NSFW and a tag.
    const lists = await (await ctx.require("moderation")).lists();
    const once = new Set();
    const kept = read.filter(p => p && !once.has(p.ref) && once.add(p.ref) && (since === 0 || p.at >= since) && !lists.flagged({ by: p.by, id: p.id, ref: p.ref, space: p.board?.id }));
    return (await shownOf(kept, { tag })).sort(sorter(sort, by));
  }
  // A LIST, as it was asked before the one feed (`where`, every app): the same feed, its source named the old way.
  function list(where = {}, sort = "hot", kind = "post", opts = {}) {
    const src = where.outside ? { space: where.outside, outside: true } : where.discover ? { home: true } : where.board ? { space: where.board } : where.feed ? { feed: true } : { person: where.by ?? null };
    return (src.person === null ? me().then(by => feed({ person: by }, { kinds: kind, sort, tag: where.tag, ...opts })) : feed(src, { kinds: kind, sort, tag: where.tag, ...opts }));
  }

  async function get(ref, { outside = null } = {}) {
    if (ref.startsWith("space:")) {
      // ONE item of a space: read where this person is a member (whatever apps the space shows — its acts may not be
      // read yet on a page just opened), or from outside.
      const sid = ref.slice(6, ref.indexOf("/"));
      const sp = outside ?? (await space.mine.strict()).find(s => s.id === sid) ?? null;
      if (!sp) return null;
      // ONE item: its place read from its time on (it, its votes and comments) — never the place whole.
      if (!outside) await sinceItem(await boardRoom(sp), idOf(ref));
      // From outside: read where its pointer names (its writer and who answered it), never every member.
      const p = outside ? await pointerOf(ref) : null;
      const room = p ? await pointedRoom(outside, [p]) : null;
      const find = async () => (await boardPosts(sp, { outside: !!outside, kinds: [...TOP], window: outside ? "all" : "held", room })).find(x => x.ref === ref) ?? null;
      const it = await find();
      if (it || outside) return it;
      // NOT FOUND YET is not "not there" (R4: nothing from a short read): its place's writers each tried once
      // (`settled` — a writer's feed is found as its space is read: 10-20, a shared document opened at once read as
      // gone), then looked for again.
      const r = await boardRoom(sp);
      await r.settled;
      // …and its writers are found as the place is read (a shared item's maker's feed: seconds), so it is looked for
      // until it shows or 15 s have passed — then "not there".
      for (const end = Date.now() + 15000; Date.now() < end; await new Promise(x => setTimeout(x, 1000))) {
        await sinceItem(r, idOf(ref));
        const got = await find();
        if (got) return got;
      }
      return null;
    }
    // One of THIS person's own: their private items read first (a private note, a post "only you" is among them).
    if (whereOf(ref) === (await me())) await (await profileRoom(whereOf(ref))).whenPrivate?.();
    return (await profilePosts([whereOf(ref)], await pointersTo(ref), [...TOP])).find(p => p.ref === ref) ?? null;
  }

  // WHERE AN ITEM GOES for who is to see it (the one routing: `submit`, and an item moved — `share.relocate`): the
  // place (a space, a group sealed to its audience, a circle, or yours: null) and its audience there ("public",
  // "members", "private"). `place`: that place's description, where it is a space.
  async function route({ board = null, audience = "public", kind = "post", title = "", meta = {} }) {
    // ONLY YOU, IN A SPACE: a group of its own (`share.privateIn`: its maker alone, kept to the space) — shared later.
    if (board && audience === "private") {
      board = (await (await ctx.require("share")).privateIn(board, kind, title)).id;
      audience = "members";
    }
    // YOUR FRIENDS or YOUR FOLLOWERS: an item of that CIRCLE's place (`circles`: sealed to its members).
    if (!board && (audience === "friends" || audience === "followers")) {
      board = (await (await ctx.require("circles")).of(audience)).id;
      audience = "members";
    }
    // A SPACE's AUDIENCE narrower than its members (a role's holders, the admins, the owner): an item of that
    // audience's GROUP (`conversation.audience`: sealed to them alone), whatever its kind.
    if (board && (String(audience).startsWith("role:") || String(audience).startsWith("list:") || audience === "admins" || audience === "owner")) {
      const parent = await boardOf(board);
      if (!parent) throw await notInBoard(board);
      board = (await (await ctx.require("conversation")).audience(parent, audience)).id;
      meta = { ...meta, space: parent.id, aud: audience };
      audience = "members";
    }
    return { board, audience, meta, place: board ? await boardOf(board) : null };
  }
  // WHERE AN ITEM IS, and WHO READS IT — the one answer (ARCHITECTURE §7, "What attaches to an item follows that
  // item"): everything kept FOR an item (a document's changes, its files, what is attached) is kept where this says,
  // never by a test of its own (10-21: codoc, files, captions, comments each decided it themselves — and disagreed).
  //   place     the space it is in (a group too), or null: a person's own
  //   outside   read from outside that space (not a member: its public side only)
  //   public    anyone reads it — a space's item by its room's one rule (`pub`), a person's by not being private
  //   table(n)  the CONTAINER of the item's own table `n`, beside it (`content.table`): a space's members table or its
  //             public one (`pub-…`: anyone reads, its members write); a person's public tail, or their own account's
  // A SIBLING TABLE's NAME, made to fit (a table's name is 1 to 32 bytes; a space's own prefix `x<12>-` takes 14): a
  // name too long is its stable short hash — its `pub-` kept in front (what makes a space's table public: storage).
  const fitName = (prefix, n, room) => {
    if (prefix.length + n.length <= room) return prefix + n;
    let h1 = 0x811c9dc5, h2 = 0x01000193;
    for (const c of n) (h1 = Math.imul(h1 ^ c.charCodeAt(0), 16777619) >>> 0), (h2 = Math.imul(h2 ^ c.charCodeAt(0), 2246822519) >>> 0);
    return (prefix + "h" + h1.toString(36) + h2.toString(36)).slice(0, room);
  };
  async function placeOf(ref) {
    const it = typeof ref === "string" ? await get(ref, { outside: await outsideOf(ref) }) : ref;
    if (!it) throw Object.assign(new Error("that item is not here"), { kind: "absent" });
    if (String(it.ref).startsWith("space:")) {
      const sid = it.ref.slice(6, it.ref.indexOf("/"));
      const sp = (await space.mine.strict()).find(s => s.id === sid) ?? null;
      if (sp) {
        const pub = !!it.pub;
        // (A members table keeps its plain name, as tables kept for items always had; a public one is the space's `pub-`
        // table, the form storage makes public.)
        return { place: sp, outside: false, public: pub, table: n => ({ messages: pub ? space.tableOf(sp, fitName("pub-", n, 18)) : n, scope: sp }) };
      }
      const desc = await outsideOf(it.ref);
      if (!desc) throw Object.assign(new Error("you are not in that space, and it lists nothing in public"), { kind: "refused-expected" });
      return { place: desc, outside: true, public: true, table: n => ({ outside: true, messages: space.tableOf(desc, fitName("pub-", n, 18)), scope: desc }) };
    }
    const did = whereOf(it.ref);
    const pub = !it.private;
    const acc = await space.account.strict();
    return { place: null, outside: false, public: pub, did, table: n => (pub ? { kind: "public", did, name: fitName("", n, 32) } : { messages: fitName("", n, 32), scope: acc }) };
  }

  // WHERE AN ITEM's FILES ARE UPLOADED, before it is made (the one rule — every picker, the publisher, Drive ask it,
  // never choose): a space's members or public item, that space (no copy); yours, your account (public or not); an
  // audience NARROWER than a whole space (only you in a space, a role, the admins, chosen people, your friends or
  // followers) your own ACCOUNT — nobody else reads its rows — and `submit` adopts it where the item goes (10-21: put
  // in the space, its key row stayed readable by all its members).
  async function uploadPlace({ board = null, audience = "public" } = {}) {
    const a = String(audience ?? "");
    const whole = a === "public" || a === "members";
    if (board && whole) {
      const sp = await boardOf(board);
      if (sp) return { space: sp, public: a === "public" };
    }
    return { space: null, public: !board && a === "public" };
  }
  // AN ITEM's FILES PUT WHERE IT IS: adopted into its place (a copy under that place's key, where they were elsewhere),
  // public exactly when it is — after every make, edit and move (`submit`, `editItem`), never by a caller.
  async function placeFiles(refs, place, pub) {
    if (!refs?.length) return refs ?? [];
    const F = await ctx.require("files");
    const out = await Promise.all(refs.map(f => (f && !f.inline ? F.adopt(f, place, { pub }) : f)));
    await F.publicity(out, place, pub);
    return out;
  }

  // `audience` (the `audience` picker's value): yours — "public" or "private"; a space's — "public" or "members".
  // `at`: when it was made (an item brought over from before keeps its own time).
  // `write`: who may comment and vote on it ({ comment, vote }: anyone · followers · friends · members · author) — its
  // own rule, over its space's policy (`roles.mayWrite`).
  async function submit({ board = null, place = null, title, body, audience = "public", files = [], kind = "post", meta = {}, at = Date.now(), write = null, outside = null }) {
    // Every interaction rule chosen is STORED, "anyone" too (`audience.withWrite`: the one rule, tested).
    meta = (await ctx.require("audience")).withWrite(meta, write);
    const only = audience === "private";
    title = String(title ?? "").trim();
    body = String(body ?? "").trim();
    board = board ?? place;
    if (!TOP.has(kind)) throw new Error(`not something to post: ${kind}`);
    if (!title && kinds.titled(kind)) throw new Error(`a ${kind} needs a title`);
    if (title.length > 300) throw new Error("a title of at most 300 characters");
    // A LEVEL'S ITEM of yours (rewards §5a, PRO or VIP): SEALED in your level group (`circles.level`: only its members —
    // who give at that level — read it), and a public TEASER where it would have been (its title, its level, where
    // it is sealed): anyone sees it exists; one at the level asks in on opening it.
    if (!board && audience === "public" && !meta?.gate && (meta?.level === "pro" || meta?.level === "vip")) {
      const g = await (await ctx.require("circles")).level(meta.level);
      // Its files PRIVATE first (made public with a public post: the teaser carries none; the group re-keys them for
      // its members, as any file brought into a space).
      await (await ctx.require("files")).publicity(files, null, false).catch(e => ctx.failed("posts", e, `its files kept private: ${e.message}`));
      const sealed = await submit({ board: g.id, title, body, kind, files, meta, at, write, audience: "members" });
      // The gate names the group (its owner and nonce prove its id): one at the level enters it by itself.
      return submit({ title, body: "", kind, meta: { level: meta.level, gate: { space: g.id, ref: sealed, name: g.name, kind: g.kind, owner: g.governance.owner, nonce: g.governance.nonce, ...(g.group ? { group: g.group } : {}) } }, at });
    }
    ({ board, audience, meta } = await route({ board, audience, kind, title, meta }));
    if (board) {
      const sp = await boardOf(board);
      // Its files where it is (a space's: public exactly when it is).
      if (sp) files = await placeFiles(files, sp, audience === "public" && (await publicIn(sp, kind)));
      // NOT IN IT, where its policy lets anyone post: kept in this person's profile, for that space (`meta.space`),
      // pointed in its bag — one more item of its room (`outsideSource`), the room's one filter deciding.
      if (!sp && outside?.id === board) {
        const pr = await roles.ofPublic.strict(outside);
        await pr.settled;
        const self = await me();
        if (pr.banned(self) || !pr.allowsIn("post", self, domainOf(kind))) throw new Error(`only ${space.shown(outside)}'s members post here`);
        const id = await (await profileRoom(self)).post(kind, body, { title, files, meta: { ...meta, space: outside.id }, at });
        await pointToSpace(outside);
        return `space:${outside.id}/~${self}~${id}`;
      }
      if (!sp) throw await notInBoard(board);
      if (audience === "public" && !(await publicIn(sp, kind))) throw new Error(`${space.shown(sp)} keeps this to its members: it cannot be public`);
      return `space:${sp.id}/${await (await boardRoom(sp)).post(kind, body, { title, files, meta, at, aud: audience === "members" ? "members" : null })}`;
    }
    const self = await me();
    // Its files: yours, public exactly when the post is (one picked while "Everyone" was chosen, posted "Only you": re-keyed).
    files = await placeFiles(files, null, !only);
    const ref = `${self}/${await (await profileRoom(self)).post(kind, body, { title, private: only, files, meta, at })}`;
    // Listed in Discover (its own bag made with it: nobody reading it waits on one that does not exist).
    // UNLISTED (`meta.unlisted`): public — anyone with its link reads it — but never listed in Discover.
    if (!only && !meta.unlisted) await listPublic(ref, { kind, at }, { did: self }).catch(e => ctx.failed("posts", e, `listing ${ref}: ${e.message}`));
    return ref;
  }

  // THE THREAD: comments as a tree, best first (a reply whose parent is gone goes to the top).
  async function thread(ref, { outside = null } = {}) {
    const self = await me();
    const all = [];
    if (ref.startsWith("space:")) {
      const sp = outside ?? (await boardOf(ref));
      if (!sp) return [];
      const ptr = outside ? await pointerOf(ref) : null;
      const r = await (outside ? (ptr ? pointedRoom(sp, [ptr]) : outsideRoom(sp)) : boardRoom(sp));
      // A thread's comments are all made AFTER its post: the place read from the post's time on — complete, whatever
      // the list's window, and nothing older.
      if (!outside) await sinceItem(r, idOf(ref));
      const votes = tally(r.reactions());
      const post = idOf(ref);
      for (const it of r.list())
        if (it.kind === "comment" && postOf(it) === post)
          all.push({ ...it, ref: `space:${sp.id}/${it.id}`, parent: it.re === post ? ref : `space:${sp.id}/${it.re}`, ...scored(it.id, votes, self), mayRemove: r.mayRemove(it), replies: [] });
    } else {
      const rs = await profiles([whereOf(ref), self, ...(await following()), ...(await pointersTo(ref))]);
      // The post's rule (`roles.mayWrite`): who may comment and vote on it, as it was when each was written (a rule made
      // later never takes back what it allowed) — what does not pass, not counted.
      const post = rs.flatMap(r => r.list()).find(it => itemRef(it.by, it.id) === ref) ?? null;
      const may = (action, by, cred, at) => !post || roles.mayWrite({ action, item: post, writer: by, cred, at: at ?? Infinity }) === true;
      const votes = new Map();
      for (const r of rs) tally(r.reactions().filter(x => x.item !== ref || signalPasses(x.emoji, action => may(action, x.by, x.cred, x.at))), votes);
      for (const r of rs)
        for (const it of r.list())
          if (it.kind === "comment" && postOf(it) === ref && may("comment", it.by, it.meta?.cred, it.at)) {
            const cref = itemRef(it.by, it.id);
            all.push({ ...it, ref: cref, parent: it.re, ...scored(cref, votes, self), mayRemove: it.by === self, replies: [] });
          }
    }
    const byRef = new Map(all.map(c => [c.ref, c]));
    const top = [];
    for (const c of all) (c.parent !== ref && byRef.get(c.parent) ? byRef.get(c.parent).replies : top).push(c);
    const best = (a, b) => b.score - a.score || a.at - b.at;
    const order = cs => (cs.sort(best), cs.forEach(c => order(c.replies)), cs);
    return order(top);
  }

  // MAY THIS PERSON write about a profile item (comment, vote), by its rule (`roles.mayWrite`, the one check: its own,
  // else its author's policy now) — and the CREDENTIAL to cite where the rule names the author's friends or followers
  // (`roles.credToCite`): null where none is needed; refused, by name, where they may not.
  async function writeCred(ref, action) {
    const owner = whereOf(ref);
    const self = await me();
    if (!owner || owner === self) return null;
    const post = (await profileRoom(owner)).list().find(it => itemRef(it.by, it.id) === ref);
    if (!post) return null;
    await roles.personal(owner).ready;
    const cred = await roles.credToCite(post, action);
    // false: refused (null: a credential still being read — let it through; every reader checks it).
    if (roles.mayWrite({ action, item: post, writer: self, cred }) === false) throw new Error(`only ${(await ctx.require("directory")).shown(owner)} may ${action} here`);
    return cred;
  }
  // May this person comment or vote on item `it` (as `list`/`get` give it): the one check.
  async function mayWriteOn(it, action, { outside = null } = {}) {
    if (!it) return false;
    if (String(it.ref).startsWith("space:")) {
      const sp = await boardOf(it.ref);
      if (sp) return (await (await boardRoom(sp)).mayAbout?.(it.id, action)) ?? true;
      // Not in it: as the space's public policy says (anyone), unless banned there.
      if (!outside) return false;
      const pr = await roles.ofPublic.strict(outside);
      await pr.settled;
      const self = await me();
      return !pr.banned(self) && roles.mayWrite({ action, item: it, writer: self, r: pr }) === true;
    }
    return ctx.optional("items: writeCred", writeCred(it.ref, action).then(() => true), () => false);
  }

  // Something of this person's about someone's profile post: a pointer to them in its bag, once per page.
  const pointed = new Set();
  async function pointTo(ref) {
    const self = await me();
    if (ref.startsWith(`${self}/`) || pointed.has(ref)) return;
    await index.point(ref, { from: self });
    pointed.add(ref);
  }

  async function comment(post, re, body, { files = [], outside = null } = {}) {
    // Its files where the item it is about is, public exactly when that item is (what attaches follows its item).
    if (files?.length && !outside) {
      const at = await placeOf(post);
      if (!at.outside) files = await placeFiles(files, at.place, at.public);
    }
    body = String(body ?? "").trim();
    if (!body && !files.length) throw new Error("a comment needs something in it");
    if (post.startsWith("space:")) {
      const sp = await boardOf(post);
      if (!sp) {
        // NOT IN IT (public participation): in this person's own profile, pointed in the space's bag.
        if (!outside || !(await mayWriteOn(await get(post, { outside }), "comment", { outside }))) throw new Error("only its members comment here");
        await (await profileRoom(await me())).post("comment", body, { re: re ?? post, in: post, files });
        await pointToSpace(outside);
        return;
      }
      const room = await boardRoom(sp);
      // Its post's rule (the one check): a credential cited where only its author's friends or followers comment.
      const it = room.list().find(x => x.id === idOf(post));
      const cred = it ? await roles.credToCite(it, "comment", await roles.of.strict(sp)) : null;
      await room.post("comment", body, { re: idOf(re ?? post), in: idOf(post), files, ...(cred ? { meta: { cred } } : {}) });
      return;
    }
    const mine = await profileRoom(await me());
    // Its post's rule: the credential this person cites where only its author's friends or followers comment.
    const cred = await writeCred(post, "comment");
    await mine.post("comment", body, { re: re ?? post, in: post, files, ...(cred ? { meta: { cred } } : {}) });
    // On a private post: private too, and no pointer anywhere.
    await mine.whenPrivate?.();
    if (!mine.isPrivate?.(post)) await pointTo(post);
  }

  // A SIGNAL (`signals`): this person's interaction with an item, set (or taken back) — the ONE writer of them all.
  // `value`: a vote's +1, −1 or 0; a many-signal's value (a tag) with `on`; else on/off. Kept where their vote is: the
  // item's space's room (a member; `post`: where a comment's are kept — its post's), their own profile pointed in the
  // space's bag (from outside a public space, by its policy), or their own profile (a person's item: its rule's
  // credential cited, pointed for its author where public). Only what changes is written.
  async function signal(ref, id, value = true, { post = ref, outside = null, on = true } = {}) {
    const s = signals.of(id);
    if (!s) throw new Error(`no such signal: ${id}`);
    const want = new Map(
      s.value === "updown"
        ? [[signals.markFor(id, 1), value === 1], [signals.markFor(id, -1), value === -1]]
        : [[signals.markFor(id, value), s.value === "many" ? !!on : !!value]],
    );
    const adding = [...want.values()].some(Boolean);
    // A SUM replaces this person's last value: every other value of it they gave, taken back.
    const replacing = had => (s.value === "sum" ? new Map([...[...had].filter(e => signals.byMark(e)?.id === id && !want.has(e)).map(e => [e, false]), ...want]) : want);
    const self = await me();
    const inSpace = String(post).startsWith("space:");
    const sp = inSpace ? await boardOf(post) : null;
    if (inSpace && !sp) {
      // NOT IN IT (public participation): in this person's own profile, pointed in the space's bag.
      outside ??= await outsideOf(post);
      if (!outside) throw new Error("not a space you can reach");
      if (s.action !== "read" && !(await mayWriteOn(await get(post, { outside }), s.action, { outside }))) throw new Error(`only its members ${s.action} here`);
      const mine = await profileRoom(self);
      const had = new Set(mine.reactions().filter(x => x.item === ref && x.by === self).map(x => x.emoji));
      for (const [e, v] of replacing(had)) if (v !== had.has(e)) await mine.react(ref, e, v, {});
      if (adding && s.point) await pointToSpace(outside);
      return;
    }
    const onBoard = !!sp;
    const r = onBoard ? await boardRoom(sp) : await profileRoom(self);
    const item = onBoard && ref.startsWith("space:") ? idOf(ref) : ref;
    const had = new Set(r.reactions().filter(x => x.item === item && x.by === self).map(x => x.emoji));
    const cred = onBoard || s.action === "read" ? null : await writeCred(ref, s.action);
    for (const [e, v] of replacing(had)) if (v !== had.has(e)) await r.react(item, e, v, { cred });
    if (adding && !onBoard) await r.whenPrivate?.();
    if (adding && s.point && !onBoard && !r.isPrivate?.(post)) await pointTo(post);
  }
  // A VOTE: the up/down signal.
  const vote = (ref, v, post = ref, { outside = null } = {}) => signal(ref, "vote", v, { post, outside });

  // VIEWED: this person read the item (its page opened) — the view signal, once per page, never on one's own. Never
  // fails the page: a view is best effort.
  const viewedHere = new Set();
  async function view(ref, { outside = null } = {}) {
    // A GUEST's view is nobody's: not counted (a signal is its signer's).
    if (viewedHere.has(ref) || !(await me())) return;
    viewedHere.add(ref);
    try {
      const it = await ctx.optional("items: get", get(ref, { outside }), () => null);
      if (!it || it.by === (await me()) || it.signaled?.view) return;
      await signal(ref, "view", true, { outside });
    } catch (e) {
      ctx.failed("posts", e, `a view not kept: ${e.message ?? e}`);
    }
  }

  // USED: this person's running TIME and DATA on an item (`usage`), as its public counts — never on one's own; best
  // effort, as a view.
  async function used(ref, { s = 0, b = 0 } = {}) {
    if (!(await me())) return; // a guest's: nobody's, as a view
    try {
      const outside = await ctx.optional("items: outsideOf", outsideOf(ref), () => null);
      const it = await ctx.optional("items: get", get(ref, { outside }), () => null);
      if (!it || it.by === (await me())) return;
      if (s) await signal(ref, "time", s, { outside });
      if (b) await signal(ref, "data", b, { outside });
    } catch (e) {
      ctx.failed("posts", e, `time and data not kept: ${e.message ?? e}`);
    }
  }

  // REMOVE: this person's own, or — on a board, as its moderator — anyone's (hidden, as the server's moderation does).
  // (Through the DELETE machine: asked, then hidden — or refused, with why.)
  const remove = async ref => (await ctx.require("lifecycle")).deleting("item", ref, () => removeNow(ref), "hidden");
  async function removeNow(ref) {
    if (ref.startsWith("space:")) {
      const sp = await boardOf(ref);
      if (!sp) throw await notInBoard(ref);
      return (await boardRoom(sp)).remove(idOf(ref));
    }
    const self = await me();
    if (!ref.startsWith(`${self}/`)) throw new Error("only its author removes it");
    await (await profileRoom(self)).remove(idOf(ref));
  }

  // Every board this person is in: their items' public copies as the board reads NOW (`upkeep`, every tick) — a board
  // opened and synced only when what reads in public there CHANGED since this device last synced it (a post, an edit,
  // a vote sync as they are written). What it was synced as: upkeep's mark (`storage`), never every board each page.
  async function syncPublic() {
    const storage = await ctx.require("storage");
    for (const sp of await ctx.optional("items: boards", boards(), () => [])) {
      const r = await roles.of(sp);
      if (!r) continue;
      const sig = r.domains().map(d => (domainReads(r, d) ? 1 : 0)).join("");
      const mark = `public ${sp.id.slice(0, 16)} ${String(sp.self).slice(0, 16)}`;
      if ((await storage.upkeepMark(mark)) === sig) continue;
      await (await boardRoom(sp))
        .sync()
        .then(() => storage.setUpkeepMark(mark, sig))
        .catch(e => ctx.failed("posts", e, `${sp.name}: public copies: ${e.message}`));
    }
  }
  // ATTACHED ITEMS (a subtitle on a video): contributed like a comment — in the item's place (a board), or on a
  // profile in the contributor's own tail with a pointer on the item (a private item: private too) — and listed with it.
  async function attach(post, kind, body, { meta = {}, files = [], place = undefined } = {}) {
    if (!kinds.attaching().includes(kind)) throw new Error(`${kind} does not attach to an item`);
    // Its files: where it is kept — beside its item (that item's place and publicity), or the place named.
    if (files?.length && place === undefined) {
      const at = await ctx.optional("items: placeOf", placeOf(post), () => null);
      if (at && !at.outside) files = await placeFiles(files, at.place, at.public);
    }
    // KEPT ELSEWHERE (`place`: a space, or null — this person's own): about `post` by its full reference.
    if (place !== undefined && !(place && post.startsWith(`space:${place.id}/`))) {
      if (place) return `space:${place.id}/${await (await boardRoom(place)).post(kind, body, { in: post, meta, files })}`;
      const mine = await profileRoom(await me());
      return `${await me()}/${await mine.post(kind, body, { in: post, meta, files })}`;
    }
    if (post.startsWith("space:")) {
      const sp = await boardOf(post);
      if (!sp) throw await notInBoard(post);
      return `space:${sp.id}/${await (await boardRoom(sp)).post(kind, body, { in: idOf(post), meta, files })}`;
    }
    const mine = await profileRoom(await me());
    const id = await mine.post(kind, body, { in: post, meta, files });
    await mine.whenPrivate?.();
    if (!mine.isPrivate?.(post)) await pointTo(post);
    return `${await me()}/${id}`;
  }
  // A place read from an item's time on (its id: `t` ‖ time in ms, base 36): everything made after it.
  async function sinceItem(r, id) {
    const m = /^t([0-9a-z]{9})/.exec(id);
    if (!m) throw new Error(`not an item id: ${id}`);
    return r.since?.(parseInt(m[1], 36));
  }
  async function attached(ref, kind, { outside = null } = {}) {
    const self = await me();
    const out = [];
    if (ref.startsWith("space:")) {
      const sp = outside ?? (await boardOf(ref));
      if (!sp) return [];
      const ptr = outside ? await pointerOf(ref) : null;
      const r = await (outside ? (ptr ? pointedRoom(sp, [ptr]) : outsideRoom(sp)) : boardRoom(sp));
      // What attaches to an item is made AFTER it: read from its time on — never bounded by a list's window.
      if (!outside) await sinceItem(r, idOf(ref));
      for (const it of r.list()) if (it.kind === kind && it.in === idOf(ref)) out.push({ ...shape(it, `space:${sp.id}/${it.id}`, { id: sp.id, name: sp.name }), mayRemove: r.mayRemove(it) });
    } else {
      for (const r of await profiles([whereOf(ref), self, ...(await following()), ...(await pointersTo(ref))]))
        for (const it of r.list()) if (it.kind === kind && it.in === ref) out.push({ ...shape(it, itemRef(it.by, it.id), null), mayRemove: it.by === self });
    }
    const seen = new Set();
    return out.filter(x => !seen.has(x.ref) && seen.add(x.ref)).sort((a, b) => a.at - b.at);
  }
  // An item of this person's EDITED (a subtitle's text or label): its body, files and meta.
  async function editItem(ref, body, { files = null, meta = null, title = undefined } = {}) {
    // Its files where it is, public exactly when it is (`placeOf`).
    if (files?.length) {
      const at = await placeOf(ref);
      if (!at.outside) files = await placeFiles(files, at.place, at.public);
    }
    if (ref.startsWith("space:")) {
      const sp = await boardOf(ref);
      if (sp) return (await boardRoom(sp)).editFull(idOf(ref), body, { files, meta, title });
      // NOT IN IT (public participation): what this person wrote there lives in their own profile (`~<did>~<id>`).
      const m = /^~(did:craftec:[^~]+)~(.+)$/.exec(idOf(ref));
      if (!m || m[1] !== (await me())) throw await notInBoard(ref);
      return (await profileRoom(m[1])).editFull(m[2], body, { files, meta, title });
    }
    return (await profileRoom(await me())).editFull(idOf(ref), body, { files, meta, title });
  }

  // ITS HISTORY: the versions an item had (`content`'s `history`: newest first, each with `until` — when it was
  // replaced); from outside, none (the public copy keeps no history of its own here).
  async function history(ref) {
    if (ref.startsWith("space:")) {
      const sp = await boardOf(ref);
      if (sp) return (await boardRoom(sp)).history(idOf(ref));
      // Not in it: what someone wrote there from outside lives in their own profile (`~<did>~<id>`).
      const m = /^~(did:craftec:[^~]+)~(.+)$/.exec(idOf(ref));
      return m ? ((await profileRoom(m[1])).history?.(m[2]) ?? []) : [];
    }
    const room = await profileRoom(whereOf(ref));
    return room.history ? room.history(idOf(ref)) : [];
  }

  // A post's FILES replaced by its author (a video whose renditions grew): its place's room.
  async function setFiles(ref, files) {
    if (ref.startsWith("space:")) {
      const sp = await boardOf(ref);
      if (!sp) throw await notInBoard(ref);
      return (await boardRoom(sp)).setFiles(idOf(ref), files);
    }
    return (await profileRoom(await me())).setFiles(idOf(ref), files);
  }

  // Items of a KIND in exactly these places, read whole (few, chosen places: a subtitle's lookup), never every board.
  // `after` (an item id): only what was made after it (a subtitle is always newer than its video) — a bound, not a window.
  // A space's AUDIENCE groups this person is in (`conversation.audience`: what only a role, the admins, the owner read).
  // (One this person was taken out of — no longer of that audience — shown no more: what was there before stays sealed
  // to its key, never presented.)
  const audienceGroups = async sp => {
    const gs = (await space.mine.strict()).filter(g => g.group?.startsWith(`audience:${sp.id}/`));
    const out = await Promise.all(gs.map(g => ctx.optional("items: roles.of", roles.of(g).then(async r => (await r.settled, r.left ? null : g)), () => null)));
    return out.filter(Boolean);
  };
  async function inPlaces({ spaces = [], people: dids = [] }, kind, { after = null, withVotes = true } = {}) {
    // A space READ FROM OUTSIDE (`outside: true`, its public description — someone not in it, a guest): its public
    // items, as Discover lists them; no audience groups.
    const inside = spaces.filter(sp => !sp.outside);
    spaces = [...spaces, ...(await Promise.all(inside.map(audienceGroups))).flat()];
    const ks = kindsFor(kind);
    const read = async sp => {
      if (sp.outside) return pointedPosts(ks, 0, { space: sp });
      if (after) await sinceItem(await boardRoom(sp), after);
      return boardPosts(sp, { kinds: ks, window: after ? "held" : "all" });
    };
    const [a, b] = await Promise.all([Promise.all(spaces.map(sp => ctx.optional("items: read", read(sp), () => []))), ctx.optional("items: profilePosts", profilePosts(dids, [], ks, 0, { withVotes }), () => [])]);
    return [...a.flat(), ...b];
  }

  // WHERE AN ITEM IS SHOWN — the one link to its page, for every app that links to one: a video or a track where it
  // plays (Video, Audio, Image), a post (or anything else) on its board, a note in Note, a file in Drive.
  // (Where an item's page is, its link, the item a text names: the module's `pure`, as an app's frame has them.)
  const { appOf, pageOf, linkOf, refOf } = pure(kinds, () => `${location.origin}${location.pathname}`);

  // MENTIONS told: everyone a text mentions (`[@…](person:DID)`: `md-editor`) is sent a notice to their inbox (`index`)
  // — so a mention reaches them wherever it is written (a personal post of someone they never read, a comment), not
  // only where they already read (`activity.mentions`). `ref`: what opens it (a comment: its post).
  const mentionsTold = async (ref, kind, body) => {
    const me = await space.me.strict();
    const dids = [...new Set([...String(body ?? "").matchAll(/\(person:(did:craftec:[1-9A-HJ-NP-Za-km-z]{20,64})\)/g)].map(m => m[1]))].filter(d => d !== me);
    for (const did of dids) await index.send(did, { kind: "mention", ref, itemKind: kind, from: me, text: String(body).slice(0, 280), at: Date.now() }).catch(e => ctx.failed("posts", e, `telling ${did.slice(12, 20)}… of a mention: ${e.message}`));
  };
  const told = (p, ref, kind, body) => (ref && ctx.background("items: mentionsTold", mentionsTold(ref, kind, body)), p);
  return {
    itemRef,
    submit: async o => {
      const ref = await submit(o);
      if (o?.audience !== "private") told(null, ref, o?.kind ?? "post", o?.body);
      return ref;
    },
    comment: async (post, re, body, o) => {
      const out = await comment(post, re, body, o);
      const ref = typeof post === "string" ? post : post?.ref;
      told(null, ref, typeof post === "string" ? "post" : (post?.kind ?? "post"), body);
      return out;
    },
    editItem: async (ref, body, o) => {
      const out = await editItem(ref, body, o);
      told(null, ref, null, body);
      return out;
    },
    // THE RANKING (a feed's sort), the one: what `list` orders by — for a list made of several (`tag`'s).
    rank: (list, sort = "new", by = null) => [...list].sort(sorter(sort, by)),
    route, placeOf, uploadPlace, mayWriteOn: ctx.total("items.mayWriteOn", () => false, mayWriteOn), pageOf, linkOf, refOf, appOf, list: ctx.total("items.list", () => [], list), feed: ctx.total("items.feed", () => [], feed), get: ctx.total("items.get", () => null, get), setFiles, attach, attached: ctx.total("items.attached", () => [], attached), history, publicIn: ctx.total("items.publicIn", () => [], publicIn), inPlaces: ctx.total("items.inPlaces", () => [], inPlaces), following, thread: ctx.total("items.thread", () => [], thread), vote, remove, boards: ctx.total("items.boards", () => [], boards), boardOf: ctx.total("items.boardOf", () => null, boardOf), spaceIdOf, publicSpaces: ctx.total("items.publicSpaces", () => [], publicSpaces), publicSpace: ctx.total("items.publicSpace", () => null, publicSpace), outsideOf: ctx.total("items.outsideOf", () => null, outsideOf), syncPublic, signal, view, used, tagsOf, normalTags, isNsfw, nsfwShown, visible: (list, where = {}) => shownOf(list, where), onChange: f => changed.push(f) };
}
