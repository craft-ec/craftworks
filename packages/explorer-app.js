// NETWORK, an app (`#/network`; owner 10-11: "more transparent on what is happening … like a block explorer"):
// every public record the token machinery runs on, as tables — the clock, the epoch statements and their witnesses,
// the witness lot's entrants, the witness queue with each entry's signatures and status, the registries — yours
// marked (what your nodes do is the Nodes app's, your money the Wallet's). Read-only; every value is the core's reading of a public contract (`clock`, `statement_status`,
// `claims_read`, `entry_status`, …). Refreshed as the beacon advances.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  el.innerHTML = `<style>@layer apps {
    .ex { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); max-width: 64rem; }
    .ex h2, .ex h3, .ex p { margin: 0; }
    .ex h3 { font-size: 1rem; margin-top: var(--cw-space-2); }
    .ex .note { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .ex table { border-collapse: collapse; width: 100%; font-size: var(--cw-text-sm); }
    .ex th, .ex td { text-align: left; padding: 4px 10px 4px 0; border-bottom: 1px solid var(--cw-line); vertical-align: top; white-space: nowrap; }
    .ex td.num, .ex th.num { text-align: right; font-variant-numeric: tabular-nums; }
    .ex code { font-size: var(--cw-text-sm); }
    .ex .ok { color: #2aa876; } .ex .wait { color: #b38700; } .ex .bad { color: var(--cw-danger); }
    .ex .scroll { overflow-x: auto; }
  }</style><div class="ex"><h2>🌐 Network</h2><p class="note" data-x="said"></p><div data-x="body"></div></div>`;
  const said = el.querySelector('[data-x="said"]');
  const body = el.querySelector('[data-x="body"]');
  const tokens = await ctx.require("tokens");
  if (!tokens.on) return (said.textContent = "No beacon on this site.");
  const [node, index, beaconSvc, statement, keeper, auth] = await Promise.all(["node", "index", "beacon", "statement", "keeper", "auth"].map(n => ctx.require(n)));
  const { core } = node;
  const Core = node.glue.CraftworksCore;
  const { run, lot, bytes, hexOf } = tokens;
  const me = await auth.check();
  const myDid = hexOf(me.didBytes);
  const myKey = me.member.toLowerCase();
  const clk = Core.clock();
  const short = h => (h ? `${h.slice(0, 8)}…` : "—");
  const craft = n => (Number(BigInt(n) / 1000000n) / 1000).toLocaleString(undefined, { maximumFractionDigits: 3 });
  const cell = (t, cls) => Object.assign(document.createElement("td"), { textContent: t ?? "—", className: cls ?? "" });
  const code = t => {
    const td = document.createElement("td");
    td.append(Object.assign(document.createElement("code"), { textContent: t }));
    return td;
  };
  const table = (head, rows) => {
    const t = document.createElement("table");
    t.createTHead().insertRow().append(...head.map(([x, cls]) => Object.assign(document.createElement("th"), { textContent: x, className: cls ?? "" })));
    const tb = t.createTBody();
    for (const r of rows) tb.insertRow().append(...r);
    if (!rows.length) tb.insertRow().append(Object.assign(cell("none"), { colSpan: head.length, className: "note" }));
    const w = Object.assign(document.createElement("div"), { className: "scroll" });
    w.append(t);
    return w;
  };
  const section = (title, note, node_) => {
    const s = document.createElement("section");
    s.append(Object.assign(document.createElement("h3"), { textContent: title }), Object.assign(document.createElement("p"), { className: "note", textContent: note }), node_);
    return s;
  };
  // A contract's state as the node gives it (null: not there).
  const stateOf = async idBytes => {
    const idHex = hexOf(idBytes);
    await tokens.read(idHex);
    return core.take_got(idHex) ?? null;
  };

  async function draw() {
    const h = beaconSvc.status().height;
    if (h == null) return (said.textContent = "Reading the beacon…");
    const e = Math.floor(h / clk.standing);
    const c = Math.floor(h / clk.claims);
    const parts = [];

    // CLOCK
    parts.push(section("Clock", "One beacon for the network: every stamp, epoch and lot is read from it.", table(
      [["Height", "num"], ["Standing epoch", "num"], ["Claims epoch", "num"], ["Next claims epoch at", "num"], ["Its entrants close at", "num"]],
      [[cell(String(h), "num"), cell(String(e), "num"), cell(String(c), "num"), cell(String((c + 1) * clk.claims), "num"), cell(String((c + 1) * clk.claims - clk.lead), "num")]],
    )));

    // STATEMENTS
    const st = [];
    for (let x = e; x >= Math.max(0, e - 3); x--) {
      const posted = (await index.bag.read(Core.statement_bag_address(x), "reading an epoch's statement")).map(p => Core.statement_posted_read(p)).filter(p => p?.epoch === x);
      const iss = await statement.issuance(x).catch(() => null);
      if (!posted.length) st.push([cell(String(x), "num"), cell("not posted yet"), cell(""), cell(iss ? String(iss.keepers) : "—", "num"), cell(iss ? craft(iss.total) : "—", "num")]);
      for (const p of posted) {
        const s = await run(() => core.statement_status(tokens.code, tokens.beacon, tokens.idlogCode, lot.pool, lot.bag, lot.beacon, x, p.root)).catch(err => ({ waiting: err.message ?? String(err) }));
        st.push([cell(String(x), "num"), code(short(p.root)), s?.accepted != null ? cell(`accepted (${s.accepted})`, "ok") : cell(s?.waiting ?? "—", "wait"), cell(iss ? String(iss.keepers) : "—", "num"), cell(iss ? craft(iss.total) : "—", "num")]);
      }
    }
    parts.push(section("Statements", "Each epoch's root of who kept and earned; value once its witnesses sign it.", table([["Epoch", "num"], ["Root"], ["Witnesses"], ["Keepers paid", "num"], ["$CRAFT minted", "num"]], st)));

    // WITNESS LOT
    const lotRows = [];
    for (const x of [c, c + 1]) {
      const reg = await stateOf(Core.claims_registry_id(tokens.code, tokens.beacon, tokens.idlogCode, x));
      const claims = reg ? Core.claims_read(reg) : [];
      if (!claims.length) lotRows.push([cell(String(x), "num"), cell("no entrants: the baseline witnesses alone", "note"), cell(""), cell(""), cell("")]);
      for (const k of claims) lotRows.push([cell(String(x), "num"), cell(k.owner === myDid ? "you" : short(k.owner)), cell(craft(k.amount), "num"), code(short(k.key)), cell(k.key === myKey ? "this node" : "")]);
    }
    parts.push(section("Witness lot", "Stakers who entered each claims epoch: each entry's witnesses are drawn from them, by stake.", table([["Claims epoch", "num"], ["Account"], ["$CRAFT staked", "num"], ["Witness key"], [""]], lotRows)));

    // WITNESS QUEUE
    const qRows = [];
    for (const x of [e, e - 1].filter(y => y >= 0)) {
      const items = await index.bag.chain(Core.witness_queue_address, x, "reading the witness queue");
      for (const it of items.slice(-25).reverse()) {
        const q = Core.queued_read(it);
        if (!q) continue;
        const s = await run(() => core.entry_status(tokens.code, tokens.beacon, tokens.idlogCode, lot.pool, lot.bag, lot.beacon, it)).catch(err => ({ missing: err.message ?? String(err) }));
        const status = s?.missing ? cell(s.missing, "bad") : s?.accepted != null ? cell(`accepted (${s.accepted})`, "ok") : cell(s?.waiting ?? "—", "wait");
        qRows.push([cell(String(x), "num"), code(short(q.token)), cell(q.owner === myDid ? "you" : short(q.owner)), cell(String(q.seq), "num"), cell(s?.kind ?? "—"), cell(s?.stamp != null ? String(s.stamp) : "—", "num"), cell(s?.signatures != null ? String(s.signatures) : "—", "num"), status]);
      }
    }
    parts.push(section("Witness queue", "Every new token entry, waiting for its drawn witnesses (this epoch and the last, newest first).", table([["Epoch", "num"], ["Token"], ["Chain owner"], ["Seq", "num"], ["Entry"], ["Stamp", "num"], ["Signatures", "num"], ["Status"]], qRows)));

    // REGISTRIES
    const net = await keeper.network?.(e).catch(() => null);
    parts.push(section("Registries", "What the network keeps, and who keeps it (this epoch and the last).", table([["Files listed", "num"], ["Bytes", "num"], ["Keepers", "num"]], net ? [[cell(String(net.files), "num"), cell(`${(net.bytes / 1e6).toFixed(1)} MB`, "num"), cell(String(net.keepers), "num")]] : [])));

    said.textContent = `At height ${h}.`;
    body.replaceChildren(...parts);
  }

  let drawing = false;
  let drawnAt = -Infinity;
  const refresh = async h => {
    if (drawing || !el.isConnected || (h != null && h - drawnAt < 8)) return;
    drawing = true;
    drawnAt = h ?? drawnAt;
    await draw().catch(e => (said.textContent = `Could not read: ${e?.message ?? e}`));
    drawing = false;
  };
  beaconSvc.onHeight(refresh);
  said.textContent = "Reading…";
  await refresh(beaconSvc.status().height);
}
