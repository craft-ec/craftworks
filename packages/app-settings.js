// PERMISSIONS, a component (the one settings dialog of an app in a space, or of the space itself): a list of fields,
// each either a POLICY — who may do an action at a path (`roles`' access: inherited along the path; "Inherit" drops
// this path's own policy so its parent's applies) — or a CONTENT setting of the app (a `config` act: Board's rules).
// For the space's owner and admins; reading in public is the owner's. `extra(host)` draws what else the app keeps
// there (Chat: its channels, each with its own policy).
//
//   const permissions = await ctx.require("app-settings");
//   permissions.open(sp, "Board", [
//     { action: "post", path: "board", label: "Who may post" },
//     { key: "rules", app: "board", label: "Rules" }], { extra, saved })   // saved(changed): after a save
//   permissions.who(r, path, action)   // a <select> for one policy (for an app's own rows: a channel)
export async function start(ctx) {
  const roles = await ctx.require("roles");
  const style = document.createElement("style");
  style.textContent = `
    .cw-appset { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-4); width: min(480px, calc(100vw - 32px)); box-shadow: var(--cw-shadow-lg);
      background: var(--cw-surface); color: var(--cw-fg); }
    .cw-appset h3 { margin: 0 0 var(--cw-space-3); font-size: 1.05rem; }
    .cw-appset h4 { margin: var(--cw-space-3) 0 var(--cw-space-2); font-size: var(--cw-text-xs); letter-spacing: .08em; text-transform: uppercase; color: var(--cw-muted); }
    .cw-appset label { display: grid; gap: 4px; margin-bottom: var(--cw-space-3); font-size: var(--cw-text-sm); font-weight: 600; }
    .cw-appset select, .cw-appset textarea, .cw-appset input { font: inherit; padding: 6px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
    .cw-appset textarea { min-height: 90px; resize: vertical; }
    .cw-appset button { font: inherit; cursor: pointer; border: 1px solid var(--cw-line); background: none; color: var(--cw-fg); border-radius: var(--cw-radius-sm); padding: 4px var(--cw-space-3); }
    .cw-appset button.main { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
    .cw-appset .row { display: flex; gap: var(--cw-space-2); justify-content: flex-end; align-items: center; }
    .cw-appset .said { color: var(--cw-danger); font-size: var(--cw-text-sm); margin: 0; flex: 1; }
    .cw-appset .ok { color: var(--cw-muted); font-size: var(--cw-text-sm); margin: 0; flex: 1; }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const NAMES = { anyone: "Anyone (public)", members: "Members", admins: "Admins", owner: "The owner", nobody: "Nobody" };
  const parentOf = path => (path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : path ? "" : null);

  // ONE POLICY's choice: Inherit (what the parent says), or a who. Reading by anyone is offered to the owner only.
  function who(r, path, action, { me = null } = {}) {
    const own = r.policiesAt(path)[action] ?? "";
    const parent = parentOf(path);
    const inherited = parent == null ? "members" : r.policy(parent, action);
    const options = ["anyone", "members", "admins", "owner", "nobody"].filter(w => w !== "anyone" || action === "read" || action === "join");
    const sel = h(
      "select",
      { ariaLabel: `${action} at ${path || "the space"}` },
      // The space itself has nothing above it: its "inherit" is the built-in default (a tenant, later, will be its parent).
      h("option", { value: "", textContent: `${parent == null ? "Default" : "Inherit"} (${NAMES[inherited] ?? inherited})` }),
      ...options.map(w => h("option", { value: w, textContent: NAMES[w] })),
    );
    sel.value = own;
    sel.dataset.was = own;
    sel.dataset.path = path;
    sel.dataset.action = action;
    if (action === "read" && me && r.role(me) !== "owner") sel.disabled = true;
    return sel;
  }
  // Save a policy <select> if it changed: `inherit` drops the override.
  async function save(r, sel) {
    if (sel.value === sel.dataset.was) return false;
    await r.act({ act: "policy", path: sel.dataset.path, action: sel.dataset.action, who: sel.value || "inherit" });
    sel.dataset.was = sel.value;
    return true;
  }

  async function open(sp, title, fields, { extra = null, saved = null } = {}) {
    const r = await roles.of(sp);
    const me = (await (await ctx.require("space")).account()).id;
    const d = h("dialog", { className: "cw-appset" });
    const said = h("p", { className: "said" });
    const inputs = fields.map(f => {
      if (f.action) return { f, input: who(r, f.path, f.action, { me }) };
      const now = r.config(f.app, f.key, "");
      return { f, input: h("textarea", { value: now ?? "", maxLength: 2000 }), now };
    });
    const btn = h("button", { type: "submit", className: "main", textContent: "Save" });
    const form = h(
      "form",
      {},
      ...inputs.map(({ f, input }) => h("label", {}, f.label, input)),
      h("div", { className: "row" }, said, h("button", { type: "button", textContent: "Close", onclick: () => d.close() }), btn),
    );
    const more = extra ? h("div", {}) : null;
    form.onsubmit = async e => {
      e.preventDefault();
      btn.disabled = true;
      said.className = "said";
      said.textContent = "";
      try {
        const changed = {};
        for (const { f, input, now } of inputs) {
          if (f.action) {
            if (await save(r, input)) changed[`${f.path}|${f.action}`] = input.value || "inherit";
          } else if (input.value !== (now ?? "")) {
            await r.act({ act: "config", app: f.app, key: f.key, value: input.value });
            changed[f.key] = input.value;
          }
        }
        // The app's own rows (a channel's policy): saved the same way.
        for (const sel of more?.querySelectorAll("select[data-path]") ?? []) if (await save(r, sel)) changed[`${sel.dataset.path}|${sel.dataset.action}`] = sel.value || "inherit";
        if (saved) await saved(changed);
        said.className = "ok";
        said.textContent = "Saved.";
      } catch (err) {
        said.textContent = err?.message ?? String(err);
      } finally {
        btn.disabled = false;
      }
    };
    d.append(h("h3", { textContent: `${title} · ${sp.name}` }), form, more);
    if (extra) await extra(more, r);
    d.addEventListener("click", e => e.target === d && d.close());
    d.addEventListener("close", () => d.remove());
    document.body.append(d);
    d.showModal();
  }

  return { open, who: (r, path, action) => who(r, path, action) };
}
