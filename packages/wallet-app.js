// WALLET, an app (`#/wallet`): this account's TOKENS (TOKEN §7c), held PRIVATELY (§7f, owner 10-10: what an account
// holds is not linkable to its DID). SEND to anyone by their id: a fresh one-time owner derived from their card's PAY
// KEYS, announced in the epoch's PAYMENTS BAGS (never their inbox: its address is their DID's). RECEIVE by SCANNING
// the bags with this wallet's scan key — each payment found is kept, its credit taken once its margin has passed
// (worked, no signature; while one is inside its margin, again each time the beacon advances). The wallet SEED and
// what it found are the account's private table `wallet` (sealed, on every device, recovered by the words); the card
// carries the pay keys. A DID chain (stake, rewards, a creator's supply) is shown and spent from too. No token is MADE
// here (owner 10-10: a page that made them would let anyone flood the network with tokens). The tokens listed are the
// account's table `tokens`. UI only: every entry is the core's (`wallet_*`, `stealth_*`), every step `tokens`' run.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  el.innerHTML = `<style>@layer apps {
    .wl { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); max-width: 44rem; }
    .wl h2, .wl h3, .wl p { margin: 0; }
    .wl .note { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .wl .said { color: var(--cw-danger); font-size: var(--cw-text-sm); }
    .wl .tok { background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-3); display: grid; gap: var(--cw-space-2); }
    .wl .tok .top { display: flex; gap: var(--cw-space-3); align-items: baseline; flex-wrap: wrap; }
    .wl .tok .name { font-weight: 600; }
    .wl .tok .bal { font-variant-numeric: tabular-nums; font-size: 1.1rem; }
    .wl form { display: flex; gap: var(--cw-space-2); align-items: end; flex-wrap: wrap; }
    .wl label { display: grid; gap: var(--cw-space-1); font-size: var(--cw-text-sm); }
    .wl input { min-width: 0; }
  }</style><div class="wl"><h2>👛 Wallet</h2></div>`;
  const root = el.querySelector(".wl");
  const tokens = await ctx.require("tokens");
  if (!tokens.on) {
    root.append(Object.assign(document.createElement("p"), { className: "note", textContent: "Tokens are not set up on this site." }));
    return;
  }
  const [node, storage, index, conversation, directory] = await Promise.all(["node", "storage", "index", "conversation", "directory"].map(n => ctx.require(n)));
  const { core, glue } = node;
  const { beacon, code, idlogCode, run, putAll, now, bytes, hexOf } = tokens;
  const table = await storage.table("tokens");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };

  // AMOUNTS: the smallest unit (a decimal string) shown with the token's decimals, and back.
  const shown = (v, dec) => {
    const n = BigInt(v);
    if (!dec) return n.toLocaleString();
    const p = 10n ** BigInt(dec);
    const frac = (n % p).toString().padStart(dec, "0").replace(/0+$/, "");
    return `${(n / p).toLocaleString()}${frac ? `.${frac}` : ""}`;
  };
  const units = (text, dec) => {
    const m = String(text).trim().match(/^(\d*)(?:\.(\d*))?$/);
    if (!m || (!m[1] && !m[2])) throw new Error("an amount is a number");
    const frac = (m[2] ?? "").replace(/0+$/, "");
    if (frac.length > dec) throw new Error(`at most ${dec} decimals`);
    return (BigInt(m[1] || "0") * 10n ** BigInt(dec) + BigInt(frac.padEnd(dec, "0") || "0")).toString();
  };

  const Core = glue.CraftworksCore;
  const EPOCH = Core.standing_epoch();
  const ROOM = Core.stealth_room();
  const priv = await storage.table("wallet");
  const hexToBytes = x => bytes(x);

  // THE WALLET SEEDS (TOKEN §7f): rows `seed/<fingerprint>` of the account's private table `wallet` (sealed, on every
  // device, recovered by the words). The first device makes one; two devices racing each keep theirs — every seed is
  // scanned, the card names the first. Each row: { seed, epoch } (the epoch it was made: no scan before it).
  async function seeds() {
    const have = priv.rows().filter(r => r.key.startsWith("seed/")).map(r => JSON.parse(r.value)).sort((a, b) => (a.seed < b.seed ? -1 : 1));
    if (have.length) return have;
    const seed = hexOf(crypto.getRandomValues(new Uint8Array(32)));
    const made = { seed, epoch: Math.floor((await now()) / EPOCH) };
    await priv.put(`seed/${seed.slice(0, 16)}`, JSON.stringify(made));
    return [made];
  }
  // The card's PAY KEYS: the first seed's, published if the card does not have them.
  async function ensureCard(me, first) {
    const keys = Core.stealth_public(hexToBytes(first.seed));
    const card = await directory.card(me.did, { fresh: true }).catch(() => null);
    if (card?.pay?.scan !== keys.scan || card?.pay?.spend !== keys.spend) await directory.setPay(keys);
  }
  // FOUND payments: rows `found/<to>` → { a (announcement hex), seed (its fingerprint), token, done }.
  const found = () => priv.rows().filter(r => r.key.startsWith("found/")).map(r => JSON.parse(r.value));

  // A token as this account holds it: on its DID chain (stake, rewards, a creator's supply) and privately (the chains
  // of the payments it found), summed.
  async function viewOf(id, me) {
    const v = await run(() => core.wallet_view(code, beacon, idlogCode, bytes(id), me.didBytes));
    const mine = found().filter(f => f.token === id);
    const p = mine.length ? await run(() => core.wallet_private(code, beacon, idlogCode, bytes(id), mine.map(f => hexToBytes(f.to)))) : { balance: "0", chains: [] };
    const onId = BigInt(v.begun || v.creator !== hexOf(me.didBytes) ? v.balance : v.supply);
    return { ...v, onId: onId.toString(), private: p.balance, chains: p.chains.length, total: (onId + BigInt(p.balance)).toString() };
  }

  // DROP an announcement into the current epoch's payments bags: the first with room, read back (over a cap a bag
  // keeps its most-worked items: not kept → the next).
  async function announce(at, a) {
    const epoch = Math.floor(at / EPOCH);
    const want = hexOf(a);
    for (let k = 0; k < 64; k++) {
      const address = Core.stealth_payments_address(epoch, k);
      const items = await index.bag.read(address, "reading a payments bag");
      if (items.some(i => hexOf(i) === want)) return;
      if (items.length >= ROOM) continue;
      await index.bag.add(address, a, "announcing a payment");
      const again = await index.bag.read(address, "reading a payments bag back");
      if (again.some(i => hexOf(i) === want)) return;
    }
    throw new Error("the payment was made but could not be announced: the payments bags are full");
  }

  // SCAN the payments bags from the last epoch scanned (or each seed's first) to now: every payment to this wallet
  // kept as found (and its token listed); then RECEIVE each not yet received.
  async function receiveAll(me) {
    const all = await seeds();
    const at = await now();
    const current = Math.floor(at / EPOCH);
    const scanned = Number(priv.rows().find(r => r.key === "scanned")?.value ?? Math.min(...all.map(x => x.epoch)));
    for (let e = scanned; e <= current; e++) {
      for (let k = 0; k < 64; k++) {
        const items = await index.bag.read(Core.stealth_payments_address(e, k), "reading a payments bag");
        for (const sd of all) {
          for (const f of Core.stealth_scan(hexToBytes(sd.seed), items)) {
            if (priv.rows().some(r => r.key === `found/${f.to}`)) continue;
            await priv.put(`found/${f.to}`, JSON.stringify({ a: hexOf(f.bytes), seed: sd.seed.slice(0, 16), token: f.token, to: f.to, done: false }));
            if (!table.rows().some(r => r.key === f.token)) await table.put(f.token, JSON.stringify({ name: "" }));
          }
        }
        if (items.length < ROOM) break;
      }
      if (e < current) await priv.put("scanned", String(e + 1));
    }
    let got = 0;
    let pending = 0;
    for (const f of found().filter(f => !f.done)) {
      // Each payment on its own: one that cannot be received (another clock, a chain that does not read) stops no other.
      try {
        const r = await run(() => core.wallet_receive_private(code, beacon, idlogCode, hexToBytes(f.a), at));
        await putAll(r.puts);
        if (r.puts?.length) got++;
        pending += r.pending ?? 0;
        if (!r.pending) await priv.put(`found/${f.to}`, JSON.stringify({ ...f, done: true }));
      } catch (e) {
        ctx.log("wallet", { what: `receiving ${f.to.slice(0, 12)}…: ${e.message ?? e}` });
      }
    }
    return { got, pending };
  }

  // SEND PRIVATELY to a person (their card's pay keys): from this wallet's private chains when they hold enough (of
  // one seed), else from the DID chain; each debit to a fresh one-time owner of theirs, announced.
  async function send(id, me, toText, amountText, dec) {
    const to = await conversation.person(toText.trim());
    if (!to) throw new Error("no such person: an id (did:craftec:…) or name#abc123 of someone you know");
    const amount = units(amountText, dec);
    if (amount === "0") throw new Error("send more than nothing");
    const pay = (await directory.card(to, { fresh: true }))?.pay;
    if (!pay) throw new Error(`${directory.shown(to)} has no pay keys yet: they open their Wallet once`);
    const rand = crypto.getRandomValues(new Uint8Array(32));
    const at = await now();
    const all = await seeds();
    let announces = null;
    for (const sd of all) {
      const mine = found().filter(f => f.token === id && f.seed === sd.seed.slice(0, 16) && f.done);
      if (!mine.length) continue;
      const p = await run(() => core.wallet_private(code, beacon, idlogCode, bytes(id), mine.map(f => hexToBytes(f.to))));
      if (BigInt(p.balance) < BigInt(amount)) continue;
      const s = await run(() => core.wallet_send_private(code, beacon, idlogCode, bytes(id), hexToBytes(sd.seed), mine.map(f => hexToBytes(f.a)), hexToBytes(pay.scan), hexToBytes(pay.spend), amount, rand, at));
      await putAll(s.puts);
      announces = s.announces;
      break;
    }
    if (!announces) {
      const { member, creds } = await tokens.device();
      const s = await run(sigs => core.wallet_send(code, beacon, idlogCode, bytes(id), me.didBytes, hexToBytes(pay.scan), hexToBytes(pay.spend), amount, rand, at, member, creds, sigs));
      await putAll(s.puts);
      announces = [s.announce];
    }
    for (const a of announces) await announce(at, a);
    return { to };
  }

  function row(id, me) {
    const box = h("div", { className: "tok" }, h("span", { className: "note", textContent: "reading…" }));
    viewOf(id, me).then(
      v => {
        const said = h("span", { className: "said" });
        const to = h("input", { placeholder: "name#abc123 or did:craftec:…", size: 28 });
        const amount = h("input", { placeholder: "amount", size: 10 });
        const go = h("button", { type: "submit", textContent: "Send" });
        const f = h("form", {}, h("label", {}, "To", to), h("label", {}, "Amount", amount), go);
        f.onsubmit = async ev => {
          ev.preventDefault();
          go.disabled = true;
          said.textContent = "";
          try {
            const r = await send(id, me, to.value, amount.value, v.decimals);
            status.textContent = `Sent ${amount.value.trim()} ${v.name} to ${directory.shown(r.to)}, privately.`;
            box.replaceWith(row(id, me));
          } catch (e) {
            said.textContent = e.message ?? String(e);
            go.disabled = false;
          }
        };
        const made = v.creator === hexOf(me.didBytes);
        const where = [v.private !== "0" ? `${shown(v.private, v.decimals)} private (${v.chains} chain${v.chains === 1 ? "" : "s"})` : null, v.onId !== "0" ? `${shown(v.onId, v.decimals)} on your id` : null].filter(Boolean).join(" · ");
        box.replaceChildren(
          h("div", { className: "top" }, h("span", { className: "name", textContent: v.name || "a token" }), h("span", { className: "bal", textContent: shown(v.total, v.decimals) }), where ? h("span", { className: "note", textContent: where }) : null, made ? h("span", { className: "note", textContent: `you made it · supply ${shown(v.supply, v.decimals)}` }) : null),
          h("span", { className: "note", textContent: `id ${id}` }),
          f,
          said,
        );
      },
      e => box.replaceChildren(h("span", { className: "said", textContent: `${id.slice(0, 12)}… could not be read: ${e.message ?? e}` })),
    );
    return box;
  }

  async function list(me) {
    const out = h("div", { className: "wl" });
    const said = h("span", { className: "note" });
    const add = h("input", { placeholder: "a token's id (64 hex)", size: 40 });
    const addGo = h("button", { type: "submit", textContent: "Add" });
    const addForm = h("form", {}, h("label", {}, "Add a token", add), addGo);
    addForm.onsubmit = async ev => {
      ev.preventDefault();
      const id = add.value.trim().toLowerCase();
      if (!/^[0-9a-f]{64}$/.test(id)) return (said.textContent = "a token's id is 64 hex characters");
      try {
        const v = await viewOf(id, me);
        await table.put(id, JSON.stringify({ name: v.name }));
        draw();
      } catch (e) {
        said.textContent = e.message ?? String(e);
      }
    };
    const rows = table.rows();
    out.append(...(rows.length ? rows.map(r => row(r.key, me)) : [h("p", { className: "note", textContent: "No tokens yet: be sent one, or add one by its id." })]), addForm, said);
    return out;
  }

  const status = h("p", { className: "note", role: "status" });
  const body = h("div");
  root.append(status, body);
  const me = await (await ctx.require("auth")).check();
  ensureCard(me, (await seeds())[0]).catch(e => ctx.log("wallet", { what: `the card's pay keys: ${e.message ?? e}` }));
  const draw = async () => body.replaceChildren(await list(me));
  priv.onChange(() => el.isConnected && draw());
  await draw();
  table.onChange(() => el.isConnected && draw());
  // RECEIVE now, and again each time the beacon advances while a send to this account waits out its margin.
  const beaconSvc = await ctx.require("beacon");
  let busy = false;
  let stop = null;
  const receive = async () => {
    if (busy || !el.isConnected) return stop?.();
    busy = true;
    try {
      const r = await receiveAll(me);
      if (r.got) draw();
      if (r.pending && !stop) {
        status.textContent = `${r.pending} payment(s) to you on the way: taken in a moment.`;
        stop = beaconSvc.onHeight(receive);
      } else if (!r.pending) {
        if (stop) status.textContent = r.got ? "Received." : status.textContent;
        stop?.();
        stop = null;
      }
    } catch (e) {
      ctx.log("wallet", { what: `receiving: ${e.message ?? e}` });
    } finally {
      busy = false;
    }
  };
  receive();
}
