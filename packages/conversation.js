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
//   await conversation.invite(sp, did) // that person into a space (a server): their nodes added, the welcome sent
//   await conversation.accept()      // conversations waiting in this account's inbox: joined
//   await conversation.list()        // this account's direct conversations
//   await conversation.members(sp)   // the accounts (DIDs) whose nodes are in a space's group
//   await conversation.person(text)  // a DID from `did:craftec:…`, or from `name#abc123` among the people this account knows
export async function start(ctx) {
  const [space, keys, directory, index, content] = await Promise.all(["space", "keys", "directory", "index", "content"].map(n => ctx.require(n)));
  const short = did => `${did.replace(/^did:craftec:/, "").slice(0, 8)}…`;

  // WELCOME a person into a space: each of their nodes added to its group (from their card's key packages), and the
  // welcome — what the space is, and one per node — sealed into their inbox. `name`: what the space is called for them.
  async function welcome(sp, did, name) {
    const me = await space.account();
    if (!me) throw new Error("nobody is logged in");
    if (did === me.id) throw new Error("that is you");
    const card = await directory.card(did);
    if (!card?.inbox || !card.keyPackages.length) throw new Error("that person has no card yet");
    const g = keys.group(sp);
    const welcomes = [];
    for (const k of card.keyPackages) welcomes.push({ node: k.node, welcome: await g.add(k.keyPackage) });
    const { owner, nonce } = sp.governance;
    await index.send(did, { kind: "welcome", space: sp.id, spaceKind: sp.kind, from: me.id, owner, nonce, name, welcomes });
    ctx.log("conversation", { what: `${directory.shown(did, card.handle)} welcomed into a ${sp.kind}: ${welcomes.length} node(s)` });
    return card;
  }

  // DIRECT: a two-person space, the other welcomed.
  async function direct(did) {
    const me = await space.account();
    const card = await directory.card(did);
    if (!card) throw new Error("that person has no card yet");
    const sp = await space.create("direct", card.handle ?? short(did), { with: did });
    await welcome(sp, did, (await directory.card(me.id))?.handle ?? short(me.id));
    await (await content.in(sp)).post("system", "started the conversation");
    return sp;
  }

  // INVITE a person into a space this node is in (a server): their nodes join its group from the welcome.
  const invite = (sp, did) => welcome(sp, did, sp.name);

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
      const v = { kind: it.spaceKind, name: it.name, owner: it.owner ?? it.from, nonce: it.nonce ?? null, ...(it.spaceKind === "direct" ? { with: it.from } : {}) };
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

  // A space's MEMBERS: its people, as `roles` has them (the accounts its group's nodes' credentials name).
  async function members(sp) {
    const r = await (await ctx.require("roles")).of(sp);
    await r.refresh();
    return r.members().map(m => m.did);
  }

  // A PERSON from what someone typed: a DID as it is, or `name#abc123` (as people are shown) matched among the people
  // this account knows — its conversations and its servers' members. The 6 characters are only the id's start: a
  // stranger is found by their full id (their card is addressed by it).
  async function person(text) {
    const t = String(text ?? "").trim();
    if (t.startsWith("did:craftec:")) return t;
    const m = t.match(/^([^#]*)#([1-9A-HJ-NP-Za-km-z]{6})$/);
    if (!m) throw new Error("give their id (did:craftec:…) or name#abc123");
    const mine = await space.mine();
    const known = new Set(mine.filter(s => s.kind === "direct" && s.with).map(s => s.with));
    for (const sp of mine.filter(s => s.kind === "server")) for (const d of await members(sp)) known.add(d);
    const prefix = [...known].filter(d => d.replace(/^did:craftec:/, "").startsWith(m[2]));
    const named = m[1] ? (await Promise.all(prefix.map(async d => ((await directory.handle(d)) === m[1] ? d : null)))).filter(Boolean) : prefix;
    const found = named.length ? named : prefix;
    if (found.length === 1) return found[0];
    if (!found.length) throw new Error(`${t} is nobody you know yet: give their full id (did:craftec:…)`);
    throw new Error(`${t} matches ${found.length} people: give their full id`);
  }

  return { direct, invite, accept, list, members, person };
}
