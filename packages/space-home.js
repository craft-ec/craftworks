// A SPACE's old address (`#/s/<id>`): a space's settings and config are the Space app's (`#/spaces/<id>`), its content
// each app's — this page only sends there (owner 10-13: one place, never a second Home).
export async function mount(ctx) {
  location.replace(ctx.space && ctx.space !== "discover" ? `#/spaces/${ctx.space}` : "#/");
}
