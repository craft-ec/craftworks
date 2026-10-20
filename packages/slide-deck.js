// SLIDE DECK, a component: a presentation as a presentation app shows it — a FILMSTRIP of slides on the left, the
// selected SLIDE (16:9) on the canvas, its title and text edited in place, its SPEAKER NOTES under it; layouts (Title,
// Title and content, Section header, Blank), new / duplicate / delete / move up / move down, a theme, and PRESENT (full
// screen, arrows and Escape). Over a co-edited document's CELLS (`codoc`): each slide `s:<id>:pos` (its place — a
// number between its neighbours: two people moving slides never fight over one list), `:layout`, `:title`, `:body`,
// `:notes`, `:gone`; the deck's `deck:theme` — each key the last write winning.
//
//   const d = (await ctx.require("slide-deck")).create({ cells, readOnly, onSet })
//   d.canvas  d.tools  d.menus  d.undo()  d.redo()  d.set({ cells })  d.busy()  d.readOnly(on)
const LAYOUTS = [
  ["title", "Title slide"],
  ["content", "Title and content"],
  ["section", "Section header"],
  ["blank", "Blank"],
];
const THEMES = {
  light: { bg: "#ffffff", fg: "#202124", accent: "#1a73e8" },
  dark: { bg: "#202124", fg: "#f1f3f4", accent: "#8ab4f8" },
  ocean: { bg: "#0b3d5c", fg: "#e8f4fa", accent: "#7fd1f5" },
  sunrise: { bg: "#fff4e6", fg: "#3d2a14", accent: "#e8710a" },
};

// THE SLIDES from the cells (pure; tested: tools/slide-deck.test.mjs): each id's fields, ordered by place, the gone left out.
export function slidesOf(cells) {
  const by = new Map();
  for (const [k, v] of Object.entries(cells ?? {})) {
    const m = /^s:([a-z0-9]+):(pos|layout|title|body|notes|gone)$/.exec(k);
    if (!m) continue;
    if (!by.has(m[1])) by.set(m[1], { id: m[1] });
    by.get(m[1])[m[2]] = v;
  }
  return [...by.values()].filter(s => !s.gone && s.pos != null).sort((a, b) => Number(a.pos) - Number(b.pos) || a.id.localeCompare(b.id));
}
// A PLACE between two (none: at an end).
export const between = (a, b) => (a == null && b == null ? 1 : a == null ? Number(b) - 1 : b == null ? Number(a) + 1 : (Number(a) + Number(b)) / 2);

export async function start(ctx) {
  const theme = await ctx.require("theme");
  const h = theme.h;
  const css = document.createElement("style");
  css.textContent = `@layer components {
    .sd { display: grid; grid-template-columns: 180px 1fr; gap: 0; min-height: 70vh; background: var(--cw-bg-sunken, #f1f3f4); }
    @media (max-width: 700px) { .sd { grid-template-columns: 1fr; } .sd .strip { display: flex; overflow-x: auto; } }
    .sd .strip { padding: 10px; display: grid; gap: 10px; align-content: start; overflow-y: auto; max-height: 80vh; border-right: 1px solid var(--cw-line); }
    .sd .thumb { display: grid; grid-template-columns: 18px 1fr; gap: 6px; align-items: start; cursor: pointer; }
    .sd .thumb .n { color: var(--cw-muted); font-size: var(--cw-text-xs); text-align: right; padding-top: 4px; }
    .sd .thumb .mini { aspect-ratio: 16 / 9; border: 2px solid transparent; border-radius: 4px; overflow: hidden; box-shadow: 0 1px 2px rgba(0,0,0,.2); min-width: 120px; }
    .sd .thumb.on .mini { border-color: var(--cw-accent, #1a73e8); }
    .sd .mini .sl { transform: scale(.14); transform-origin: top left; width: 960px; height: 540px; pointer-events: none; }
    .sd .main { padding: 20px; display: grid; gap: 12px; align-content: start; justify-items: center; overflow: auto; }
    .sd .stage { width: min(960px, 100%); aspect-ratio: 16 / 9; position: relative; box-shadow: 0 2px 8px rgba(0,0,0,.18); overflow: hidden; container-type: inline-size; }
    .sd .sl { position: relative; width: 100%; height: 100%; box-sizing: border-box; padding: 6% 8%; display: grid; align-content: center; gap: 4%; font-family: inherit; }
    .sd .sl.content { align-content: start; }
    .sd .sl .t { font-size: 5.2cqi; font-weight: 700; line-height: 1.15; outline: none; min-height: 1.2em; }
    .sd .sl.title .t, .sd .sl.section .t { text-align: center; font-size: 6.4cqi; }
    .sd .sl.section .t { border-bottom: .4cqi solid var(--accent); padding-bottom: 2%; }
    .sd .sl .b { font-size: 2.8cqi; line-height: 1.45; outline: none; white-space: pre-wrap; min-height: 1.4em; }
    .sd .sl.title .b { text-align: center; color: var(--muted); }
    .sd .sl.content .b { padding-left: 1.2em; }
    .sd .sl .b ul { margin: 0; padding-left: 1em; }
    .sd .sl [data-empty]:empty::before { content: attr(data-empty); opacity: .45; }
    .sd .notes { width: min(960px, 100%); box-sizing: border-box; min-height: 70px; padding: 10px; border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); color: var(--cw-fg); font: inherit; resize: vertical; }
    .sd .none { color: var(--cw-muted); }
    .sd-show { position: fixed; inset: 0; z-index: 1000; background: #000; display: grid; place-items: center; }
    .sd-show .stage { width: min(100vw, calc(100vh * 16 / 9)); aspect-ratio: 16 / 9; container-type: inline-size; }
    .sd-show .hint { position: fixed; bottom: 10px; right: 14px; color: #888; font-size: 12px; }
  }`;
  document.head.append(css);

  function create({ cells = {}, readOnly = false, onSet = () => {} } = {}) {
    let data = { ...cells };
    let curId = null;
    let typing = 0;
    const undos = [];
    const redos = [];
    const newId = () => Math.random().toString(36).slice(2, 10);
    function put(key, value, record = true) {
      const before = data[key] ?? null;
      if ((value ?? null) === before) return;
      if (value == null) delete data[key];
      else data[key] = value;
      typing = Date.now();
      if (record) (undos.push({ key, before, after: value ?? null }), (redos.length = 0));
      onSet(key, value);
    }
    // (A step of several writes undone as one: a new slide, a delete, a move.)
    const group = f => {
      const n = undos.length;
      f();
      if (undos.length - n > 1) undos.push({ group: undos.splice(n) });
    };
    const apply = (s, back) => (s.group ? (back ? [...s.group].reverse() : s.group).forEach(x => apply(x, back)) : put(s.key, back ? s.before : s.after, false));
    const undo = () => {
      const s = undos.pop();
      if (s) (apply(s, true), redos.push(s), draw());
    };
    const redo = () => {
      const s = redos.pop();
      if (s) (apply(s, false), undos.push(s), draw());
    };
    const look = () => THEMES[data["deck:theme"]] ?? THEMES.light;

    const strip = h("div", { className: "strip" });
    const stage = h("div", { className: "stage" });
    const notes = h("textarea", { className: "notes", placeholder: "Speaker notes", readOnly });
    const main = h("div", { className: "main" }, stage, notes);
    const el = h("div", { className: "sd" }, strip, main);

    // A SLIDE drawn (editable on the canvas; plain in the filmstrip and when presenting).
    function slideEl(s, editable) {
      const t = look();
      const box = h("div", { className: `sl ${s.layout ?? "content"}`, style: `background:${t.bg};color:${t.fg};--accent:${t.accent};--muted:${t.fg}b3` });
      if ((s.layout ?? "content") === "blank") return box;
      const title = h("div", { className: "t", textContent: s.title ?? "" });
      const body = h("div", { className: "b" });
      if ((s.layout ?? "content") === "content" && !editable) body.append(h("ul", {}, ...String(s.body ?? "").split("\n").filter(Boolean).map(l => h("li", { textContent: l }))));
      else body.textContent = s.body ?? "";
      if (editable && !readOnly) {
        for (const [e, field, empty] of [[title, "title", "Click to add title"], [body, "body", (s.layout ?? "content") === "content" ? "Click to add text (a line each point)" : "Click to add subtitle"]]) {
          e.contentEditable = "plaintext-only";
          e.dataset.empty = empty;
          e.spellcheck = true;
          let timer = null;
          e.oninput = () => {
            typing = Date.now();
            clearTimeout(timer);
            timer = setTimeout(() => put(`s:${s.id}:${field}`, e.innerText.replace(/\n$/, "")), 500);
          };
          e.onblur = () => (clearTimeout(timer), put(`s:${s.id}:${field}`, e.innerText.replace(/\n$/, "")), drawStrip());
        }
      }
      if ((s.layout ?? "content") === "section" && !s.body && !editable) return box.append(title), box;
      box.append(title, body);
      return box;
    }
    function drawStrip() {
      const ss = slidesOf(data);
      strip.replaceChildren(
        ...ss.map((s, i) => {
          const t = h("div", { className: `thumb${s.id === curId ? " on" : ""}`, title: `Slide ${i + 1}` }, h("span", { className: "n", textContent: String(i + 1) }), h("div", { className: "mini" }, slideEl(s, false)));
          t.onclick = () => ((curId = s.id), draw());
          return t;
        }),
      );
    }
    function draw() {
      const ss = slidesOf(data);
      if (!ss.some(s => s.id === curId)) curId = ss[0]?.id ?? null;
      drawStrip();
      const s = ss.find(x => x.id === curId);
      stage.replaceChildren(s ? slideEl(s, true) : h("p", { className: "none", textContent: readOnly ? "No slides yet." : "No slides yet: ＋ New slide." }));
      if (document.activeElement !== notes) notes.value = s?.notes ?? "";
      layoutSel.value = s?.layout ?? "content";
      themeSel.value = data["deck:theme"] ?? "light";
    }
    let notesTimer = null;
    notes.oninput = () => {
      typing = Date.now();
      clearTimeout(notesTimer);
      notesTimer = setTimeout(() => curId && put(`s:${curId}:notes`, notes.value || null), 500);
    };

    // SLIDES made, copied, removed, moved — each one step to undo.
    const ordered = () => slidesOf(data);
    function add(layout = "content") {
      if (readOnly) return;
      const ss = ordered();
      const i = ss.findIndex(s => s.id === curId);
      const id = newId();
      group(() => {
        put(`s:${id}:pos`, String(between(ss[i]?.pos ?? ss.at(-1)?.pos, ss[i + 1]?.pos)));
        put(`s:${id}:layout`, layout);
      });
      curId = id;
      draw();
    }
    function duplicate() {
      const ss = ordered();
      const i = ss.findIndex(s => s.id === curId);
      if (readOnly || i < 0) return;
      const id = newId();
      group(() => {
        put(`s:${id}:pos`, String(between(ss[i].pos, ss[i + 1]?.pos)));
        for (const f of ["layout", "title", "body", "notes"]) if (ss[i][f] != null) put(`s:${id}:${f}`, ss[i][f]);
      });
      curId = id;
      draw();
    }
    function remove() {
      const ss = ordered();
      const i = ss.findIndex(s => s.id === curId);
      if (readOnly || i < 0) return;
      put(`s:${curId}:gone`, "1");
      curId = ss[i + 1]?.id ?? ss[i - 1]?.id ?? null;
      draw();
    }
    function moveBy(d) {
      const ss = ordered();
      const i = ss.findIndex(s => s.id === curId);
      const j = i + d;
      if (readOnly || i < 0 || j < 0 || j >= ss.length) return;
      const [a, b] = d < 0 ? [ss[j - 1]?.pos, ss[j].pos] : [ss[j].pos, ss[j + 1]?.pos];
      put(`s:${curId}:pos`, String(between(a, b)));
      draw();
    }

    // PRESENT: the slides full screen, one at a time — arrows, space, a click; Escape ends.
    function present() {
      const ss = ordered();
      if (!ss.length) return;
      let i = Math.max(0, ss.findIndex(s => s.id === curId));
      const show = h("div", { className: "sd-show", tabIndex: 0 });
      const st = h("div", { className: "stage" });
      const hint = h("div", { className: "hint" });
      show.append(st, hint);
      const go = n => {
        i = Math.max(0, Math.min(ss.length - 1, n));
        st.replaceChildren(slideEl(ss[i], false));
        hint.textContent = `${i + 1} / ${ss.length} · ← → · Esc`;
      };
      // (Full screen asked and left: refused or not, the show goes on in the page — said once, never fatal.)
      const end = () => (document.fullscreenElement && ctx.background("slides: leaving full screen", document.exitFullscreen()), show.remove(), removeEventListener("keydown", keys, true));
      const keys = e => {
        if (["ArrowRight", "ArrowDown", " ", "PageDown", "Enter"].includes(e.key)) (e.preventDefault(), go(i + 1));
        else if (["ArrowLeft", "ArrowUp", "PageUp", "Backspace"].includes(e.key)) (e.preventDefault(), go(i - 1));
        else if (e.key === "Escape") (e.preventDefault(), end());
      };
      show.onclick = () => (i === ss.length - 1 ? end() : go(i + 1));
      document.body.append(show);
      addEventListener("keydown", keys, true);
      document.addEventListener("fullscreenchange", () => !document.fullscreenElement && show.isConnected && end(), { once: true });
      if (show.requestFullscreen) ctx.background("slides: full screen", show.requestFullscreen());
      show.focus();
      go(i);
    }

    // ITS TOOLBAR GROUPS and MENUS (the office shell draws them).
    const btn = (label, title, run) => {
      const b = h("button", { type: "button", textContent: label, title, ariaLabel: title });
      b.onclick = run;
      return b;
    };
    const layoutSel = h("select", { title: "Layout", ariaLabel: "Layout" }, ...LAYOUTS.map(([v, t]) => h("option", { value: v, textContent: t })));
    layoutSel.onchange = () => curId && !readOnly && (put(`s:${curId}:layout`, layoutSel.value), draw());
    const themeSel = h("select", { title: "Theme", ariaLabel: "Theme" }, ...Object.keys(THEMES).map(t => h("option", { value: t, textContent: `Theme: ${t[0].toUpperCase()}${t.slice(1)}` })));
    themeSel.onchange = () => !readOnly && (put("deck:theme", themeSel.value), draw());
    const tools = [
      [btn("＋ Slide", "New slide", () => add()), layoutSel],
      [btn("⧉", "Duplicate slide", () => duplicate()), btn("🗑", "Delete slide", () => remove()), btn("↑", "Move slide up", () => moveBy(-1)), btn("↓", "Move slide down", () => moveBy(1))],
      [themeSel],
      [btn("▶ Present", "Present (full screen)", () => present())],
    ];
    const menus = {
      Insert: [{ label: "New slide", run: () => add(), keys: "Ctrl+M" }, ...LAYOUTS.map(([v, t]) => ({ label: `New slide: ${t}`, run: () => add(v) }))],
      Format: [...LAYOUTS.map(([v, t]) => ({ label: `Layout: ${t}`, run: () => curId && (put(`s:${curId}:layout`, v), draw()) })), "-", ...Object.keys(THEMES).map(t => ({ label: `Theme: ${t}`, run: () => (put("deck:theme", t), draw()) }))],
      View: [{ label: "Present", run: () => present(), keys: "F5", edits: false }],
    };
    el.addEventListener("keydown", e => {
      if (e.target.isContentEditable || e.target === notes) return;
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "m") (e.preventDefault(), add());
      if (e.key === "F5") (e.preventDefault(), present());
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        const ss = ordered();
        const i = ss.findIndex(s => s.id === curId);
        const n = ss[i + (e.key === "ArrowDown" ? 1 : -1)];
        if (n) (e.preventDefault(), (curId = n.id), draw());
      }
    });
    el.tabIndex = 0;
    draw();

    return {
      el,
      canvas: el,
      tools,
      menus,
      undo,
      redo,
      // ANOTHER's CHANGES: drawn — never under a field being typed in (its own read draws it when the person leaves).
      set(next) {
        data = { ...(next.cells ?? {}) };
        const a = document.activeElement;
        if (a && el.contains(a) && (a.isContentEditable || a === notes)) return drawStrip();
        draw();
      },
      busy: () => Date.now() - typing < 1500,
      readOnly: on => ((readOnly = on), (notes.readOnly = on), draw()),
    };
  }
  return { create };
}
