// LABEL MENU, a component: the label UI every page uses — the "Label note" menu (tick labels, type to filter or to
// create one) and a thing's labels as chips, both kept in step with the account's labels (the `edge` capability).
// The words shown are the page's (`title`).
//
//   const labelUI = await ctx.require("label-menu");
//   labelUI.key(itemRef)        // an ITEM's label key — one for every kind (`item:<ref>`)
//   labelUI.menu(anchor, key, { title: "Label note" })
//   el.append(labelUI.chips(key, { onPick: id => … }))
export async function start(ctx) {
  const labels = await (await ctx.require("edge")).labels();
  const { list, of, set, create } = labels;
  // ONE KEY per item, whatever its kind (a note, a post, a video): its labels are the account's one list.
  const key = ref => `item:${ref}`;
  const find = name => list().find(l => l.name.localeCompare(name.trim(), undefined, { sensitivity: "base" }) === 0);
  const fail = e => ctx.log("label failed", { what: e?.message ?? String(e) });
  // Notes' labels from before (`notes:<ref>`) moved to it, once (none left: nothing to do).
  for (const l of list())
    for (const old of labels.refs(l.id, "notes:"))
      await set(key(old.slice(6)), l.id, true)
        .then(() => set(old, l.id, false))
        .catch(fail);
  const t = { onChange: labels.onChange };

  const style = document.createElement("style");
  style.textContent = `@layer components {

    .cw-chips { display: flex; flex-wrap: wrap; gap: var(--cw-space-1); }
    .cw-chips:empty { display: none; }
    .cw-chip { font-size: var(--cw-text-xs); padding: 1px var(--cw-space-2); border-radius: var(--cw-radius-pill); border: 0;
      background: var(--cw-hover); cursor: pointer; }
    .cw-chip:hover { background: var(--cw-pressed); }
    .cw-labels-menu { position: fixed; z-index: 50; background: var(--cw-surface); color: var(--cw-fg); border-radius: var(--cw-radius);
      box-shadow: var(--cw-shadow-lg); padding: var(--cw-space-2) 0; width: 220px; max-height: 60vh; overflow: auto; font-size: var(--cw-text-sm); }
    .cw-labels-menu .head { padding: 0 var(--cw-space-3) var(--cw-space-1); font-weight: 600; }
    .cw-labels-menu input[type="text"] { width: calc(100% - 24px); margin: 0 var(--cw-space-3) 6px; border: 0; border-radius: 0;
      border-bottom: 1px solid var(--cw-line); background: transparent; outline: 0; padding: 2px 0; }
    .cw-labels-menu label, .cw-labels-menu .create { display: flex; align-items: center; gap: var(--cw-space-2);
      padding: var(--cw-space-1) var(--cw-space-3); cursor: pointer; overflow-wrap: anywhere; }
    .cw-labels-menu label:hover, .cw-labels-menu .create:hover { background: var(--cw-hover); }
    .cw-labels-menu .create { border: 0; background: none; width: 100%; text-align: left; }
}`;
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

  return { key, menu, chips };
}
