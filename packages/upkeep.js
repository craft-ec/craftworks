// UPKEEP, a service (started by the header, so on every page, after the page is up): what keeps this person's spaces
// current without any one page open — every 30 s (and at once): WELCOMES waiting in the inbox joined (a space someone
// let this person into appears on the rail), and, in every shared space where this person may invite, whoever ASKED
// to join (an invite code, an open space) let in. The pages that did this only while open (Chat, a space's Home) no
// longer need to.
//
// With NO page open, the IDENTITY DELEGATE admits (the node wakes it every minute; node ≥ 0.2.139, with the Background
// grant) by the MANDATE handed over here each tick: per space where this person may invite, how people get in, who is
// in, and its MLS group. What it did meanwhile is taken first on every tick: the groups it moved, loaded; the people it
// let in, written as `admitted` acts (when it happened). A page that ticks keeps the delegate out of its way.
export async function start(ctx) {
  const [space, conversation, roles, keys, auth] = await Promise.all(["space", "conversation", "roles", "keys", "auth"].map(n => ctx.require(n)));
  let running = false;
  async function tick() {
    if (running) return;
    running = true;
    try {
      const me = await space.account();
      if (!me) return;
      await absorb(me).catch(e => ctx.log("upkeep", { what: `what the delegate did: ${e.message}` }));
      const joined = await conversation.accept().catch(e => (ctx.log("upkeep", { what: `the inbox: ${e.message}` }), []));
      if (joined.length) ctx.log("upkeep", { what: `joined ${joined.length} space(s) from the inbox` });
      for (const sp of (await space.mine()).filter(s => s.kind === "server")) {
        const r = await roles.of(sp).catch(() => null);
        // The acts hold in the group: whoever is banned, removed or has left and is still in it, out — by ANY member
        // (the acts already decided it; the group's one order refuses a second commit of the same removal).
        if (r && !r.left && r.role(me.id)) {
          const n = await (await ctx.require("moderation")).of(sp).then(m => m.enforce()).catch(e => (ctx.log("upkeep", { what: `${sp.name}: bans: ${e.message}` }), 0));
          if (n) ctx.log("upkeep", { what: `${sp.name}: ${n} banned device(s) taken out of the group` });
        }
        if (!r?.can(me.id, "invite")) continue;
        const let_in = await conversation.admit(sp).catch(e => (ctx.log("upkeep", { what: `${sp.name}: ${e.message}` }), []));
        if (let_in.length) ctx.log("upkeep", { what: `${let_in.length} let into ${sp.name}` });
      }
      await mandate(me).catch(e => ctx.log("upkeep", { what: `the mandate: ${e.message}` }));
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
        bans: r.bannedList(), members: r.members().map(m => m.did), epoch: g.epoch, state: g.state,
      });
    }
    const r = await auth.identity.upkeepMandate(me.id, spaces);
    if (r.upkeep?.stale?.length) ctx.log("upkeep", { what: `${r.upkeep.stale.length} space(s): the delegate's group is newer (loaded next tick)` });
  }
  // THE DELEGATE's upkeep (no page open: the node wakes it — node ≥ 0.2.139, with the Background grant): told which
  // contract the account's inbox is, once per login.
  async function handOver() {
    const me = await space.account();
    if (!me) return;
    const [{ glue }, bagCode, tailCode, idlogCode] = await Promise.all([ctx.require("node"), ctx.require("bag-wasm"), ctx.require("tail-wasm"), ctx.require("idlog-wasm")]);
    const Core = glue.CraftworksCore;
    const id = Core.bag_id(bagCode, Core.inbox_address(me.idBytes));
    const bytes = new Uint8Array(id.match(/../g).map(x => parseInt(x, 16)));
    const r = await auth.identity.upkeepWatch(bytes).catch(e => ({ refused: e.message }));
    // The contracts it writes (their code) and reads: handed over when they are not the ones it holds.
    if (r.upkeep && r.upkeep.codes !== Core.upkeep_codes_hash(bagCode, tailCode, idlogCode)) await auth.identity.upkeepCodes(bagCode, tailCode, idlogCode).catch(() => {});
    ctx.log("upkeep", { what: r.upkeep ? `the delegate watches the inbox (${r.upkeep.wakeups} wake-up(s) so far)` : `the delegate: ${r.refused ?? JSON.stringify(r)}` });
  }
  handOver().catch(() => {});
  addEventListener("craftworks:auth", () => handOver().catch(() => {}));
  tick();
  setInterval(tick, 30000);
  addEventListener("craftworks:auth", () => tick());
  return { tick };
}
