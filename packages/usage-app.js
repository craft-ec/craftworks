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
  // KEPT FOR OTHERS (rewards step 2, `keep`): what this node keeps of what you played — re-read a day apart so the
  // node does not drop it — within the limit chosen here (oldest played dropped past it).
  const keep = await ctx.require("keep");
  const kh = Object.assign(document.createElement("h3"), { textContent: "Kept for others" });
  const knote = Object.assign(document.createElement("p"), {
    className: "note",
    textContent: "What you play is kept on your node for whoever plays it next, re-read once a day so your node does not drop it. It shares your node's contract storage limit (Freenet's --max-hosting-storage: by default an eighth of its memory, at most 1 GiB — not the larger “Disk budget” on the node's dashboard), so keep this below that.",
  });
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
    if (!k.files.length) return (klist.textContent = k.limit ? "Nothing yet: play something (not yours)." : "Off.");
    const t = document.createElement("table");
    const head = t.createTHead().insertRow();
    for (const x of ["What", "Size", "Played", "Kept", "Health", "Keepers"]) head.append(Object.assign(document.createElement("th"), { textContent: x }));
    const body = t.createTBody();
    for (const f of k.files) {
      const tr = body.insertRow();
      const what = document.createElement("td");
      what.append(f.item ? Object.assign(document.createElement("a"), { href: items.pageOf(f.item, "video"), textContent: f.title || "an item" }) : `${f.root.slice(0, 8)}…`);
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
  root.append(kh, knote, limit, klist);
  await drawKept();
}
