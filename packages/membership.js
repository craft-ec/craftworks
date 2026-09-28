// MEMBERSHIP, a capability: who belongs. For your ACCOUNT, its members are its NODES — and the one list of them is the
// account's MLS group (the `keys` capability): each member's credential names its node key and is signed by an owner
// key of the account. There is no second list to fall out of step. (Spaces' members come here too, with spaces.)
//
//   const membership = await ctx.require("membership");
//   await membership.nodes()   // [{ index, key, me }] — null where the account's group is not kept (not the home site)
export async function start(ctx) {
  const keys = await ctx.require("keys");

  async function nodes() {
    const st = await keys.ready();
    if (!st) return null;
    return st.members.map(m => ({ index: m.index, key: m.key, me: m.index === st.me }));
  }

  return { nodes };
}
