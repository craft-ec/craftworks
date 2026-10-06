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
  }</style><div class="us"><h2>📊 Usage</h2></div>`;
  const root = el.querySelector(".us");
  const usage = await ctx.require("usage");
  const items = await ctx.require("items");
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
  pick.onchange = draw;
  const note = Object.assign(document.createElement("p"), { className: "note", textContent: "Time: what played, for a video or an audio; the time its page was in front of you, for everything else. Data: what its files brought over the network. Kept in your own table — only you see it." });
  root.append(pick, note, out);
  await draw();
}
