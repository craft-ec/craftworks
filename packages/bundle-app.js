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
  // Each screen's changing part is a VIEW (`ctx.view`): read, drawn, drawn again when it changes; stopped on a new route.
  let views = [];
  async function list() {
    const sp = at.space && !at.outside ? at.space : null;
    // DISCOVER (and someone else's, or a space from outside): the bundles they show in public — as every app's.
    const others = at.who === "discover" || at.who === "person" || at.outside;
    const read = async () =>
      others
        ? (await at.read({ kinds: ["bundle"] })).map(b => ({ ref: b.ref, title: b.title || "untitled", members: Array.isArray(b.meta?.members) ? b.meta.members : [], item: b }))
        : await bundles.of(sp);
    // WHO SEES a new one: in your space just you, or anyone (public — listed in Discover); in a shared space its members,
    // or anyone.
    const who = h("select", { ariaLabel: "Who sees it" }, ...(sp ? [["members", "Its members"], ["public", "Anyone (public)"]] : [["private", "Only you"], ["public", "Anyone (public)"]]).map(([v, t]) => h("option", { value: v, textContent: t })));
    const make = h("button", {
      type: "button",
      className: "main",
      textContent: "＋ New bundle",
      onclick: async () => {
        const name = prompt("The bundle's name");
        if (name) await bundles.make(name, sp, { audience: who.value }).then(ref => (location.hash = at.href(`b/${encodeURIComponent(ref)}`)), fail);
      },
    });
    const listEl = h("div");
    root.replaceChildren(h("div", { className: "row" }, h("h2", { textContent: at.who === "discover" ? "Public bundles" : at.who === "person" ? "Their bundles" : sp ? `${(await ctx.require("space")).shown(sp)} · bundles` : "Your bundles" }), others ? null : who, others ? null : make), listEl, said);
    views.push(
      ctx.view(listEl, {
        read,
        draw: bs => h("ul", {}, ...bs.map(b => h("li", {}, h("a", { href: at.href(`b/${encodeURIComponent(b.ref)}`), textContent: `${b.title} · ${b.members.length} item${b.members.length === 1 ? "" : "s"}${b.item?.audience === "public" || b.item?.pub ? " · public" : ""}` })))),
        looks: { empty: others ? "No public bundles yet." : "No bundles yet.", loading: () => theme.loading("Reading the bundles…") },
      }),
    );
  }

  async function one(bref) {
    const meId = await (await ctx.require("space")).me();
    const mineHere = at.who === "mine" || (at.who === "space" && !at.outside);
    const body = h("div");
    root.replaceChildren(h("p", {}, h("a", { href: at.base, textContent: "← Bundles" })), body, said);
    const v = ctx.view(body, {
      read: async () => {
        const b = (mineHere ? (await bundles.of(at.space && !at.outside ? at.space : null)).find(x => x.ref === bref) : null) ?? (await items.get(bref, { outside: await items.outsideOf(bref) }).then(it => (it ? { ref: it.ref, title: it.title, members: it.meta?.members ?? [], item: it } : null)));
        return b ? { b, got: await Promise.all(b.members.map(r => items.get(r))) } : null;
      },
      empty: d => !d,
      looks: { empty: "Not found.", loading: () => theme.loading("Reading the bundle…") },
      draw: ({ b, got }) => drawOne(bref, b, got, mineHere, meId, () => v.refresh()),
    });
    views.push(v);
  }
  function drawOne(bref, b, got, mineHere, meId, again) {
    const rows = b.members.map((ref, i) => {
      const it = got[i];
      const label = it ? `${kinds.of(it.kind)?.label ?? it.kind} · ${it.title || String(it.body ?? "").slice(0, 60) || "untitled"}` : "(not readable here)";
      // (Only its author changes a bundle: the tools are theirs.)
      const tools = !mineHere || b.item?.by !== meId
        ? null
        : h(
            "span",
            { className: "row" },
            h("button", { type: "button", textContent: "↑", ariaLabel: "Up", onclick: () => bundles.move(bref, ref, -1).then(again, fail) }),
            h("button", { type: "button", textContent: "↓", ariaLabel: "Down", onclick: () => bundles.move(bref, ref, 1).then(again, fail) }),
            h("button", { type: "button", textContent: "Take out", onclick: () => bundles.drop(bref, ref).then(again, fail) }),
          );
      return h("li", { className: "m" }, it ? h("a", { href: items.pageOf(ref, it.kind), textContent: label }) : h("span", { className: "none", textContent: label }), tools);
    });
    return h(
      "div",
      {},
      h("h2", { textContent: b.title || "untitled" }),
      h("p", {}, h("a", { href: items.pageOf(bref, "bundle"), textContent: "Its page: comments, votes, who sees it" })),
      rows.length ? h("ul", {}, ...rows) : h("p", { className: "none", textContent: "Nothing in it yet: “Add to a bundle” on any item's page." }),
    );
  }

  const draw = async () => {
    for (const v of views) v.stop();
    views = [];
    said.hidden = true;
    at = await where.of({ kind: "bundle", app: "bundle", yours: "Your bundles" });
    at.tabs([]);
    root.replaceChildren(theme.loading("Reading the bundles…"));
    const s = at.sub;
    // A BUNDLE's OWN PAGE (`…/p/<ref>`): the one item page — its comments, votes, who sees it, its EDITIONS and copies
    // (item > edition > copy, as every item's) — framed here, with the way back to its members.
    const pm = /(?:^|\/)p\/(.+)$/.exec(s);
    if (pm) {
      const bref = decodeURIComponent(pm[1]);
      const page = await (await ctx.require("item-page")).show(bref, { app: "bundle", back: at.base, discover: at.who === "discover" }).catch(e => h("p", { textContent: ctx.say(e) }));
      return root.replaceChildren(h("p", {}, h("a", { href: at.href(`b/${encodeURIComponent(bref)}`), textContent: "← Its items" })), page);
    }
    return (s.startsWith("b/") ? one(decodeURIComponent(s.slice(2))) : list()).catch(fail);
  };
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/bundle" && draw(), { signal: ctx.signal });
  await draw();
}
