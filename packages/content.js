// CONTENT, a capability: the SHAPE of an authored item — a message, a post, a comment, a note — defined once, for
// every container it lives in (a channel, a thread, a notebook). UI never defines its own: a page shows content.
//
// An item is `{ id, kind, body, at, by }`: its own id, what kind it is ("message", …), its body (text), when it was
// made, and its author (a DID). It lives in its CONTAINER's table — each item in its author's own feed (`storage`), so
// nobody writes into another's; readers see every author's merged. In a space its author is its feed's WRITER, as the
// space's roles know it (never what the item claims). Only its author edits or removes it; removing another's is a
// moderator's hide (`moderation`), and what is hidden is left out.
//
//   const content = await ctx.require("content");
//   const room = await content.in(channel)     // a container: a channel (a sub-space), or the account (none)
//   room.list()                                // [{ id, kind, body, at, by }], oldest first
//   await room.post("message", "hello")        // the new item's id
//   await room.edit(id, body)   await room.remove(id)   room.onChange(fn)
//   room.mayRemove(item)                      // its author, or a moderator here
//   await room.settled                        // every author's feed tried once (more may still arrive)
export async function start(ctx) {
  const storage = await ctx.require("storage");
  const space = await ctx.require("space");
  const newId = () => [...crypto.getRandomValues(new Uint8Array(8))].map(x => x.toString(16).padStart(2, "0")).join("");

  // Any conversation names its `messages` table and the `scope` (space) it lives in: a channel, its server; a direct
  // conversation, itself.
  async function tableOf(container) {
    if (!container?.messages || !container.scope) throw new Error(`content does not live in a ${container?.kind ?? "nothing"}`);
    return storage.table(container.messages, container.scope);
  }

  async function in_(container) {
    const t = await tableOf(container);
    const me = (await space.account()).id;
    // In a space (not the account): its roles (who wrote what) and its moderation (what is hidden).
    const inSpace = container.scope.kind !== "account";
    const [r, m] = inSpace
      ? await Promise.all([ctx.require("roles").then(x => x.of(container.scope)), ctx.require("moderation").then(x => x.of(container.scope))])
      : [null, null];
    const item = row => {
      try {
        const v = JSON.parse(row.value);
        return { id: row.key, kind: v.kind ?? "message", body: String(v.body ?? v.text ?? ""), at: Number(v.at) || 0, by: (r ? r.author(row) : null) ?? v.by ?? null };
      } catch {
        return null;
      }
    };
    const list = () => {
      const hidden = m ? m.hidden(container.messages) : new Set();
      return t
        .rows()
        .filter(row => !hidden.has(row.key))
        .map(item)
        .filter(Boolean)
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
    return {
      list,
      settled: Promise.all([t.settled, r?.settled]).then(() => {}),
      onChange: f => changed.push(f),
      mayRemove: it => it.by === me || !!r?.can(me, "moderate"),
      async post(kind, body) {
        const id = newId();
        await t.put(id, JSON.stringify({ kind, body, at: Date.now(), by: me }));
        return id;
      },
      async edit(id, body) {
        const it = mine(id);
        await t.put(id, JSON.stringify({ kind: it.kind, body, at: it.at, by: me, edited: Date.now() }));
      },
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
