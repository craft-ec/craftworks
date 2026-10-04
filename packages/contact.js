// CONTACTS, a page: the people this person knows — friend requests, friends, following, hidden, blocked (the one
// `people-list`) — and FIND someone by `name#abc123` or their id, opening what can be done with them (the one `person`
// menu: message, friend, follow, hide, block); and "Show me in Discover". In DISCOVER (`#/discover/contacts`, its
// public view): the people who chose to be shown, each opening the same menu. UI only: people are `directory`'s.
export async function mount(ctx, el) {
  const login = await ctx.require("login");
  if (!(await login.session())) {
    location.hash = "#/";
    return;
  }
  const [conversation, person, list, directory] = await Promise.all(["conversation", "person", "people-list", "directory"].map(n => ctx.require(n)));
  // WHERE (`where`): your people, or DISCOVER (anyone listed) — the tabs every app has.
  const at = await (await ctx.require("where")).of({ kind: "contact", app: "contact", yours: "Your people", saves: false });
  at.tabs();
  if (at.discover) return people(ctx, el, { directory, person });
  if (at.space) return spacePeople(ctx, el, at.space);
  el.innerHTML = `
    <style>
      .ct { max-width: 640px; margin: 0 auto; display: grid; gap: var(--cw-space-3); }
      .ct form { display: flex; gap: var(--cw-space-2); padding: 0 var(--cw-space-3); }
      .ct form input { flex: 1; min-width: 0; padding: 8px var(--cw-space-3); border-radius: var(--cw-radius); }
      .ct form button { font: inherit; border: 0; border-radius: var(--cw-radius); padding: 0 var(--cw-space-4); background: var(--cw-accent); color: var(--cw-accent-fg); cursor: pointer; }
      .ct .said { color: var(--cw-danger); font-size: var(--cw-text-sm); margin: 0; padding: 0 var(--cw-space-3); }
      .ct .listed { display: flex; gap: var(--cw-space-2); align-items: center; padding: 0 var(--cw-space-3); font-size: var(--cw-text-sm); color: var(--cw-muted); }
    </style>
    <div class="ct">
      <form><input name="who" autocomplete="off" placeholder="Find someone: name#abc123 or did:craftec:…" aria-label="Find someone"><button>Find</button></form>
      <p class="said" hidden></p>
      <label class="listed"><input type="checkbox"> Show me in Discover (anyone can find your name, id and public posts)</label>
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
  // SHOW ME IN DISCOVER: this person's own choice (their card says it).
  const box = el.querySelector(".listed input");
  directory.isListed().then(on => (box.checked = on), () => {});
  box.onchange = () =>
    directory.listMe(box.checked).catch(e => {
      box.checked = !box.checked;
      said.textContent = e?.message ?? String(e);
      said.hidden = false;
    });
  await list.show(el.querySelector(".list"));
}

// A SPACE's Contact: its people and their roles — the one list (`members-list`), with its tools for who may — and,
// for who may invite, Invite.
async function spacePeople(ctx, el, sp) {
  const [ml, roles, settings, space] = await Promise.all(["members-list", "roles", "server-settings", "space"].map(n => ctx.require(n)));
  const [r, me] = await Promise.all([roles.of(sp), space.account()]);
  el.innerHTML = `
    <style>
      .ct { max-width: 760px; margin: 0 auto; display: grid; gap: var(--cw-space-3); }
      .ct .top { display: flex; gap: var(--cw-space-3); align-items: center; flex-wrap: wrap; }
      .ct .top h2 { margin: 0; font-size: 1.3rem; }
      .ct .top button { font: inherit; border: 0; border-radius: var(--cw-radius); padding: 6px var(--cw-space-4); background: var(--cw-accent); color: var(--cw-accent-fg); cursor: pointer; }
      .ct .s { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    </style>
    <div class="ct"><div class="top"><h2></h2><span class="s"></span></div></div>`;
  const root = el.querySelector(".ct");
  root.querySelector("h2").textContent = `${space.shown(sp)} · people`;
  const count = () => (root.querySelector(".top .s").textContent = `${r.members().length} member${r.members().length === 1 ? "" : "s"} · you: ${r.role(me.id) ?? "member"}`);
  count();
  r.onChange(() => root.isConnected && count());
  if (r.can(me.id, "invite"))
    root.querySelector(".top").append(Object.assign(document.createElement("button"), { type: "button", textContent: "Invite", onclick: () => settings.open(sp, { tab: "invites" }) }));
  root.append(await ml.show(sp, { manage: true }));
}

// DISCOVER → Contacts: the people who chose to be shown (and not flagged by a moderation list you apply).
async function people(ctx, el, { directory, person }) {
  const [moderation, posts, theme] = await Promise.all(["moderation", "items", "theme"].map(n => ctx.require(n)));
  el.innerHTML = `
    <style>
      .ppl { max-width: 880px; margin: 0 auto; display: grid; gap: var(--cw-space-3); }
      .ppl h2 { margin: 0; font-size: 1.4rem; }
      .ppl .note { margin: 0; color: var(--cw-muted); font-size: var(--cw-text-sm); }
      .ppl .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: var(--cw-space-3); }
      .ppl .card { display: grid; gap: 4px; padding: var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); cursor: pointer; }
      .ppl .card:hover { border-color: var(--cw-muted); }
      .ppl .card b { overflow-wrap: anywhere; }
      .ppl .card span { color: var(--cw-muted); font-size: var(--cw-text-sm); }
    </style>
    <div class="ppl"><h2>👤 People</h2><p class="note">People who chose to be shown in Discover. Show yourself from your Contacts.</p><div class="grid"></div></div>`;
  const grid = el.querySelector(".grid");
  grid.replaceChildren(theme.loading("Finding people…"));
  const lists = await moderation.lists().catch(() => null);
  const dids = (await directory.listed()).filter(d => !lists?.flagged({ by: d }));
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(Boolean));
    return e;
  };
  grid.replaceChildren(
    ...(dids.length
      ? dids.map(d => {
          const name = directory.nameEl(d, "b");
          const about = h("span", { textContent: "public profile" });
          posts.list({ by: d }).then(ps => (about.textContent = `${ps.length} public post${ps.length === 1 ? "" : "s"}`), () => {});
          return h("div", { className: "card", onclick: e => person.open(e.currentTarget, d) }, name, about);
        })
      : [h("p", { className: "note", textContent: "Nobody listed yet: show yourself from your Contacts (Show me in Discover)." })]),
  );
}
