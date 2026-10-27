// UPKEEP, a service (started by the header, so on every page, after the page is up): what keeps this person's spaces
// current without any one page open — every 30 s (and at once): spaces this person was TOLD they are in, entered (they
// appear in the spaces panel), and each space's state here watched.
//
// MEMBERS AND KEYS (FOUNDATION "Members and keys"): a space's BeeKEM tree follows its list — whoever reconciles places a
// listed account and takes out whom the list no longer names (`members`; the account's node too, with every page closed:
// asked to run now when that changes). The list says who is in.
export async function start(ctx) {
  const [space, conversation, roles, keys, auth, index] = await Promise.all(["space", "conversation", "roles", "keys", "auth", "index"].map(n => ctx.require(n)));
  // (Its jobs started with the page's upkeep: devices dropped from this account recorded in its spaces, `retire`.)
  ctx.background("upkeep: moderation", ctx.require("moderation"));
  let running = false;
  let renewed = false; // the card made current this page (once)
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
      await absorb().catch(e => ctx.failed("upkeep", e, `what the delegate did: ${e.message}`));
      // THE CARD as it should be (once a page): its inbox and devices.
      if (!renewed) {
        renewed = true;
        await (await ctx.require("directory")).publish().catch(e => ((renewed = false), ctx.failed("upkeep", e, `the card: ${e.message}`)));
      }
      const joined = await conversation.accept().catch(e => (ctx.failed("upkeep", e, `the inbox: ${e.message}`), []));
      if (joined.length) ctx.log("upkeep", { what: `entered ${joined.length} space(s) this person was told of` });
      // BLOG: this person's scheduled drafts published when due, and approved ones (`blog-press`) — by any page open.
      await (await ctx.require("blog-press")).publishDue().catch(e => ctx.failed("upkeep", e, `blog drafts: ${e.message}`));
      // THE NODE's list first (before each space's slower work): what it keeps with no page open.
      await mandate(me).catch(e => ctx.failed("upkeep", e, `the mandate: ${e.message}`));
      // One space at a time, each once the node is idle (BACKGROUND: a page's reads first).
      const node = await ctx.require("node");
      const servers = await space.mine.strict();
      let follow = false;
      // TELEMETRY: this page's state in each space (the HEARTBEAT, hourly), and what it sees of the other members.
      const seen = [];
      // The space open first.
      for (const sp of [...servers.filter(s => s.id === ctx.space), ...servers.filter(s => s.id !== ctx.space)]) {
        await node.idle();
        // REMOVED from it (the list says so — K8.2), or NOT MIGRATED — the list never moved this account in or out
        // (`unlisted`), or the space has no BeeKEM tree at all (`members.hasTree`: an MLS-era space nobody moved, whose
        // sealed acts this account cannot even read): no longer in this person's list — never left shown as theirs.
        // (WHY it is taken out, recorded with the move — which rule said so.)
        const gone = async () => {
          const r = await roles.of(sp);
          await r?.settled;
          if (r?.left) return "left";
          if (r?.unlisted) return `unlisted (list of ${r.roster().length}, whole)`;
          return (await (await ctx.require("members")).group(sp).hasTree()) === false ? "no tree (its owner's key operations read empty)" : null;
        };
        const why = await ctx.optional("upkeep: removed", gone(), () => null);
        if (why) {
          (await ctx.require("lifecycle")).become("group", sp.id, "removed", { why });
          await space.forget(sp).catch(e => ctx.failed("upkeep", e, `${sp.name}: ${e.message}`));
          continue;
        }
        // What this page cannot read here, reported into the space (seen by any member on Members & roles).
        await conversation.reportReads(sp).catch(e => ctx.failed("upkeep", e, `${sp.name}: reporting reads: ${e.message}`));
        const self = await conversation.selfState(sp).catch(e => ({ s: sp.id.slice(0, 6), why: String(e?.message ?? e).slice(0, 80) }));
        seen.push(self);
        // ITS GROUP HERE, as a lifecycle state (FOUNDATION.md §3: `group`): HELD when its tree places this account and
        // nothing here is unreadable, UNREADABLE when something is, NOT_HELD when the tree does not place it (K8.6: the
        // tree's word — no MLS epoch, branch or self-entry).
        const life = await ctx.require("lifecycle");
        if (!self.why) life.become("group", sp.id, !self.placed ? "not-held" : self.u?.length ? "unreadable" : "held", { e: self.e, u: self.u });
        await conversation.observe(sp).catch(e => ctx.failed("upkeep", e, `${sp.name}: observing members: ${e.message}`));
        const r = await roles.of(sp);
        // AN OPEN SPACE's roster public (the owner, once a page): who is in, seen from outside.
        if (r && !published.has(sp.id) && r.role(me.id) === "owner" && ["anyone", "ask"].includes(r.policy("", "join"))) {
          published.add(sp.id);
          await r.publish().catch(e => ctx.failed("upkeep", e, `${sp.name}: publishing its roster: ${e.message}`));
        }
        // Whoever the acts put out (banned, removed, left) still in the group: where their feeds stand recorded — their
        // entries are the node's follower's to take out.
        if (r && !r.left && r.role(me.id)) {
          // A space made before its members were listed: they put on its list (by one who may list).
          await conversation.relist(sp).catch(e => ctx.failed("upkeep", e, `${sp.name}: its list carried over: ${e.message}`));
          const n = await (await ctx.require("moderation")).of(sp).then(m => m.enforce()).catch(e => (ctx.failed("upkeep", e, `${sp.name}: those out: ${e.message}`), 0));
          // (The node asked when that count CHANGES — never every tick while it waits its turn.)
          if (n !== (lastOut.get(sp.id) ?? 0) && n > 0) (follow = true), ctx.log("upkeep", { what: `${sp.name}: ${n} put out by the acts still in its group — the node takes them out` });
          lastOut.set(sp.id, n);
          // A LEVEL GROUP: whoever's card no longer reaches its level, taken out (in turn).
          await conversation.lapse(sp).catch(e => ctx.failed("upkeep", e, `${sp.name}: lapses: ${e.message}`));
          // AUDIENCES (a restricted channel's, a post's: a role, admins, chosen people): in step with who passes now — by
          // their maker, or an admin in them while the maker is away.
          await conversation.keepReaders(sp).catch(e => ctx.failed("upkeep", e, `${sp.name}: readers: ${e.message}`));
        }
      }
      // …or its follower's NEXT TURN has come (it said when: a takeover of a member away, with no wake-ups to bring it).
      const next = (await ctx.optional("upkeep: upkeepStatus", auth.identity.upkeepStatus(), () => null))?.upkeep?.next;
      if (next && Date.now() >= next) follow = true;
      if (follow) await auth.identity.followNow().catch(e => ctx.failed("upkeep", e, `asking the node to follow now: ${e.message}`));
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
  // WHAT THE NODE DID: each group it moved (its removals), loaded; its decisions, moves of the lifecycle.
  async function absorb() {
    const st = (await auth.identity.upkeepStatus())?.upkeep;
    if (!st) return;
    const mine = await space.mine.strict();
    const of = id => mine.find(s => s.id === id);
    // ITS DECISIONS: moves of the `group` machine (by the node), reported like a page's — ONCE each: those past the newest
    // of the node's this journal holds, by the node's SEQUENCE (I5: never its clock).
    const life = await ctx.require("lifecycle");
    const since = Math.max(0, ...(await life.journal()).filter(e => e.by === "node").map(e => Number(e.seq) || 0));
    const fresh = (st.decided ?? []).filter(d => (Number(d.seq) || 0) > since);
    // LOST to the node's limit (its newest 128 kept while no page reads them): how many, REPORTED — a gap in what the
    // node did is said, never silently missing.
    const first = Math.min(...fresh.map(d => Number(d.seq) || 0));
    if (since > 0 && fresh.length && first > since + 1) life.transition("group", "account", "node-dropped", { by: "node", why: `${first - since - 1} of the node's steps dropped before a page read them`, n: first - since - 1 });
    // Each by ITS OWN NAME (`rules::steps`, one list); a name the list does not know (an older build's) as `node-step` —
    // one step never stops the rest.
    for (const d of fresh) {
      const known = !!life.MACHINES.group.events[d.step] && !life.MACHINES.group.events[d.step].to;
      const detail = { by: "node", why: known ? d.why : `${d.step}: ${d.why}`, at: d.at, seq: d.seq, ...(d.did ? { to: d.did.replace(/^did:craftec:/, "").slice(0, 6) } : {}) };
      try {
        // (A step about no space — the node's keeper, witness, re-key turns: `steps::ACCOUNT` — is the account's.)
        life.transition("group", /^0+$/.test(String(d.space)) ? "account" : d.space, known ? d.step : "node-step", detail);
      } catch (e) {
        ctx.log("upkeep", { what: `the node's step ${d.step} not recorded: ${e?.message ?? e}` });
      }
    }
  }
  const lastOut = new Map(); // space → how many the acts put out still held an entry last tick (the same)
  // THE MANDATE: every shared space this person is in, NAMED (its id, name, owner, channel) — never its member list (the
  // node reads that from the acts), never an MLS group (K8.6: epoch 0, no state — the fields stay in the wire).
  async function mandate(me) {
    const spaces = [];
    // EVERY KIND (a server, a conversation, a level group, an audience, a circle): one follower for all.
    for (const sp of await space.mine.strict()) {
      spaces.push({ space: sp.id, name: sp.name, kind: sp.kind, owner: sp.governance.owner, nonce: sp.governance.nonce ?? null, channel: sp.tables.channel, epoch: 0, state: "" });
    }
    const r = await auth.identity.upkeepMandate(me.id, spaces);
    const said = spaces.map(x => x.name).join(", ");
    (await ctx.require("lifecycle")).logOnce("upkeep", "mandate", `the node upkeeps ${spaces.length} space(s): ${said}`);
  }
  // THE DELEGATE's upkeep (no page open: the node wakes it — node ≥ 0.2.139, with the Background grant): told which
  // contract the account's inbox is, once per login.
  async function handOver() {
    const me = await space.account.strict();
    if (!me) return;
    const [{ glue }, bagCode, tailCode, idlogCode, sealedCode, pieceCode, blockCode, devicesCode] = await Promise.all(["node", "bag-wasm", "tail-wasm", "idlog-wasm", "sealed-wasm", "piece-wasm", "block-wasm", "devices-wasm"].map(n => ctx.require(n)));
    const Core = glue.CraftworksCore;
    const id = Core.bag_id(bagCode, Core.inbox_address(me.idBytes));
    const bytes = (await ctx.require("node")).bytes(id);
    const r = await auth.identity.upkeepWatch(bytes).catch(e => ({ refused: e.message }));
    // The contracts it writes (their code) and reads: handed over when they are not the ones it holds — the Sealed,
    // Piece and Block contracts too (it re-keys a space's files with no page open).
    if (r.upkeep && r.upkeep.codes !== Core.upkeep_codes_hash(bagCode, tailCode, idlogCode, sealedCode, pieceCode, blockCode, devicesCode))
      await ctx.background("upkeep: auth.identity.upkeepCodes", auth.identity.upkeepCodes(bagCode, tailCode, idlogCode, sealedCode, pieceCode, blockCode, devicesCode));
    ctx.log("upkeep", { what: r.upkeep ? `the delegate watches the inbox (${r.upkeep.wakeups} wake-up(s) so far)` : `the delegate: ${r.refused ?? JSON.stringify(r)}` });
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
