// HISTORY, a component: an item's earlier VERSIONS — every kind alike, in a personal space or a shared one (`items.history`:
// the roots its table logged, `storage`) — each with who changed it and when, and RESTORE (an edit back to it, by whoever
// may edit it now). Kept as the plan says: the baseline 7 days, the newest 20.
//
//   const history = await ctx.require("history");
//   el.append(history.button(ref, { current: item, restored: () => … }))
export async function start(ctx) {
  const [items, directory, theme] = await Promise.all(["items", "directory", "theme"].map(n => ctx.require(n)));
  const style = document.createElement("style");
  style.textContent = `
    dialog.cw-history { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-3) var(--cw-space-4);
      width: min(560px, calc(100vw - 32px)); max-height: calc(100vh - 32px); background: var(--cw-surface); color: var(--cw-fg);
      box-shadow: var(--cw-shadow); display: grid; grid-template-rows: auto minmax(0, 1fr) auto; gap: var(--cw-space-2); }
    dialog.cw-history:not([open]) { display: none; }
    dialog.cw-history h3 { margin: 0; font-size: 1rem; }
    dialog.cw-history .list { overflow-y: auto; display: grid; gap: var(--cw-space-2); }
    dialog.cw-history .v { border: 1px solid var(--cw-line); border-radius: var(--cw-radius); padding: var(--cw-space-2) var(--cw-space-3); display: grid; gap: 4px; }
    dialog.cw-history .v .who { font-size: var(--cw-text-xs); color: var(--cw-muted); display: flex; justify-content: space-between; gap: var(--cw-space-2); align-items: center; }
    dialog.cw-history .v .t { font-weight: 600; }
    dialog.cw-history .v .b { white-space: pre-wrap; overflow-wrap: anywhere; max-height: 9em; overflow: hidden; }
    dialog.cw-history .none, dialog.cw-history .said { color: var(--cw-muted); margin: 0; }
    dialog.cw-history .end { justify-self: end; }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const name = async did => (did ? directory.shown(did, await directory.handle(did).catch(() => null)) : "someone");
  const when = t => (t ? new Date(t).toLocaleString() : "");
  const stamp = it => it?.edited || it?.at || 0;

  // The versions shown in a dialog over `anchor`'s page (inside an open dialog when asked from one: outside it is inert).
  // `mayRestore`: this person may edit it (its author, or where its space lets them) — else its history to read only.
  async function open(anchor, ref, { current = null, restored = () => {}, mayRestore = true } = {}) {
    const said = h("p", { className: "said" });
    const list = h("div", { className: "list" }, theme.loading("Reading its history…"));
    const close = h("button", { type: "button", className: "end", textContent: "Close" });
    const d = h("dialog", { className: "cw-history" }, h("h3", { textContent: "History" }), list, h("div", {}, said, close));
    close.onclick = () => d.close();
    d.addEventListener("close", () => d.remove());
    // A click outside it (on its backdrop: the dialog itself, outside its box) closes it, as Close does.
    d.addEventListener("click", e => {
      if (e.target !== d) return;
      const r = d.getBoundingClientRect();
      if (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom) d.close();
    });
    (anchor.closest("dialog") ?? document.body).append(d);
    d.showModal();
    let vs = [];
    try {
      // The version it is NOW is not an earlier one.
      vs = (await items.history(ref)).filter(v => !current || stamp(v) !== stamp(current));
    } catch (e) {
      said.textContent = `Its history did not read: ${e?.message ?? e}`;
    }
    if (!vs.length) {
      list.replaceChildren(h("p", { className: "none", textContent: "No earlier versions yet. Each saved change is kept for 7 days (the last 20)." }));
      return;
    }
    list.replaceChildren(
      ...(await Promise.all(
        vs.map(async v => {
          const restore = h("button", { type: "button", textContent: "Restore" });
          restore.onclick = async () => {
            restore.disabled = true;
            said.textContent = "Restoring…";
            try {
              await items.editItem(ref, v.body, { title: v.title ?? undefined, files: v.files, meta: v.meta });
              said.textContent = "Restored.";
              d.close();
              restored(v);
            } catch (e) {
              said.textContent = `Not restored: ${e?.message ?? e}`;
              restore.disabled = false;
            }
          };
          const by = v.edited ? `Edited by ${await name(v.editor ?? v.by)} · ${when(v.edited)}` : `Made by ${await name(v.by)} · ${when(v.at)}`;
          return h("div", { className: "v" }, h("div", { className: "who" }, h("span", { textContent: by }), mayRestore ? restore : null), v.title ? h("div", { className: "t", textContent: v.title }) : null, h("div", { className: "b", textContent: v.body }));
        }),
      )),
    );
  }

  return {
    open,
    button: (ref, opts = {}) => {
      const b = h("button", { type: "button", className: "history-b", title: "History", textContent: "🕘" });
      b.onclick = () => open(b, ref, typeof opts === "function" ? opts() : opts);
      return b;
    },
  };
}
