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
//   await conversation.list()        // this account's conversations: direct ones and groups
//   await conversation.group([did…], name)   // a group conversation (you and two or more others), each welcomed
//   await conversation.members(sp)   // the accounts (DIDs) whose nodes are in a space's group
//   await conversation.person(text)  // a DID from `did:craftec:…`, or from `name#abc123` among the people this account knows
//   await conversation.createInvite(sp, { days, uses })  // an INVITE CODE for a space (0: no limit): "xxxx-xxxx-xxxx-xxxx"
//   await conversation.revokeInvite(sp, code)
//   await conversation.join(code)    // ask to join by a code: any member who may invite admits the asker when next online
//   await conversation.admit(sp)     // the requests under this space's codes in force: each asker welcomed (and recorded)
//   await conversation.befriend(did) // a FRIEND request (their inbox); friends once both have asked, or they accept
//   await conversation.friendRequests()   // who asked this person, not yet answered: [did]
//   await conversation.answerFriend(did, yes)
//   await conversation.unfriend(did)      // no longer friends — on both sides (a notice in their inbox)
//   (A BLOCKED person's welcomes, mail and requests are left unopened: `edge.people`.)
//   const chs = await conversation.channels(server)   // a server's channels: chs.list() add(name) rename(c, name)
//     remove(c) onChange(fn) settled — those made by someone who may make channels, less the deleted (hidden)
//   await conversation.mail.send([did…], subject, body, re)   // a mail (re: the id of the one it answers)
//   await conversation.mail.fetch()  // mails pointed to in the inbox, opened and kept
//   await conversation.mail.list("in" | "sent")   // [{ id, from, to, subject, body, at, re }], newest first
//   await conversation.mail.prune()  // this account's sealed copies older than 30 days dropped (after each send)
export async function start(ctx) {
  const [space, keys, directory, index, content] = await Promise.all(["space", "keys", "directory", "index", "content"].map(n => ctx.require(n)));
  const short = did => `${did.replace(/^did:craftec:/, "").slice(0, 8)}…`;

  // WELCOME a person into a space: they (their DID) added to its group by a key package from their card, and the welcome
  // — what the space is — sealed into their inbox. `name`: what the space is called for them.
  // `code`: the request it answers (an invite code, or "open"), so the asker knows which of theirs is answered.
  async function welcome(sp, did, name, code = null, repair = null) {
    const me = await space.account();
    if (!me) throw new Error("nobody is logged in");
    if (did === me.id) throw new Error("that is you");
    const card = await directory.card(did);
    if (!card?.inbox || !card.keyPackage) throw new Error("that person has no card yet");
    // A key package works ONCE: never one this account used already (a card read before its person renewed it still
    // lists the one used last time) — kept in the account's table `keypacks`.
    const used = await (await ctx.require("storage")).table("keypacks");
    await used.settled;
    const tagOf = async kp => [...new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(kp)))].slice(0, 16).map(x => x.toString(16).padStart(2, "0")).join("");
    const tags = await Promise.all(card.keyPackages.map(tagOf));
    const spent = new Set(used.rows().filter(r => r.value).map(r => r.key));
    const unused = card.keyPackages.filter((_, i) => !spent.has(tags[i]));
    if (!unused.length) throw new Error("their card has no key package left unused (it renews when they are next online): try again then");
    const kp = unused[Math.floor(Math.random() * unused.length)];
    await used.put(tags[card.keyPackages.indexOf(kp)], String(Date.now()));
    // WELCOMED AGAIN (a welcome that never opened — its key package lost — or a member who lost the space): whatever
    // of theirs the group still holds is taken out first; the group admits one entry per key.
    const g = keys.group(sp);
    const { glue } = await ctx.require("node");
    const didOfCred = h => glue.did_of(new Uint8Array(h.match(/../g).slice(4, 36).map(x => parseInt(x, 16))));
    const stale = ((await g.ready().catch(() => null))?.members ?? []).filter(m => m.cred && didOfCred(m.cred) === did).map(m => m.index);
    if (stale.length) {
      await g.remove(stale);
      ctx.log("conversation", { what: `${directory.shown(did, card.handle)}: an earlier entry in the group taken out before welcoming again` });
    }
    const welcome = await g.add(kp);
    const { owner, nonce } = sp.governance;
    await index.send(did, { kind: "welcome", space: sp.id, spaceKind: sp.kind, from: me.id, owner, nonce, name, welcome, ...(code ? { code } : {}), ...(sp.circle ? { circle: sp.circle } : {}), ...(sp.group ? { group: sp.group } : {}), ...(repair ? { repair } : {}) });
    ctx.log("conversation", { what: `${directory.shown(did, card.handle)} welcomed into a ${sp.kind}` });
    return card;
  }

  // DIRECT: a two-person space, the other welcomed — or the one this person already has with them.
  async function direct(did) {
    const had = (await space.mine()).find(s => s.kind === "direct" && s.with === did);
    if (had) return had;
    const me = await space.account();
    const card = await directory.card(did);
    if (!card) throw new Error("that person has no card yet");
    const sp = await space.create("direct", card.handle ?? short(did), { with: did });
    await welcome(sp, did, (await directory.card(me.id))?.handle ?? short(me.id));
    return sp;
  }

  // INVITE a person into a space this node is in (a server): their nodes join its group from the welcome.
  // (Recorded as `added`: someone removed before is back by this.)
  async function invite(sp, did) {
    const r = await (await ctx.require("roles")).of(sp);
    if (r.banned(did)) throw new Error("they are banned from this server");
    const card = await welcome(sp, did, sp.name);
    await r.act({ act: "added", did }).catch(e => ctx.log("conversation", { what: `recording the invite: ${e.message}` }));
    return card;
  }

  const people = () => ctx.require("edge").then(e => e.people());

  // FRIENDS: a request in their inbox; both asked (or one accepts) → friends on both sides.
  async function befriend(did) {
    const me = await space.account();
    if (!me) throw new Error("nobody is logged in");
    if (did === me.id) throw new Error("that is you");
    const p = await people();
    await p.set("asked", did, true);
    await index.send(did, { kind: "friend", from: me.id, at: Date.now() });
  }
  async function friendRequests() {
    const [p, items] = await Promise.all([people(), index.inbox({ show: true })]);
    await p.settled;
    const asking = new Set();
    // In the order they were made (an inbox is a set: what arrived first is not what was said first).
    for (const it of [...items].sort((a, b) => (Number(a.at) || 0) - (Number(b.at) || 0))) {
      // Their unfriending, newer than the friendship here: no longer friends on this side either.
      if (it.kind === "unfriend" && it.from && p.is("friend", it.from) && Number(it.at) > p.at("friend", it.from)) {
        await p.set("friend", it.from, false);
        await p.set("answered", it.from, true, Number(it.at));
        asking.delete(it.from);
        continue;
      }
      if (!it.from || p.is("block", it.from) || p.is("friend", it.from)) continue;
      // Their yes to this person's request — made after it (an older yes answered an older request) — or their own
      // request, not answered yet, while this person has asked too: friends.
      const fresh = it.kind === "friend-yes" ? Number(it.at) > p.at("asked", it.from) : it.kind === "friend" && Number(it.at) > p.at("answered", it.from);
      if (fresh && p.is("asked", it.from)) {
        await p.set("friend", it.from, true);
        await p.set("asked", it.from, false);
        if (it.kind === "friend") await index.send(it.from, { kind: "friend-yes", from: (await space.account()).id, at: Date.now() }).catch(() => {});
        continue;
      }
      // A request not answered since it was made (yes, no, or a friendship ended after it).
      if (it.kind === "friend" && Number(it.at) > p.at("answered", it.from)) asking.add(it.from);
    }
    return [...asking];
  }
  async function unfriend(did) {
    const p = await people();
    await p.set("friend", did, false);
    await p.set("answered", did, true);
    await index.send(did, { kind: "unfriend", from: (await space.account()).id, at: Date.now() }).catch(() => {});
  }
  async function answerFriend(did, yes) {
    const p = await people();
    await p.set("answered", did, true);
    if (!yes) return;
    await p.set("friend", did, true);
    await index.send(did, { kind: "friend-yes", from: (await space.account()).id, at: Date.now() });
  }

  const welcomesTried = new Set();
  // WELCOMES in this account's inbox: every conversation not yet joined here, joined (with this node's key package).
  // A SPACE's OWNER's branch, as its devices announced it in the space's writers list (`keys`: `{ w, epoch, branch }`):
  // the newest — the branch every member's diff is against.
  async function ownerBranches(sp) {
    const r = await (await ctx.require("roles")).of(sp);
    await r.settled;
    const devs = await directory.devices(r.owner);
    return (await index.spacePointers(sp, "writers")).filter(p => p?.branch && devs.includes(p.w)).sort((a, b) => b.epoch - a.epoch);
  }
  const ownerBranch = async sp => (await ownerBranches(sp))[0] ?? null;
  // REPAIR (a fork's heal, for forks from before snapshots): a member whose announced branch is not this node's —
  // where this node is on the OWNER's — welcomed again onto it (`welcome`: their old entry out, a fresh one in; their
  // node keeps its branch's keys, nothing they wrote is lost). Once per divergence, by whichever member gets there
  // first: the repair is recorded in the writers list (`{ repair, for }`), and a member repaired after their last
  // announcement is waited for. A node off the owner's branch repairs nobody: it is repaired.
  async function repair(sp) {
    const me = await space.account();
    const g = keys.group(sp);
    const st = await g.ready().catch(() => null);
    if (!me || !st || st.removed) return [];
    const own = await ownerBranch(sp).catch(() => null);
    if (!own || st.epoch < own.epoch || (await g.fingerprint(own.epoch)) !== own.branch) return [];
    const ptrs = await index.spacePointers(sp, "writers");
    const done = new Set(ptrs.filter(p => p?.repair).map(p => `${p.repair}|${p.for}`));
    const r = await (await ctx.require("roles")).of(sp);
    const out = [];
    for (const { did } of r.members()) {
      if (did === me.id) continue;
      const devs = await directory.devices(did);
      const ann = ptrs.filter(p => p?.branch && devs.includes(p.w)).sort((a, b) => b.epoch - a.epoch)[0];
      if (!ann || ann.epoch > st.epoch || done.has(`${did}|${ann.branch}`)) continue;
      const fp = await g.fingerprint(ann.epoch).catch(() => null);
      if (!fp || fp === ann.branch) continue;
      try {
        await welcome(sp, did, sp.name, null, { epoch: own.epoch, branch: own.branch });
        await index.spacePoint(sp, "writers", { repair: did, for: ann.branch });
        ctx.log("conversation", { what: `${sp.name}: ${short(did)} was on another branch of its keys — welcomed back onto the owner's` });
        out.push(did);
      } catch (e) {
        ctx.log("conversation", { what: `${sp.name}: repairing ${short(did)}: ${e.message}` });
      }
    }
    return out;
  }
  async function accept() {
    const me = await space.account();
    if (!me) return [];
    const mine = await space.mine();
    const p = await people();
    const out = [];
    for (const it of await index.inbox()) {
      if (it.kind !== "welcome" || p.is("block", it.from) || !it.welcome) continue;
      // Joined already — unless REMOVED since (invited back), or this node is on ANOTHER BRANCH of the space's group and
      // this is its REPAIR: taken only onto the branch the OWNER announced (read here, never the message's word).
      const had = mine.find(s => s.id === it.space);
      let expect = null;
      if (had && !(await keys.group(had).ready().catch(() => null))?.removed) {
        if (!it.repair) continue;
        // The branch it names must be one the owner announced (at any epoch: the repair itself moves the owner on).
        const own = (await ownerBranches(had).catch(() => [])).find(a => a.epoch === it.repair.epoch && a.branch === it.repair.branch);
        if (!own) continue;
        if ((await keys.group(had).fingerprint(own.epoch).catch(() => null)) === own.branch) continue; // on it already
        expect = { epoch: own.epoch, branch: own.branch };
      }
      const tried = `${it.space}|${it.welcome.slice(0, 64)}`;
      if (had && welcomesTried.has(tried)) continue;
      welcomesTried.add(tried);
      const v = { kind: it.spaceKind, name: it.name, owner: it.owner ?? it.from, nonce: it.nonce ?? null, ...(it.spaceKind === "direct" ? { with: it.from } : {}), ...(it.circle ? { circle: it.circle } : {}), ...(typeof it.group === "string" ? { group: it.group.slice(0, 200) } : {}) };
      try {
        const sp = await space.describe(it.space, v);
        await keys.group(sp).join(it.welcome, { expect });
        await space.record(it.space, v);
        // Its request answered: no longer waiting — the space's, and the code the welcome names.
        const t = await asks().catch(() => null);
        const answered = x => x.key === it.space || (it.code && x.key === `code:${it.code}`);
        if (t) for (const r of t.rows().filter(x => x.value && answered(x))) await t.remove(r.key).catch(() => {});
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

  // This person's conversations (Messages): direct ones and groups.
  const list = async () => (await space.mine()).filter(s => s.kind === "direct" || s.kind === "group");

  // A GROUP conversation: a space of several people (no channels, no roles to set up), each welcomed.
  async function group(dids, name) {
    const me = await space.account();
    const others = [...new Set(dids)].filter(d => d !== me.id);
    if (others.length < 2) throw new Error("a group is you and at least two others");
    const names = await Promise.all(others.map(d => directory.name(d)));
    const sp = await space.create("group", name?.trim() || names.join(", "));
    for (const d of others) await welcome(sp, d, sp.name);

    return sp;
  }

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

  // INVITE CODES. A code is an act in the space's log (who made it, when it expires, how many it admits: `roles`). Who
  // holds it drops a request in the bag the code names (`index`); a member who may invite, when online, reads the bags
  // of the codes in force and welcomes each asker not yet in — recorded as `admitted` (the uses counted from those).
  const hex4 = () => [...crypto.getRandomValues(new Uint8Array(2))].map(x => x.toString(16).padStart(2, "0")).join("");
  const codeOf = text => String(text ?? "").trim().toLowerCase().replace(/[^0-9a-f]/g, "").replace(/(.{4})(?=.)/g, "$1-");
  async function createInvite(sp, { days = 7, uses = 0 } = {}) {
    const r = await (await ctx.require("roles")).of(sp);
    const code = [hex4(), hex4(), hex4(), hex4()].join("-");
    await index.openRequests(code);
    await r.act({ act: "invite", code, expires: days ? Date.now() + days * 86400000 : 0, uses });
    return code;
  }
  const revokeInvite = async (sp, code) => (await (await ctx.require("roles")).of(sp)).act({ act: "revoke-invite", code });
  async function join(text) {
    const me = await space.account();
    if (!me) throw new Error("nobody is logged in");
    const code = codeOf(text);
    if (code.length !== 19) throw new Error("an invite code is 16 letters and digits: xxxx-xxxx-xxxx-xxxx");
    await directory.publish().catch(() => {}); // the asker's card carries key packages to be added by
    await index.request(code, { kind: "join", did: me.id, at: Date.now() });
    await noteAsk(`code:${code}`, { code });
    return code;
  }
  // OPEN SPACES: who may join without a code (the space's `config` space/join: "open"). The asker drops a request in
  // the bag the space's id names; a member who may invite welcomes them, recorded `admitted` (code "open").
  const openCode = id => `open ${id}`;
  async function joinOpen(desc) {
    const me = await space.account();
    if (!me) throw new Error("nobody is logged in");
    await directory.publish().catch(() => {});
    await index.request(openCode(desc.id), { kind: "join", did: me.id, at: Date.now() });
    await noteAsk(desc.id, { name: desc.name ?? null });
  }
  // REQUESTS this account made and is still waiting on (its table `asks`: every device shows them): by space id, or
  // `code:<code>` (the space is not known until the welcome). Gone once in.
  const asks = async () => (await ctx.require("storage")).table("asks");
  async function noteAsk(key, v) {
    await (await asks()).put(key, JSON.stringify({ ...v, at: Date.now() })).catch(e => ctx.log("conversation", { what: `keeping the request: ${e.message}` }));
  }
  const parseAsk = r => {
    try {
      return { key: r.key, ...JSON.parse(r.value) };
    } catch {
      return null;
    }
  };
  // The request for this space, while not in it: `{ at, name }`, or null.
  async function asked(id) {
    if ((await space.mine()).some(s => s.id === id)) return null;
    const t = await asks();
    await t.settled;
    const r = t.rows().find(x => x.key === id && x.value);
    return r ? parseAsk(r) : null;
  }
  // Requests by CODE still waiting (none joined since they were made): `[{ code, at }]`.
  async function askedCodes() {
    const t = await asks();
    await t.settled;
    return t.rows().filter(x => x.key.startsWith("code:") && x.value).map(parseAsk).filter(Boolean);
  }
  async function setJoin(sp, how) {
    const r = await (await ctx.require("roles")).of(sp);
    if (how === "open") await index.openRequests(openCode(sp.id));
    await r.act({ act: "policy", path: "", action: "join", who: how === "open" ? "anyone" : "members" });
    // Open: listed in Discover, its acts published (who is in, how to join).
    if (how === "open") (await r.publish().catch(() => {}), await index.listSpace(sp));
  }
  async function admit(sp) {
    const me = await space.account();
    const r = await (await ctx.require("roles")).of(sp);
    await r.refresh();
    if (!me || !r.can(me.id, "invite")) return [];
    const inside = new Set(r.members().map(m => m.did));
    const out = [];
    if (r.policy("", "join") === "anyone")
      for (const q of await index.requests(openCode(sp.id))) {
        if (q.kind !== "join" || !q.did || inside.has(q.did) || r.banned(q.did)) continue;
        try {
          await welcome(sp, q.did, sp.name, "open");
          await r.act({ act: "admitted", code: "open", did: q.did });
          inside.add(q.did);
          out.push(q.did);
          ctx.log("conversation", { what: `${directory.shown(q.did)} joined ${sp.name} (open)` });
        } catch (e) {
          ctx.log("conversation", { what: `could not admit ${short(q.did)}: ${e.message}` });
        }
      }
    for (const inv of r.invites()) {
      for (const q of await index.requests(inv.code)) {
        if (q.kind !== "join" || !q.did || inside.has(q.did) || r.banned(q.did) || !r.invites().some(i => i.code === inv.code)) continue;
        try {
          await welcome(sp, q.did, sp.name, inv.code);
          await r.act({ act: "admitted", code: inv.code, did: q.did });
          inside.add(q.did);
          out.push(q.did);
          ctx.log("conversation", { what: `${directory.shown(q.did)} admitted to ${sp.name} by a code` });
        } catch (e) {
          ctx.log("conversation", { what: `could not admit ${short(q.did)}: ${e.message}` });
        }
      }
    }
    return out;
  }

  // A SERVER's CHANNELS: ITEMS of kind `channel` in the space (`items`: title its name, `meta.cid` its messages'
  // table — `space.channel`), counted where someone who may make channels made them; each with its own rule for who
  // may post (`meta.write.post`, else the space's Chat policy). Every page that shows or watches channels asks here.
  const channelSets = new Map();
  const channelName = name => {
    name = String(name ?? "").trim().toLowerCase().replace(/\s+/g, "-");
    if (!name) throw new Error("name it first");
    return name;
  };
  // A space's AUDIENCES narrower than its members (a role's holders, the admins, the owner): each a GROUP of its own
  // (`groups`, keyed `audience:<space>/<who>`) holding what only they read — any kind, a channel too. Its members as
  // they should be — the space's members who pass its `who` — kept by whoever made it (theirs to add to and remove from).
  const audienceKey = (sp, who) => `audience:${sp.id}/${who}`;
  async function audience(sp, who) {
    const groups = await ctx.require("groups");
    const r = await (await ctx.require("roles")).of(sp);
    const g = await groups.of(audienceKey(sp, who), {
      name: `${space.shown(sp)} · ${who.startsWith("role:") ? (r.roles().find(x => `role:${x.id}` === who)?.name ?? "a role") : who}`,
      // Its apps: the space's (whatever kind is kept for this audience).
      setup: async (_, gr) => {
        for (const app of r.apps()) await gr.act({ act: "app", app, on: true });
      },
    });
    await keepReaders(sp).catch(e => ctx.log("audiences", { what: `${space.shown(sp)}: ${e.message}` }));
    return g;
  }
  async function keepReaders(sp) {
    const me = (await space.account())?.id;
    const [groups, roles] = await Promise.all(["groups", "roles"].map(n => ctx.require(n)));
    const r = await roles.of(sp);
    await r.settled;
    const prefix = `audience:${sp.id}/`;
    for (const g of await space.mine()) {
      if (!g.group?.startsWith(prefix) || g.governance?.owner !== me) continue;
      const who = g.group.slice(prefix.length);
      await groups.keep(g, new Set(r.members().map(x => x.did).filter(d => !r.banned(d) && r.passes(who, d))));
    }
  }
  function channels(server) {
    if (!channelSets.has(server.id))
      channelSets.set(
        server.id,
        (async () => {
          const [roles, items] = await Promise.all(["roles", "items"].map(n => ctx.require(n)));
          // Its acts and its channel items read AT ONCE: who may make channels is checked as the list is drawn.
          const r = await roles.of(server);
          const changed = [];
          const fire = () => changed.forEach(f => f());
          let its = [];
          let reading = null;
          const read = () =>
            (reading ??= items
              .inPlaces({ spaces: [server] }, "channel", { withVotes: false })
              .then(x => ((its = x), fire()), () => {})
              .finally(() => (reading = null)));
          const first = read();
          items.onChange(() => read());
          r.onChange(fire);
          const list = () => {
            const out = new Map();
            for (const it of its) {
              if (!r.can(it.by, "channels")) continue;
              const cid = it.meta?.cid ?? it.id;
              out.set(cid, space.channel(server, cid, it.title || cid, it));
            }
            return [...out.values()].sort((a, b) => a.name.localeCompare(b.name));
          };
          let done = false;
          const settled = Promise.all([first, r.settled]).finally(() => (done = true));
          const me = async () => (await space.account()).id;
          const mayMake = async () => {
            if (!r.can(await me(), "channels")) throw new Error("only who may make channels here changes them");
          };
          // A channel ITEM: public where the space lets anyone read its chat, else its members'.
          const make = async (name, cid, meta = {}) =>
            items.submit({ board: server.id, title: channelName(name), body: "", kind: "channel", meta: { ...meta, cid }, audience: (await items.publicIn(server, "channel").catch(() => false)) ? "public" : "members" });
          return {
            list,
            settled,
            get done() {
              return done;
            },
            onChange: f => changed.push(f),
            add: async name => (await mayMake(), await make(name, newId().slice(0, 8)), read()),
            rename: async (c, name) => (await mayMake(), await items.editItem(c.item.ref, "", { title: channelName(name) }), read()),
            remove: async c => (await items.remove(c.item.ref), read()),
            // WHO MAY POST in one channel: its item's own rule (null: as the space's Chat policy).
            // WHO MAY READ it (`who`: none/"members" — its space's members; a role, "admins", "owner"): narrower
            // than the space, its messages go to a GROUP of its own (`groups`) whose members are its readers — kept
            // in step by its maker (`keepReaders`, every upkeep pass).
            setRead: async (c, who) => {
              await mayMake();
              const it = c.item;
              const narrow = who && who !== "members" && who !== "anyone";
              const meta = { ...(it.meta ?? {}) };
              if (narrow) {
                // Its messages: in its AUDIENCE's group (one per space and audience, whatever kind it keeps).
                const g = await audience(server, who);
                Object.assign(meta, { read: who, group: g.id });
                await items.editItem(it.ref, "", { meta });
              } else {
                delete meta.read;
                await items.editItem(it.ref, "", { meta });
              }
              await read();
            },
            setPost: async (c, who) => {
              await mayMake();
              const it = c.item;
              const write = { ...(it.meta?.write ?? {}) };
              if (who) write.post = who;
              else delete write.post;
              await items.editItem(it.ref, "", { meta: { ...(it.meta ?? {}), write } });
              await read();
            },
          };
        })().catch(e => {
          channelSets.delete(server.id);
          throw e;
        }),
      );
    return channelSets.get(server.id);
  }

  // MAIL.
  const MAIL = "mail";
  const MONTH = 30 * 24 * 3600 * 1000;
  const hexOf = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  const bytesOf = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const newId = () => hexOf(crypto.getRandomValues(new Uint8Array(8)));
  const { glue } = await ctx.require("node");
  // Kept mail is private: the account's table `mailbox` (not `mail`: that name is the public tail's).
  const kept = async () => (await ctx.require("storage")).table("mailbox");
  const mail = {
    // `files`: references (`files`), sealed in the mail with the rest: its recipients read them.
    async send(to, subject, body, re = null, files = []) {
      const me = await space.account();
      if (!me) throw new Error("nobody is logged in");
      to = [...new Set(to)];
      if (!to.length) throw new Error("to nobody");
      const m = { id: `${Date.now().toString(36)}.${newId()}`, from: me.id, to, subject: String(subject ?? ""), body: String(body ?? ""), at: Date.now(), re, ...(files.length ? { files } : {}) };
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
      mail.prune().catch(e => ctx.log("mail", { what: `pruning: ${e.message}` }));
      return m;
    },
    // OLD MAIL in this account's public tail: each recipient keeps what they opened (their `mailbox`), so a sealed
    // copy older than 30 days is dropped (a mail's id starts with when it was sent, base 36; one without is left).
    async prune() {
      const me = await space.account();
      const box = me && (await directory.publicOf(me.id, MAIL));
      if (!box) return 0;
      const old = box.rows().filter(r => {
        const m = /^([0-9a-z]+)\./.exec(r.key);
        return r.value && m && Date.now() - parseInt(m[1], 36) > MONTH;
      });
      for (const r of old) await box.remove(r.key);
      return old.length;
    },
    // Every mail pointed to in the inbox and not kept yet: read from its sender's own tail, opened, kept.
    async fetch() {
      const me = await space.account();
      if (!me) return 0;
      const t = await kept();
      const have = new Set(t.rows().map(r => r.key));
      let n = 0;
      const p = await people();
      for (const it of await index.inbox()) {
        if (it.kind !== "mail" || !it.from || !it.key || have.has(`in/${it.key}`) || p.is("block", it.from)) continue;
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

  return { direct, group, invite, accept, repair, list, members, person, mail, createInvite, revokeInvite, join, joinOpen, asked, askedCodes, setJoin, admit, befriend, friendRequests, answerFriend, unfriend, channels , keepReaders, audience};
}
