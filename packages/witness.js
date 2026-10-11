// WITNESS, a service (TOKEN §7a, S3): while a page is open and logged in, its account's $CRAFT stake (claimed by the
// keeper, `keeper_claim`) stands in the witness lots — and the page WITNESSES: every few beacon heights it reads the
// witness queue (this standing epoch's and the last), and for each entry its member key is drawn for, runs THE WITNESS
// RULE over what it reads (`core.witness_entry`: `wallet::may_witness`, the rule Wren's tool runs too); an entry the
// rule takes is signed through the identity delegate (`witnessSign`: it records the place first and never signs
// another entry there) and the signature dropped into the entry's witness bag. The EPOCH STATEMENT too: where its
// member key is seated at the statement's place, the root THIS page computes (`statement.root`) is signed — no tool
// and no one node is needed for a statement to be accepted. With the page CLOSED, the delegate witnesses entries.
//
//   (await ctx.require("witness")).status()   // { signed, refused, seated, last } — what this page did
export async function start(ctx) {
  const beaconSvc = await ctx.require("beacon");
  if (beaconSvc.status().off) return { status: () => ({ off: true }) };
  const [node, auth, index] = await Promise.all(["node", "auth", "index"].map(n => ctx.require(n)));
  const { core } = node;
  const Core = node.glue.CraftworksCore;
  const EPOCH = Core.standing_epoch();
  const EVERY = 4; // heights between passes
  const state = { signed: 0, refused: 0, seated: 0, last: null };
  const done = new Map(); // queued item (hex) → "signed" | "not seated" | why refused (asked again while refused)
  let busy = false;
  let lastPass = -Infinity;

  // SIGNED through the delegate (it records the place first) and dropped into the place's witness bag.
  async function signInto(s, h, tokens, what) {
    const a = await auth.identity.witnessSign(s.chain, s.seq, s.entry);
    if (!a?.signed) {
      ctx.log("witness", { what: `not signed (${a?.refused ?? "no answer"})` });
      return false;
    }
    const { address, item } = Core.witness_sig_item(s.chain, s.seq, s.entry, tokens.bytes(a.signed));
    await index.bag.add(address, item, "witnessing");
    state.signed += 1;
    state.last = h;
    ctx.log("witness", { what: `signed ${what}` });
    return true;
  }

  async function pass(h) {
    if (busy || h - lastPass < EVERY) return;
    busy = true;
    lastPass = h;
    try {
      const me = await auth.check().catch(() => null);
      if (!me) return;
      const tokens = await ctx.require("tokens");
      if (!tokens.on) return;
      const { run, lot, bytes, hexOf } = tokens;
      const epoch = Math.floor(h / EPOCH);
      for (let e = Math.max(0, epoch - 1); e <= epoch; e++) {
        for (const q of await index.bag.chain(Core.witness_queue_address, e, "reading the witness queue")) {
          const k = hexOf(q);
          if (done.get(k) === "signed" || done.get(k) === "not seated") continue;
          const v = await run(() => core.witness_entry(tokens.code, tokens.beacon, tokens.idlogCode, lot.pool, lot.bag, lot.beacon, q, bytes(me.member), h)).catch(x => ({ refused: x.message ?? String(x) }));
          if (v?.seated === false) {
            done.set(k, "not seated");
            continue;
          }
          if (v?.later) continue; // not yet decidable: asked again next pass
          state.seated += 1;
          if (v?.refused) {
            if (done.get(k) !== v.refused) ctx.log("witness", { what: `refused ${k.slice(8, 20)}…: ${v.refused}` });
            done.set(k, v.refused);
            state.refused += 1;
            continue;
          }
          if (await signInto(v.sign, h, tokens, `${hexOf(v.sign.chain).slice(0, 8)}… seq ${v.sign.seq}`)) done.set(k, "signed");
        }
      }
      // THE EPOCH STATEMENT due now (and the one before), once its lot is drawn: this page's own root, if seated.
      const statement = await ctx.require("statement");
      const se = Math.floor((h - statement.settle) / EPOCH);
      for (let e = Math.max(0, se - 1); e <= se; e++) {
        if (h < Core.statement_lot_height(e)) continue;
        const root = await statement.root(e).catch(() => null);
        const k = `statement ${e} ${root}`;
        if (!root || done.get(k) === "signed" || done.get(k) === "not seated") continue;
        const v = await run(() => core.witness_statement(tokens.code, tokens.beacon, tokens.idlogCode, lot.pool, lot.bag, lot.beacon, e, root, bytes(me.member))).catch(x => ({ refused: x.message ?? String(x) }));
        if (v?.seated === false) {
          done.set(k, "not seated");
          continue;
        }
        if (v?.later) continue;
        if (v?.refused) {
          if (done.get(k) !== v.refused) ctx.log("witness", { what: `the statement of epoch ${e}: ${v.refused}` });
          done.set(k, v.refused);
          continue;
        }
        state.seated += 1;
        if (await signInto(v.sign, h, tokens, `the statement of epoch ${e}: root ${root.slice(0, 12)}…`)) done.set(k, "signed");
      }
    } catch (e) {
      ctx.log("witness", { what: e.message ?? String(e) });
    } finally {
      busy = false;
    }
  }
  // THE MANDATE (once a page): so the delegate witnesses with no page open too (its `witness_round`).
  let mandated = false;
  beaconSvc.onHeight(async () => {
    if (mandated) return;
    const me = await auth.check().catch(() => null);
    const tokens = me ? await ctx.require("tokens") : null;
    if (!tokens?.on) return;
    mandated = true;
    const bagCode = await ctx.require("bag-wasm");
    const a = await auth.identity.witnessMandate(tokens.code, tokens.idlogCode, bagCode, tokens.lot.beacon, tokens.beacon, tokens.lot.pool).catch(e => ({ refused: e.message ?? String(e) }));
    ctx.log("witness", { what: a?.witnessMandated ? "the delegate witnesses with no page open too" : `the delegate not handed the witness round (${a?.refused ?? "no answer"})` });
  });
  beaconSvc.onHeight(pass);
  return { status: () => ({ ...state }) };
}
