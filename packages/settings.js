// SETTINGS, a page: this ACCOUNT's settings in the personal space; in a space, its settings are the Space app's (sent
// there). The account's:
// (`account`: its card, nodes, security, storage, apps, recovery, moderation). `…/settings/<tab>` opens at a tab.
export async function mount(ctx, el) {
  if (!ctx.space) return (await ctx.require("account")).mount(ctx, el);
  // A SPACE's settings are the Space app's (`#/spaces/<id>/<tab>`: the one place, owner 10-13) — this address sends
  // there.
  const tab = String(ctx.sub || "").split("/")[0];
  location.replace(`#/spaces/${ctx.space}${tab ? `/${tab}` : ""}`);
}
