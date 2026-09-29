// POSTS, a capability: what people say in PUBLIC, Reddit-shaped — a POST (a title and text) submitted to a BOARD
// (`b/<name>`: nobody owns it, anyone submits to it), COMMENTS on it (answering the post or another comment), and VOTES
// on either. All of it is `content` (its one shape) in its AUTHOR's public tail `posts` (only their account writes it:
// whose tail it is, is who wrote it; anyone reads it): a post an item of kind "post" (`title`, `in`: its board); a
// comment an item of kind "comment" (`re`: what it answers, `in`: its post); a vote a reaction ▲ or ▼ to either. Each is
// named by its REF, `<author did>/<id>`.
// Where to look is POINTERS (`index`, `{ from }`): a board's bag holds one per person who submitted to it, a post's bag
// one per person who commented or voted on it; the reader resolves each in its author's tail (a pointer to nothing is
// nothing). Counts and scores are what this reader resolved.
//
//   const posts = await ctx.require("posts");
//   await posts.submit({ board, title, body })       // a new post: its ref
//   await posts.list({ board } | { by } | {}, sort)   // [post]: a board's, a person's, or HOME (whom this person follows,
//                                                     // their own, and their boards'); sort "hot" | "new" | "top"
//   await posts.get(ref)                             // one post, or null
//   await posts.thread(ref)                          // [comment], each with its `replies`, best first
//   await posts.comment(post, re, body)              // `re`: the post's ref or a comment's
//   await posts.vote(ref, 1 | -1 | 0, post?)         // a post, or a comment of `post`
//   await posts.remove(ref)                          // this person's own post or comment
//   await posts.boards()   await posts.join(board, on)   posts.boardName(text)
//   posts.onChange(fn)
// A post: { ref, id, by, title, body, board, at, edited, comments, score, mine }; a comment: { ref, id, by, body, at,
// edited, score, mine, replies }.
export async function start(ctx) {
  const [content, index, space, edge] = await Promise.all(["content", "index", "space", "edge"].map(n => ctx.require(n)));
  const TAIL = "posts";
  const UP = "▲";
  const DOWN = "▼";
  const changed = [];
  const fire = () => changed.forEach(f => f());

  // A board's name: lower case, letters, digits and `_`.
  const boardName = text => {
    const n = String(text ?? "").trim().replace(/^b\//i, "").toLowerCase();
    if (!/^[a-z0-9_]{2,24}$/.test(n)) throw new Error("a board's name: 2 to 24 letters, digits or _");
    return n;
  };
  const boardRef = name => `board:${name}`;

  // One room per person's tail (opened once per page).
  const rooms = new Map();
  const roomOf = did => {
    if (!rooms.has(did)) {
      const p = content.in({ kind: "public", did, name: TAIL }).then(r => (r.onChange(fire), r));
      p.catch(() => rooms.delete(did));
      rooms.set(did, p);
    }
    return rooms.get(did);
  };
  const roomsOf = async dids => (await Promise.all([...new Set(dids)].map(d => roomOf(d).catch(() => null)))).filter(Boolean);
  const me = async () => (await space.account()).id;
  const mine = async () => roomOf(await me());
  const following = async () => (await edge.people()).list("follow");
  const pins = () => edge.pins();
  const authorOf = ref => ref.slice(0, ref.lastIndexOf("/"));
  const fromPointers = async ref =>
    (await index.pointers(ref).catch(() => [])).map(p => p.from).filter(d => typeof d === "string" && d.startsWith("did:craftec:"));

  // Votes on anything, from these tails: ref → (did → 1 | -1). Both at once (a change half-made): neither counts.
  function votesIn(rs) {
    const votes = new Map();
    for (const r of rs)
      for (const x of r.reactions()) {
        if (x.emoji !== UP && x.emoji !== DOWN) continue;
        const on = votes.get(x.item) ?? new Map();
        on.set(x.by, on.has(x.by) ? 0 : x.emoji === UP ? 1 : -1);
        votes.set(x.item, on);
      }
    return votes;
  }
  const scored = (ref, votes, self) => {
    const on = votes.get(ref);
    return { score: on ? [...on.values()].reduce((n, v) => n + v, 0) : 0, mine: on?.get(self) ?? 0 };
  };
  // A comment's post: `in` (an old comment answering its post directly has none: its `re`).
  const postOf = c => c.in ?? c.re;

  const SORTS = {
    // Reddit's: the score's order of magnitude, plus time (every 12.5 hours counts as ten times the votes).
    hot: (a, b) => hot(b) - hot(a),
    new: (a, b) => b.at - a.at,
    top: (a, b) => b.score - a.score || b.at - a.at,
  };
  const hot = p => Math.sign(p.score) * Math.log10(Math.max(Math.abs(p.score), 1)) + p.at / 45000000;

  // Posts (with their scores and comment counts) from these authors' tails, as far as these tails know.
  async function gather(authors, keep, readers, sort) {
    const self = await me();
    const rs = await roomsOf([...authors, ...readers]);
    const votes = votesIn(rs);
    const counts = new Map();
    for (const r of rs) for (const it of r.list()) if (it.kind === "comment") counts.set(postOf(it), (counts.get(postOf(it)) ?? 0) + 1);
    const byAuthor = await roomsOf(authors);
    const out = [];
    for (const r of byAuthor)
      for (const it of r.list()) {
        if (it.kind !== "post") continue;
        const ref = `${it.by}/${it.id}`;
        // A post from before titles: its first line is its title.
        const [first, ...rest] = it.body.split("\n");
        const board = /^[a-z0-9_]{2,24}$/.test(it.in ?? "") ? it.in : null;
        const p = { ref, id: it.id, by: it.by, title: it.title ?? first.slice(0, 300), body: it.title ? it.body : rest.join("\n").trim(), board, at: it.at, edited: it.edited };
        if (!keep(p)) continue;
        out.push({ ...p, comments: counts.get(ref) ?? 0, ...scored(ref, votes, self) });
      }
    return out.sort(SORTS[sort] ?? SORTS.hot);
  }

  async function list(where = {}, sort = "hot") {
    const self = await me();
    const follows = await following();
    if (where.board) {
      const b = boardName(where.board);
      const from = await fromPointers(boardRef(b));
      return gather([self, ...follows, ...from], p => p.board === b, [], sort);
    }
    if (where.by) return gather([where.by], () => true, [self, ...follows], sort);
    // HOME: whom this person follows and their own, and every post in the boards they joined.
    const joined = await boards();
    const from = (await Promise.all(joined.map(b => fromPointers(boardRef(b))))).flat();
    const people = new Set([self, ...follows]);
    const inBoards = new Set(joined);
    return gather([...people, ...from], p => people.has(p.by) || inBoards.has(p.board), [], sort);
  }

  async function get(ref) {
    const author = authorOf(ref);
    const from = await fromPointers(ref);
    return (await gather([author], p => p.ref === ref, [await me(), ...(await following()), ...from], "new"))[0] ?? null;
  }

  async function submit({ board, title, body }) {
    const b = boardName(board);
    title = String(title ?? "").trim();
    body = String(body ?? "").trim();
    if (!title) throw new Error("a post needs a title");
    if (title.length > 300) throw new Error("a title of at most 300 characters");
    const self = await me();
    const id = await (await mine()).post("post", body, { title, in: b });
    const ref = `${self}/${id}`;
    // Its own pointer bag, made now (nobody reading it waits on one that does not exist); and the board's pointer.
    await index.openPointers(ref).catch(e => ctx.log("posts", { what: `the pointer bag of ${id}: ${e.message}` }));
    await pointTo(boardRef(b));
    return ref;
  }

  // THE THREAD: the post's pointers resolved; its comments as a tree, best first (a reply whose parent is gone goes
  // to the top).
  async function thread(ref) {
    const self = await me();
    const rs = await roomsOf([authorOf(ref), self, ...(await following()), ...(await fromPointers(ref))]);
    const votes = votesIn(rs);
    const all = [];
    for (const r of rs) for (const it of r.list()) if (it.kind === "comment" && postOf(it) === ref) all.push({ ...it, ref: `${it.by}/${it.id}`, replies: [] });
    const byRef = new Map(all.map(c => [c.ref, Object.assign(c, scored(c.ref, votes, self))]));
    const top = [];
    for (const c of all) (c.re !== ref && byRef.get(c.re)?.replies ? byRef.get(c.re).replies : top).push(c);
    const best = (a, b) => b.score - a.score || a.at - b.at;
    const order = cs => (cs.sort(best), cs.forEach(c => order(c.replies)), cs);
    return order(top);
  }

  // Something of this person's in a place: a pointer to them there, dropped once per page.
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
    const id = await (await mine()).post("comment", body, { re: re ?? post, in: post });
    await pointTo(post);
    return `${await me()}/${id}`;
  }
  async function vote(ref, v, post = ref) {
    const r = await mine();
    const had = new Set(r.reactions().filter(x => x.item === ref).map(x => x.emoji));
    // Only what changes is written (taking back a vote that is not there writes nothing).
    for (const [e, on] of [[UP, v === 1], [DOWN, v === -1]]) if (on !== had.has(e)) await r.react(ref, e, on);
    if (v) await pointTo(post);
  }
  async function remove(ref) {
    const self = await me();
    if (!ref.startsWith(`${self}/`)) throw new Error("only its author removes it");
    await (await mine()).remove(ref.slice(self.length + 1));
  }

  // BOARDS this person joined: pins of `board:<name>` (their home shows them).
  const boards = async () => (await pins()).refs("board:").map(r => r.slice(6)).sort();
  const join = async (board, on) => (await pins()).set(boardRef(boardName(board)), on);

  return { submit, list, get, thread, comment, vote, remove, boards, join, boardName, onChange: f => changed.push(f) };
}
