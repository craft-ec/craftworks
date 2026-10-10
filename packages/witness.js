// WITNESS, a service (TOKEN §7a, S3): while a page is open and logged in, its account's $CRAFT stake (claimed by the
// keeper, `keeper_claim`) stands in the witness lots — and the page WITNESSES: every few beacon heights it reads the
// witness queue (this standing epoch's and the last), and for each entry its member key is drawn for, runs THE WITNESS
// RULE over what it reads (`core.witness_entry`: `wallet::may_witness`, the rule Wren's tool runs too); an entry the
// rule takes is signed through the identity delegate (`witnessSign`: it records the place first and never signs
// another entry there) and the signature dropped into the entry's witness bag. (Witnessing with the page CLOSED is
// the delegate's: later.)
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
          state.seated += 1;
          if (v?.refused) {
            if (done.get(k) !== v.refused) ctx.log("witness", { what: `refused ${k.slice(8, 20)}…: ${v.refused}` });
            done.set(k, v.refused);
            state.refused += 1;
            continue;
          }
          const s = v.sign;
          const a = await auth.identity.witnessSign(s.chain, s.seq, s.entry);
          if (!a?.signed) {
            ctx.log("witness", { what: `not signed (${a?.refused ?? "no answer"})` });
            continue;
          }
          const { address, item } = Core.witness_sig_item(s.chain, s.seq, s.entry, bytes(a.signed));
          await index.bag.add(address, item, "witnessing an entry");
          done.set(k, "signed");
          state.signed += 1;
          state.last = h;
          ctx.log("witness", { what: `signed ${hexOf(s.chain).slice(0, 8)}… seq ${s.seq}` });
        }
      }
    } catch (e) {
      ctx.log("witness", { what: e.message ?? String(e) });
    } finally {
      busy = false;
    }
  }
  beaconSvc.onHeight(pass);
  return { status: () => ({ ...state }) };
}
