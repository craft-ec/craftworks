// SLIDES, an app (`#/slides`): presentations written together, live — a filmstrip and a slide (`slide-deck`: layouts,
// titles and bullets, present full screen) in the office shell, over `codoc`'s cells. The list, New and an open
// presentation are every office app's (`office-app`). Who may edit: its maker, a space's `edit` policy, or its editors.
export async function mount(ctx, el) {
  return (await ctx.require("office-app")).open(ctx, el, { kind: "deck", route: "slides", name: "Slides", icon: "📽", noun: "presentation" });
}
