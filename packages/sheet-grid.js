// SHEET GRID, a component: a spreadsheet as a spreadsheet app shows it — a FORMULA BAR (the selected cell's name and
// what it holds), COLUMN letters and ROW numbers, a GRID of cells (A–Z × 200), each showing its VALUE (a formula
// computed: `=A1*2`, `=SUM(A1:A9)`), and its toolbar groups for the office shell (bold, italic, alignment, number
// format, clear). Over a co-edited document's CELLS (`codoc`): `c:A1` a cell's input, `f:A1` its format (JSON: b, i,
// al, n) — each key the last write winning, so two people editing different cells never conflict.
// Keys: arrows move, Shift+arrows select, Enter edits / commits (and moves down), Tab moves right, Delete clears,
// typing starts editing. Copy, cut and paste carry tab-separated rows (to and from any spreadsheet).
//
//   const g = (await ctx.require("sheet-grid")).create({ cells, readOnly, onSet })
//   g.canvas  g.tools  g.menus  g.undo()  g.redo()  g.set({ cells })  g.busy()  g.readOnly(on)
export const COLS = 26;
export const ROWS = 200;

// THE PURE PARTS (tested: tools/sheet-grid.test.mjs).
export const colName = c => String.fromCharCode(65 + c);
export const nameOf = (r, c) => `${colName(c)}${r + 1}`;
export function parseRef(s) {
  const m = /^([A-Z])([1-9][0-9]*)$/.exec(String(s).toUpperCase());
  if (!m) return null;
  const c = m[1].charCodeAt(0) - 65;
  const r = Number(m[2]) - 1;
  return c < COLS && r < ROWS ? { r, c } : null;
}
// A FORMULA's VALUE: `+ - * / ^`, parentheses, numbers, cell refs, ranges in SUM AVERAGE MIN MAX COUNT; `#ERR` for what
// does not parse, `#CYCLE` for a cell that reaches itself, `#DIV/0` for a division by zero.
export function evaluate(cells, key, seen = new Set()) {
  const raw = cells[`c:${key}`];
  if (raw == null || raw === "") return "";
  if (!String(raw).startsWith("=")) {
    const n = Number(raw);
    return raw.trim() !== "" && Number.isFinite(n) ? n : raw;
  }
  if (seen.has(key)) return "#CYCLE";
  const inner = new Set(seen).add(key);
  try {
    return formula(String(raw).slice(1), ref => {
      const v = evaluate(cells, ref, inner);
      if (typeof v === "string" && v.startsWith("#")) throw new Error(v);
      return v;
    });
  } catch (e) {
    return /^#/.test(e.message) ? e.message : "#ERR";
  }
}
function formula(src, valueOf) {
  const toks = src.match(/\s*([A-Za-z]+[0-9]*(?::[A-Za-z]+[0-9]+)?|[0-9]*\.?[0-9]+(?:[eE][-+]?[0-9]+)?|[-+*/^(),])/g)?.map(t => t.trim()) ?? [];
  if (toks.join("") !== src.replace(/\s+/g, "")) throw new Error("#ERR");
  let i = 0;
  const peek = () => toks[i];
  const take = t => (toks[i] === t ? (i++, true) : false);
  const num = v => (v === "" ? 0 : typeof v === "number" ? v : Number.isFinite(Number(v)) ? Number(v) : (() => { throw new Error("#VALUE"); })());
  const range = s => {
    const [a, b] = s.split(":").map(parseRef);
    if (!a || !b) throw new Error("#REF");
    const out = [];
    for (let r = Math.min(a.r, b.r); r <= Math.max(a.r, b.r); r++) for (let c = Math.min(a.c, b.c); c <= Math.max(a.c, b.c); c++) out.push(valueOf(nameOf(r, c)));
    return out;
  };
  const FN = {
    SUM: vs => vs.filter(v => typeof v === "number").reduce((a, b) => a + b, 0),
    AVERAGE: vs => {
      const ns = vs.filter(v => typeof v === "number");
      if (!ns.length) throw new Error("#DIV/0");
      return ns.reduce((a, b) => a + b, 0) / ns.length;
    },
    MIN: vs => Math.min(...vs.filter(v => typeof v === "number")),
    MAX: vs => Math.max(...vs.filter(v => typeof v === "number")),
    COUNT: vs => vs.filter(v => typeof v === "number").length,
  };
  function atom() {
    const t = toks[i++];
    if (t === undefined) throw new Error("#ERR");
    if (t === "(") {
      const v = expr();
      if (!take(")")) throw new Error("#ERR");
      return v;
    }
    if (t === "-") return -atom();
    if (t === "+") return atom();
    if (/^[0-9.]/.test(t)) return Number(t);
    const up = t.toUpperCase();
    if (FN[up] && take("(")) {
      const vs = [];
      if (!take(")")) {
        do vs.push(...(/:/.test(peek() ?? "") ? range(toks[i++].toUpperCase()) : [expr()]));
        while (take(","));
        if (!take(")")) throw new Error("#ERR");
      }
      return FN[up](vs.map(v => (v === "" ? v : typeof v === "number" ? v : Number.isFinite(Number(v)) ? Number(v) : v)));
    }
    if (parseRef(up)) return num(valueOf(up));
    throw new Error("#ERR");
  }
  function power() {
    const b = atom();
    return take("^") ? b ** power() : b;
  }
  function term() {
    let v = power();
    for (;;) {
      if (take("*")) v *= power();
      else if (take("/")) {
        const d = power();
        if (d === 0) throw new Error("#DIV/0");
        v /= d;
      } else return v;
    }
  }
  function expr() {
    let v = term();
    for (;;) {
      if (take("+")) v += term();
      else if (take("-")) v -= term();
      else return v;
    }
  }
  const v = expr();
  if (i !== toks.length) throw new Error("#ERR");
  return v;
}
// A VALUE SHOWN by its number format (`n`: "", "num" two decimals, "pct", "cur").
export function shown(v, n = "") {
  if (typeof v !== "number") return String(v);
  if (n === "num") return v.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  if (n === "pct") return `${(v * 100).toLocaleString(undefined, { maximumFractionDigits: 2 })}%`;
  if (n === "cur") return v.toLocaleString(undefined, { style: "currency", currency: "USD" });
  return String(Math.round(v * 1e10) / 1e10);
}

export async function start(ctx) {
  const theme = await ctx.require("theme");
  const h = theme.h;
  const css = document.createElement("style");
  css.textContent = `@layer components {
    .sg { display: grid; grid-template-rows: auto 1fr; background: var(--cw-surface); min-height: 70vh; }
    .sg .fx { display: flex; align-items: center; gap: 8px; padding: 4px 10px; border-bottom: 1px solid var(--cw-line); }
    .sg .fx .ref { min-width: 54px; font-weight: 600; color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .sg .fx input { flex: 1; font: inherit; border: 0; outline: none; background: none; color: var(--cw-fg); padding: 4px; }
    .sg .wrap { overflow: auto; max-height: 75vh; }
    .sg table { border-collapse: collapse; table-layout: fixed; font-size: 13px; }
    .sg th { position: sticky; background: var(--cw-bg-sunken, #f1f3f4); color: var(--cw-muted); font-weight: 500; border: 1px solid var(--cw-line); z-index: 1; user-select: none; }
    .sg thead th { top: 0; height: 24px; }
    .sg tbody th { left: 0; width: 44px; text-align: center; }
    .sg thead th.corner { left: 0; z-index: 2; }
    .sg td { border: 1px solid var(--cw-line); width: 100px; max-width: 100px; height: 22px; padding: 0 4px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; cursor: cell; }
    .sg td.num { text-align: right; }
    .sg td.err { color: var(--cw-danger); }
    .sg td.sel { background: var(--cw-accent-soft, #e8f0fe); }
    .sg td.cur { outline: 2px solid var(--cw-accent, #1a73e8); outline-offset: -2px; }
    .sg td input { width: 100%; box-sizing: border-box; border: 0; outline: none; font: inherit; background: var(--cw-surface); color: var(--cw-fg); padding: 0; }
  }`;
  document.head.append(css);

  function create({ cells = {}, readOnly = false, onSet = () => {} } = {}) {
    let data = { ...cells };
    let cur = { r: 0, c: 0 };
    let anchor = { r: 0, c: 0 };
    let editing = null; // { input, r, c }
    let typing = 0;
    const undos = [];
    const redos = [];
    const fmt = key => {
      try {
        return JSON.parse(data[`f:${key}`] ?? "{}") ?? {};
      } catch {
        return {};
      }
    };
    const refEl = h("span", { className: "ref", textContent: "A1" });
    const fxIn = h("input", { ariaLabel: "Formula", placeholder: "", spellcheck: false });
    const table = h("table", {});
    const wrap = h("div", { className: "wrap", tabIndex: 0 }, table);
    const el = h("div", { className: "sg" }, h("div", { className: "fx" }, refEl, h("span", { textContent: "ƒx" }), fxIn), wrap);

    // A WRITE: the cell's new value here at once, handed on, kept to undo (its old value).
    function put(key, value, record = true) {
      const before = data[key] ?? null;
      if ((value ?? null) === before) return;
      if (value == null || value === "") delete data[key];
      else data[key] = value;
      typing = Date.now();
      if (record) (undos.push({ key, before, after: value ?? null }), (redos.length = 0));
      onSet(key, value == null || value === "" ? null : value);
    }
    const undo = () => {
      const s = undos.pop();
      if (s) (put(s.key, s.before, false), redos.push(s), draw());
    };
    const redo = () => {
      const s = redos.pop();
      if (s) (put(s.key, s.after, false), undos.push(s), draw());
    };

    const box = () => ({ r0: Math.min(anchor.r, cur.r), r1: Math.max(anchor.r, cur.r), c0: Math.min(anchor.c, cur.c), c1: Math.max(anchor.c, cur.c) });
    function draw() {
      const b = box();
      const head = h("tr", {}, h("th", { className: "corner" }), ...Array.from({ length: COLS }, (_, c) => h("th", { textContent: colName(c) })));
      const rows = [];
      for (let r = 0; r < ROWS; r++) {
        const tr = h("tr", {}, h("th", { textContent: String(r + 1) }));
        for (let c = 0; c < COLS; c++) {
          const key = nameOf(r, c);
          const v = evaluate(data, key);
          const f = fmt(key);
          const td = h("td", { textContent: shown(v, f.n) });
          td.dataset.r = r;
          td.dataset.c = c;
          if (typeof v === "number") td.classList.add("num");
          if (typeof v === "string" && v.startsWith("#")) td.classList.add("err");
          if (f.b) td.style.fontWeight = "700";
          if (f.i) td.style.fontStyle = "italic";
          if (f.al) td.style.textAlign = f.al;
          if (r >= b.r0 && r <= b.r1 && c >= b.c0 && c <= b.c1 && (b.r0 !== b.r1 || b.c0 !== b.c1)) td.classList.add("sel");
          if (r === cur.r && c === cur.c) td.classList.add("cur");
          tr.append(td);
        }
        rows.push(tr);
      }
      table.replaceChildren(h("thead", {}, head), h("tbody", {}, ...rows));
      refEl.textContent = nameOf(cur.r, cur.c);
      if (document.activeElement !== fxIn) fxIn.value = data[`c:${nameOf(cur.r, cur.c)}`] ?? "";
      for (const [k, b_] of Object.entries(toggles)) b_.ariaPressed = String(!!fmt(nameOf(cur.r, cur.c))[k]);
    }
    const cellEl = (r, c) => table.querySelector(`td[data-r="${r}"][data-c="${c}"]`);

    function move(dr, dc, extend = false) {
      cur = { r: Math.max(0, Math.min(ROWS - 1, cur.r + dr)), c: Math.max(0, Math.min(COLS - 1, cur.c + dc)) };
      if (!extend) anchor = { ...cur };
      draw();
      cellEl(cur.r, cur.c)?.scrollIntoView({ block: "nearest", inline: "nearest" });
    }
    function edit(start = null) {
      if (readOnly) return;
      const td = cellEl(cur.r, cur.c);
      if (!td) return;
      const key = nameOf(cur.r, cur.c);
      const input = h("input", { value: start ?? data[`c:${key}`] ?? "" });
      editing = { input, r: cur.r, c: cur.c };
      td.replaceChildren(input);
      input.focus();
      input.setSelectionRange(input.value.length, input.value.length);
      input.onkeydown = e => {
        if (e.key === "Enter" || e.key === "Tab") (e.preventDefault(), commit(), move(e.key === "Enter" ? (e.shiftKey ? -1 : 1) : 0, e.key === "Tab" ? (e.shiftKey ? -1 : 1) : 0), wrap.focus());
        else if (e.key === "Escape") (e.preventDefault(), (editing = null), draw(), wrap.focus());
        e.stopPropagation();
      };
      input.oninput = () => ((typing = Date.now()), (fxIn.value = input.value));
      input.onblur = () => editing?.input === input && (commit(), draw());
    }
    function commit() {
      if (!editing) return;
      const { input, r, c } = editing;
      editing = null;
      put(`c:${nameOf(r, c)}`, input.value.trim() === "" ? null : input.value);
    }
    // THE SELECTED CELLS (a box), each by its key.
    const each = f => {
      const b = box();
      for (let r = b.r0; r <= b.r1; r++) for (let c = b.c0; c <= b.c1; c++) f(nameOf(r, c));
    };
    function format(patch) {
      if (readOnly) return;
      each(key => {
        const f = { ...fmt(key), ...patch };
        for (const k of Object.keys(f)) if (f[k] === false || f[k] === "" || f[k] == null) delete f[k];
        put(`f:${key}`, Object.keys(f).length ? JSON.stringify(f) : null);
      });
      draw();
    }
    const toggle = k => format({ [k]: !fmt(nameOf(cur.r, cur.c))[k] });
    function clear() {
      if (readOnly) return;
      each(key => put(`c:${key}`, null));
      draw();
    }

    wrap.addEventListener("mousedown", e => {
      const td = e.target.closest("td[data-r]");
      if (!td || editing?.input === e.target) return;
      commit();
      cur = { r: Number(td.dataset.r), c: Number(td.dataset.c) };
      if (!e.shiftKey) anchor = { ...cur };
      draw();
      wrap.focus();
    });
    wrap.addEventListener("dblclick", e => e.target.closest("td[data-r]") && edit());
    wrap.addEventListener("keydown", e => {
      if (editing) return;
      const k = e.key;
      const mod = e.metaKey || e.ctrlKey;
      if (mod && k.toLowerCase() === "z") return void (e.preventDefault(), e.shiftKey ? redo() : undo());
      if (mod && k.toLowerCase() === "y") return void (e.preventDefault(), redo());
      if (mod && k.toLowerCase() === "b") return void (e.preventDefault(), toggle("b"));
      if (mod && k.toLowerCase() === "i") return void (e.preventDefault(), toggle("i"));
      const arrows = { ArrowUp: [-1, 0], ArrowDown: [1, 0], ArrowLeft: [0, -1], ArrowRight: [0, 1] };
      if (arrows[k]) return void (e.preventDefault(), move(...arrows[k], e.shiftKey));
      if (k === "Tab") return void (e.preventDefault(), move(0, e.shiftKey ? -1 : 1));
      if (k === "Enter" || k === "F2") return void (e.preventDefault(), edit());
      if (k === "Delete" || k === "Backspace") return void (e.preventDefault(), clear());
      if (k.length === 1 && !mod) (e.preventDefault(), edit(k));
    });
    // COPY, CUT, PASTE: tab-separated rows (what every spreadsheet reads and writes).
    wrap.addEventListener("copy", e => {
      if (editing) return;
      const b = box();
      const rows = [];
      for (let r = b.r0; r <= b.r1; r++) rows.push(Array.from({ length: b.c1 - b.c0 + 1 }, (_, i) => data[`c:${nameOf(r, b.c0 + i)}`] ?? "").join("\t"));
      e.clipboardData.setData("text/plain", rows.join("\n"));
      e.preventDefault();
    });
    wrap.addEventListener("cut", e => {
      if (editing || readOnly) return;
      const b = box();
      const rows = [];
      for (let r = b.r0; r <= b.r1; r++) rows.push(Array.from({ length: b.c1 - b.c0 + 1 }, (_, i) => data[`c:${nameOf(r, b.c0 + i)}`] ?? "").join("\t"));
      e.clipboardData.setData("text/plain", rows.join("\n"));
      e.preventDefault();
      clear();
    });
    wrap.addEventListener("paste", e => {
      if (editing || readOnly) return;
      e.preventDefault();
      const text = e.clipboardData.getData("text/plain").replace(/\r\n?/g, "\n").replace(/\n$/, "");
      text.split("\n").forEach((line, dr) => line.split("\t").forEach((v, dc) => {
        const r = cur.r + dr;
        const c = cur.c + dc;
        if (r < ROWS && c < COLS) put(`c:${nameOf(r, c)}`, v === "" ? null : v);
      }));
      draw();
    });
    // THE FORMULA BAR: what the cell holds, edited there too.
    fxIn.onkeydown = e => {
      if (e.key === "Enter") (e.preventDefault(), put(`c:${nameOf(cur.r, cur.c)}`, fxIn.value.trim() === "" ? null : fxIn.value), move(1, 0), wrap.focus());
      if (e.key === "Escape") (e.preventDefault(), draw(), wrap.focus());
    };
    fxIn.readOnly = readOnly;

    // ITS TOOLBAR GROUPS and MENUS (the office shell draws them).
    const btn = (label, title, run, style = "") => {
      const b = h("button", { type: "button", textContent: label, title, ariaLabel: title, style });
      b.onclick = () => (run(), wrap.focus());
      return b;
    };
    const toggles = { b: btn("B", "Bold (Ctrl+B)", () => toggle("b"), "font-weight:700"), i: btn("I", "Italic (Ctrl+I)", () => toggle("i"), "font-style:italic") };
    const numSel = h("select", { title: "Number format", ariaLabel: "Number format" }, ...[["", "Automatic"], ["num", "Number 1,000.00"], ["pct", "Percent %"], ["cur", "Currency $"]].map(([v, t]) => h("option", { value: v, textContent: t })));
    numSel.onchange = () => (format({ n: numSel.value }), wrap.focus());
    const tools = [
      [numSel],
      [toggles.b, toggles.i],
      [btn("⇤", "Align left", () => format({ al: "left" })), btn("↔", "Align center", () => format({ al: "center" })), btn("⇥", "Align right", () => format({ al: "right" }))],
      [btn("Σ", "Sum of the cells above", () => sumAbove()), btn("✕", "Clear cells (Delete)", () => clear())],
    ];
    function sumAbove() {
      if (readOnly || cur.r === 0) return;
      let top = cur.r - 1;
      while (top > 0 && data[`c:${nameOf(top - 1, cur.c)}`] != null) top--;
      put(`c:${nameOf(cur.r, cur.c)}`, `=SUM(${nameOf(top, cur.c)}:${nameOf(cur.r - 1, cur.c)})`);
      draw();
    }
    const menus = {
      Insert: [{ label: "Sum (Σ)", run: () => sumAbove() }],
      Format: [
        { label: "Bold", run: () => toggle("b"), keys: "Ctrl+B" },
        { label: "Italic", run: () => toggle("i"), keys: "Ctrl+I" },
        "-",
        { label: "Number", run: () => format({ n: "num" }) },
        { label: "Percent", run: () => format({ n: "pct" }) },
        { label: "Currency", run: () => format({ n: "cur" }) },
        { label: "Automatic", run: () => format({ n: "" }) },
        "-",
        { label: "Align left", run: () => format({ al: "left" }) },
        { label: "Align center", run: () => format({ al: "center" }) },
        { label: "Align right", run: () => format({ al: "right" }) },
        "-",
        { label: "Clear formatting", run: () => each(key => put(`f:${key}`, null)) || draw() },
      ],
    };
    draw();

    return {
      el,
      canvas: el,
      tools,
      menus,
      undo,
      redo,
      // ANOTHER's CELLS: drawn (a cell being edited here kept as it is).
      set(next) {
        data = { ...(next.cells ?? {}) };
        if (!editing) draw();
      },
      busy: () => !!editing || Date.now() - typing < 1500,
      readOnly: on => ((readOnly = on), (fxIn.readOnly = on)),
    };
  }
  return { create };
}
