// CONTACTS, a page: the people this person knows — friend requests, friends, following, hidden, blocked (the one
// `people-list`) — and FIND someone by `name#abc123` or their id, opening what can be done with them (the one `person`
// menu: message, friend, follow, hide, block). UI only.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [conversation, person, list] = await Promise.all(["conversation", "person", "people-list"].map(n => ctx.require(n)));
  el.innerHTML = `
    <style>
      .ct { max-width: 640px; margin: 0 auto; display: grid; gap: var(--cw-space-3); }
      .ct form { display: flex; gap: var(--cw-space-2); padding: 0 var(--cw-space-3); }
      .ct form input { flex: 1; min-width: 0; padding: 8px var(--cw-space-3); border-radius: var(--cw-radius); }
      .ct form button { font: inherit; border: 0; border-radius: var(--cw-radius); padding: 0 var(--cw-space-4); background: var(--cw-accent); color: var(--cw-accent-fg); cursor: pointer; }
      .ct .said { color: var(--cw-danger); font-size: var(--cw-text-sm); margin: 0; padding: 0 var(--cw-space-3); }
    </style>
    <div class="ct">
      <form><input name="who" autocomplete="off" placeholder="Find someone: name#abc123 or did:craftec:…" aria-label="Find someone"><button>Find</button></form>
      <p class="said" hidden></p>
      <div class="list"></div>
    </div>`;
  const said = el.querySelector(".said");
  const form = el.querySelector("form");
  form.onsubmit = async e => {
    e.preventDefault();
    said.hidden = true;
    try {
      const did = await conversation.person(form.elements.who.value);
      person.open(form.elements.who, did);
    } catch (err) {
      said.textContent = err?.message ?? String(err);
      said.hidden = false;
    }
  };
  await list.show(el.querySelector(".list"));
}
