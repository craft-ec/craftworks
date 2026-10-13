// BUNDLE, an app: the bundles of each group (yours, or a space's that enables them) — make one, open it: its members
// of any kind (a video, a book, a post …) in order, moved up or down, taken out; its own page (comments, votes, who
// sees it) one link away. Items are added to a bundle from their own page ("Add to a bundle", `item-page`). UI only:
// bundles are `bundles`', items `items`'.
export async function mount(ctx, el) {
  const [bundles, items, kinds, cards, theme] = await Promise.all(["bundles", "items", "kinds", "cards", "theme"].map(n => ctx.require(n)));
  const where = await ctx.require("where");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  el.innerHTML = `<style>@layer apps {
    .bd { max-width: 900px; margin: 0 auto; padding: 0 16px; display: grid; gap: 12px; }
    .bd ul { list-style: none; padding: 0; display: grid; gap: 8px; }
    .bd li.m { display: grid; grid-template-columns: 1fr auto; gap: 8px; align-items: center; }
    .bd .row { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
    .bd .none { color: var(--cw-muted); }
  }</style><div class="bd"></div>`;
  const root = el.querySelector(".bd");
  const said = h("p", { className: "said", hidden: true });
  const fail = e => ((said.textContent = ctx.say(e)), (said.hidden = false));

  let at = null;
  async function list() {
    const sp = at.space && !at.outside ? at.space : null;
    const bs = await bundles.of(sp);
    const make = h("button", {
      type: "button",
      className: "main",
      textContent: "＋ New bundle",
      onclick: async () => {
        const name = prompt("The bundle's name");
        if (name) await bundles.make(name, sp).then(ref => (location.hash = at.href(`b/${encodeURIComponent(ref)}`)), fail);
      },
    });
    root.replaceChildren(
      h("div", { className: "row" }, h("h2", { textContent: sp ? `${(await ctx.require("space")).shown(sp)} · bundles` : "Your bundles" }), at.outside ? null : make),
      bs.length ? h("ul", {}, ...bs.map(b => h("li", {}, h("a", { href: at.href(`b/${encodeURIComponent(b.ref)}`), textContent: `${b.title} · ${b.members.length} item${b.members.length === 1 ? "" : "s"}` })))) : h("p", { className: "none", textContent: "No bundles yet." }),
      said,
    );
  }

  async function one(bref) {
    const b = (await bundles.of(at.space && !at.outside ? at.space : null)).find(x => x.ref === bref) ?? (await items.get(bref).then(it => (it ? { ref: it.ref, title: it.title, members: it.meta?.members ?? [], item: it } : null)));
    if (!b) return root.replaceChildren(h("p", { className: "none", textContent: "Not found." }));
    const got = await Promise.all(b.members.map(r => items.get(r).catch(ctx.quiet(() => null))));
    const rows = b.members.map((ref, i) => {
      const it = got[i];
      const label = it ? `${kinds.of(it.kind)?.label ?? it.kind} · ${it.title || String(it.body ?? "").slice(0, 60) || "untitled"}` : "(not readable here)";
      const tools = at.outside
        ? null
        : h(
            "span",
            { className: "row" },
            h("button", { type: "button", textContent: "↑", ariaLabel: "Up", onclick: () => bundles.move(bref, ref, -1).then(draw, fail) }),
            h("button", { type: "button", textContent: "↓", ariaLabel: "Down", onclick: () => bundles.move(bref, ref, 1).then(draw, fail) }),
            h("button", { type: "button", textContent: "Take out", onclick: () => bundles.drop(bref, ref).then(draw, fail) }),
          );
      return h("li", { className: "m" }, it ? h("a", { href: items.pageOf(ref, it.kind), textContent: label }) : h("span", { className: "none", textContent: label }), tools);
    });
    root.replaceChildren(
      h("p", {}, h("a", { href: at.base, textContent: "← Bundles" })),
      h("h2", { textContent: b.title || "untitled" }),
      h("p", {}, h("a", { href: items.pageOf(bref, "bundle"), textContent: "Its page: comments, votes, who sees it" })),
      rows.length ? h("ul", {}, ...rows) : h("p", { className: "none", textContent: "Nothing in it yet: “Add to a bundle” on any item's page." }),
      said,
    );
  }

  const draw = async () => {
    said.hidden = true;
    at = await where.of({ kind: "bundle", app: "bundle", yours: "Your bundles" });
    at.tabs([]);
    root.replaceChildren(theme.loading("Reading the bundles…"));
    const s = at.sub;
    return (s.startsWith("b/") ? one(decodeURIComponent(s.slice(2))) : list()).catch(fail);
  };
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/bundle" && draw(), { signal: ctx.signal });
  await draw();
}
