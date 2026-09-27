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

  // APPS WITH ACCESS: the sites this account allowed to write its tables (the home app's own need no prompt).
  const access = document.createElement("section");
  access.innerHTML = `<h3>Apps with access</h3><p class="note">Your data belongs to your account. These sites may
    change it; any site can be removed, and will ask again next time.</p><ul class="grants"><li>Reading…</li></ul>`;
  box.append(access);
  const drawGrants = async () => {
    const ul = access.querySelector(".grants");
    const r = await auth.identity.grants();
    const list = r.grants ?? [];
    ul.replaceChildren();
    if (!list.length) ul.append(Object.assign(document.createElement("li"), { textContent: "None yet." }));
    const here = location.pathname.split("/")[4];
    // One line per SITE, with the kinds of data it may change; each can be removed on its own.
    const bySite = new Map();
    for (const g of list) bySite.set(g.app, [...(bySite.get(g.app) ?? []), g.table]);
    for (const [app, tables] of bySite) {
      const li = document.createElement("li");
      const code = document.createElement("code");
      code.textContent = `${app.slice(0, 10)}…`;
      li.append(app === here ? "This app " : "Site ", code, app === here ? "" : "", ": ");
      tables.forEach((table, i) => {
        const chip = Object.assign(document.createElement("button"), { type: "button", textContent: `${table} ✕`, title: `Remove its access to your ${table}` });
        chip.onclick = async () => {
          await auth.identity.revoke(app, table);
          drawGrants();
        };
        li.append(i ? " " : "", chip);
      });
      ul.append(li);
    }
  };
  drawGrants().catch(e => (access.querySelector(".grants").textContent = `Could not read: ${e?.message ?? e}`));

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
