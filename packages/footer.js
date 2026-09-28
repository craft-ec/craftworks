// FOOTER: one line. The app's own component, like the header.
export function mount(ctx, el) {
  el.innerHTML = `<p style="border-top:1px solid var(--cw-line);padding-top:var(--cw-space-2);color:var(--cw-muted);font-size:var(--cw-text-sm)"></p>`;
  el.querySelector("p").textContent = `${ctx.app} · on Freenet`;
}
