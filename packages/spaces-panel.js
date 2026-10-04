// SPACES PANEL, a component: every space you can go to, and the one you are in — opened from the header's name of the
// space open, a whole side panel of three columns (each a kind of space: yours and the shared ones you are in, your
// friends' personal spaces, the spaces you follow). (Discover is no place of its own: each app's tab in yours.) SPACES: Personal (yours: the default), the SHARED spaces you are in (what is
// unread in each), and Make or join one (a name, or an invite code). FRIENDS: each opens their space. FOLLOWING: the
// people you follow (their space) and the public spaces you follow (read from outside). A choice, a click outside or
// Escape closes it. UI only: spaces are `space`'s, joining `conversation`'s, people `edge`'s, counts `activity`'s.
//
//   const panel = await ctx.require("spaces-panel");
//   panel.open()            // the panel, over the page (open again: closed)
//   await panel.here()      // the name of the space you are in ("Personal", "Discover", a shared space's, a person's)
//   panel.personOf()        // the PERSON whose space is open (`#/u/<did>`, `#/<app>/u/<did>`), else null
//   await panel.hrefTo({ personal } | { space } | { person })  // that space, at the SAME APP when it shows
//                           // it (the manifest's `views`; a shared space: the apps it uses), else its Home
export async function start(ctx) {
  const [space, conversation, directory, edge, roles, where] = await Promise.all(["space", "conversation", "directory", "edge", "roles", "where"].map(n => ctx.require(n)));
  // A PERSON's space (whose it is: `where`'s, the one reading of an address).
  const personOf = where.personOf;
  // SWITCHING keeps the app: the app open, in the space chosen, where that space shows it; else that space's Home.
  async function hrefTo({ personal = false, space: sp = null, person = null }) {
    const app = ctx.apps.find(a => a.route === ctx.route);
    const views = app?.views ?? [];
    const me = (await space.account().catch(() => null))?.id;
    if (person && person !== me) return views.includes("person") ? `#${app.route}/u/${person}` : `#/u/${person}`;
    if (sp) return views.includes("shared") && (await roles.of(sp).then(r => r.apps(), () => [])).includes(app.route.slice(1)) ? `#/s/${sp.id}${app.route}` : `#/s/${sp.id}`;
    return views.includes("personal") ? `#${app.route}` : "#/";
  }
  const style = document.createElement("style");
  style.textContent = `
    .cw-panel { position: fixed; top: var(--cw-bar); bottom: 0; left: 0; z-index: 70; width: min(1180px, 100vw); box-sizing: border-box;
      background: var(--cw-bg); border-right: 1px solid var(--cw-line); box-shadow: var(--cw-shadow-lg); display: grid;
      grid-template-columns: repeat(4, minmax(0, 1fr)); overflow: hidden; }
    @media (max-width: 760px) { .cw-panel { grid-template-columns: 1fr 1fr; grid-auto-rows: minmax(0, 1fr); } }
    .cw-panel[hidden] { display: none; }
    .cw-panel section { display: flex; flex-direction: column; min-height: 0; border-right: 1px solid var(--cw-line); }
    .cw-panel section:last-child { border-right: 0; }
    .cw-panel h3 { margin: 0; padding: var(--cw-space-3) var(--cw-space-4); font-size: var(--cw-text-xs); letter-spacing: .08em; color: var(--cw-muted);
      border-bottom: 1px solid var(--cw-line); }
    .cw-panel ul { list-style: none; margin: 0; padding: var(--cw-space-2); overflow-y: auto; display: grid; align-content: start; gap: 2px; }
    .cw-panel li a, .cw-panel li button { display: flex; align-items: center; gap: var(--cw-space-2); width: 100%; box-sizing: border-box; text-align: left;
      padding: 8px var(--cw-space-3); border: 0; border-radius: var(--cw-radius-sm); background: none; color: inherit; font: inherit; text-decoration: none; cursor: pointer; }
    .cw-panel li a:hover, .cw-panel li button:hover { background: var(--cw-hover); }
    .cw-panel li a[aria-current="page"] { background: var(--cw-pressed); font-weight: 600; }
    .cw-panel .ic { width: 28px; height: 28px; flex: none; border-radius: 8px; display: grid; place-items: center; background: var(--cw-surface);
      border: 1px solid var(--cw-line); font-weight: 700; font-size: .75rem; }
    .cw-panel .n { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .cw-panel .add { color: var(--cw-accent); }
    .cw-panel li.requested { display: grid; gap: 2px; }
    .cw-panel li.requested .sub { font-size: var(--cw-text-xs); letter-spacing: .08em; color: var(--cw-muted); padding: var(--cw-space-2) var(--cw-space-2) 0; }
    .cw-panel li.requested .n { display: grid; min-width: 0; }
    .cw-panel li.requested .st { color: var(--cw-muted); font-size: var(--cw-text-xs); }
    .cw-panel .none { color: var(--cw-muted); font-size: var(--cw-text-sm); padding: var(--cw-space-2) var(--cw-space-3); margin: 0; }
    .cw-panel .sep { height: 1px; background: var(--cw-line); margin: var(--cw-space-1) var(--cw-space-2); }
    @media (max-width: 700px) { .cw-panel { grid-template-columns: minmax(0, 1fr); overflow-y: auto; } .cw-panel section { border-right: 0; border-bottom: 1px solid var(--cw-line); } .cw-panel ul { overflow: visible; } }
    .cw-ask { border: 0; border-radius: var(--cw-radius); padding: var(--cw-space-4); width: min(380px, calc(100vw - 32px)); box-shadow: var(--cw-shadow-lg);
      background: var(--cw-surface); color: var(--cw-fg); }
    .cw-ask h3 { margin: 0 0 var(--cw-space-2); font-size: 1rem; }
    .cw-ask p { margin: 0 0 var(--cw-space-3); color: var(--cw-muted); font-size: var(--cw-text-sm); }
    .cw-ask form { display: flex; gap: var(--cw-space-2); margin-bottom: var(--cw-space-3); }
    .cw-ask input { flex: 1; min-width: 0; padding: 6px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
    .cw-ask button { font: inherit; border: 0; border-radius: var(--cw-radius-sm); padding: 6px var(--cw-space-3); background: var(--cw-accent); color: var(--cw-accent-fg); cursor: pointer; }
    .cw-ask .said { color: var(--cw-danger); }
    .cw-ask .waiting { margin: 0; padding-left: 1.2em; color: var(--cw-muted); font-size: var(--cw-text-sm); }`;
  document.head.append(style);
  const h = (tag, props = {}, ...kids) => {
    const e = Object.assign(document.createElement(tag), props);
    e.append(...kids.filter(k => k != null && k !== false));
    return e;
  };
  const initials = name =>
    String(name ?? "?")
      .replace(/^#/, "")
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map(w => [...w][0])
      .join("")
      .toUpperCase() || "?";

  // MAKE OR JOIN: a new space by its name, or one joined by an invite code.
  function ask() {
    const d = h("dialog", { className: "cw-ask" });
    const said = h("p", { className: "said", hidden: true });
    const fail = e => ((said.className = "said"), (said.textContent = e?.message ?? String(e)), (said.hidden = false));
    const make = h("form", {}, h("input", { name: "name", placeholder: "Its name", autocomplete: "off", required: true, ariaLabel: "New space" }), h("button", { textContent: "Create" }));
    make.onsubmit = async e => {
      e.preventDefault();
      try {
        // A new space: its Home and settings only; its apps are added there.
        const s = await space.create("server", make.elements.name.value.trim());
        d.close();
        location.hash = `#/s/${s.id}`;
      } catch (err) {
        fail(err);
      }
    };
    const join = h("form", {}, h("input", { name: "code", placeholder: "xxxx-xxxx-xxxx-xxxx", autocomplete: "off", required: true, ariaLabel: "Invite code" }), h("button", { textContent: "Join" }));
    join.onsubmit = async e => {
      e.preventDefault();
      try {
        await conversation.join(join.elements.code.value);
        said.hidden = false;
        said.className = "";
        said.textContent = "Requested ✓ A member who may invite lets you in — it takes a moment, and happens even while their app is closed. The space appears here when you are in.";
        drawWaiting();
      } catch (err) {
        fail(err);
      }
    };
    // Requests by code still waiting: shown, so nobody asks again wondering whether it went through.
    const waiting = h("ul", { className: "waiting" });
    const drawWaiting = () =>
      conversation.askedCodes().then(
        list => waiting.replaceChildren(...list.map(a => h("li", { textContent: `Requested with ${a.code} · ${new Date(a.at).toLocaleString()} — waiting to be let in` }))),
        () => {},
      );
    drawWaiting();
    d.append(h("h3", { textContent: "Make a space" }), h("p", { textContent: "Its members and roles are its own; add its apps (Chat, Board, Notes) on its Home." }), make, h("h3", { textContent: "Join a space" }), join, said, waiting);
    d.addEventListener("click", e => e.target === d && d.close());
    d.addEventListener("close", () => d.remove());
    document.body.append(d);
    d.showModal();
  }

  const panel = h("div", { className: "cw-panel", hidden: true });
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-label", "Spaces: yours, friends\u2019, following");
  document.body.append(panel);
  let activity = null;
  ctx.require("activity").then(a => ((activity = a), a.onChange(() => !panel.hidden && draw())), () => {});

  const row = (href, icon, name, { on = false, badge = 0, title = "" } = {}) => {
    const a = h("a", { href, title: title || name }, h("span", { className: "ic", textContent: icon }), h("span", { className: "n", textContent: name }), badge ? h("span", { className: "cw-badge", textContent: String(badge) }) : null);
    if (on) a.setAttribute("aria-current", "page");
    return h("li", {}, a);
  };
  const column = (title, items, none) => h("section", {}, h("h3", { textContent: title }), h("ul", {}, ...(items.length ? items : [h("p", { className: "none", textContent: none })])));

  async function draw() {
    const me = (await space.account().catch(() => null))?.id;
    if (!me) return panel.replaceChildren();
    const [mine, people, myName] = await Promise.all([space.mine().catch(() => []), edge.people(), directory.handle(me).catch(() => null)]);
    // (A CIRCLE — an audience, `circles` — is no place of its own: never listed.)
    const shared = mine.filter(s => s.kind === "server" && !space.isGroup(s));
    const friends = people.list("friend").filter(d => d.startsWith("did:"));
    const follows = people.list("follow");
    const names = await Promise.all([...friends, ...follows.filter(d => d.startsWith("did:"))].map(d => directory.handle(d).then(n => [d, n], () => [d, null])));
    const nameOf = new Map(names);
    const open = personOf();
    // Each link at its space's Home first, then at the same app where that space shows it (`hrefTo`: a shared space's
    // apps take a read).
    const at = (a, target) => (hrefTo(target).then(href => (a.querySelector("a").href = href), () => {}), a);
    const spaces = [
      at(row("#/", initials(directory.shown(me, myName)), `${directory.shown(me, myName)} · Personal`, { on: (!ctx.space || ctx.space === "discover") && !open }), { personal: true }),
      shared.length ? h("li", { className: "sep" }) : null,
      ...shared.map(s => at(row(`#/s/${s.id}`, initials(s.name), space.shown(s), { on: ctx.space === s.id, badge: activity?.of(s.id) ?? 0 }), { space: s })),
      h("li", { className: "sep" }),
      h("li", {}, h("button", { type: "button", className: "add", onclick: () => (close(), ask()) }, h("span", { className: "ic", textContent: "+" }), h("span", { className: "n", textContent: "Make or join a space" }))),
    ].filter(Boolean);
    // REQUESTED: spaces asked to join, each with where it stands — read after the rest (the panel never waits on it).
    const requested = h("li", { className: "requested", hidden: true });
    spaces.splice(spaces.length - 2, 0, requested);
    const people_ = list => list.map(d => at(row(`#/u/${d}`, initials(nameOf.get(d) ?? d.slice(12)), directory.shown(d, nameOf.get(d)), { title: d, on: open === d }), { person: d }));
    const followed = follows.map(d =>
      d.startsWith("did:") ? people_([d])[0] : row(`#/s/${d}`, "🌐", people.about("follow", d)?.name || `a space #${d.slice(0, 6)}`, { title: d }),
    );
    // DISCOVER: every public space this person is not in (Discover's list) — each its CARD (`person.openSpace`:
    // about, members, apps, Join · Requested · Open). Read after the rest: the panel never waits on it.
    const discover = h("ul", {}, h("p", { className: "none", textContent: "Looking…" }));
    panel.replaceChildren(
      column("YOUR SPACES", spaces, ""),
      column("FRIENDS", people_(friends), "No friends yet — add one from anyone's name."),
      column("FOLLOWING", followed, "You follow nobody yet."),
      h("section", {}, h("h3", { textContent: "DISCOVER SPACES" }), discover),
    );
    conversation.waiting().then(async ws => {
      if (!ws.length) return;
      const person = await ctx.require("person");
      const STATUS = { asked: "asked · waiting for a member who may invite", "let-in": "let in · opening the welcome", code: "asked by a code · waiting" };
      requested.hidden = false;
      requested.replaceChildren(
        h("div", { className: "sub", textContent: "REQUESTED" }),
        ...ws.map(w =>
          h(
            "button",
            { type: "button", className: "row", title: w.id ?? w.code, onclick: e => w.desc && person.openSpace(e.currentTarget, w.desc) },
            h("span", { className: "ic", textContent: w.name ? initials(w.name) : "…" }),
            h("span", { className: "n" }, h("span", { textContent: w.name ?? (w.code ? `code ${w.code}` : `a space #${String(w.id).slice(0, 6)}`) }), h("small", { className: "st", textContent: STATUS[w.status] })),
          ),
        ),
      );
    }, () => {});
    const inMine = new Set(mine.map(s => s.id));
    const [listed, person] = await Promise.all([ctx.require("items").then(i => i.publicSpaces()).catch(() => []), ctx.require("person")]);
    const others = listed.filter(d => !inMine.has(d.id));
    discover.replaceChildren(
      ...(others.length
        ? others.map(d => h("li", {}, h("button", { type: "button", className: "row", title: d.id, onclick: e => person.openSpace(e.currentTarget, d) }, h("span", { className: "ic", textContent: initials(d.name) }), h("span", { className: "n", textContent: space.shown(d) }))))
        : [h("p", { className: "none", textContent: "No public space listed yet." })]),
    );
  }

  const outside = e => !panel.contains(e.target) && !e.target.closest?.("[data-spaces]") && close();
  const esc = e => e.key === "Escape" && close();
  function close() {
    panel.hidden = true;
    removeEventListener("click", outside, true);
    removeEventListener("keydown", esc);
  }
  function open() {
    if (!panel.hidden) return close();
    panel.hidden = false;
    addEventListener("click", outside, true);
    addEventListener("keydown", esc);
    draw();
  }
  panel.addEventListener("click", e => e.target.closest("a") && close());
  addEventListener("hashchange", close);

  // WHERE you are now, named (the header's button).
  async function here() {
    // A PERSON's space (from Friends or Following): theirs, named.
    const who = personOf();
    if (who && who !== (await space.account().catch(() => null))?.id) return directory.shown(who, await directory.handle(who).catch(() => null));
    if (ctx.space && ctx.space !== "discover") {
      const sp = (await space.mine().catch(() => [])).find(s => s.id === ctx.space);
      if (sp) return space.shown(sp);
      // Not in it (seen from outside): its name as Discover lists it.
      const d = (await (await ctx.require("items")).publicSpaces().catch(() => [])).find(s => s.id === ctx.space);
      return d ? space.shown(d) : "Space";
    }
    return "Personal";
  }
  return { open, close, here, personOf, hrefTo };
}
