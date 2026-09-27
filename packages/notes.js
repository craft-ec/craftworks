// NOTES, an app: the account's notes, one table of the account (the `data` service), the same on every node of the
// account. A PRIVATE page: nothing shows until someone is logged in.
export async function mount(ctx, el) {
  const auth = await ctx.require("auth");
  if (!(await auth.session())) {
    location.hash = "#/";
    return;
  }
  el.innerHTML = `<h2>Notes</h2>
    <p class="note">Your notes belong to your account: any of your nodes shows and edits them.</p>
    <form class="add"><input name="text" maxlength="500" placeholder="Write a note" required style="width:70%">
      <button>Add</button></form>
    <p class="said"></p>
    <ul class="rows"><li>Reading your notes from the network…</li></ul>`;
  const said = t => (el.querySelector(".said").textContent = t);
  let notes;
  try {
    notes = await (await ctx.require("data")).table("notes");
  } catch (e) {
    return said(`Could not open your notes: ${e?.message ?? e}`);
  }
  const render = () => {
    const ul = el.querySelector(".rows");
    ul.replaceChildren();
    const rows = [...notes.rows()].sort((a, b) => (a.key < b.key ? 1 : -1));
    if (!rows.length) ul.append(Object.assign(document.createElement("li"), { textContent: "No notes yet." }));
    for (const r of rows) {
      const li = document.createElement("li");
      const del = document.createElement("button");
      del.textContent = "Delete";
      del.onclick = () => notes.remove(r.key).catch(e => said(`Could not delete: ${e?.message ?? e}`));
      li.append(r.value, " ", del);
      ul.append(li);
    }
  };
  notes.onChange(() => el.isConnected && render());
  render();
  const form = el.querySelector("form.add");
  form.onsubmit = async e => {
    e.preventDefault();
    const text = form.text.value.trim();
    if (!text) return;
    said("Saving…");
    // A key that sorts newest first and never collides between nodes: time, then randomness.
    const key = `${Date.now().toString(36).padStart(10, "0")}-${[...crypto.getRandomValues(new Uint8Array(4))].map(b => b.toString(16).padStart(2, "0")).join("")}`;
    try {
      await notes.put(key, text);
      form.text.value = "";
      said("");
    } catch (err) {
      said(`Could not save: ${err?.message ?? err}`);
    }
  };
}
