// STATEMENT, a service (TOKEN §7d): every open page PUBLISHES the current epoch's STATEMENT if nobody has yet — anyone
// may (the two rules) — computed from every vouch registry up to it (`core.statement_build`: reputation as of the
// epoch's start, each signed item verified by its voucher's key log), its tree's blocks put through the BLOCKS door
// (public Block contracts), its root posted in the epoch's STATEMENT BAG. Witnesses check and sign a posted root (the
// lot, later); every page computes the same root from the same registries.
//
//   (await ctx.require("statement")).status()   // { epoch, root, lines } of the last one this page published
export async function start(ctx) {
  const [node, index, blocks, idlogCode] = await Promise.all(["node", "index", "blocks", "idlog-wasm"].map(n => ctx.require(n)));
  const { core, glue } = node;
  const Core = glue.CraftworksCore;
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const EPOCH = 1024; // standing epoch (beacon heights)
  const MARGIN = 2; // heights after an epoch starts before its statement is due
  const TICK = 5 * 60 * 1000;
  const state = { epoch: null, root: null, lines: null };

  async function publishIfMissing() {
    const h = (await ctx.require("beacon")).status().height;
    if (h == null || h < MARGIN) return;
    const epoch = Math.floor((h - MARGIN) / EPOCH);
    const address = Core.statement_bag_address(epoch);
    const posted = await index.bag.read(address, "reading the epoch's statement");
    if (posted.some(p => Core.statement_posted_read(p)?.epoch === epoch)) return;
    const items = [];
    for (let e = 0; e <= epoch; e++) items.push(...(await index.bag.read(Core.vouch_registry_address(e), "reading a vouch registry")));
    for (let round = 0; round < 6; round++) {
      const r = core.statement_build(idlogCode, items, epoch);
      if (r.want) {
        await Promise.all(r.want.map(id => node.read(bytes(id), "reading a voucher's key log", { absent: true }).catch(() => false)));
        continue;
      }
      await blocks.put([...r.puts], "the epoch's statement");
      await index.bag.add(address, r.posted, "posting the epoch's statement");
      Object.assign(state, { epoch, root: r.root, lines: r.lines });
      ctx.log("statement", { what: `epoch ${epoch}: root ${r.root.slice(0, 12)}…, ${r.lines} line(s)` });
      return;
    }
  }
  (async function loop() {
    for (;;) {
      await new Promise(r => setTimeout(r, Math.random() * TICK));
      await publishIfMissing().catch(e => ctx.log("statement", { what: e.message ?? String(e) }));
      await new Promise(r => setTimeout(r, TICK));
    }
  })();
  return { status: () => ({ ...state }), now: publishIfMissing };
}
