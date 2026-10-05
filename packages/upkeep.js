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
  const [space, conversation, roles, keys, auth] = await Promise.all(["space", "conversation", "roles", "keys", "auth"].map(n => ctx.require(n)));
  // The account's record of key packages used (`conversation.welcome` writes it too).
  const storage = async () => {
    const t = await (await ctx.require("storage")).table("keypacks");
    await t.settled;
    return t;
  };
  let running = false;
  let renewed = false; // key packages renewed this page (once)
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
      const joined = await conversation.accept().catch(e => (ctx.log("upkeep", { what: `the inbox: ${e.message}` }), []));
      if (joined.length) ctx.log("upkeep", { what: `joined ${joined.length} space(s) from the inbox` });
      // One space at a time, each once the node is idle (BACKGROUND: a page's reads first).
      const node = await ctx.require("node");
      const servers = (await space.mine()).filter(s => s.kind === "server");
      // The space open first.
      for (const sp of [...servers.filter(s => s.id === ctx.space), ...servers.filter(s => s.id !== ctx.space)]) {
        await node.idle();
        // REMOVED from it (its group says so): no longer in this person's list — never left shown as theirs.
        if (await (await ctx.require("keys")).group(sp).removed().catch(() => false)) {
          await space.forget(sp).catch(e => ctx.log("upkeep", { what: `${sp.name}: ${e.message}` }));
          continue;
        }
        const r = await roles.of(sp).catch(() => null);
        // The acts hold in the group: whoever is banned, removed or has left and is still in it, out — by ANY member
        // (the acts already decided it; the group's one order refuses a second commit of the same removal).
        if (r && !r.left && r.role(me.id)) {
          const n = await (await ctx.require("moderation")).of(sp).then(m => m.enforce()).catch(e => (ctx.log("upkeep", { what: `${sp.name}: bans: ${e.message}` }), 0));
          if (n) ctx.log("upkeep", { what: `${sp.name}: ${n} banned device(s) taken out of the group` });
          // A FORK's heal: members on another branch of the group's keys welcomed back onto the owner's (by any member
          // on it — nothing new is let in: only who is a member already).
          const fixed = await conversation.repair(sp).catch(e => (ctx.log("upkeep", { what: `${sp.name}: repair: ${e.message}` }), []));
          if (fixed.length) ctx.log("upkeep", { what: `${sp.name}: ${fixed.length} member(s) on another branch welcomed back` });
          // AUDIENCES (a restricted channel's, a post's: a role, admins, chosen people): in step with who passes now — by
          // their maker, or an admin in them while the maker is away.
          await conversation.keepReaders(sp).catch(e => ctx.log("upkeep", { what: `${sp.name}: readers: ${e.message}` }));
        }
        if (!r?.can(me.id, "invite")) continue;
        const let_in = await conversation.admit(sp).catch(e => (ctx.log("upkeep", { what: `${sp.name}: ${e.message}` }), []));
        if (let_in.length) ctx.log("upkeep", { what: `${let_in.length} let into ${sp.name}` });
      }
      await mandate(me).catch(e => ctx.log("upkeep", { what: `the mandate: ${e.message}` }));
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
      const r = await roles.of(sp);
      await r.refresh();
      await r.act({ act: "admitted", code: a.code, did: a.did, at: a.at });
      written.push([a.space, a.did]);
      ctx.log("upkeep", { what: `the delegate let ${a.did.slice(12, 20)}… into ${sp.name} while no page ran` });
    }
    if (written.length) await auth.identity.upkeepAck(written);
  }
  // THE MANDATE: every shared space where this person may invite, as this page knows it now.
  async function mandate(me) {
    const spaces = [];
    for (const sp of (await space.mine()).filter(s => s.kind === "server")) {
      const r = await roles.of(sp).catch(() => null);
      if (!r?.can(me.id, "invite")) continue;
      const g = await keys.group(sp).snapshot().catch(() => null);
      if (!g) continue;
      spaces.push({
        space: sp.id, name: sp.name, kind: sp.kind, owner: sp.governance.owner, nonce: sp.governance.nonce ?? null, channel: sp.tables.channel,
        open: r.policy("", "join") === "anyone",
        codes: r.invites().map(i => [i.code, i.expires || 0, i.uses ? i.uses - i.admitted.length : 0]),
        // Never let in again: the banned AND whoever the acts put out (removed, left) — an old request of theirs still
        // sits in a bag upkeep reads.
        bans: [...new Set([...r.bannedList(), ...r.goneList()])], members: r.members().map(m => m.did), epoch: g.epoch, state: g.state,
      });
    }
    // With it, the key packages this account used already: upkeep never uses one again.
    const used = await storage().catch(() => null);
    const spent = used ? used.rows().filter(r => r.value && /^[0-9a-f]{32}$/.test(r.key)).map(r => r.key) : [];
    const r = await auth.identity.upkeepMandate(me.id, spaces, spent);
    if (r.upkeep?.stale?.length) ctx.log("upkeep", { what: `${r.upkeep.stale.length} space(s): the delegate's group is newer (loaded next tick)` });
  }
  // THE DELEGATE's upkeep (no page open: the node wakes it — node ≥ 0.2.139, with the Background grant): told which
  // contract the account's inbox is, once per login.
  async function handOver() {
    const me = await space.account();
    if (!me) return;
    const [{ glue }, bagCode, tailCode, idlogCode, sealedCode, pieceCode, blockCode] = await Promise.all(["node", "bag-wasm", "tail-wasm", "idlog-wasm", "sealed-wasm", "piece-wasm", "block-wasm"].map(n => ctx.require(n)));
    const Core = glue.CraftworksCore;
    const id = Core.bag_id(bagCode, Core.inbox_address(me.idBytes));
    const bytes = new Uint8Array(id.match(/../g).map(x => parseInt(x, 16)));
    const r = await auth.identity.upkeepWatch(bytes).catch(e => ({ refused: e.message }));
    // The contracts it writes (their code) and reads: handed over when they are not the ones it holds — the Sealed,
    // Piece and Block contracts too (it re-keys a space's files with no page open).
    if (r.upkeep && r.upkeep.codes !== Core.upkeep_codes_hash(bagCode, tailCode, idlogCode, sealedCode, pieceCode, blockCode))
      await auth.identity.upkeepCodes(bagCode, tailCode, idlogCode, sealedCode, pieceCode, blockCode).catch(() => {});
    ctx.log("upkeep", { what: r.upkeep ? `the delegate watches the inbox (${r.upkeep.wakeups} wake-up(s) so far)` : `the delegate: ${r.refused ?? JSON.stringify(r)}` });
    // What it did with no page open (admissions, re-keys): its last lines, each said once a page opens.
    for (const line of String(r.upkeep?.said ?? "").split("\n").filter(Boolean)) ctx.log("upkeep said", { what: line });
  }
  handOver().catch(() => {});
  addEventListener("craftworks:auth", () => handOver().catch(() => {}));
  tick();
  setInterval(tick, 30000);
  addEventListener("craftworks:auth", () => tick());
  return { tick };
}
