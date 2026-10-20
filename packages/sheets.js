// SHEETS, an app (`#/sheets`): spreadsheets written together, live — a grid with a formula bar (`sheet-grid`: cells,
// formulas, formats) in the office shell, over `codoc`'s cells. The list, New and an open spreadsheet are every office
// app's (`office-app`). Who may edit: its maker, a space's `edit` policy, or its editors when shared (`share`).
export async function mount(ctx, el) {
  return (await ctx.require("office-app")).open(ctx, el, { kind: "sheet", route: "sheets", name: "Sheets", icon: "📊", noun: "spreadsheet" });
}
