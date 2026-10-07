// USAGE, an app (`#/usage`): this person's month — what they watched, listened to and opened, and the data it took
// (the `usage` capability: their own table, nobody else's). The first step of the rewards (docs/REWARDS.md): what a
// subscription would be split by.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  el.innerHTML = `<style>@layer apps {
    .us { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); justify-items: start; }
    .us h2, .us .note { margin: 0; }
    .us .note { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .us table { border-collapse: collapse; }
    .us th, .us td { text-align: left; padding: 6px 14px 6px 0; border-bottom: 1px solid var(--cw-line); }
    .us td.num { text-align: right; font-variant-numeric: tabular-nums; }
    .us tfoot td { font-weight: 600; border-bottom: 0; }
    .us input.amount { width: 6em; margin: 0 4px; }
    .us .level { margin: 0; font-weight: 600; }
    .us .us-bar { display: grid; gap: 0; width: 460px; max-width: 100%; height: 24px; border-radius: 6px; overflow: hidden; font-size: var(--cw-text-sm); border: 1px solid var(--cw-line); }
    .us .us-bar > span { display: flex; min-width: 0; margin: 0; padding: 0; align-items: center; justify-content: center; color: #fff; white-space: nowrap; overflow: hidden; }
    .us .us-bar .c { background: #2f7de1; } .us .us-bar .k { background: #2aa876; } .us .us-bar .n { background: #8a8f98; }
    .us .us-bar .used { background: #2aa876; } .us .us-bar .free { background: var(--cw-line); }
    .us .us-row { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  }</style><div class="us"><h2>📊 Usage</h2></div>`;
  const root = el.querySelector(".us");
  const usage = await ctx.require("usage");
  const items = await ctx.require("items");
  // FIVE PAGES (owner 10-07), the header's tabs: Summary, Usage, Kept, Contribution, Reward.
  const PAGES = [["summary", "Summary"], ["use", "Usage"], ["kept", "Kept"], ["give", "Contribution"], ["reward", "Reward"]];
  const page = PAGES.some(([k]) => k === (ctx.sub ?? "")) ? ctx.sub : "summary";
  ctx.actions[ctx.route] = PAGES.map(([k, label]) => ({ label, href: `#/usage/${k}`, on: k === page }));
  dispatchEvent(new CustomEvent("craftworks:actions"));
  root.querySelector("h2").textContent = `📊 ${PAGES.find(([k]) => k === page)[1]}`;
  const months = [0, 1, 2].map(n => {
    const d = new Date();
    d.setUTCDate(1);
    d.setUTCMonth(d.getUTCMonth() - n);
    return d.toISOString().slice(0, 7);
  });
  const pick = Object.assign(document.createElement("select"), { title: "Month" });
  pick.append(...months.map(m => Object.assign(document.createElement("option"), { value: m, textContent: m })));
  const out = document.createElement("div");
  const clock = s => (s >= 3600 ? `${Math.floor(s / 3600)} h ${Math.round((s % 3600) / 60)} min` : s >= 60 ? `${Math.floor(s / 60)} min ${Math.round(s % 60)} s` : `${Math.round(s)} s`);
  const size = b => (b >= 1e9 ? `${(b / 1e9).toFixed(2)} GB` : b >= 1e6 ? `${(b / 1e6).toFixed(1)} MB` : b >= 1e3 ? `${Math.round(b / 1e3)} KB` : `${b} B`);
  const td = (t, cls) => Object.assign(document.createElement("td"), { textContent: t, className: cls ?? "" });
  const draw = async () => {
    out.textContent = "Reading…";
    const rows = await usage.month(pick.value).catch(e => (out.textContent = `Could not read: ${e?.message ?? e}`, null));
    if (!rows) return;
    if (!rows.length) return (out.textContent = "Nothing used this month yet.");
    const t = document.createElement("table");
    const head = t.createTHead().insertRow();
    for (const x of ["What", "Time", "Data", "Opened"]) head.append(Object.assign(document.createElement("th"), { textContent: x }));
    const body = t.createTBody();
    for (const r of rows) {
      const tr = body.insertRow();
      const what = document.createElement("td");
      const isItem = !r.ref.startsWith("file:");
      what.append(isItem ? Object.assign(document.createElement("a"), { href: items.pageOf(r.ref, r.kind), textContent: r.title || r.kind || "an item" }) : `a file (${r.ref.slice(5, 13)}…)`);
      tr.append(what, td(r.s ? clock(r.s) : "—", "num"), td(r.b ? size(r.b) : "—", "num"), td(r.n ? String(r.n) : "—", "num"));
    }
    const sum = k => rows.reduce((n, r) => n + (r[k] ?? 0), 0);
    const foot = t.createTFoot().insertRow();
    foot.append(td("Total"), td(clock(sum("s")), "num"), td(size(sum("b")), "num"), td(String(sum("n")), "num"));
    out.replaceChildren(t);
  };
  const note = document.createDocumentFragment();
  // YOUR CONTRIBUTION (rewards §2-§3, SHADOW MODE): where an amount you give this month would go — any amount (owner
  // 10-07: contribution, not subscription); creators by your time on their items,
  // carriers by the data their kept files brought you, the network the rest. Nothing is charged or paid yet.
  const directory = await ctx.require("directory");
  const fh = Object.assign(document.createElement("h3"), { textContent: "Your contributions (preview)" });
  const amount = Object.assign(document.createElement("input"), { type: "number", min: "0", step: "0.00000001", value: "10", title: "Tokens (8 decimals)", className: "amount" });
  // The token's 8 decimals, trailing zeros dropped: 0.33333333, 1, 4.2.
  const tok = n => Number(n).toFixed(8).replace(/\.?0+$/, "");
  // OVER: a week or a month (owner 10-07), a month unless chosen.
  const days = Object.assign(document.createElement("select"), { title: "Spread over" });
  days.append(...[["7", "a week (7 days)"], ["30", "a month (30 days)"]].map(([v, t]) => Object.assign(document.createElement("option"), { value: v, textContent: t, selected: v === "30" })));
  const giveBtn = Object.assign(document.createElement("button"), { type: "button", textContent: "Give (preview)" });
  const amountLine = Object.assign(document.createElement("div"), { className: "us-row" });
  amountLine.append(amount, days, giveBtn);
  const given = document.createElement("div");
  const levelLine = document.createElement("div");
  // THE SPLIT, as a bar: creators 60 · keepers 30 · network 10.
  const fnote = Object.assign(document.createElement("div"), { className: "us-bar", title: "Each day's share: creators by your time on their items, keepers by the data their copies brought you, the network the rest" });
  fnote.style.gridTemplateColumns = "60fr 30fr 10fr";
  for (const [cls, label] of [["c", "Creators 60%"], ["k", "Keepers 30%"], ["n", "10%"]]) fnote.append(Object.assign(document.createElement("span"), { className: cls, textContent: label }));
  const grid = (head, rows) => {
    const t = document.createElement("table");
    const h = t.createTHead().insertRow();
    for (const x of head) h.append(Object.assign(document.createElement("th"), { textContent: x }));
    const b = t.createTBody();
    for (const r of rows) b.insertRow().append(...r.map((v, i) => (v instanceof Node ? Object.assign(document.createElement("td"), {}).appendChild(v).parentNode : td(String(v), i ? "num" : ""))));
    return t;
  };
  const fout = document.createElement("div");
  const drawFee = async () => {
    fout.textContent = "Working it out…";
    const cs = await usage.contributions().catch(() => []);
    const lv = await usage.level().catch(() => null);
    const plan = await usage.levelPlan().catch(() => []);
    levelLine.replaceChildren(grid(["Level", "Per month", "Until"], lv ? [[lv.name, String(lv.perMonth), plan.find(p => p.id === lv.id)?.until ?? "—"], ...plan.filter(p => p.id !== lv.id).map(p => [p.name, "", p.until])] : []));
    given.replaceChildren(grid(["Given", "Days", "From", "To"], cs.map(c => [tok(c.amount), String(c.days), usage.dayOf(c.at), usage.dayOf(c.at + (c.days - 1) * 86400000)])));
    const st = await usage.statement(pick.value).catch(e => ((fout.textContent = `Could not: ${e?.message ?? e}`), null));
    if (!st) return;
    if (!st.amount) return fout.replaceChildren();
    const t = document.createElement("table");
    const head = t.createTHead().insertRow();
    for (const x of ["To", "For", "Tokens"]) head.append(Object.assign(document.createElement("th"), { textContent: x }));
    const body = t.createTBody();
    const row = (who, why, n) => {
      const tr = body.insertRow();
      const w = document.createElement("td");
      w.append(who);
      tr.append(w, td(why), td(tok(n), "num"));
    };
    for (const did of [...new Set([...Object.keys(st.creators), ...Object.keys(st.unclaimed)])].sort((a, b) => (st.creators[b] ?? 0) + (st.unclaimed[b] ?? 0) - (st.creators[a] ?? 0) - (st.unclaimed[a] ?? 0))) {
      const u = st.unclaimed[did] ?? 0;
      row(directory.nameEl(did), u ? "creator + keeper share" : "creator", (st.creators[did] ?? 0) + u);
    }
    for (const [did, n] of Object.entries(st.carriers).sort((a, b) => b[1] - a[1])) row(directory.nameEl(did), "keeper", n);
    row("Network", "network", st.network);
    const foot = t.createTFoot().insertRow();
    foot.append(td("Total"), td(`${st.days} day(s)`), td(tok(st.amount), "num"));
    fout.replaceChildren(t);
  };
  giveBtn.onclick = async () => {
    giveBtn.disabled = true;
    await usage.contribute(amount.value, days.value).catch(e => (given.textContent = `✕ ${e?.message ?? e}`));
    giveBtn.disabled = false;
    drawFee();
  };
  // YOUR EARNINGS (pending) and THE LEDGER (rewards §5, shadow mode): every contributor's public statement for the
  // month, summed here — the same sum anyone gets; yours is your line of it. Nothing is paid yet.
  const eh = Object.assign(document.createElement("h3"), { textContent: "Earnings (preview)" });
  const eout = document.createElement("div");
  // ONE ROW PER DAY (the cutoff: today running, earlier days final), newest first.
  const drawEarnings = async () => {
    eout.textContent = "…";
    const e = await usage.earnings(pick.value).catch(err => ((eout.textContent = `Could not: ${err?.message ?? err}`), null));
    if (!e) return;
    const me = (await (await ctx.require("space")).account())?.id;
    const t = document.createElement("table");
    const head = t.createTHead().insertRow();
    for (const x of ["Day", "", "You: creator", "You: keeper", "Ledger total", "To creators", "To keepers", "To network"]) head.append(Object.assign(document.createElement("th"), { textContent: x }));
    const b = t.createTBody();
    const days = Object.entries(e.ledger.perDay).sort((x, y) => (x[0] < y[0] ? 1 : -1));
    for (const [day, d] of days) {
      const mine = d.by[me] ?? { creator: 0, carrier: 0 };
      b.insertRow().append(td(day), td(d.final ? "final" : "running"), td(tok(mine.creator), "num"), td(tok(mine.carrier), "num"), td(tok(d.total), "num"), td(tok(d.creators), "num"), td(tok(d.carriers), "num"), td(tok(d.network), "num"));
    }
    if (!days.length) b.insertRow().append(td("—"), td(""), td("0", "num"), td("0", "num"), td("0", "num"), td("0", "num"), td("0", "num"), td("0", "num"));
    eout.replaceChildren(t);
  };
  // SUMMARY: the month in numbers — what was used, the level, what was given, what was earned, what is kept.
  const summary = async () => {
    const m = pick.value;
    const [rows, lv, cs, st, e, k] = await Promise.all([
      usage.month(m).catch(() => []),
      usage.level().catch(() => null),
      usage.contributions().catch(() => []),
      usage.statement(m).catch(() => null),
      usage.earnings(m).catch(() => null),
      (await ctx.require("keep")).kept().catch(() => null),
    ]);
    const sum = key => rows.reduce((n, r) => n + (r[key] ?? 0), 0);
    const t = document.createElement("table");
    const b = t.createTBody();
    for (const [x, v] of [
      ["Time", clock(sum("s"))],
      ["Data", size(sum("b"))],
      ["Opened", String(sum("n"))],
      ["Level", lv ? `${lv.name} (${lv.perMonth} a month now)` : "—"],
      ["Contributions running", String(cs.filter(c => c.at + c.days * 86400000 > Date.now()).length)],
      ["Given this month (split)", st ? tok(st.amount) : "0"],
      ["Earned this month (pending)", e ? tok(e.total) : "0"],
      ["Kept for others", k ? `${size(k.total)} of ${size(k.limit)}` : "—"],
    ])
      b.insertRow().append(td(x), td(v, "num"));
    const box = document.createElement("div");
    box.append(t);
    return box;
  };
  root.append(pick);
  pick.onchange = () => show();
  const body = document.createElement("div");
  root.append(body);
  const show = async () => {
    if (page === "kept") return; // its own, below (no month)
    if (page === "use") (body.replaceChildren(note, out), await draw());
    else if (page === "give") (body.replaceChildren(amountLine, levelLine, given, fnote, fout), await drawFee());
    else if (page === "reward") (body.replaceChildren(eh, eout), await drawEarnings());
    else body.replaceChildren(await summary());
  };
  await show();
  // KEPT FOR OTHERS (rewards step 2, `keep`): what this node keeps of what you played — re-read a day apart so the
  // node does not drop it — within the limit chosen here (oldest played dropped past it).
  const keep = await ctx.require("keep");
  const kh = Object.assign(document.createElement("h3"), { textContent: "Kept for others" });
  const knote = Object.assign(document.createElement("div"), { className: "us-bar", title: "Kept of your limit (it shares your node's contract storage, at most 1 GiB by default)" });
  const kfig = Object.assign(document.createElement("span"), { className: "num" });
  const limit = Object.assign(document.createElement("select"), { title: "How much to keep" });
  const LIMITS = [["0", "Off"], ["256000000", "256 MB"], ["512000000", "512 MB"], ["1000000000", "1 GB"], ["5000000000", "5 GB"]];
  limit.append(...LIMITS.map(([v, t]) => Object.assign(document.createElement("option"), { value: v, textContent: t })));
  const klist = document.createElement("div");
  const ago = t => {
    if (!t) return "not yet";
    const m = Math.round((Date.now() - t) / 60000);
    return m < 1 ? "just now" : m < 60 ? `${m} min ago` : m < 2880 ? `${Math.round(m / 60)} h ago` : `${Math.round(m / 1440)} days ago`;
  };
  const drawKept = async () => {
    const k = await keep.kept().catch(e => ((klist.textContent = `Could not read: ${e?.message ?? e}`), null));
    if (!k) return;
    limit.value = String(LIMITS.find(([v]) => Number(v) === k.limit)?.[0] ?? "512000000");
    const pct = k.limit ? Math.min(100, Math.round((100 * k.total) / k.limit)) : 0;
    kfig.textContent = `${size(k.total)} / ${size(k.limit)} · ${k.files.length} file(s)`;
    knote.style.gridTemplateColumns = `${pct}fr ${100 - pct}fr`;
    knote.replaceChildren(Object.assign(document.createElement("span"), { className: "used", textContent: pct >= 12 ? `${pct}%` : "" }), Object.assign(document.createElement("span"), { className: "free", textContent: "" }));
    if (!k.files.length) return klist.replaceChildren();
    const t = document.createElement("table");
    const head = t.createTHead().insertRow();
    for (const x of ["What", "Size", "Played", "Kept", "Health", "Keepers"]) head.append(Object.assign(document.createElement("th"), { textContent: x }));
    const body = t.createTBody();
    for (const f of k.files) {
      const tr = body.insertRow();
      const what = document.createElement("td");
      what.append(f.item ? Object.assign(document.createElement("a"), { href: items.pageOf(f.item, f.kind ?? "video"), textContent: f.title || "an item" }) : `${f.root.slice(0, 8)}…`);
      const health = f.error ? f.error : f.gens ? `${f.whole}/${f.gens} whole${f.missing ? `, ${f.missing} unanswered` : ""}` : "—";
      const who = td("…", "num");
      tr.append(what, td(size(f.size), "num"), td(ago(f.used)), td(ago(f.at)), td(health), who);
      // Who keeps it (the keepers bag, this month or last): you among them once it is whole here.
      keep.keepers(f.root).then(ds => (who.textContent = String(ds.length)), () => (who.textContent = "—"));
    }
    const foot = t.createTFoot().insertRow();
    foot.append(td("Total"), td(`${size(k.total)} of ${size(k.limit)}`, "num"), td(""), td(""), td(""), td(""));
    klist.replaceChildren(t);
  };
  limit.onchange = async () => (await keep.setLimit(Number(limit.value)), drawKept());
  if (page === "kept") {
    pick.hidden = true;
    const krow = Object.assign(document.createElement("div"), { className: "us-row" });
    krow.append(limit, knote, kfig);
    body.replaceChildren(krow, klist);
    await drawKept();
  }
}
