// LABELS, a service: the account's labels, for every page and app. One table (`labels`, the account's):
//   `l/<id>`          { name }   a label
//   `a/<id>/<ref>`    { at }     that label on a thing, named as pins name it (`notes:<id>`, …)
// and the label UI every page uses, so labels look and behave the same everywhere:
//
//   const labels = await ctx.require("labels");
//   labels.list()   labels.of("notes:<id>")   labels.refs(id, "notes:")   labels.onChange(fn)
//   await labels.create(name)   labels.rename(id, name)   labels.remove(id)   labels.set(ref, id, on)   labels.clear(ref)
//   labels.menu(anchor, ref)    // "Label note": a box of labels to tick, and "Create “…”" from what is typed
//   el.append(labels.chips(ref, { onPick: id => … }))   // the thing's labels as chips, kept in step by itself
export async function start(ctx) {
  const t = await (await ctx.require("data")).table("labels");

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

  const parse = v => {
    try {
      return JSON.parse(v) ?? {};
    } catch {
      return {};
    }
  };
  const byName = (a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  const list = () =>
    t.rows()
      .filter(r => r.key.startsWith("l/"))
      .map(r => ({ id: r.key.slice(2), name: String(parse(r.value).name ?? "") }))
      .filter(l => l.name)
      .sort(byName);
  const assigned = () => t.rows().filter(r => r.key.startsWith("a/")).map(r => {
    const rest = r.key.slice(2);
    const i = rest.indexOf("/");
    return { id: rest.slice(0, i), ref: rest.slice(i + 1) };
  });
  const of = ref => {
    const ids = new Set(assigned().filter(a => a.ref === ref).map(a => a.id));
    return list().filter(l => ids.has(l.id));
  };
  const refs = (id, prefix = "") => assigned().filter(a => a.id === id && a.ref.startsWith(prefix)).map(a => a.ref);
  const find = name => list().find(l => l.name.localeCompare(name.trim(), undefined, { sensitivity: "base" }) === 0);

  const newId = () => `${Date.now().toString(36)}${[...crypto.getRandomValues(new Uint8Array(3))].map(b => b.toString(16).padStart(2, "0")).join("")}`;
  async function create(name) {
    name = name.trim().slice(0, 50);
    if (!name) throw new Error("a label needs a name");
    const same = find(name);
    if (same) return same.id;
    const id = newId();
    await t.put(`l/${id}`, JSON.stringify({ name }));
    return id;
  }
  const rename = (id, name) => {
    name = name.trim().slice(0, 50);
    const same = find(name);
    if (!name || (same && same.id !== id)) return Promise.reject(new Error(name ? `there is already a label “${same.name}”` : "a label needs a name"));
    return t.put(`l/${id}`, JSON.stringify({ name }));
  };
  // A label goes with every use of it.
  async function remove(id) {
    for (const ref of refs(id)) await t.remove(`a/${id}/${ref}`);
    await t.remove(`l/${id}`);
  }
  const set = (ref, id, on) => (on ? t.put(`a/${id}/${ref}`, JSON.stringify({ at: Date.now() })) : t.remove(`a/${id}/${ref}`));
  // A thing that is gone takes its labels with it.
  async function clear(ref) {
    for (const l of of(ref)) await t.remove(`a/${l.id}/${ref}`);
  }
  const fail = e => ctx.log("label failed", { what: e?.message ?? String(e) });

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

  return { list, of, refs, create, rename, remove, set, clear, chips, menu, onChange: t.onChange };
}
