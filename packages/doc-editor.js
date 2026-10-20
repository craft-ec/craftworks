// DOC EDITOR, a component: a document as a word processor shows it — a PAGE on a grey desk, a TOOLBAR (paragraph
// style: Normal, Heading 1–3, Quote, bulleted and numbered lists; bold, italic, underline, strikethrough, code, link;
// clear formatting) — over a co-edited document's TEXT and MARKS (`codoc`: inline marks `bold` `italic` `underline`
// `strike` `code` `link`; a paragraph's style is the `block` mark over its line and the newline ending it: `h1` `h2`
// `h3` `quote` `ul` `ol`). Every keystroke is taken by the editor (`beforeinput`), applied to its own copy at once and
// handed on in order — `onSplice(pos, del, insert)`, `onMark(start, end, name, value, on)` — so what the person sees is
// never waiting on the network; `set(doc)` draws another's changes (the caret kept, moved by what came before it).
// Positions are UTF-16 units, as `codoc`'s.
//
//   const ed = (await ctx.require("doc-editor")).create({ text, marks, readOnly, onSplice, onMark })
//   ed.canvas  ed.tools  ed.menus  ed.undo()  ed.redo()   // its parts, for the office shell (`office-shell`)
//   ed.set({ text, marks })   ed.busy()   // typing now (another's change waits)   ed.readOnly(on)
// UNDO: each edit kept with its inverse (a burst of typing one step); a mark's undo restores the marks it replaced.
const INLINE = ["bold", "italic", "underline", "strike", "code", "link"];
const BLOCKS = [
  ["", "Normal text"],
  ["h1", "Heading 1"],
  ["h2", "Heading 2"],
  ["h3", "Heading 3"],
  ["quote", "Quote"],
];

// THE MODEL's PURE PARTS (tested: tools/doc-editor.test.mjs).
// Lines: [{ start, end }] (end excludes its newline).
export function linesOf(text) {
  const out = [];
  let s = 0;
  for (let i = 0; i <= text.length; i++)
    if (i === text.length || text[i] === "\n") {
      out.push({ start: s, end: i });
      s = i + 1;
    }
  return out;
}
// A line's style: the `block` mark over its first unit — its text's, or (an empty line) its newline's.
export function blockOf(marks, line, text) {
  const at = line.start < line.end ? line.start : line.end < text.length ? line.end : -1;
  if (at < 0) return "";
  return marks.find(m => m.name === "block" && m.start <= at && at < m.end)?.value ?? "";
}
// A line's inline runs: [{ text, start, marks: { bold: true, link: "…" } }], split where any inline mark starts or ends.
export function runsOf(text, marks, line) {
  const ms = marks.filter(m => INLINE.includes(m.name) && m.start < line.end && m.end > line.start);
  const cuts = new Set([line.start, line.end]);
  for (const m of ms) [m.start, m.end].forEach(x => x > line.start && x < line.end && cuts.add(x));
  const at = [...cuts].sort((a, b) => a - b);
  const out = [];
  for (let i = 0; i < at.length - 1; i++) {
    const [a, b] = [at[i], at[i + 1]];
    const on = {};
    for (const m of ms) if (m.start <= a && b <= m.end) on[m.name] = m.value ?? true;
    out.push({ text: text.slice(a, b), start: a, marks: on });
  }
  return out;
}
// THIS PAGE's COPY moved by an edit (before the network says so): marks after it shift, inside it grow or shrink —
// an insertion at a mark's end extends it (`codoc`'s marks expand after), at its start does not.
export function shifted(marks, pos, del, ins) {
  const d = ins - del;
  const end = pos + del;
  return marks
    .map(m => {
      let { start, end: e } = m;
      const f = x => (x <= pos ? x : x >= end ? x + d : pos);
      start = start < pos ? start : start >= end ? start + d : pos;
      e = e < pos ? e : e === pos && del === 0 ? e + ins : f(e);
      return { ...m, start, end: e };
    })
    .filter(m => m.end > m.start);
}

export async function start(ctx) {
  const theme = await ctx.require("theme");
  const h = theme.h;
  // ITS LOOK, once, in the components' layer (a component's own: a page places it, never restyles it — theme.js's cascade).
  const css = document.createElement("style");
  css.textContent = `@layer components {
      .de .desk { padding: 12px 12px 48px; }
      .de .page { box-sizing: border-box; max-width: 816px; min-height: 1056px; margin: 0 auto; padding: 72px 80px; background: var(--cw-surface); color: var(--cw-fg); box-shadow: 0 1px 3px rgba(0,0,0,.15), 0 4px 12px rgba(0,0,0,.06); outline: none; font-size: 15px; line-height: 1.6; white-space: pre-wrap; overflow-wrap: break-word; }
      @media (max-width: 700px) { .de .page { padding: 28px 20px; min-height: 70vh; } }
      .de .page > * { margin: 0 0 .35em; }
      .de .page h1 { font-size: 1.9em; line-height: 1.25; margin-top: .6em; }
      .de .page h2 { font-size: 1.45em; line-height: 1.3; margin-top: .5em; }
      .de .page h3 { font-size: 1.2em; margin-top: .4em; }
      .de .page blockquote { border-left: 3px solid var(--cw-line); padding-left: 1em; color: var(--cw-muted); }
      .de .page ul, .de .page ol { padding-left: 1.6em; }
      .de .page li { margin: 0 0 .15em; }
      .de .page code { font-family: var(--cw-font-mono); font-size: .9em; background: var(--cw-hover); padding: 0 .25em; border-radius: 3px; }
      .de .page a { color: var(--cw-accent, #0b57d0); text-decoration: underline; }
  }`;
  document.head.append(css);

  function create({ text = "", marks = [], readOnly = false, onSplice = () => {}, onMark = () => {} } = {}) {
    let doc = { text, marks: marks.map(m => ({ ...m })) };
    let typing = 0;
    const page = h("div", { className: "page", contentEditable: readOnly ? "false" : "true", spellcheck: true, role: "textbox", ariaMultiLine: "true" });
    const desk = h("div", { className: "desk" }, page);
    const styleSel = h("select", { ariaLabel: "Paragraph style", title: "Paragraph style" }, ...BLOCKS.map(([v, t]) => h("option", { value: v, textContent: t })));
    const btn = (label, title, run, style = "") => {
      const b = h("button", { type: "button", textContent: label, title, ariaLabel: title, style });
      b.onmousedown = e => e.preventDefault(); // (the selection stays where it is)
      b.onclick = () => run();
      return b;
    };
    const toggles = {};
    const tog = (name, label, title, style) => (toggles[name] = btn(label, title, () => toggle(name), style));
    // ITS TOOLBAR GROUPS and MENUS (the office shell draws them).
    const tools = [
      [styleSel],
      [tog("bold", "B", "Bold (Ctrl+B)", "font-weight:700"), tog("italic", "I", "Italic (Ctrl+I)", "font-style:italic"), tog("underline", "U", "Underline (Ctrl+U)", "text-decoration:underline"), tog("strike", "S", "Strikethrough", "text-decoration:line-through"), tog("code", "</>", "Code"), btn("🔗", "Link (Ctrl+K)", () => link())],
      [btn("•≡", "Bulleted list", () => block("ul")), btn("1≡", "Numbered list", () => block("ol"))],
      [btn("T̸", "Clear formatting", () => clear())],
    ];
    const menus = {
      Insert: [{ label: "Link…", run: () => link(), keys: "Ctrl+K" }],
      Format: [
        { label: "Bold", run: () => toggle("bold"), keys: "Ctrl+B" },
        { label: "Italic", run: () => toggle("italic"), keys: "Ctrl+I" },
        { label: "Underline", run: () => toggle("underline"), keys: "Ctrl+U" },
        { label: "Strikethrough", run: () => toggle("strike") },
        { label: "Code", run: () => toggle("code") },
        "-",
        ...BLOCKS.map(([v, t]) => ({ label: t, run: () => block(v) })),
        { label: "Bulleted list", run: () => block("ul") },
        { label: "Numbered list", run: () => block("ol") },
        "-",
        { label: "Clear formatting", run: () => clear() },
      ],
    };
    const el = h("div", { className: "de" }, desk);

    // DRAWN from the model: each line an element by its style (lists grouped), its runs as spans.
    function draw() {
      const out = [];
      let list = null;
      for (const line of linesOf(doc.text)) {
        const b = blockOf(doc.marks, line, doc.text);
        const tag = { h1: "h1", h2: "h2", h3: "h3", quote: "blockquote", ul: "li", ol: "li" }[b] ?? "p";
        const node = h(tag, {});
        node.dataset.start = String(line.start);
        const runs = runsOf(doc.text, doc.marks, line);
        for (const r of runs) {
          let x = document.createTextNode(r.text);
          for (const [m, w] of [["code", "code"], ["strike", "s"], ["underline", "u"], ["italic", "em"], ["bold", "strong"]]) if (r.marks[m]) x = h(w, {}, x);
          if (r.marks.link) x = h("a", { href: String(r.marks.link), target: "_blank", rel: "noopener" }, x);
          node.append(x);
        }
        if (line.start === line.end) node.append(h("br", {}));
        if (b === "ul" || b === "ol") {
          if (!list || list.tagName.toLowerCase() !== b) out.push((list = h(b, {})));
          list.append(node);
        } else {
          list = null;
          out.push(node);
        }
      }
      page.replaceChildren(...out);
    }

    // THE CARET as a position in the text (and back): a line element's start, plus what precedes the point inside it.
    function posOf(node, off) {
      const line = (node.nodeType === 1 ? node : node.parentElement)?.closest("[data-start]");
      if (!line || !page.contains(line)) return node === page ? (off === 0 ? 0 : doc.text.length) : null;
      const r = document.createRange();
      r.setStart(line, 0);
      r.setEnd(node, off);
      return Number(line.dataset.start) + r.toString().length;
    }
    function selection() {
      const s = page.ownerDocument.getSelection();
      if (!s?.rangeCount) return null;
      const a = posOf(s.anchorNode, s.anchorOffset);
      const f = posOf(s.focusNode, s.focusOffset);
      if (a == null || f == null) return null;
      return { start: Math.min(a, f), end: Math.max(a, f) };
    }
    function place(pos) {
      const lines = [...page.querySelectorAll("[data-start]")];
      let line = lines[0];
      for (const l of lines) if (Number(l.dataset.start) <= pos) line = l;
      if (!line) return;
      let left = pos - Number(line.dataset.start);
      const w = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
      let n = w.nextNode();
      const s = page.ownerDocument.getSelection();
      const r = document.createRange();
      while (n) {
        if (left <= n.length) {
          r.setStart(n, left);
          r.collapse(true);
          s.removeAllRanges();
          s.addRange(r);
          return;
        }
        left -= n.length;
        n = w.nextNode();
      }
      r.setStart(line, 0);
      r.collapse(true);
      s.removeAllRanges();
      s.addRange(r);
    }

    // UNDO and REDO: each step its inverse — a splice's (the text it removed put back), a mark's (the marks it replaced).
    const undos = [];
    const redos = [];
    let replaying = false;
    const record = step => {
      if (replaying) return;
      redos.length = 0;
      const top = undos.at(-1);
      // (A burst of typing: one step — a character put right after the last, within 1.5 s.)
      // (Typing that goes on after a replacement — the selection typed over — is the same step: undone, what it replaced
      // comes back, as a word processor's.)
      if (step.kind === "splice" && !step.removed && step.inserted.length === 1 && step.inserted !== "\n" && top?.kind === "splice" && top.pos + top.inserted.length === step.pos && Date.now() - top.at < 1500)
        return void ((top.inserted += step.inserted), (top.at = Date.now()));
      undos.push({ ...step, at: Date.now() });
      if (undos.length > 500) undos.shift();
    };
    function replay(step, back) {
      replaying = true;
      try {
        if (step.kind === "splice") {
          const [del, ins] = back ? [step.inserted.length, step.removed] : [step.removed.length, step.inserted];
          splice(step.pos, del, ins);
        } else {
          if (back) {
            mark(step.start, step.end, step.name, null, false);
            for (const m of step.before) mark(m.start, m.end, step.name, m.value, true);
          } else mark(step.start, step.end, step.name, step.value, step.on);
          draw();
          place(back ? step.start : step.end);
        }
      } finally {
        replaying = false;
      }
    }
    const undo = () => {
      const st = undos.pop();
      if (st) (replay(st, true), redos.push(st));
    };
    const redo = () => {
      const st = redos.pop();
      if (st) (replay(st, false), undos.push(st));
    };

    // AN EDIT: the model changed here at once, handed on, drawn, the caret placed after it.
    function splice(pos, del, ins) {
      if (!del && !ins) return;
      record({ kind: "splice", pos, removed: doc.text.slice(pos, pos + del), inserted: ins });
      doc = { text: doc.text.slice(0, pos) + ins + doc.text.slice(pos + del), marks: shifted(doc.marks, pos, del, ins.length) };
      typing = Date.now();
      onSplice(pos, del, ins);
      // A NEW PARAGRAPH after a heading or quote is Normal (as a word processor's): its new line's style taken off.
      if (ins === "\n") {
        const line = linesOf(doc.text).find(l => l.start === pos + 1);
        const b = line ? blockOf(doc.marks, line, doc.text) : "";
        if (b && b !== "ul" && b !== "ol") setBlock(line, "", false);
      }
      draw();
      place(pos + ins.length);
      sync();
    }
    function mark(start, end, name, value, on) {
      if (end <= start) return;
      record({ kind: "mark", start, end, name, value: value ?? null, on, before: doc.marks.filter(m => m.name === name && m.start < end && m.end > start).map(m => ({ start: Math.max(m.start, start), end: Math.min(m.end, end), value: m.value ?? null })) });
      doc.marks = on ? [...doc.marks.filter(m => !(m.name === name && m.start >= start && m.end <= end)), { start, end, name, value: value ?? null }] : cut(doc.marks, start, end, name);
      typing = Date.now();
      onMark(start, end, name, value, on);
    }
    const cut = (ms, start, end, name) =>
      ms.flatMap(m => (m.name !== name || m.end <= start || m.start >= end ? [m] : [...(m.start < start ? [{ ...m, end: start }] : []), ...(m.end > end ? [{ ...m, start: end }] : [])]));
    const covered = (start, end, name) => end > start && doc.marks.some(m => m.name === name && m.start <= start && m.end >= end);
    function toggle(name) {
      const s = selection();
      if (!s || s.start === s.end) return;
      mark(s.start, s.end, name, null, !covered(s.start, s.end, name));
      draw();
      restore(s);
      sync();
    }
    async function link() {
      const s = selection();
      if (!s || s.start === s.end) return;
      // (The one way a page asks: `theme.ask`, next to the text it links.)
      const url = await theme.ask(page.ownerDocument.getSelection()?.getRangeAt(0) ?? page, { text: "Link to (https://…)", value: "https://", ok: "Link" });
      if (url == null) return;
      restore(s);
      const ok = /^https?:\/\/\S+$/i.test(url.trim());
      mark(s.start, s.end, "link", ok ? url.trim() : null, ok);
      draw();
      restore(s);
    }
    function clear() {
      const s = selection();
      if (!s || s.start === s.end) return;
      for (const n of INLINE) if (doc.marks.some(m => m.name === n && m.start < s.end && m.end > s.start)) mark(s.start, s.end, n, null, false);
      draw();
      restore(s);
    }
    // A PARAGRAPH STYLE over the lines the selection touches: each line's style mark replaced (over its newline too).
    function setBlock(line, value, draw_ = true) {
      const end = line.end < doc.text.length ? line.end + 1 : line.end;
      if (end <= line.start) return;
      if (doc.marks.some(m => m.name === "block" && m.start < end && m.end > line.start)) mark(line.start, end, "block", null, false);
      if (value) mark(line.start, end, "block", value, true);
      if (draw_) draw();
    }
    function block(value) {
      const s = selection() ?? { start: 0, end: 0 };
      const lines = linesOf(doc.text).filter(l => l.end >= s.start && l.start <= s.end);
      // (A list pressed again on a list: back to Normal.)
      const all = lines.every(l => blockOf(doc.marks, l, doc.text) === value);
      for (const l of lines) setBlock(l, all && (value === "ul" || value === "ol") ? "" : value, false);
      draw();
      restore(s);
      sync();
    }
    styleSel.onchange = () => block(styleSel.value);
    function restore(s) {
      place(s.start);
      if (s.end > s.start) {
        const sel = page.ownerDocument.getSelection();
        const a = sel.getRangeAt(0).cloneRange();
        place(s.end);
        const b = sel.getRangeAt(0);
        a.setEnd(b.startContainer, b.startOffset);
        sel.removeAllRanges();
        sel.addRange(a);
      }
    }
    // THE TOOLBAR follows the caret: the line's style, each mark on where it is.
    function sync() {
      const s = selection();
      if (!s) return;
      const line = linesOf(doc.text).find(l => l.start <= s.start && s.start <= l.end);
      const b = line ? blockOf(doc.marks, line, doc.text) : "";
      styleSel.value = BLOCKS.some(([v]) => v === b) ? b : "";
      const at = s.end > s.start ? [s.start, s.end] : [Math.max(0, s.start - 1), s.start];
      for (const [name, b_] of Object.entries(toggles)) b_.ariaPressed = String(covered(at[0], at[1], name));
    }
    page.ownerDocument.addEventListener("selectionchange", () => page.isConnected && page.contains(page.ownerDocument.getSelection()?.anchorNode ?? null) && sync());

    // EVERY INPUT TAKEN HERE (the browser never edits the page itself): typed, pasted, deleted, a new paragraph.
    let composing = null;
    page.addEventListener("beforeinput", e => {
      if (readOnly) return e.preventDefault();
      const t = e.inputType;
      if (t === "insertCompositionText") return; // (an IME's text: taken whole at its end)
      e.preventDefault();
      const s = selection();
      if (!s) return;
      const span = () => {
        const r = e.getTargetRanges?.()[0];
        if (!r) return null;
        const a = posOf(r.startContainer, r.startOffset);
        const b = posOf(r.endContainer, r.endOffset);
        return a == null || b == null ? null : { start: a, end: b };
      };
      if (t === "insertText" || t === "insertReplacementText") return splice(s.start, s.end - s.start, e.data ?? e.dataTransfer?.getData("text/plain") ?? "");
      if (t === "insertParagraph" || t === "insertLineBreak") return splice(s.start, s.end - s.start, "\n");
      if (t === "insertFromPaste" || t === "insertFromDrop") return splice(s.start, s.end - s.start, (e.dataTransfer?.getData("text/plain") ?? "").replace(/\r\n?/g, "\n"));
      if (t.startsWith("delete")) {
        if (s.end > s.start) return splice(s.start, s.end - s.start, "");
        const r = span();
        if (r && r.end > r.start) return splice(r.start, r.end - r.start, "");
        // (A collapsed caret with no range given: one character, a surrogate pair whole.)
        if (t.includes("Backward") && s.start > 0) {
          const two = /[\uDC00-\uDFFF]/.test(doc.text[s.start - 1] ?? "") ? 2 : 1;
          return splice(s.start - two, two, "");
        }
        if (t.includes("Forward") && s.start < doc.text.length) return splice(s.start, /[\uD800-\uDBFF]/.test(doc.text[s.start]) ? 2 : 1, "");
        return;
      }
      if (t === "historyUndo") return undo();
      if (t === "historyRedo") return redo();
      if (t === "formatBold") return toggle("bold");
      if (t === "formatItalic") return toggle("italic");
      if (t === "formatUnderline") return toggle("underline");
    });
    page.addEventListener("compositionstart", () => (composing = selection()));
    page.addEventListener("compositionend", e => {
      const s = composing;
      composing = null;
      if (!s) return;
      draw(); // (what the IME left in the page undone: the model is drawn again, the text then put in)
      splice(s.start, s.end - s.start, e.data ?? "");
    });
    page.addEventListener("keydown", e => {
      if (!(e.metaKey || e.ctrlKey) || readOnly) return;
      const k = e.key.toLowerCase();
      if (k === "b" || k === "i" || k === "u") (e.preventDefault(), toggle({ b: "bold", i: "italic", u: "underline" }[k]));
      if (k === "z" && !e.shiftKey) (e.preventDefault(), e.stopPropagation(), undo());
      if (k === "y" || (k === "z" && e.shiftKey)) (e.preventDefault(), e.stopPropagation(), redo());
      if (k === "k") (e.preventDefault(), link());
    });
    draw();

    return {
      el,
      canvas: el,
      tools,
      menus,
      undo,
      redo,
      // ANOTHER's CHANGES: drawn, the caret kept — moved by what changed before it.
      set(next) {
        const s = page.contains(page.ownerDocument.getSelection()?.anchorNode ?? null) ? selection() : null;
        const before = doc.text;
        doc = { text: next.text, marks: next.marks.map(m => ({ ...m })) };
        draw();
        if (!s) return;
        let i = 0;
        while (i < before.length && i < doc.text.length && before[i] === doc.text[i]) i++;
        const d = doc.text.length - before.length;
        restore({ start: s.start > i ? s.start + d : s.start, end: s.end > i ? s.end + d : s.end });
      },
      busy: () => Date.now() - typing < 1500 || !!composing,
      readOnly: on => ((readOnly = on), (page.contentEditable = on ? "false" : "true")),
    };
  }
  return { create };
}
