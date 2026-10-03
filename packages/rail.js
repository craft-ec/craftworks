// RAIL, a component (the layout's side): the SPACES, always there, Discord-style — PERSONAL first (your space: the whole
// canvas, the default), DISCOVER (the public network), then the SHARED spaces you are in, then + (make a space, or join
// one by an invite code). The
// space open is marked; a shared space shows what is unread in it. Choosing a space opens its Home (`#/s/<id>`;
// Personal: `#/`). Nothing shows until someone is logged in. UI only: spaces are `space`'s, joining `conversation`'s,
// counts `activity`'s.
export async function mount(ctx, el) {
  const [auth, space, conversation, directory] = await Promise.all(["auth", "space", "conversation", "directory"].map(n => ctx.require(n)));
  el.innerHTML = `
    <style>
      .cw-rail { width: 64px; box-sizing: border-box; border-right: 1px solid var(--cw-line); background: var(--cw-bg); display: flex; flex-direction: column;
        align-items: center; gap: var(--cw-space-2); padding: var(--cw-space-2) 0; overflow-y: auto; overscroll-behavior: contain; }
      .cw-rail a, .cw-rail button { position: relative; width: 44px; height: 44px; flex: none; border-radius: 50%; display: grid; place-items: center;
        background: var(--cw-surface); border: 1px solid var(--cw-line); color: var(--cw-fg); text-decoration: none; font: inherit; font-weight: 700;
        font-size: .85rem; cursor: pointer; transition: border-radius .15s; }
      .cw-rail a:hover, .cw-rail button:hover { border-radius: 14px; }
      .cw-rail a[aria-current="page"] { border-radius: 14px; background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
      .cw-rail a[aria-current="page"]::before { content: ""; position: absolute; left: -10px; width: 4px; height: 28px; border-radius: 0 4px 4px 0; background: var(--cw-fg); }
      .cw-rail .sep { width: 28px; height: 2px; border-radius: 1px; background: var(--cw-line); flex: none; }
      .cw-rail .add { color: var(--cw-accent); font-size: 1.3rem; font-weight: 400; }
      .cw-rail .cw-badge { position: absolute; right: -6px; bottom: -4px; margin: 0; }
      .rail-ask { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-4); width: min(380px, calc(100vw - 32px)); box-shadow: var(--cw-shadow-lg);
        background: var(--cw-surface); color: var(--cw-fg); }
      .rail-ask h3 { margin: 0 0 var(--cw-space-2); font-size: 1rem; }
      .rail-ask p { margin: 0 0 var(--cw-space-3); color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .rail-ask form { display: flex; gap: var(--cw-space-2); margin-bottom: var(--cw-space-3); }
      .rail-ask input { flex: 1; min-width: 0; padding: 6px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
      .rail-ask button { font: inherit; border: 0; border-radius: var(--cw-radius-sm); padding: 6px var(--cw-space-3); background: var(--cw-accent); color: var(--cw-accent-fg); cursor: pointer; }
      .rail-ask .said { color: var(--cw-danger); }
      .rail-ask .waiting { margin: 0; padding-left: 1.2em; color: var(--cw-muted); font-size: var(--cw-text-sm); }
      @media (max-width: 600px) { .cw-rail { width: 52px; } .cw-rail a, .cw-rail button { width: 38px; height: 38px; } }
    </style>
    <nav class="cw-rail" aria-label="Spaces"></nav>`;
  const nav = el.querySelector(".cw-rail");
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(Boolean));
    return e;
  };
  const initials = name =>
    String(name ?? "?")
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map(w => [...w][0])
      .join("")
      .toUpperCase() || "?";

  // + : a new space, or one joined by an invite code.
  function ask() {
    const d = h("dialog", { className: "rail-ask" });
    const said = h("p", { className: "said", hidden: true });
    const fail = e => ((said.textContent = e?.message ?? String(e)), (said.hidden = false));
    const make = h("form", {}, h("input", { name: "name", placeholder: "Its name", autocomplete: "off", required: true, ariaLabel: "New space" }), h("button", { textContent: "Create" }));
    make.onsubmit = async e => {
      e.preventDefault();
      try {
        // A new space: its Home and settings only; its apps are added there.
        const s = await space.create("server", make.elements.name.value.trim());
        d.close();
        location.hash = `#/s/${s.id}`;
      } catch (err) {
        fail(err);
      }
    };
    const join = h("form", {}, h("input", { name: "code", placeholder: "xxxx-xxxx-xxxx-xxxx", autocomplete: "off", required: true, ariaLabel: "Invite code" }), h("button", { textContent: "Join" }));
    join.onsubmit = async e => {
      e.preventDefault();
      try {
        await conversation.join(join.elements.code.value);
        said.hidden = false;
        said.className = "";
        said.textContent = "Requested ✓ A member who may invite lets you in — it takes a moment, and happens even while their app is closed. The space appears here when you are in.";
        drawWaiting();
      } catch (err) {
        said.className = "said";
        fail(err);
      }
    };
    // Requests by code still waiting: shown, so nobody asks again wondering whether it went through.
    const waiting = h("ul", { className: "waiting" });
    const drawWaiting = () =>
      conversation.askedCodes().then(
        list =>
          waiting.replaceChildren(...list.map(a => h("li", { textContent: `Requested with ${a.code} · ${new Date(a.at).toLocaleString()} — waiting to be let in` }))),
        () => {},
      );
    drawWaiting();
    d.append(h("h3", { textContent: "Make a space" }), h("p", { textContent: "Its members and roles are its own; add its apps (Chat, Board, Notes) on its Home." }), make, h("h3", { textContent: "Join a space" }), join, said, waiting);
    d.addEventListener("click", e => e.target === d && d.close());
    d.addEventListener("close", () => d.remove());
    document.body.append(d);
    d.showModal();
  }

  let activity = null;
  async function draw() {
    const me = await auth.check().catch(() => null);
    el.hidden = !me;
    if (!me) return nav.replaceChildren();
    // (A CIRCLE — an audience, `circles` — is never on the rail.)
    const shared = (await space.mine().catch(() => [])).filter(s => s.kind === "server" && !s.circle);
    const mine = await directory.handle(me.did).catch(() => null);
    const personal = h("a", { href: "#/", title: "Personal", textContent: initials(mine ?? "Me") });
    if (!ctx.space) personal.setAttribute("aria-current", "page");
    // DISCOVER: the public network (nobody's space), under Personal.
    const discover = h("a", { href: "#/discover", title: "Discover: the public network", textContent: "🧭" });
    if (ctx.space === "discover") discover.setAttribute("aria-current", "page");
    nav.replaceChildren(
      personal,
      discover,
      h("div", { className: "sep" }),
      ...shared.map(s => {
        const a = h("a", { href: `#/s/${s.id}`, title: space.shown(s), textContent: initials(s.name) });
        if (ctx.space === s.id) a.setAttribute("aria-current", "page");
        const n = activity?.of(s.id) ?? 0;
        if (n) a.append(h("span", { className: "cw-badge", textContent: String(n) }));
        return a;
      }),
      h("button", { type: "button", className: "add", title: "Make or join a space", textContent: "+", onclick: ask }),
    );
  }
  await draw();
  addEventListener("craftworks:route", () => draw());
  addEventListener("craftworks:auth", () => draw());
  // Spaces joined (a welcome accepted) or left, and what is unread.
  ctx.require("activity").then(a => ((activity = a), a.onChange(() => draw()), draw()), () => {});
  (await ctx.require("storage")).table("spaces").then(t => t.onChange(() => draw()), () => {});
}
