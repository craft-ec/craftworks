// CIRCLES, a capability: a person's AUDIENCES as SPACES — the one implementation of "seen by just these people": their
// FRIENDS, their FOLLOWERS. A circle is a space they own (kind `server`, `circle: "friends" | "followers"`, never on
// the rail): its MLS group seals what is posted there to its members, each welcomed through their inbox, each removed
// by a `remove` act (the group moves to a new epoch: a removed friend reads nothing written after). Its place holds
// the items posted for it (`items.submit({ audience: "friends" })`), read in the members' feeds like any space's.
// Its owner posts; its members read, comment and vote (the same `roles` policies as any space).
// MEMBERS kept by itself (`upkeep`, every tick): friends are the mutual friends (`edge`: friend); followers whoever
// follows — a follow drops a notice in the followed person's inbox (`edge.people.set("follow")`), read here as
// `follower` — never asked, never approved.
//
//   const circles = await ctx.require("circles");
//   const sp = await circles.of("friends")   // the circle (made the first time it is wanted)
//   await circles.sync()                     // its members as they should be (upkeep)
export async function start(ctx) {
  const [space, conversation, roles, edge, index] = await Promise.all(["space", "conversation", "roles", "edge", "index"].map(n => ctx.require(n)));
  const AUDIENCES = { friends: { name: "Friends", relation: "friend" }, followers: { name: "Followers", relation: "follower" } };
  const mineOf = async which => {
    const me = (await space.account())?.id;
    return (await space.mine()).find(s => s.circle === which && s.governance?.owner === me) ?? null;
  };

  const making = new Map();
  async function of(which) {
    if (!AUDIENCES[which]) throw new Error(`no such audience: ${which}`);
    const had = await mineOf(which);
    if (had) return had;
    if (!making.has(which))
      making.set(
        which,
        (async () => {
          const me = await space.account();
          const handle = await (await ctx.require("directory")).name(me.id).catch(() => "");
          const sp = await space.create("server", `${AUDIENCES[which].name} of ${handle || "me"}`, { circle: which });
          const r = await roles.of(sp);
          // Its place (a board): its owner posts; its members read, comment and vote.
          await r.act({ act: "app", app: "board", on: true });
          await r.act({ act: "policy", path: "", action: "post", who: "admins" });
          ctx.log("circles", { what: `your ${which} circle made` });
          await sync().catch(() => {});
          return sp;
        })().finally(() => making.delete(which)),
      );
    return making.get(which);
  }

  // FOLLOW NOTICES in the inbox: each follower as `follower` (the newest notice of each person counts).
  async function takeFollows(people) {
    const items = await index.inbox().catch(() => []);
    const last = new Map();
    for (const it of items) if ((it.kind === "follow" || it.kind === "unfollow") && it.from && (last.get(it.from)?.at ?? 0) <= (it.at ?? 0)) last.set(it.from, it);
    for (const [did, it] of last) {
      const on = it.kind === "follow";
      if (people.is("follower", did) !== on) await people.set("follower", did, on, it.at ?? Date.now()).catch(() => {});
    }
  }

  // MEMBERS as they should be: each circle that exists — its relation's people in, everyone else out.
  async function sync() {
    const me = await space.account();
    if (!me) return;
    const people = await edge.people();
    await takeFollows(people);
    for (const [which, a] of Object.entries(AUDIENCES)) {
      const sp = await mineOf(which);
      if (!sp) continue;
      const want = new Set(people.list(a.relation).filter(d => d !== me.id && !people.is("block", d)));
      const r = await roles.of(sp);
      const have = new Set(r.members().map(m => m.did).filter(d => d !== me.id));
      for (const did of want) if (!have.has(did)) await conversation.invite(sp, did).catch(e => ctx.log("circles", { what: `${which}: ${did.slice(12, 20)}… not added yet: ${e.message}` }));
      const mod = await (await ctx.require("moderation")).of(sp);
      for (const did of have) if (!want.has(did)) await mod.remove(did).catch(e => ctx.log("circles", { what: `${which}: ${did.slice(12, 20)}… not removed yet: ${e.message}` }));
    }
  }
  return { of, sync };
}
