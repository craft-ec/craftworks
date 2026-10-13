// NODES, an app (`#/nodes`; owner 10-11: "my machines and the work they do"): the account's nodes, each a row read
// from the PUBLIC records (its standing record, the witness lot) — what it holds and proves for the network, which one
// holds the account's witness seat — the files this node keeps, and the data the account used. Money is the Wallet's;
// the network as a whole is the Network app's.
export async function mount(ctx, el) {
  el.innerHTML = `<style>@layer apps {
    .nd { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); max-width: 64rem; }
    .nd h2, .nd h3, .nd p { margin: 0; }
    .nd h3 { font-size: 1rem; margin-top: var(--cw-space-2); }
    .nd .note { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .nd table { border-collapse: collapse; width: 100%; font-size: var(--cw-text-sm); }
    .nd th, .nd td { text-align: left; padding: 4px 10px 4px 0; border-bottom: 1px solid var(--cw-line); vertical-align: top; white-space: nowrap; }
    .nd td.num, .nd th.num { text-align: right; font-variant-numeric: tabular-nums; }
    .nd tfoot td { font-weight: 600; border-bottom: 0; }
    .nd .ok { color: #2aa876; } .nd .no { color: var(--cw-muted); }
    .nd .scroll { overflow-x: auto; }
    .nd .row { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    .nd .bar { display: grid; width: 320px; max-width: 100%; height: 18px; border-radius: 6px; overflow: hidden; border: 1px solid var(--cw-line); }
    .nd .bar .used { background: #2aa876; } .nd .bar .free { background: var(--cw-line); }
  }</style><div class="nd"><h2>🖥️ Nodes</h2><div data-x="body"></div></div>`;
  const body = el.querySelector('[data-x="body"]');
  const PAGES = [["", "Nodes"], ["keeper", "Keeper"], ["data", "Data used"]];
  const page = PAGES.some(([k]) => k === (ctx.sub ?? "")) ? (ctx.sub ?? "") : "";
  ctx.actions[ctx.route] = PAGES.map(([k, label]) => ({ label, href: k ? `#/nodes/${k}` : "#/nodes", on: k === page }));
  dispatchEvent(new CustomEvent("craftworks:actions"));
  const size = b => (b >= 1e9 ? `${(b / 1e9).toFixed(2)} GB` : b >= 1e6 ? `${(b / 1e6).toFixed(1)} MB` : b >= 1e3 ? `${Math.round(b / 1e3)} KB` : `${b} B`);
  const craft = n => (Number(BigInt(n ?? "0") / 1000000n) / 1000).toLocaleString(undefined, { maximumFractionDigits: 3 });
  const cell = (t, cls) => Object.assign(document.createElement("td"), { textContent: t ?? "—", className: cls ?? "" });
  const table = (head, rows, foot) => {
    const t = document.createElement("table");
    t.createTHead().insertRow().append(...head.map(([x, cls]) => Object.assign(document.createElement("th"), { textContent: x, className: cls ?? "" })));
    const tb = t.createTBody();
    for (const r of rows) tb.insertRow().append(...r);
    if (!rows.length) tb.insertRow().append(Object.assign(cell("none"), { colSpan: head.length, className: "note" }));
    if (foot) t.createTFoot().insertRow().append(...foot);
    const w = Object.assign(document.createElement("div"), { className: "scroll" });
    w.append(t);
    return w;
  };
  const section = (title, note, n) => {
    const s = document.createElement("section");
    s.append(Object.assign(document.createElement("h3"), { textContent: title }), Object.assign(document.createElement("p"), { className: "note", textContent: note }), n);
    return s;
  };

  // NODES: the account row, then one row per node.
  async function nodes() {
    const tokens = await ctx.require("tokens");
    if (!tokens.on) return [Object.assign(document.createElement("p"), { className: "note", textContent: "No beacon on this site." })];
    const [node, auth, membership, beaconSvc, statement, standingCode, beaconCode] = await Promise.all(["node", "auth", "membership", "beacon", "statement", "standing-wasm", "beacon-wasm"].map(n => ctx.require(n)));
    const { core } = node;
    const Core = node.glue.CraftworksCore;
    const { bytes, hexOf } = tokens;
    const me = await auth.check();
    const did = hexOf(me.didBytes);
    const h = beaconSvc.status().height ?? 0;
    const e = Math.floor(h / Core.standing_epoch());
    const c = Math.floor(h / Core.clock().claims);
    const stateOf = async idBytes => {
      const id = hexOf(idBytes);
      await tokens.read(id);
      return core.take_got(id) ?? null;
    };
    // The witness SEAT: the account's place among the VALID STAKERS of this claims epoch (and the next, as far as its
    // claims are accepted yet), naming one node's key.
    const seat = {};
    for (const x of [c, c + 1]) {
      const v = await ctx.optional("nodes-app: tokens.run", tokens.run(() => core.stakers(tokens.code, tokens.beacon, tokens.idlogCode, tokens.lot.bag, tokens.lot.beacon, x)), () => []);
      const mine = (v ?? []).find(k => k.owner === did);
      if (mine) seat[x] = mine;
    }
    // ACCOUNT: the last two epochs' statement lines.
    const acct = [];
    for (const x of [e - 1, e - 2].filter(y => y >= 0)) {
      const p = await ctx.optional("nodes-app: statement.paid", statement.paid(x, did), () => null);
      acct.push([cell(String(x), "num"), p && p.keeper !== "0" ? cell("✓", "ok") : cell("✗", "no"), cell(p ? craft(p.keeper) : "0", "num"), cell(p ? craft(p.witness) : "0", "num")]);
    }
    const seats = [c, c + 1].map(x => (seat[x] ? `claims epoch ${x}: node ${seat[x].key.slice(0, 8)}… · ${craft(seat[x].amount)} staked` : `claims epoch ${x}: none`)).join(" · ");
    // NODES: each node's standing record (its key names it).
    const list = (await membership.nodes()) ?? [{ index: 0, key: me.member, me: true }];
    const limits = await ctx.optional("nodes-app: limits", (await ctx.require("keep")).limits(list.map(n => n.key)), () => ({}));
    const rows = [];
    for (const n of list) {
      const key = n.key.toLowerCase();
      const rec = await stateOf(Core.keeper_record_id(standingCode, beaconCode, tokens.beacon, bytes(key)));
      const eps = rec ? Core.standing_read(rec) : [];
      const now = eps.find(x => x.epoch === e);
      const last = eps.find(x => x.epoch === e - 1);
      const lastSeen = eps.map(x => x.last).find(x => x != null);
      const holds = Object.values(seat).some(s => s.key === key);
      rows.push([cell(`node ${n.index}${n.me ? " (this one)" : ""}`), cell(`${key.slice(0, 8)}…`), cell(limits[key] != null ? size(limits[key]) : "—", "num"), cell(now ? String(now.pieces) : "—", "num"), cell(now ? size(now.bytes) : "—", "num"), cell(now ? String(now.answers) : "0", "num"), cell(last ? String(last.answers) : "0", "num"), cell(lastSeen != null ? String(lastSeen) : "never", "num"), holds ? cell("✓", "ok") : cell("")]);
    }
    return [
      section("Account", `Its statement lines (all its nodes together). Witness seat — ${seats}.`, table([["Epoch", "num"], ["Keeper"], ["Earned as keeper ($CRAFT, staked)", "num"], ["Earned as witness ($CRAFT)", "num"]], acct)),
      section("Nodes", `Each node's own standing record: what it committed and the challenges it answered (epoch ${e} and ${e - 1}).`, table([["Node"], ["Key"], ["Storage limit", "num"], ["Pieces held", "num"], ["Bytes", "num"], [`Answered ${e}`, "num"], [`Answered ${e - 1}`, "num"], ["Last answer at", "num"], ["Witness seat"]], rows)),
    ];
  }

  // KEEPER: what this node keeps for others, within its limit.
  async function keeper() {
    const [keep, items] = await Promise.all(["keep", "items"].map(n => ctx.require(n)));
    const k = await keep.kept();
    const LIMITS = [["0", "Off"], ["256000000", "256 MB"], ["512000000", "512 MB"], ["1000000000", "1 GB"], ["5000000000", "5 GB"]];
    const limit = Object.assign(document.createElement("select"), { title: "How much this node keeps" });
    limit.append(...LIMITS.map(([v, t]) => Object.assign(document.createElement("option"), { value: v, textContent: t })));
    limit.value = String(LIMITS.find(([v]) => Number(v) === k.limit)?.[0] ?? "512000000");
    limit.onchange = async () => (await keep.setLimit(Number(limit.value)), show());
    const pct = k.limit ? Math.min(100, Math.round((100 * k.total) / k.limit)) : 0;
    const bar = Object.assign(document.createElement("div"), { className: "bar" });
    bar.style.gridTemplateColumns = `${pct}fr ${100 - pct}fr`;
    bar.append(Object.assign(document.createElement("span"), { className: "used" }), Object.assign(document.createElement("span"), { className: "free" }));
    const row = Object.assign(document.createElement("div"), { className: "row" });
    row.append(limit, bar, Object.assign(document.createElement("span"), { className: "note", textContent: `${size(k.total)} of ${size(k.limit)} · ${k.files.length} file(s)` }));
    const ago = t => {
      if (!t) return "not yet";
      const m = Math.round((Date.now() - t) / 60000);
      return m < 1 ? "just now" : m < 60 ? `${m} min ago` : m < 2880 ? `${Math.round(m / 60)} h ago` : `${Math.round(m / 1440)} days ago`;
    };
    const rows = k.files.map(f => {
      const what = document.createElement("td");
      what.append(f.item ? Object.assign(document.createElement("a"), { href: items.pageOf(f.item, f.kind ?? "video"), textContent: f.title || "an item" }) : `${f.root.slice(0, 8)}…`);
      const who = cell("…", "num");
      ctx.optional("nodes-app: keep.keepers", keep.keepers(f.root).then(ds => (who.textContent = String(ds.length))), () => who.textContent = "—");
      return [what, cell(size(f.size), "num"), cell(ago(f.used)), cell(ago(f.at)), cell(f.error ? f.error : f.gens ? `${f.whole}/${f.gens} whole${f.missing ? `, ${f.missing} unanswered` : ""}` : "—"), who];
    });
    return [row, section("Files this node keeps", "Re-read so the network does not drop them; assigned files join its holdings.", table([["What"], ["Size", "num"], ["Played"], ["Last read"], ["Health"], ["Keepers", "num"]], rows))];
  }

  // DATA USED: what the account watched, listened to and opened, and the data it took (its `usage` table).
  async function data() {
    const [usage, items] = await Promise.all(["usage", "items"].map(n => ctx.require(n)));
    const months = [0, 1, 2].map(n => {
      const d = new Date();
      d.setUTCDate(1);
      d.setUTCMonth(d.getUTCMonth() - n);
      return d.toISOString().slice(0, 7);
    });
    const pick = Object.assign(document.createElement("select"), { title: "Month" });
    pick.append(...months.map(m => Object.assign(document.createElement("option"), { value: m, textContent: m })));
    pick.value = data.month ?? months[0];
    pick.onchange = () => ((data.month = pick.value), show());
    const clock = s => (s >= 3600 ? `${Math.floor(s / 3600)} h ${Math.round((s % 3600) / 60)} min` : s >= 60 ? `${Math.floor(s / 60)} min ${Math.round(s % 60)} s` : `${Math.round(s)} s`);
    const rows = await usage.month(pick.value);
    const sum = k => rows.reduce((n, r) => n + (r[k] ?? 0), 0);
    const trs = rows.map(r => {
      const what = document.createElement("td");
      what.append(!r.ref.startsWith("file:") ? Object.assign(document.createElement("a"), { href: items.pageOf(r.ref, r.kind), textContent: r.title || r.kind || "an item" }) : `a file (${r.ref.slice(5, 13)}…)`);
      return [what, cell(r.s ? clock(r.s) : "—", "num"), cell(r.b ? size(r.b) : "—", "num"), cell(r.n ? String(r.n) : "—", "num")];
    });
    return [pick, section("Data used", "What the account watched, listened to and opened, and the data it took.", table([["What"], ["Time", "num"], ["Data", "num"], ["Opened", "num"]], trs, [cell("Total"), cell(clock(sum("s")), "num"), cell(size(sum("b")), "num"), cell(String(sum("n")), "num")]))];
  }

  async function show() {
    body.replaceChildren(Object.assign(document.createElement("p"), { className: "note", textContent: "Reading…" }));
    const parts = await (page === "keeper" ? keeper() : page === "data" ? data() : nodes()).catch(e => [Object.assign(document.createElement("p"), { className: "note", textContent: `Could not read: ${e?.message ?? e}` })]);
    body.replaceChildren(...parts);
  }
  await show();
}
