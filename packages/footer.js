// FOOTER: one line. The app's own component, like the header.
export function mount(ctx, el) {
  el.innerHTML = `<p style="border-top:1px solid #8884;padding-top:8px;color:#888;font-size:.85rem">
    ${ctx.app} · served from freenet · every part checked by its hash · press <em>trace</em> to see what loaded</p>`;
}
