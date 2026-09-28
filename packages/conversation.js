// CONVERSATION, a capability: people talking. KINDS only where the mechanism differs:
// - `direct`: two people. Their own space and group; the first contact is a WELCOME dropped in the other's inbox (they
//   may be away: their card's key packages let them be added anyway). Each person's messages in their own feed.
// - `group`: a space's members (a server; its channels are sub-spaces). Joining by invite. (Its page is Chat.)
// - `mail`: items addressed per item. A mail lives in its SENDER's public tail `mail` (only their account writes it: that
//   is what proves who sent it), sealed to each recipient's inbox key; a POINTER to it goes in each recipient's inbox.
//   The recipient opens it and keeps it in their own table `mailbox` (so the sender may drop old ones).
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
//   await conversation.mail.send([did…], subject, body, re)   // a mail (re: the id of the one it answers)
//   await conversation.mail.fetch()  // mails pointed to in the inbox, opened and kept
//   await conversation.mail.list("in" | "sent")   // [{ id, from, to, subject, body, at, re }], newest first
export async function start(ctx) {
  const [space, keys, directory, index, content] = await Promise.all(["space", "keys", "directory", "index", "content"].map(n => ctx.require(n)));
  const short = did => `${did.replace(/^did:craftec:/, "").slice(0, 8)}…`;

  // WELCOME a person into a space: they (their DID) added to its group by a key package from their card, and the welcome
  // — what the space is — sealed into their inbox. `name`: what the space is called for them.
  async function welcome(sp, did, name) {
    const me = await space.account();
    if (!me) throw new Error("nobody is logged in");
    if (did === me.id) throw new Error("that is you");
    const card = await directory.card(did);
    if (!card?.inbox || !card.keyPackage) throw new Error("that person has no card yet");
    const welcome = await keys.group(sp).add(card.keyPackage);
    const { owner, nonce } = sp.governance;
    await index.send(did, { kind: "welcome", space: sp.id, spaceKind: sp.kind, from: me.id, owner, nonce, name, welcome });
    ctx.log("conversation", { what: `${directory.shown(did, card.handle)} welcomed into a ${sp.kind}` });
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
      if (!it.welcome) continue;
      const v = { kind: it.spaceKind, name: it.name, owner: it.owner ?? it.from, nonce: it.nonce ?? null, ...(it.spaceKind === "direct" ? { with: it.from } : {}) };
      try {
        const sp = await space.describe(it.space, v);
        await keys.group(sp).join(it.welcome);
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

  // MAIL.
  const MAIL = "mail";
  const hexOf = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  const bytesOf = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const newId = () => hexOf(crypto.getRandomValues(new Uint8Array(8)));
  const { glue } = await ctx.require("node");
  // Kept mail is private: the account's table `mailbox` (not `mail`: that name is the public tail's).
  const kept = async () => (await ctx.require("storage")).table("mailbox");
  const mail = {
    async send(to, subject, body, re = null) {
      const me = await space.account();
      if (!me) throw new Error("nobody is logged in");
      to = [...new Set(to)];
      if (!to.length) throw new Error("to nobody");
      const m = { id: newId(), from: me.id, to, subject: String(subject ?? ""), body: String(body ?? ""), at: Date.now(), re };
      const box = await directory.publicOf(me.id, MAIL);
      const cards = await Promise.all(to.map(d => directory.card(d)));
      const missing = to.filter((d, i) => !cards[i]?.inbox);
      if (missing.length) throw new Error(`no inbox yet for ${missing.map(d => directory.shown(d)).join(", ")}`);
      const text = new TextEncoder().encode(JSON.stringify(m));
      for (const [i, d] of to.entries()) {
        const key = `${m.id}/${d.replace(/^did:craftec:/, "").slice(0, 16)}`;
        await box.put(key, hexOf(glue.CraftworksCore.seal_to(bytesOf(cards[i].inbox), text, crypto.getRandomValues(new Uint8Array(32)))));
        await index.send(d, { kind: "mail", from: me.id, key });
      }
      await (await kept()).put(`sent/${m.id}`, JSON.stringify(m));
      return m;
    },
    // Every mail pointed to in the inbox and not kept yet: read from its sender's own tail, opened, kept.
    async fetch() {
      const me = await space.account();
      if (!me) return 0;
      const t = await kept();
      const have = new Set(t.rows().map(r => r.key));
      let n = 0;
      for (const it of await index.inbox()) {
        if (it.kind !== "mail" || !it.from || !it.key || have.has(`in/${it.key}`)) continue;
        try {
          const box = await directory.publicOf(it.from, MAIL);
          const sealed = box?.rows().find(r => r.key === it.key)?.value;
          if (!sealed) continue;
          const r = await (await ctx.require("auth")).identity.inboxOpen([bytesOf(sealed)]);
          const m = JSON.parse(new TextDecoder().decode(bytesOf(r.opened?.[0] ?? "")));
          // Who sent it is whose tail it was in — never what it says.
          if (m.from !== it.from || !m.to?.includes(me.id)) continue;
          await t.put(`in/${it.key}`, JSON.stringify(m));
          n += 1;
        } catch (e) {
          ctx.log("mail", { what: `a mail from ${short(it.from)} did not open: ${e.message}` });
        }
      }
      return n;
    },
    async list(which = "in") {
      const t = await kept();
      return t
        .rows()
        .filter(r => r.key.startsWith(`${which}/`))
        .map(r => {
          try {
            return JSON.parse(r.value);
          } catch {
            return null;
          }
        })
        .filter(Boolean)
        .sort((a, b) => b.at - a.at);
    },
    onChange: async f => (await kept()).onChange(f),
  };

  return { direct, invite, accept, list, members, person, mail };
}
