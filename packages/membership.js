// MEMBERSHIP, a capability: who belongs. For your ACCOUNT, its members are its NODES — and the one list of them is the
// account's MLS group (the `keys` capability): each member's credential names its node key and is signed by an owner
// key of the account. There is no second list to fall out of step. (Spaces' members come here too, with spaces.)
//
//   const membership = await ctx.require("membership");
//   await membership.nodes()   // [{ index, key, me }] — null where the account's group is not kept (not the home site)
//   await membership.remove(index)
//   await membership.writers() // the node keys (hex) whose feeds count; null where the group is not kept
export async function start(ctx) {
  const keys = await ctx.require("keys");

  async function nodes() {
    const st = await keys.ready();
    if (!st) return null;
    if (st.removed) return "removed"; // this node was removed from the account
    return st.members.map(m => ({ index: m.index, key: m.key, me: m.index === st.me }));
  }

  // Remove a node from the account (by its index in the group): it reads nothing written afterwards.
  const remove = index => keys.remove(index);

  // The WRITERS of the account's tables: its current nodes (a removed node's feed stops counting).
  async function writers() {
    const st = await keys.ready();
    if (!st || st.removed) return null;
    return st.members.map(m => m.key);
  }

  return { nodes, remove, writers };
}
