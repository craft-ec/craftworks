// UPKEEP, a service (started by the header, so on every page, after the page is up): what keeps this person's spaces
// current without any one page open — every 30 s (and at once): WELCOMES waiting in the inbox joined (a space someone
// let this person into appears in the spaces panel), and, in every shared space where this person may invite, whoever ASKED
// to join (an invite code, an open space) let in. The pages that did this only while open (Chat, a space's Home) no
// longer need to.
//
// With NO page open, the IDENTITY DELEGATE admits (the node wakes it every minute; node ≥ 0.2.139, with the Background
// grant) by the MANDATE handed over here each tick: per space where this person may invite, how people get in, who is
// in, and its MLS group. What it did meanwhile is taken first on every tick: the groups it moved, loaded; the people it
// let in, written as `admitted` acts (when it happened). A page that ticks keeps the delegate out of its way.
export async function start(ctx) {
  const [space, conversation, roles, keys, auth, index] = await Promise.all(["space", "conversation", "roles", "keys", "auth", "index"].map(n => ctx.require(n)));
  // The account's record of key packages used (`conversation.welcome` writes it too).
  const storage = async () => {
    const t = await (await ctx.require("storage")).table("keypacks");
    await t.settled;
    return t;
  };
  let running = false;
  let renewed = false; // key packages renewed this page (once)
  const published = new Set(); // open spaces whose roster this page made public
  async function tick() {
    if (running) return;
    running = true;
    try {
      const me = await space.account();
      if (!me) return;
      await absorb(me).catch(e => ctx.log("upkeep", { what: `what the delegate did: ${e.message}` }));
      // THE CARD as it should be (once a page): its key packages ones this account answers — none means every welcome
      // to it fails — a fresh set put there otherwise (whoever invites next picks one of them).
      if (!renewed) {
        renewed = true;
        await (await ctx.require("directory")).publish().catch(e => ((renewed = false), ctx.log("upkeep", { what: `the card: ${e.message}` })));
        await conversation.askStuck().catch(e => ctx.log("upkeep", { what: `requests let in, never in: ${e.message}` }));
      }
      // WELCOMES KEPT UNSENT (their commit made, the send failed): sent again, on their own wait (`conversation`).
      await conversation.resendWelcomes().catch(e => ctx.log("upkeep", { what: `welcomes kept to send again: ${e.message}` }));
      const joined = await conversation.accept().catch(e => (ctx.log("upkeep", { what: `the inbox: ${e.message}` }), []));
      if (joined.length) ctx.log("upkeep", { what: `joined ${joined.length} space(s) from the inbox` });
      // BLOG: this person's scheduled drafts published when due, and approved ones (`blog-press`) — by any page open.
      await (await ctx.require("blog-press")).publishDue().catch(e => ctx.log("upkeep", { what: `blog drafts: ${e.message}` }));
      // THE NODE's list first (before each space's slower work): what it keeps with no page open.
      await mandate(me).catch(e => ctx.log("upkeep", { what: `the mandate: ${e.message}` }));
      // One space at a time, each once the node is idle (BACKGROUND: a page's reads first).
      const node = await ctx.require("node");
      const servers = (await space.mine()).filter(s => s.kind === "server");
      // TELEMETRY: this page's state in each space (the HEARTBEAT, hourly), and what it sees of the other members.
      const seen = [];
      // The space open first.
      for (const sp of [...servers.filter(s => s.id === ctx.space), ...servers.filter(s => s.id !== ctx.space)]) {
        await node.idle();
        // REMOVED from it (its group says so): no longer in this person's list — never left shown as theirs.
        if (await (await ctx.require("keys")).group(sp).removed().catch(() => false)) {
          (await ctx.require("lifecycle")).become("space", sp.id.slice(0, 12), "removed", {});
          await space.forget(sp).catch(e => ctx.log("upkeep", { what: `${sp.name}: ${e.message}` }));
          continue;
        }
        // BEHIND on its keys (a welcome onto an older branch): the owner and admins asked to welcome this node onto
        // theirs, once each load — before anything that needs the acts this node cannot read yet.
        // (Made SAFE 10-06: asked only after 10 minutes behind, and answered only when the asker is confirmed behind.)
        await conversation.catchUp(sp).catch(e => ctx.log("upkeep", { what: `${sp.name}: catching up: ${e.message}` }));
        // What this page cannot read here, reported into the space (seen by any member on Members & roles).
        await conversation.reportReads(sp).catch(e => ctx.log("upkeep", { what: `${sp.name}: reporting reads: ${e.message}` }));
        const self = await conversation.selfState(sp).catch(e => ({ s: sp.id.slice(0, 6), why: String(e?.message ?? e).slice(0, 80) }));
        seen.push(self);
        // ITS MEMBERSHIP HERE, as a lifecycle state (FOUNDATION.md §3: `space`).
        const life = await ctx.require("lifecycle");
        const ev = self.why ? "not-held" : self.e == null ? "not-held" : self.oe != null && self.e < self.oe ? "behind" : self.on === false ? "off-branch" : self.u ? "unreadable" : "held";
        life.become("space", sp.id.slice(0, 12), ev, { e: self.e, oe: self.oe, u: self.u, ...(self.why ? { why: self.why } : {}) });
        await conversation.observe(sp).catch(e => ctx.log("upkeep", { what: `${sp.name}: observing members: ${e.message}` }));
        const r = await roles.of(sp).catch(() => null);
        // AN OPEN SPACE's roster public (the owner, once a page): askers outside learn they were let in.
        if (r && !published.has(sp.id) && r.role(me.id) === "owner" && r.policy("", "join") === "anyone") {
          published.add(sp.id);
          await r.publish().catch(e => ctx.log("upkeep", { what: `${sp.name}: publishing its roster: ${e.message}` }));
        }
        // The acts hold in the group: whoever is banned, removed or has left and is still in it, out — by ANY member
        // (the acts already decided it; the group's one order refuses a second commit of the same removal).
        if (r && !r.left && r.role(me.id)) {
          const n = await (await ctx.require("moderation")).of(sp).then(m => m.enforce()).catch(e => (ctx.log("upkeep", { what: `${sp.name}: bans: ${e.message}` }), 0));
          if (n) ctx.log("upkeep", { what: `${sp.name}: ${n} banned device(s) taken out of the group` });
          // A LEVEL GROUP: whoever's card no longer reaches its level, taken out (in turn).
          await conversation.lapse(sp).catch(e => ctx.log("upkeep", { what: `${sp.name}: lapses: ${e.message}` }));
          // A FORK's heal: members on another branch of the group's keys welcomed back onto the owner's (by any member
          // on it — nothing new is let in: only who is a member already).
          const fixed = await conversation.repair(sp).catch(e => (ctx.log("upkeep", { what: `${sp.name}: repair: ${e.message}` }), []));
          if (fixed.length) ctx.log("upkeep", { what: `${sp.name}: ${fixed.length} member(s) on another branch welcomed back` });
          // AUDIENCES (a restricted channel's, a post's: a role, admins, chosen people): in step with who passes now — by
          // their maker, or an admin in them while the maker is away.
          await conversation.keepReaders(sp).catch(e => ctx.log("upkeep", { what: `${sp.name}: readers: ${e.message}` }));
        }
        if (!r?.can(me.id, "invite")) continue;
        // A member's ask from behind (the space's STUCK bag): answered here while this page is open — its node stands
        // aside meanwhile.
        const again = await conversation.answerStuck(sp).catch(e => (ctx.log("upkeep", { what: `${sp.name}: stuck asks: ${e.message}` }), []));
        if (again.length) ctx.log("upkeep", { what: `${sp.name}: ${again.length} member(s) welcomed again from behind` });
        // In by the acts, not in the group: added (in turn).
        const added = await conversation.reconcile(sp).catch(e => (ctx.log("upkeep", { what: `${sp.name}: reconcile: ${e.message}` }), []));
        if (added.length) ctx.log("upkeep", { what: `${added.length} added to ${sp.name} as its acts say` });
        const let_in = await conversation.admit(sp).catch(e => (ctx.log("upkeep", { what: `${sp.name}: ${e.message}` }), []));
        if (let_in.length) ctx.log("upkeep", { what: `${let_in.length} let into ${sp.name}` });
      }
      // THE HEARTBEAT: this page open, its version, and its state in each space (a member who stops beating, or whose
      // state turns, is seen in the day's health bag: `tools/health-watch`).
      await index.health({ k: "seen", sp: seen.slice(0, 12), ...(ctx.appsOpened?.size ? { apps: [...ctx.appsOpened].slice(0, 24) } : {}) }).catch(() => {});
      ctx.appsOpened?.clear();
      // AUDIENCES: each circle's members as they should be (friends; followers, from their notices).
      await (await ctx.require("circles")).sync().catch(e => ctx.log("upkeep", { what: `circles: ${e?.message ?? (typeof e === "object" ? JSON.stringify(e) : String(e))}${e?.stack ? ` (${String(e.stack).split("\n")[1]?.trim() ?? ""})` : ""}` }));
      // Boards: what this person wrote is public exactly while its board reads in public (a board made public shows
      // what was written before; one made private takes it back).
      await (await ctx.require("items")).syncPublic().catch(e => ctx.log("upkeep", { what: `boards: ${e.message}` }));
    } finally {
      running = false;
    }
  }
  // WHAT THE DELEGATE DID with no page open: each group it moved, loaded; each person it let in, an `admitted` act (by
  // this person, who may invite: at the time it happened); then forgotten there.
  async function absorb(me) {
    const st = (await auth.identity.upkeepStatus())?.upkeep;
    if (!st) return;
    const mine = await space.mine();
    const of = id => mine.find(s => s.id === id);
    for (const g of st.groups ?? []) if (of(g.space)) await keys.group(of(g.space)).adopt(g.epoch, g.state);
    const written = [];
    // The key packages it used: recorded as spent (a key package works once), before the admissions are acknowledged.
    const used = await storage().catch(() => null);
    for (const a of st.admitted ?? []) if (used && a.kp) await used.put(a.kp, String(a.at || Date.now())).catch(() => {});
    for (const a of st.admitted ?? []) {
      const sp = of(a.space);
      if (!sp) continue;
      written.push([a.space, a.did]);
      // ITS KEY PACKAGE named in the space as used (`kp`, the tag the node's record carries: `conversation.sharedTags`):
      // no member's welcome — page or node — uses it again (10-11: two welcomes for one package, one never opened).
      // (Only once a page of this account absorbs it: until then the members cannot see the node used it.)
      const kp = typeof a.kp === "string" && /^[0-9a-f]{32}$/.test(a.kp) ? { kp: a.kp } : {};
      // Let in AGAIN (a member stuck behind its own commit): a member already — no act.
      if (a.code === "stuck") {
        // Recorded in the space as any member's welcome again (`conversation`'s once-per-epoch rule reads it).
        await index.spacePoint(sp, "writers", { w: sp.self, repair: a.did, epoch: Number(a.epoch), ...kp }).catch(() => {});
        ctx.log("upkeep", { what: `the delegate let ${a.did.slice(12, 20)}… into ${sp.name} again (stuck behind its own change) while no page ran` });
        continue;
      }
      if (kp.kp) await index.spacePoint(sp, "writers", { w: sp.self, welcomed: a.did, ...kp, at: a.at || Date.now() }).catch(() => {});
      const r = await roles.of(sp);
      await r.refresh();
      await r.act({ act: "admitted", code: a.code, did: a.did, at: a.at });
      ctx.log("upkeep", { what: `the delegate let ${a.did.slice(12, 20)}… into ${sp.name} while no page ran` });
    }
    if (written.length) await auth.identity.upkeepAck(written);
  }
  // Out of a level group by a lapse alone (no removal, ban or leave since): may be let in again.
  const lapsedOnly = (r, did) => {
    const outs = r.acts().filter(a => (a.did ?? a.by) === did && ["lapsed", "remove", "ban", "leave"].includes(a.act));
    return outs.length > 0 && outs.at(-1).act === "lapsed";
  };
  const stuckBags = new Set();
  let lastMandate = null;
  // THE MANDATE: every shared space this person is in, as this page knows it now. Admitting anyone NEW (codes, the
  // open door, a level) only where they may invite; letting a STUCK member in again (no one new) wherever they are a
  // member — its members by the acts, or, where the acts do not read here, by the group's own.
  async function mandate(me) {
    const spaces = [];
    const { glue } = await ctx.require("node");
    for (const sp of (await space.mine()).filter(s => s.kind === "server")) {
      const r = await roles.of(sp).catch(() => null);
      const g = await keys.group(sp).snapshot().catch(() => null);
      if (!g) continue;
      const mayInvite = !!r?.can(me.id, "invite");
      const inGroup = ((await keys.group(sp).ready().catch(() => null))?.members ?? []).filter(m => m.cred).map(m => glue.did_of(new Uint8Array(m.cred.match(/../g).slice(4, 36).map(x => parseInt(x, 16)))));
      const members = [...new Set([...(r?.members() ?? []).map(m => m.did), ...inGroup])];
      // The key packages its members (and their nodes) used, mirrored into this account's `keypacks` (`shared:<tag>`):
      // handed below as spent with this account's own — a package is spent for every space once any member used it.
      await conversation.mirrorShared(sp).catch(e => ctx.log("upkeep", { what: `${sp.name}: key packages used there: ${e?.message ?? e}` }));
      // Its STUCK bag made (once a page): the delegate reads it every round, and never asks for one not made.
      if (!stuckBags.has(sp.id)) await index.openRequests(`stuck ${sp.id}`).then(() => stuckBags.add(sp.id), () => {});
      spaces.push({
        space: sp.id, name: sp.name, kind: sp.kind, owner: sp.governance.owner, nonce: sp.governance.nonce ?? null, channel: sp.tables.channel,
        open: mayInvite && r.policy("", "join") === "anyone",
        // A level group's level: the delegate lets in askers at `level <id>` whose card reaches it.
        level: mayInvite ? (/^level:(pro|vip)$/.exec(r.policy("", "join") ?? "")?.[1] ?? null) : null,
        codes: mayInvite ? r.invites().map(i => [i.code, i.expires || 0, i.uses ? i.uses - i.admitted.length : 0]) : [],
        // Never let in again: the banned AND whoever the acts put out (removed, left) — an old request of theirs still
        // sits in a bag upkeep reads.
        // (One out only by a LAPSE may come back: they ask again when they give again.)
        bans: r ? [...new Set([...r.bannedList(), ...r.goneList().filter(d => !lapsedOnly(r, d))])] : [], members, epoch: g.epoch, state: g.state,
      });
    }
    // With it, the key packages this account used already — and those another member used (`shared:<tag>`): upkeep
    // never uses one again.
    const used = await storage().catch(() => null);
    const spent = used ? [...new Set(used.rows().filter(r => r.value).map(r => r.key.replace(/^shared:/, "")).filter(k => /^[0-9a-f]{32}$/.test(k)))] : [];
    const r = await auth.identity.upkeepMandate(me.id, spaces, spent);
    const said = spaces.map(x => `${x.name}${x.codes.length || x.open || x.level ? "" : " (re-add only)"}`).join(", ");
    if (said !== lastMandate) (lastMandate = said), ctx.log("upkeep", { what: `the node upkeeps ${spaces.length} space(s): ${said}` });
    if (r.upkeep?.stale?.length) ctx.log("upkeep", { what: `${r.upkeep.stale.length} space(s): the delegate's group is newer (loaded next tick)` });
  }
  // THE DELEGATE's upkeep (no page open: the node wakes it — node ≥ 0.2.139, with the Background grant): told which
  // contract the account's inbox is, once per login.
  async function handOver() {
    const me = await space.account();
    if (!me) return;
    const [{ glue }, bagCode, tailCode, idlogCode, sealedCode, pieceCode, blockCode, devicesCode] = await Promise.all(["node", "bag-wasm", "tail-wasm", "idlog-wasm", "sealed-wasm", "piece-wasm", "block-wasm", "devices-wasm"].map(n => ctx.require(n)));
    const Core = glue.CraftworksCore;
    const id = Core.bag_id(bagCode, Core.inbox_address(me.idBytes));
    const bytes = new Uint8Array(id.match(/../g).map(x => parseInt(x, 16)));
    const r = await auth.identity.upkeepWatch(bytes).catch(e => ({ refused: e.message }));
    // The contracts it writes (their code) and reads: handed over when they are not the ones it holds — the Sealed,
    // Piece and Block contracts too (it re-keys a space's files with no page open).
    if (r.upkeep && r.upkeep.codes !== Core.upkeep_codes_hash(bagCode, tailCode, idlogCode, sealedCode, pieceCode, blockCode, devicesCode))
      await auth.identity.upkeepCodes(bagCode, tailCode, idlogCode, sealedCode, pieceCode, blockCode, devicesCode).catch(() => {});
    ctx.log("upkeep", { what: r.upkeep ? `the delegate watches the inbox (${r.upkeep.wakeups} wake-up(s) so far)` : `the delegate: ${r.refused ?? JSON.stringify(r)}` });
    // What it did with no page open (admissions, re-keys): its last lines, each said once a page opens.
    for (const line of String(r.upkeep?.said ?? "").split("\n").filter(Boolean)) ctx.log("upkeep said", { what: line });
  }
  handOver().catch(() => {});
  addEventListener("craftworks:auth", () => handOver().catch(() => {}));
  // A JOB (FOUNDATION.md §3): every 30 s, only while this device may write for the account (its session open, its
  // account group a member) — paused, and said why, otherwise.
  const life = await ctx.require("lifecycle");
  const job = life.job("upkeep", { every: 30000, needs: () => keys.writable(), run: tick });
  addEventListener("craftworks:auth", () => job.now());
  return { tick };
}
