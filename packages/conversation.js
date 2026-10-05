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
//   const sm = await conversation.mail.of(space)   // A SPACE's MAIL: its own address (`space:<id>`), its admins' to use
//     sm.may()  await sm.enabled()  await sm.enable()  (its owner)  await sm.send(to, subject, body, re, files)
//     await sm.fetch()  await sm.list("in" | "sent")  sm.onChange(fn)
//   (`to` may name a space: `space:<id>` — of a space this person knows, or one whose mail they had.)
export async function start(ctx) {
  const [space, keys, directory, index, content] = await Promise.all(["space", "keys", "directory", "index", "content"].map(n => ctx.require(n)));
  const short = did => `${did.replace(/^did:craftec:/, "").slice(0, 8)}…`;

  // WELCOME a person into a space: they (their DID) added to its group by a key package from their card, and the welcome
  // — what the space is — sealed into their inbox. `name`: what the space is called for them.
  // `code`: the request it answers (an invite code, or "open"), so the asker knows which of theirs is answered.
  // A welcome's FINGERPRINT (sha-256 of its bytes, 8 hex): the same welcome named alike where it is sent and received.
  const fingerprint = async hex => [...new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(String(hex))))].slice(0, 4).map(x => x.toString(16).padStart(2, "0")).join("");
  async function welcome(sp, did, name, code = null, repair = null) {
    const me = await space.account();
    if (!me) throw new Error("nobody is logged in");
    if (did === me.id) throw new Error("that is you");
    // Their card AS IT IS NOW (read again, never a copy this page read before): key packages work once and are
    // replaced — one from an older read is spent, or lost with what held its secret, and the welcome never opens.
    const card = await directory.card(did, { fresh: true });
    if (!card?.inbox || !card.keyPackage) throw new Error("that person has no card yet");
    // A key package works ONCE: never one this account used already (a card read before its person renewed it still
    // lists the one used last time) — kept in the account's table `keypacks`.
    const used = await (await ctx.require("storage")).table("keypacks");
    await used.settled;
    const tagOf = async kp => [...new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(kp)))].slice(0, 16).map(x => x.toString(16).padStart(2, "0")).join("");
    const tags = await Promise.all(card.keyPackages.map(tagOf));
    const spent = new Set(used.rows().filter(r => r.value).map(r => r.key));
    const unused = card.keyPackages.filter((_, i) => !spent.has(tags[i]));
    if (!unused.length) {
      // Told so, once per card (its packages as they are): their page puts a fresh set on it and asks again.
      const k = `renew-asked:${did}|${tags.join("").slice(0, 32)}`;
      if (!used.rows().some(r => r.key === k && r.value))
        await index.send(did, { kind: "renew-keys", from: me.id, at: Date.now() }).then(() => used.put(k, String(Date.now())), () => {});
      throw new Error("their card has no key package left unused (asked to renew it; it renews when they are next online): try again then");
    }
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
    // MADE FOR THAT PACKAGE, or not sent: a welcome named another key package of theirs (an old one, used already) —
    // it could never open, and it spent a fresh one; said, with both, never sent.
    const made = await keys.welcomeFor(welcome, kp);
    if (!made.ok) {
      ctx.log("conversation", { what: `a welcome for ${directory.shown(did, card.handle)} was made for key package ${made.aimed.join(", ")}, not the one asked (${made.asked}): not sent` });
      throw new Error(`the group made a welcome for another key package of theirs (${made.aimed.join(", ")}), not ${made.asked}`);
    }
    const { owner, nonce } = sp.governance;
    // ITS HISTORY: the space's earlier epochs' secrets held here — the joiner reads everything before it at once, never
    // walking a log per epoch (one log missing cut a late joiner off from all before it).
    const at = (await g.ready().catch(() => null))?.epoch ?? 0;
    const history = sp.idBytes && at > 0 ? await (await ctx.require("auth")).identity.epochSecrets(sp.idBytes, at).catch(() => []) : [];
    // `kp`: the TAG of the key package it is made for (what the person's page names when it does not open).
    await index.send(did, { kind: "welcome", space: sp.id, spaceKind: sp.kind, from: me.id, owner, nonce, name, welcome, kp: tags[card.keyPackages.indexOf(kp)], made: Date.now(), ...(history.length ? { history } : {}), ...(code ? { code } : {}), ...(sp.circle ? { circle: sp.circle } : {}), ...(sp.group ? { group: sp.group } : {}), ...(repair ? { repair } : {}) });
    ctx.log("conversation", { what: `${directory.shown(did, card.handle)} welcomed into a ${sp.kind} (welcome ${await fingerprint(welcome)}, aimed at ${made.aimed.join(", ")}, tag ${tags[card.keyPackages.indexOf(kp)].slice(0, 8)})` });
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
    // Its owner as its acts say — or, a member whose keys are behind (the acts unread), as the space's record says.
    const devs = await directory.devices(r.owner ?? sp.governance?.owner);
    return (await index.spacePointers(sp, "writers")).filter(p => p?.branch && devs.includes(p.w)).sort((a, b) => b.epoch - a.epoch);
  }
  const ownerBranch = async sp => (await ownerBranches(sp))[0] ?? null;
  // BEHIND: this node's keys of a space not on the owner's branch at the owner's epoch (a welcome that opened onto an
  // older branch; a restored account's) — it reads nothing new there. On EVERY LOAD it asks the owner, the admins and
  // the other members to welcome it again onto the owner's branch (`welcome-again`, a repair: each ask answered by any
  // member on that branch — nobody new is let in), until it is on it.
  const caughtUpAsked = new Set();
  async function catchUp(sp) {
    if (caughtUpAsked.has(sp.id)) return false;
    const me = await space.account();
    const g = keys.group(sp);
    const st = await g.ready().catch(() => null);
    if (!me || !st || st.removed) return false;
    const own = await ownerBranch(sp).catch(() => null);
    if (!own) return false;
    if (st.epoch >= own.epoch && (await g.fingerprint(own.epoch).catch(() => null)) === own.branch) return false;
    caughtUpAsked.add(sp.id);
    const r = await (await ctx.require("roles")).of(sp).catch(() => null);
    // Its people as the public acts list them too (this node may read none of the space's own acts yet).
    const pub = await (await ctx.require("roles")).ofPublic(sp).then(async p => (await p.settled, p.members()), () => []);
    const byRole = [...(r?.members() ?? []), ...pub].sort((a, b) => ["owner", "admin"].includes(b.role) - ["owner", "admin"].includes(a.role));
    const to = [...new Set([r?.owner ?? sp.governance?.owner, ...byRole.map(m => m.did)].filter(d => d && d !== me.id))].slice(0, 12);
    for (const did of to) await index.send(did, { kind: "welcome-again", space: sp.id, from: me.id, repair: true, at: Date.now() }).catch(e => ctx.log("conversation", { what: `${sp.name}: asking ${short(did)} for its keys: ${e.message}` }));
    ctx.log("conversation", { what: `${sp.name}: this node's keys are behind the owner's (epoch ${st.epoch}, theirs ${own.epoch}) — asked ${to.length} to welcome it onto theirs` });
    return true;
  }
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
  // A space's HISTORY handed over (a welcome's, or an answer to a history ask): each earlier epoch's secret kept.
  async function keepHistory(sp, history) {
    if (!Array.isArray(history) || !history.length) return 0;
    const identity = (await ctx.require("auth")).identity;
    let n = 0;
    for (const [e, s] of history) if (typeof e === "number" && typeof s === "string") await identity.epochKeep(e, bytesOf(s), sp.idBytes).then(() => (n += 1), () => {});
    if (n) {
      ctx.log("conversation", { what: `${sp.name ?? "a space"}: ${n} earlier epoch(s) of its history handed over` });
      dispatchEvent(new CustomEvent("craftworks:keys"));
    }
    return n;
  }
  // A HISTORY GAP here (`keys`: its walk stopped): asked of the space's owner and admins, once a page.
  const askedHistory = new Set();
  addEventListener("craftworks:history-gap", async ({ detail }) => {
    if (askedHistory.has(detail.space)) return;
    askedHistory.add(detail.space);
    const me = await space.account();
    const sp = (await space.mine()).find(s => s.id === detail.space);
    if (!me || !sp) return;
    const r = await (await ctx.require("roles")).of(sp).catch(() => null);
    const to = new Set([sp.governance?.owner, ...((r?.members() ?? []).filter(m => m.role === "admin" || m.role === "owner").map(m => m.did))].filter(d => d && d !== me.id));
    for (const did of to) await index.send(did, { kind: "history-ask", space: sp.id, from: me.id, below: detail.below + 1, at: Date.now() }).catch(() => {});
    ctx.log("conversation", { what: `${sp.name}: its history asked of ${to.size} admin(s)` });
  });
  let renewedForWelcomes = false;
  async function accept() {
    const me = await space.account();
    if (!me) return [];
    const mine = await space.mine();
    const p = await people();
    const out = [];
    // Kept once (`keypacks`, the account's): each welcome asked for again, each such ask answered.
    const once = await (await ctx.require("storage")).table("keypacks");
    await once.settled;
    const done = k => once.rows().some(r => r.key === k && r.value);
    let unanswerable = 0; // welcomes made for a key package this account does not hold
    for (const it of await index.inbox()) {
      // WELCOME AGAIN, asked by someone a welcome from this account never opened for (made from their card's key
      // packages before they renewed them): welcomed again from their card now — into a space this account has, where
      // they belong (the other of a direct conversation; a member of a group or a space this account may invite to).
      if (it.kind === "welcome-again" && it.from && !p.is("block", it.from)) {
        // Each ASK its own (asked again later — the card renewed since — answered again).
        const k = `again:${it.space}|${it.from}|${String(it.kp ?? "").slice(0, 32)}|${it.at ?? ""}`;
        const sp = mine.find(s => s.id === it.space);
        if (!sp || done(k)) continue;
        try {
          let belongs = sp.kind === "direct" ? sp.with === it.from : false;
          if (sp.kind !== "direct") {
            const r = await (await ctx.require("roles")).of(sp);
            await r.settled;
            // A REPAIR lets nobody new in: any member answers it (from the owner's branch, checked below).
            belongs = r.members().some(m => m.did === it.from) && (sp.kind === "group" || it.repair || r.can(me.id, "invite"));
          }
          if (!belongs) continue;
          const name = sp.kind === "direct" ? ((await directory.card(me.id))?.handle ?? short(me.id)) : sp.name;
          // A REPAIR that did not open: made again (onto the owner's branch, from their card as it is now) — whatever
          // the space's record says was repaired already.
          const own = it.repair ? await ownerBranch(sp).catch(() => null) : null;
          if (it.repair && !own) continue;
          // Only from ON the owner's branch: a welcome from another branch would leave them as behind as before.
          if (own && (await keys.group(sp).fingerprint(own.epoch).catch(() => null)) !== own.branch) continue;
          await welcome(sp, it.from, name, null, own ? { epoch: own.epoch, branch: own.branch } : undefined);
          await once.put(k, String(Date.now()));
          ctx.log("conversation", { what: `${short(it.from)} could not open a ${own ? "repair " : ""}welcome into ${sp.name ?? "a conversation"}: welcomed again` });
        } catch (e) {
          ctx.log("conversation", { what: `welcoming ${short(it.from)} again: ${e.message}` });
        }
        continue;
      }
      // RENEW, asked by someone whose welcomes to this account found every key package on its card used: a fresh set,
      // and every request still waiting made again (once per ask).
      if (it.kind === "renew-keys" && it.from && !p.is("block", it.from)) {
        const k = `renewed-for:${it.from}|${it.at}`;
        if (done(k)) continue;
        await directory
          .renew()
          .then(() => askAgain())
          .then(
            async n => (await once.put(k, String(Date.now())), ctx.log("conversation", { what: `${short(it.from)} found this card's key packages all used: a fresh set put on it${n ? `, ${n} request(s) made again` : ""}` })),
            e => ctx.log("conversation", { what: `renewing the card's key packages: ${e?.message ?? e}` }),
          );
        continue;
      }
      // A HISTORY ASK (a member's walk stopped): their space's earlier secrets held here, sealed to them — once each.
      if (it.kind === "history-ask" && it.from && !p.is("block", it.from)) {
        const k = `history:${it.space}|${it.from}|${it.below}`;
        const sp = mine.find(s => s.id === it.space);
        if (!sp || done(k)) continue;
        const r = await (await ctx.require("roles")).of(sp).catch(() => null);
        if (!r?.members().some(m => m.did === it.from)) continue;
        const epochs = await (await ctx.require("auth")).identity.epochSecrets(sp.idBytes, Math.max(0, Number(it.below) || 0)).catch(() => []);
        if (epochs.length) await index.send(it.from, { kind: "history", space: sp.id, from: me.id, epochs, at: Date.now() }).catch(() => {});
        await once.put(k, String(Date.now()));
        ctx.log("conversation", { what: `${sp.name}: ${epochs.length} earlier epoch(s) of its history handed to ${short(it.from)}` });
        continue;
      }
      // A HISTORY handed over (an answer to this page's ask): kept, once each.
      if (it.kind === "history" && it.from && Array.isArray(it.epochs)) {
        const k = `history-kept:${it.space}|${it.from}|${it.at}`;
        const sp = mine.find(s => s.id === it.space);
        if (!sp || done(k)) continue;
        await keepHistory(sp, it.epochs);
        await once.put(k, String(Date.now()));
        continue;
      }
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
        await keepHistory(sp, it.history);
        await space.record(it.space, v);
        // Its request answered: no longer waiting — the space's, and the code the welcome names.
        const t = await asks().catch(() => null);
        const answered = x => x.key === it.space || (it.code && x.key === `code:${it.code}`);
        if (t) for (const r of t.rows().filter(x => x.value && answered(x))) await t.remove(r.key).catch(() => {});
        out.push(sp);
        ctx.log("conversation", { what: `joined a ${it.spaceKind} conversation with ${short(it.from)}` });
      } catch (e) {
        const held = it.kp ? await keys.holdsTag(it.kp).catch(() => null) : null;
        // Made for a key package this account does not hold (its card from before): its sender asked — once — to
        // welcome again from the card as it is now (renewed below). A REPAIR's too (this account on another branch of
        // a space it has): asked again AS a repair — else it stays on that branch for good.
        // Asked again at most once an HOUR while it keeps failing (an answer made from a card that still listed a
        // dead package fails too — never asked only once for good).
        const k = `asked-again:${it.space}|${(it.kp ?? it.welcome).slice(0, 32)}|${Math.floor(Date.now() / 3600e3)}`;
        if (!held && !done(k)) {
          unanswerable += 1;
          await index
            .send(it.from, { kind: "welcome-again", space: it.space, from: me.id, kp: it.kp ?? it.welcome.slice(0, 32), ...(it.repair || had ? { repair: !!it.repair } : {}), at: Date.now() })
            .then(
              () => (once.put(k, String(Date.now())), ctx.log("conversation", { what: `asked ${short(it.from)} to welcome again into ${it.name ?? "a space"} (a welcome made for a key package not held here)` })),
              err => ctx.log("conversation", { what: `asking ${short(it.from)} to welcome again: ${err.message}` }),
            );
        }
        ctx.log("conversation", {
          what: `a welcome from ${short(it.from)} (welcome ${await fingerprint(it.welcome)}) did not open here: ${e.message}${it.kp ? ` — made ${it.made ? new Date(it.made).toISOString().slice(0, 16) : "(when unknown)"} for key package ${it.kp.slice(0, 8)}, ${held ? `one this account offered (batch ${held})` : "NOT one this account offered"}` : " — from before welcomes named their key package"}`,
        });
      }
    }
    // Its key package is used up: a new one on the card.
    if (out.length) await directory.renew().catch(e => ctx.log("conversation", { what: `renewing the key package: ${e.message}` }));
    // WELCOMES THIS ACCOUNT CANNOT OPEN: each spent one of the card's key packages (the welcomer marks it used), none
    // opened — the card's may be all spent, or not this account's. A fresh set put on it (once a page), and every request
    // still waiting made again: the next welcome is made from these.
    else if (unanswerable && !renewedForWelcomes) {
      renewedForWelcomes = true;
      await directory
        .renew()
        .then(() => askAgain())
        .then(
          n => ctx.log("conversation", { what: `${unanswerable} welcome(s) made for key packages not held here: a fresh set put on the card${n ? `, ${n} request(s) made again` : ""}` }),
          e => ((renewedForWelcomes = false), ctx.log("conversation", { what: `renewing the card's key packages: ${e?.message ?? e}` })),
        );
    }
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
  // ASK AGAIN for every request still waiting (the card's key packages renewed: a welcome made from the old ones
  // never opens): the request made anew, so whoever admits sees one newer than the admission and welcomes again.
  async function askAgain() {
    const me = await space.account();
    if (!me) return 0;
    const t = await asks();
    await t.settled;
    const mine = new Set((await space.mine()).map(s => s.id));
    let n = 0;
    for (const r of t.rows().filter(x => x.value && !mine.has(x.key))) {
      const a = parseAsk(r);
      if (!a) continue;
      await index.request(r.key.startsWith("code:") ? r.key.slice(5) : openCode(r.key), { kind: "join", did: me.id, at: Date.now() }).then(() => (n += 1), e => ctx.log("conversation", { what: `asking again: ${e.message}` }));
      await noteAsk(r.key, { ...a, key: undefined });
    }
    return n;
  }
  // LET IN, NEVER IN: an open space whose public acts admit this person while they are still not in it — the welcome
  // never opened here (made from a key package lost since): asked again, once per admission (counted, never timed:
  // the admitter's clock is not this one's).
  const askedThisPage = new Set();
  async function askStuck() {
    const me = await space.account();
    if (!me) return 0;
    const t = await asks();
    await t.settled;
    const mine = new Set((await space.mine()).map(s => s.id));
    const waiting = t.rows().filter(x => x.value && !x.key.startsWith("code:") && !mine.has(x.key)).map(parseAsk).filter(Boolean);
    if (!waiting.length) return 0;
    const listed = await index.spaces().catch(() => []);
    const roles = await ctx.require("roles");
    let n = 0;
    for (const a of waiting) {
      const desc = listed.find(s => s.id === a.key);
      const said = what => ctx.log("conversation", { what: `waiting on ${a.name ?? a.key.slice(0, 8)}: ${what}` });
      if (!desc) {
        said("not listed in Discover (its public acts unread)");
        continue;
      }
      const pr = await roles.ofPublic(desc).catch(() => null);
      await pr?.settled;
      const admissions = (pr?.acts("admitted") ?? []).filter(x => x.did === me.id).length;
      // Asked again once a PAGE while let in and still not in (each load one request: whoever admits answers it once),
      // and whenever it has let this person in again since.
      if (!admissions || (admissions <= (a.seen ?? 0) && askedThisPage.has(a.key))) {
        said(admissions ? `let in ${admissions} time(s), asked again this page already — its welcome not here yet` : "not let in yet");
        continue;
      }
      askedThisPage.add(a.key);
      await index.request(openCode(a.key), { kind: "join", did: me.id, at: Date.now() });
      await noteAsk(a.key, { name: a.name ?? null, seen: admissions });
      ctx.log("conversation", { what: `${a.name ?? a.key.slice(0, 8)}: let in, but its welcome never opened here — asked again` });
      n += 1;
    }
    return n;
  }
  // THE REQUESTS WAITING, each with where it stands: `[{ key, id, name, code, at, status, desc }]` — status `asked`
  // (nobody let them in yet), `let-in` (admitted: the welcome is being opened), `code` (asked by a code: the space not
  // known until its welcome). The spaces panel lists them.
  async function waiting() {
    const t = await asks();
    await t.settled;
    const mine = new Set((await space.mine()).map(s => s.id));
    const me = await space.account();
    const rows = t.rows().filter(x => x.value && !mine.has(x.key)).map(parseAsk).filter(Boolean);
    if (!rows.length) return [];
    const listed = await (await ctx.require("items")).publicSpaces().catch(() => []);
    const roles = await ctx.require("roles");
    return Promise.all(
      rows.map(async a => {
        if (a.key.startsWith("code:")) return { key: a.key, code: a.key.slice(5), name: a.name ?? null, at: a.at, status: "code" };
        const desc = listed.find(s => s.id === a.key) ?? null;
        const pr = desc ? await roles.ofPublic(desc).catch(() => null) : null;
        await pr?.settled;
        const letIn = !!pr?.acts("admitted").some(x => x.did === me?.id);
        return { key: a.key, id: a.key, name: a.name ?? desc?.name ?? null, at: a.at, status: letIn ? "let-in" : "asked", desc };
      }),
    );
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
  // LET IN: one asker welcomed and recorded `admitted` (by the code they asked with, or "open") — the one way in, for
  // the automatic admit below and for a member's own Let in (`requestsOf`).
  async function letIn(sp, did, code) {
    const r = await (await ctx.require("roles")).of(sp);
    await welcome(sp, did, sp.name, code);
    await r.act({ act: "admitted", code, did });
  }
  // WHO ASKED TO JOIN this space, and who was let in: `{ waiting: [{ did, at, code }], admitted: [{ did, at, by, code }] }`
  // — waiting: a request under its open door or a code in force, from someone not in it (nor banned), newer than their
  // last admission (one let in but never in asks again). For its Settings (who may invite sees and lets them in).
  async function requestsOf(sp) {
    const r = await (await ctx.require("roles")).of(sp);
    await r.refresh();
    const inside = new Set(r.members().map(m => m.did));
    const admittedAt = new Map();
    for (const a of r.acts("admitted")) if (a.did && (a.at ?? 0) > (admittedAt.get(a.did) ?? 0)) admittedAt.set(a.did, a.at ?? 0);
    const asks = [];
    if (r.policy("", "join") === "anyone") for (const q of await index.requests(openCode(sp.id)).catch(() => [])) asks.push({ ...q, code: "open" });
    for (const inv of r.invites()) for (const q of await index.requests(inv.code).catch(() => [])) asks.push({ ...q, code: inv.code });
    // OUT BY THE ACTS: a REMOVED member never comes back by a request (Settings says so); one who LEFT only by a request
    // made after they left (an old one still sits in the bag).
    const removedSet = new Set(r.acts().filter(a => a.act === "remove" && a.did).map(a => a.did));
    const leftAt = new Map(r.acts().filter(a => a.act === "leave").map(a => [a.did ?? a.by, a.at ?? 0]));
    const newest = new Map();
    for (const q of asks) {
      if (q.kind !== "join" || !q.did || r.banned(q.did) || removedSet.has(q.did) || (q.at ?? 0) <= (leftAt.get(q.did) ?? -1)) continue;
      if (inside.has(q.did) && !(admittedAt.has(q.did) && (q.at ?? 0) > admittedAt.get(q.did))) continue;
      if (!newest.has(q.did) || (q.at ?? 0) > newest.get(q.did).at) newest.set(q.did, { did: q.did, at: q.at ?? 0, code: q.code });
    }
    return {
      waiting: [...newest.values()].sort((x, y) => y.at - x.at),
      admitted: r.acts("admitted").map(a => ({ did: a.did, at: a.at ?? 0, by: a.by ?? null, code: a.code ?? null })).sort((x, y) => y.at - x.at).slice(0, 20),
    };
  }
  async function admit(sp) {
    const me = await space.account();
    const r = await (await ctx.require("roles")).of(sp);
    await r.refresh();
    if (!me || !r.can(me.id, "invite")) return [];
    const out = [];
    // Each WAITING asker (`requestsOf`): its open door only while the space lets anyone in; a code while in force.
    for (const q of (await requestsOf(sp)).waiting) {
      try {
        await letIn(sp, q.did, q.code);
        out.push(q.did);
        ctx.log("conversation", { what: `${directory.shown(q.did)} ${q.code === "open" ? `joined ${sp.name} (open)` : `admitted to ${sp.name} by a code`}` });
      } catch (e) {
        ctx.log("conversation", { what: `could not admit ${short(q.did)}: ${e.message}` });
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
      name: `${space.shown(sp)} · ${who.startsWith("role:") ? (r.roles().find(x => `role:${x.id}` === who)?.name ?? "a role") : who.startsWith("list:") ? "chosen people" : who}`,
      // Its apps: the space's (whatever kind is kept for this audience).
      setup: async (_, gr) => {
        for (const app of r.apps()) await gr.act({ act: "app", app, on: true });
      },
    });
    await keepReaders(sp).catch(e => ctx.log("audiences", { what: `${space.shown(sp)}: ${e.message}` }));
    return g;
  }
  // AN AUDIENCE KEPT IN STEP — by its maker, and by the space's admins in it too: its maker makes them the group's admins
  // (so they may add and remove), and an admin acts on a change still due TEN MINUTES after their page first saw it
  // (the maker online does it within a tick; two pages committing the same change at once would fork the group).
  const dueSince = new Map(); // `${group}|${did}|in|out` → when first seen due here
  const GRACE = 10 * 60 * 1000;
  async function keepReaders(sp) {
    const me = (await space.account())?.id;
    const [groups, roles] = await Promise.all(["groups", "roles"].map(n => ctx.require(n)));
    const r = await roles.of(sp);
    await r.settled;
    const prefix = `audience:${sp.id}/`;
    const adminsOf = d => ["owner", "admin"].includes(r.role(d));
    for (const g of await space.mine()) {
      if (!g.group?.startsWith(prefix)) continue;
      const who = g.group.slice(prefix.length);
      const want = new Set(r.members().map(x => x.did).filter(d => !r.banned(d) && r.passes(who, d)));
      const gr = await roles.of(g).catch(() => null);
      if (!gr) continue;
      await gr.settled;
      const maker = g.governance?.owner === me;
      if (maker) {
        await groups.keep(g, want);
        // The space's admins in it (as it stands now): the group's admins too — they keep it when its maker is away.
        await gr.refresh?.();
        for (const m of gr.members())
          if (m.did !== me && adminsOf(m.did) && gr.role(m.did) === "member")
            await gr.grant(m.did, "admin").catch(e => ctx.log("audiences", { what: `${g.name ?? "an audience"}: making ${short(m.did)} its admin: ${e.message}` }));
        continue;
      }
      if (gr.role(me) !== "admin" || !gr.can(me, "invite") || !gr.can(me, "remove")) continue;
      // Not its maker: only what is still due after the grace (the maker had their chance).
      const have = new Set(gr.members().map(m => m.did));
      const due = [...want].filter(d => !have.has(d)).map(d => `${g.id}|${d}|in`).concat([...have].filter(d => !want.has(d) && d !== g.governance?.owner && d !== me).map(d => `${g.id}|${d}|out`));
      const now = Date.now();
      for (const k of due) if (!dueSince.has(k)) dueSince.set(k, now);
      for (const k of [...dueSince.keys()]) if (k.startsWith(`${g.id}|`) && !due.includes(k)) dueSince.delete(k);
      if (!due.some(k => now - dueSince.get(k) >= GRACE)) continue;
      ctx.log("audiences", { what: `${space.shown(sp)}: its maker away — keeping ${g.name ?? "an audience"} in step` });
      await groups.keep(g, want);
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
  // A SPACE's MAIL KEY: made once by its OWNER — its secret kept in the space's ADMINS' group (`audience`: only the
  // owner and admins read it), its public halves published in the owner's own public tail `spacemail` (`<space id>` →
  // { box, sign }): trusted as the owner's because the space's id proves its owner (`space.owner`). Mail TO the space is
  // sealed to `box` and pointed to in the space's own inbox; mail FROM it is signed with the key — from the space, not
  // from whichever admin sent it.
  const SPACEMAIL = "spacemail";
  const canon = m => [m.id, m.from, m.via ?? "", JSON.stringify(m.to), m.subject, m.body, String(m.at), m.re ?? "", JSON.stringify(m.files ?? [])].join("\n");
  // A space as mail knows it: its description ({ id, name, governance }) — this person's, a public one, or one a mail named.
  const knownSpaces = new Map();
  async function spaceDesc(id) {
    if (knownSpaces.has(id)) return knownSpaces.get(id);
    const mine = (await space.mine()).find(s => s.id === id);
    const pub = mine ? null : await (await ctx.require("items")).publicSpace(id);
    const d = mine ?? pub;
    if (d) knownSpaces.set(id, d);
    return d ?? null;
  }
  // Its key's public halves, from its owner's record (the id proving the owner); null: no space mail (yet).
  async function spaceKey(desc) {
    const owner = desc && (await space.owner(desc));
    if (!owner) return null;
    const t = await directory.publicOf(owner, SPACEMAIL);
    const v = t?.rows().find(r => r.key === desc.id)?.value;
    try {
      return v ? JSON.parse(v) : null;
    } catch {
      return null;
    }
  }
  const spaceMails = new Map();
  async function spaceMailOf(sp) {
    if (spaceMails.has(sp.id)) return spaceMails.get(sp.id);
    const [roles, storage] = await Promise.all(["roles", "storage"].map(n => ctx.require(n)));
    const r = await roles.of(sp);
    const me = await space.account();
    const may = () => ["owner", "admin"].includes(r.role(me.id));
    // Its admins' group (made by whoever first needs it: the owner, turning space mail on): the key, and kept mail.
    let gp = null;
    const group = () => (gp ??= audience(sp, "admins"));
    const table = async name => {
      const g = await group();
      return storage.table(space.tableOf(g, name), g);
    };
    const seed = async () => {
      const t = await table("mailkey");
      await t.settled;
      const v = t.rows().find(x => x.key === "seed" && x.value)?.value;
      return v ? bytesOf(v) : null;
    };
    const kept = () => table("mailbox");
    const sm = {
      may,
      enabled: async () => !!(await spaceKey(sp)),
      // ON (its owner): the key made, kept for the admins, its public halves published under the owner's name.
      async enable() {
        if (r.role(me.id) !== "owner") throw new Error("only the space's owner turns its mail on");
        let s = await seed();
        if (!s) {
          s = crypto.getRandomValues(new Uint8Array(32));
          await (await table("mailkey")).put("seed", hexOf(s));
        }
        const rec = { box: hexOf(glue.CraftworksCore.box_public(s)), sign: hexOf(glue.CraftworksCore.public_of(s)) };
        await (await directory.publicOf(me.id, SPACEMAIL)).put(sp.id, JSON.stringify(rec));
        return rec;
      },
      // AS THE SPACE (an admin): signed with its key; sealed to each recipient (a person, or another space).
      async send(to, subject, body, re = null, files = []) {
        if (!may()) throw new Error("only its owner and admins write as the space");
        const s = await seed();
        if (!s) throw new Error("this space's mail is not on: its owner turns it on");
        to = [...new Set(to)];
        if (!to.length) throw new Error("to nobody");
        const m = { id: `${Date.now().toString(36)}.${newId()}`, from: `space:${sp.id}`, via: me.id, space: { id: sp.id, name: sp.name, governance: sp.governance }, to, subject: String(subject ?? ""), body: String(body ?? ""), at: Date.now(), re, ...(files.length ? { files } : {}) };
        m.sig = hexOf(glue.CraftworksCore.sign_with(s, new TextEncoder().encode(canon(m))));
        await deliver(m, me);
        await (await kept()).put(`sent/${m.id}`, JSON.stringify(m));
        return m;
      },
      // ITS INBOX: every mail pointed to there, read from its sender's tail, opened with the space's key, kept.
      async fetch() {
        if (!may()) return 0;
        const s = await seed();
        if (!s) return 0;
        const t = await kept();
        const have = new Set(t.rows().map(x => x.key));
        let n = 0;
        for (const it of await index.inboxWith(glue.CraftworksCore.inbox_address(sp.idBytes), s)) {
          if (it.kind !== "mail" || !it.from || !it.key || have.has(`in/${it.key}`)) continue;
          try {
            const sealed = (await directory.publicOf(it.from, MAIL))?.rows().find(x => x.key === it.key)?.value;
            if (!sealed) continue;
            const m = JSON.parse(new TextDecoder().decode(glue.CraftworksCore.open_with(s, bytesOf(sealed))));
            if (!(await genuine(m, it.from)) || !m.to?.includes(`space:${sp.id}`)) continue;
            await t.put(`in/${it.key}`, JSON.stringify(m));
            n += 1;
          } catch (e) {
            ctx.log("mail", { what: `${sp.name}: a mail from ${short(it.from)} did not open: ${e.message}` });
          }
        }
        return n;
      },
      list: async (which = "in") => listOf(await kept(), which),
      onChange: async f => (await kept()).onChange(f),
    };
    spaceMails.set(sp.id, sm);
    return sm;
  }
  // WHO SENT IT: whose tail it was in — and, from a space, its key's signature (whoever of its admins sent it).
  async function genuine(m, tailOwner) {
    if (!String(m.from ?? "").startsWith("space:")) return m.from === tailOwner;
    if (m.via !== tailOwner || m.space?.id !== m.from.slice(6)) return false;
    const key = await spaceKey(m.space);
    if (!key?.sign || !m.sig) return false;
    if (!glue.CraftworksCore.verify_with(bytesOf(key.sign), new TextEncoder().encode(canon(m)), bytesOf(m.sig))) return false;
    knownSpaces.set(m.space.id, m.space);
    return true;
  }
  // DELIVERED: a sealed copy per recipient in the sender's own tail `mail`, a pointer in the recipient's inbox — a
  // person's (their card's key), or a space's (its key, from its owner's record).
  async function deliver(m, me) {
    const box = await directory.publicOf(me.id, MAIL);
    const text = new TextEncoder().encode(JSON.stringify(m));
    const where = await Promise.all(
      m.to.map(async d => {
        if (d.startsWith("space:")) {
          const desc = await spaceDesc(d.slice(6));
          const key = await spaceKey(desc);
          if (!key?.box) throw new Error(`${desc?.name ?? "that space"} has no mail`);
          return { d, pub: bytesOf(key.box), drop: item => index.sendTo(glue.CraftworksCore.inbox_address(bytesOf(d.slice(6))), bytesOf(key.box), item) };
        }
        const card = await directory.card(d);
        if (!card?.inbox) throw new Error(`no inbox yet for ${directory.shown(d)}`);
        return { d, pub: bytesOf(card.inbox), drop: item => index.send(d, item) };
      }),
    );
    for (const w of where) {
      const key = `${m.id}/${w.d.replace(/^(did:craftec:|space:)/, "").slice(0, 16)}`;
      await box.put(key, hexOf(glue.CraftworksCore.seal_to(w.pub, text, crypto.getRandomValues(new Uint8Array(32)))));
      await w.drop({ kind: "mail", from: me.id, key });
    }
  }
  const listOf = (t, which) =>
    t
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

  const mail = {
    of: spaceMailOf,
    // Does a space take mail (its owner turned its mail on: a key in their record)?
    accepts: async id => !!(await spaceKey(await spaceDesc(id)).catch(() => null))?.box,
    // WHOM mail can be written to BY NAME (the "To" picker): the people this person knows (friends, followed) and the
    // spaces whose mail is on — theirs and the public ones listed. [{ ref, label, kind: "person" | "space" }]
    async addresses() {
      const people = await (await ctx.require("edge")).people();
      const dids = [...new Set([...people.list("friend"), ...people.list("follow")])].filter(d => d.startsWith("did:"));
      const persons = dids.map(d => ({ ref: d, label: directory.shown(d), kind: "person" }));
      const listed = await (await ctx.require("items")).publicSpaces().catch(() => []);
      const descs = [...(await space.mine()).filter(s => s.kind === "server" && !space.isGroup(s)), ...listed];
      const seen = new Set();
      const spaces = (
        await Promise.all(
          descs
            .filter(d => !seen.has(d.id) && seen.add(d.id))
            .map(async d => ((await mail.accepts(d.id)) ? { ref: `space:${d.id}`, label: `🏠 ${space.shown(d)}`, kind: "space" } : null)),
        )
      ).filter(Boolean);
      return [...spaces, ...persons];
    },
    // `files`: references (`files`), sealed in the mail with the rest: its recipients read them.
    async send(to, subject, body, re = null, files = []) {
      const me = await space.account();
      if (!me) throw new Error("nobody is logged in");
      to = [...new Set(to)];
      if (!to.length) throw new Error("to nobody");
      const m = { id: `${Date.now().toString(36)}.${newId()}`, from: me.id, to, subject: String(subject ?? ""), body: String(body ?? ""), at: Date.now(), re, ...(files.length ? { files } : {}) };
      await deliver(m, me);
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
          // Who sent it is whose tail it was in (from a space: its key's signature) — never what it says.
          if (!(await genuine(m, it.from)) || !m.to?.includes(me.id)) continue;
          await t.put(`in/${it.key}`, JSON.stringify(m));
          n += 1;
        } catch (e) {
          ctx.log("mail", { what: `a mail from ${short(it.from)} did not open: ${e.message}` });
        }
      }
      return n;
    },
    list: async (which = "in") => listOf(await kept(), which),
    onChange: async f => (await kept()).onChange(f),
  };

  return { direct, group, invite, accept, repair, catchUp, list, members, person, mail, createInvite, revokeInvite, join, joinOpen, asked, askedCodes, askAgain, askStuck, waiting, requestsOf, letIn, setJoin, admit, befriend, friendRequests, answerFriend, unfriend, channels , keepReaders, audience};
}
