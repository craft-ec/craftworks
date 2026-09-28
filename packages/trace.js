// THE TRACE, the loader's own: an overlay with two tabs. LOADING is the page's log, in order. STACK is the app as its
// manifest names it — the wrapper, the loader, the layout's slots, the pages, and every package with its pieces (k data + m parity) and
// whether it is loaded yet. It only reads; it changes nothing.
export function mount(ctx, el, stack) {
  el.innerHTML = `
    <div style="display:flex;gap:4px;margin-bottom:8px">
      <button data-tab="loading">Loading</button>
      <button data-tab="stack">Stack</button>
    </div>
    <div data-pane="loading">
      <table><thead><tr><th>ms</th><th>step</th><th>what</th><th>bytes</th><th>hash</th><th>took</th></tr></thead><tbody></tbody></table>
    </div>
    <div data-pane="stack" hidden></div>`;
  const panes = el.querySelectorAll("[data-pane]");
  const tabs = el.querySelectorAll("[data-tab]");
  const show = name => {
    for (const p of panes) p.hidden = p.dataset.pane !== name;
    for (const t of tabs) t.style.fontWeight = t.dataset.tab === name ? "bold" : "normal";
    if (name === "stack") drawStack();
  };
  for (const t of tabs) t.onclick = () => show(t.dataset.tab);

  const body = el.querySelector("tbody");
  ctx.on(e => {
    const tr = document.createElement("tr");
    [e.t, e.step, e.what ?? "", e.bytes ?? "", e.hash ?? "", e.ms != null ? `${e.ms} ms` : ""].forEach((v, i) => {
      const td = document.createElement("td");
      td.textContent = String(v);
      if (i === 0 || i === 3 || i === 5) td.className = "num";
      tr.append(td);
    });
    body.append(tr);
    if (!el.querySelector('[data-pane="stack"]').hidden) drawStack();
  });

  function drawStack() {
    const m = stack.manifest();
    const pane = el.querySelector('[data-pane="stack"]');
    const esc = t => String(t).replace(/[&<>"]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
    const row = cells => `<tr>${cells.map(c => `<td>${c}</td>`).join("")}</tr>`;
    const short = t => `<code>${esc(String(t).slice(0, 12))}…</code>`;
    const slots = ["header", "footer"].map(s => row([`slot <b>${s}</b>`, esc((m.layout?.[s] ?? []).join(", ") || "—")]));
    const pages = Object.entries(m.pages).map(([route, v]) => {
      const p = Array.isArray(v) ? { body: v } : v;
      const parts = ["body", "header", "footer"].filter(k => p[k]).map(k => `${k}: ${esc(p[k].join(", "))}`);
      return row([`page <b>${esc(route)}</b>${route === ctx.route ? " (here)" : ""}`, parts.join(" · ")]);
    });
    // WHERE EACH PACKAGE IS PLACED: the slots that name it (the layout's on every page, a page's on that page), or,
    // for one no slot names, the component or service that asked for it.
    const placed = name => {
      const at = [];
      for (const s of ["header", "footer"]) if ((m.layout?.[s] ?? []).includes(name)) at.push(`${s} · every page`);
      for (const [route, v] of Object.entries(m.pages)) {
        const p = Array.isArray(v) ? { body: v } : v;
        for (const s of ["header", "body", "footer"]) if ((p[s] ?? []).includes(name)) at.push(`${s} · ${route}`);
      }
      if (at.length) return esc(at.join(", "));
      const by = stack.askedBy(name);
      return by ? `via <b>${esc(by)}</b>` : stack.loaded(name) ? "the loader" : "<i>not asked for yet</i>";
    };
    const pkgs = Object.entries(m.packages).map(([name, p]) =>
      row([`<b>${esc(name)}</b>`, placed(name), esc(p.kind), esc(`${p.k}+${p.m}`), short(p.sha256), stack.loaded(name) ? "loaded" : "not yet"]),
    );
    pane.innerHTML = `
      <p style="margin:0 0 8px"><button type="button" class="fresh">Get the newest</button>
        <span class="fresh-said" style="opacity:.7">this node follows the app's and the loader's sites, then the page loads again</span></p>
      <table>
        ${row(["<b>wrapper</b>", `app site ${short(stack.appSite())} (index.html, boot.js, manifest.json)`])}
        ${row(["<b>loader</b>", `loader site ${short(stack.loaderSite())} · v${esc(stack.loaderVersion)} (loader.js, trace)`])}
        ${slots.join("")}
        ${pages.join("")}
      </table>
      <h3 style="font-size:.9rem;margin:10px 0 4px">Packages (${Object.keys(m.packages).length})</h3>
      <table><thead><tr><th>name</th><th>placed in</th><th>kind</th><th>pieces (k+m)</th><th>sha256</th><th>now</th></tr></thead>${pkgs.join("")}</table>`;
    const b = pane.querySelector(".fresh");
    b.onclick = async () => {
      b.disabled = true;
      pane.querySelector(".fresh-said").textContent = "asking the network…";
      const said = await stack.fresh();
      pane.querySelector(".fresh-said").textContent = said.map(x => `${x.site.slice(0, 8)}…: ${x.kind}`).join(" · ");
      if (said.some(x => x.kind === "got")) setTimeout(() => location.reload(), 800);
      else b.disabled = false;
    };
  }

  show("loading");
}
