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
//   const room = await content.in(channel, { paged: true })   // READ BY PAGES: the newest first; room.older(n) for
//                                              // more, room.hasMore() — never the whole history (phase 3, Reads)
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
  // An item's ID SORTS BY TIME (phase 3, Reads): `t` ‖ its time (ms, base 36, 9 places) ‖ 8 random hex — so a table read
  // backwards from its end gives the newest first ("latest N", then older pages). `t` sorts after the random hex ids from
  // before and the reactions' `r-` keys: a scan of `t…` is the items made since.
  const newId = (at = Date.now()) => `t${Math.floor(at).toString(36).padStart(9, "0")}${[...crypto.getRandomValues(new Uint8Array(4))].map(x => x.toString(16).padStart(2, "0")).join("")}`;

  // Any conversation names its `messages` table and the `scope` (space) it lives in: a channel, its server; a direct
  // conversation, itself.
  async function tableOf(container, opts = {}) {
    // A space's PUBLIC table read from OUTSIDE (not a member): its writers are the public acts' (`roles.ofPublic`).
    // `writers` given (DISCOVER's pointers: who wrote and who answered): only those — still only those the space's roles
    // count as its writers.
    if (container?.outside) {
      const r = await (await ctx.require("roles")).ofPublic(container.scope);
      const keys = () => (container.writers ? container.writers.filter(k => r.writerKeys().includes(k)) : r.writerKeys());
      const t = storage.readOnly(container.messages, keys());
      r.onChange(() => t.add(keys()));
      return t;
    }
    if (container?.kind === "public") {
      const t = await (await ctx.require("directory")).publicOf(container.did, container.name);
      if (!t) throw new Error("that person has no key log yet");
      return t;
    }
    if (!container?.messages || !container.scope) throw new Error(`content does not live in a ${container?.kind ?? "nothing"}`);
    return storage.table(container.messages, container.scope, opts);
  }

  // `extra`: more items, already attributed ({ items(), onChange }: a space's outsiders, `items`) — through the same filter.
  async function in_(container, { paged = false, extra = null } = {}) {
    // PAGED (phase 3, Reads): the table opened LAZY — only the newest page read (`older()` for the next), never the
    // whole history. (A table read from outside or a person's public tail: read whole, as before.)
    paged = paged && !container?.outside && container?.kind !== "public";
    const t = await tableOf(container, { lazy: paged });
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
    // (An ATTACHING kind — a caption — is contributed like a comment.)
    const K = await ctx.require("kinds");
    const ACTION = { message: "post", comment: "comment", reaction: "vote", ...Object.fromEntries(K.all().map(k => [k, "post"])), ...Object.fromEntries(K.attaching().map(k => [k, "comment"])) };
    const [r, m] = outside
      ? await ctx.require("roles").then(async x => {
          const pr = await x.ofPublic(container.scope);
          // From outside, what moderation hid is the public acts' hides.
          return [pr, { hidden: table => new Set(pr.acts("hide").filter(a => a.table === table).map(a => a.item)) }];
        })
      : inSpace
        ? await Promise.all([ctx.require("roles").then(x => x.of(container.scope)), governed ? ctx.require("moderation").then(x => x.of(container.scope)) : null])
        : [null, null];
    // WHO MADE IT: its row's writer — or, a COLLABORATIVE item (a note) another member changed, its creator as the
    // version says, while that member may edit there (the space's `edit` policy then).
    const byOf = (row, v) => {
      const writer = (r ? r.author(row) : null) ?? v.by ?? null;
      if (!r || !v.editor || v.editor !== writer || !K.collaborative(v.kind) || typeof v.by !== "string") return writer;
      return R.mayWrite({ action: "edit", item: { kind: v.kind, by: v.by, meta: v.meta }, writer, r, at: Number(v.edited) || Infinity }) === true ? v.by : writer;
    };
    const item = row => {
      try {
        const v = JSON.parse(row.value);
        return {
          id: row.key,
          kind: K.canon(v.kind ?? "message"),
          body: String(v.body ?? v.text ?? ""),
          at: Number(v.at) || 0,
          by: open ? container.did : byOf(row, v),
          // Who last changed it, when not its creator (a collaborative item: `kinds.collaborative`).
          editor: !open && v.editor && r ? r.author(row) : null,
          re: typeof v.re === "string" ? v.re : null,
          title: typeof v.title === "string" ? v.title : null,
          // WHO SEES IT, chosen when made (`audience`): "members" keeps it to a space's members; none (from before):
          // as the space's policy reads.
          aud: typeof v.aud === "string" ? v.aud : null,
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
    // THE ROWS NOW: the whole table's — or, PAGED, the pages read so far with the table's own newer rows over them.
    const loaded = new Map(); // key → row, from pages
    let oldest = null; // the oldest item key read (the next page goes on below it)
    let more = true; // pages left (items with time ids, then the ones from before them)
    let timesDone = false;
    const rowsNow = () => {
      if (!paged) return t.rows();
      const all = new Map(loaded);
      for (const row of t.rows()) all.set(row.key, row);
      return [...all.values()];
    };
    // Its REACTIONS, read with each item (keyed `r-<item>-…`: a range per item).
    const withReactions = async rows => {
      const items = rows.filter(r => !r.key.startsWith("r-"));
      const rs = await Promise.all(items.map(r => t.page({ lo: `r-${r.key}-`, hi: `r-${r.key}.`, limit: 500 }).then(p => p.rows, () => [])));
      return [...rows, ...rs.flat()];
    };
    // THE NEXT PAGE, older than what is held: items with time ids newest first; then, once, those from before them.
    async function older(n = 50) {
      if (!paged || !more) return 0;
      let got = [];
      if (!timesDone) {
        const p = await t.page({ lo: "t", before: oldest ?? "", limit: n });
        got = p.rows;
        if (got.length) oldest = got.at(-1).key;
        if (!p.next) timesDone = true;
      }
      if (timesDone && !got.length) {
        // Items from before time ids (and their reactions): read once, whole.
        got = (await t.page({ hi: "t", limit: 100000 })).rows;
        more = false;
      }
      for (const row of await withReactions(got)) loaded.set(row.key, row);
      changed.forEach(f => f());
      return got.length;
    }
    // A TIME WINDOW read (paged): every item made since `ms` (its key range: ids sort by time), and their reactions.
    const timeKey = ms => `t${Math.max(0, Math.floor(ms)).toString(36).padStart(9, "0")}`;
    let since_ = Infinity;
    async function since(ms) {
      if (!paged || ms >= since_) return;
      const p = await t.page({ lo: timeKey(ms), hi: since_ === Infinity ? "" : timeKey(since_), limit: 100000 });
      for (const row of await withReactions(p.rows)) loaded.set(row.key, row);
      since_ = ms;
      if (!oldest || timeKey(ms) < oldest) oldest = timeKey(ms);
      changed.forEach(f => f());
    }
    // EVERYTHING (a thread opened: its comments may be anywhere): the table read whole from now on.
    async function loadAll() {
      if (!paged) return;
      await t.whole?.();
      paged = false;
      more = false;
    }
    // THIS PERSON's own rows, whole (their copies to keep in step), the rest as paged.
    const ownAll = async () => (paged ? t.ownWhole?.() : undefined);
    // CHANGED (a new row, a flush moving rows into the tree): what is held read again, from the newest to the oldest held.
    let refreshing = null;
    const refresh = () =>
      (refreshing ??= (async () => {
        if (!paged || !oldest) return;
        const p = await t.page({ lo: oldest, limit: 100000 });
        for (const row of await withReactions(p.rows)) loaded.set(row.key, row);
      })()
        .catch(() => {})
        .finally(() => (refreshing = null)));
    // An extra source changed (an outsider wrote): drawn again.
    extra?.onChange(() => changed.forEach(f => f()));
    const every = () => {
      const hidden = m ? m.hidden(container.messages) : new Set();
      const unseen = people.unseen();
      return [...rowsNow().filter(row => !hidden.has(row.key)).map(item), ...(extra?.items() ?? []).filter(it => !hidden.has(it.id) && !r?.banned?.(it.by))]
        .filter(it => it && !unseen.has(it.by))
        // A post or message by someone the setting did not allow when it was made: not counted.
        .filter((it, _, all) => !(governed && app && ACTION[it.kind] && !allowed(ACTION[it.kind], it.by, it, all, it.at)));
    };
    // WHICH RULE: a channel's (its path); on a board, THE ONE CHECK (`roles.mayWrite`, the same for a personal item):
    // posting by the space's policy for what is posted; a comment or a vote by the rule of the item it answers (its
    // own setting, else the policy for that item's domain), its credential cited where friends or followers may.
    const rootOf = (it, all) => {
      for (let x = it, depth = 0; x && depth < 8; depth++) {
        if (K.of(x.kind)) return x;
        const up = x.in ?? x.item ?? x.re;
        x = up ? all.find(y => y.id === up) : null;
      }
      return null;
    };
    const R = await ctx.require("roles");
    const allowed = (action, did, it, all, at) => {
      // A CHANNEL with its item: by that item's rule (its own, else the space's Chat policy — `roles.mayWrite`).
      if (container.kind === "channel" && container.item) return R.mayWrite({ action, item: container.item, writer: did, r, at }) === true;
      if (app !== "board") return r.allows(action, did, app, at);
      const root = action === "post" ? { kind: it.kind, by: null, meta: {} } : rootOf(it, all) ?? { kind: "post", by: null, meta: {} };
      return R.mayWrite({ action, item: root, writer: did, cred: it.meta?.cred ?? it.cred ?? null, r, at }) === true;
    };
    const reactionsOf = all => all.filter(x => x.kind === "reaction" && x.item && x.emoji && x.by).map(({ item, emoji, by, at, meta }) => ({ item, emoji, by, at, ...(meta?.cred ? { cred: meta.cred } : {}) }));
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
    // MAY CHANGE IT: its author — or, a COLLABORATIVE kind in a space, whoever its domain's `edit` policy allows.
    const mayEdit = it => it.by === me || (governed && app === "board" && K.collaborative(it.kind) && R.mayWrite({ action: "edit", item: it, writer: me, r }) === true);
    const mine = id => {
      const it = list().find(x => x.id === id);
      if (!it) throw new Error("no such item");
      if (!mayEdit(it)) throw new Error(K.collaborative(it.kind) ? "this space's setting does not let you edit it" : "only its author changes an item");
      return it;
    };
    const changed = [];
    t.onChange(() => (paged ? refresh().then(() => changed.forEach(f => f())) : changed.forEach(f => f())));
    // PAGED: the newest page read before it is handed out.
    const first = paged ? older(50) : Promise.resolve();
    r?.onChange(() => changed.forEach(f => f()));
    // A credential read (a friend's or a follower's): what it lets count, counted.
    R.onChecked(() => changed.forEach(f => f()));
    people.onChange(() => changed.forEach(f => f()));
    return {
      list,
      // PAGED: the next older page (how many items it brought); whether any are left.
      older,
      since,
      loadAll,
      ownAll,
      hasMore: () => paged && more,
      paged,
      reactions: () => reactionsOf(every()),
      settled: Promise.all([t.settled, r?.settled, first]).then(() => {}),
      onChange: f => changed.push(f),
      mayRemove: it => it.by === me || (governed && !!r?.can(me, "moderate")),
      // May this person post here now (the app's setting; a conversation: always).
      mayPost: (kind = "post") => !(governed && app) || allowed("post", me, { kind }, [], Infinity),
      may: (action, kind = "post") => !(governed && app) || allowed(action, me, { kind }, [], Infinity),
      // The rule of writing about ITEM `id` (a comment, a vote): may this person, and the credential they cite.
      mayAbout: async (id, action) => {
        const it = list().find(x => x.id === id);
        if (!(governed && app) || !it) return true;
        const cred = await R.credToCite(it, action, r).catch(() => false);
        return cred !== false && R.mayWrite({ action, item: it, writer: me, cred, r }) !== false;
      },
      mayEdit,
      postingRule: () => (governed && app ? r.policy(app, "post") : "members"),
      // `at`: when it was made (a note brought over from before: its own time).
      async post(kind, body, { re = null, title = null, in: where = null, files = [], meta = null, aud = null, at = Date.now() } = {}) {
        if (outside) throw new Error("only the space's members post here");
        const id = newId(at);
        await t.put(id, JSON.stringify({ kind, body, at, by: me, ...(re ? { re } : {}), ...(title ? { title } : {}), ...(where ? { in: where } : {}), ...(files.length ? { files } : {}), ...(meta && Object.keys(meta).length ? { meta } : {}), ...(aud ? { aud } : {}) }));
        return id;
      },
      // A REACTION: this person's, to one item, one emoji — its own row (the author in its key), put or taken back.
      // `cred`: a write credential (`circles`) where only friends or followers may vote.
      async react(id, emoji, on, { cred = null } = {}) {
        if (outside) throw new Error("only the space's members vote here");
        const code = [...emoji].map(c => c.codePointAt(0).toString(16)).join("-");
        const key = `r-${id}-${code}-${me.slice(12, 24)}`;
        if (on) await t.put(key, JSON.stringify({ kind: "reaction", item: id, emoji, at: Date.now(), by: me, ...(cred ? { meta: { cred } } : {}) }));
        else await t.remove(key);
      },
      async edit(id, body, { files = null, meta = null, title = undefined } = {}) {
        const it = mine(id);
        const fs = files ?? it.files ?? [];
        it.meta = meta ?? it.meta;
        if (title !== undefined) it.title = title;
        // Its creator kept (`by`); another member's change named (`editor`).
        await t.put(id, JSON.stringify({ kind: it.kind, body, at: it.at, by: it.by ?? me, ...(it.by && it.by !== me ? { editor: me } : {}), edited: Date.now(), ...(it.re ? { re: it.re } : {}), ...(it.title ? { title: it.title } : {}), ...(it.in ? { in: it.in } : {}), ...(fs.length ? { files: fs } : {}), ...(it.meta && Object.keys(it.meta).length ? { meta: it.meta } : {}), ...(it.aud ? { aud: it.aud } : {}) }));
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
