// PIN BUTTON, a component: the one 📌 for every page — pressed when the thing is pinned, kept in step with the
// account's pins (the `edge` capability) by itself. A page only places it (position, size).
//
//   const pin = await ctx.require("pin-button");
//   el.append(pin.button("notes:<id>", { onSet: on => … }))
// A button is NEVER put inside a `#` link: the loader takes every click on one. Place it beside the link.
export async function start(ctx) {
  const pins = await (await ctx.require("edge")).pins();
  const { has, set } = pins;
  const t = { onChange: pins.onChange };

  // One look for a pin everywhere: unpinned grey and faint, pinned in colour. A page places it (position, size).
  const style = document.createElement("style");
  style.textContent = `
    .cw-pin { border: 0; background: none; cursor: pointer; line-height: 1; padding: var(--cw-space-1); border-radius: 50%; }
    .cw-pin[aria-pressed="false"] { filter: grayscale(1); opacity: .45; }
    .cw-pin[aria-pressed="true"] { opacity: 1; }
    .cw-pin:hover { background: var(--cw-hover); }
    .cw-pin.small { font-size: 14px; }`;
  document.head.append(style);

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

  return { button };
}
