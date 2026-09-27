// HOME: public and private in one page.
// - PUBLIC (nobody logged in): a welcome, and Log in / Register, which open auth's dialog.
// - PRIVATE (logged in): the main page and what Craftworks offers.
// It asks auth quietly (`check`, never a dialog) which view to show, and switches when someone logs in or out.
export async function mount(ctx, el) {
  const auth = await ctx.require("auth");

  const publicView = () => {
    el.innerHTML = `<h2>Welcome to Craftworks</h2>
      <p>Apps that live on freenet: no server, no company holding your data. Your account is yours, named by twelve
      recovery words, and your data is written by your own nodes.</p>
      <p><button type="button" data-tab="login">Log in</button> <button type="button" data-tab="register">Register</button></p>`;
    for (const b of el.querySelectorAll("[data-tab]")) b.onclick = () => auth.session({ tab: b.dataset.tab });
  };

  const privateView = s => {
    el.innerHTML = `<h2>Welcome back</h2>
      <p class="who">Logged in as <code></code></p>
      <h3>What you can do</h3>
      <ul>
        <li><a href="#/account">Your account</a>: your DID, this node, and your recovery words.</li>
        <li>Your data: a table only your nodes write, readable wherever you log in <em>(next)</em>.</li>
        <li>Your other nodes: add one with its node id and a PIN <em>(soon)</em>.</li>
      </ul>`;
    el.querySelector(".who code").textContent = s.did;
  };

  const show = s => (s ? privateView(s) : publicView());
  const onAuth = e => (el.isConnected ? show(e.detail) : removeEventListener("craftworks:auth", onAuth));
  addEventListener("craftworks:auth", onAuth);
  show(await auth.check().catch(() => null));
}
