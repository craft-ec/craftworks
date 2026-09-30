// CONTENT, a capability: the SHAPE of an authored item — a message, a post, a comment, a note — defined once, for
// every container it lives in (a channel, a thread, a notebook). UI never defines its own: a page shows content.
//
// An item is `{ id, kind, body, at, by, re, title, in, edited, reactions, files }`: its own id, what kind it is ("message", …),
// its body (text), when it was made, its author (a DID), the item it answers (`re`), a title (a post's), where it was
// put when that is not its table (`in`: a post's board, a comment's post), when it was last edited, and the
// REACTIONS to it (`{ "👍": [did…] }`) — each a small item of kind "reaction" in the same table, keyed by the item,
// the emoji and its author (so two people's never meet in one row), never listed itself. It lives in its CONTAINER's table — each item in its author's own feed (`storage`), so
// nobody writes into another's; readers see every author's merged. In a space its author is its feed's WRITER, as the
// space's roles know it (never what the item claims). Only its author edits or removes it; removing another's is a
// moderator's hide (`moderation`), and what is hidden is left out.
//
//   const content = await ctx.require("content");
//   const room = await content.in(channel)     // a container: a channel (a sub-space), or the account (none)
//   room.list()                                // [{ id, kind, body, at, by }], oldest first
//   await room.post("message", "hello")        // the new item's id
//   await room.post("message", "hi", { re })  // a REPLY: `re` the id of the item it answers
//   await room.post("post", body, { title, in })  // a titled item, put in a place (`in`) its table is not
//   await room.react(id, "👍", on)             // this person's reaction to an item, on or off
//   await room.edit(id, body)   await room.remove(id)   room.onChange(fn)
//   room.mayRemove(item)                      // its author, or a moderator here
//   room.mayPost()   room.postingRule()        // this person may post here now; "everyone" | "admins" (the app's setting)
//   room.reactions()                           // every reaction row here [{ item, emoji, by }] (also to items elsewhere)
//   room.own()   await room.putOwn(key, value)   await room.dropOwn(key)   // this person's rows as stored (a copy's)
//   await room.settled                        // every author's feed tried once (more may still arrive)
//
// A PUBLIC container `{ kind: "public", did, name }` is one person's public tail `name` (`directory.publicOf`): only
// their account writes it, so every item in it is THEIRS, whatever it claims; anyone reads it. Their reactions there
// may name items anywhere (`posts` keeps votes and comments on others' posts in the voter's own tail).
export async function start(ctx) {
  const storage = await ctx.require("storage");
  const space = await ctx.require("space");
  const newId = () => [...crypto.getRandomValues(new Uint8Array(8))].map(x => x.toString(16).padStart(2, "0")).join("");

  // Any conversation names its `messages` table and the `scope` (space) it lives in: a channel, its server; a direct
  // conversation, itself.
  async function tableOf(container) {
    // A space's PUBLIC table read from OUTSIDE (not a member): its writers are the public acts' (`roles.ofPublic`).
    if (container?.outside) {
      const r = await (await ctx.require("roles")).ofPublic(container.scope);
      const t = storage.readOnly(container.messages, r.writerKeys());
      r.onChange(() => t.add(r.writerKeys()));
      return t;
    }
    if (container?.kind === "public") {
      const t = await (await ctx.require("directory")).publicOf(container.did, container.name);
      if (!t) throw new Error("that person has no key log yet");
      return t;
    }
    if (!container?.messages || !container.scope) throw new Error(`content does not live in a ${container?.kind ?? "nothing"}`);
    return storage.table(container.messages, container.scope);
  }

  async function in_(container) {
    const t = await tableOf(container);
    const me = (await space.account()).id;
    // In a space (not the account): its roles (who wrote what); in a SHARED space (a server) its moderation too (what is
    // hidden). A conversation (direct, group) is between equals: nobody moderates another's items.
    const open = container.kind === "public";
    const outside = !!container.outside;
    const inSpace = !open && (outside || container.scope.kind !== "account");
    const governed = inSpace && (outside || container.scope.kind === "server");
    // Its PATH for access (a channel: `chat/<id>`; a board: `board`): the space's policies there say who may post,
    // comment and vote (inherited from the app and the space).
    const app = container.kind === "channel" ? `chat/${container.id.split("/").pop()}` : container.kind === "board" ? "board" : null;
    // Anything that stands on its own (a post, a video, …: `kinds`) is posted; a comment commented; a reaction voted.
    const ACTION = { message: "post", comment: "comment", reaction: "vote", ...Object.fromEntries((await ctx.require("kinds")).all().map(k => [k, "post"])) };
    const [r, m] = outside
      ? await ctx.require("roles").then(async x => {
          const pr = await x.ofPublic(container.scope);
          // From outside, what moderation hid is the public acts' hides.
          return [pr, { hidden: table => new Set(pr.acts("hide").filter(a => a.table === table).map(a => a.item)) }];
        })
      : inSpace
        ? await Promise.all([ctx.require("roles").then(x => x.of(container.scope)), governed ? ctx.require("moderation").then(x => x.of(container.scope)) : null])
        : [null, null];
    const item = row => {
      try {
        const v = JSON.parse(row.value);
        return {
          id: row.key,
          kind: v.kind ?? "message",
          body: String(v.body ?? v.text ?? ""),
          at: Number(v.at) || 0,
          by: open ? container.did : ((r ? r.author(row) : null) ?? v.by ?? null),
          re: typeof v.re === "string" ? v.re : null,
          title: typeof v.title === "string" ? v.title : null,
          in: typeof v.in === "string" ? v.in : null,
          edited: Number(v.edited) || 0,
          item: typeof v.item === "string" ? v.item : null,
          emoji: typeof v.emoji === "string" ? v.emoji : null,
          // FILES on it: their references (`files`; shown by `attachments`).
          files: Array.isArray(v.files) ? v.files.filter(f => f && typeof f === "object").slice(0, 20) : [],
          // Its kind's own FIELDS (`kinds`: a movie's year, an episode's season …).
          meta: v.meta && typeof v.meta === "object" && !Array.isArray(v.meta) ? v.meta : {},
        };
      } catch {
        return null;
      }
    };
    // What this person does not see: what moderation hid (for everyone), and whom they hid or blocked (for them).
    const people = await (await ctx.require("edge")).people();
    const every = () => {
      const hidden = m ? m.hidden(container.messages) : new Set();
      const unseen = people.unseen();
      return t
        .rows()
        .filter(row => !hidden.has(row.key))
        .map(item)
        .filter(it => it && !unseen.has(it.by))
        // A post or message by someone the app's setting did not allow when it was made: not counted.
        .filter(it => !(governed && app && ACTION[it.kind] && !r.allows(ACTION[it.kind], it.by, app, it.at)));
    };
    const reactionsOf = all => all.filter(x => x.kind === "reaction" && x.item && x.emoji && x.by).map(({ item, emoji, by }) => ({ item, emoji, by }));
    const list = () => {
      const all = every();
      // Reactions gathered onto the items they react to.
      const reactions = new Map();
      for (const x of all) {
        if (x.kind !== "reaction" || !x.item || !x.emoji || !x.by) continue;
        const on = reactions.get(x.item) ?? {};
        (on[x.emoji] ??= []).includes(x.by) || on[x.emoji].push(x.by);
        reactions.set(x.item, on);
      }
      return all
        .filter(x => x.kind !== "reaction")
        .map(x => ({ ...x, reactions: reactions.get(x.id) ?? {} }))
        .sort((a, b) => a.at - b.at || a.id.localeCompare(b.id));
    };
    const mine = id => {
      const it = list().find(x => x.id === id);
      if (!it) throw new Error("no such item");
      if (it.by !== me) throw new Error("only its author changes an item");
      return it;
    };
    const changed = [];
    t.onChange(() => changed.forEach(f => f()));
    r?.onChange(() => changed.forEach(f => f()));
    people.onChange(() => changed.forEach(f => f()));
    return {
      list,
      reactions: () => reactionsOf(every()),
      settled: Promise.all([t.settled, r?.settled]).then(() => {}),
      onChange: f => changed.push(f),
      mayRemove: it => it.by === me || (governed && !!r?.can(me, "moderate")),
      // May this person post here now (the app's setting; a conversation: always).
      mayPost: () => !(governed && app) || r.allows("post", me, app),
      may: action => !(governed && app) || r.allows(action, me, app),
      postingRule: () => (governed && app ? r.policy(app, "post") : "members"),
      async post(kind, body, { re = null, title = null, in: where = null, files = [], meta = null } = {}) {
        if (outside) throw new Error("only the space's members post here");
        const id = newId();
        await t.put(id, JSON.stringify({ kind, body, at: Date.now(), by: me, ...(re ? { re } : {}), ...(title ? { title } : {}), ...(where ? { in: where } : {}), ...(files.length ? { files } : {}), ...(meta && Object.keys(meta).length ? { meta } : {}) }));
        return id;
      },
      // A REACTION: this person's, to one item, one emoji — its own row (the author in its key), put or taken back.
      async react(id, emoji, on) {
        if (outside) throw new Error("only the space's members vote here");
        const code = [...emoji].map(c => c.codePointAt(0).toString(16)).join("-");
        const key = `r-${id}-${code}-${me.slice(12, 24)}`;
        if (on) await t.put(key, JSON.stringify({ kind: "reaction", item: id, emoji, at: Date.now(), by: me }));
        else await t.remove(key);
      },
      async edit(id, body, { files = null } = {}) {
        const it = mine(id);
        const fs = files ?? it.files ?? [];
        await t.put(id, JSON.stringify({ kind: it.kind, body, at: it.at, by: me, edited: Date.now(), ...(it.re ? { re: it.re } : {}), ...(it.title ? { title: it.title } : {}), ...(it.in ? { in: it.in } : {}), ...(fs.length ? { files: fs } : {}), ...(it.meta && Object.keys(it.meta).length ? { meta: it.meta } : {}) }));
      },
      // Its FILES replaced (a video's manifest, once more renditions are made): the author's, the item otherwise as is.
      async setFiles(id, files) {
        const it = mine(id);
        await this.edit(id, it.body, { files });
      },
      // THIS PERSON's ROWS as stored (items, reactions): to copy them into another table of the same place (a board's
      // public table: `posts`), and to put or drop one there by its key — only ever this person's own.
      own: () => t.rows().filter(row => row.value && (open ? true : r ? r.author(row) === me : true)).map(row => ({ key: row.key, value: row.value })),
      putOwn: (key, value) => {
        if (outside) throw new Error("only the space's members write here");
        return t.put(key, value);
      },
      dropOwn: key => t.remove(key),
      async remove(id) {
        const it = list().find(x => x.id === id);
        if (!it) throw new Error("no such item");
        if (it.by === me) return t.remove(id);
        if (!m) throw new Error("only its author removes an item");
        await m.hide(container.messages, id);
      },
    };
  }

  return { in: in_ };
}
