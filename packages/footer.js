// FOOTER: one line. The app's own component, like the header.
export function mount(ctx, el) {
  el.innerHTML = `<p style="margin:0;display:flex;align-items:center;height:var(--cw-bar-low);box-sizing:border-box;border-top:1px solid var(--cw-line);color:var(--cw-muted);font-size:var(--cw-text-sm)"></p>`;
  el.querySelector("p").textContent = `${ctx.app} · on Freenet`;
}
