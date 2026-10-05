// HEADER: a top bar like a Mac's menu bar. On the left: ⌂ (home), the SPACE you are in — its name opens the spaces panel,
// every space you can go to (yours, your friends', those you follow) —, the CURRENT APP — a dropdown switching between the apps of that space
// (its Home, and the apps it uses; Personal: the apps with a personal view) — and
// that app's MENU — its sub-pages and actions; on the right: the app's ending actions (Account is the Settings app's). It changes with the app: on Home it reads
// "Home"; in an app, the app's name (from the manifest's apps) and what the app put under its route in `ctx.actions`:
// `{ label, href }` (a sub-page: a link; `on` when it is the one shown), `{ label, run }` (an action; `end`: at the
// right), a search box, or a MENU (`{ label, menu: [{ label, href, on } | { label, run }] }`: a button opening a dropdown, closed
// by a choice, a click outside or Escape). An app that changes them later says so with a `craftworks:actions` event. The app's own
// component (the layout names it), so editing it is publishing the app, never the loader.
export function mount(ctx, el) {
  el.innerHTML = `
    <style>
      .bar { display: flex; align-items: center; gap: 14px; border-bottom: 1px solid var(--cw-line); height: var(--cw-bar); box-sizing: border-box;
        font-size: var(--cw-text-sm); }
      .bar .home { text-decoration: none; font-size: 1.1rem; color: inherit; }
      .bar .space-name { border: 0; background: none; font: inherit; font-weight: 700; color: inherit; cursor: pointer; padding: 2px var(--cw-space-2);
        border-radius: var(--cw-radius-sm); max-width: 16em; min-width: 6em; flex: 0 1 auto; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      /* The app's own actions give way first (they scroll), never the space's name. */
      .bar .actions { min-width: 0; overflow-x: auto; scrollbar-width: none; }
      .bar .space-name:hover { background: var(--cw-hover); }
      .bar .space-name .cw-badge { margin-left: 4px; }
      .bar .name { font-weight: 600; }
      .bar .name .drop > button { border: 0; background: none; font-weight: 700; padding: 2px var(--cw-space-2); }
      .bar .name .drop > button:hover { background: var(--cw-hover); }
      .bar .actions { display: flex; gap: 10px; align-items: center; }
      .bar .actions button { border: 0; background: none; padding: 2px var(--cw-space-1); cursor: pointer; border-radius: var(--cw-radius-sm); }
      .bar .actions button:hover { background: var(--cw-hover); }
      .bar .actions button[aria-pressed="true"], .bar .actions a[aria-current="page"] { background: var(--cw-pressed); }
      .bar .actions a { color: inherit; text-decoration: none; padding: 2px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
      .bar .actions a:hover { background: var(--cw-hover); }
      .bar .end { display: flex; gap: 10px; margin-left: auto; }
      .bar .end button { border: 0; background: none; padding: 2px var(--cw-space-1); cursor: pointer; border-radius: var(--cw-radius-sm); }
      .bar .end button:hover, .bar .end a:hover { background: var(--cw-hover); }
      .bar .end { align-items: center; }
      .bar .end a { color: inherit; text-decoration: none; padding: 2px var(--cw-space-2); border-radius: var(--cw-radius-sm); }
      .bar .end a[aria-current="page"] { background: var(--cw-pressed); }
      .bar .actions .search { padding: var(--cw-space-1) 10px; border-radius: var(--cw-radius-pill); width: 16em; }
      .bar .drop { position: relative; }
      .bar .drop > button { border: 1px solid var(--cw-line); border-radius: var(--cw-radius-sm); padding: 2px var(--cw-space-2); font-weight: 600;
        max-width: 22em; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .bar .drop .list { position: absolute; top: calc(100% + 4px); left: 0; z-index: 60; min-width: 12em; display: grid; padding: var(--cw-space-1);
        background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); box-shadow: var(--cw-shadow-lg); }
      .bar .drop .list[hidden] { display: none; }
      .bar .drop .list a, .bar .drop .list button { padding: 6px var(--cw-space-3); text-align: left; font: inherit; color: inherit; }
      .bar .drop .list button { border-top: 1px solid var(--cw-line); border-radius: 0; margin-top: var(--cw-space-1); }
      .bar .drop .list a[aria-current="page"] { font-weight: 600; }
      .bar .find { position: relative; }
      .bar .find > input { font: inherit; padding: 2px 10px; border-radius: var(--cw-radius-pill); border: 1px solid var(--cw-line); width: 14em; background: var(--cw-surface); color: inherit; }
      .bar .find .results { position: absolute; top: calc(100% + 4px); right: 0; z-index: 60; display: grid; padding: var(--cw-space-1); width: min(28em, calc(100vw - 32px));
        max-height: 70vh; overflow-y: auto; background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); box-shadow: var(--cw-shadow-lg); }
      .bar .find .results[hidden] { display: none; }
      .bar .find .results .head { color: var(--cw-muted); font-size: var(--cw-text-xs); letter-spacing: .08em; text-transform: uppercase; padding: 6px var(--cw-space-2) 2px; }
      .bar .find .results a, .bar .find .results button { display: block; text-align: left; font: inherit; color: inherit; text-decoration: none; border: 0; background: none;
        padding: 5px var(--cw-space-2); border-radius: var(--cw-radius-sm); cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .bar .find .results a:hover, .bar .find .results button:hover, .bar .find .results .sel { background: var(--cw-hover); }
      .bar .find .results .none { color: var(--cw-muted); padding: 6px var(--cw-space-2); margin: 0; }
      @media (max-width: 640px) { .bar .find > input { width: 7em; } }
      .bar .inbox { position: relative; }
      .bar .inbox > button { border: 0; background: none; cursor: pointer; padding: 2px var(--cw-space-1); border-radius: var(--cw-radius-sm); font: inherit; }
      .bar .inbox > button:hover { background: var(--cw-hover); }
      .bar .inbox .results { position: absolute; top: calc(100% + 4px); right: 0; z-index: 60; display: grid; padding: var(--cw-space-1); width: min(26em, calc(100vw - 32px));
        max-height: 70vh; overflow-y: auto; background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); box-shadow: var(--cw-shadow-lg); }
      .bar .inbox .results[hidden] { display: none; }
      .bar .inbox .results button { display: grid; gap: 2px; text-align: left; font: inherit; color: inherit; border: 0; background: none; padding: 6px var(--cw-space-2);
        border-radius: var(--cw-radius-sm); cursor: pointer; }
      .bar .inbox .results button:hover { background: var(--cw-hover); }
      .bar .inbox .results .where { color: var(--cw-muted); font-size: var(--cw-text-xs); }
      .bar .inbox .results .what { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .bar .inbox .results .none { color: var(--cw-muted); padding: 6px var(--cw-space-2); margin: 0; }

    </style>
    <nav class="bar">
      <a class="home" href="#/" title="Home">⌂</a>
      <button type="button" class="space-name" data-spaces title="Spaces: yours, your friends', following" hidden></button>
      <span class="name"></span>
      <span class="actions"></span>
      <span class="end"></span>
      <span class="inbox" data-of="recent"><button type="button" title="Notifications">🔔</button><div class="results" hidden></div></span>
      <span class="inbox" data-of="mentions"><button type="button" title="Mentions">@</button><div class="results" hidden></div></span>
      <span class="find"><input type="search" placeholder="🔍 Search or paste a link" aria-label="Search everything" autocomplete="off"><div class="results" hidden></div></span>
    </nav>`;
  // NOTIFICATIONS (🔔: each place with something unread, newest first — opened there) and MENTIONS (@: every message,
  // post or comment that mentions this person — opened at that very item), each with its count (`activity`).
  for (const box of el.querySelectorAll(".inbox")) {
    const of = box.dataset.of;
    const btn = box.querySelector("button");
    const out = box.querySelector(".results");
    const label = btn.textContent;
    const h = (tag, props = {}, ...kids) => {
      const e = Object.assign(document.createElement(tag), props);
      e.append(...kids.filter(k => k != null && k !== false));
      return e;
    };
    const badge = n => (n ? h("span", { className: "cw-badge", textContent: String(n) }) : null);
    ctx
      .require("activity")
      .then(async activity => {
        const directory = await ctx.require("directory");
        const markdown = await ctx.require("markdown");
        const count = () => (of === "recent" ? activity.recent().reduce((n, x) => n + x.count, 0) : activity.mentionsNew());
        const drawBtn = () => btn.replaceChildren(label, ...[badge(count())].filter(Boolean));
        const drawList = () => {
          const list = of === "recent" ? activity.recent() : activity.mentions().slice(0, 30);
          out.replaceChildren(
            ...(list.length
              ? list.map(x =>
                  h(
                    "button",
                    { type: "button", onclick: () => ((out.hidden = true), of === "recent" ? (location.hash = x.route) : x.open()) },
                    h("span", { className: "where" }, x.by ? directory.nameEl(x.by) : null, ` · ${x.where}${x.count > 1 ? ` · ${x.count} new` : ""} · ${new Date(x.at).toLocaleString()}`),
                    h("span", { className: "what", textContent: markdown.plain(x.text ?? "") }),
                  ),
                )
              : [h("p", { className: "none", textContent: of === "recent" ? "Nothing new." : "Nobody has mentioned you yet." })]),
          );
        };
        drawBtn();
        activity.onChange(() => (drawBtn(), out.hidden || drawList()));
        btn.onclick = () => {
          out.hidden = !out.hidden;
          if (!out.hidden) (drawList(), of === "mentions" && activity.mentionsSeen().catch(() => {}));
        };
      })
      .catch(() => (box.hidden = true));
    addEventListener("mousedown", e => !out.hidden && !box.contains(e.target) && (out.hidden = true), true);
  }
  // SEARCH, across every app: a shared item's LINK (`items.linkOf`, from any address — a node's, a server's own domain)
  // or its reference opens it; `#tag` its tag's page; text finds APPS, SPACES, PEOPLE (`person.mentionable`: their
  // card) and ITEMS of every kind — what this person reads (their spaces, whom they follow) and Discover's.
  {
    const box = el.querySelector(".find");
    const input = box.querySelector("input");
    const out = box.querySelector(".results");
    const h = (tag, props = {}, ...kids) => {
      const e = Object.assign(document.createElement(tag), props);
      e.append(...kids.filter(k => k != null && k !== false));
      return e;
    };
    const close = () => (out.hidden = true);
    const go = href => (close(), (input.value = ""), (location.hash = href));
    let itemsHeld = null; // the items read once per opening of the search (not per key)
    let asked = 0;
    async function find() {
      const q = input.value.trim();
      const n = ++asked;
      if (!q) return close();
      const [items, space, person, kinds] = await Promise.all(["items", "space", "person", "kinds"].map(x => ctx.require(x)));
      const groups = [];
      const ref = items.refOf(q);
      if (ref) {
        const page = /#\/\S*\/p\//.test(q) ? q.slice(q.indexOf("#")) : null;
        groups.push(["Open", [h("button", { type: "button", textContent: "🔗 Open this item", onclick: async () => go(page ?? items.pageOf(ref, (await items.get(ref).catch(() => null))?.kind ?? "post")) })]]);
      } else if (/^did:craftec:[1-9A-HJ-NP-Za-km-z]{20,64}$/.test(q)) {
        // A PERSON's id: their card, and their space.
        const shownAs = (await ctx.require("directory")).nameEl(q);
        groups.push(["Person", [h("button", { type: "button", onclick: e => person.open(e.currentTarget, q) }, "👤 ", shownAs, " — their card"), h("a", { href: `#/u/${q}`, textContent: "🏠 Their space", onclick: e => (e.preventDefault(), go(`#/u/${q}`)) })]]);
      } else if (/^(space:)?[0-9a-f]{64}$/.test(q)) {
        // A SPACE's id: opened (yours, or a public one).
        const id = q.replace(/^space:/, "");
        const known = [...(await space.mine().catch(() => [])), ...(await items.publicSpaces().catch(() => []))].find(s => s.id === id);
        groups.push(["Space", [h("a", { href: `#/s/${id}`, textContent: `🏠 ${known?.name ?? "This space"}`, onclick: e => (e.preventDefault(), go(`#/s/${id}`)) })]]);
      } else if (/^#[^\s#]+$/.test(q)) {
        groups.push(["Tag", [h("a", { href: `#/tag/${encodeURIComponent(q.slice(1).toLowerCase())}`, textContent: `# ${q.slice(1).toLowerCase()}`, onclick: e => (e.preventDefault(), go(`#/tag/${encodeURIComponent(q.slice(1).toLowerCase())}`)) })]]);
      } else {
        const t = q.replace(/^@/, "").toLowerCase();
        const apps = ctx.apps.filter(a => (a.views ?? []).includes("personal") && a.name.toLowerCase().includes(t));
        if (apps.length) groups.push(["Apps", apps.slice(0, 5).map(a => h("a", { href: `#${a.route}`, textContent: `${a.icon ?? ""} ${a.name}`, onclick: e => (e.preventDefault(), go(`#${a.route}`)) }))]);
        const mine = (await space.mine().catch(() => [])).filter(s => s.kind === "server" && !s.group);
        const pub = await items.publicSpaces().catch(() => []);
        const seen = new Set();
        const sps = [...mine, ...pub].filter(s => s?.name && s.name.toLowerCase().includes(t) && !seen.has(s.id) && seen.add(s.id)).slice(0, 5);
        if (sps.length) groups.push(["Spaces", sps.map(s => h("a", { href: `#/s/${s.id}`, textContent: `🏠 ${s.name}`, onclick: e => (e.preventDefault(), go(`#/s/${s.id}`)) }))]);
        if (n !== asked) return;
        const people = (await person.mentionable(t).catch(() => [])).slice(0, 5);
        if (people.length) groups.push(["People", people.map(p => h("button", { type: "button", textContent: `👤 ${p.shown}`, onclick: e => person.open(e.currentTarget, p.did) }))]);
        out.replaceChildren(...groups.flatMap(([head, rows]) => [h("div", { className: "head", textContent: head }), ...rows]), h("p", { className: "none", textContent: "Looking in items…" }));
        out.hidden = false;
        const shown = kinds.all().filter(k => !["channel", "folder"].includes(k));
        itemsHeld ??= Promise.all([
          items.list({ feed: true }, "new", shown, { window: "all" }).catch(() => []),
          items.list({ discover: true }, "new", shown, { window: "all" }).catch(() => []),
        ]).then(([a, b]) => {
          const seen = new Set();
          return [...a, ...b].filter(it => !seen.has(it.ref) && seen.add(it.ref));
        });
        const found = (await itemsHeld).filter(it => `${it.title ?? ""} ${it.body ?? ""}`.toLowerCase().includes(t)).slice(0, 8);
        if (n !== asked) return;
        if (found.length) groups.push(["Items", found.map(it => h("a", { href: items.pageOf(it.ref, it.kind), textContent: `${ctx.apps.find(a => a.route === `/${items.appOf(it.kind)}`)?.icon ?? "📄"} ${it.title || String(it.body ?? "").slice(0, 60) || "untitled"}`, onclick: e => (e.preventDefault(), go(items.pageOf(it.ref, it.kind))) }))]);
      }
      if (n !== asked) return;
      out.replaceChildren(...(groups.length ? groups.flatMap(([head, rows]) => [h("div", { className: "head", textContent: head }), ...rows]) : [h("p", { className: "none", textContent: `Nothing found for “${q}”.` })]));
      out.hidden = false;
    }
    let timer = null;
    input.addEventListener("input", () => (clearTimeout(timer), (timer = setTimeout(() => find().catch(() => {}), 250))));
    input.addEventListener("focus", () => ((itemsHeld = null), input.value.trim() && find().catch(() => {})));
    input.addEventListener("keydown", e => {
      if (e.key === "Escape") return close(), input.blur();
      if (e.key === "Enter") out.querySelector("a, button")?.click();
    });
    addEventListener("mousedown", e => !out.hidden && !box.contains(e.target) && !e.target.closest?.(".cw-person") && close(), true);
  }
  // A DROPDOWN: a button, and its list of links and actions (closed by a choice, a click outside or Escape).
  const dropdown = (label, entries, item) => {
    const list = Object.assign(document.createElement("div"), { className: "list", hidden: true });
    list.setAttribute("role", "menu");
    list.append(
      ...entries.map(
        // A link, or an action (a button: a `#` link is the loader's, never a click handler's).
        m => item(m) ?? Object.assign(document.createElement("button"), { type: "button", textContent: m.label, onclick: () => m.run() }),
      ),
    );
    // What is new in its entries (their `count`s): a badge on the closed menu too.
    const total = entries.reduce((t, m) => t + (m.count || 0), 0);
    const b = Object.assign(document.createElement("button"), { type: "button", textContent: `${label} ▾` });
    if (total) b.append(Object.assign(document.createElement("span"), { className: "cw-badge", textContent: String(total) }));
    b.setAttribute("aria-haspopup", "menu");
    const wrap = Object.assign(document.createElement("span"), { className: "drop" });
    const close = () => {
      list.hidden = true;
      removeEventListener("click", outside, true);
      removeEventListener("keydown", esc);
    };
    const outside = e => !wrap.contains(e.target) && close();
    const esc = e => e.key === "Escape" && close();
    b.onclick = () => {
      if (!list.hidden) return close();
      list.hidden = false;
      addEventListener("click", outside, true);
      addEventListener("keydown", esc);
    };
    list.onclick = e => e.target.closest("a, button") && close();
    wrap.append(b, list);
    return wrap;
  };
  const link = a => {
    if (!a.href) return null;
    const l = Object.assign(document.createElement("a"), { href: a.href, textContent: a.label });
    // What is new in it (`activity`): its badge, as on its Home tile.
    if (a.count) l.append(" ", Object.assign(document.createElement("span"), { className: "cw-badge", textContent: String(a.count) }));
    if (a.on) l.setAttribute("aria-current", "page");
    return l;
  };

  // THE APP SWITCHER: the apps of the space open (a shared space: its Home and the apps it uses; Personal: Home and the
  // apps with a personal view). Logged out: "Home".
  let drawing = 0;
  const watched = new Set();
  async function switcher() {
    const n = ++drawing;
    const box = el.querySelector(".name");
    const app = ctx.apps.find(a => a.route === ctx.route);
    const here = ctx.route === "/" || ctx.route === "/space" ? "Home" : ctx.route === "/settings" ? "Settings" : (app?.name ?? "");
    // Asked quietly (never a login dialog: the header is on every page).
    const session = await ctx.require("auth").then(a => a.check()).catch(() => null);
    if (n !== drawing) return;
    if (!session) return box.replaceChildren(here);
    let entries, who;
    // WHAT IS NEW per app (`activity`: the same counts as each Home's tiles) — redrawn as they change.
    const activity = await ctx.require("activity").catch(() => null);
    if (activity && !watched.has("activity")) (watched.add("activity"), activity.onChange(() => switcher()));
    // (DISCOVER is your personal space's: each app's Discover tab — the personal apps here, as at home.)
    if (ctx.space && ctx.space !== "discover") {
      const space = await ctx.require("space");
      const sp = (await space.mine()).find(s => s.id === ctx.space);
      const r = sp ? await (await ctx.require("roles")).of(sp) : null;
      const on = r ? r.apps() : [];
      // The space's apps changing (one added or removed): the switcher again.
      if (r && !watched.has(sp.id)) {
        watched.add(sp.id);
        r.onChange(() => ctx.space === sp.id && switcher());
      }
      entries = [
        { label: `${await (await ctx.require("spaces-panel")).here()} · Home`, href: `#/s/${ctx.space}`, on: ctx.route === "/space" },
        ...ctx.apps.filter(a => (a.views ?? []).includes("shared") && (a.always || on.includes(a.route.slice(1)))).map(a => ({ label: `${a.icon ?? ""} ${a.name}`, href: `#/s/${ctx.space}${a.route}`, on: a.route === ctx.route, count: activity && ["chat", "board"].includes(a.route.slice(1)) ? activity.of(ctx.space, a.route.slice(1)) : 0 })),
      ];
    } else if ((who = await ctx.require("where").then(w => w.personOf())) && who !== (await ctx.require("space").then(s => s.account()))?.id) {
      // A PERSON's space: their Home and the apps that show someone's space (the manifest's `person` view) — staying theirs.
      const name = (await ctx.require("directory")).shown(who, await (await ctx.require("directory")).handle(who).catch(() => null));
      entries = [
        { label: `${name} · Home`, href: `#/u/${who}`, on: ctx.route === "/" },
        ...ctx.apps.filter(a => (a.views ?? []).includes("person")).map(a => ({ label: `${a.icon ?? ""} ${a.name}`, href: `#${a.route}/u/${who}`, on: a.route === ctx.route })),
      ];
    } else
      entries = [
        { label: "Home", href: "#/", on: ctx.route === "/" },
        ...ctx.apps.filter(a => (a.views ?? []).includes("personal")).map(a => ({ label: `${a.icon ?? ""} ${a.name}`, href: a.route, on: a.route === ctx.route, count: a.counts && activity ? activity.total(a.counts) : 0 })),
      ].map(e => ({ ...e, href: e.href.startsWith("#") ? e.href : `#${e.href}` }));
    if (n !== drawing) return;
    box.replaceChildren(dropdown(here, entries, link));
  }

  // THE SPACE you are in: its name (logged in); a click opens the spaces panel. What is unread in your other spaces on it.
  const spaceB = el.querySelector(".space-name");
  spaceB.onclick = () => ctx.require("spaces-panel").then(p => p.open());
  let naming = 0;
  async function spaceName() {
    const n = ++naming;
    const session = await ctx.require("auth").then(a => a.check()).catch(() => null);
    if (n !== naming) return;
    spaceB.hidden = !session;
    if (!session) return;
    const [panel, activity] = await Promise.all([ctx.require("spaces-panel"), ctx.require("activity").catch(() => null)]);
    const name = await panel.here();
    const spaceCap = await ctx.require("space");
    const elsewhere = activity ? (await spaceCap.mine().catch(() => [])).filter(s => s.kind === "server" && !spaceCap.isGroup(s) && s.id !== ctx.space).reduce((t, s) => t + (activity.of(s.id) ?? 0), 0) : 0;
    if (n !== naming) return;
    spaceB.replaceChildren(`${name} ▾`, ...(elsewhere ? [Object.assign(document.createElement("span"), { className: "cw-badge", textContent: String(elsewhere) })] : []));
    if (activity && !spaceB.dataset.watching) (spaceB.dataset.watching = "1"), activity.onChange(() => spaceName());
  }
  const draw = () => {
    spaceName();
    switcher();
    const actions = el.querySelector(".actions");
    const all = ctx.actions[ctx.route] ?? [];
    const item = link;
    el.querySelector(".end").replaceChildren(
      // At the END (right): a link (a create page) or a button (Log out, Compose).
      ...all.filter(a => a.end).map(a => item(a) ?? Object.assign(document.createElement("button"), { type: "button", textContent: a.label, onclick: () => a.run() })),
    );
    actions.replaceChildren(
      ...all.filter(a => !a.end).map(a => {
        const link = item(a);
        if (link) return link;
        // A MENU: a button, and its dropdown.
        if (a.menu) return dropdown(a.label, a.menu, item);
        // A search box ({ search: fn, placeholder, value }) or a button ({ label, run, on }).
        if (a.search) {
          const input = document.createElement("input");
          input.type = "search";
          input.className = "search";
          input.placeholder = a.placeholder ?? "Search";
          input.value = a.value ?? "";
          input.oninput = () => a.search(input.value);
          return input;
        }
        const b = document.createElement("button");
        b.type = "button";
        b.textContent = a.label;
        if (a.on) b.setAttribute("aria-pressed", "true");
        b.onclick = () => a.run();
        return b;
      }),
    );
  };
  draw();
  addEventListener("craftworks:route", draw);
  addEventListener("craftworks:actions", draw);
  addEventListener("craftworks:auth", () => (spaceName(), switcher()));
  // THE BACKGROUND — on every page, AFTER it: started one at a time, each once the node is idle (the page's own reads
  // answered), never on a clock that lands among them. ACTIVITY (what is new: notified, whichever app is open);
  // UPKEEP (welcomes joined, askers let in); FILE KEYS (every file on the key its access calls for); KEEP (this person's
  // data kept on the network); VIDEO STUDIO (renditions still to make).
  (async () => {
    const node = await ctx.require("node");
    for (const name of ["activity", "upkeep", "file-keys", "keep", "video-studio"]) {
      await node.idle();
      await ctx.require(name).catch(() => {});
    }
  })().catch(() => {});
}
