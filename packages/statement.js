// STATEMENT, a service (TOKEN §7d): every open page PUBLISHES the current epoch's STATEMENT if nobody has yet — anyone
// may (the two rules) — computed from every vouch registry up to it (`core.statement_build`: reputation as of the
// epoch's start, each signed item verified by its voucher's key log), its tree's blocks put through the BLOCKS door
// (public Block contracts), its root posted in the epoch's STATEMENT BAG. Witnesses check and sign a posted root (the
// lot, later); every page computes the same root from the same registries. Then THE ALLOWANCE (TOKEN §7d): if the
// treasury escrowed the epoch's pot and nobody decided its program yet, this page puts its VERDICT (each line owed,
// proven under the root) and the treasury's release — `allowance_verdict`; anyone may, nobody has to stay online.
//
//   (await ctx.require("statement")).status()   // { epoch, root, lines } of the last one this page published
//   await statement.issuance(epoch)             // { keepers, total, top, pot }: $CRAFT keeper issuance that epoch
export async function start(ctx) {
  const [node, index, blocks, idlogCode, standingCode, beaconCode, tokenCode, bagCode] = await Promise.all(["node", "index", "blocks", "idlog-wasm", "standing-wasm", "beacon-wasm", "token-wasm", "bag-wasm"].map(n => ctx.require(n)));
  const { core, glue } = node;
  const Core = glue.CraftworksCore;
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const EPOCH = Core.standing_epoch(); // standing epoch (beacon heights)
  const MARGIN = Core.statement_settle(); // heights after an epoch starts before its statement is due
  const TICK = 5 * 60 * 1000;
  const state = { epoch: null, root: null, lines: null };

  const built = new Map(); // epoch → the statement this page computed (its root, what it owes)
  async function build(epoch) {
    if (built.has(epoch)) return built.get(epoch);
    const items = [];
    for (let e = 0; e <= epoch; e++) items.push(...(await index.bag.read(Core.vouch_registry_address(e), "reading a vouch registry")));
    // KEPT: the keepers listed the epoch before (`keeper`), their records proving it.
    const keepers = epoch > 0 ? await index.bag.chain(Core.keepers_address, epoch - 1, "reading the keepers registry") : [];
    const beacon = bytes((await ctx.require("beacon")).status().id);
    // SIGNED: the epoch before's witness queue (each seated signature on its entries earns its claimant fees).
    const queued = epoch > 0 ? await index.bag.chain(Core.witness_queue_address, epoch - 1, "reading the witness queue") : [];
    for (let round = 0; round < 6; round++) {
      const r = core.statement_build(idlogCode, items, epoch, keepers, standingCode, beaconCode, beacon, tokenCode, bagCode, queued);
      if (r.want) {
        await Promise.all(r.want.map(id => node.read(bytes(id), "reading a key log or a keeper's record", { absent: true }).catch(() => false)));
        continue;
      }
      built.set(epoch, r);
      return r;
    }
    throw new Error("the vouchers' key logs or the keepers' records could not be read");
  }
  const decided = new Set(); // epochs whose allowance this page saw decided
  async function publishIfMissing() { // (and a root of its own beside another)
    const h = (await ctx.require("beacon")).status().height;
    if (h == null || h < MARGIN) return;
    const epoch = Math.floor((h - MARGIN) / EPOCH);
    const address = Core.statement_bag_address(epoch);
    const posted = await index.bag.read(address, "reading the epoch's statement");
    // POSTED unless the root this page computes is there already (a root computed elsewhere from another moment's view
    // stands beside it; witnesses sign the one they compute too). Recomputed each pass: records settle.
    built.delete(epoch);
    const mine = await build(epoch);
    if (!posted.some(p => (x => x?.epoch === epoch && x.root === mine.root)(Core.statement_posted_read(p)))) {
      const r = mine;
      await blocks.put([...r.puts], "the epoch's statement");
      await index.bag.add(address, r.posted, "posting the epoch's statement");
      Object.assign(state, { epoch, root: r.root, lines: r.lines });
      ctx.log("statement", { what: `epoch ${epoch}: root ${r.root.slice(0, 12)}…, ${r.lines} line(s), ${r.kept} account(s) kept the epoch before` });
    }
    const tokens = await ctx.require("tokens");
    if (!tokens.on) return;
    // KEEPER ISSUANCE (TOKEN §7d): every keeper line of the epoch's statement minted as $CRAFT stake once its root is
    // ACCEPTED — by any page, for every keeper (each mint worked: nobody need be online to be paid; `keeper_mint`
    // gives nothing for a line minted already or a root not accepted yet).
    // (The last few epochs too: a root is accepted after its epoch has begun, and the next may begin first.)
    for (let e = Math.max(0, epoch - 2); e <= epoch; e++) await mintKeepers(tokens, e, h).catch(x => ctx.log("statement", { what: `epoch ${e}: keeper mints: ${x.message ?? x}` }));
    // THE ALLOWANCE of this epoch.
    if (!tokens.allowance || decided.has(epoch)) return;
    const r = await build(epoch);
    const { allowance, run, putAll } = tokens;
    const v = await run(() => core.allowance_verdict(tokens.code, allowance.code, tokens.beacon, idlogCode, tokens.lot.pool, tokens.lot.bag, tokens.lot.beacon, allowance.token, epoch, r.root, r.owed, h));
    if (!v?.puts) return; // nothing escrowed to it (yet): the verdict step is done with nothing built
    await putAll(v.puts);
    decided.add(epoch);
    if (v.puts.length) ctx.log("statement", { what: `epoch ${epoch}: the allowance's verdict and release put (${r.owed.length} owed)` });
  }
  const minted = new Set(); // `${epoch}/${did hex}` this page minted
  let saidWaiting = null;
  async function mintKeepers(tokens, epoch, h) {
    const r = await build(epoch);
    const { run, putAll, lot, hexOf } = tokens;
    let n = 0;
    let waiting = 0;
    for (const [did, proof] of r.keepers ?? []) {
      const k = `${epoch}/${hexOf(did)}`;
      if (minted.has(k)) continue;
      const v = await run(() => core.keeper_mint(tokens.code, tokens.beacon, idlogCode, lot.pool, lot.bag, lot.beacon, did, epoch, r.root, proof, h));
      if (v?.puts) {
        await putAll(v.puts);
        n += 1;
        minted.add(k);
      } else waiting += 1; // null: minted already, or its root not accepted yet — asked again next tick
    }
    if (n) ctx.log("statement", { what: `epoch ${epoch}: ${n} keeper line(s) minted as $CRAFT stake` });
    const said = `${epoch}/${waiting}`;
    if (waiting && saidWaiting !== said) ctx.log("statement", { what: `epoch ${epoch}: ${waiting} keeper line(s) not minted here (minted already, or the statement's root not accepted yet)` });
    saidWaiting = said;
  }
  (async function loop() {
    for (;;) {
      await new Promise(r => setTimeout(r, Math.random() * TICK));
      await publishIfMissing().catch(e => ctx.log("statement", { what: e.message ?? String(e) }));
      await new Promise(r => setTimeout(r, TICK));
    }
  })();
  // THE ISSUANCE of epoch `epoch` as this page computes its statement: { keepers, total, top, pot } (units).
  const issuance = async epoch => (await build(epoch)).issuance;
  return { status: () => ({ ...state }), now: publishIfMissing, issuance };
}
