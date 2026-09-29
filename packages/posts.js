// POSTS, a capability: posts Reddit-shaped — a POST (a title and text), COMMENTS on it (answering the post or another
// comment), VOTES on either — in one of two places:
// - A BOARD: a SPACE's (`space.board(sp)`), one of its apps (added by its owner or admins) beside its messages — a
//   server's, a group or direct conversation's — the same members, roles, governance and moderation. Its posts
//   are `content` in the space's table `board`: members write, moderators hide, what is hidden is left out, and a
//   person removed or banned there writes no more. Joined by joining the space (an invite, a welcome).
// - A person's PROFILE: `content` in their own public tail `posts` (only their account writes it; anyone reads it) —
//   what their FOLLOWERS see. A comment or a vote on someone's profile post is in the commenter's own tail, with a
//   pointer (`{ from }`) in the post's public bag (`index`) so the post's readers find it.
// A post is `content` of kind "post" (`title`); a comment of kind "comment" (`re`: what it answers, `in`: its post); a
// vote a reaction ▲ or ▼. REFS: `space:<server id>/<id>` on a board, `<author did>/<id>` on a profile.
//
//   const posts = await ctx.require("posts");
//   await posts.submit({ board, title, body })       // `board`: a space's id, or none (this person's profile): its ref
//   await posts.list({ board } | { by } | {}, sort)   // a board's, a profile's, or HOME (this person's boards, and the
//                                                     // profiles they follow and their own); sort "hot" | "new" | "top"
//   await posts.get(ref)   await posts.thread(ref)    // one post; its comments as a tree (`replies`), best first
//   await posts.comment(post, re, body)   await posts.vote(ref, 1 | -1 | 0, post)   await posts.remove(ref)
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
  const changed = [];
  const fire = () => changed.forEach(f => f());
  const me = async () => (await space.account()).id;
  const following = async () => (await edge.people()).list("follow");

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
  const boards = async () => {
    const all = await space.mine();
    const on = await Promise.all(all.map(sp => roles.of(sp).then(r => r.apps().includes("board"), () => false)));
    return all.filter((_, i) => on[i]);
  };
  const boardOf = async x => {
    const id = String(x).startsWith("space:") ? String(x).slice(6, String(x).indexOf("/")) : x;
    return (await boards()).find(s => s.id === id) ?? null;
  };
  const boardRoom = sp => opened(`space:${sp.id}`, () => content.in(space.board(sp)));
  const profileRoom = did => opened(did, () => content.in({ kind: "public", did, name: TAIL }));
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
    return { ref, id: it.id, by: it.by, title: it.title ?? first.slice(0, 300), body: it.title ? it.body : rest.join("\n").trim(), board, at: it.at, edited: it.edited };
  };

  // A BOARD's posts: everything is in its one room (reactions keyed by the item's id).
  async function boardPosts(sp) {
    const r = await boardRoom(sp);
    const self = await me();
    const items = r.list();
    const votes = tally(r.reactions());
    const counts = new Map();
    for (const it of items) if (it.kind === "comment") counts.set(postOf(it), (counts.get(postOf(it)) ?? 0) + 1);
    return items
      .filter(it => it.kind === "post")
      .map(it => ({ ...shape(it, `space:${sp.id}/${it.id}`, { id: sp.id, name: sp.name }), comments: counts.get(it.id) ?? 0, ...scored(it.id, votes, self), mayRemove: r.mayRemove(it) }));
  }
  // PROFILE posts: from their authors' tails; comments and votes from the tails known here (the reader's, whom they
  // follow, and whoever `readers` names).
  async function profilePosts(authors, readers = []) {
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
        if (it.kind !== "post") continue;
        const ref = `${it.by}/${it.id}`;
        out.push({ ...shape(it, ref, null), comments: counts.get(ref) ?? 0, ...scored(ref, votes, self), mayRemove: it.by === self });
      }
    return out;
  }

  const hot = p => Math.sign(p.score) * Math.log10(Math.max(Math.abs(p.score), 1)) + p.at / 45000000; // Reddit's
  const SORTS = { hot: (a, b) => hot(b) - hot(a), new: (a, b) => b.at - a.at, top: (a, b) => b.score - a.score || b.at - a.at };

  async function list(where = {}, sort = "hot") {
    let out;
    if (where.board) {
      const sp = await boardOf(where.board);
      if (!sp) throw new Error("you are not in that board's space: join it with an invite");
      out = await boardPosts(sp);
    } else if (where.by) out = await profilePosts([where.by]);
    else {
      const [bs, people] = await Promise.all([boards(), following()]);
      out = [...(await Promise.all(bs.map(sp => boardPosts(sp).catch(() => [])))).flat(), ...(await profilePosts([await me(), ...people]))];
    }
    return out.sort(SORTS[sort] ?? SORTS.hot);
  }

  async function get(ref) {
    if (ref.startsWith("space:")) {
      const sp = await boardOf(ref);
      return sp ? ((await boardPosts(sp)).find(p => p.ref === ref) ?? null) : null;
    }
    return (await profilePosts([whereOf(ref)], await pointersTo(ref))).find(p => p.ref === ref) ?? null;
  }

  async function submit({ board = null, title, body }) {
    title = String(title ?? "").trim();
    body = String(body ?? "").trim();
    if (!title) throw new Error("a post needs a title");
    if (title.length > 300) throw new Error("a title of at most 300 characters");
    if (board) {
      const sp = await boardOf(board);
      if (!sp) throw new Error("you are not in that board's space");
      return `space:${sp.id}/${await (await boardRoom(sp)).post("post", body, { title })}`;
    }
    const self = await me();
    const ref = `${self}/${await (await profileRoom(self)).post("post", body, { title })}`;
    // Its pointer bag, made now: nobody reading it waits on one that does not exist.
    await index.openPointers(ref).catch(e => ctx.log("posts", { what: `the pointer bag of ${ref}: ${e.message}` }));
    return ref;
  }

  // THE THREAD: comments as a tree, best first (a reply whose parent is gone goes to the top).
  async function thread(ref) {
    const self = await me();
    const all = [];
    if (ref.startsWith("space:")) {
      const sp = await boardOf(ref);
      if (!sp) return [];
      const r = await boardRoom(sp);
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
    await (await profileRoom(await me())).post("comment", body, { re: re ?? post, in: post });
    await pointTo(post);
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
    if (v && !onBoard) await pointTo(post);
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

  return { submit, list, get, thread, comment, vote, remove, boards, boardOf, onChange: f => changed.push(f) };
}
