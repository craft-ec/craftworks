// DOCS, an app (`#/docs`): documents written together, live — a word processor (`doc-editor`: a page, paragraph styles,
// bold, italic, lists, links) in the office shell, over `codoc`. The list, New and an open document are every office
// app's (`office-app`); yours, a space's, shared with you, saved — `where`'s tabs. Who may edit: its maker, a space's
// `edit` policy, or its editors when shared (`share`); everyone else reads.
export async function mount(ctx, el) {
  return (await ctx.require("office-app")).open(ctx, el, { kind: "doc", route: "docs", name: "Docs", icon: "📝", noun: "document" });
}
