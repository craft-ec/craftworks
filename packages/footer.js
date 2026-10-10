// FOOTER: one line. The app's own component, like the header.
// (A new version of the app reaches an open page by itself: the loader loads it once nothing would be lost.)
export function mount(ctx, el) {
  el.innerHTML = `<p style="margin:0;display:flex;align-items:center;height:var(--cw-bar-low);box-sizing:border-box;border-top:1px solid var(--cw-line);color:var(--cw-muted);font-size:var(--cw-text-sm)"></p>`;
  el.querySelector("p").textContent = `${ctx.app} · on Freenet`;
  // Every open page keeps the network's clock going (`beacon`: it extends the beacon when it stalls).
  ctx.require("beacon").catch(e => ctx.log("beacon", { what: e.message ?? String(e) }));
  // …and publishes the current epoch's statement if nobody has (`statement`: anyone may).
  ctx.require("statement").catch(e => ctx.log("statement", { what: e.message ?? String(e) }));
  ctx.require("keeper").catch(e => ctx.log("keeper", { what: e.message ?? String(e) }));
}
