// CONVERSATION, a capability: people talking. KINDS only where the mechanism differs:
// - `direct`: two people. Their own space and group; the first contact is a WELCOME dropped in the other's inbox (they
//   may be away: their card's key packages let them be added anyway). Each person's messages in their own feed.
// - `group`: a space's members (a server; its channels are sub-spaces). Joining by invite. (Its page is Chat.)
// - `mail`: items addressed per item, sealed to each recipient, delivered to their inboxes. (Later.)
// An announcement is a group message where only some roles post; a thread is items replying to items — compositions,
// not kinds. The items themselves are `content`'s.
//
//   const conversation = await ctx.require("conversation");
//   await conversation.direct(did)   // a direct conversation with that person (made, and they are welcomed)
//   await conversation.accept()      // conversations waiting in this account's inbox: joined
//   await conversation.list()        // this account's direct conversations
export async function start(ctx) {
  const [space, keys, directory, index, content] = await Promise.all(["space", "keys", "directory", "index", "content"].map(n => ctx.require(n)));
  const short = did => `${did.replace(/^did:craftec:/, "").slice(0, 8)}…`;

  async function direct(did) {
    const me = await space.account();
    if (!me) throw new Error("nobody is logged in");
    if (did === me.id) throw new Error("that is you");
    const card = await directory.card(did);
    if (!card?.inbox || !card.keyPackages.length) throw new Error("that person has not published a card yet (Account → Your card)");
    // This person's own card: their reply comes back to its inbox.
    const mine = await directory.publish();
    const sp = await space.create("direct", card.handle ?? short(did), { with: did });
    const g = keys.group(sp);
    const welcomes = [];
    for (const k of card.keyPackages) welcomes.push({ node: k.node, welcome: await g.add(k.keyPackage) });
    await index.send(did, { kind: "welcome", space: sp.id, spaceKind: "direct", from: me.id, name: mine.handle ?? short(me.id), welcomes });
    await (await content.in(sp)).post("system", "started the conversation");
    ctx.log("conversation", { what: `a direct conversation with ${short(did)}: ${welcomes.length} node(s) welcomed` });
    return sp;
  }

  // WELCOMES in this account's inbox: every conversation not yet joined here, joined (with this node's key package).
  async function accept() {
    const me = await space.account();
    if (!me) return [];
    const listed = new Set((await space.mine()).map(s => s.id));
    const out = [];
    for (const it of await index.inbox()) {
      if (it.kind !== "welcome" || listed.has(it.space)) continue;
      const w = it.welcomes?.find(x => x.node === me.self);
      if (!w) continue;
      const v = { kind: it.spaceKind, name: it.name, owner: it.from, with: it.from };
      try {
        const sp = await space.describe(it.space, v);
        await keys.group(sp).join(w.welcome);
        await space.record(it.space, v);
        await (await content.in(sp)).post("system", "joined");
        out.push(sp);
        ctx.log("conversation", { what: `joined a ${it.spaceKind} conversation with ${short(it.from)}` });
      } catch (e) {
        ctx.log("conversation", { what: `a welcome from ${short(it.from)} did not open here: ${e.message}` });
      }
    }
    // Its key package is used up: a new one on the card.
    if (out.length) await directory.renew().catch(e => ctx.log("conversation", { what: `renewing the key package: ${e.message}` }));
    return out;
  }

  const list = async () => (await space.mine()).filter(s => s.kind === "direct");

  return { direct, accept, list };
}
