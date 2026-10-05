// SETTINGS, a page: the settings of the place open — a SPACE's (its overview, members and roles, roles, invites, the
// moderation log, its apps and their rules: `server-settings`, as a page) or, in the personal space, this ACCOUNT's
// (`account`: its card, nodes, security, storage, apps, recovery, moderation). `…/settings/<tab>` opens at a tab.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  if (!ctx.space) return (await ctx.require("account")).mount(ctx, el);
  // A SPACE's settings: none of the personal settings' header tabs (the account's) — its own tabs are in the page.
  delete ctx.actions[ctx.route];
  dispatchEvent(new CustomEvent("craftworks:actions"));
  // The whole area (.cw-fill, as Chat and Mail): its tabs and the tab open, each scrolling itself.
  el.classList.add("cw-fill");
  const sp = (await (await ctx.require("space")).mine()).find(s => s.id === ctx.space);
  if (!sp) return void (el.textContent = "You are not in this space (left, or not joined yet).");
  const settings = await ctx.require("server-settings");
  el.replaceChildren();
  const [tab, focus] = (ctx.sub || "overview").split("/");
  await settings.render(el, sp, { tab: tab || "overview", focus: focus || null, left: () => (location.hash = "#/") });
  // Another tab in the address (a link to `…/settings/invites`): drawn again there.
  const here = ctx.space;
  const moved = () => {
    if (!el.isConnected || ctx.route !== "/settings" || ctx.space !== here) return removeEventListener("craftworks:route", moved);
    removeEventListener("craftworks:route", moved);
    el.replaceChildren();
    mount(ctx, el);
  };
  addEventListener("craftworks:route", moved);
}
