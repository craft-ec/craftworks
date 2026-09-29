// APP SETTINGS, a component: one APP's own settings in one space (Chat's, Board's, Notes'), for its owner and admins —
// a dialog of fields, each a `config` act in the space's log (`roles`: every member's app then follows it). An app
// names its fields; a field is a CHOICE (`options`: [[value, label]]) or TEXT. `extra(host)` draws what else the app
// keeps there (Chat: its channels). The space's own settings (members, roles, invites) are its Home's, never here.
//
//   const appSettings = await ctx.require("app-settings");
//   appSettings.open(sp, "board", "Board", [{ key: "post", label: "Who may post", options: [["everyone", "Every member"], ["admins", "Admins only"]] },
//     { key: "rules", label: "Rules", text: true }], { extra, saved })   // saved(changed): after a save, what changed
export async function start(ctx) {
  const roles = await ctx.require("roles");
  const style = document.createElement("style");
  style.textContent = `
    .cw-appset { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-4); width: min(460px, calc(100vw - 32px)); box-shadow: var(--cw-shadow-lg);
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

  async function open(sp, app, title, fields, { extra = null, saved = null } = {}) {
    const r = await roles.of(sp);
    const d = h("dialog", { className: "cw-appset" });
    const said = h("p", { className: "said" });
    const inputs = fields.map(f => {
      const now = r.config(app, f.key, f.options ? f.options[0][0] : "");
      const input = f.options
        ? h("select", {}, ...f.options.map(([v, label]) => h("option", { value: v, textContent: label, selected: v === now })))
        : h("textarea", { value: now ?? "", maxLength: 2000 });
      return { f, input, now };
    });
    const save = h("button", { type: "submit", className: "main", textContent: "Save" });
    const form = h(
      "form",
      {},
      ...inputs.map(({ f, input }) => h("label", {}, f.label, input)),
      h("div", { className: "row" }, said, h("button", { type: "button", textContent: "Close", onclick: () => d.close() }), save),
    );
    form.onsubmit = async e => {
      e.preventDefault();
      save.disabled = true;
      said.className = "said";
      said.textContent = "";
      try {
        // Only what changed: an act each.
        const changed = {};
        for (const { f, input, now } of inputs)
          if (input.value !== (now ?? "")) {
            await r.act({ act: "config", app, key: f.key, value: input.value });
            changed[f.key] = input.value;
          }
        // What follows from a change (Board made public: the space's acts published).
        if (saved) await saved(changed);
        said.className = "ok";
        said.textContent = "Saved.";
      } catch (err) {
        said.textContent = err?.message ?? String(err);
      } finally {
        save.disabled = false;
      }
    };
    const more = extra ? h("div", {}) : null;
    d.append(h("h3", { textContent: `${title} settings · ${sp.name}` }), form, more);
    if (extra) await extra(more);
    d.addEventListener("click", e => e.target === d && d.close());
    d.addEventListener("close", () => d.remove());
    document.body.append(d);
    d.showModal();
  }

  return { open };
}
