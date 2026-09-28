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
      onChange: f => r.onChange(f),
      settled: r.settled,
    };
  }

  return { of };
}
