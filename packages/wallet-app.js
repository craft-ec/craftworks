// WALLET, an app (`#/wallet`): this account's TOKENS (TOKEN §7c) — each with its balance; SEND to anyone by their id
// (signed by this node, the account's device), and RECEIVE what others sent (a note in this account's inbox names the
// sender: its credit is taken from their chain once its margin has passed — a worked credit, no signature; while one
// is inside its margin, received again each time the beacon advances). No token is MADE here (owner 10-10: a page that
// made them would let anyone flood the network with tokens) — issuers make theirs with their tools. The tokens shown
// are the account's table `tokens` (token id → { name }), on every device of the account. UI only: every entry is the
// core's (`wallet_*`), every step `tokens`' run.
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

  const viewOf = async (id, me) => run(() => core.wallet_view(code, beacon, idlogCode, bytes(id), me.didBytes));

  // RECEIVE: the senders each "token" note in this account's inbox names, by token; each token's credits taken. New
  // tokens named there are added to the table.
  async function receiveAll(me) {
    const by = new Map();
    for (const it of await index.inbox()) {
      if (it.kind !== "token" || !/^[0-9a-f]{64}$/.test(it.token ?? "") || !it.from) continue;
      if (!by.has(it.token)) by.set(it.token, new Set());
      by.get(it.token).add(it.from);
    }
    let got = 0;
    let pending = 0;
    for (const [id, from] of by) {
      if (!table.rows().some(r => r.key === id)) {
        const v = await viewOf(id, me).catch(() => null);
        if (!v) continue;
        await table.put(id, JSON.stringify({ name: v.name }));
      }
      // Each token on its own: one that cannot be received (another clock, a chain that does not read) stops no other.
      try {
        const senders = [...from].map(d => glue.did_bytes(d));
        const { member, creds } = await tokens.device();
        const at = await now();
        const r = await run(sigs => core.wallet_receive(code, beacon, idlogCode, bytes(id), me.didBytes, senders, at, member, creds, sigs));
        await putAll(r.puts);
        if (r.puts?.length) got++;
        pending += r.pending ?? 0;
      } catch (e) {
        ctx.log("wallet", { what: `receiving ${id.slice(0, 12)}…: ${e.message ?? e}` });
      }
    }
    return { got, pending };
  }

  async function send(id, me, toText, amountText, dec) {
    const to = await conversation.person(toText.trim());
    if (!to) throw new Error("no such person: an id (did:craftec:…) or name#abc123 of someone you know");
    const amount = units(amountText, dec);
    if (amount === "0") throw new Error("send more than nothing");
    const { member, creds } = await tokens.device();
    const at = await now();
    const s = await run(sigs => core.wallet_send(code, beacon, idlogCode, bytes(id), me.didBytes, glue.did_bytes(to), amount, at, member, creds, sigs));
    await putAll(s.puts);
    // The note: who sent which token — their wallet takes the credit from this chain.
    let noted = true;
    await index.send(to, { kind: "token", from: me.did, token: id }).catch(e => {
      noted = false;
      ctx.log("wallet", { what: `sent; the note to ${directory.shown(to)} did not go: ${e.message ?? e}` });
    });
    return { to, noted };
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
            status.textContent = `Sent ${amount.value.trim()} ${v.name} to ${directory.shown(r.to)}${r.noted ? "" : " (they were not told: tell them to open their wallet)"}.`;
            box.replaceWith(row(id, me));
          } catch (e) {
            said.textContent = e.message ?? String(e);
            go.disabled = false;
          }
        };
        const mine = v.creator === hexOf(me.didBytes);
        box.replaceChildren(
          h("div", { className: "top" }, h("span", { className: "name", textContent: v.name || "a token" }), h("span", { className: "bal", textContent: shown(v.begun || !mine ? v.balance : v.supply, v.decimals) }), mine ? h("span", { className: "note", textContent: `you made it · supply ${shown(v.supply, v.decimals)}` }) : null),
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
  const draw = async () => body.replaceChildren(await list(me));
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
