// TOKENS, a capability: what every token action on a page shares (TOKEN §7c) — the set-up, the token's code, READ a
// contract into the core, PUT what a step built, RUN a step to its end (read what it wants, have this node's identity
// delegate sign what it asks: `tokenSign`), and NOW (the beacon's height, every chain entry's stamp). The core
// (`tokens` in it) builds every entry; nothing here decides a token rule. Used by `editions` and the Wallet app.
//
//   const tokens = await ctx.require("tokens");
//   tokens.on                    // the page has a beacon: false, no token action anywhere
//   tokens.beacon  tokens.pay    // bytes: the beacon (the one `beacon` extends: one clock); the token editions are paid
//                                // in (null: none)
//   tokens.allowance             // { token, code } of the WREN ALLOWANCE (TOKEN §7d), or null: not set up
//   (a payment is RECEIVED only once its debit is ACCEPTED by its witnesses — `purse.receiveAll`'s `unwitnessed`)
//   tokens.code  tokens.idlogCode
//   await tokens.read(idHex)  await tokens.putAll(puts)  await tokens.run(sigs => core.…(…, sigs))  await tokens.now()
//   await tokens.device()        // { member, creds }: this node and the account's credentials (what a step signs with)
//   tokens.purse                 // THE PRIVATE WALLET (TOKEN §7f), below: seeds, what it found, pay, receive, payees
//
// ON when the page has a beacon (`beacon`, from its `beacon-config`); the `tokens-config` package (`{ pay, allowance }`,
// hex ids) names the token editions are paid in and the token the allowance pays.
export async function start(ctx) {
  let cfg = {};
  try {
    cfg = JSON.parse(new TextDecoder().decode(await ctx.require("tokens-config")));
  } catch {}
  const clock = (await ctx.require("beacon")).status();
  if (clock.off || !clock.id) return { on: false };
  const [node, auth, membership, code, idlogCode] = await Promise.all(["node", "auth", "membership", "token-wasm", "idlog-wasm"].map(n => ctx.require(n)));
  const { core, ask } = node;
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const hexOf = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  const beacon = bytes(clock.id);
  const allowance = cfg.allowance ? { token: bytes(cfg.allowance), code: await ctx.require("allowance-wasm") } : null;
  // THE LOT a payment's acceptance is read from (TOKEN §7a): the stake pool — $CRAFT, keepers' earned stake (`craft_id`:
  // one per network; while fewer than six claim, the baseline keeps its seat) — the bag's and beacon's codes.
  const lot = { pool: bytes(node.glue.CraftworksCore.craft_id(code, beacon, idlogCode)), bag: await ctx.require("bag-wasm"), beacon: await ctx.require("beacon-wasm") };

  // READ a contract so the core holds it (`node.read`; a contract the node does not have: "read, not there").
  const read = idHex => node.read(bytes(idHex), "reading a token contract", { absent: true });
  // PUT what a step built, each answered; then FORGOTTEN by the core (read again, fresh, when next asked). A token
  // chain's new entries then go into the WITNESS QUEUE (TOKEN §7a: how they reach their witnesses).
  async function putAll(puts) {
    for (const p of puts ?? []) {
      const said = await ask(p.frames, s => (s.kind === "put" && s.key === p.id) || s.kind === "refused", "putting a token contract", 60000);
      if (said.kind !== "put") throw new Error(`the node refused it (${said.said})`);
      core.take_got(hexOf(p.idBytes));
      // KEPT TO QUEUE (audit 10-11): each new entry's queue item and witness bag recorded as soon as its put is taken,
      // and queued from there — one that fails is queued again on the next put, never left unwitnessed.
      const qs = p.queue ?? [];
      for (let i = 0; i < qs.length; i++) await priv.put(`toqueue/${hexOf(qs[i])}`, JSON.stringify({ q: hexOf(qs[i]), bag: p.bags?.[i] ? hexOf(p.bags[i]) : null, done: false }));
    }
    await queueAll();
  }
  // Each entry waiting to be queued: its WITNESS BAG made (empty) first — a witness with no page open only adds to a
  // bag already there — then the item into the witness queue of this epoch.
  async function queueAll() {
    const waiting = rows("toqueue/").filter(o => !o.done);
    if (!waiting.length) return;
    const at = await now();
    for (const o of waiting) {
      try {
        if (o.bag) await index.bag.add(bytes(o.bag), new Uint8Array(0), "making an entry's witness bag");
        await chained(Core.witness_queue_address, at, bytes(o.q), "queueing an entry for its witnesses");
        await priv.put(`toqueue/${o.q}`, JSON.stringify({ ...o, done: true }));
      } catch (e) {
        ctx.log("tokens", { what: `an entry waits to be queued again: ${e.message ?? e}` });
      }
    }
  }
  // RUN a step to its end: read what it wants, have the delegate sign what it asks (one entry a round), until done.
  async function run(step) {
    const sigs = [];
    for (let round = 0; round < 16; round++) {
      const r = step(sigs);
      if ("done" in r) return r.done;
      if (r.want) {
        await Promise.all(r.want.map(read));
        continue;
      }
      if (r.sign) {
        const a = await auth.identity.tokenSign(r.sign);
        if (!a?.signed) throw new Error(`this node did not sign (${a?.refused ?? "no answer"})`);
        sigs.push([r.sign, bytes(a.signed)]);
        continue;
      }
      throw new Error("a step with no answer");
    }
    throw new Error("too many rounds");
  }
  // NOW: the beacon's height (every chain entry is stamped by it).
  async function now() {
    await read(clock.id);
    const n = core.beacon_height(beacon);
    if (n == null) throw new Error("the beacon is not there");
    return n;
  }
  // THIS DEVICE: the node (its member key) and the account's credentials.
  async function device() {
    const me = await auth.check();
    return { me, member: bytes(me.member), creds: await membership.credentials() };
  }

  // THE PURSE (TOKEN §7f: a DID holds only stake; everything spendable is private) — the account's private table
  // `wallet` (sealed, on every device, recovered by the words):
  //   `seed/<fp>` → { seed, epoch }   the wallet seeds (the first device makes one; racing devices keep both: every
  //                                    seed is scanned, the card names the first; no scan before its epoch)
  //   `found/<to>` → { a, seed, token, to, done, edition?, taken? }   a one-time owner of this wallet's: a payment found
  //                                    by scanning (`a`: its announcement), or a PAYEE it made (an edition's: `edition`,
  //                                    the sales it took in `taken`)
  //   `sent/<announce>` → { token, kind: "send" | "shield", to, amount, at }   what this wallet paid (its HISTORY's half;
  //                                    `at`: wall-clock ms)
  //   `scanned` → the first epoch not yet scanned to its end
  const [storage, index, directory] = await Promise.all(["storage", "index", "directory"].map(n => ctx.require(n)));
  const Core = node.glue.CraftworksCore;
  const EPOCH = Core.standing_epoch();
  const priv = await storage.table("wallet");
  // A CHAINED BAG's drop (`index.bag.chained`: payments, the witness queue) into the epoch of beacon height `at`.
  function chained(addressOf, at, item, what) {
    return index.bag.chained(addressOf, Math.floor(at / EPOCH), item, what);
  }
  const rows = prefix => priv.rows().filter(r => r.key.startsWith(prefix)).map(r => JSON.parse(r.value));
  async function seeds() {
    const have = rows("seed/").sort((a, b) => (a.seed < b.seed ? -1 : 1));
    if (have.length) return have;
    const seed = hexOf(crypto.getRandomValues(new Uint8Array(32)));
    const made = { seed, epoch: Math.floor((await now()) / EPOCH) };
    await priv.put(`seed/${seed.slice(0, 16)}`, JSON.stringify(made));
    return [made];
  }
  const found = () => rows("found/");
  // The card's PAY KEYS: the first seed's, published if the card does not have them.
  async function ensureCard(me) {
    const keys = Core.stealth_public(bytes((await seeds())[0].seed));
    const card = await directory.card(me.did, { fresh: true }).catch(() => null);
    if (card?.pay?.scan !== keys.scan || card?.pay?.spend !== keys.spend) await directory.setPay(keys);
  }
  // DROP an announcement into the current epoch's payments bags.
  const announce = (at, a) => chained(Core.stealth_payments_address, at, a, "announcing a payment");
  // THE OUTBOX (audit 10-11): every announcement written here BEFORE its debit is put, and announced from here — one
  // that fails (a timeout, full bags) is announced again on every pass, never lost with the money already moved.
  // `sent`: the row its announcement leaves in the wallet's history (null: none — a rest of this wallet's own).
  const outbox = (a, at, sent) => priv.put(`outbox/${hexOf(a)}`, JSON.stringify({ a: hexOf(a), at, sent, done: false }));
  async function announceAll() {
    for (const o of rows("outbox/").filter(o => !o.done)) {
      try {
        await announce(o.at, bytes(o.a));
        if (o.sent) await priv.put(`sent/${o.a}`, JSON.stringify(o.sent));
        await priv.put(`outbox/${o.a}`, JSON.stringify({ ...o, done: true }));
      } catch (e) {
        ctx.log("wallet", { what: `an announcement waits to be sent again: ${e.message ?? e}` });
      }
    }
  }
  // RECEIVE everything owed this wallet: SCAN the payments bags from the last epoch scanned to now (every payment
  // found kept; `onToken(id)` for its token), take each payment not yet taken, and each PAYEE's sales (an edition's
  // records name them). `{ got, pending }`: chains credited, payments still inside their margin.
  async function receiveAll({ onToken } = {}) {
    // (What waits in the outbox first: a payment whose announcement did not go out is announced again.)
    await announceAll();
    const all = await seeds();
    const at = await now();
    const current = Math.floor(at / EPOCH);
    const scanned = Number(priv.rows().find(r => r.key === "scanned")?.value ?? Math.min(...all.map(x => x.epoch)));
    for (let e = scanned; e <= current; e++) {
      // (STRICT: an epoch whose bags did not answer stops the scan — the cursor never moves past it unread.)
      const items = await index.bag.chain(Core.stealth_payments_address, e, "reading the payments bags", { strict: true });
      for (const sd of all) {
        for (const f of Core.stealth_scan(bytes(sd.seed), items)) {
          if (priv.rows().some(r => r.key === `found/${f.to}`)) continue;
          await priv.put(`found/${f.to}`, JSON.stringify({ a: hexOf(f.bytes), seed: sd.seed.slice(0, 16), token: f.token, to: f.to, done: false, at: Date.now() }));
          await onToken?.(f.token);
        }
      }
      if (e < current) await priv.put("scanned", String(e + 1));
    }
    let got = 0;
    let pending = 0;
    let unwitnessed = 0;
    // One payment (or sale) on its own (`a`: its announcement's bytes): one that cannot be taken (another clock, a chain
    // that does not read) stops no other.
    const take = async (a, what) => {
      try {
        const r = await run(() => core.wallet_receive_private(code, beacon, idlogCode, lot.pool, lot.bag, lot.beacon, a, at));
        await putAll(r.puts);
        if (r.puts?.length) got++;
        pending += r.pending ?? 0;
        unwitnessed += r.unwitnessed ?? 0;
        if (r.why) ctx.log("wallet", { what: `${what}: not accepted yet — ${r.why}` });
        return !r.pending && !r.unwitnessed;
      } catch (e) {
        ctx.log("wallet", { what: `receiving ${what}: ${e.message ?? e}` });
        return false;
      }
    };
    for (const f of found().filter(f => !f.done && !f.edition)) {
      if (await take(bytes(f.a), f.to.slice(0, 12))) await priv.put(`found/${f.to}`, JSON.stringify({ ...f, done: true }));
    }
    // THE ALLOWANCE (TOKEN §7d): each recent epoch's payout to this account's DID chain, then MOVED TO PRIVATE (§7f: a
    // DID holds only stake) — never the token's creator's (the treasury is public on purpose).
    if (allowance) {
      const me = await auth.check();
      const from = Number(priv.rows().find(r => r.key === "allowed")?.value ?? Math.max(0, current - 8));
      for (let e = from; e <= current; e++) {
        try {
          const r = await run(() => core.allowance_payout(code, allowance.code, beacon, idlogCode, lot.pool, lot.bag, lot.beacon, allowance.token, e, me.didBytes, at));
          if (r?.puts?.length) {
            await putAll(r.puts);
            got++;
          }
        } catch (err) {
          ctx.log("wallet", { what: `the allowance of epoch ${e}: ${err.message ?? err}` });
        }
      }
      if (current - 1 > from) await priv.put("allowed", String(current - 1));
      const v = await run(() => core.wallet_view(code, beacon, idlogCode, allowance.token, me.didBytes)).catch(() => null);
      if (v && v.begun && v.balance !== "0" && v.creator !== hexOf(me.didBytes)) {
        await shield(me, hexOf(allowance.token), v.balance).catch(err => ctx.log("wallet", { what: `moving the allowance to private: ${err.message ?? err}` }));
        await onToken?.(hexOf(allowance.token));
      }
    }
    // $CRAFT's WITNESS FEES (spendable, minted on the DID chain): moved to private like any payment (TOKEN §7f: a DID
    // chain holds only stake — the keeper share stays there, locked, the lot's weight).
    {
      const me = await auth.check();
      const craftId = Core.craft_id(code, beacon, idlogCode);
      const v = await run(() => core.wallet_view(code, beacon, idlogCode, bytes(craftId), me.didBytes)).catch(() => null);
      if (v && v.begun && v.balance !== "0") {
        await shield(me, craftId, v.balance).catch(err => ctx.log("wallet", { what: `moving witness fees to private: ${err.message ?? err}` }));
        await onToken?.(craftId);
      }
    }
    for (const f of found().filter(f => f.edition)) {
      const sales = await run(() => core.edition_sales(code, beacon, idlogCode, bytes(f.edition))).catch(() => []);
      const taken = new Set(f.taken ?? []);
      for (const s of sales) {
        const key = hexOf(s);
        if (!taken.has(key) && (await take(s, `a sale of ${f.edition.slice(0, 12)}…`))) taken.add(key);
      }
      if (taken.size !== (f.taken ?? []).length) await priv.put(`found/${f.to}`, JSON.stringify({ ...f, taken: [...taken] }));
    }
    return { got, pending, unwitnessed };
  }
  // This wallet's spendable private chains of a token, by seed: [{ seed, mine: [found rows] }].
  async function privately(id) {
    return (await seeds()).map(sd => ({ seed: sd.seed, mine: found().filter(f => f.token === id && f.seed === sd.seed.slice(0, 16) && (f.done || f.edition)) })).filter(g => g.mine.length);
  }
  // PAY `amount` (smallest unit, a string) of the token `id` to the wallet whose card keys are `pay`, from this
  // wallet's private chains (every seed's, as many as it takes), each debit to a fresh one-time owner, announced.
  async function pay(id, pay, amount, to = null) {
    const at = await now();
    let left = BigInt(amount);
    const plan = [];
    for (const g of await privately(id)) {
      if (left === 0n) break;
      const p = await run(() => core.wallet_private(code, beacon, idlogCode, bytes(id), g.mine.map(f => bytes(f.to))));
      const take = BigInt(p.balance) < left ? BigInt(p.balance) : left;
      if (take > 0n) plan.push({ ...g, amount: take });
      left -= take;
    }
    if (left > 0n) throw new Error(`not enough held privately (${left} short)`);
    const announces = [];
    for (const g of plan) {
      const rand = crypto.getRandomValues(new Uint8Array(32));
      const s = await run(() => core.wallet_send_private(code, beacon, idlogCode, bytes(id), bytes(g.seed), g.mine.map(f => bytes(f.a)), bytes(pay.scan), bytes(pay.spend), g.amount.toString(), rand, at));
      // In the OUTBOX before the debit is put: the payment's announcements, and each chain's REST (moved to a fresh
      // address of this wallet's own — a chain pays once: kept found now, announced so the account's other devices
      // find it too).
      for (const a of s.announces) await outbox(a, at, { token: id, kind: "send", to, at: Date.now() });
      for (const r of s.rest ?? []) {
        const rto = hexOf(r.slice(84, 116));
        await priv.put(`found/${rto}`, JSON.stringify({ a: hexOf(r), seed: g.seed.slice(0, 16), token: id, to: rto, done: false, at: Date.now(), rest: true }));
        await outbox(r, at, null);
      }
      await putAll(s.puts);
    }
    await announceAll();
  }
  // MOVE TO PRIVATE: `amount` of the token `id` from this account's DID chain to its own card — the one DID → private
  // step (§7f: a DID holds only stake), of what was already public.
  async function shield(me, id, amount) {
    const keys = Core.stealth_public(bytes((await seeds())[0].seed));
    const at = await now();
    const { member, creds } = await device();
    // One random for the whole run: each round rebuilds the debit, and its one-time owner must stay the one signed.
    const rand = crypto.getRandomValues(new Uint8Array(32));
    const s = await run(sigs => core.wallet_send(code, beacon, idlogCode, bytes(id), me.didBytes, bytes(keys.scan), bytes(keys.spend), amount, rand, at, member, creds, sigs));
    await outbox(s.announce, at, { token: id, kind: "shield", to: me.did, at: Date.now() });
    await putAll(s.puts);
    await announceAll();
  }
  // A PAYEE: a fresh one-time owner of this wallet's own for the token `pay` — an edition's (it collects that
  // edition's sales: `edition` set once the edition is made).
  async function payee(payToken) {
    const sd = (await seeds())[0];
    const o = Core.stealth_own(bytes(sd.seed), bytes(payToken), crypto.getRandomValues(new Uint8Array(32)));
    const row = { a: hexOf(o.bytes), seed: sd.seed.slice(0, 16), token: payToken, to: o.to, done: true };
    return { to: o.to, keep: edition => priv.put(`found/${o.to}`, JSON.stringify({ ...row, edition })) };
  }
  // A token's HISTORY, newest first: what this wallet sent (and moved to private), what it received, its editions'
  // sales — each { kind, a (the announcement, hex: its status is `status(a)`), to?, at? }.
  function history(id) {
    const sent = priv.rows().filter(r => r.key.startsWith("sent/")).map(r => ({ a: r.key.slice(5), ...JSON.parse(r.value) })).filter(x => x.token === id);
    const mine = new Set(sent.map(x => x.a));
    const received = found().filter(f => f.token === id && !f.edition && !f.rest && !mine.has(f.a)).map(f => ({ kind: "received", a: f.a, at: f.at ?? null }));
    const sales = found().filter(f => f.token === id && f.edition).flatMap(f => (f.taken ?? []).map(a => ({ kind: "sale", a, edition: f.edition, at: null })));
    return [...sent, ...received, ...sales].sort((x, y) => (y.at ?? 0) - (x.at ?? 0));
  }
  // A payment's STATUS (`wallet_payment_status`): { amount, stamp, stage, why, signed } or null.
  async function status(a) {
    const at = await now();
    return run(() => core.wallet_payment_status(code, beacon, idlogCode, lot.pool, lot.bag, lot.beacon, bytes(a), at));
  }
  const purse = { seeds, found, history, status, ensureCard, receiveAll, privately, pay, shield, payee, owners: () => found().map(f => bytes(f.to)), onChange: fn => priv.onChange(fn) };

  return { on: true, purse, allowance, lot, beacon, pay: cfg.pay ? bytes(cfg.pay) : null, code, idlogCode, read, putAll, run, now, device, bytes, hexOf };
}
