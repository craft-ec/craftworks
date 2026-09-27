// ACCOUNT, a PRIVATE page: it shows nothing until someone is logged in. It asks `auth`, which shows its dialog if
// nobody is; closing the dialog goes home.
export async function mount(ctx, el) {
  const auth = await ctx.require("auth");
  const s = await auth.session();
  if (!s) {
    location.hash = "#/";
    return;
  }
  const box = document.createElement("div");
  box.innerHTML = `<h2>Account</h2>`;
  const line = (label, value) => {
    const row = document.createElement("p");
    const code = document.createElement("code");
    code.textContent = value;
    row.append(`${label}: `, code);
    box.append(row);
  };
  line("Account (DID)", s.did);
  line("This node's member key", s.member);

  // YOUR NODES: the account's members as the network holds them.
  const dev = document.createElement("section");
  dev.innerHTML = `<h3>Your nodes</h3><p class="line">Reading your account from the network…</p>`;
  box.append(dev);
  auth.nodes().then(
    list => {
      const ul = document.createElement("ul");
      for (const m of list.sort((a, b) => a.since - b.since)) {
        const li = document.createElement("li");
        const code = document.createElement("code");
        code.textContent = `${m.key.slice(0, 16)}…`;
        // A node is shown by its key: a name guessed from the browser was wrong (the browser is not the member).
        li.append("Node ", code, ` · since ${new Date(m.since).toLocaleString()}`);
        if (m.key === s.member) li.append(" (this node)");
        ul.append(li);
      }
      dev.querySelector(".line").replaceWith(list.length ? ul : "No nodes listed yet: this account was made before the member list.");
    },
    e => (dev.querySelector(".line").textContent = `Could not read your nodes: ${e?.message ?? e}`),
  );

  // RECOVERY WORDS: shown once, at registration; no node keeps them.
  const rec = document.createElement("section");
  rec.innerHTML = `<h3>Recovery words</h3>
    <p>Your recovery words were shown once, when you registered. They are your account: with them you log in on any
    node and get your account back. No node keeps them, so a lost or stolen node cannot give your account away.</p>`;
  box.append(rec);

  const out = document.createElement("button");
  out.textContent = "Log out";
  out.onclick = () => auth.logout();
  box.append(out);
  el.append(box);
}
