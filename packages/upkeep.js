// UPKEEP, a service (started by the header, so on every page, after the page is up): what keeps this person's spaces
// current without any one page open — every 30 s (and at once): WELCOMES waiting in the inbox joined (a space someone
// let this person into appears on the rail), and, in every shared space where this person may invite, whoever ASKED
// to join (an invite code, an open space) let in. The pages that did this only while open (Chat, a space's Home) no
// longer need to.
export async function start(ctx) {
  const [space, conversation, roles] = await Promise.all(["space", "conversation", "roles"].map(n => ctx.require(n)));
  let running = false;
  async function tick() {
    if (running) return;
    running = true;
    try {
      const me = await space.account();
      if (!me) return;
      const joined = await conversation.accept().catch(e => (ctx.log("upkeep", { what: `the inbox: ${e.message}` }), []));
      if (joined.length) ctx.log("upkeep", { what: `joined ${joined.length} space(s) from the inbox` });
      for (const sp of (await space.mine()).filter(s => s.kind === "server")) {
        const r = await roles.of(sp).catch(() => null);
        if (!r?.can(me.id, "invite")) continue;
        const let_in = await conversation.admit(sp).catch(e => (ctx.log("upkeep", { what: `${sp.name}: ${e.message}` }), []));
        if (let_in.length) ctx.log("upkeep", { what: `${let_in.length} let into ${sp.name}` });
      }
    } finally {
      running = false;
    }
  }
  // THE DELEGATE's upkeep (no page open: the node wakes it — node ≥ 0.2.139, with the Background grant): told which
  // contract the account's inbox is, once per login.
  async function handOver() {
    const me = await space.account();
    if (!me) return;
    const [{ glue, core }, bagCode, auth] = await Promise.all([ctx.require("node"), ctx.require("bag-wasm"), ctx.require("auth")]);
    const Core = glue.CraftworksCore;
    const id = Core.bag_id(bagCode, Core.inbox_address(me.idBytes));
    const bytes = new Uint8Array(id.match(/../g).map(x => parseInt(x, 16)));
    const r = await auth.identity.upkeepWatch(bytes).catch(e => ({ refused: e.message }));
    ctx.log("upkeep", { what: r.upkeep ? `the delegate watches the inbox (${r.upkeep.wakeups} wake-up(s) so far)` : `the delegate: ${r.refused ?? JSON.stringify(r)}` });
  }
  handOver().catch(() => {});
  addEventListener("craftworks:auth", () => handOver().catch(() => {}));
  tick();
  setInterval(tick, 30000);
  addEventListener("craftworks:auth", () => tick());
  return { tick };
}
