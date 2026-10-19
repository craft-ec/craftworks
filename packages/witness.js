// WITNESS, a service (TOKEN §7a, S3): while a page is open and logged in, its account's $CRAFT stake (claimed by the
// keeper, `keeper_claim`) stands in the witness lots — and the page WITNESSES: every few beacon heights it reads the
// witness queue (this standing epoch's and the last), and for each entry its member key is drawn for, runs THE WITNESS
// RULE over what it reads (`core.witness_entry`: `wallet::may_witness`, the rule Wren's tool runs too); an entry the
// rule takes is signed through the identity delegate (`witnessSign`: it records the place first and never signs
// another entry there) and the signature put into its place's signatures container (`witness::Params`: it takes only
// signatures by keys that wrote a claim for the lot's epoch, named with their staker). The EPOCH STATEMENT too: where its
// member key is seated at the statement's place, the root THIS page computes (`statement.root`) is signed — no tool
// and no one node is needed for a statement to be accepted. With the page CLOSED, the delegate witnesses entries, and
// statements: it builds the root by the same rule (`rules::statement::build`) and signs it where it matches a posted one.
//
//   (await ctx.require("witness")).status()   // { signed, refused, seated, last } — what this page did
export async function start(ctx) {
  const beaconSvc = await ctx.require("beacon");
  if (beaconSvc.status().off) return { status: () => ({ off: true }) };
  const [node, auth, index, standingCode] = await Promise.all(["node", "auth", "index", "standing-wasm"].map(n => ctx.require(n)));
  const { core } = node;
  const Core = node.glue.CraftworksCore;
  const EPOCH = Core.standing_epoch();
  const EVERY = 4; // heights between passes
  const state = { signed: 0, refused: 0, seated: 0, last: null };
  const done = new Map(); // queued item (hex) → "signed" | "not seated" | why refused (asked again while refused)
  let busy = false;
  let lastPass = -Infinity;

  // SIGNED through the delegate (it records the place first) and PUT into the place's signatures container (`params`:
  // the verdict's), under this page's member key — signed at `h`, the beacon height of this pass (part of what is signed:
  // the container takes it only near the beacon's height, `witness::fresh`).
  async function signInto(s, h, tokens, me, what) {
    const a = await auth.identity.witnessSign(s.chain, s.seq, s.entry, h);
    if (!a?.signed) {
      ctx.log("witness", { what: `not signed (${a?.refused ?? "no answer"})` });
      return false;
    }
    await tokens.putAll([core.witness_put(tokens.code, s.params, tokens.bytes(me.member), s.owner, s.entry, tokens.bytes(a.signed), h)]);
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
      const me = await auth.check();
      if (!me) return;
      const tokens = await ctx.require("tokens");
      if (!tokens.on) return;
      const { run, lot, bytes, hexOf } = tokens;
      const epoch = Math.floor(h / EPOCH);
      // MEASURED, never guessed (owner 10-11): each entry's verdict said once, as this page's key, and the pass counted.
      const key = hexOf(bytes(me.member)).slice(0, 8);
      const tally = { read: 0, signed: 0, "not seated": 0, refused: 0, later: 0 };
      const life = await ctx.require("lifecycle");
      const say = (k, what) => life.logOnce("witness", k, `${Core.queued_read(bytes(k))?.token?.slice(0, 8) ?? "?"}…/${Core.queued_read(bytes(k))?.owner?.slice(0, 8) ?? "?"} seq ${Core.queued_read(bytes(k))?.seq ?? "?"}: ${what} (as ${key})`);
      for (let e = Math.max(0, epoch - 1); e <= epoch; e++) {
        for (const q of await index.bag.chain(Core.witness_queue_address, e, "reading the witness queue")) {
          const k = hexOf(q);
          tally.read += 1;
          if (done.get(k) === "signed" || done.get(k) === "not seated") {
            tally[done.get(k)] += 1;
            continue;
          }
          const v = await run(() => core.witness_entry(tokens.code, tokens.beacon, tokens.idlogCode, lot.pool, lot.bag, lot.beacon, q, bytes(me.member), h, standingCode)).catch(x => ({ refused: x.message ?? String(x) }));
          if (v?.seated === false) {
            done.set(k, "not seated");
            tally["not seated"] += 1;
            say(k, "not seated");
            continue;
          }
          if (v?.later) {
            tally.later += 1;
            say(k, "not yet decidable");
            continue; // asked again next pass
          }
          state.seated += 1;
          if (v?.refused) {
            say(k, `refused: ${v.refused}`);
            done.set(k, v.refused);
            state.refused += 1;
            tally.refused += 1;
            continue;
          }
          if (await signInto(v.sign, h, tokens, me, `${hexOf(v.sign.chain).slice(0, 8)}… seq ${v.sign.seq}`)) (done.set(k, "signed"), (tally.signed += 1));
        }
      }
      const line = `epochs ${Math.max(0, epoch - 1)}–${epoch}: ${tally.read} queued — ${tally.signed} signed, ${tally["not seated"]} not seated, ${tally.refused} refused, ${tally.later} not yet (as ${key})`;
      if ((await ctx.require("lifecycle")).logOnce("witness", "tally", line)) {
        ctx.background("witness: ctx.require", ctx.require("index").then(ix => ix.health({ k: "witness-pass", d: { ...tally, key, epoch } })));
      }
      // THE EPOCH STATEMENT due now (and the one before), once its lot is drawn: this page's own root, if seated.
      const statement = await ctx.require("statement");
      const se = Math.floor((h - statement.settle) / EPOCH);
      for (let e = Math.max(0, se - 1); e <= se; e++) {
        if (h < Core.statement_lot_height(e)) continue;
        const root = await ctx.optional("witness: statement.root", statement.root(e), () => null);
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
        if (await signInto(v.sign, h, tokens, me, `the statement of epoch ${e}: root ${root.slice(0, 12)}…`)) done.set(k, "signed");
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
    const me = await auth.check();
    const tokens = me ? await ctx.require("tokens") : null;
    if (!tokens?.on) return;
    mandated = true;
    const [bagCode, standingCode] = await Promise.all(["bag-wasm", "standing-wasm"].map(n => ctx.require(n)));
    await ctx.require("storage"); // (it sets the core's Block code: the mandate names the statement's tree blocks by it)
    const a = await auth.identity.witnessMandate(tokens.code, tokens.idlogCode, bagCode, tokens.lot.beacon, tokens.beacon, tokens.lot.pool, standingCode).catch(e => ({ refused: e.message ?? String(e) }));
    ctx.log("witness", { what: a?.witnessMandated ? "the delegate witnesses entries and epoch statements with no page open too" : `the delegate not handed the witness round (${a?.refused ?? "no answer"})` });
  });
  beaconSvc.onHeight(pass);
  return { status: () => ({ ...state }) };
}
