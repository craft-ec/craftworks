// MODERATION, a capability: acts on OBJECTS (hide: a message, a channel, any row of any table in a space) and on
// ACTORS (remove a person from a space). It applies to every object alike: an object is named by its table and its
// key. Who may is `roles`' (moderate, remove); the acts are roles' acts, so every reader agrees on what counted.
// Nothing is deleted from anyone's feed (nobody writes another's): readers leave out what is hidden.
//
//   const m = await moderation.of(sp)
//   m.hidden(table)          // the Set of keys hidden in that table
//   await m.hide(table, key) // hidden for everyone (a moderator's act)
//   await m.remove(did)      // that person's nodes out of the space's group: a new epoch they cannot read
//   m.onChange(fn)
export async function start(ctx) {
  const [roles, keys] = await Promise.all(["roles", "keys"].map(n => ctx.require(n)));

  async function of(sp) {
    const r = await roles.of(sp);
    return {
      hidden: table => new Set(r.acts("hide").filter(a => a.table === table).map(a => a.item)),
      hide: (table, item) => r.act({ act: "hide", table, item }),
      async remove(did) {
        await r.refresh();
        const nodes = r.nodesOf(did);
        if (!nodes.length) throw new Error("they are not in this space");
        // The act first (what they wrote stays theirs: the act names their nodes), then their nodes out of the group.
        await r.act({ act: "remove", did, nodes: nodes.map(n => n.key) });
        await keys.group(r.space).remove(nodes.map(n => n.index));
        await r.refresh();
      },
      onChange: f => r.onChange(f),
      settled: r.settled,
    };
  }

  return { of };
}
