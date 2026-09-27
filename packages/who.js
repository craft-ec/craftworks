// WHO: the logged-in person. This page NEEDS a session, so it asks `auth`, which shows its dialog if nobody is logged
// in on this page yet. Home and every other public page never load auth at all.
export async function mount(ctx, el) {
  el.innerHTML = `<h2>Who is logged in</h2><p class="line">Asking…</p>`;
  const auth = await ctx.require("auth");
  const s = await auth.session();
  const p = document.createElement("div");
  const line = (label, value) => {
    const row = document.createElement("p");
    const code = document.createElement("code");
    code.textContent = value;
    row.append(`${label}: `, code);
    p.append(row);
  };
  line("Account (DID)", s.did);
  line("This device's member key", s.member);
  const out = document.createElement("button");
  out.textContent = "Log out";
  out.onclick = async () => {
    await auth.logout();
    el.querySelector(".line")?.remove();
    p.replaceChildren("Logged out.");
  };
  p.append(out);
  el.querySelector(".line").replaceWith(p);
}
