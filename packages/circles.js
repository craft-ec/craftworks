// CIRCLES, a capability: a person's AUDIENCES as SPACES — the one implementation of "seen by just these people": their
// FRIENDS, their FOLLOWERS. A circle is a space they own (kind `server`, `circle: "friends" | "followers"`, never on
// the spaces panel): its MLS group seals what is posted there to its members, each welcomed through their inbox, each removed
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
//   (Checked by every reader, cited by every writer: `roles.mayWrite`, `roles.credToCite`.)
//
// WRITE CREDENTIALS — who may comment or vote where only friends or followers may, provable to ANY reader without the
// list ever published: per friend or follower, an UNLISTED public table of the owner's account at a random name
// (`directory.credTable`: named by the token), its row `for` naming the holder's DID and circle — a signed table write like any other. The token
// reaches the holder through their inbox; a comment or vote cites it; a reader reads that table and checks it names
// the writer. Never listed (card, catalog): not found by anyone not told it. A friend no longer: the table cleared.
export async function start(ctx) {
  const [space, roles, edge, index] = await Promise.all(["space", "roles", "edge", "index"].map(n => ctx.require(n)));
  const AUDIENCES = { friends: { name: "Friends", relation: "friend" }, followers: { name: "Followers", relation: "follower" } };
  const mineOf = async which => {
    const me = (await space.account())?.id;
    return (await space.mine()).find(s => s.circle === which && s.governance?.owner === me) ?? null;
  };

  // A CIRCLE is a GROUP (`groups`: the one "seen by just these people"), keyed by its audience.
  const groups = await ctx.require("groups");
  async function of(which) {
    if (!AUDIENCES[which]) throw new Error(`no such audience: ${which}`);
    const had = await mineOf(which);
    if (had) return had;
    const handle = await (await ctx.require("directory")).name((await space.account()).id).catch(() => "");
    const sp = await groups.of(`circle:${which}`, {
      name: `${AUDIENCES[which].name} of ${handle || "me"}`,
      // Its place (a board): its owner posts; its members read, comment and vote.
      setup: async (_, r) => {
        await r.act({ act: "app", app: "board", on: true });
        await r.act({ act: "policy", path: "", action: "post", who: "admins" });
      },
    });
    await sync().catch(() => {});
    return sp;
  }

  // FOLLOW NOTICES in the inbox: each follower as `follower` (the newest notice of each person counts).
  // A follower COUNTS by this person's rule (`roles.followable`: anyone · friends · nobody — `mayWrite`, the check a
  // comment passes, its credential cited): asked every pass, so a rule made narrower drops who no longer meets it.
  async function takeFollows(people) {
    const items = await index.inbox().catch(() => []);
    const mine = await roles.followable((await space.account()).id);
    const last = new Map();
    for (const it of items) if ((it.kind === "follow" || it.kind === "unfollow") && it.from && (last.get(it.from)?.at ?? 0) <= (it.at ?? 0)) last.set(it.from, it);
    for (const [did, it] of last) {
      const on = it.kind === "follow" ? roles.mayWrite({ action: "follow", item: mine, writer: did, cred: it.cred ?? null }) : false;
      if (on === null) continue; // its credential not read yet: the next pass
      if (people.is("follower", did) !== on) await people.set("follower", did, on, it.at ?? Date.now()).catch(() => {});
    }
  }

  const hexOf = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  const credTable = (owner, token) => ctx.require("directory").then(d => d.credTable(owner, token));
  // ISSUED: each of the relation's people holds one; anyone no longer one, theirs cleared.
  async function issue(people, me, which, want) {
    const rel = `cred-${which}`;
    for (const did of want) {
      if (people.is(rel, did)) continue;
      const token = hexOf(crypto.getRandomValues(new Uint8Array(16)));
      await (await credTable(me.id, token)).put("for", JSON.stringify({ did, circle: which, at: Date.now() }));
      await people.set(rel, did, true, Date.now(), { token });
      await index.send(did, { kind: "cred", circle: which, token, from: me.id, at: Date.now() }).catch(e => ctx.log("circles", { what: `handing ${did.slice(12, 20)}… their ${which} credential: ${e.message}` }));
    }
    for (const did of people.list(rel)) {
      if (want.has(did)) continue;
      const token = people.about(rel, did)?.token;
      if (token) await (await credTable(me.id, token)).remove("for").catch(() => {});
      await people.set(rel, did, false);
    }
  }
  // HELD: the credentials others handed this person (the newest of each).
  async function takeCreds(people) {
    for (const it of await index.inbox().catch(() => [])) {
      if (it.kind !== "cred" || !it.from || !AUDIENCES[it.circle] || !/^[0-9a-f]{32}$/.test(it.token ?? "")) continue;
      const rel = `credin-${it.circle}`;
      if (people.about(rel, it.from)?.token !== it.token && (people.at(rel, it.from) || 0) <= (it.at || 0)) await people.set(rel, it.from, true, it.at || Date.now(), { token: it.token });
    }
  }
  // MEMBERS as they should be: each circle that exists — its relation's people in, everyone else out.
  async function sync() {
    const me = await space.account();
    if (!me) return;
    const people = await edge.people();
    await takeFollows(people);
    await takeCreds(people);
    // Write credentials: one per friend and per follower, whether or not their circle was made.
    for (const [which, a] of Object.entries(AUDIENCES)) await issue(people, me, which, new Set(people.list(a.relation).filter(d => d !== me.id && !people.is("block", d))));
    for (const [which, a] of Object.entries(AUDIENCES)) {
      const sp = await mineOf(which);
      if (!sp) continue;
      await groups.keep(sp, new Set(people.list(a.relation).filter(d => d !== me.id && !people.is("block", d))));
    }
  }
  return { of, sync };
}
