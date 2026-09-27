// WHO: which identity this node's signer signs for. The signer holds the key and never shows it; it answers with
// the Register it signs for, whose params name the public key. It asks for the `node` service, which loads then.
export async function mount(ctx, el) {
  el.innerHTML = `<h2>Who is logged in</h2><p class="line">Asking the signer…</p>`;
  const line = el.querySelector(".line");
  const { core, ask, url } = await ctx.require("node");
  const [id, frames] = core.frames_who();
  const said = await ask(frames, s => s.kind === "signer" && s.answers.some(a => a.id === id), "asking the signer who it signs for");
  const a = said.answers.find(a => a.id === id);
  if (!a.who) {
    ctx.log("identity", { what: "none: this node's signer holds no key yet" });
    line.textContent = `Nobody yet: the signer on ${url} holds no key.`;
    return;
  }
  const keys = a.who.identity?.keys ?? [];
  ctx.log("identity", { what: `${keys[0]?.slice(0, 16)}… (${a.who.label})` });
  const p = document.createElement("p");
  p.append("Public key: ");
  const code = document.createElement("code");
  code.textContent = keys.join(", ");
  p.append(code, ` · record "${a.who.label}" · node ${url}`);
  line.replaceWith(p);
}
