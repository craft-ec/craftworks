// LABEL MENU, a component: the label UI every page uses — the "Label note" menu (tick labels, type to filter or to
// create one) and a thing's labels as chips, both kept in step with the account's labels (the `edge` capability).
// The words shown are the page's (`title`).
//
//   const labelUI = await ctx.require("label-menu");
//   labelUI.menu(anchor, ref, { title: "Label note" })
//   el.append(labelUI.chips(ref, { onPick: id => … }))
export async function start(ctx) {
  const labels = await (await ctx.require("edge")).labels();
  const { list, of, set, create } = labels;
  const find = name => list().find(l => l.name.localeCompare(name.trim(), undefined, { sensitivity: "base" }) === 0);
  const fail = e => ctx.log("label failed", { what: e?.message ?? String(e) });
  const t = { onChange: labels.onChange };

  const style = document.createElement("style");
  style.textContent = `
    .cw-chips { display: flex; flex-wrap: wrap; gap: 4px; }
    .cw-chips:empty { display: none; }
    .cw-chip { font: inherit; font-size: .75rem; padding: 1px 8px; border-radius: 10px; border: 0; background: #0000000f;
      color: inherit; cursor: pointer; }
    .cw-chip:hover { background: #0000001f; }
    .cw-labels-menu { position: fixed; z-index: 50; background: Canvas; color: CanvasText; border-radius: 8px;
      box-shadow: 0 2px 12px #0004; padding: 8px 0; width: 220px; max-height: 60vh; overflow: auto; font-size: .9rem; }
    .cw-labels-menu .head { padding: 0 12px 4px; font-weight: 600; }
    .cw-labels-menu input[type="text"] { font: inherit; width: calc(100% - 24px); margin: 0 12px 6px; border: 0;
      border-bottom: 1px solid #8886; background: transparent; color: inherit; outline: 0; padding: 2px 0; }
    .cw-labels-menu label, .cw-labels-menu .create { display: flex; align-items: center; gap: 8px; padding: 4px 12px;
      cursor: pointer; overflow-wrap: anywhere; }
    .cw-labels-menu label:hover, .cw-labels-menu .create:hover { background: #8881; }
    .cw-labels-menu .create { border: 0; background: none; font: inherit; color: inherit; width: 100%; text-align: left; }`;
  document.head.append(style);

  // CHIPS: a thing's labels, following the table.
  const chipSets = new Set();
  const paint = box => {
    box.replaceChildren(
      ...of(box.dataset.ref).map(l => {
        const b = Object.assign(document.createElement("button"), { type: "button", className: "cw-chip", textContent: l.name, title: `Show “${l.name}”` });
        b.onclick = e => (e.stopPropagation(), box.onPick?.(l.id));
        return b;
      }),
    );
  };
  function chips(ref, { onPick } = {}) {
    const box = Object.assign(document.createElement("div"), { className: "cw-chips" });
    box.dataset.ref = ref;
    box.onPick = onPick;
    paint(box);
    chipSets.add(box);
    return box;
  }

  // THE MENU under `anchor`: tick labels on and off; type to filter, or to create one and put it on at once.
  let openMenu = null;
  function menu(anchor, ref, { title = "Label note" } = {}) {
    openMenu?.remove();
    const box = Object.assign(document.createElement("div"), { className: "cw-labels-menu" });
    box.innerHTML = `<div class="head"></div><input type="text" placeholder="Enter label name" maxlength="50"><div class="items"></div>`;
    box.querySelector(".head").textContent = title;
    const input = box.querySelector("input");
    const items = box.querySelector(".items");
    const fill = () => {
      const q = input.value.trim();
      const on = new Set(of(ref).map(l => l.id));
      const shown = list().filter(l => !q || l.name.toLowerCase().includes(q.toLowerCase()));
      items.replaceChildren(
        ...shown.map(l => {
          const row = document.createElement("label");
          const cb = Object.assign(document.createElement("input"), { type: "checkbox", checked: on.has(l.id) });
          cb.onchange = () => set(ref, l.id, cb.checked).catch(fail);
          row.append(cb, Object.assign(document.createElement("span"), { textContent: l.name }));
          return row;
        }),
      );
      if (q && !find(q)) {
        const c = Object.assign(document.createElement("button"), { type: "button", className: "create" });
        c.textContent = `+ Create “${q}”`;
        c.onclick = async () => {
          input.value = "";
          try {
            await set(ref, await create(q), true);
          } catch (e) {
            fail(e);
          }
          fill();
        };
        items.append(c);
      }
    };
    input.oninput = fill;
    input.onkeydown = e => e.key === "Enter" && (e.preventDefault(), items.querySelector(".create")?.click());
    fill();
    box.refresh = fill;
    box.onclick = e => e.stopPropagation();
    // Inside an open dialog when asked from one: while a modal dialog is open, everything outside it is inert.
    (anchor.closest("dialog") ?? document.body).append(box);
    const r = anchor.getBoundingClientRect();
    box.style.left = `${Math.max(8, Math.min(r.left, innerWidth - 228))}px`;
    box.style.top = `${Math.min(r.bottom + 4, innerHeight - 80)}px`;
    openMenu = box;
    input.focus();
    const close = e => {
      if (box.contains(e.target)) return;
      box.remove();
      if (openMenu === box) openMenu = null;
      removeEventListener("click", close, true);
    };
    setTimeout(() => addEventListener("click", close, true));
    return box;
  }

  t.onChange(() => {
    for (const box of chipSets) {
      if (!box.isConnected) chipSets.delete(box);
      else paint(box);
    }
    if (openMenu?.isConnected) openMenu.refresh();
  });

  return { menu, chips };
}
