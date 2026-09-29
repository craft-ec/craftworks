// SPACE APPS, a component: the APPS a space has — Messages (a server's channels in Chat, a conversation in Messages),
// and those its owner or admins ADDED: Board (posts) and Notes (shared). One space, the same members, roles and
// moderation, in every app of it (as apps are added to a team in Teams). Every page on a space puts ONE dropdown first in
// its top bar: the space and the app open (`Kiln · Board ▾`), listing its apps, and — for who may — "Add or remove
// apps…". The PERSONAL space (the account) has all three, fixed: Messages (your conversations), Board (your profile:
// public, what your followers read), Notes (yours). UI only: which apps a space has is `roles`' (an `app` act).
//
//   const spaceApps = await ctx.require("space-apps");
//   await spaceApps.menu(sp, "messages" | "board" | "notes")   // top-bar actions: [one menu]
//   await spaceApps.has(sp, "board")                            // the space has that app
export async function start(ctx) {
  const roles = await ctx.require("roles");
  const space = await ctx.require("space");
  const NAMES = { messages: "Messages", board: "Board", notes: "Notes" };
  const ABOUT = { board: "Posts, comments and votes, Reddit-style.", notes: "Notes the members keep together." };
  const hrefOf = (sp, app) =>
    sp.kind === "account"
      ? { messages: "#/messages", board: `#/board/u/${sp.id}`, notes: "#/notes" }[app]
      : { messages: sp.kind === "server" ? `#/chat/${sp.id}` : `#/messages/${sp.id}`, board: `#/board/b/${sp.id}`, notes: `#/notes/s/${sp.id}` }[app];
  const appsOf = async sp => (sp.kind === "account" ? ["messages", "board", "notes"] : (await roles.of(sp)).apps());
  const has = async (sp, app) => (await appsOf(sp)).includes(app);

  const style = document.createElement("style");
  style.textContent = `
    .cw-apps { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-4); width: min(380px, calc(100vw - 32px)); box-shadow: var(--cw-shadow-lg);
      background: var(--cw-surface); color: var(--cw-fg); }
    .cw-apps h3 { margin: 0 0 var(--cw-space-3); font-size: 1rem; }
    .cw-apps .app { display: flex; align-items: center; gap: var(--cw-space-3); padding: var(--cw-space-2) 0; border-top: 1px solid var(--cw-line); }
    .cw-apps .app div { flex: 1; min-width: 0; }
    .cw-apps .app b { display: block; }
    .cw-apps .app span { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-apps button { font: inherit; cursor: pointer; border-radius: var(--cw-radius-sm); padding: 4px var(--cw-space-3); border: 1px solid var(--cw-line); background: none; color: var(--cw-fg); }
    .cw-apps button.add { background: var(--cw-accent); color: var(--cw-accent-fg); border-color: transparent; }
    .cw-apps .said { color: var(--cw-danger); font-size: var(--cw-text-sm); margin: var(--cw-space-2) 0 0; }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(Boolean));
    return e;
  };

  // ADD OR REMOVE APPS: a dialog (closed by Escape or a click outside it).
  async function manage(sp) {
    const r = await roles.of(sp);
    const d = h("dialog", { className: "cw-apps" });
    const said = h("p", { className: "said", hidden: true });
    const draw = () => {
      const on = r.apps();
      d.replaceChildren(
        h("h3", { textContent: `Apps of ${sp.name}` }),
        h("div", { className: "app" }, h("div", {}, h("b", { textContent: "Messages" }), h("span", { textContent: "Every space's." }))),
        ...["board", "notes"].map(app => {
          const added = on.includes(app);
          const b = h("button", { type: "button", className: added ? "" : "add", textContent: added ? "Remove" : "Add" });
          b.onclick = async () => {
            b.disabled = true;
            said.hidden = true;
            try {
              await r.act({ act: "app", app, on: !added });
            } catch (e) {
              said.textContent = e?.message ?? String(e);
              said.hidden = false;
            }
            b.disabled = false;
          };
          return h("div", { className: "app" }, h("div", {}, h("b", { textContent: NAMES[app] }), h("span", { textContent: ABOUT[app] })), b);
        }),
        said,
      );
    };
    r.onChange(() => {
      if (!d.isConnected) return;
      draw();
      dispatchEvent(new CustomEvent("craftworks:space-apps", { detail: sp.id }));
    });
    draw();
    d.addEventListener("click", e => e.target === d && d.close());
    d.addEventListener("close", () => d.remove());
    document.body.append(d);
    d.showModal();
  }

  async function menu(sp, here) {
    const apps = await appsOf(sp);
    const items = apps.map(app => ({ label: NAMES[app], href: hrefOf(sp, app), on: app === here }));
    if (sp.kind !== "account") {
      const me = (await space.account())?.id;
      if ((await roles.of(sp)).can(me, "apps")) items.push({ label: "Add or remove apps…", run: () => manage(sp) });
    }
    return [{ label: `${sp.kind === "account" ? "Your space" : sp.name} · ${NAMES[here]}`, menu: items }];
  }

  return { menu, has };
}
