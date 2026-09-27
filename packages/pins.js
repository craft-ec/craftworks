// PINS, a service: the account's pins, for every page and app. One table (`pins`, the account's), keyed by what is
// pinned — `app:/notes` (an app on the desktop), `notes:<id>` (a note), … — and one pin BUTTON, so a pin looks and
// behaves the same everywhere and a page never builds its own.
//
//   const pins = await ctx.require("pins");
//   pins.has("notes:<id>")   pins.refs("app:")   await pins.set(ref, on)   pins.onChange(fn)
//   el.append(pins.button("notes:<id>", { onSet: on => … }))   // 📌, pressed when pinned, kept in step by itself
//
// A button is NEVER put inside a `#` link: the loader takes every click on one. Place it beside the link.
export async function start(ctx) {
  const t = await (await ctx.require("data")).table("pins");

  // One look for a pin everywhere: unpinned grey and faint, pinned in colour. A page places it (position, size).
  const style = document.createElement("style");
  style.textContent = `
    .cw-pin { border: 0; background: none; cursor: pointer; font: inherit; line-height: 1; padding: 4px; border-radius: 50%; }
    .cw-pin[aria-pressed="false"] { filter: grayscale(1); opacity: .45; }
    .cw-pin[aria-pressed="true"] { opacity: 1; }
    .cw-pin:hover { background: #8882; }`;
  document.head.append(style);

  const has = ref => t.rows().some(r => r.key === ref);
  const refs = prefix => t.rows().map(r => r.key).filter(k => !prefix || k.startsWith(prefix));
  const set = (ref, on) => (on ? t.put(ref, JSON.stringify({ at: Date.now() })) : t.remove(ref));

  // Every live button follows the table: a pin changed anywhere (this page, another tab, another node) shows at once.
  const buttons = new Set();
  const paint = (b, on) => {
    b.setAttribute("aria-pressed", String(on));
    b.title = on ? "Unpin" : "Pin";
  };
  t.onChange(() => {
    for (const b of buttons) {
      if (!b.isConnected) buttons.delete(b);
      else paint(b, has(b.dataset.ref));
    }
  });

  function button(ref, { onSet, className = "" } = {}) {
    const b = Object.assign(document.createElement("button"), { type: "button", className: `cw-pin ${className}`.trim(), textContent: "📌" });
    b.dataset.ref = ref;
    paint(b, has(ref));
    b.onclick = e => {
      e.stopPropagation();
      const on = !has(ref);
      paint(b, on); // at once; the table's change confirms it (or puts it back if the write fails)
      onSet?.(on);
      set(ref, on).catch(err => {
        paint(b, has(ref));
        ctx.log("pin failed", { what: `${ref}: ${err?.message ?? err}` });
      });
    };
    buttons.add(b);
    return b;
  }

  return { has, refs, set, onChange: t.onChange, button };
}
