// ITEMS, a capability: every item that STANDS ON ITS OWN — of any kind in the catalog (`kinds`), siblings: a `post`
// (text), a `video`, a `movie` … — with what attaches to it (COMMENTS, and attaching kinds: a `subtitle`) and VOTES,
// in one of two PLACES. Apps are lenses on it by domain (Board: posts; Videos: the video kinds; Subtitles: subtitles).
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
//   await posts.list({ board } | { by } | { feed }, sort, kind)   // (`kind`: a kind, or kinds — a domain's)
//                                                     // a space's board, a profile, or the FEED (the PERSONAL
//                                                     // view: the boards of every space this person is in, their own
//                                                     // profile, those they follow and their friends'); sort "hot" | "new" | "top"
//   await posts.get(ref)   await posts.thread(ref)    // one post; its comments as a tree (`replies`), best first
//   await posts.comment(post, re, body)   await posts.vote(ref, 1 | -1 | 0, post)   await posts.remove(ref)
//   await posts.syncPublic()      // this person's items on each board, public exactly while the board reads in public
//   await posts.boards()                             // the spaces this person is in: their boards
//   await posts.boardOf(ref | id)                    // the space a board post or board id is in (null: a profile's)
//   posts.onChange(fn)
// A post: { ref, id, by, title, body, board, at, edited, comments, score, mine, mayRemove }; a comment: the same
// without title and board, with `replies`.
export async function start(ctx) {
  const [content, index, space, edge] = await Promise.all(["content", "index", "space", "edge"].map(n => ctx.require(n)));
  const TAIL = "posts";
  const UP = "▲";
  const DOWN = "▼";
  // What stands on its own (a post, a video, a movie …: `kinds`): comments and votes answer these.
  const kinds = await ctx.require("kinds");
  const TOP = new Set(kinds.all());
  // What a list shows: a kind, or KINDS (a domain's: `kinds.inDomain("video")` — videos, movies, episodes …).
  const kindsFor = k => (Array.isArray(k) ? k : [k]);
  const changed = [];
  const fire = () => changed.forEach(f => f());
  const me = async () => (await space.account()).id;
  // Whose profiles this person reads: whom they follow, and their friends (friends need no follow as well).
  // A follow names a SPACE: a person's DID (their personal space) or a shared space's id.
  const following = async () => {
    const p = await edge.people();
    return [...new Set([...p.list("follow"), ...p.list("friend")])].filter(id => id.startsWith("did:"));
  };
  // The SHARED spaces followed (not joined): each read from outside, as Discover reads a public one.
  const followedSpaces = async () => {
    const p = await edge.people();
    const inside = new Set((await space.mine()).map(s => s.id));
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
      p.catch(() => rooms.delete(key));
      rooms.set(key, p);
    }
    return rooms.get(key);
  };
  // The spaces this person is in that have a Board (an app of the space, added by its owner or admins: `roles`).
  const roles = await ctx.require("roles");
  // A space's board room is its place for every item `posts` keeps — posts, videos, subtitles — so any app on it opens
  // the place (a space with Videos and no Board still takes videos).
  const PLACE_APPS = ["board", "videos", "subtitles"];
  const boards = async () => {
    const all = await space.mine();
    const on = await Promise.all(all.map(sp => roles.of(sp).then(r => r.apps().some(a => PLACE_APPS.includes(a)), () => false)));
    return all.filter((_, i) => on[i]);
  };
  const boardOf = async x => {
    const id = String(x).startsWith("space:") ? String(x).slice(6, String(x).indexOf("/")) : x;
    return (await boards()).find(s => s.id === id) ?? null;
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
    return JSON.stringify({ ...it, files: await Promise.all(it.files.map(f => files.current(f).catch(() => f))) });
  }
  // A DOMAIN's read setting in a space: its own (`text`, `video`, `subtitle` …: content decides, whatever app shows it),
  // else the app-named setting from before (text: "board", video: "videos", subtitle: "subtitles"), else the space's.
  const LEGACY = { text: "board", video: "videos", subtitle: "subtitles" };
  const domainReads = (r, domain) => {
    const own = r.policiesAt(domain).read;
    if (own) return own === "anyone";
    return r.policy(LEGACY[domain] ?? domain, "read") === "anyone";
  };
  // Whether an item of `kind` is public in a space now.
  async function publicIn(sp, kind) {
    const r = await roles.of(sp);
    await r.settled;
    return domainReads(r, kinds.policyDomain(kind));
  }
  // (A space's PLACE: its board room — the same for every kind.)
  const boardRoom = sp =>
    opened(`space:${sp.id}`, async () => {
      const [a, b, r] = await Promise.all([content.in(space.board(sp)), content.in(space.board(sp, { pub: true })), roles.of(sp)]);
      // PUBLIC is per item, by its own APP's setting now (a video: Videos'; a post: Board's) — what attaches to an item
      // (a comment, a vote, a subtitle) as that item is. Not bound to Board: a space's place holds every kind.
      const readsAnyone = domain => domainReads(r, domain);
      const kindOf = new Map(); // item id → kind (every author's, as listed)
      const learnKinds = () => [...a.list(), ...b.list()].forEach(x => kindOf.set(x.id, x.kind));
      // What governs an item: its own kind's domain — or, attached to an item HERE, that item's (a comment, a vote, a
      // subtitle beside its video); an attaching item about something elsewhere is its own (a team's subtitle).
      const rootDomain = (kind, about, depth = 0) => {
        if (TOP.has(kind)) return kinds.policyDomain(kind);
        const k = kindOf.get(about);
        if (k && depth < 8) return rootDomain(k, null, depth + 1);
        return kinds.of(kind) ? kinds.policyDomain(kind) : "text";
      };
      const pubOfValue = v => {
        try {
          const x = JSON.parse(v);
          return readsAnyone(rootDomain(x.kind ?? "post", x.in ?? x.item ?? x.re ?? null));
        } catch {
          return false;
        }
      };
      const pubOfItem = it => readsAnyone(rootDomain(it.kind, it.in ?? it.re ?? null));
      // THE SYNC: this person's rows — a public copy of each while ITS APP reads in public, none while it does not. Rows
      // only in the public table (written there before the sealed table held everything) move into the sealed one first.
      let syncing = null;
      const sync = () =>
        (syncing ??= (async () => {
          await Promise.all([a.settled, b.settled]);
          const inA = new Map(a.own().map(x => [x.key, x.value]));
          const inB = new Map(b.own().map(x => [x.key, x.value]));
          for (const [k, v] of inB) if (!inA.has(k)) (await a.putOwn(k, v), inA.set(k, v));
          learnKinds();
          for (const [k, v0] of inA) {
            if (pubOfValue(v0)) {
              const v = await currentKeys(v0);
              if (inB.get(k) !== v) await b.putOwn(k, v);
            } else if (inB.has(k)) await b.dropOwn(k);
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
        mayPost: () => a.mayPost(),
        onChange: f => (a.onChange(f), b.onChange(f)),
        settled: Promise.all([a.settled, b.settled]),
        sync,
        post: async (kind, body, opts = {}) => {
          await Promise.all([a.settled, b.settled]);
          const id = await a.post(kind, body, opts);
          await sync();
          return id;
        },
        react: async (id, e, on) => (await Promise.all([a.settled, b.settled]), await sync(), await a.react(id, e, on), await sync()),
        edit: async (id, body) => (await sync(), await a.edit(id, body), await sync()),
        setFiles: async (id, files) => (await sync(), await a.setFiles(id, files), await sync()),
        editFull: async (id, body, o) => (await sync(), await a.edit(id, body, o), await sync()),
        // Out of both tables (an author's own; a moderator's hide, in each table it is listed in).
        remove: async id => {
          for (const x of rooms) if (x.list().some(it => it.id === id)) await x.remove(id);
        },
      };
    });
  const outsideRoom = desc =>
    opened(`outside:${desc.id}`, () => content.in(space.board(desc, { outside: true })).then(b => ({ ...b, list: () => b.list().map(x => ({ ...x, pub: true })), mayRemove: () => false })));
  // A PROFILE's room: their public tail. YOURS also holds your PRIVATE posts (per post: "only you"), sealed in your
  // account's table `journal` — with their comments and votes, so nothing of them reaches the public tail.
  const JOURNAL = "journal";
  const profileRoom = did =>
    opened(did, async () => {
      const pub = await content.in({ kind: "public", did, name: TAIL });
      const acc = await space.account();
      if (did !== acc.id) return pub;
      const priv = await content.in({ kind: "journal", messages: JOURNAL, scope: acc });
      const isPrivate = id => priv.list().some(x => x.id === id);
      const roomOf = id => (isPrivate(id) ? priv : pub);
      const idIn = ref => String(ref ?? "").slice(String(ref ?? "").lastIndexOf("/") + 1);
      return {
        list: () => [...pub.list(), ...priv.list().map(x => ({ ...x, private: true }))],
        reactions: () => [...pub.reactions(), ...priv.reactions()],
        mayRemove: it => it.by === acc.id,
        onChange: f => (pub.onChange(f), priv.onChange(f)),
        settled: Promise.all([pub.settled, priv.settled]),
        isPrivate: ref => isPrivate(idIn(ref)),
        post: (kind, body, opts = {}) => (TOP.has(kind) ? (opts.private ? priv : pub) : roomOf(idIn(opts.in))).post(kind, body, opts),
        react: (item, e, on) => roomOf(idIn(item)).react(item, e, on),
        remove: id => roomOf(id).remove(id),
        edit: (id, body) => roomOf(id).edit(id, body),
        setFiles: (id, files) => roomOf(id).setFiles(id, files),
        editFull: (id, body, o) => roomOf(id).edit(id, body, o),
      };
    });
  const profiles = async dids => (await Promise.all([...new Set(dids)].map(d => profileRoom(d).catch(() => null)))).filter(Boolean);
  const pointersTo = async ref => (await index.pointers(ref).catch(() => [])).map(p => p.from).filter(d => typeof d === "string" && d.startsWith("did:craftec:"));
  const idOf = ref => ref.slice(ref.lastIndexOf("/") + 1);
  const whereOf = ref => ref.slice(0, ref.lastIndexOf("/"));

  // VOTES: ref → (did → 1 | -1), from reactions (both at once — a change half-made — counts as neither).
  function tally(reactions, votes = new Map()) {
    for (const x of reactions) {
      if (x.emoji !== UP && x.emoji !== DOWN) continue;
      const on = votes.get(x.item) ?? new Map();
      on.set(x.by, on.has(x.by) ? 0 : x.emoji === UP ? 1 : -1);
      votes.set(x.item, on);
    }
    return votes;
  }
  const scored = (key, votes, self) => {
    const on = votes.get(key);
    return { score: on ? [...on.values()].reduce((n, v) => n + v, 0) : 0, mine: on?.get(self) ?? 0 };
  };
  const postOf = c => c.in ?? c.re; // a comment's post (an old one answering its post directly has no `in`)
  const shape = (it, ref, board) => {
    const [first, ...rest] = it.body.split("\n"); // a post from before titles: its first line is its title
    return { ref, id: it.id, kind: it.kind, in: it.in ?? null, by: it.by, title: it.title ?? first.slice(0, 300), body: it.title ? it.body : rest.join("\n").trim(), board, at: it.at, edited: it.edited, private: !!it.private, files: it.files ?? [], meta: it.meta ?? {} };
  };

  // A BOARD's posts: everything is in its one room (reactions keyed by the item's id).
  async function boardPosts(sp, { outside = false, kinds = ["post"] } = {}) {
    const r = await (outside ? outsideRoom(sp) : boardRoom(sp));
    const self = await me();
    const items = r.list();
    const votes = tally(r.reactions());
    const counts = new Map();
    for (const it of items) if (it.kind === "comment") counts.set(postOf(it), (counts.get(postOf(it)) ?? 0) + 1);
    return items
      .filter(it => kinds.includes(it.kind))
      .map(it => ({ ...shape(it, `space:${sp.id}/${it.id}`, { id: sp.id, name: sp.name }), pub: it.pub, comments: counts.get(it.id) ?? 0, ...scored(it.id, votes, self), mayRemove: r.mayRemove(it) }));
  }
  // PROFILE posts: from their authors' tails; comments and votes from the tails known here (the reader's, whom they
  // follow, and whoever `readers` names).
  async function profilePosts(authors, readers = [], kinds = ["post"]) {
    const self = await me();
    const rs = await profiles([...authors, self, ...(await following()), ...readers]);
    const votes = new Map();
    const counts = new Map();
    for (const r of rs) {
      tally(r.reactions(), votes);
      for (const it of r.list()) if (it.kind === "comment") counts.set(postOf(it), (counts.get(postOf(it)) ?? 0) + 1);
    }
    const out = [];
    for (const r of await profiles(authors))
      for (const it of r.list()) {
        if (!kinds.includes(it.kind)) continue;
        const ref = `${it.by}/${it.id}`;
        out.push({ ...shape(it, ref, null), comments: counts.get(ref) ?? 0, ...scored(ref, votes, self), mayRemove: it.by === self });
      }
    return out;
  }

  const hot = p => Math.sign(p.score) * Math.log10(Math.max(Math.abs(p.score), 1)) + p.at / 45000000; // Reddit's
  const SORTS = { hot: (a, b) => hot(b) - hot(a), new: (a, b) => b.at - a.at, top: (a, b) => b.score - a.score || b.at - a.at };

  // DISCOVER: the public spaces listed (their descriptions), each proved by its id (its owner), one per id.
  async function publicSpaces() {
    const seen = new Map();
    const lists = await (await ctx.require("moderation")).lists().catch(() => null);
    for (const d of await index.spaces().catch(() => [])) {
      if (!d?.id || seen.has(d.id) || !d.governance?.owner) continue;
      if ((await space.owner(d).catch(() => null)) !== d.governance.owner) continue;
      if (lists?.flagged({ space: d.id, by: d.governance.owner })) continue;
      seen.set(d.id, { id: d.id, name: String(d.name ?? "").slice(0, 100), kind: "server", governance: d.governance });
    }
    return [...seen.values()];
  }

  async function list(where = {}, sort = "hot", kind = "post") {
    const kinds = kindsFor(kind);
    let out;
    // A space's PUBLIC board, seen from outside (`where.outside`: its description); DISCOVER: every public space's.
    if (where.outside) out = await boardPosts(where.outside, { outside: true, kinds });
    // DISCOVER: every public space's board, and the profile posts (public by being there) of the people shown in
    // Discover (each chose to be).
    else if (where.discover) {
      const people = await (await ctx.require("directory")).listed().catch(() => []);
      const [spacesPosts, profile] = await Promise.all([
        Promise.all((await publicSpaces()).map(d => boardPosts(d, { outside: true, kinds }).catch(() => []))).then(x => x.flat()),
        profilePosts(people, [], kinds).catch(() => []),
      ]);
      out = [...spacesPosts, ...profile];
    }
    // DISCOVER is filtered by the moderation lists this person applies (theirs, and whom they chose).
    if (where.outside || where.discover) {
      const lists = await (await ctx.require("moderation")).lists();
      out = out.filter(p => !lists.flagged({ by: p.by, ref: p.ref, space: p.board?.id }));
    }
    else if (where.board) {
      const sp = await boardOf(where.board);
      if (!sp) throw new Error("you are not in that board's space: join it with an invite");
      out = await boardPosts(sp, { kinds });
    } else if (where.feed) {
      // The FEED: every space followed or joined — the spaces this person is in, the people they follow (their
      // personal spaces) and the shared spaces they follow (read from outside).
      const [bs, people, fs] = await Promise.all([boards(), following(), followedSpaces()]);
      out = [
        ...(await Promise.all(bs.map(sp => boardPosts(sp, { kinds }).catch(() => [])))).flat(),
        ...(await Promise.all(fs.map(d => boardPosts(d, { outside: true, kinds }).catch(() => [])))).flat(),
        ...(await profilePosts([await me(), ...people], [], kinds)),
      ];
    } else {
      // A PERSON's posts (Reddit's profile): their profile's, and theirs on every board this reader can read — the
      // public boards (anyone's), and the boards of the spaces this reader is in (their members').
      const by = where.by ?? (await me());
      const [bs, pub] = await Promise.all([boards().catch(() => []), publicSpaces().catch(() => [])]);
      const inside = new Set(bs.map(sp => sp.id));
      const onBoards = (
        await Promise.all([
          ...bs.map(sp => boardPosts(sp, { kinds }).catch(() => [])),
          ...pub.filter(d => !inside.has(d.id)).map(d => boardPosts(d, { outside: true, kinds }).catch(() => [])),
        ])
      )
        .flat()
        .filter(p => p.by === by);
      const seen = new Set();
      out = [...(await profilePosts([by], [], kinds)), ...onBoards].filter(p => !seen.has(p.ref) && seen.add(p.ref));
    }
    return out.sort(SORTS[sort] ?? SORTS.hot);
  }

  async function get(ref, { outside = null } = {}) {
    if (ref.startsWith("space:")) {
      const sp = outside ?? (await boardOf(ref));
      return sp ? ((await boardPosts(sp, { outside: !!outside, kinds: [...TOP] })).find(p => p.ref === ref) ?? null) : null;
    }
    return (await profilePosts([whereOf(ref)], await pointersTo(ref), [...TOP])).find(p => p.ref === ref) ?? null;
  }

  async function submit({ board = null, place = null, title, body, private: only = false, files = [], kind = "post", meta = {} }) {
    title = String(title ?? "").trim();
    body = String(body ?? "").trim();
    board = board ?? place;
    if (!TOP.has(kind)) throw new Error(`not something to post: ${kind}`);
    if (!title) throw new Error(`a ${kind} needs a title`);
    if (title.length > 300) throw new Error("a title of at most 300 characters");
    if (board) {
      const sp = await boardOf(board);
      if (!sp) throw new Error("you are not in that board's space");
      return `space:${sp.id}/${await (await boardRoom(sp)).post(kind, body, { title, files, meta })}`;
    }
    const self = await me();
    // Its files: public exactly when the post is (one picked while "Everyone" was chosen, posted "Only you": re-keyed).
    await (await ctx.require("files")).publicity(files, null, !only).catch(e => ctx.log("posts", { what: `its files: ${e.message}` }));
    const ref = `${self}/${await (await profileRoom(self)).post(kind, body, { title, private: only, files, meta })}`;
    // Its pointer bag, made now (a public post: nobody reading it waits on one that does not exist).
    if (!only) await index.openPointers(ref).catch(e => ctx.log("posts", { what: `the pointer bag of ${ref}: ${e.message}` }));
    return ref;
  }

  // THE THREAD: comments as a tree, best first (a reply whose parent is gone goes to the top).
  async function thread(ref, { outside = null } = {}) {
    const self = await me();
    const all = [];
    if (ref.startsWith("space:")) {
      const sp = outside ?? (await boardOf(ref));
      if (!sp) return [];
      const r = await (outside ? outsideRoom(sp) : boardRoom(sp));
      const votes = tally(r.reactions());
      const post = idOf(ref);
      for (const it of r.list())
        if (it.kind === "comment" && postOf(it) === post)
          all.push({ ...it, ref: `space:${sp.id}/${it.id}`, parent: it.re === post ? ref : `space:${sp.id}/${it.re}`, ...scored(it.id, votes, self), mayRemove: r.mayRemove(it), replies: [] });
    } else {
      const rs = await profiles([whereOf(ref), self, ...(await following()), ...(await pointersTo(ref))]);
      const votes = new Map();
      for (const r of rs) tally(r.reactions(), votes);
      for (const r of rs)
        for (const it of r.list())
          if (it.kind === "comment" && postOf(it) === ref) {
            const cref = `${it.by}/${it.id}`;
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

  // Something of this person's about someone's profile post: a pointer to them in its bag, once per page.
  const pointed = new Set();
  async function pointTo(ref) {
    const self = await me();
    if (ref.startsWith(`${self}/`) || pointed.has(ref)) return;
    await index.point(ref, { from: self });
    pointed.add(ref);
  }

  async function comment(post, re, body) {
    body = String(body ?? "").trim();
    if (!body) throw new Error("a comment needs something in it");
    if (post.startsWith("space:")) {
      const sp = await boardOf(post);
      if (!sp) throw new Error("you are not in that board's space");
      await (await boardRoom(sp)).post("comment", body, { re: idOf(re ?? post), in: idOf(post) });
      return;
    }
    const mine = await profileRoom(await me());
    await mine.post("comment", body, { re: re ?? post, in: post });
    // On a private post: private too, and no pointer anywhere.
    if (!mine.isPrivate?.(post)) await pointTo(post);
  }

  async function vote(ref, v, post = ref) {
    const onBoard = ref.startsWith("space:");
    const sp = onBoard ? await boardOf(ref) : null;
    if (onBoard && !sp) throw new Error("you are not in that board's space");
    const r = onBoard ? await boardRoom(sp) : await profileRoom(await me());
    const item = onBoard ? idOf(ref) : ref;
    const self = await me();
    const had = new Set(r.reactions().filter(x => x.item === item && x.by === self).map(x => x.emoji));
    // Only what changes is written (taking back a vote that is not there writes nothing).
    for (const [e, on] of [[UP, v === 1], [DOWN, v === -1]]) if (on !== had.has(e)) await r.react(item, e, on);
    if (v && !onBoard && !r.isPrivate?.(post)) await pointTo(post);
  }

  // REMOVE: this person's own, or — on a board, as its moderator — anyone's (hidden, as the server's moderation does).
  async function remove(ref) {
    if (ref.startsWith("space:")) {
      const sp = await boardOf(ref);
      if (!sp) throw new Error("you are not in that board's space");
      return (await boardRoom(sp)).remove(idOf(ref));
    }
    const self = await me();
    if (!ref.startsWith(`${self}/`)) throw new Error("only its author removes it");
    await (await profileRoom(self)).remove(idOf(ref));
  }

  // Every board this person is in: their items' public copies as the board reads NOW (`upkeep`, every tick).
  async function syncPublic() {
    for (const sp of await boards().catch(() => [])) await (await boardRoom(sp)).sync().catch(e => ctx.log("posts", { what: `${sp.name}: public copies: ${e.message}` }));
  }

  // ATTACHED ITEMS (a subtitle on a video): contributed like a comment — in the item's place (a board), or on a
  // profile in the contributor's own tail with a pointer on the item (a private item: private too) — and listed with it.
  async function attach(post, kind, body, { meta = {}, files = [], place = undefined } = {}) {
    if (!kinds.attaching().includes(kind)) throw new Error(`${kind} does not attach to an item`);
    // KEPT ELSEWHERE (`place`: a space, or null — this person's own): about `post` by its full reference.
    if (place !== undefined && !(place && post.startsWith(`space:${place.id}/`))) {
      if (place) return `space:${place.id}/${await (await boardRoom(place)).post(kind, body, { in: post, meta, files })}`;
      const mine = await profileRoom(await me());
      return `${await me()}/${await mine.post(kind, body, { in: post, meta, files })}`;
    }
    if (post.startsWith("space:")) {
      const sp = await boardOf(post);
      if (!sp) throw new Error("you are not in that board's space");
      return `space:${sp.id}/${await (await boardRoom(sp)).post(kind, body, { in: idOf(post), meta, files })}`;
    }
    const mine = await profileRoom(await me());
    const id = await mine.post(kind, body, { in: post, meta, files });
    if (!mine.isPrivate?.(post)) await pointTo(post);
    return `${await me()}/${id}`;
  }
  async function attached(ref, kind, { outside = null } = {}) {
    const self = await me();
    const out = [];
    if (ref.startsWith("space:")) {
      const sp = outside ?? (await boardOf(ref));
      if (!sp) return [];
      const r = await (outside ? outsideRoom(sp) : boardRoom(sp));
      for (const it of r.list()) if (it.kind === kind && it.in === idOf(ref)) out.push({ ...shape(it, `space:${sp.id}/${it.id}`, { id: sp.id, name: sp.name }), mayRemove: r.mayRemove(it) });
    } else {
      for (const r of await profiles([whereOf(ref), self, ...(await following()), ...(await pointersTo(ref))]))
        for (const it of r.list()) if (it.kind === kind && it.in === ref) out.push({ ...shape(it, `${it.by}/${it.id}`, null), mayRemove: it.by === self });
    }
    const seen = new Set();
    return out.filter(x => !seen.has(x.ref) && seen.add(x.ref)).sort((a, b) => a.at - b.at);
  }
  // An item of this person's EDITED (a subtitle's text or label): its body, files and meta.
  async function editItem(ref, body, { files = null, meta = null } = {}) {
    if (ref.startsWith("space:")) {
      const sp = await boardOf(ref);
      if (!sp) throw new Error("you are not in that board's space");
      return (await boardRoom(sp)).editFull(idOf(ref), body, { files, meta });
    }
    return (await profileRoom(await me())).editFull(idOf(ref), body, { files, meta });
  }

  // A post's FILES replaced by its author (a video whose renditions grew): its place's room.
  async function setFiles(ref, files) {
    if (ref.startsWith("space:")) {
      const sp = await boardOf(ref);
      if (!sp) throw new Error("you are not in that board's space");
      return (await boardRoom(sp)).setFiles(idOf(ref), files);
    }
    return (await profileRoom(await me())).setFiles(idOf(ref), files);
  }

  return { submit, list, get, setFiles, attach, attached, editItem, publicIn, thread, comment, vote, remove, boards, boardOf, publicSpaces, syncPublic, onChange: f => changed.push(f) };
}
