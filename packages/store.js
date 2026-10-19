// APP STORE, an app (ARCHITECTURE §1, "The desktop"): every app in the catalog — this site's for now — with what it is,
// what it needs a space to enable, and Install or Remove (`installed`). An app not ours is offered only once apps run in
// their own frames (FOUNDATION.md build order 8): until then the catalog is the site's own apps.
export async function mount(ctx, el) {
  const installed = await ctx.require("installed");
  const h = (await ctx.require("theme")).h;
  el.innerHTML = `<style>@layer components {
    .st { padding: 16px; }
    .st ul { list-style: none; padding: 0; display: grid; gap: 10px; }
    .st li { display: grid; grid-template-columns: 40px 1fr auto; gap: 12px; align-items: center; padding: 10px; border: 1px solid var(--line, #8883); border-radius: 10px; }
    .st .ic { font-size: 28px; text-align: center; }
    .st small { color: var(--muted, #888); display: block; }
  }</style><div class="st"></div>`;
  const root = el.querySelector(".st");
  const said = h("p", { className: "said", hidden: true });
  // THE CATALOG: a VIEW (`ctx.view`) — what is installed read, drawn, and drawn again when it changes (here or on
  // another device of the account).
  const list = h("div");
  root.replaceChildren(list, said); // (its name is the header's)
  const v = ctx.view(list, {
    read: async () => [...(await installed.list()).map(a => a.route)].sort(),
    draw: haveList => {
      const have = new Set(haveList);
      const byCat = [...(ctx.apps ?? [])].sort((a, b) => String(a.category).localeCompare(String(b.category)) || a.name.localeCompare(b.name));
      return h(
        "ul",
        {},
        ...byCat.map(a => {
          const on = have.has(a.route);
          const b = h("button", { type: "button", className: on ? "" : "main", textContent: installed.system(a.route) ? "System" : on ? "Remove" : "Install", disabled: installed.system(a.route) });
          b.onclick = () => installed.set(a.route, !on).then(() => v.refresh(), e => ((said.textContent = ctx.say(e)), (said.hidden = false)));
          const needs = (a.needs ?? []).length ? `In a space, it needs: ${a.needs.join(", ")}` : "Your own space only";
          return h("li", {}, h("span", { className: "ic", textContent: a.icon ?? "▫️" }), h("div", {}, h("strong", { textContent: `${a.name} · ${a.category ?? ""}` }), h("small", { textContent: a.about ?? "" }), h("small", { textContent: needs })), b);
        }),
      );
    },
    // (Nothing installed is still a catalog to draw.)
    empty: () => false,
  });
  addEventListener("craftworks:installed", () => el.isConnected && v.refresh(), { signal: ctx.signal });
}
