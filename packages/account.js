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

  // RECOVERY WORDS: made with the account; they ARE the account (its owner key and its DID come from them). Shown on
  // request, never kept on the page.
  const rec = document.createElement("section");
  rec.innerHTML = `<h3>Recovery words</h3>
    <p>With these words you can log in on any node, and get your account back if you lose this one. Anyone who has
    them has your account: write them down and keep them offline.</p>
    <button type="button" class="show">Show my recovery words</button>
    <ol class="words" hidden style="columns:3;font-family:monospace"></ol>
    <p class="said"></p>`;
  const list = rec.querySelector(".words");
  const show = rec.querySelector(".show");
  show.onclick = async () => {
    if (!list.hidden) {
      list.hidden = true;
      list.replaceChildren();
      show.textContent = "Show my recovery words";
      return;
    }
    const r = await auth.identity.recovery();
    if (!r.recovery) {
      rec.querySelector(".said").textContent =
        r.refused === "NoRecovery" ? "This node does not hold this account's words (it joined without them)." : `Refused: ${r.refused}`;
      return;
    }
    const bytes = new Uint8Array(r.recovery.match(/../g).map(b => parseInt(b, 16)));
    for (const w of auth.words(bytes).split(" ")) {
      const li = document.createElement("li");
      li.textContent = w;
      list.append(li);
    }
    bytes.fill(0);
    list.hidden = false;
    show.textContent = "Hide my recovery words";
  };
  box.append(rec);

  const out = document.createElement("button");
  out.textContent = "Log out";
  out.onclick = () => auth.logout();
  box.append(out);
  el.append(box);
}
