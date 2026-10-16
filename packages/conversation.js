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
// KEY PACKAGES USED BY OTHERS (10-11: welcomes refused "no key package of this account answers that welcome … not held":
// TWO welcomes made for the SAME one-use package — each welcomer's record of what it used is its own account's
// `keypacks`, which no other member, and no other member's node, reads). Each welcome now names its package's TAG in
// the space's writers list (`{ w, welcomed, kp }`, or the `kp` on a node's `repair` there), which every member reads:
// `sharedTags` — those tags, each with the device that wrote it. Pure (no node, no page): what `welcome` and the tests
// both run.
const TAG = /^[0-9a-f]{32}$/;
export function sharedTags(pointers) {
  const out = new Map();
  for (const p of pointers ?? []) if (p && typeof p.kp === "string" && TAG.test(p.kp) && !out.has(p.kp)) out.set(p.kp, String(p.w ?? ""));
  return out;
}
// A kept welcome's wait before its next try (`tries`: made so far): 2 minutes, doubling, at most an hour.
export const resendWait = tries => Math.min(3600e3, 120e3 * 2 ** Math.max(0, (Number(tries) || 1) - 1));

// A level group's code (`level pro`): pure, as an app's frame has it (FOUNDATION §3).
const levelCode = id => `level ${id}`;
export function frame() {
  return { levelCode };
}

export async function start(ctx) {
  const [space, keys, directory, index, content, storage] = await Promise.all(["space", "keys", "directory", "index", "content", "storage"].map(n => ctx.require(n)));
  const short = did => `${did.replace(/^did:craftec:/, "").slice(0, 8)}…`;

  // WELCOME a person into a space: they (their DID) added to its group WITH THE KEY THEIR REQUEST CARRIES (`key`: `{ kp,
  // tag }`, what `admit` — `rules::admission` — chose; owner 10-13: "the right key to the right invite") and the welcome
  // — what the space is — sealed into their inbox. `name`: what the space is called for them. `code`: the request it
  // answers (a code, "open", "level"), so the asker knows which of theirs is answered. `repair`: the owner's branch, for
  // a member off it.
  // A welcome's FINGERPRINT (sha-256 of its bytes, 8 hex): the same welcome named alike where it is sent and received.
  const fingerprint = async hex => [...new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(String(hex))))].slice(0, 4).map(x => x.toString(16).padStart(2, "0")).join("");
  async function welcome(sp, did, name, code, key, repair = null) {
    const me = await space.account.strict();
    if (!me) throw space.loginNeeded();
    if (did === me.id) throw new Error("that is you");
    if (!key?.kp || !key?.tag) throw new Error("no key of their request: a welcome is made only with the key their request carries");
    const card = await directory.card.strict(did);
    if (!card?.inbox) throw new Error("that person has no card yet");
    const used = await (await ctx.require("storage")).table("keypacks");
    await used.settled;
    const { kp, tag } = key;
    await used.put(tag, String(Date.now()));
    const g = keys.group(sp);
    const { glue } = await ctx.require("node");
    const didOfCred = h => glue.did_of(new Uint8Array(h.match(/../g).slice(4, 36).map(x => parseInt(x, 16))));
    // In ONE commit (`readd`): whatever of theirs the group still holds out, the new entry in (a removal then an add
    // cost two epochs, and every epoch leaves whoever is behind further behind).
    const stale = ((await g.ready.strict())?.members ?? []).filter(m => m.cred && didOfCred(m.cred) === did).map(m => m.index);
    const welcome = stale.length ? await g.readd(stale, kp) : await g.add(kp);
    if (stale.length) ctx.log("conversation", { what: `${directory.shown(did, card.handle)}: its earlier entry in the group replaced, in one commit` });
    // MADE FOR THAT PACKAGE, or not sent: a welcome naming another key package could never open.
    const made = await keys.welcomeFor(welcome, kp);
    if (!made.ok) {
      ctx.log("conversation", { what: `a welcome for ${directory.shown(did, card.handle)} was made for key package ${made.aimed.join(", ")}, not the one asked (${made.asked}): not sent` });
      throw new Error(`the group made a welcome for another key package of theirs (${made.aimed.join(", ")}), not ${made.asked}`);
    }
    // THE GROUP'S SHARED LIST of welcomes made (`{ welcomed, kp, at }` in the writers list): what every member's — and
    // every node's — `rules::admission` reads: this request answered, its key spent.
    await index.spacePoint(sp, "writers", { w: sp.self, welcomed: did, kp: tag, at: Date.now() }).catch(e => ctx.failed("conversation", e, `${directory.shown(did, card.handle)}: the welcome (key ${tag.slice(0, 8)}) not listed in the space: ${e?.message ?? e}`));
    // An earlier welcome to them here still kept unsent (below): its entry replaced by this commit, it could never open.
    for (const r of used.rows().filter(x => x.value && x.key.startsWith(`outbox:${sp.id}|${did}|`))) await ctx.background("conversation: used.remove", used.remove(r.key));
    const { owner, nonce } = sp.governance;
    // ITS HISTORY: the space's earlier epochs' secrets held here — the joiner reads everything before it at once.
    const at = (await g.ready())?.epoch ?? 0;
    // THE ONE WELCOME (`identity::welcome_message` through the core — the node's too): `msg` is its input less the
    // history, which is added when it is sent (`welcomeMsg`).
    const msg = { space: sp.id, spaceKind: sp.kind, from: me.id, owner, nonce: nonce ?? null, name, welcome, kp: tag, made: Date.now(), ...(code ? { code } : {}), ...(sp.circle ? { circle: sp.circle } : {}), ...(sp.group ? { group: sp.group } : {}), ...(repair ? { repair } : {}) };
    // SENT, or KEPT TO SEND AGAIN (10-11): the commit above already replaced their entry in the group — kept in
    // `keypacks` (`outbox:<space>|<did>|<tag>`: the message less its history) and sent again by upkeep (`resendWelcomes`).
    const newestKey = `newest:${sp.id}|${did}`;
    try {
      await index.send(did, await welcomeMsg(msg, sp, at));
    } catch (e) {
      const why = String(e?.message ?? e).slice(0, 160);
      const was = used.rows().find(r => r.key === newestKey)?.value ?? null;
      await used.put(`outbox:${sp.id}|${did}|${tag}`, JSON.stringify({ msg, at, was, first: Date.now(), last: Date.now(), tries: 1, why })).catch(k => {
        ctx.log("conversation", { what: `${directory.shown(did, card.handle)}: the welcome not sent (${why}) and not kept to send again (${k?.message ?? k})` });
        return Promise.reject(e);
      });
      ctx.log("conversation", { what: `${directory.shown(did, card.handle)}: the welcome (tag ${tag.slice(0, 8)}) not sent (${why}) — kept, sent again by upkeep` });
      return { card, kept: why };
    }
    // Their NEWEST welcome here (its key package's tag).
    await ctx.background("conversation: used.put", used.put(newestKey, tag));
    ctx.log("conversation", { what: `${directory.shown(did, card.handle)} welcomed into a ${sp.kind} (welcome ${await fingerprint(welcome)}, aimed at ${made.aimed.join(", ")}, tag ${tag.slice(0, 8)})` });
    return { card, kept: null };
  }
  // A welcome's MESSAGE as sent: its input (`msg`) with the space's HISTORY (its epochs below `at` held here), built by
  // the one builder.
  async function welcomeMsg(msg, sp, at) {
    const history = sp.idBytes && at > 0 ? await ctx.optional("conversation: identity", (await ctx.require("auth")).identity.epochSecrets(sp.idBytes, at), () => []) : [];
    return JSON.parse((await ctx.require("node")).glue.welcome_message(JSON.stringify({ ...msg, history })));
  }
  // THE WELCOME OUTBOX, sent again (upkeep, every tick): each kept welcome tried after a wait doubling from 2 minutes
  // to an hour; given up after a day (reported: `welcome-send-failed`, final) — their own page asks again from there
  // (`askStuck`: let in, never in; `catchUp`: a member behind). Dropped unsent when a NEWER welcome to them went out meanwhile (it replaced this one's entry: this
  // one could never open), or the space is no longer this account's.
  const WAIT = { giveUp: 864e5 };
  async function resendWelcomes() {
    const used = await (await ctx.require("storage")).table("keypacks");
    await used.settled;
    const now = Date.now();
    let sent = 0;
    for (const r of used.rows().filter(x => x.value && x.key.startsWith("outbox:"))) {
      const [spId, did, tag] = r.key.slice(7).split("|");
      let o;
      try {
        o = JSON.parse(r.value);
      } catch {
        await ctx.background("conversation: used.remove", used.remove(r.key));
        continue;
      }
      if (now - (o.last ?? 0) < resendWait(o.tries)) continue;
      const list = await space.mine.strict();
      const sp = list.find(s => s.id === spId);
      // "No longer this account's" only from a COMPLETE list (a slow node's partial one dropped kept welcomes, audit 10-13).
      if (!sp && list.complete === false) continue;
      const newest = used.rows().find(x => x.key === `newest:${spId}|${did}`)?.value ?? null;
      const superseded = !!newest && newest !== o.was && newest !== tag;
      if (!sp || superseded || now - (o.first ?? now) > WAIT.giveUp) {
        if (!sp || superseded) ctx.log("conversation", { what: `a kept welcome for ${short(did)} dropped: ${!sp ? "the space is no longer this account's" : "a newer welcome to them went out"}` });
        else {
          (await ctx.require("lifecycle")).transition("member", reqKey(tag), "fail", { by: "page", s: spId.slice(0, 6), to: did.replace(/^did:craftec:/, "").slice(0, 6), why: `its welcome not sent in a day: ${String(o.why ?? "").slice(0, 120)}`, tries: o.tries });
          ctx.log("conversation", { what: `a kept welcome for ${short(did)} (tag ${tag.slice(0, 8)}) given up after ${o.tries} tries over a day: ${o.why}` });
        }
        await ctx.background("conversation: used.remove", used.remove(r.key));
        continue;
      }
      try {
        await index.send(did, await welcomeMsg(o.msg, sp, o.at));
      } catch (e) {
        await ctx.background("conversation: used.put", used.put(r.key, JSON.stringify({ ...o, last: Date.now(), tries: (o.tries ?? 1) + 1, why: String(e?.message ?? e).slice(0, 160) })));
        continue;
      }
      await ctx.background("conversation: used.put", used.put(`newest:${spId}|${did}`, tag));
      await ctx.background("conversation: used.remove", used.remove(r.key));
      sent += 1;
      (await ctx.require("lifecycle")).transition("member", reqKey(tag), "welcomed", { by: "page", s: spId.slice(0, 6), to: did.replace(/^did:craftec:/, "").slice(0, 6), tries: (o.tries ?? 1) + 1 });
      ctx.log("conversation", { what: `a kept welcome for ${short(did)} into ${sp.name ?? sp.kind} (tag ${tag.slice(0, 8)}) sent, try ${(o.tries ?? 1) + 1}` });
    }
    return sent;
  }
  // EVERY SPACE's used key packages (the writers lists' `kp` tags, by any member or node) mirrored into this account's
  // `keypacks` as `shared:<tag>` (→ which device, which space, when) — what `welcome` checks for a space it is not
  // welcoming into, and what the node's mandate is handed as spent with this account's own (`upkeep`). Upkeep, each
  // tick, per space.
  async function mirrorShared(sp) {
    const used = await (await ctx.require("storage")).table("keypacks");
    await used.settled;
    const have = new Set(used.rows().filter(r => r.value).map(r => r.key));
    let n = 0;
    for (const [t, w] of sharedTags(await index.spacePointers.strict(sp, "writers"))) {
      if (have.has(t) || have.has(`shared:${t}`)) continue;
      await used.put(`shared:${t}`, `${w.slice(0, 16)}|${sp.id.slice(0, 8)}|${Date.now()}`);
      n += 1;
    }
    return n;
  }

  // ASKING IN — every request to be let into a group, ONE way (`rules::admission`; owner 10-13): it carries a key
  // package THIS DEVICE makes for it, and its welcome is made with that key and no other. `bag`: a code's, the open
  // door's, a level's (a JOIN) — or the space's MEMBER bag (`memberBag`: AGAIN, from a member by the acts: told it is
  // invited, behind, off the owner's branch, its welcome never opened; `epoch`/`branch`: where its keys are, if any).
  const memberBag = id => `stuck ${id}`;
  // Its STORY is the `join` machine's, keyed by the request (`reqKey`: its key's tag) — the deciders' `member` moves
  // under the same key (lifecycle: every move one health report). The tag returned (kept with the ask: `q`).
  async function askIn(bag, extra = {}, name = null) {
    const me = await space.account.strict();
    if (!me) throw space.loginNeeded();
    const [kp] = await keys.keyPackages(1);
    const tag = (await ctx.require("node")).glue.admission_tag(kp);
    await index.openRequests(bag);
    await index.request(bag, { kind: "join", did: me.id, at: Date.now(), kp, ...extra });
    const s = /^(stuck|open|level) ([0-9a-f]{6})/.exec(bag)?.[2] ?? null;
    (await ctx.require("lifecycle")).transition("join", reqKey(tag), "ask", { s, entry: extra.kind === "stuck" ? "again" : "join", e: extra.epoch ?? null, ...(name ? { name } : {}) });
    return tag;
  }
  // A REQUEST's key in the lifecycle (and so in health): its key package's tag — the same for the asker and every
  // decider, page or node. A request with no key of its own (an old app): by who asked.
  const reqKey = (tag, did = "") => (tag && !/^0+$/.test(tag) ? `q:${String(tag).slice(0, 10)}` : `q:${String(did).replace(/^did:craftec:/, "").slice(0, 6)}-nokey`);
  const askAgainIn = (id, extra = {}) => askIn(memberBag(id), { kind: "stuck", ...extra });
  // A space's member bag MADE (once a page, by its members): it is read every tick here and by the node, and a bag never
  // put costs a whole network search on each read.
  const bagsOpen = new Set();
  const openMemberBag = sp => (bagsOpen.has(sp.id) ? null : index.openRequests(memberBag(sp.id)).then(() => bagsOpen.add(sp.id)));
  // TOLD: who a space's own rule says is in — a server's `added` act, the other of a direct conversation, a group
  // conversation's people (recorded `invited:<space>|<did>` in the account's `keypacks`: those kinds keep no acts) —
  // told in their inbox; their device asks in (`askAgainIn`) and is let in by `admit`, like every request.
  async function tell(sp, did, name) {
    const me = await space.account.strict();
    if (!me) throw space.loginNeeded();
    if (!(await directory.card.strict(did))) throw new Error("that person has no card yet");
    const used = await (await ctx.require("storage")).table("keypacks");
    await used.put(`invited:${sp.id}|${did}`, String(Date.now()));
    await index.send(did, { kind: "invited", space: sp.id, spaceKind: sp.kind, from: me.id, name, at: Date.now() });
    ctx.log("conversation", { what: `${short(did)} told they are in ${sp.name ?? "a conversation"}: their device asks in` });
  }
  // DIRECT: a two-person space, the other told — or the one this person already has with them.
  async function direct(did) {
    const had = (await space.mine.strict()).find(s => s.kind === "direct" && s.with === did);
    if (had) return had;
    const me = await space.account.strict();
    const card = await directory.card.strict(did);
    if (!card) throw new Error("that person has no card yet");
    const sp = await space.create("direct", card.handle ?? short(did), { with: did });
    await tell(sp, did, (await directory.card.strict(me.id))?.handle ?? short(me.id));
    return sp;
  }

  // INVITE a person into a space this node is in (a server): recorded as `added` (someone removed before is back by
  // this), then told — THE ACT FIRST: the acts decide, the group carries out (`admit`, on their own request).
  async function invite(sp, did) {
    const r = await (await ctx.require("roles")).of(sp);
    if (r.banned(did)) throw new Error("they are banned from this server");
    await r.act({ act: "added", did });
    return tell(sp, did, sp.name);
  }

  const people = () => ctx.require("edge").then(e => e.people());

  // FRIENDS: a request in their inbox; both asked (or one accepts) → friends on both sides.
  async function befriend(did) {
    const me = await space.account.strict();
    if (!me) throw space.loginNeeded();
    if (did === me.id) throw new Error("that is you");
    const p = await people();
    await p.set("asked", did, true);
    await index.send(did, { kind: "friend", from: me.id, at: Date.now() });
  }
  async function friendRequests() {
    const [p, items] = await Promise.all([people(), index.inbox.strict({ show: true })]);
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
        if (it.kind === "friend") await ctx.background("conversation: index.send", index.send(it.from, { kind: "friend-yes", from: (await space.me.strict()), at: Date.now() }));
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
    await ctx.background("conversation: index.send", index.send(did, { kind: "unfriend", from: (await space.me.strict()), at: Date.now() }));
  }
  async function answerFriend(did, yes) {
    const p = await people();
    await p.set("answered", did, true);
    if (!yes) return;
    await p.set("friend", did, true);
    await index.send(did, { kind: "friend-yes", from: (await space.me.strict()), at: Date.now() });
  }

  const welcomesTried = new Set();
  // WELCOMES in this account's inbox: every conversation not yet joined here, joined (with this node's key package).
  // A SPACE's OWNER's branch, as its devices announced it in the space's writers list (`keys`: `{ w, epoch, branch }`):
  // the newest — the branch every member's diff is against.
  async function ownerBranches(sp) {
    const r = await (await ctx.require("roles")).of(sp);
    await r.settled;
    // Its owner as its acts say — or, a member whose keys are behind (the acts unread), as the space's record says.
    // Its devices NOW (the one map: `storage.writersOf`).
    const devs = devicesNow(await storage.writersOf(sp), r.owner ?? sp.governance?.owner);
    return (await index.spacePointers.strict(sp, "writers")).filter(p => p?.branch && devs.includes(p.w)).sort((a, b) => b.epoch - a.epoch);
  }
  const ownerBranch = async sp => (await ownerBranches(sp))[0] ?? null;
  // EVERY MEMBER's KEYS, as their devices announced them in the space's writers list (any member reads it): each one
  // on the owner's keys now, BEHIND (an older epoch), on ANOTHER BRANCH, or not announced — and who caught them up.
  // `Map did → { state, epoch, branch, devices, repairedBy }`, and the owner's own `{ epoch, branch }`.
  async function keyStatus(sp) {
    const r = await (await ctx.require("roles")).of(sp);
    await r.settled;
    const ptrs = await index.spacePointers(sp, "writers");
    const own = (await ctx.optional("conversation: ownerBranches", ownerBranches(sp), () => []))[0] ?? null;
    const out = new Map();
    const w = await storage.writersOf(sp);
    for (const m of r.members()) {
      const devs = devicesNow(w, m.did);
      const anns = ptrs.filter(p => p?.branch && devs.includes(p.w)).sort((a, b) => b.epoch - a.epoch);
      const a = anns[0] ?? null;
      const repaired = ptrs.filter(p => p?.repair === m.did || p?.welcomed === m.did);
      const by = repaired.length ? await Promise.all(repaired.map(async p => (await ctx.optional("conversation: directory.name", directory.name(Object.keys(await whoseDevice(p.w, r)).at(0) ?? ""), () => null)) ?? short(p.w ?? ""))) : [];
      // NO DEVICES READ here is not "not announced": what this page could not read, said as such.
      const unreadDevs = !devs.length ? directory.devicesWhy(m.did) : null;
      const state = unreadDevs ? "devices not read" : !a ? "not announced" : !own ? "unknown" : a.epoch === own.epoch && a.branch === own.branch ? "current" : a.epoch < own.epoch ? "behind" : a.epoch === own.epoch ? "another branch" : "ahead";
      // What their page REPORTED it cannot read here (`reportReads`): the newest report of each of their devices.
      const reports = devs.map(w => ptrs.filter(p => p?.reads && p.w === w).sort((x, y) => (y.reads.at ?? 0) - (x.reads.at ?? 0))[0]).filter(Boolean);
      const unread = [...new Set(reports.flatMap(p => (Array.isArray(p.reads.unread) ? p.reads.unread : []).map(String)))].slice(0, 6);
      // Why a welcome did not open on their device (`welcomeFailed`): the newest of each device's.
      const fails = devs.map(w => ptrs.filter(p => p?.welcomeFailed && p.w === w).sort((x, y) => (y.welcomeFailed.at ?? 0) - (x.welcomeFailed.at ?? 0))[0]).filter(Boolean);
      const welcomeFailed = fails.map(p => String(p.welcomeFailed.why)).slice(0, 2);
      out.set(m.did, { state, why: unreadDevs, devs: devs.length, epoch: a?.epoch ?? null, branch: a?.branch ?? null, devices: anns.length, repairedBy: by, unread, welcomeFailed, reported: Math.max(0, ...reports.map(p => p.reads.at ?? 0)) || null });
    }
    return { owner: own ? { epoch: own.epoch, branch: own.branch } : null, writers: ptrs.length, members: out };
  }
  // TELEMETRY (owner 10-11: "more telemetry so you can investigate and monitor our applications health throughout the
  // network"), into the day's public health bag — never content. THIS PAGE's own state in a space (the heartbeat's
  // line): `{ s, e, oe, on, u }` — its keys' epoch, the owner's, whether it is on the owner's branch, what it cannot read.
  async function selfState(sp) {
    const g = keys.group(sp);
    const st = await g.ready();
    const own = await ctx.optional("conversation: ownerBranch", ownerBranch(sp), () => null);
    const on = st && own ? st.epoch >= own.epoch && (await g.fingerprint(own.epoch)) === own.branch : null;
    const u = (await ctx.require("storage")).problemsIn(sp.id).length;
    return { s: sp.id.slice(0, 6), e: st?.epoch ?? null, oe: own?.epoch ?? null, on, u };
  }
  // …and what this page SEES of the other members (hourly a space): each one not current reported (`member-keys`) with
  // what settles why — devices read here or why not, their announcements found, the writers list's size.
  const observedAt = new Map();
  async function observe(sp) {
    if (Date.now() - (observedAt.get(sp.id) ?? 0) < 3600e3) return 0;
    observedAt.set(sp.id, Date.now());
    const ks = await keyStatus(sp);
    const me = (await space.account())?.id;
    let n = 0;
    for (const [did, k] of ks.members) {
      // (This page's own state is its heartbeat's.)
      if (k.state === "current" || did === me) continue;
      n++;
      await index.health({ k: "member-keys", s: sp.id.slice(0, 6), to: did.replace(/^did:craftec:/, "").slice(0, 6), st: k.state, e: k.epoch, oe: ks.owner?.epoch ?? null, d: { devs: k.devs, why: k.why, anns: k.devices, writers: ks.writers, repairedBy: k.repairedBy.length } });
    }
    return n;
  }
  // WHAT THIS PAGE CANNOT READ in a space (`storage.problemsIn`), reported into its writers list — only when it
  // changes (the last report kept in the account's upkeep marks): any member then sees it (`keyStatus`).
  async function reportReads(sp) {
    const storage = await ctx.require("storage");
    const unread = storage.problemsIn(sp.id).slice(0, 5);
    const key = `reads:${sp.id}`;
    const was = await ctx.optional("conversation: storage.upkeepMark", storage.upkeepMark(key), () => null);
    const now = JSON.stringify(unread);
    if (was === now || (was === null && !unread.length)) return false;
    await index.spacePoint(sp, "writers", { w: sp.self, reads: { unread, at: Date.now() } });
    await ctx.background("conversation: storage.setUpkeepMark", storage.setUpkeepMark(key, now));
    return true;
  }
  // The member a device writes for (`{ did: true }`) — the one map (`storage.writersOf`): a departed device's too.
  async function whoseDevice(w, r) {
    const x = (await storage.writersOf(r.space)).byNode.get(w);
    return x ? { [x.did]: true } : {};
  }
  // A member's devices NOW (not departed), from the one map.
  const devicesNow = (w, did) => [...w.byNode].filter(([, x]) => x.did === did && !x.caps).map(([n]) => n);
  // BEHIND: this node's keys of a space not on the owner's branch at the owner's epoch (a welcome that opened onto an
  // older branch; a restored account's) — it reads nothing new there. It ASKS IN AGAIN (`askAgainIn`, the space's
  // member bag, carrying where its keys are): one member lets it in (`admit`) — onto the owner's branch.
  // SAFE (owner 10-06, after an ask from a page only briefly behind took its asker out): asked only once BEHIND FOR
  // 10 minutes in this page (a key change still being processed never asks).
  const BEHIND_FOR = 10 * 60 * 1000;
  // Kept in the page (the app's frame is sandboxed: no localStorage): a page behind for 10 minutes asks; a reload
  // starts the clock again — on the safe side.
  const behind = new Map(); // space id → since when this page has seen it behind
  const behindSince = id => behind.get(id) ?? 0;
  const setBehind = (id, at) => (at ? behind.set(id, at) : behind.delete(id));
  const caughtUpAsked = new Set();
  async function catchUp(sp) {
    if (caughtUpAsked.has(sp.id)) return false;
    const me = await space.account.strict();
    const g = keys.group(sp);
    const st = await g.ready();
    if (!me || !st || st.removed) return false;
    const own = await ctx.optional("conversation: ownerBranch", ownerBranch(sp), () => null);
    if (!own) return false;
    const mine = await g.fingerprint(st.epoch);
    if (st.epoch >= own.epoch && (await g.fingerprint(own.epoch)) === own.branch) return (setBehind(sp.id, 0), false);
    const since = behindSince(sp.id);
    if (!since) return (setBehind(sp.id, Date.now()), false);
    if (Date.now() - since < BEHIND_FOR) return false;
    caughtUpAsked.add(sp.id);
    await askAgainIn(sp.id, { epoch: st.epoch, branch: mine, seen: own.epoch });
    ctx.log("conversation", { what: `${sp.name}: this node's keys are behind the owner's (epoch ${st.epoch}, theirs ${own.epoch}) — asked in again` });
    return true;
  }
  // A space's HISTORY handed over (a welcome's, or an answer to a history ask): each earlier epoch's secret kept.
  async function keepHistory(sp, history) {
    if (!Array.isArray(history) || !history.length) return 0;
    const identity = (await ctx.require("auth")).identity;
    let n = 0;
    for (const [e, s] of history) if (typeof e === "number" && typeof s === "string") await ctx.background("conversation: identity.epochKeep", identity.epochKeep(e, bytesOf(s), sp.idBytes).then(() => (n += 1)));
    if (n) {
      (await ctx.require("lifecycle")).transition("group", sp.id, "history-received", { space: sp.id, epochs: n, from: history.length ? `${history[0][0]}..${history.at(-1)[0]}` : null });
      ctx.log("conversation", { what: `${sp.name ?? "a space"}: ${n} earlier epoch(s) of its history handed over` });
      dispatchEvent(new CustomEvent("craftworks:keys"));
    }
    return n;
  }
  // A HISTORY GAP here (`keys`: its walk stopped; `storage`: a table sealed with an epoch never given): ALL of the
  // space's history up to its epoch now asked — once a page — of its members, the owner and admins first (any member
  // holding it answers: the owner need not be online).
  const askedHistory = new Set();
  addEventListener("craftworks:history-gap", async ({ detail }) => {
    if (askedHistory.has(detail.space)) return;
    askedHistory.add(detail.space);
    const me = await space.account.strict();
    const sp = (await space.mine.strict()).find(s => s.id === detail.space);
    if (!me || !sp) return;
    const r = await ctx.optional("conversation: of", (await ctx.require("roles")).of(sp), () => null);
    const now = (await keys.group(sp).ready())?.epoch ?? 0;
    const below = Math.max(Number(detail.below) + 1 || 0, now);
    const ranked = (r?.members() ?? []).sort((a, b) => ["owner", "admin"].includes(b.role) - ["owner", "admin"].includes(a.role)).map(m => m.did);
    // THIS ACCOUNT FIRST (its other devices hold the space's whole history — 10-13: the owner's recovered device asked
    // only the other members, who joined after the epochs it lacked; its own original device was never asked), then
    // the owner and admins. (This device reads the ask too: it holds none of what it asks, and answers nothing.)
    const to = [...new Set([me.id, sp.governance?.owner, ...ranked].filter(Boolean))].slice(0, 12);
    for (const did of to) await ctx.background("conversation: index.send", index.send(did, { kind: "history-ask", space: sp.id, from: me.id, below, at: Date.now() }));
    ctx.log("conversation", { what: `${sp.name}: its history (epochs below ${below}) asked of ${to.length} member(s)` });
  });
  // The DIDs in this node's group of a space (its MLS members' credentials).
  async function groupDids(sp) {
    const { glue } = await ctx.require("node");
    const st = await keys.group(sp).ready();
    return [...new Set((st?.members ?? []).filter(m => m.cred).map(m => glue.did_of(new Uint8Array(m.cred.match(/../g).slice(4, 36).map(x => parseInt(x, 16))))))];
  }
  // STUCK behind its own commit (`keys`): asked in again, once a page.
  const askedStuck = new Set();
  addEventListener("craftworks:keys-stuck", async ({ detail }) => {
    if (askedStuck.has(detail.space)) return;
    askedStuck.add(detail.space);
    const me = await space.account.strict();
    const sp = (await space.mine.strict()).find(s => s.id === detail.space);
    if (!me || !sp) return;
    await askAgainIn(sp.id, { epoch: detail.epoch, branch: detail.branch, seen: detail.epoch + 1 }).catch(e => ctx.failed("conversation", e, `${sp.name}: asking in again: ${e?.message ?? e}`));
    ctx.log("conversation", { what: `${sp.name}: stuck behind its own change at epoch ${detail.epoch} — asked in again` });
  });
  // ONE AT A TIME (the chat page and upkeep both ask): two passes taking the same welcome at once — one joins, spending
  // its key package; the other finds it not held and asks for another, a remove and an add more.
  // Asked again mid-pass (a welcome may have arrived after its read): one more pass after it.
  let accepting = null;
  let acceptAgain = false;
  const accept = () => {
    if (accepting) return ((acceptAgain = true), accepting);
    return (accepting = (async () => {
      let out = await acceptOnce();
      while (acceptAgain) (acceptAgain = false), (out = [...out, ...(await acceptOnce())]);
      return out;
    })().finally(() => (accepting = null)));
  };
  async function acceptOnce() {
    const me = await space.account.strict();
    if (!me) return [];
    const mine = await space.mine.strict();
    const p = await people();
    const out = [];
    // Kept once (`keypacks`, the account's): each welcome asked for again, each such ask answered.
    const once = await (await ctx.require("storage")).table("keypacks");
    await once.settled;
    const done = k => once.rows().some(r => r.key === k && r.value);
    const inbox = await index.inbox.strict();
    // Each space's NEWEST welcome (its inviter's last word): an older one for the same space is from before (a removal,
    // a welcome again) — its key package spent or never to be used — set aside, never tried or asked about again.
    const newestOf = new Map();
    // Per SENDER (their own clock, compared with itself), never a blocked one's: one sender's welcome never sets
    // another's aside.
    const newestKey = it => `${it.space}|${it.from}`;
    for (const it of inbox) if (it.kind === "welcome" && it.welcome && !it.repair && !p.is("block", it.from)) newestOf.set(newestKey(it), Math.max(newestOf.get(newestKey(it)) ?? 0, Number(it.made) || 0));
    for (const it of inbox) {
      // TOLD it is in (`tell`: a server's `added`, a direct or group conversation started with this person): asked in
      // with this device's own key (`askAgainIn`), once per telling — let in by whoever's turn it is (`admit`).
      if (it.kind === "invited" && it.from && it.space && !p.is("block", it.from)) {
        const k = `invited-asked:${it.space}|${it.from}|${it.at ?? ""}`;
        if (done(k) || mine.some(s => s.id === it.space)) continue;
        await askAgainIn(it.space).then(
          () => (once.put(k, String(Date.now())), ctx.log("conversation", { what: `told by ${short(it.from)} of ${it.name ?? "a conversation"}: asked in` })),
          e => ctx.failed("conversation", e, `asking into ${it.name ?? "a conversation"}: ${e?.message ?? e}`),
        );
        continue;
      }
      // A HISTORY ASK (a member's walk stopped): their space's earlier secrets held here, sealed to them — once each.
      if (it.kind === "history-ask" && it.from && !p.is("block", it.from)) {
        const k = `history:${it.space}|${it.from}|${it.below}`;
        const sp = mine.find(s => s.id === it.space);
        if (!sp || done(k)) continue;
        const r = await ctx.optional("conversation: of", (await ctx.require("roles")).of(sp), () => null);
        if (!r?.members().some(m => m.did === it.from)) continue;
        const epochs = await ctx.optional("conversation: identity", (await ctx.require("auth")).identity.epochSecrets(sp.idBytes, Math.max(0, Number(it.below) || 0)), () => []);
        if (epochs.length) await ctx.background("conversation: index.send", index.send(it.from, { kind: "history", space: sp.id, from: me.id, epochs, at: Date.now() }));
        (await ctx.require("lifecycle")).transition("group", sp.id, "history-handed", { space: sp.id, to: it.from, epochs: epochs.length, below: it.below });
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
      // Joined already: a group here, not removed. LISTED BUT NO GROUP (its keys lost — owner 10-07: a lost key table):
      // not joined, the welcome taken.
      const st = had ? await keys.group(had).ready() : null;
      let above = null;
      if (had && st && !st.removed && !it.repair) {
        // LET IN AGAIN while held here (10-09: #HLt27K behind at 46 — every re-add took its old entry out and sent a
        // plain welcome, which a node holding the group skipped; with its entry gone no later commit reached it, so it
        // stayed behind for good): taken if it leads past this node's epoch (`keys.join`'s `above`), else not.
        above = st.epoch;
      } else if (had && st && !st.removed) {
        // The branch it names must be one the owner announced (at any epoch: the repair itself moves the owner on).
        const own = (await ctx.optional("conversation: ownerBranches", ownerBranches(had), () => [])).find(a => a.epoch === it.repair.epoch && a.branch === it.repair.branch);
        if (!own) continue;
        if ((await keys.group(had).fingerprint(own.epoch)) === own.branch) continue; // on it already
        expect = { epoch: own.epoch, branch: own.branch };
      }
      const tried = `${it.space}|${it.welcome.slice(0, 64)}`;
      // Tried once per page, listed or not (a space left after a removal is not listed: two tries of one welcome).
      if (welcomesTried.has(tried)) continue;
      if (!it.repair && (Number(it.made) || 0) < (newestOf.get(newestKey(it)) ?? 0)) {
        welcomesTried.add(tried);
        if (!done(`dead:${tried}`)) await ctx.background("conversation: once.put", once.put(`dead:${tried}`, `older ${Date.now()}`));
        continue;
      }
      // DEAD: a welcome made for a key package this account never held opens nowhere, ever (its sender was asked once
      // to welcome again: a NEW welcome). Skipped from then on — measured 10-07: 33 of them tried every 30 s, ~2 s each.
      if (done(`dead:${tried}`)) continue;
      welcomesTried.add(tried);
      const v = { kind: it.spaceKind, name: it.name, owner: it.owner ?? it.from, nonce: it.nonce ?? null, ...(it.spaceKind === "direct" ? { with: it.from } : {}), ...(it.circle ? { circle: it.circle } : {}), ...(typeof it.group === "string" ? { group: it.group.slice(0, 200) } : {}) };
      try {
        const sp = await space.describe(it.space, v);
        await keys.group(sp).join(it.welcome, { expect, above });
        // JOINED by it: never tried again (its key package is spent — after a removal it would only fail and ask).
        await ctx.background("conversation: once.put", once.put(`dead:${tried}`, `joined ${Date.now()}`));
        if (it.kp) await ctx.background("conversation: once.put", once.put(`joined-kp:${String(it.kp).slice(0, 32)}`, String(Date.now())));
        await keepHistory(sp, it.history);
        await space.record(it.space, v);
        // Its request answered: no longer waiting — the space's, and the code the welcome names.
        const t = await ctx.optional("conversation: asks", asks(), () => null);
        const answered = x => x.key === it.space || (it.code && x.key === `code:${it.code}`) || (it.code === "level" && x.key === `code:${levelCode(it.space)}`);
        if (t) for (const r of t.rows().filter(x => x.value && answered(x))) await ctx.background("conversation: t.remove", t.remove(r.key));
        out.push(sp);
        if (it.kp) (await ctx.require("lifecycle")).become("join", reqKey(it.kp), "joined", { s: it.space.slice(0, 6), from: String(it.from ?? "").slice(0, 8), e: (await keys.group(sp).ready())?.epoch ?? null });
        ctx.log("conversation", { what: it.repair ? `${it.name ?? it.space.slice(0, 8)}: caught up onto the owner's keys — welcomed by ${short(it.from)}, ${Array.isArray(it.history) ? it.history.length : 0} earlier key(s) with it` : `joined a ${it.spaceKind} conversation with ${short(it.from)}` });
      } catch (e) {
        // NOT PAST this node's epoch (an older re-add): done with for good — never a reason to ask anyone again.
        if (/not past this node's/.test(e?.message ?? "")) {
          await ctx.background("conversation: once.put", once.put(`dead:${tried}`, `not past ${Date.now()}`));
          continue;
        }
        const held = it.kp ? await ctx.optional("conversation: keys.holdsTag", keys.holdsTag(it.kp), () => null) : null;
        // With what settles its cause: the package it was made for and when, who sent it, whether this account offered
        // that package (and from which batch), whether a repair, and this device's packages counted.
        const d = {
          kp: it.kp ? String(it.kp).slice(0, 8) : null,
          made: Number(it.made) || null,
          from: String(it.from ?? "").slice(0, 8),
          offered: held || false,
          repair: !!it.repair,
          joined: !!(had && st && !st.removed),
          pk: await ctx.optional("conversation: keys.packageCounts", keys.packageCounts(), () => null),
        };
        // ITS REQUEST FAILED (the `join` machine: reported), with the reason that decides what may follow (FOUNDATION.md
        // §3) — only a key not held asks anew; keys not held here, never.
        const reason = (await keys.writable()) !== true ? "account-keys-unreadable" : /not past/.test(e?.message ?? "") ? "not-past" : !held ? "package-not-held" : "branch";
        (await ctx.require("lifecycle")).transition("join", reqKey(it.kp ?? it.welcome.slice(0, 32)), "fail", { s: it.space.slice(0, 6), reason, e: st?.epoch ?? null, why: String(e?.message ?? e).slice(0, 160), ...d });
        // SAID in the space (its writers record: the members list shows it), so a member stuck behind is seen WHY.
        if (had) await ctx.background("conversation: index.spacePoint", index.spacePoint(had, "writers", { w: had.self, welcomeFailed: { why: String(e?.message ?? e).slice(0, 200), from: it.from, at: Date.now() } }));
        if (!held && /WelcomeKeyPackageNotFound/.test(e?.message ?? "")) await ctx.background("conversation: once.put", once.put(`dead:${tried}`, String(Date.now())));
        // Not dead (a passing failure): tried again on the next pass.
        else welcomesTried.delete(tried);
        // Made for a key package this account does not hold (a welcome from before requests carried their own): ASKED IN
        // ANEW — a new request with a new key, once per such welcome (`rules::admission` answers each request once).
        // A DUPLICATE (10-11: this account joined by that very package already) asks nothing.
        const k = `asked-again:${tried}`;
        const duplicate = it.kp && done(`joined-kp:${String(it.kp).slice(0, 32)}`);
        if (duplicate) await ctx.background("conversation: once.put", once.put(`dead:${tried}`, `duplicate ${Date.now()}`));
        // (Asking is the account's write: never from a device without its keys.)
        else if (!held && !done(k) && (await keys.writable()) === true) {
          const where = had && st && !st.removed ? { epoch: st.epoch, branch: await keys.group(had).fingerprint(st.epoch), seen: st.epoch + 1 } : {};
          const bag = it.code === "open" ? openCode(it.space) : it.code === "level" ? levelCode(it.space) : it.code;
          await (it.code ? askIn(bag) : askAgainIn(it.space, where)).then(
            () => (once.put(k, String(Date.now())), ctx.log("conversation", { what: `a welcome into ${it.name ?? "a space"} made for a key not held here: asked in anew` })),
            err => ctx.failed("conversation", err, `asking into ${it.name ?? "a space"} anew: ${err?.message ?? err}`),
          );
        }
        ctx.log("conversation", {
          what: `a welcome from ${short(it.from)} (welcome ${await fingerprint(it.welcome)}) did not open here: ${e.message}${it.kp ? ` — made ${it.made ? new Date(it.made).toISOString().slice(0, 16) : "(when unknown)"} for key package ${it.kp.slice(0, 8)}, ${held ? `one this account offered (batch ${held})` : "NOT one this account offered"}` : " — from before welcomes named their key package"}`,
        });
      }
    }
    return out;
  }

  // This person's conversations (Messages): direct ones and groups.
  const list = async () => (await space.mine.strict()).filter(s => s.kind === "direct" || s.kind === "group");

  // A GROUP conversation: a space of several people (no channels, no roles to set up), each told (`tell`).
  async function group(dids, name) {
    const me = await space.account.strict();
    const others = [...new Set(dids)].filter(d => d !== me.id);
    if (others.length < 2) throw new Error("a group is you and at least two others");
    const names = await Promise.all(others.map(d => directory.name(d)));
    const sp = await space.create("group", name?.trim() || names.join(", "));
    for (const d of others) await tell(sp, d, sp.name);

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
    const mine = await space.mine.strict();
    const known = new Set(mine.filter(s => s.kind === "direct" && s.with).map(s => s.with));
    for (const sp of mine.filter(s => s.kind === "server")) for (const d of await members(sp)) known.add(d);
    const prefix = [...known].filter(d => d.replace(/^did:craftec:/, "").startsWith(m[2]));
    const named = m[1] ? (await Promise.all(prefix.map(async d => ((await directory.handle.strict(d)) === m[1] ? d : null)))).filter(Boolean) : prefix;
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
    const me = await space.account.strict();
    if (!me) throw space.loginNeeded();
    const code = codeOf(text);
    if (code.length !== 19) throw new Error("an invite code is 16 letters and digits: xxxx-xxxx-xxxx-xxxx");
    await ctx.background("conversation: directory.publish", directory.publish()); // the asker's card: its inbox, where the welcome comes
    const q = await askIn(code);
    await noteAsk(`code:${code}`, { code, q });
    return code;
  }
  // OPEN SPACES: who may join without a code (the space's `config` space/join: "open"). The asker drops a request in
  // the bag the space's id names; a member who may invite welcomes them, recorded `admitted` (code "open").
  const openCode = id => `open ${id}`;
  // LEVEL GROUPS (rewards §5a: a creator's PRO, VIP): who asks drops a request in the bag `level <id>`; any member lets
  // them in while their public card's contributions reach its level (here, and the identity delegate with no page open).
  const levelOfGroup = r => /^level:(pro|vip)$/.exec(r.policy("", "join") ?? "")?.[1] ?? null;
  // A person's level NOW from their card read again (the one rule: gov's, through `usage`) — kept TEN MINUTES (every
  // member's page asks of every member each tick); null where the card did not read (unknown, never "free": a failed
  // read must not put a giver out).
  const levelsRead = new Map(); // did → { at, id }
  async function levelNow(did) {
    const had = levelsRead.get(did);
    if (had && Date.now() - had.at < 600000) return had.id;
    const card = await directory.card(did, { fresh: true });
    if (!card) return null;
    const id = (await ctx.require("usage")).levelFrom(card.contrib ?? []).id;
    levelsRead.set(did, { at: Date.now(), id });
    return id;
  }
  async function joinLevel(gid, name = null) {
    const me = await space.account.strict();
    if (!me) throw space.loginNeeded();
    await ctx.background("conversation: directory.publish", directory.publish());
    const q = await askIn(levelCode(gid), {}, name);
    await noteAsk(`code:${levelCode(gid)}`, { code: "level", name, q });
    ctx.log("conversation", { what: `asked to join ${name ?? gid.slice(0, 8)} (a level group)` });
  }
  // LAPSED: in a level group, any member (in turn) takes out another whose card no longer reaches its level — a
  // `lapsed` act (not a ban: they ask again when they give again); `moderation.enforce` takes their entry out. Who.
  async function lapse(sp) {
    const me = await space.account.strict();
    const r = await (await ctx.require("roles")).of(sp);
    const lv = levelOfGroup(r);
    if (!me || !lv || r.left || !r.role(me.id)) return [];
    const usage = await ctx.require("usage");
    const ordering = await ctx.require("ordering");
    const members = r.members().map(m => m.did);
    const out = [];
    for (const did of members) {
      if (did === me.id || did === r.owner) continue;
      const have = await levelNow(did);
      if (have === null || usage.reaches(have, lv)) continue;
      const k = `lapse|${sp.id}|${did}`;
      if (!pendingSince.has(k)) pendingSince.set(k, Date.now());
      // Ranked among the OTHER members (the lapsed one never acts on itself: ranked first, it cost every other a turn).
      const turn = ordering.turn(k, members.filter(d => d !== did), me.id);
      if (turn < 0 || Date.now() - pendingSince.get(k) < (turn + 1) * ordering.TAKEOVER) continue;
      await r.act({ act: "lapsed", did }).then(
        () => (pendingSince.delete(k), out.push(did), ctx.log("conversation", { what: `${sp.name}: ${short(did)}'s level ran out — taken out (they ask again when they give again)` })),
        e => ctx.failed("conversation", e, `${sp.name}: lapsing ${short(did)}: ${e.message}`),
      );
    }
    return out;
  }
  async function joinOpen(desc) {
    const me = await space.account.strict();
    if (!me) throw space.loginNeeded();
    await ctx.background("conversation: directory.publish", directory.publish());
    const q = await askIn(openCode(desc.id), {}, desc.name ?? null);
    await noteAsk(desc.id, { name: desc.name ?? null, q });
    ctx.log("conversation", { what: `asked to join ${desc.name ?? desc.id.slice(0, 8)} (its open door)` });
  }
  // WAITING REQUESTS KEPT ANSWERABLE (upkeep, each tick): each request this account still waits on made ANEW — with
  // this device's own key (`askIn`) — once a page when (a) its newest in the bag carries no key (made before requests
  // carried one: `rules::admission` lets nobody in by it), or (b) its open space's public acts let this person in
  // while still not in (its welcome never opened here) — once per admission, counted (never timed: the admitter's
  // clock is not this one's).
  const askedThisPage = new Set();
  const waitSaid = new Map(); // ask → the why last said
  async function askStuck() {
    const me = await space.account.strict();
    if (!me) return 0;
    const may = await (await ctx.require("keys")).writable();
    if (may !== true) return (ctx.log("conversation", { what: `stuck requests not asked again: ${may}` }), 0);
    const t = await asks();
    await t.settled;
    const mine = new Set((await space.mine.strict()).map(s => s.id));
    const waiting = t.rows().filter(x => x.value && !mine.has(x.key)).map(parseAsk).filter(Boolean);
    const listed = await index.spaces();
    // THE REQUEST ITSELF where the account's copy is missing (10-11, Wren: "Requested" from the open door's bag, its
    // `asks` row gone — so never asked again): each listed space this person is not in, its door's bag read; a request
    // of theirs there is put back into `asks` (when it was made) and handled as any other.
    for (const desc of listed) {
      if (mine.has(desc.id) || waiting.some(a => a.key === desc.id)) continue;
      const at = Math.max(0, ...(await index.requests(openCode(desc.id))).filter(q => q?.kind === "join" && q.did === me.id).map(q => Number(q.at) || 0));
      if (!at) continue;
      await ctx.background("conversation: t.put", t.put(desc.id, JSON.stringify({ name: desc.name ?? null, at })));
      waiting.push({ key: desc.id, name: desc.name ?? null, at });
      ctx.log("conversation", { what: `${desc.name ?? desc.id.slice(0, 8)}: a request found in its door's bag, missing from this account's record — kept again` });
    }
    if (!waiting.length) return 0;
    const roles = await ctx.require("roles");
    const life = await ctx.require("lifecycle");
    let n = 0;
    for (const a of waiting) {
      // (a) Its newest request without a key of its own: made anew.
      const bag = a.key.startsWith("code:") ? a.key.slice(5) : openCode(a.key);
      if (!askedThisPage.has(`kp:${a.key}`)) {
        const newest = (await index.requests(bag)).filter(q => q?.kind === "join" && q.did === me.id).sort((x, y) => (Number(y.at) || 0) - (Number(x.at) || 0))[0];
        if (!newest?.kp) {
          askedThisPage.add(`kp:${a.key}`);
          const q = await askIn(bag, {}, a.name ?? null);
          await noteAsk(a.key, { ...a, key: undefined, q });
          ctx.log("conversation", { what: `${a.name ?? a.key.slice(0, 8)}: its request carried no key of its own — made anew with one` });
          n += 1;
          continue;
        }
      }
      if (a.key.startsWith("code:")) continue;
      const desc = listed.find(s => s.id === a.key);
      // WAITING, with why (the `join` machine: reported when the why changes).
      const said = (what, d) => {
        if (waitSaid.get(a.key) === what) return;
        waitSaid.set(a.key, what);
        ctx.log("conversation", { what: `waiting on ${a.name ?? a.key.slice(0, 8)}: ${what}` });
        if (a.q) life.transition("join", reqKey(a.q), "wait", { s: a.key.slice(0, 6), why: what, days: Math.floor((Date.now() - (a.at ?? Date.now())) / 864e5), ...d });
      };
      if (!desc) {
        said("not listed in Discover (its public acts unread)");
        continue;
      }
      const pr = await roles.ofPublic(desc);
      await pr?.settled;
      const admissions = (pr?.acts("admitted") ?? []).filter(x => x.did === me.id).length;
      // Asked again once a PAGE while let in and still not in (each load one request: whoever admits answers it once),
      // and whenever it has let this person in again since.
      if (!admissions || (admissions <= (a.seen ?? 0) && askedThisPage.has(a.key))) {
        said(admissions ? `let in ${admissions} time(s), asked again this page already — its welcome not here yet` : "not let in yet (by its public acts)", { admissions, pubActs: pr?.acts().length ?? null });
        continue;
      }
      askedThisPage.add(a.key);
      const q = await askIn(openCode(a.key), { admissions }, a.name ?? null);
      await noteAsk(a.key, { name: a.name ?? null, seen: admissions, q });
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
    const mine = new Set((await space.mine.strict()).map(s => s.id));
    const me = await space.account.strict();
    const rows = t.rows().filter(x => x.value && !mine.has(x.key)).map(parseAsk).filter(Boolean);
    if (!rows.length) return [];
    const listed = await ctx.optional("conversation: publicSpaces", (await ctx.require("items")).publicSpaces(), () => []);
    const roles = await ctx.require("roles");
    return Promise.all(
      rows.map(async a => {
        if (a.key.startsWith("code:")) return { key: a.key, code: a.key.slice(5), name: a.name ?? null, at: a.at, status: "code" };
        const desc = listed.find(s => s.id === a.key) ?? null;
        const pr = desc ? await roles.ofPublic(desc) : null;
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
    await (await asks()).put(key, JSON.stringify({ ...v, at: Date.now() })).catch(e => ctx.failed("conversation", e, `keeping the request: ${e.message}`));
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
    if ((await space.mine.strict()).some(s => s.id === id)) return null;
    const t = await ctx.optional("conversation: asks", asks(), () => null);
    await t?.settled;
    const r = t?.rows().find(x => x.key === id && x.value);
    if (r) return parseAsk(r);
    // THE REQUEST ITSELF (its open door's bag, on the network): the one fact — the account's record of it is a copy
    // that can be missing (not saved yet: a page closed while "Asking…"; a write that failed).
    const me = await space.account.strict();
    const mine = me ? (await index.requests(openCode(id))).filter(q => q?.kind === "join" && q.did === me.id) : [];
    const at = Math.max(0, ...mine.map(q => q.at ?? 0));
    return at ? { key: id, at } : null;
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
    if (how === "open") (await ctx.background("conversation: r.publish", r.publish()), await index.listSpace(sp));
  }
  // LET IN by a member's own hand (Settings' Let in): that asker's request decided now, no turn waited (`admit`).
  const letIn = (sp, did) => admit(sp, did);
  const pendingSince = new Map(); // what `lapse` saw pending: when this page first saw it
  // WHO ASKED TO JOIN this space, and who was let in: `{ waiting: [{ did, at, code, kp }], admitted: [{ did, at, by, code }] }`
  // — waiting: a request under its open door or a code in force, from someone not in it (nor banned), newer than their
  // last admission (one let in but never in asks again). For its Settings (who may invite sees and lets them in).
  async function requestsOf(sp) {
    const r = await (await ctx.require("roles")).of(sp);
    await r.refresh();
    const inside = new Set(r.members().map(m => m.did));
    const admittedAt = new Map();
    for (const a of r.acts("admitted")) if (a.did && (a.at ?? 0) > (admittedAt.get(a.did) ?? 0)) admittedAt.set(a.did, a.at ?? 0);
    const asks = [];
    if (r.policy("", "join") === "anyone") for (const q of await index.requests(openCode(sp.id))) asks.push({ ...q, code: "open" });
    // A LEVEL GROUP: its bag's askers whose card reaches its level now (checked as each is read).
    const lv = levelOfGroup(r);
    if (lv) {
      const usage = await ctx.require("usage");
      const inGroup = new Set(r.members().map(m => m.did));
      for (const q of await index.requests(levelCode(sp.id))) {
        if (!q?.did || inGroup.has(q.did)) continue;
        const have = await levelNow(q.did);
        if (have !== null && usage.reaches(have, lv)) asks.push({ ...q, code: "level" });
      }
    }
    for (const inv of r.invites()) for (const q of await index.requests(inv.code)) asks.push({ ...q, code: inv.code });
    // OUT BY THE ACTS: a REMOVED member never comes back by a request (Settings says so); one who LEFT only by a request
    // made after they left (an old one still sits in the bag).
    const removedSet = new Set(r.acts().filter(a => a.act === "remove" && a.did).map(a => a.did));
    const leftAt = new Map(r.acts().filter(a => a.act === "leave").map(a => [a.did ?? a.by, a.at ?? 0]));
    const newest = new Map();
    for (const q of asks) {
      if (q.kind !== "join" || !q.did || r.banned(q.did) || removedSet.has(q.did) || (q.at ?? 0) <= (leftAt.get(q.did) ?? -1)) continue;
      if (inside.has(q.did) && !(admittedAt.has(q.did) && (q.at ?? 0) > admittedAt.get(q.did))) continue;
      if (!newest.has(q.did) || (q.at ?? 0) > newest.get(q.did).at) newest.set(q.did, { did: q.did, at: q.at ?? 0, code: q.code, kp: q.kp ?? null });
    }
    return {
      waiting: [...newest.values()].sort((x, y) => y.at - x.at),
      admitted: r.acts("admitted").map(a => ({ did: a.did, at: a.at ?? 0, by: a.by ?? null, code: a.code ?? null })).sort((x, y) => y.at - x.at).slice(0, 20),
    };
  }
  // LETTING PEOPLE IN — every way into every group, ONE path (owner 10-13: "one implementation"): each request read
  // (a server's codes, open door and level — `requestsOf`; the space's MEMBER bag, from those its rule says are in),
  // decided by `rules::admission` through the core — the request's own key, once per request, one admitter in turn;
  // the identity delegate runs the same rule with no page open — and let in by `welcome`. A JOIN is recorded
  // `admitted` first (the act, then the group). `only`: a member's own Let in of that asker (no turn waited). Each
  // request not let in is SAID why, once. Who was let in.
  const skipSaid = new Map();
  // ONE PASS AT A TIME per space (10-14, the journal on fx: upkeep's pass and another made two commits for one request
  // 3 s apart — `add`, then `readd` — the second deciding before the first's welcome was seen): a pass asked while one
  // runs waits for it, then decides over what it did.
  const admitting = new Map();
  const admit = (sp, only = null) => {
    // (Waited for, not judged: its own caller reports how it ended.)
    const run = Promise.allSettled([admitting.get(sp.id)]).then(() => admitOnce(sp, only));
    admitting.set(sp.id, run.finally(() => admitting.get(sp.id) === run && admitting.delete(sp.id)));
    return run;
  };
  async function admitOnce(sp, only = null) {
    const me = await space.account.strict();
    const g = keys.group(sp);
    const st = await g.ready();
    if (!me || !st || st.removed) return [];
    const server = sp.kind === "server";
    const r = server ? await (await ctx.require("roles")).of(sp) : null;
    if (r) await r.refresh();
    if (r && (r.left || !r.role(me.id))) return [];
    const used = await (await ctx.require("storage")).table("keypacks");
    await used.settled;
    const held = new Set(await groupDids(sp));
    const told = did => used.rows().some(x => x.value && x.key === `invited:${sp.id}|${did}`);
    // WHO MAY LET IN (the kind's rule): a server's members who may invite; a conversation's members — and one this
    // account told, by this account (no other member knows of it).
    const admitters = server ? r.members().filter(m => r.can(m.did, "invite")).map(m => m.did) : [...held];
    // WHO MAY ASK AGAIN (the member bag): a member by the acts; the other of a direct conversation; a group
    // conversation's member, or one this account told.
    const entitled = did => (server ? r.roster().includes(did) && !r.banned(did) : sp.kind === "direct" ? did === sp.with : held.has(did) || told(did));
    const asks = [];
    if (server && r.can(me.id, "invite")) for (const q of (await requestsOf(sp)).waiting) asks.push({ did: q.did, at: q.at, kp: q.kp, code: q.code, entry: "join" });
    await openMemberBag(sp);
    for (const q of await index.requests(memberBag(sp.id)))
      if (q?.kind === "stuck" && q.did && q.did !== me.id && entitled(q.did)) asks.push({ did: q.did, at: Number(q.at) || 0, kp: q.kp ?? null, entry: "again", epoch: q.epoch ?? null, branch: q.branch ?? null, seen: q.seen ?? null });
    const todo = asks.filter(q => !only || q.did === only);
    if (!todo.length) return [];
    const writers = await index.spacePointers.strict(sp, "writers");
    const welcomed = writers.filter(p => p?.welcomed && p.kp).map(p => [p.welcomed, p.kp, Number(p.at) || 0]);
    // KEYS SPENT: this account's record (`keypacks`) and every member's (the writers list's tags).
    const spent = new Set([...used.rows().filter(x => x.value).map(x => x.key.replace(/^shared:/, "")).filter(k => TAG.test(k)), ...sharedTags(writers).keys()]);
    const ordering = await ctx.require("ordering");
    const life = await ctx.require("lifecycle");
    const { glue } = await ctx.require("node");
    const name = sp.kind === "direct" ? ((await directory.card.strict(me.id))?.handle ?? short(me.id)) : sp.name;
    const out = [];
    for (const q of todo) {
      // WHERE ITS KEYS ARE (an ask again): on this branch, behind — `from` its epoch; elsewhere (another branch, none) —
      // let in onto the OWNER's branch (a repair), only by a node on it; ahead of this node — not this node's to answer.
      let from = null;
      let repair = null;
      if (q.entry === "again" && q.epoch != null) {
        if (Number(q.epoch) > st.epoch) continue;
        if (q.branch && (await g.fingerprint(Number(q.epoch))) === q.branch) from = Number(q.epoch);
        else if (server) {
          const own = await ctx.optional("conversation: ownerBranch", ownerBranch(sp), () => null);
          if (!own || (await g.fingerprint(own.epoch)) !== own.branch) continue;
          repair = { epoch: own.epoch, branch: own.branch };
        }
      }
      // THEIR OWN KEY, or nobody's request: its credential names them (their key log read).
      const notTheirs = q.kp ? await directory.keyNotTheirs(q.did, q.kp) : null;
      const solo = only || (!server && !held.has(q.did) && told(q.did));
      const view = { me: me.id, admitters: solo ? [me.id] : admitters, spent: [...spent], welcomed, held: held.has(q.did), epoch: st.epoch, now: Date.now(), takeover: ordering.TAKEOVER };
      const d = notTheirs ? { skip: notTheirs } : JSON.parse(glue.admission_decide(JSON.stringify({ did: q.did, kp: q.kp, at: q.at, entry: q.entry, from, seen: q.seen ?? null }), JSON.stringify(view)));
      // ITS STORY (the `member` machine, keyed by the request: reported): not let in here and why — said when the why
      // changes — or let in, welcomed, failed.
      const key = reqKey(q.kp ? glue.admission_tag(q.kp) : null, q.did);
      const about = { by: "page", s: sp.id.slice(0, 6), to: q.did.replace(/^did:craftec:/, "").slice(0, 6), entry: q.entry, ...(q.code ? { code: q.code === "open" || q.code === "level" ? q.code : "code" } : {}), e: st.epoch };
      if (!d.admit) {
        if (skipSaid.get(key) !== d.skip) {
          skipSaid.set(key, d.skip);
          ctx.log("conversation", { what: `${sp.name ?? "a conversation"}: ${short(q.did)}'s request (${q.entry}${q.code ? ` by ${q.code}` : ""}) not let in here — ${d.skip}` });
          life.transition("member", key, "skip", { ...about, why: d.skip });
        }
        if (only) throw new Error(d.skip);
        continue;
      }
      life.transition("member", key, "admit", about);
      try {
        if (q.entry === "join" && !r.roster().includes(q.did)) await r.act({ act: "admitted", code: q.code, did: q.did });
        const { kept } = await welcome(sp, q.did, name, q.entry === "join" ? q.code : null, d.admit, repair);
        welcomed.push([q.did, d.admit.tag, Date.now()]);
        spent.add(d.admit.tag);
        out.push(q.did);
        life.transition("member", key, kept ? "kept" : "welcomed", { ...about, from, repair: !!repair, e: (await g.ready())?.epoch ?? null, ...(kept ? { why: kept } : {}) });
        ctx.log("conversation", { what: `${directory.shown(q.did)} let into ${sp.name ?? "a conversation"} (${q.entry === "join" ? (q.code === "open" ? "open door" : q.code === "level" ? "level" : "a code") : repair ? "onto the owner's keys" : from != null ? `behind at ${from}` : "asked in"})` });
      } catch (e) {
        ctx.failed("conversation", e, `letting ${short(q.did)} into ${sp.name ?? "a conversation"}: ${e.message}`);
        life.transition("member", key, "fail", { ...about, why: String(e?.message ?? e).slice(0, 160) });
        if (only) throw e;
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
      // Its capabilities: the space's (whatever kind is kept for this audience).
      setup: async (_, gr) => {
        // (The always-on ones are on already: enabling them throws — every new audience group failed, audit 10-13.)
        for (const cap of r.enabled().filter(c => !r.base().includes(c))) await gr.enable(cap, true);
      },
    });
    await keepReaders(sp).catch(e => ctx.failed("audiences", e, `${space.shown(sp)}: ${e.message}`));
    return g;
  }
  // AN AUDIENCE KEPT IN STEP — by its maker, and by the space's admins in it too: its maker makes them the group's admins
  // (so they may add and remove), and an admin acts on a change still due TEN MINUTES after their page first saw it
  // (the maker online does it within a tick; two pages committing the same change at once would fork the group).
  const dueSince = new Map(); // `${group}|${did}|in|out` → when first seen due here
  const GRACE = 10 * 60 * 1000;
  async function keepReaders(sp) {
    const me = await space.me.strict();
    const [groups, roles] = await Promise.all(["groups", "roles"].map(n => ctx.require(n)));
    const r = await roles.of.strict(sp);
    await r.settled;
    const prefix = `audience:${sp.id}/`;
    const adminsOf = d => ["owner", "admin"].includes(r.role(d));
    for (const g of await space.mine.strict()) {
      if (!g.group?.startsWith(prefix)) continue;
      const who = g.group.slice(prefix.length);
      const want = new Set(r.members().map(x => x.did).filter(d => !r.banned(d) && r.passes(who, d)));
      const gr = await roles.of(g);
      if (!gr) continue;
      await gr.settled;
      const maker = g.governance?.owner === me;
      if (maker) {
        await groups.keep(g, want);
        // The space's admins in it (as it stands now): the group's admins too — they keep it when its maker is away.
        await gr.refresh?.();
        for (const m of gr.members())
          if (m.did !== me && adminsOf(m.did) && gr.role(m.did) === "member")
            await gr.grant(m.did, "admin").catch(e => ctx.failed("audiences", e, `${g.name ?? "an audience"}: making ${short(m.did)} its admin: ${e.message}`));
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
          const r = await roles.of.strict(server);
          const changed = [];
          const fire = () => changed.forEach(f => f());
          let its = [];
          let reading = null;
          const read = () =>
            (reading ??= ctx.background("conversation: items.inPlaces", items
              .inPlaces({ spaces: [server] }, "channel", { withVotes: false }).then(x => ((its = x), fire())))
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
          const me = async () => space.me.strict();
          const mayMake = async () => {
            if (!r.can(await me(), "channels")) throw new Error("only who may make channels here changes them");
          };
          // A channel ITEM: public where the space lets anyone read its chat, else its members'.
          const make = async (name, cid, meta = {}) =>
            items.submit({ board: server.id, title: channelName(name), body: "", kind: "channel", meta: { ...meta, cid }, audience: (await items.publicIn(server, "channel")) ? "public" : "members" });
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
    const mine = (await space.mine.strict()).find(s => s.id === id);
    const pub = mine ? null : await (await ctx.require("items")).publicSpace(id);
    const d = mine ?? pub;
    if (d) knownSpaces.set(id, d);
    return d ?? null;
  }
  // Its key's public halves, from its owner's record (the id proving the owner); null: no space mail (yet).
  async function spaceKey(desc) {
    const owner = desc && (await space.owner.strict(desc));
    if (!owner) return null;
    const t = await directory.publicOf.strict(owner, SPACEMAIL);
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
    const r = await roles.of.strict(sp);
    const me = await space.account.strict();
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
        await (await directory.publicOf.strict(me.id, SPACEMAIL)).put(sp.id, JSON.stringify(rec));
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
            const sealed = (await directory.publicOf.strict(it.from, MAIL))?.rows().find(x => x.key === it.key)?.value;
            if (!sealed) continue;
            const m = JSON.parse(new TextDecoder().decode(glue.CraftworksCore.open_with(s, bytesOf(sealed))));
            if (!(await genuine(m, it.from)) || !m.to?.includes(`space:${sp.id}`)) continue;
            await t.put(`in/${it.key}`, JSON.stringify(m));
            n += 1;
          } catch (e) {
            ctx.failed("mail", e, `${sp.name}: a mail from ${short(it.from)} did not open: ${e.message}`);
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
    const box = await directory.publicOf.strict(me.id, MAIL);
    const text = new TextEncoder().encode(JSON.stringify(m));
    const where = await Promise.all(
      m.to.map(async d => {
        if (d.startsWith("space:")) {
          const desc = await spaceDesc(d.slice(6));
          const key = await spaceKey(desc);
          if (!key?.box) throw new Error(`${desc?.name ?? "that space"} has no mail`);
          return { d, pub: bytesOf(key.box), drop: item => index.sendTo(glue.CraftworksCore.inbox_address(bytesOf(d.slice(6))), bytesOf(key.box), item) };
        }
        const card = await directory.card.strict(d);
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
    accepts: async id => !!(await ctx.optional("conversation: spaceKey", spaceKey(await spaceDesc(id)), () => null))?.box,
    // WHOM mail can be written to BY NAME (the "To" picker): the people this person knows (friends, followed) and the
    // spaces whose mail is on — theirs and the public ones listed. [{ ref, label, kind: "person" | "space" }]
    async addresses() {
      const people = await (await ctx.require("edge")).people();
      const dids = [...new Set([...people.list("friend"), ...people.list("follow")])].filter(d => d.startsWith("did:"));
      const persons = dids.map(d => ({ ref: d, label: directory.shown(d), kind: "person" }));
      const listed = await ctx.optional("conversation: publicSpaces", (await ctx.require("items")).publicSpaces(), () => []);
      const descs = [...(await space.mine.strict()).filter(s => s.kind === "server" && !space.isGroup(s)), ...listed];
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
      const me = await space.account.strict();
      if (!me) throw space.loginNeeded();
      to = [...new Set(to)];
      if (!to.length) throw new Error("to nobody");
      const m = { id: `${Date.now().toString(36)}.${newId()}`, from: me.id, to, subject: String(subject ?? ""), body: String(body ?? ""), at: Date.now(), re, ...(files.length ? { files } : {}) };
      await deliver(m, me);
      await (await kept()).put(`sent/${m.id}`, JSON.stringify(m));
      mail.prune().catch(e => ctx.failed("mail", e, `pruning: ${e.message}`));
      return m;
    },
    // OLD MAIL in this account's public tail: each recipient keeps what they opened (their `mailbox`), so a sealed
    // copy older than 30 days is dropped (a mail's id starts with when it was sent, base 36; one without is left).
    async prune() {
      const me = await space.account.strict();
      const box = me && (await directory.publicOf.strict(me.id, MAIL));
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
      const me = await space.account.strict();
      if (!me) return 0;
      const t = await kept();
      const have = new Set(t.rows().map(r => r.key));
      let n = 0;
      const p = await people();
      for (const it of await index.inbox.strict()) {
        if (it.kind !== "mail" || !it.from || !it.key || have.has(`in/${it.key}`) || p.is("block", it.from)) continue;
        try {
          const box = await directory.publicOf.strict(it.from, MAIL);
          const sealed = box?.rows().find(r => r.key === it.key)?.value;
          if (!sealed) continue;
          const r = await (await ctx.require("auth")).identity.inboxOpen([bytesOf(sealed)]);
          const m = JSON.parse(new TextDecoder().decode(bytesOf(r.opened?.[0] ?? "")));
          // Who sent it is whose tail it was in (from a space: its key's signature) — never what it says.
          if (!(await genuine(m, it.from)) || !m.to?.includes(me.id)) continue;
          await t.put(`in/${it.key}`, JSON.stringify(m));
          n += 1;
        } catch (e) {
          ctx.failed("mail", e, `a mail from ${short(it.from)} did not open: ${e.message}`);
        }
      }
      return n;
    },
    list: async (which = "in") => listOf(await kept(), which),
    onChange: async f => (await kept()).onChange(f),
  };

  return { direct, group, invite, accept, joinLevel, lapse, levelCode, catchUp, keyStatus, reportReads, selfState, observe, list, members: ctx.total("conversation.members", () => [], members), person: ctx.total("conversation.person", () => null, person), mail, createInvite, revokeInvite, join, joinOpen, asked: ctx.total("conversation.asked", () => null, asked), askedCodes: ctx.total("conversation.askedCodes", () => [], askedCodes), askStuck, waiting: ctx.total("conversation.waiting", () => [], waiting), requestsOf, letIn, setJoin, admit, befriend, friendRequests: ctx.total("conversation.friendRequests", () => [], friendRequests), answerFriend, unfriend, channels , keepReaders, audience, resendWelcomes, mirrorShared, openMemberBag, reqKey };
}
