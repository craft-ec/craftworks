// APP STORE, an app (ARCHITECTURE §1, "The desktop"): every app in the catalog — this site's for now — with what it is,
// what it needs a space to enable, and Install or Remove (`installed`). An app not ours is offered only once apps run in
// their own frames (FOUNDATION.md build order 8): until then the catalog is the site's own apps.
export async function mount(ctx, el) {
  const installed = await ctx.require("installed");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  el.innerHTML = `<style>@layer components {
    .st { max-width: 860px; margin: 0 auto; padding: 16px; }
    .st ul { list-style: none; padding: 0; display: grid; gap: 10px; }
    .st li { display: grid; grid-template-columns: 40px 1fr auto; gap: 12px; align-items: center; padding: 10px; border: 1px solid var(--line, #8883); border-radius: 10px; }
    .st .ic { font-size: 28px; text-align: center; }
    .st small { color: var(--muted, #888); display: block; }
  }</style><div class="st"></div>`;
  const root = el.querySelector(".st");
  const said = h("p", { className: "said", hidden: true });
  async function draw() {
    const have = new Set((await installed.list()).map(a => a.route));
    const byCat = [...(ctx.apps ?? [])].sort((a, b) => String(a.category).localeCompare(String(b.category)) || a.name.localeCompare(b.name));
    root.replaceChildren(
      h("h2", { textContent: "App Store" }),
      h(
        "ul",
        {},
        ...byCat.map(a => {
          const on = have.has(a.route);
          const b = h("button", { type: "button", className: on ? "" : "main", textContent: installed.system(a.route) ? "System" : on ? "Remove" : "Install", disabled: installed.system(a.route) });
          b.onclick = () => installed.set(a.route, !on).then(draw, e => ((said.textContent = ctx.say(e)), (said.hidden = false)));
          const needs = (a.needs ?? []).length ? `In a space, it needs: ${a.needs.join(", ")}` : "Your own space only";
          return h("li", {}, h("span", { className: "ic", textContent: a.icon ?? "▫️" }), h("div", {}, h("strong", { textContent: `${a.name} · ${a.category ?? ""}` }), h("small", { textContent: a.about ?? "" }), h("small", { textContent: needs })), b);
        }),
      ),
      said,
    );
  }
  addEventListener("craftworks:installed", () => el.isConnected && draw(), { signal: ctx.signal });
  await draw();
}
