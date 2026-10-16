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
  // (Its jobs started with the page's upkeep: devices dropped from this account recorded in its spaces, `retire`.)
  ctx.background("upkeep: moderation", ctx.require("moderation"));
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
    // (Called directly too — a Chat or a Space Home asks — so its own rule here: only while this device may write for
    // the account; audit 10-13.)
    if ((await ctx.optional("upkeep: keys.writable", keys.writable(), () => "unknown")) !== true) return;
    running = true;
    try {
      const me = await space.account.strict();
      if (!me) return;
      await absorb(me).catch(e => ctx.failed("upkeep", e, `what the delegate did: ${e.message}`));
      // THE CARD as it should be (once a page): its inbox and devices — and each request still waiting kept answerable
      // (`askStuck`: made anew with this device's own key where it carries none, or was let in and never opened).
      if (!renewed) {
        renewed = true;
        await (await ctx.require("directory")).publish().catch(e => ((renewed = false), ctx.failed("upkeep", e, `the card: ${e.message}`)));
        await conversation.askStuck().catch(e => ctx.failed("upkeep", e, `requests let in, never in: ${e.message}`));
      }
      // WELCOMES KEPT UNSENT (their commit made, the send failed): sent again, on their own wait (`conversation`).
      await conversation.resendWelcomes().catch(e => ctx.failed("upkeep", e, `welcomes kept to send again: ${e.message}`));
      const joined = await conversation.accept().catch(e => (ctx.failed("upkeep", e, `the inbox: ${e.message}`), []));
      if (joined.length) ctx.log("upkeep", { what: `joined ${joined.length} space(s) from the inbox` });
      // BLOG: this person's scheduled drafts published when due, and approved ones (`blog-press`) — by any page open.
      await (await ctx.require("blog-press")).publishDue().catch(e => ctx.failed("upkeep", e, `blog drafts: ${e.message}`));
      // THE NODE's list first (before each space's slower work): what it keeps with no page open.
      await mandate(me).catch(e => ctx.failed("upkeep", e, `the mandate: ${e.message}`));
      // One space at a time, each once the node is idle (BACKGROUND: a page's reads first).
      const node = await ctx.require("node");
      const servers = (await space.mine.strict()).filter(s => s.kind === "server");
      // TELEMETRY: this page's state in each space (the HEARTBEAT, hourly), and what it sees of the other members.
      const seen = [];
      // The space open first.
      for (const sp of [...servers.filter(s => s.id === ctx.space), ...servers.filter(s => s.id !== ctx.space)]) {
        await node.idle();
        // REMOVED from it (its group says so): no longer in this person's list — never left shown as theirs.
        if (await ctx.optional("upkeep: group", (await ctx.require("keys")).group(sp).removed(), () => false)) {
          (await ctx.require("lifecycle")).become("group", sp.id, "removed", {});
          await space.forget(sp).catch(e => ctx.failed("upkeep", e, `${sp.name}: ${e.message}`));
          continue;
        }
        // BEHIND on its keys (a welcome onto an older branch): the owner and admins asked to welcome this node onto
        // theirs, once each load — before anything that needs the acts this node cannot read yet.
        // (Made SAFE 10-06: asked only after 10 minutes behind, and answered only when the asker is confirmed behind.)
        await conversation.catchUp(sp).catch(e => ctx.failed("upkeep", e, `${sp.name}: catching up: ${e.message}`));
        // What this page cannot read here, reported into the space (seen by any member on Members & roles).
        await conversation.reportReads(sp).catch(e => ctx.failed("upkeep", e, `${sp.name}: reporting reads: ${e.message}`));
        const self = await conversation.selfState(sp).catch(e => ({ s: sp.id.slice(0, 6), why: String(e?.message ?? e).slice(0, 80) }));
        seen.push(self);
        // ITS GROUP HERE, as a lifecycle state (FOUNDATION.md §3: `group`).
        const life = await ctx.require("lifecycle");
        const ev = self.why ? "not-held" : self.e == null ? "not-held" : self.oe != null && self.e < self.oe ? "behind" : self.on === false ? "off-branch" : self.u ? "unreadable" : "held";
        life.become("group", sp.id, ev, { e: self.e, oe: self.oe, u: self.u, ...(self.why ? { why: self.why } : {}) });
        await conversation.observe(sp).catch(e => ctx.failed("upkeep", e, `${sp.name}: observing members: ${e.message}`));
        const r = await roles.of(sp);
        // AN OPEN SPACE's roster public (the owner, once a page): askers outside learn they were let in.
        if (r && !published.has(sp.id) && r.role(me.id) === "owner" && r.policy("", "join") === "anyone") {
          published.add(sp.id);
          await r.publish().catch(e => ctx.failed("upkeep", e, `${sp.name}: publishing its roster: ${e.message}`));
        }
        // The acts hold in the group: whoever is banned, removed or has left and is still in it, out — by ANY member
        // (the acts already decided it; the group's one order refuses a second commit of the same removal).
        if (r && !r.left && r.role(me.id)) {
          const n = await (await ctx.require("moderation")).of(sp).then(m => m.enforce()).catch(e => (ctx.failed("upkeep", e, `${sp.name}: bans: ${e.message}`), 0));
          if (n) ctx.log("upkeep", { what: `${sp.name}: ${n} banned device(s) taken out of the group` });
          // A LEVEL GROUP: whoever's card no longer reaches its level, taken out (in turn).
          await conversation.lapse(sp).catch(e => ctx.failed("upkeep", e, `${sp.name}: lapses: ${e.message}`));
          // AUDIENCES (a restricted channel's, a post's: a role, admins, chosen people): in step with who passes now — by
          // their maker, or an admin in them while the maker is away.
          await conversation.keepReaders(sp).catch(e => ctx.failed("upkeep", e, `${sp.name}: readers: ${e.message}`));
        }
        if (!r?.can(me.id, "invite")) continue;
        // LETTING PEOPLE IN (`conversation.admit`: every request, one rule — its node stands aside while this page is open).
        const let_in = await conversation.admit(sp).catch(e => (ctx.failed("upkeep", e, `${sp.name}: ${e.message}`), []));
        if (let_in.length) ctx.log("upkeep", { what: `${let_in.length} let into ${sp.name}` });
      }
      // …and into this person's CONVERSATIONS (direct, group: their people asking in, by the same rule).
      for (const sp of (await space.mine.strict()).filter(s => s.kind === "direct" || s.kind === "group")) {
        const let_in = await conversation.admit(sp).catch(e => (ctx.failed("upkeep", e, `${sp.name}: ${e.message}`), []));
        if (let_in.length) ctx.log("upkeep", { what: `${let_in.length} let into ${sp.name ?? "a conversation"}` });
      }
      // THE HEARTBEAT: this page open, its version, and its state in each space (a member who stops beating, or whose
      // state turns, is seen in the day's health bag: `tools/health-watch`).
      await ctx.background("upkeep: index.health", index.health({ k: "seen", sp: seen.slice(0, 12), ...(ctx.appsOpened?.size ? { apps: [...ctx.appsOpened].slice(0, 24) } : {}) }));
      ctx.appsOpened?.clear();
      // AUDIENCES: each circle's members as they should be (friends; followers, from their notices).
      await (await ctx.require("circles")).sync().catch(e => ctx.log("upkeep", { what: `circles: ${e?.message ?? (typeof e === "object" ? JSON.stringify(e) : String(e))}${e?.stack ? ` (${String(e.stack).split("\n")[1]?.trim() ?? ""})` : ""}` }));
      // Boards: what this person wrote is public exactly while its board reads in public (a board made public shows
      // what was written before; one made private takes it back).
      await (await ctx.require("items")).syncPublic().catch(e => ctx.failed("upkeep", e, `boards: ${e.message}`));
    } finally {
      running = false;
    }
  }
  // WHAT THE DELEGATE DID with no page open: each group it moved, loaded; each person it let in, an `admitted` act (by
  // this person, who may invite: at the time it happened); then forgotten there.
  async function absorb(me) {
    const st = (await auth.identity.upkeepStatus())?.upkeep;
    if (!st) return;
    const mine = await space.mine.strict();
    const of = id => mine.find(s => s.id === id);
    for (const g of st.groups ?? []) if (of(g.space)) await keys.group(of(g.space)).adopt(g.epoch, g.state);
    const written = [];
    // The key packages it used: recorded as spent (a key package works once), before the admissions are acknowledged.
    const used = await ctx.optional("upkeep: storage", storage(), () => null);
    for (const a of st.admitted ?? []) if (used && a.kp) await ctx.background("upkeep: used.put", used.put(a.kp, String(a.at || Date.now())));
    for (const a of st.admitted ?? []) {
      const sp = of(a.space);
      if (!sp) continue;
      written.push([a.space, a.did]);
      // (Its welcome is in the space's writers list already: the node wrote it there when it sent it.)
      // Let in AGAIN (a member asking in again): a member already — no act.
      if (a.code === "stuck") continue;
      const r = await roles.of.strict(sp);
      await r.refresh();
      await r.act({ act: "admitted", code: a.code, did: a.did, at: a.at });
      ctx.log("upkeep", { what: `the delegate let ${a.did.slice(12, 20)}… into ${sp.name} while no page ran` });
    }
    if (written.length) await auth.identity.upkeepAck(written);
    // ITS DECISIONS on each request: moves of the `member` lifecycle (by the node), reported like a page's — once each.
    const life = await ctx.require("lifecycle");
    for (const d of st.decided ?? []) {
      const k = `${d.q}|${d.step}|${d.why}`;
      if (nodeDecided.has(k)) continue;
      nodeDecided.add(k);
      life.transition("member", conversation.reqKey(d.q, d.did), d.step, { by: "node", s: d.space.slice(0, 6), to: d.did.replace(/^did:craftec:/, "").slice(0, 6), why: d.why, at: d.at });
    }
  }
  const nodeDecided = new Set();
  // Out of a level group by a lapse alone (no removal, ban or leave since): may be let in again.
  const lapsedOnly = (r, did) => {
    const outs = r.acts().filter(a => (a.did ?? a.by) === did && ["lapsed", "remove", "ban", "leave"].includes(a.act));
    return outs.length > 0 && outs.at(-1).act === "lapsed";
  };
  let lastMandate = null;
  // THE MANDATE: every shared space this person is in, as this page knows it now. Admitting anyone NEW (codes, the
  // open door, a level) only where they may invite; letting a STUCK member in again (no one new) wherever they are a
  // member — its members by the acts, or, where the acts do not read here, by the group's own.
  async function mandate(me) {
    const spaces = [];
    const { glue } = await ctx.require("node");
    for (const sp of (await space.mine.strict()).filter(s => s.kind === "server")) {
      const r = await roles.of(sp);
      const g = await ctx.optional("upkeep: keys.group", keys.group(sp).snapshot(), () => null);
      if (!g) continue;
      const mayInvite = !!r?.can(me.id, "invite");
      const inGroup = ((await keys.group(sp).ready())?.members ?? []).filter(m => m.cred).map(m => glue.did_of(new Uint8Array(m.cred.match(/../g).slice(4, 36).map(x => parseInt(x, 16)))));
      const members = [...new Set([...(r?.members() ?? []).map(m => m.did), ...inGroup])];
      // The key packages its members (and their nodes) used, mirrored into this account's `keypacks` (`shared:<tag>`):
      // handed below as spent with this account's own — a package is spent for every space once any member used it.
      await conversation.mirrorShared(sp).catch(e => ctx.failed("upkeep", e, `${sp.name}: key packages used there: ${e?.message ?? e}`));
      // Its MEMBER bag made (once a page): the delegate reads it every round, and never asks for one not made.
      await ctx.background("upkeep: conversation.openMemberBag", conversation.openMemberBag(sp));
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
        // Who may let in (`rules::admission` there as here; the welcomes made it reads from the space's writers list).
        admitters: r ? r.members().filter(m => r.can(m.did, "invite")).map(m => m.did) : [],
      });
    }
    // With it, the key packages this account used already — and those another member used (`shared:<tag>`): upkeep
    // never uses one again.
    const used = await ctx.optional("upkeep: storage", storage(), () => null);
    const spent = used ? [...new Set(used.rows().filter(r => r.value).map(r => r.key.replace(/^shared:/, "")).filter(k => /^[0-9a-f]{32}$/.test(k)))] : [];
    const r = await auth.identity.upkeepMandate(me.id, spaces, spent);
    const said = spaces.map(x => `${x.name}${x.codes.length || x.open || x.level ? "" : " (re-add only)"}`).join(", ");
    if (said !== lastMandate) (lastMandate = said), ctx.log("upkeep", { what: `the node upkeeps ${spaces.length} space(s): ${said}` });
    if (r.upkeep?.stale?.length) ctx.log("upkeep", { what: `${r.upkeep.stale.length} space(s): the delegate's group is newer (loaded next tick)` });
  }
  // THE DELEGATE's upkeep (no page open: the node wakes it — node ≥ 0.2.139, with the Background grant): told which
  // contract the account's inbox is, once per login.
  async function handOver() {
    const me = await space.account.strict();
    if (!me) return;
    const [{ glue }, bagCode, tailCode, idlogCode, sealedCode, pieceCode, blockCode, devicesCode] = await Promise.all(["node", "bag-wasm", "tail-wasm", "idlog-wasm", "sealed-wasm", "piece-wasm", "block-wasm", "devices-wasm"].map(n => ctx.require(n)));
    const Core = glue.CraftworksCore;
    const id = Core.bag_id(bagCode, Core.inbox_address(me.idBytes));
    const bytes = new Uint8Array(id.match(/../g).map(x => parseInt(x, 16)));
    const r = await auth.identity.upkeepWatch(bytes).catch(e => ({ refused: e.message }));
    // The contracts it writes (their code) and reads: handed over when they are not the ones it holds — the Sealed,
    // Piece and Block contracts too (it re-keys a space's files with no page open).
    if (r.upkeep && r.upkeep.codes !== Core.upkeep_codes_hash(bagCode, tailCode, idlogCode, sealedCode, pieceCode, blockCode, devicesCode))
      await ctx.background("upkeep: auth.identity.upkeepCodes", auth.identity.upkeepCodes(bagCode, tailCode, idlogCode, sealedCode, pieceCode, blockCode, devicesCode));
    ctx.log("upkeep", { what: r.upkeep ? `the delegate watches the inbox (${r.upkeep.wakeups} wake-up(s) so far)` : `the delegate: ${r.refused ?? JSON.stringify(r)}` });
    // What it did with no page open (admissions, re-keys): its last lines, each said once a page opens.
    for (const line of String(r.upkeep?.said ?? "").split("\n").filter(Boolean)) ctx.log("upkeep said", { what: line });
  }
  ctx.background("upkeep: handOver", handOver());
  addEventListener("craftworks:auth", () => ctx.background("upkeep: handOver", handOver()));
  // A JOB (FOUNDATION.md §3): every 30 s, only while this device may write for the account (its session open, its
  // account group a member) — paused, and said why, otherwise.
  const life = await ctx.require("lifecycle");
  const job = life.job("upkeep", { every: 30000, needs: () => keys.writable(), run: tick });
  addEventListener("craftworks:auth", () => job.now());
  return { tick };
}
