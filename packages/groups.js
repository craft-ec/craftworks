// GROUPS, a capability: "seen by just these people" — ONE implementation for every narrower audience: a person's
// circles (friends, followers), a channel only some may read (a role's holders, named people). A group is a SPACE (kind
// `server`, never on the spaces panel) whose MLS group seals what is written there to its members; its members are
// COMPUTED (`want`: who should be in it now) and KEPT IN STEP by whoever manages it — ITS LIST written (the missing put
// on it and told: their devices enter by themselves; the extra taken off: the account's node takes their keys out —
// FOUNDATION "Members and keys").
//
//   const groups = await ctx.require("groups");
//   const sp = await groups.of(key, { name, setup })   // this person's group for `key` (made the first time it is
//                                                      // wanted; `setup(sp, r)`: its first policies)
//   groups.find(key)                                   // the group for `key` this person is in (any owner), or null
//   await groups.keep(sp, want)                        // its members as `want` (a Set of DIDs) says: in, and out
//   (A group's own key: `circle` for a person's circles, as they were first made; `group` for every other.)
export async function start(ctx) {
  const [space, conversation, roles] = await Promise.all(["space", "conversation", "roles"].map(n => ctx.require(n)));
  const keyOf = sp => (sp.circle ? `circle:${sp.circle}` : (sp.group ?? null));
  const mine = async () => (await space.mine()).filter(s => keyOf(s));
  const find = async key => (await mine()).find(s => keyOf(s) === key) ?? null;
  const making = new Map();
  async function of(key, { name, setup = null } = {}) {
    const me = await space.me.strict();
    const had = (await mine()).find(s => keyOf(s) === key && space.madeBy(s, me));
    if (had) return had;
    if (!making.has(key))
      making.set(
        key,
        (async () => {
          const circle = key.startsWith("circle:") ? key.slice(7) : null;
          const sp = await space.create("server", name, circle ? { circle } : { group: key });
          if (setup) await setup(sp, await roles.of.strict(sp));
          ctx.log("groups", { what: `${name} made` });
          return sp;
        })().finally(() => making.delete(key)),
      );
    return making.get(key);
  }
  // ITS LIST as `want` says (this person and its owner always): the missing put on it, the extra taken off — by the
  // list, never by who holds keys now (one listed and not entered yet is not told again).
  async function keep(sp, want) {
    const me = await space.me.strict();
    if (!me) return;
    const r = await roles.of.strict(sp);
    await r.settled;
    const keepIn = new Set([me, r.owner].filter(Boolean));
    const have = new Set(r.roster());
    for (const did of want) if (!have.has(did) && !keepIn.has(did)) await conversation.invite(sp, did).catch(e => ctx.failed("groups", e, `${sp.name}: ${did.slice(12, 20)}… not added yet: ${e.message}`));
    const mod = await (await ctx.require("moderation")).of(sp);
    for (const did of have) if (!want.has(did) && !keepIn.has(did)) await mod.remove(did).catch(e => ctx.failed("groups", e, `${sp.name}: ${did.slice(12, 20)}… not removed yet: ${e.message}`));
  }
  return { of, find, keep, keyOf };
}
