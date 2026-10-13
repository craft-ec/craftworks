// SPACES, an app (ARCHITECTURE §1, "The desktop"): where spaces are made, joined and kept. `#/spaces`: the shared spaces
// this person is in, Make one, Join with a code. `#/spaces/<id>`: one space — what it ENABLES (its capabilities: the one
// list its owner and admins keep; every app composes from it — an app shows the space when the space enables what the
// app needs, and draws only what is on), the apps that show it now, and its people, roles, invites and bans (its
// Settings). UI only: spaces are `space`'s, joining `conversation`'s, the list `roles`'.
const WORDS = {
  text: "Posts and articles",
  video: "Videos",
  audio: "Music and podcasts",
  image: "Pictures",
  book: "Books and comics",
  caption: "Captions",
  note: "Notes",
  file: "Files",
  chat: "Chat channels",
  form: "Forms",
  wiki: "Wiki pages",
  vote: "Polls and proposals",
  comments: "Comments",
  reactions: "Votes and reactions",
  mail: "Mail to the space",
  contact: "Its people (Contact)",
};
export async function mount(ctx, el) {
  const [space, roles, conversation, theme] = await Promise.all(["space", "roles", "conversation", "theme"].map(n => ctx.require(n)));
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  el.innerHTML = `<style>@layer components {
    .sa { max-width: 760px; margin: 0 auto; padding: 16px; }
    .sa ul { list-style: none; padding: 0; }
    .sa li { padding: 8px 0; border-bottom: 1px solid var(--line, #8883); display: flex; gap: 12px; align-items: center; }
    .sa form { display: flex; gap: 8px; margin: 8px 0; flex-wrap: wrap; }
    .sa .caps label { display: flex; gap: 8px; align-items: center; padding: 6px 0; }
    .sa .said { color: var(--bad, #b3261e); }
  }</style><div class="sa"></div>`;
  const root = el.querySelector(".sa");
  const said = h("p", { className: "said", hidden: true });
  const fail = e => ((said.textContent = ctx.say(e)), (said.hidden = false));

  async function list() {
    const mine = (await space.mine()).filter(s => s.kind === "server");
    const make = h("form", {}, h("input", { name: "name", placeholder: "A new space's name", autocomplete: "off", required: true, ariaLabel: "New space" }), h("button", { textContent: "Make" }));
    make.onsubmit = async e => {
      e.preventDefault();
      try {
        const s = await space.create("server", make.elements.name.value.trim());
        location.hash = `#/spaces/${s.id}`;
      } catch (err) {
        fail(err);
      }
    };
    const join = h("form", {}, h("input", { name: "code", placeholder: "An invite code", autocomplete: "off", required: true, ariaLabel: "Invite code" }), h("button", { textContent: "Join" }));
    join.onsubmit = async e => {
      e.preventDefault();
      try {
        await conversation.join(join.elements.code.value);
        said.hidden = false;
        said.className = "";
        said.textContent = "Requested ✓ A member who may invite lets you in; the space appears here when you are in.";
      } catch (err) {
        fail(err);
      }
    };
    root.replaceChildren(
      h("h2", { textContent: "Spaces" }),
      mine.length ? h("ul", {}, ...mine.map(s => h("li", {}, h("a", { href: `#/spaces/${s.id}`, textContent: space.shown(s) }), h("a", { href: `#/s/${s.id}`, textContent: "Open" })))) : h("p", { textContent: "You are in no shared space yet." }),
      h("h3", { textContent: "Make one" }),
      make,
      h("h3", { textContent: "Join one" }),
      join,
      said,
    );
  }

  async function one(id) {
    const sp = (await space.mine()).find(s => s.id === id && s.kind === "server");
    if (!sp) return root.replaceChildren(h("p", { textContent: "Not one of your spaces." }), h("a", { href: "#/spaces", textContent: "All spaces" }));
    const r = await roles.of(sp);
    await r.settled;
    const me = await space.me();
    const may = r.can(me, "apps");
    const on = new Set(r.enabled());
    const base = new Set(r.base?.() ?? []);
    const caps = r.capabilities().map(cap => {
      const box = h("input", { type: "checkbox", checked: on.has(cap), disabled: !may || base.has(cap), title: base.has(cap) ? "Always on in a space" : "" });
      box.onchange = () =>
        r.enable(cap, box.checked).then(
          () => draw(),
          e => ((box.checked = !box.checked), fail(e)),
        );
      return h("label", {}, box, h("span", { textContent: `${WORDS[cap] ?? cap}${base.has(cap) ? " — always on" : ""}` }));
    });
    const apps = (ctx.apps ?? []).filter(a => r.apps().includes(a.route.slice(1)));
    root.replaceChildren(
      h("p", {}, h("a", { href: "#/spaces", textContent: "← Spaces" })),
      h("h2", { textContent: space.shown(sp) }),
      h("h3", { textContent: "What it enables" }),
      h("p", { textContent: may ? "Every app composes from this list: an app shows the space when what it needs is on." : "Its owner and admins keep this list." }),
      h("div", { className: "caps" }, ...caps),
      h("h3", { textContent: "Apps that show it" }),
      apps.length ? h("ul", {}, ...apps.map(a => h("li", {}, h("a", { href: `#/s/${sp.id}${a.route}`, textContent: `${a.icon ?? ""} ${a.name}` })))) : h("p", { textContent: "None yet: enable what an app needs." }),
      h("h3", { textContent: "People, roles, invites, bans" }),
      h("p", {}, h("a", { href: `#/s/${sp.id}/settings`, textContent: "Its settings" })),
      said,
    );
  }

  const draw = () => {
    said.hidden = true;
    const id = (ctx.sub ?? "").split("/")[0];
    root.replaceChildren(theme.loading?.("Opening…") ?? "");
    return (id ? one(id) : list()).catch(fail);
  };
  addEventListener("craftworks:route", () => el.isConnected && ctx.route === "/spaces" && draw(), { signal: ctx.signal });
  await draw();
}
