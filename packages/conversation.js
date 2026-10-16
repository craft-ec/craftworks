// CONVERSATION, a capability: people talking. KINDS only where the mechanism differs:
// - `direct`: two people. Their own space and group; the other is put on its list and TOLD in their inbox (they may be
//   away): their device enters by itself when it reads it. Each person's messages in their own feed.
// - `group`: a space's members (a server; its channels are sub-spaces). Joining by invite. (Its page is Chat.)
// - `mail`: items addressed per item. A mail lives in its SENDER's public tail `mail` (only their account writes it: that
//   is what proves who sent it), sealed to each recipient's inbox key; a POINTER to it goes in each recipient's inbox.
//   The recipient opens it and keeps it in their own table `mailbox` (so the sender may drop old ones).
// An announcement is a group message where only some roles post; a thread is items replying to items — compositions,
// not kinds. The items themselves are `content`'s.
//
//   const conversation = await ctx.require("conversation");
//   await conversation.direct(did)   // a direct conversation with that person (made, they listed and told)
//   await conversation.invite(sp, did) // that person onto a space's list, told — their device enters by itself
//   await conversation.accept()      // spaces this account is told it is in: entered by this device
//   await conversation.list()        // this account's conversations: direct ones and groups
//   await conversation.group([did…], name)   // a group conversation (you and two or more others), each listed and told
//   await conversation.members(sp)   // the accounts (DIDs) whose nodes are in a space's group
//   await conversation.person(text)  // a DID from `did:craftec:…`, or from `name#abc123` among the people this account knows
//   await conversation.createInvite(sp, { days, uses })  // an INVITE CODE for a space (0: no limit): "xxxx-xxxx-xxxx-xxxx"
//   await conversation.revokeInvite(sp, code)
//   await conversation.join(code)    // in by a code: entered by itself, its proof bound to this DID (`gov::join_proof`)
//   await conversation.joinOpen(desc) // a space seen from outside: its open door (entered at once), or a request ("ask")
//   await conversation.requestJoin(d)  cancelJoin(id)  letIn(sp, did)  reject(sp, did, why)  requestsOf(sp)
//                                    // A REQUEST's lifecycle: asked → cancelled | let in | rejected — never hanging
//   await conversation.befriend(did) // a FRIEND request (their inbox); friends once both have asked, or they accept
//   await conversation.friendRequests()   // who asked this person, not yet answered: [did]
//   await conversation.answerFriend(did, yes)
//   await conversation.unfriend(did)      // no longer friends — on both sides (a notice in their inbox)
//   (A BLOCKED person's notices, mail and requests are left unopened: `edge.people`.)
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
  const [space, keys, directory, index, content, storage, node] = await Promise.all(["space", "keys", "directory", "index", "content", "storage", "node"].map(n => ctx.require(n)));
  // The core: the one place each rule and guard is (`rules`, through the core).
  const glueOf = () => node.glue;
  const bytesOf = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const short = did => `${did.replace(/^did:craftec:/, "").slice(0, 8)}…`;

  // ─── MEMBERSHIP (FOUNDATION "Members and keys", the design of 10-17) ───
  // Being IN a space is being ON ITS LIST — its acts, replayed alike on every node; a listed device ENTERS the space's
  // keys BY ITSELF (`keys.group(sp).enter`: an MLS external commit — nobody answers, nobody else need be online). The
  // ways onto a list: PUT ON IT by one who may (an invite, a conversation's people, Let in — `listIn`, then TOLD in
  // their inbox); by a CODE, the OPEN door or a LEVEL (their own entry carries the proof — `Gov::admits` — then their
  // own `join` act); or a REQUEST where the space asks for one: asked, cancelled, let in or rejected — never hanging.
  const bagOf = (code, id) => glueOf().bag_name(code, bytesOf(id));
  // A space's REQUESTS bag (its own: `open <space>`).
  const openCode = id => bagOf("open", id);
  const levelCode = id => bagOf("level", id);
  // What a newcomer needs to know a space and check its id (`space.idOf(owner, nonce)`).
  const descOf = sp => ({ id: sp.id, kind: sp.kind, name: sp.name, owner: sp.governance?.owner ?? null, nonce: sp.governance?.nonce ?? null, ...(sp.circle ? { circle: sp.circle } : {}), ...(sp.group ? { group: sp.group } : {}) });
  const rowOf = (d, extra = {}) => ({ kind: d.kind, name: d.name, owner: d.owner, nonce: d.nonce ?? null, ...(d.circle ? { circle: d.circle } : {}), ...(d.group ? { group: d.group } : {}), ...extra });
  // KNOWN AND ENTERED: the space recorded as this account's, this device in its keys (checked by every reader against
  // the list — `proof` for one entering by a code, the open door or a level).
  async function enterSpace(d, why, { proof = null, extra = {} } = {}) {
    if (!d?.id || !d.owner || !d.nonce || (await space.idOf(d.owner, d.nonce)) !== d.id) throw new Error("that space's description does not match its id");
    const v = rowOf(d, extra);
    const sp = await space.describe(d.id, v);
    const g = keys.group(sp);
    const st = await g.ready();
    if (!st || st.removed) await g.enter(why, proof);
    await space.record(d.id, v);
    (await ctx.require("lifecycle")).transition("join", d.id, "joined", { s: d.id.slice(0, 6), why });
    return sp;
  }
  // ENTERED BY A PROOF, then ON THE LIST for good: its own `join` act (counted by the very rule its entry was judged by).
  async function joinBy(d, proof, why, extra = {}) {
    const me = await space.account.strict();
    if (!me) throw space.loginNeeded();
    const sp = await enterSpace(d, why, { proof, extra });
    const r = await (await ctx.require("roles")).of(sp);
    await r.refresh?.();
    if (!r.roster().includes(me.id)) await r.act({ act: "join", ...proof });
    await ctx.background("conversation: forgetAsk", forgetAsk(d.id));
    ctx.log("conversation", { what: `in ${d.name ?? "a space"} — entered by itself (${proof.join})` });
    return sp;
  }
  // TOLD: in their inbox, that they are on a space's list (an invite, a conversation started with them, Let in) — with
  // what the space is. Their device enters by itself (`accept`).
  async function tell(sp, did, name) {
    const me = await space.account.strict();
    if (!me) throw space.loginNeeded();
    if (!(await directory.card.strict(did))) throw new Error("that person has no card yet");
    await index.send(did, { kind: "invited", space: sp.id, desc: { ...descOf(sp), name }, from: me.id, at: Date.now() });
    ctx.log("conversation", { what: `${short(did)} told they are in ${sp.name ?? "a conversation"}: their device enters by itself` });
  }
  // PUT ON THE LIST (`added`) by one who may — the one way a member puts someone on a list (an invite, Let in, a
  // conversation's people): refused by the guard "may list" (the node's too) before it is written.
  async function listIn(sp, did) {
    const r = await (await ctx.require("roles")).of(sp);
    const why = r.mayList(did);
    if (why) throw new Error(why);
    await r.act({ act: "added", did });
  }
  // DIRECT: a two-person space, the other on its list and told — or the one this person already has with them.
  async function direct(did) {
    const had = (await space.mine.strict()).find(s => s.kind === "direct" && s.with === did);
    if (had) return had;
    const me = await space.account.strict();
    const card = await directory.card.strict(did);
    if (!card) throw new Error("that person has no card yet");
    const sp = await space.create("direct", card.handle ?? short(did), { with: did });
    await listIn(sp, did);
    await tell(sp, did, (await directory.card.strict(me.id))?.handle ?? short(me.id));
    return sp;
  }
  // INVITE a person into a space this account is in: on its list, then told.
  async function invite(sp, did) {
    await listIn(sp, did);
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
  // ON THE OWNER's BRANCH — the one test, for this device and for what a page shows of another member: at or past the
  // owner's newest epoch, by its fingerprint there; behind, by the owner's announcement at its OWN epoch; else not known
  // (null). `fpAt(e)`: its fingerprint at epoch e (or null); `owns`: the owner's announcements, newest first.
  async function onBranch(epoch, fpAt, owns) {
    // The rule is `guards::on_branch` (through the core): here only its inputs — the owner's announcements and this
    // one's fingerprints at the two epochs it may compare.
    if (epoch == null || !owns.length) return null;
    const fps = {};
    for (const e of new Set([owns[0].epoch, epoch])) {
      const fp = await fpAt(e);
      if (fp != null) fps[e] = fp;
    }
    return glueOf().on_branch(epoch, JSON.stringify(owns.map(a => [a.epoch, a.branch])), JSON.stringify(fps)) ?? null;
  }

  // EVERY MEMBER's KEYS, as their devices announced them in the space's writers list (any member reads it): each one
  // on the owner's keys now, BEHIND (an older epoch), on ANOTHER BRANCH, or not announced — and who caught them up.
  // `Map did → { state, epoch, branch, devices, repairedBy }`, and the owner's own `{ epoch, branch }`.
  async function keyStatus(sp) {
    const r = await (await ctx.require("roles")).of(sp);
    await r.settled;
    const ptrs = await index.spacePointers(sp, "writers");
    const owns = await ctx.optional("conversation: ownerBranches", ownerBranches(sp), () => []);
    const own = owns[0] ?? null;
    const out = new Map();
    const w = await storage.writersOf(sp);
    for (const m of r.members()) {
      const devs = devicesNow(w, m.did);
      const anns = ptrs.filter(p => p?.branch && devs.includes(p.w)).sort((a, b) => b.epoch - a.epoch);
      const a = anns[0] ?? null;
      // NO DEVICES READ here is not "not announced": what this page could not read, said as such.
      const unreadDevs = !devs.length ? directory.devicesWhy(m.did) : null;
      // Their state by the ONE classification (`guards::group_state`) over what they announced, shown in the members' words.
      const named = { held: "current", behind: "behind", "off-branch": "another branch", ahead: "ahead", unreadable: "current", "not-held": "not announced" };
      const state = unreadDevs ? "devices not read" : !a ? "not announced" : !own ? "unknown" : named[glueOf().group_state(a.epoch, own.epoch, (await onBranch(a.epoch, async e => (e === a.epoch ? a.branch : null), owns)) ?? undefined, false, false)];
      // What their page REPORTED it cannot read here (`reportReads`): the newest report of each of their devices.
      const reports = devs.map(w => ptrs.filter(p => p?.reads && p.w === w).sort((x, y) => (y.reads.at ?? 0) - (x.reads.at ?? 0))[0]).filter(Boolean);
      const unread = [...new Set(reports.flatMap(p => (Array.isArray(p.reads.unread) ? p.reads.unread : []).map(String)))].slice(0, 6);
      out.set(m.did, { state, why: unreadDevs, devs: devs.length, epoch: a?.epoch ?? null, branch: a?.branch ?? null, devices: anns.length, unread, reported: Math.max(0, ...reports.map(p => p.reads.at ?? 0)) || null });
    }
    return { owner: own ? { epoch: own.epoch, branch: own.branch } : null, writers: ptrs.length, members: out };
  }
  // TELEMETRY (owner 10-11: "more telemetry so you can investigate and monitor our applications health throughout the
  // network"), into the day's public health bag — never content. THIS PAGE's own state in a space (the heartbeat's
  // line): `{ s, e, oe, on, u }` — its keys' epoch, the owner's, whether it is on the owner's branch, what it cannot read.
  async function selfState(sp) {
    const g = keys.group(sp);
    const st = await g.ready();
    // NOT HELD only when the device's key table, read again, says so (still arriving: `why` — no state, no ask).
    if (!st && (await ctx.optional("conversation: g.holds", g.holds(), () => null)) !== false) return { s: sp.id.slice(0, 6), e: null, why: "its keys not read yet" };
    const owns = await ctx.optional("conversation: ownerBranches", ownerBranches(sp), () => []);
    const own = owns[0] ?? null;
    const on = await onBranch(st?.epoch ?? null, e => g.fingerprint(e), owns);
    // UNREADABLE: this device's own key gaps only (another member's feed failing is theirs).
    const u = (await ctx.require("storage")).keyGapsIn(sp.id);
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
  // A space's HISTORY handed over (an answer to a history ask): each earlier epoch's secret kept.
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
  // ENTER BY ITSELF — the `group` machine's REACTION (FOUNDATION "Members and keys" 1): entering a state reading the
  // logs cannot fix (`guards::cannot_read`: never held, another branch, unreadable, stuck — never behind), this device
  // enters again by itself. Every reader checks it against the list; one not listed is skipped by all.
  (await ctx.require("lifecycle")).on("group", async (key, to, from) => {
    const { glue } = await ctx.require("node");
    const state = String(to).toLowerCase().replace(/_/g, "-");
    if (to === from || !glue.group_cannot_read(state)) return;
    const sp = (await space.mine.strict()).find(s => s.id === key);
    if (sp) await keys.group(sp).enter(state).catch(e => ctx.log("conversation", { what: `${sp.name ?? "a space"}: entering by itself (${state}): ${e?.message ?? e}` }));
  });

  // TOLD and HANDED OVER, from this account's inbox — ONE AT A TIME (the chat page and upkeep both ask): each space it
  // is TOLD it is in, entered by this device (`enterSpace`); a REJECTION of its request, ended; history asks answered;
  // history handed over, kept. What it entered.
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
    // Kept once (`keypacks`, the account's): each notice acted on once.
    const once = await (await ctx.require("storage")).table("keypacks");
    await once.settled;
    const done = k => once.rows().some(r => r.key === k && r.value);
    for (const it of await index.inbox.strict()) {
      if (!it.from || p.is("block", it.from)) continue;
      // TOLD it is in: entered by this device (a direct conversation's other is its teller).
      if (it.kind === "invited" && it.desc?.id) {
        const k = `invited:${it.desc.id}|${it.from}|${it.at ?? ""}`;
        if (done(k)) continue;
        const had = mine.find(s => s.id === it.desc.id);
        if (had && (await keys.group(had).ready())) {
          await ctx.background("conversation: once.put", once.put(k, String(Date.now())));
          continue;
        }
        try {
          out.push(await enterSpace(it.desc, "told", { extra: it.desc.kind === "direct" ? { with: it.from } : {} }));
          await once.put(k, String(Date.now()));
          await ctx.background("conversation: forgetAsk", forgetAsk(it.desc.id));
          ctx.log("conversation", { what: `told by ${short(it.from)}: in ${it.desc.name ?? "a conversation"}` });
        } catch (e) {
          // Not done: tried again next pass (its group info not published yet, a read that failed).
          ctx.failed("conversation", e, `entering ${it.desc.name ?? "a conversation"}: ${e?.message ?? e}`);
        }
        continue;
      }
      // REJECTED: this account's request there ended (said; asked anew only by asking anew).
      if (it.kind === "rejected" && it.space) {
        const k = `rejected:${it.space}|${it.at ?? ""}`;
        if (done(k)) continue;
        // FROM ONE WHO MAY (the space's public acts — a space that takes requests publishes them): anyone else's
        // "rejected" ends nothing (review 10-17).
        const ask = await asked(it.space);
        const pr = ask?.desc ? await ctx.optional("conversation: ofPublic", (await ctx.require("roles")).ofPublic({ id: it.space, name: ask.desc.name, governance: { kind: "owner", owner: ask.desc.owner, nonce: ask.desc.nonce } }), () => null) : null;
        await pr?.settled;
        if (!pr?.can?.(it.from, "invite")) continue;
        await endAsk(it.space, "rejected", it.why ?? null);
        await once.put(k, String(Date.now()));
        continue;
      }
      // A HISTORY ASK (a member's walk stopped): their space's earlier secrets held here, sealed to them — once each.
      if (it.kind === "history-ask") {
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
      if (it.kind === "history" && Array.isArray(it.epochs)) {
        const k = `history-kept:${it.space}|${it.from}|${it.at}`;
        const sp = mine.find(s => s.id === it.space);
        if (!sp || done(k)) continue;
        await keepHistory(sp, it.epochs);
        await once.put(k, String(Date.now()));
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
    for (const d of others) await invite(sp, d);

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

  // INVITE CODES. A code is an act in the space's log (who made it, when it expires, how many it admits: `roles`); the
  // bag the code names holds what the space is (sealed by the code: whoever holds the code reads it). Who holds it
  // ENTERS by itself, its entry carrying the code's PROOF for its own DID (`gov::join_proof`: seen, it lets nobody
  // else in) — the uses counted from the `join` acts.
  const hex4 = () => [...crypto.getRandomValues(new Uint8Array(2))].map(x => x.toString(16).padStart(2, "0")).join("");
  const codeOf = text => String(text ?? "").trim().toLowerCase().replace(/[^0-9a-f]/g, "").replace(/(.{4})(?=.)/g, "$1-");
  async function createInvite(sp, { days = 7, uses = 0 } = {}) {
    const r = await (await ctx.require("roles")).of(sp);
    const code = [hex4(), hex4(), hex4(), hex4()].join("-");
    await index.openRequests(code);
    await r.act({ act: "invite", code, expires: days ? Date.now() + days * 86400000 : 0, uses });
    await index.request(code, { kind: "space", desc: descOf(sp), at: Date.now() });
    return code;
  }
  const revokeInvite = async (sp, code) => (await (await ctx.require("roles")).of(sp)).act({ act: "revoke-invite", code });
  async function join(text) {
    const me = await space.account.strict();
    if (!me) throw space.loginNeeded();
    const code = codeOf(text);
    if (code.length !== 19) throw new Error("an invite code is 16 letters and digits: xxxx-xxxx-xxxx-xxxx");
    const d = (await index.requests(code)).find(q => q?.kind === "space" && q.desc?.id)?.desc;
    // A CODE made before codes carried their space: a REQUEST, decided by the space's people (Settings: Let in or
    // Reject) — never left hanging: this person may cancel it.
    if (!d) {
      await index.request(code, { kind: "join", did: me.id, at: Date.now() });
      await noteAsk(`code:${code}`, { code });
      return code;
    }
    await joinBy(d, { join: "code", p: glueOf().join_proof(code, me.id) }, "code");
    return code;
  }
  // A LEVEL GROUP's level: its join policy by the one rule (`gov::level_of`).
  const levelOfGroup = r => glueOf().level_of_policy(r.policy("", "join") ?? "") ?? null;
  // Their card's contributions as last read — kept TEN MINUTES (every member's page asks of every member each tick); a
  // card that did not read is unknown, never "free" (a failed read must not put a giver out).
  const levelsRead = new Map(); // did → { at, contrib }
  // THE GUARD "level reached" (`rules::guards::level`, the node's too) over their card as read now (kept 10 minutes):
  // why not, `null` when it is reached, `undefined` when their card does not read.
  async function levelWhy(did, need) {
    let had = levelsRead.get(did);
    if (!had || Date.now() - had.at >= 600000) {
      const card = await directory.card(did, { fresh: true });
      if (!card) return undefined;
      had = { at: Date.now(), contrib: JSON.stringify(card.contrib ?? []) };
      levelsRead.set(did, had);
    }
    return (await ctx.require("node")).glue.guard_level(had.contrib, need, Date.now()) ?? null;
  }
  // A LEVEL GROUP (rewards §5a: a creator's PRO, VIP): entered by itself with the level as its proof (the honor system:
  // a member's `lapsed` takes out one whose card falls short). `gate`: the item's `{ space, name, owner, nonce }`.
  async function joinLevel(gate) {
    const d = { id: gate.space, kind: gate.kind ?? "group", name: gate.name ?? null, owner: gate.owner, nonce: gate.nonce, ...(gate.group ? { group: gate.group } : {}) };
    return joinBy(d, { join: "level" }, "level");
  }
  // LAPSED: in a level group, any member (in turn) takes out another whose card no longer reaches its level — a
  // `lapsed` act (not a ban: they enter again when they give again); the node's follower takes their entry out. Who.
  async function lapse(sp) {
    const me = await space.account.strict();
    const r = await (await ctx.require("roles")).of(sp);
    const lv = levelOfGroup(r);
    if (!me || !lv || r.left || !r.role(me.id)) return [];
    const ordering = await ctx.require("ordering");
    const members = r.members().map(m => m.did);
    const out = [];
    for (const did of members) {
      if (did === me.id || did === r.owner) continue;
      const why = await levelWhy(did, lv);
      if (why == null) continue;
      const k = `lapse|${sp.id}|${did}`;
      if (!pendingSince.has(k)) pendingSince.set(k, Date.now());
      // MY TURN (the guard), ranked among the OTHER members (the lapsed one never acts on itself).
      if (glueOf().guard_lapse_turn(ordering.turn(k, members.filter(d => d !== did), me.id), pendingSince.get(k), Date.now())) continue;
      await r.act({ act: "lapsed", did }).then(
        () => (pendingSince.delete(k), out.push(did), ctx.log("conversation", { what: `${sp.name}: ${short(did)}'s level ran out — taken out (they enter again when they give again)` })),
        e => ctx.failed("conversation", e, `${sp.name}: lapsing ${short(did)}: ${e.message}`),
      );
    }
    return out;
  }
  const pendingSince = new Map(); // what `lapse` saw pending: when this page first saw it
  // JOINING A SPACE SEEN FROM OUTSIDE (Discover, a profile): by its join policy — the OPEN door ("anyone"): entered at
  // once by itself; "ask": a REQUEST its people decide.
  async function joinOpen(desc) {
    const pr = await (await ctx.require("roles")).ofPublic(desc);
    await pr?.settled;
    const how = pr?.policy("", "join");
    const d = { id: desc.id, kind: desc.kind ?? "server", name: desc.name ?? null, owner: desc.governance?.owner ?? desc.owner, nonce: desc.governance?.nonce ?? desc.nonce };
    if (how === "anyone") return joinBy(d, { join: "open" }, "open");
    if (how === "ask") return requestJoin(d);
    throw new Error("this space lets people in only by an invite");
  }

  // ─── A REQUEST (owner 10-17: "user can cancel request to join; for manual approval, can be rejected — so requests are
  // not hanging") — its lifecycle (`join` machine, keyed by the space): REQUESTED → CANCELLED (the asker) | LET IN (one
  // who may invite: on the list, told — they enter) | REJECTED (one who may invite: a `reject` act, told). Each ends;
  // asking after an end is a new request. Items in the space's requests bag: `{ kind: "join" | "cancel", did, at }`.
  async function requestJoin(d) {
    const me = await space.account.strict();
    if (!me) throw space.loginNeeded();
    await ctx.background("conversation: directory.publish", directory.publish()); // their card: where they are told
    await index.openRequests(openCode(d.id));
    await index.request(openCode(d.id), { kind: "join", did: me.id, at: Date.now() });
    await noteAsk(d.id, { name: d.name ?? null, desc: d });
    (await ctx.require("lifecycle")).transition("join", d.id, "ask", { s: d.id.slice(0, 6), name: d.name ?? null });
    ctx.log("conversation", { what: `asked to join ${d.name ?? d.id.slice(0, 8)}` });
  }
  // `key`: the space's id, or `code:<code>` (a request by a code made before codes carried their space).
  async function cancelJoin(key) {
    const me = await space.account.strict();
    if (!me) throw space.loginNeeded();
    await index.request(key.startsWith("code:") ? key.slice(5) : openCode(key), { kind: "cancel", did: me.id, at: Date.now() });
    await endAsk(key, "cancelled");
  }
  // ENDED (cancelled, rejected — let in ends as entering): the request kept no longer, its end said.
  async function endAsk(id, how, why = null) {
    await forgetAsk(id);
    (await ctx.require("lifecycle")).transition("join", id, how, { s: id.slice(0, 6), ...(why ? { why } : {}) });
    ctx.log("conversation", { what: `the request to join ${id.slice(0, 8)} ${how}${why ? `: ${why}` : ""}` });
  }
  // LET IN (one who may invite): on the list, told — they enter by themselves.
  const letIn = async (sp, did) => {
    await listIn(sp, did);
    await tell(sp, did, sp.name);
  };
  // REJECT (one who may invite): a `reject` act (the request ended, for every member's Settings), and the asker told.
  async function reject(sp, did, why = null) {
    const r = await (await ctx.require("roles")).of(sp);
    await r.act({ act: "reject", did, ...(why ? { why: String(why).slice(0, 200) } : {}) });
    await ctx.background("conversation: index.send", index.send(did, { kind: "rejected", space: sp.id, from: (await space.me.strict()), ...(why ? { why: String(why).slice(0, 200) } : {}), at: Date.now() }));
  }
  // THE REQUESTS of a space, each where it stands — for its Settings: `{ waiting: [{ did, at, code }], admitted }`. A
  // request WAITS while its newest item is a join, made after the asker's last let in / reject, from one not listed and
  // in standing (the guard). The space's own requests bag, and its codes' (codes made before they carried the space).
  async function requestsOf(sp) {
    const r = await (await ctx.require("roles")).of(sp);
    await r.refresh();
    const inside = new Set(r.roster());
    const items = [...(await index.requests(openCode(sp.id))).map(q => ({ ...q, code: "ask" }))];
    for (const inv of r.invites()) for (const q of await index.requests(inv.code)) items.push({ ...q, code: inv.code });
    const decided = new Map(); // did → when last let in or rejected
    for (const a of [...r.acts("added"), ...r.acts("reject")]) if (a.did) decided.set(a.did, Math.max(decided.get(a.did) ?? 0, Number(a.at) || 0));
    const newest = new Map();
    for (const q of items) if ((q.kind === "join" || q.kind === "cancel") && q.did && (Number(q.at) || 0) >= (newest.get(q.did)?.at ?? 0)) newest.set(q.did, { ...q, at: Number(q.at) || 0 });
    // EVERY request waiting is shown — one from someone out of standing too (removed, banned: `out` says why), so it is
    // let in (an `added` brings them back) or rejected, never left unseen (owner 10-17: no request hangs).
    const waiting = [...newest.values()].filter(q => q.kind === "join" && !inside.has(q.did) && q.at > (decided.get(q.did) ?? 0));
    return {
      waiting: waiting.map(q => ({ did: q.did, at: q.at, code: q.code, out: r.standing(q.did, q.at) ?? null })).sort((x, y) => y.at - x.at),
      admitted: r.acts("join").map(a => ({ did: a.by, at: a.at ?? 0, code: a.join ?? null })).concat(r.acts("added").map(a => ({ did: a.did, at: a.at ?? 0, by: a.by ?? null, code: "added" }))).sort((x, y) => y.at - x.at).slice(0, 20),
    };
  }
  // REQUESTS this account made and is still waiting on (its table `asks`: every device shows them): by space id, or
  // `code:<code>`. Gone once in, cancelled or rejected.
  const asks = async () => (await ctx.require("storage")).table("asks");
  async function noteAsk(key, v) {
    await (await asks()).put(key, JSON.stringify({ ...v, at: Date.now() })).catch(e => ctx.failed("conversation", e, `keeping the request: ${e.message}`));
  }
  async function forgetAsk(key) {
    const t = await asks();
    await t.settled;
    for (const r of t.rows().filter(x => x.value && (x.key === key || x.key === `code:${key}`))) await t.remove(r.key);
  }
  const parseAsk = r => {
    try {
      return { key: r.key, ...JSON.parse(r.value) };
    } catch {
      return null;
    }
  };
  // THE REQUESTS WAITING: `[{ key, id, name, code, at, status: "asked" | "code" }]` (the spaces panel lists them, each
  // with Cancel).
  async function waiting() {
    const t = await asks();
    await t.settled;
    const mine = new Set((await space.mine.strict()).map(s => s.id));
    return t.rows().filter(x => x.value && !mine.has(x.key)).map(parseAsk).filter(Boolean).map(a => (a.key.startsWith("code:") ? { key: a.key, code: a.key.slice(5), name: a.name ?? null, at: a.at, status: "code" } : { key: a.key, id: a.key, name: a.name ?? null, at: a.at, status: "asked" }));
  }
  // The request for this space, while not in it: `{ at, name }`, or null.
  async function asked(id) {
    if ((await space.mine.strict()).some(s => s.id === id)) return null;
    const t = await ctx.optional("conversation: asks", asks(), () => null);
    await t?.settled;
    const r = t?.rows().find(x => x.key === id && x.value);
    return r ? parseAsk(r) : null;
  }
  async function setJoin(sp, how) {
    const r = await (await ctx.require("roles")).of(sp);
    if (how === "open" || how === "ask") await index.openRequests(openCode(sp.id));
    await r.act({ act: "policy", path: "", action: "join", who: how === "open" ? "anyone" : how === "ask" ? "ask" : "members" });
    // Open or asked: listed in Discover, its acts published (who is in, how to join).
    if (how !== "members") (await ctx.background("conversation: r.publish", r.publish()), await index.listSpace(sp));
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

  return { direct, group, invite, accept, joinLevel, lapse, bagOf, keyStatus, reportReads, selfState, observe, list, members: ctx.total("conversation.members", () => [], members), person: ctx.total("conversation.person", () => null, person), mail, createInvite, revokeInvite, join, joinOpen, requestJoin, cancelJoin, asked: ctx.total("conversation.asked", () => null, asked), waiting: ctx.total("conversation.waiting", () => [], waiting), requestsOf, letIn, reject, setJoin, befriend, friendRequests: ctx.total("conversation.friendRequests", () => [], friendRequests), answerFriend, unfriend, channels , keepReaders, audience };
}
