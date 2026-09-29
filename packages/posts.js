// POSTS, a capability: what people say in PUBLIC — a POST, the COMMENTS on it, the VOTES on it — and the FEED: the
// posts of the people this person follows, and their own. All of it is `content` (its one shape) in its AUTHOR's
// public tail `posts` (only their account writes it: whose tail it is, is who wrote it; anyone reads it): a post an item
// of kind "post"; a comment an item of kind "comment" answering the post (`re`: its ref); a vote a reaction ▲ or ▼ to
// it. A post is named by its REF, `<author did>/<id>`. A comment or a vote on a post also drops a pointer (`{ from }`)
// in the post's public bag (`index`), so the post's readers find it; the reader resolves each pointer in its author's
// tail (a pointer to nothing is nothing). Counts are what this reader resolved.
//
//   const posts = await ctx.require("posts");
//   await posts.post(body)                 // this person's new post: its ref
//   await posts.feed()                     // [post], newest first: this person's and whom they follow
//   await posts.of(did)                    // [post], newest first: one person's
//   await posts.thread(ref)                // { comments: [item], score, mine: 1 | -1 | 0 } (its pointers resolved)
//   await posts.comment(ref, body)   await posts.vote(ref, 1 | -1 | 0)
//   await posts.edit(ref, body)   await posts.remove(ref)   // this person's own post or comment (a comment's ref: the same shape)
//   posts.onChange(fn)                     // a tail read here changed
// A post: { ref, id, by, body, at, edited, comments, score, mine } — comments and score as far as known here.
export async function start(ctx) {
  const [content, index, space, edge] = await Promise.all(["content", "index", "space", "edge"].map(n => ctx.require(n)));
  const TAIL = "posts";
  const UP = "▲";
  const DOWN = "▼";
  const changed = [];
  const fire = () => changed.forEach(f => f());

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
  const me = async () => (await space.account()).id;
  const mine = async () => roomOf(await me());

  // What is known about a post from these people's tails: its comments and the votes on it.
  function about(ref, rs) {
    const comments = [];
    const votes = new Map(); // did → 1 | -1
    for (const r of rs) {
      for (const it of r.list()) if (it.kind === "comment" && it.re === ref) comments.push({ ...it, ref: `${it.by}/${it.id}` });
      for (const x of r.reactions()) {
        if (x.item !== ref || (x.emoji !== UP && x.emoji !== DOWN)) continue;
        // Both at once (a change half-made): neither counts.
        votes.set(x.by, votes.has(x.by) ? 0 : x.emoji === UP ? 1 : -1);
      }
    }
    comments.sort((a, b) => a.at - b.at || a.id.localeCompare(b.id));
    return { comments, votes };
  }
  const summed = votes => [...votes.values()].reduce((n, v) => n + v, 0);

  async function shaped(did) {
    const r = await roomOf(did).catch(() => null);
    return r ? r.list().filter(it => it.kind === "post").map(it => ({ ...it, ref: `${did}/${it.id}` })) : [];
  }
  // Posts with what the readers' known tails (the author's, this person's, whom they follow) say about them.
  async function dressed(list, dids) {
    const rs = (await Promise.all([...dids].map(d => roomOf(d).catch(() => null)))).filter(Boolean);
    const self = await me();
    return list
      .map(p => {
        const { comments, votes } = about(p.ref, rs);
        return { ...p, comments: comments.length, score: summed(votes), mine: votes.get(self) ?? 0 };
      })
      .sort((a, b) => b.at - a.at || b.id.localeCompare(a.id));
  }
  const following = async () => (await edge.people()).list("follow");

  async function feed() {
    const dids = [await me(), ...(await following())];
    const all = (await Promise.all(dids.map(shaped))).flat();
    return dressed(all, dids);
  }
  async function of(did) {
    return dressed(await shaped(did), new Set([did, await me(), ...(await following())]));
  }

  async function post(body) {
    body = String(body ?? "").trim();
    if (!body) throw new Error("a post needs something in it");
    const r = await mine();
    const id = await r.post("post", body);
    const ref = `${await me()}/${id}`;
    // Its pointer bag, made now: nobody reading it ever waits on one that does not exist.
    await index.openPointers(ref).catch(e => ctx.log("posts", { what: `the pointer bag of ${id}: ${e.message}` }));
    return ref;
  }

  // THE THREAD: the post's pointers resolved, with every tail already known here.
  async function thread(ref) {
    const author = ref.slice(0, ref.lastIndexOf("/"));
    const from = new Set([author, await me(), ...(await following())]);
    for (const p of await index.pointers(ref).catch(() => [])) if (typeof p.from === "string" && p.from.startsWith("did:craftec:")) from.add(p.from);
    const rs = (await Promise.all([...from].map(d => roomOf(d).catch(() => null)))).filter(Boolean);
    const { comments, votes } = about(ref, rs);
    return { comments, score: summed(votes), mine: votes.get(await me()) ?? 0 };
  }

  // Something of this person's about someone else's post: a pointer to them, dropped once per page.
  const pointed = new Set();
  async function pointTo(ref) {
    const self = await me();
    if (ref.startsWith(`${self}/`) || pointed.has(ref)) return;
    await index.point(ref, { from: self });
    pointed.add(ref);
  }

  async function comment(ref, body) {
    body = String(body ?? "").trim();
    if (!body) throw new Error("a comment needs something in it");
    const id = await (await mine()).post("comment", body, { re: ref });
    await pointTo(ref);
    return id;
  }
  async function vote(ref, v) {
    const r = await mine();
    const had = new Set(r.reactions().filter(x => x.item === ref).map(x => x.emoji));
    // Only what changes is written (taking back a vote that is not there writes nothing).
    for (const [e, on] of [[UP, v === 1], [DOWN, v === -1]]) if (on !== had.has(e)) await r.react(ref, e, on);
    if (v) await pointTo(ref);
  }
  async function remove(ref) {
    const self = await me();
    if (!ref.startsWith(`${self}/`)) throw new Error("only its author removes it");
    await (await mine()).remove(ref.slice(self.length + 1));
  }

  return { post, feed, of, thread, comment, vote, remove, edit: async (ref, body) => (await mine()).edit(ref.slice(ref.lastIndexOf("/") + 1), body), onChange: f => changed.push(f) };
}
