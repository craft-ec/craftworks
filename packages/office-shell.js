// OFFICE SHELL, a component: the ONE top panel every office app shows — Docs, Sheets, Slides — as a word processor,
// a spreadsheet and a presentation app all do: a TITLE BAR (the app's icon, the document's name — renamed in place —,
// where its changes are: "Saving…" / "All changes saved", and the item's own actions: Share, Save …), a MENU BAR
// (File · Edit · View · Insert · Format · Help: the shell's common entries, each app's own added to them), a TOOLBAR
// (common: undo, redo, print, zoom — then the app's groups), and the app's CANVAS under them (a page, a grid, a slide).
// The app gives its parts; the shell draws them the same way for every app.
//
//   const shell = (await ctx.require("office-shell")).create({
//     app: { name: "Docs", icon: "📝", home: "#/docs" }, title, readOnly, onRename(name),
//     actions,                     // an element: the item's actions (the bar's Share, Save …)
//     menus: { Insert: [...], Format: [...] },  // each entry { label, run, keys?, on?() } or "-" (a divider)
//     tools: [[el, el], [el]],     // the app's toolbar groups
//     edit: { undo, redo },        // the app's own (the toolbar's and Edit menu's Undo, Redo, and Ctrl+Z / Ctrl+Y)
//     canvas,                      // the app's body (zoomed by View → Zoom)
//   })
//   el.append(shell.el)   shell.status("saving" | "saved" | "offline" | text)   shell.readOnly(on)
export async function start(ctx) {
  const theme = await ctx.require("theme");
  const h = theme.h;
  // ITS LOOK, once, in the components' layer (a component's own: a page places it, never restyles it — theme.js's cascade).
  const css = document.createElement("style");
  css.textContent = `@layer components {
      .os { display: grid; grid-template-rows: auto auto auto 1fr; min-height: 80vh; background: var(--cw-bg-sunken, #f1f3f4); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); overflow: hidden; }
      .os .top { display: flex; align-items: center; gap: 10px; padding: 8px 12px 0; background: var(--cw-surface); }
      .os .icon { font-size: 26px; text-decoration: none; line-height: 1; }
      .os .name { font: inherit; font-size: 18px; border: 1px solid transparent; border-radius: var(--cw-radius-sm); padding: 2px 6px; background: none; color: var(--cw-fg); min-width: 8ch; max-width: 60vw; }
      .os .name:hover:not(:read-only), .os .name:focus { border-color: var(--cw-line); outline: none; }
      .os .state { color: var(--cw-muted); font-size: var(--cw-text-sm); white-space: nowrap; }
      .os .acts { margin-left: auto; display: flex; gap: 6px; align-items: center; }
      .os .menus { display: flex; gap: 2px; padding: 0 12px 4px 46px; background: var(--cw-surface); flex-wrap: wrap; }
      .os .menus > div { position: relative; }
      .os .menus > div > button { border: 0; background: none; color: var(--cw-fg); padding: 3px 8px; border-radius: var(--cw-radius-sm); cursor: pointer; font: inherit; font-size: var(--cw-text-sm); }
      .os .menus > div > button:hover, .os .menus > div.open > button { background: var(--cw-hover); }
      .os .drop { position: absolute; left: 0; top: 100%; z-index: 20; min-width: 220px; background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); box-shadow: var(--cw-shadow); padding: 6px 0; }
      .os .drop button { display: flex; justify-content: space-between; gap: 24px; width: 100%; border: 0; background: none; color: var(--cw-fg); text-align: left; padding: 6px 16px; cursor: pointer; font: inherit; font-size: var(--cw-text-sm); }
      .os .drop button:hover:not(:disabled) { background: var(--cw-hover); }
      .os .drop button:disabled { color: var(--cw-muted); cursor: default; }
      .os .drop .keys { color: var(--cw-muted); }
      .os .drop hr { border: 0; border-top: 1px solid var(--cw-line); margin: 6px 0; }
      .os .tools { position: sticky; top: 0; z-index: 10; display: flex; flex-wrap: wrap; align-items: center; gap: 2px; margin: 0 8px 8px; padding: 4px 8px; background: var(--cw-surface-2, var(--cw-surface)); border-radius: 24px; box-shadow: 0 1px 2px rgba(0,0,0,.08); }
      .os .tools button, .os .tools select { border: 0; background: none; color: var(--cw-fg); padding: 5px 8px; border-radius: var(--cw-radius-sm); cursor: pointer; font: inherit; font-size: var(--cw-text-sm); min-width: 30px; }
      .os .tools button:hover, .os .tools select:hover { background: var(--cw-hover); }
      .os .tools button[aria-pressed="true"] { background: var(--cw-accent-soft, #d3e3fd); color: var(--cw-accent, #0b57d0); }
      .os .tools .sep { width: 1px; height: 20px; background: var(--cw-line); margin: 0 6px; }
      .os .body { overflow: auto; min-width: 0; }
      .os .zoomed { transform-origin: top center; }
      .os.ro .tools, .os.ro .menus .edits { display: none; }
      @media print { .os .top, .os .menus, .os .tools { display: none; } .os { border: 0; background: none; } }
  }`;
  document.head.append(css);

  function create({ app = { name: "", icon: "", home: "#/" }, title = "", readOnly = false, onRename = null, actions = null, menus = {}, tools = [], edit = {}, canvas }) {
    const state = h("span", { className: "state" });
    const name = h("input", { className: "name", value: title, readOnly: readOnly || !onRename, ariaLabel: "Document name", spellcheck: false });
    let named = title;
    const rename = () => {
      const v = name.value.trim();
      if (!v) return void (name.value = named);
      if (v !== named && onRename) ((named = v), onRename(v));
    };
    name.onkeydown = e => e.key === "Enter" && (e.preventDefault(), name.blur());
    name.onblur = rename;

    // THE CANVAS, zoomed (View → Zoom; the toolbar's zoom).
    const zoomed = h("div", { className: "zoomed" }, canvas);
    const body = h("div", { className: "body" }, zoomed);
    let zoom = 100;
    const zoomSel = h("select", { title: "Zoom", ariaLabel: "Zoom" }, ...[50, 75, 90, 100, 125, 150, 200].map(z => h("option", { value: String(z), textContent: `${z}%` })));
    zoomSel.value = "100"; // (set once its options are in: an option marked before it is added is not kept)
    const setZoom = z => ((zoom = z), (zoomSel.value = String(z)), (zoomed.style.zoom = String(z / 100)));
    zoomSel.onchange = () => setZoom(Number(zoomSel.value));

    // THE MENU BAR: the shell's common entries, the app's added — one drop open at a time, closed on a click outside.
    const common = {
      File: [
        { label: "New", run: () => (location.hash = `${app.home}/new`) },
        { label: "Open…", run: () => (location.hash = app.home) },
        "-",
        { label: "Print", run: () => print(), keys: "Ctrl+P" },
      ],
      Edit: [
        { label: "Undo", run: () => edit.undo?.(), keys: "Ctrl+Z", edits: true, on: () => !!edit.undo },
        { label: "Redo", run: () => edit.redo?.(), keys: "Ctrl+Y", edits: true, on: () => !!edit.redo },
      ],
      View: [{ label: "Zoom in", run: () => setZoom(Math.min(200, zoom + 25)) }, { label: "Zoom out", run: () => setZoom(Math.max(50, zoom - 25)) }, { label: "Actual size", run: () => setZoom(100) }],
      Insert: [],
      Format: [],
      Help: [{ label: `About ${app.name}`, run: () => theme.ask(name, { text: `${app.name}: written together, live — every editor's changes merged, each kept in its own feed.`, ok: "Close" }) }],
    };
    let open = null;
    const close = () => (open?.classList.remove("open"), open?.querySelector(".drop")?.remove(), (open = null));
    const menuBar = h(
      "div",
      { className: "menus" },
      ...Object.keys(common).map(label => {
        const entries = [...common[label], ...(menus[label]?.length ? [...(common[label].length ? ["-"] : []), ...menus[label]] : [])];
        if (!entries.filter(e => e !== "-").length) return null;
        const wrap = h("div", { className: entries.every(e => e === "-" || e.edits) ? "edits" : "" });
        const b = h("button", { type: "button", textContent: label });
        b.onmousedown = e => e.preventDefault(); // (the document's selection kept)
        b.onclick = () => {
          const was = open === wrap;
          close();
          if (was) return;
          open = wrap;
          wrap.classList.add("open");
          wrap.append(
            h(
              "div",
              { className: "drop" },
              ...entries.map(e => {
                if (e === "-") return h("hr", {});
                const it = h("button", { type: "button", disabled: (readOnly && e.edits !== false && label !== "File" && label !== "View" && label !== "Help") || (e.on && !e.on()) }, h("span", { textContent: e.label }), h("span", { className: "keys", textContent: e.keys ?? "" }));
                it.onmousedown = ev => ev.preventDefault();
                it.onclick = () => (close(), e.run());
                return it;
              }),
            ),
          );
        };
        wrap.append(b);
        return wrap;
      }),
    );
    addEventListener("mousedown", e => open && !open.contains(e.target) && close());

    // THE TOOLBAR: common, then the app's groups.
    const tool = (label, title, run) => {
      const b = h("button", { type: "button", textContent: label, title, ariaLabel: title });
      b.onmousedown = e => e.preventDefault();
      b.onclick = run;
      return b;
    };
    const sep = () => h("span", { className: "sep" });
    const toolBar = h("div", { className: "tools" });
    const setTools = groups =>
      toolBar.replaceChildren(
        ...(edit.undo ? [tool("↶", "Undo (Ctrl+Z)", () => edit.undo()), tool("↷", "Redo (Ctrl+Y)", () => edit.redo?.())] : []),
        tool("🖨", "Print (Ctrl+P)", () => print()),
        zoomSel,
        ...groups.flatMap(g => [sep(), ...g]),
      );
    setTools(tools);

    // THE APP's KEYS for the shell's own: undo, redo (in the canvas, or anywhere in the shell but a field).
    const el = h("div", { className: `os${readOnly ? " ro" : ""}` }, h("div", { className: "top" }, h("a", { className: "icon", href: app.home, title: app.name, textContent: app.icon }), name, state, h("div", { className: "acts" }, actions)), menuBar, toolBar, body);
    el.addEventListener("keydown", e => {
      if (!(e.metaKey || e.ctrlKey) || readOnly) return;
      if (/^(INPUT|SELECT)$/.test(document.activeElement?.tagName ?? "")) return;
      const k = e.key.toLowerCase();
      if (k === "z" && !e.shiftKey && edit.undo) (e.preventDefault(), edit.undo());
      if ((k === "y" || (k === "z" && e.shiftKey)) && edit.redo) (e.preventDefault(), edit.redo());
    });

    return {
      el,
      // Where its changes are: "saving", "saved", "offline", or words of its own.
      status: s => (state.textContent = { saving: "Saving…", saved: "All changes saved", offline: "Not saved: offline" }[s] ?? String(s ?? "")),
      readOnly: on => ((readOnly = on), el.classList.toggle("ro", on), (name.readOnly = on || !onRename)),
      tools: setTools,
      title: t => ((named = t), (name.value = t)),
    };
  }
  return { create };
}
