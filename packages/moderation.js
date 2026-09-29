// MODERATION, a capability: acts on OBJECTS (hide: a message, a channel, any row of any table in a space) and on
// ACTORS (remove a person from a space). It applies to every object alike: an object is named by its table and its
// key. Who may is `roles`' (moderate, remove); the acts are roles' acts, so every reader agrees on what counted.
// Nothing is deleted from anyone's feed (nobody writes another's): readers leave out what is hidden.
//
//   const m = await moderation.of(sp)
//   m.hidden(table)          // the Set of keys hidden in that table
//   await m.hide(table, key) // hidden for everyone (a moderator's act)
//   await m.remove(did)      // that person out of the space's group (a new epoch they cannot read); back by an invite
//   await m.ban(did)         // the same, and never back (no invite or code lets them in) until `unban`
//   m.onChange(fn)
//
// MODERATION LISTS — the PUBLIC network's (Discover has no owner, so nobody's acts count there): a person's list is
// their public tail `modlist` (only their account writes it: signed by them), entries `<kind>:<ref>` → { reason, at },
// kind person | post | space. Each reader applies their own list and the lists of whom they chose (`edge` relation
// "modlist"): what is on any of them is left out of Discover.
//   const lists = await moderation.lists()
//   lists.flagged({ by, ref, space })   // on a list this reader applies (a post: its author, its ref, its space)
//   await lists.flag(kind, ref, reason)   await lists.unflag(kind, ref)   lists.mine()  lists.followed()  lists.onChange(fn)
export async function start(ctx) {
  const [roles, keys, directory] = await Promise.all(["roles", "keys", "directory"].map(n => ctx.require(n)));

  async function of(sp) {
    const r = await roles.of(sp);
    // OUT: the act first — naming their devices, so what they wrote stays theirs after they leave — then their member
    // out of the group. A ban of someone already out is the act alone.
    async function out(did, act) {
      await r.refresh();
      const nodes = r.nodesOf(did);
      if (!nodes.length && act !== "ban") throw new Error("they are not in this space");
      await r.act({ act, did, nodes: await directory.devices(did) });
      if (nodes.length) await keys.group(r.space).remove(nodes.map(n => n.index));
      await r.refresh();
    }
    return {
      hidden: table => new Set(r.acts("hide").filter(a => a.table === table).map(a => a.item)),
      hide: (table, item) => r.act({ act: "hide", table, item }),
      remove: did => out(did, "remove"),
      ban: did => out(did, "ban"),
      unban: did => r.act({ act: "unban", did }),
      // A BAN HOLDS in the group too: anyone banned still in the space's group (let in after the ban by someone who did
      // not know of it yet — the identity delegate, by a page's older mandate) taken out of it. How many.
      enforce: async () => {
        await r.refresh();
        const nodes = r.bannedList().flatMap(did => r.nodesOf(did));
        if (!nodes.length) return 0;
        await keys.group(r.space).remove(nodes.map(n => n.index));
        await r.refresh();
        return nodes.length;
      },
      onChange: f => r.onChange(f),
      settled: r.settled,
    };
  }

  let listsOpen = null;
  function lists() {
    return (listsOpen ??= (async () => {
      const [space, edge] = await Promise.all(["space", "edge"].map(n => ctx.require(n)));
      const people = await edge.people();
      const me = (await space.account()).id;
      const LIST = "modlist";
      const mineT = await directory.publicOf(me, LIST);
      const tails = new Map(); // did → tail
      const changed = [];
      const fire = () => changed.forEach(f => f());
      mineT.onChange(fire);
      const follow = async () => {
        for (const did of people.list("modlist"))
          if (!tails.has(did)) {
            const t = await directory.publicOf(did, LIST).catch(() => null);
            if (t) (tails.set(did, t), t.onChange(fire));
          }
      };
      await follow();
      people.onChange(() => follow().then(fire));
      const entries = () => {
        const on = new Set();
        for (const t of [mineT, ...[...tails.entries()].filter(([d]) => people.is("modlist", d)).map(([, t]) => t)])
          for (const r of t.rows()) if (r.value) on.add(r.key);
        return on;
      };
      return {
        flagged: ({ by = null, ref = null, space: sid = null } = {}) => {
          const on = entries();
          return (by && on.has(`person:${by}`)) || (ref && on.has(`post:${ref}`)) || (sid && on.has(`space:${sid}`));
        },
        flag: (kind, ref, reason = "") => mineT.put(`${kind}:${ref}`, JSON.stringify({ reason: String(reason).slice(0, 200), at: Date.now() })),
        unflag: (kind, ref) => mineT.remove(`${kind}:${ref}`),
        mine: () => mineT.rows().filter(r => r.value).length,
        // This person's own entries: [{ kind, ref }].
        entries: () =>
          mineT
            .rows()
            .filter(r => r.value)
            .map(r => ({ kind: r.key.slice(0, r.key.indexOf(":")), ref: r.key.slice(r.key.indexOf(":") + 1) })),
        followed: () => people.list("modlist"),
        onChange: f => changed.push(f),
      };
    })());
  }

  return { of, lists };
}
