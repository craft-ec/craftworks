// WALLET, an app (`#/wallet`): this account's TOKENS (TOKEN §7c), held PRIVATELY (§7f, owner 10-10: what an account
// holds is not linkable to its DID). SEND to anyone by their id: a fresh one-time owner derived from their card's PAY
// KEYS, announced in the epoch's PAYMENTS BAGS (never their inbox: its address is their DID's). RECEIVE by SCANNING
// the bags with this wallet's scan key — each payment found is kept, its credit taken once its margin has passed
// (worked, no signature; while one is inside its margin, again each time the beacon advances). The wallet SEED and
// what it found are the account's private table `wallet` (sealed, on every device, recovered by the words); the card
// carries the pay keys (all of it `tokens.purse`). The DID chain holds only stake: anything spendable on it is MOVED TO
// PRIVATE (a payment to the account's own card) before it is spent; no send from a DID. No token is MADE
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
    .wl a.tok { color: inherit; text-decoration: none; }
    .wl .big { font-size: 1.8rem; font-variant-numeric: tabular-nums; font-weight: 600; }
    .wl table { border-collapse: collapse; width: 100%; font-size: var(--cw-text-sm); }
    .wl th, .wl td { text-align: left; padding: 6px 10px 6px 0; border-bottom: 1px solid var(--cw-line); vertical-align: top; }
    .wl td.num { text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }
    .wl td.in { color: var(--cw-ok, #2aa876); } .wl td.out { color: inherit; }
    .wl .chip { display: inline-block; padding: 1px 8px; border-radius: 999px; font-size: var(--cw-text-sm); border: 1px solid var(--cw-line); white-space: nowrap; }
    .wl .chip.accepted { border-color: #2aa876; color: #2aa876; } .wl .chip.witnessing { border-color: #d9a400; color: #b38700; } .wl .chip.settling { color: var(--cw-muted); }
  }</style><div class="wl"><h2>👛 Wallet</h2></div>`;
  const root = el.querySelector(".wl");
  const tokens = await ctx.require("tokens");
  if (!tokens.on) {
    root.append(Object.assign(document.createElement("p"), { className: "note", textContent: "Tokens are not set up on this site." }));
    return;
  }
  const [node, storage, conversation, directory] = await Promise.all(["node", "storage", "conversation", "directory"].map(n => ctx.require(n)));
  const { core } = node;
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

  const { purse } = tokens;

  // A token as this account holds it: PRIVATELY (its one-time chains: payments found, an edition's sales) and on its
  // DID chain (stake; anything spendable there waits to be moved to private), summed.
  async function viewOf(id, me) {
    const v = await run(() => core.wallet_view(code, beacon, idlogCode, bytes(id), me.didBytes));
    const mine = purse.found().filter(f => f.token === id);
    const p = mine.length ? await run(() => core.wallet_private(code, beacon, idlogCode, bytes(id), mine.map(f => bytes(f.to)))) : { balance: "0", chains: [] };
    const onId = BigInt(v.begun || v.creator !== hexOf(me.didBytes) ? v.balance : v.supply);
    return { ...v, onId: onId.toString(), private: p.balance, chains: p.chains.filter(c => c.balance !== "0").length, total: (onId + BigInt(p.balance)).toString() };
  }

  // SEND to a person, privately: their card's pay keys, from this wallet's private chains only (§7f: a DID holds
  // only stake — what is on it is moved to private first).
  async function send(id, toText, amountText, dec) {
    const to = await conversation.person(toText.trim());
    if (!to) throw new Error("no such person: an id (did:craftec:…) or name#abc123 of someone you know");
    const amount = units(amountText, dec);
    if (amount === "0") throw new Error("send more than nothing");
    const pay = (await directory.card(to, { fresh: true }))?.pay;
    if (!pay) throw new Error(`${directory.shown(to)} has no pay keys yet: they open their Wallet once`);
    await purse.pay(id, pay, amount, to);
    return { to };
  }

  // A token's BALANCE line: its total and where it is (private chains · on the id).
  const whereOf = v => [v.private !== "0" ? `${shown(v.private, v.decimals)} private (${v.chains} chain${v.chains === 1 ? "" : "s"})` : null, v.onId !== "0" ? `${shown(v.onId, v.decimals)} on your id` : null].filter(Boolean).join(" · ");

  // THE LIST: a card per token (its balance), opening its page.
  function card(id, me) {
    const box = h("a", { className: "tok", href: `#/wallet/t/${id}` }, h("span", { className: "note", textContent: "reading…" }));
    viewOf(id, me).then(
      v => box.replaceChildren(h("div", { className: "top" }, h("span", { className: "name", textContent: v.name || "a token" }), h("span", { className: "bal", textContent: shown(v.total, v.decimals) }), h("span", { className: "note", textContent: whereOf(v) || "nothing held" }))),
      e => box.replaceChildren(h("span", { className: "said", textContent: `${id.slice(0, 12)}… could not be read: ${e.message ?? e}` })),
    );
    return box;
  }

  // WHEN, from wall-clock ms: "just now", "5 min ago", "3 h ago", or the date.
  const when = at => {
    if (!at) return "—";
    const s = (Date.now() - at) / 1000;
    return s < 60 ? "just now" : s < 3600 ? `${Math.floor(s / 60)} min ago` : s < 86400 ? `${Math.floor(s / 3600)} h ago` : new Date(at).toLocaleDateString();
  };
  // A payment's STATUS as a chip (its reason, when waiting, in its title).
  const chip = st => {
    if (!st) return h("span", { className: "chip settling", textContent: "not seen yet" });
    if (st.stage === "accepted") return h("span", { className: "chip accepted", textContent: `accepted ✓ ${st.signed}`, title: `${st.signed} witness signature(s)` });
    if (st.stage === "witnessing") return h("span", { className: "chip witnessing", textContent: "waiting for witnesses", title: st.why ?? "" });
    return h("span", { className: "chip settling", textContent: "settling", title: "inside its margin: final in a few beacon steps" });
  };

  // A TOKEN's PAGE: its balance, send, move to private, and its HISTORY with each payment's status.
  function detail(id, me) {
    const out = h("div", { className: "wl" }, h("a", { href: "#/wallet", className: "note", textContent: "← Tokens" }));
    const head = h("div", { className: "tok" }, h("span", { className: "note", textContent: "reading…" }));
    const said = h("span", { className: "said" });
    const hist = h("div", { className: "tok" }, h("h3", { textContent: "History" }));
    out.append(head, hist);
    viewOf(id, me).then(
      v => {
        const to = h("input", { placeholder: "name#abc123 or did:craftec:…", size: 28 });
        const amount = h("input", { placeholder: "amount", size: 10 });
        const go = h("button", { type: "submit", textContent: "Send" });
        const f = h("form", {}, h("label", {}, "To", to), h("label", {}, "Amount", amount), go);
        f.onsubmit = async ev => {
          ev.preventDefault();
          go.disabled = true;
          said.textContent = "";
          try {
            const r = await send(id, to.value, amount.value, v.decimals);
            status.textContent = `Sent ${amount.value.trim()} ${v.name} to ${directory.shown(r.to)}, privately.`;
            draw();
          } catch (e) {
            said.textContent = e.message ?? String(e);
            go.disabled = false;
          }
        };
        // MOVE TO PRIVATE what is spendable on the id (its genesis, a payment from before, unstaked stake).
        const move = v.onId !== "0" ? h("button", { type: "button", textContent: `Move ${shown(v.onId, v.decimals)} to private` }) : null;
        if (move)
          move.onclick = async () => {
            move.disabled = true;
            said.textContent = "";
            try {
              await purse.shield(me, id, v.onId);
              status.textContent = `Moved ${shown(v.onId, v.decimals)} ${v.name} to private: taken in a moment.`;
              receive();
              draw();
            } catch (e) {
              said.textContent = e.message ?? String(e);
              move.disabled = false;
            }
          };
        const made = v.creator === hexOf(me.didBytes);
        head.replaceChildren(
          h("div", { className: "top" }, h("span", { className: "name", textContent: v.name || "a token" }), made ? h("span", { className: "note", textContent: `you made it · supply ${shown(v.supply, v.decimals)}` }) : null),
          h("span", { className: "big", textContent: shown(v.total, v.decimals) }),
          h("span", { className: "note", textContent: whereOf(v) || "nothing held" }),
          ...[move, f, said].filter(Boolean),
          h("span", { className: "note", textContent: `token ${id}` }),
        );
        // THE HISTORY, newest first (the last 40): each line's status read from its chain and its witnesses.
        const items = purse.history(id).slice(0, 40);
        if (!items.length) return hist.append(h("p", { className: "note", textContent: "Nothing yet." }));
        const tb = h("tbody");
        hist.append(h("table", {}, h("thead", {}, h("tr", {}, ...["When", "What", "Amount", "Status"].map(t => h("th", { textContent: t })))), tb));
        for (const it of items) {
          const what = it.kind === "send" ? `Sent to ${it.to ? directory.shown(it.to) : "someone"}` : it.kind === "shield" ? "Moved to private" : it.kind === "sale" ? "A sale of your edition" : "Received (private)";
          const out = it.kind === "send";
          const amt = h("td", { className: `num ${out ? "out" : "in"}`, textContent: "…" });
          const st = h("td", {}, h("span", { className: "note", textContent: "…" }));
          tb.append(h("tr", {}, h("td", { textContent: when(it.at) }), h("td", { textContent: what }), amt, st));
          purse.status(it.a).then(
            s => {
              amt.textContent = s ? `${out ? "−" : "+"}${shown(s.amount, v.decimals)}` : "—";
              st.replaceChildren(chip(s));
            },
            e => st.replaceChildren(h("span", { className: "said", textContent: e.message ?? String(e) })),
          );
        }
      },
      e => head.replaceChildren(h("span", { className: "said", textContent: `${id.slice(0, 12)}… could not be read: ${e.message ?? e}` })),
    );
    return out;
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
    out.append(...(rows.length ? rows.map(r => card(r.key, me)) : [h("p", { className: "note", textContent: "No tokens yet: be sent one, or add one by its id." })]), addForm, said);
    return out;
  }

  const status = h("p", { className: "note", role: "status" });
  const body = h("div");
  root.append(status, body);
  const me = await (await ctx.require("auth")).check();
  purse.ensureCard(me).catch(e => ctx.log("wallet", { what: `the card's pay keys: ${e.message ?? e}` }));
  // The VIEW: a token's page (`#/wallet/t/<id>`) or the list.
  const opened = /^t\/([0-9a-f]{64})$/.exec(ctx.sub ?? "")?.[1] ?? null;
  const draw = async () => body.replaceChildren(opened ? detail(opened, me) : await list(me));
  purse.onChange(() => el.isConnected && draw());
  await draw();
  table.onChange(() => el.isConnected && draw());
  // RECEIVE now, and again EACH TIME THE BEACON ADVANCES while the Wallet is open: a payment that arrives with the page
  // open is found (it was found only on opening), one waiting out its margin or its witnesses taken once it may be.
  // (Overlapping runs skipped; the subscription ends with the page.)
  const beaconSvc = await ctx.require("beacon");
  let busy = false;
  let waited = false;
  const receive = async () => {
    if (!el.isConnected) return stop();
    if (busy) return;
    busy = true;
    try {
      const r = await purse.receiveAll({ onToken: async t => table.rows().some(r => r.key === t) || (await table.put(t, JSON.stringify({ name: "" }))) });
      if (r.got) draw();
      else if (opened && waited) draw(); // a status moved on
      const waiting = (r.pending ?? 0) + (r.unwitnessed ?? 0);
      if (waiting) status.textContent = r.unwitnessed ? `${waiting} payment(s) to you waiting for their witnesses: taken once they sign.` : `${waiting} payment(s) to you on the way: taken in a moment.`;
      else if (waited || r.got) status.textContent = r.got ? "Received." : "";
      waited = waiting > 0;
    } catch (e) {
      ctx.log("wallet", { what: `receiving: ${e.message ?? e}` });
    } finally {
      busy = false;
    }
  };
  const stop = beaconSvc.onHeight(receive);
  receive();
}
