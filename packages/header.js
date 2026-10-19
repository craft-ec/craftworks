// HEADER: a top bar like a Mac's menu bar. On the left: ⌂ (home) and the CURRENT APP's name; the search across every
// app; on the right: notifications, mentions, background work. The app's MENU — its sub-pages and actions, what it put
// under its route in `ctx.actions` — is the side panel's (`places-side`), never drawn here (owner 10-15: "remove the top
// navigation and maintain only side navigation"; until 10-18 the header still drew it until the panel took it over —
// the menu shown on top first, then moved to the side). The app's own component (the layout names it), so editing it is
// publishing the app, never the loader.
export function mount(ctx, el) {
  // (Its elements by `theme.h`, taken where each part first draws: the bar itself mounts at once.)
  const theme = ctx.require("theme");
  el.innerHTML = `
    <style>@layer components {
      .bar { display: flex; align-items: center; gap: 14px; border-bottom: 1px solid var(--cw-line); height: var(--cw-bar); box-sizing: border-box;
        font-size: var(--cw-text-sm); }
        border-radius: var(--cw-radius-sm); max-width: 16em; min-width: 6em; flex: 0 1 auto; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .bar .name { font-weight: 600; }
      .bar .end { margin-left: auto; }
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
      /* A PHONE: the search box shrinks to its icon and opens out while typed in; the header's buttons never wrap. */
      @media (max-width: 640px) { .bar .find > input { width: 2.4em; padding: 2px 6px; transition: width .15s; } .bar .find > input:focus { width: 11em; }
        .bar { gap: 8px; }
        /* The search on a second row of its own, the whole width (the theme's --cw-bar is two rows here). */
        .bar { flex-wrap: wrap; align-content: center; row-gap: 2px; height: auto; min-height: 48px; padding-block: 4px; }
        .bar::after { content: ""; order: 9; flex-basis: 100%; height: 0; }
        .bar .find { order: 11; }
      }
      .bar .inbox > button { white-space: nowrap; }
      .bar .find, .bar .inbox, .bar .end { flex: 0 0 auto; }
      .bar .inbox { position: relative; }
      .bar .inbox .results { position: absolute; top: calc(100% + 4px); right: 0; z-index: 60; display: grid; padding: var(--cw-space-1); width: min(26em, calc(100vw - 32px));
        max-height: 70vh; overflow-y: auto; background: var(--cw-surface); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); box-shadow: var(--cw-shadow-lg); }
      .bar .inbox .results[hidden] { display: none; }
      .bar .inbox .results button { display: grid; gap: 2px; text-align: left; font: inherit; color: inherit; border: 0; background: none; padding: 6px var(--cw-space-2);
        border-radius: var(--cw-radius-sm); cursor: pointer; }
      .bar .inbox .results button:hover { background: var(--cw-hover); }
      .bar .inbox .results .where { color: var(--cw-muted); font-size: var(--cw-text-xs); }
      .bar .inbox .results .what { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      .bar .inbox .results .none { color: var(--cw-muted); padding: 6px var(--cw-space-2); margin: 0; }

    }</style>
    <nav class="bar">
      <span class="name"></span>
      <span class="end"></span>
      <span class="inbox" data-of="recent"><button type="button" class="cw-flat" title="Notifications">🔔</button><div class="results" hidden></div></span>
      <span class="inbox" data-of="mentions"><button type="button" class="cw-flat" title="Mentions">@</button><div class="results" hidden></div></span>
      <span class="inbox tasks" hidden><button type="button" class="cw-flat" title="Background work: uploads and videos still being made">⏳</button><div class="results" hidden></div></span>
      <span class="find"><input type="search" placeholder="🔍 Search or paste a link" aria-label="Search everything" autocomplete="off"><div class="results" hidden></div></span>
    </nav>`;
  // NOTIFICATIONS (🔔: each place with something unread, newest first — opened there) and MENTIONS (@: every message,
  // post or comment that mentions this person — opened at that very item), each with its count (`activity`).
  for (const box of el.querySelectorAll(".inbox:not(.tasks)")) {
    const of = box.dataset.of;
    const btn = box.querySelector("button");
    const out = box.querySelector(".results");
    const label = btn.textContent;
    ctx.optional("header: activity", ctx
      .require("activity")
      .then(async activity => {
        const { h } = await theme;
        const badge = n => (n ? h("span", { className: "cw-badge", textContent: String(n) }) : null);
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
          if (!out.hidden) (drawList(), of === "mentions" && ctx.background("header: activity.mentionsSeen", activity.mentionsSeen()));
        };
      }), () => box.hidden = true);
    addEventListener("mousedown", e => !out.hidden && !box.contains(e.target) && (out.hidden = true), true);
  }
  // BACKGROUND WORK (⏳, shown only while there is some): this person's uploads not finished and videos with renditions
  // still to make — the stage, how far, on this page or another, and what went quiet (its page gone: an open page of
  // theirs takes it over within a minute). Read every 30 s; every 3 s while open. A video opens its page.
  {
    const box = el.querySelector(".tasks");
    const btn = box.querySelector("button");
    const out = box.querySelector(".results");
    const pct = p => `${Math.round((p || 0) * 100)}%`;
    let list = [];
    const read = async () => {
      if (!(await ctx.require("auth")).current()) return (list = []);
      const [files, studio] = await Promise.all([ctx.require("files"), ctx.require("video-studio")]);
      const [ups, vids] = await Promise.all([ctx.optional("header: files.uploads", files.uploads(), () => []), ctx.optional("header: studio.queue", studio.queue(), () => [])]);
      list = [
        ...ups.map(u => ({ what: `Uploading ${u.name}`, where: `${pct(u.done)} stored · ${(u.size / 1e6).toFixed(0)} MB · resumes when the file is picked again` })),
        ...vids.map(v => ({
          what: v.title,
          where: `${v.done} of ${v.of} versions · ${v.stage}${v.p ? ` ${pct(v.p)}` : ""}${v.here ? " · on this page" : v.stalled ? " · an open page of yours takes it over within a minute" : ""} · next: ${v.next.join(", ")}`,
          open: () => (location.hash = `#/video/p/${v.ref}`),
        })),
      ];
    };
    const draw = async () => {
      const { h } = await theme;
      box.hidden = !list.length;
      btn.replaceChildren("⏳", ...(list.length ? [h("span", { className: "cw-badge", textContent: String(list.length) })] : []));
      if (out.hidden) return;
      out.replaceChildren(
        ...(list.length
          ? list.map(x => h("button", { type: "button", onclick: () => x.open && ((out.hidden = true), x.open()) }, h("span", { className: "what", textContent: x.what }), h("span", { className: "where", textContent: x.where })))
          : [h("p", { className: "none", textContent: "Nothing in the background." })]),
      );
    };
    const tick = () => ctx.background("header: read", read().then(draw));
    // Drawn when what it lists CHANGES (an upload's progress, a lease, a video) — never polled (10-15: it read both
    // every 3 s while open, every 30 s closed). Its one clock: "stalled" ages while the panel is open.
    setTimeout(() => {
      tick();
      ctx.background("header: follow", (async () => {
        if (!(await ctx.require("auth")).current()) return; // a guest has no queue (logging in starts the page again)
        const [files, studio] = await Promise.all([ctx.require("files"), ctx.require("video-studio")]);
        await files.onUploads(tick);
        await studio.onQueue(tick);
      })());
    }, 20000);
    ctx.every(out, 30000, () => !out.hidden && tick());
    btn.onclick = () => ((out.hidden = !out.hidden), draw(), tick());
    addEventListener("mousedown", e => !out.hidden && !box.contains(e.target) && (out.hidden = true), true);
  }
  // SEARCH, across every app: a shared item's LINK (`items.linkOf`, from any address — a node's, a server's own domain)
  // or its reference opens it; `#tag` its tag's page; text finds APPS, SPACES, PEOPLE (`person.mentionable`: their
  // card) and ITEMS of every kind — what this person reads (their spaces, whom they follow) and Discover's.
  {
    const box = el.querySelector(".find");
    const input = box.querySelector("input");
    const out = box.querySelector(".results");
    const close = () => (out.hidden = true);
    const go = href => (close(), (input.value = ""), (location.hash = href));
    let itemsHeld = null; // the items read once per opening of the search (not per key)
    let asked = 0;
    async function find() {
      const { h } = await theme;
      const q = input.value.trim();
      const n = ++asked;
      if (!q) return close();
      const [items, space, person, kinds] = await Promise.all(["items", "space", "person", "kinds"].map(x => ctx.require(x)));
      const groups = [];
      const ref = items.refOf(q);
      if (ref) {
        const page = /#\/\S*\/p\//.test(q) ? q.slice(q.indexOf("#")) : null;
        groups.push(["Open", [h("button", { type: "button", textContent: "🔗 Open this item", onclick: async () => go(page ?? items.pageOf(ref, (await items.get(ref))?.kind ?? "post")) })]]);
      } else if (/^did:craftec:[1-9A-HJ-NP-Za-km-z]{20,64}$/.test(q)) {
        // A PERSON's id: their card, and their space.
        const shownAs = (await ctx.require("directory")).nameEl(q);
        groups.push(["Person", [h("button", { type: "button", onclick: e => person.open(e.currentTarget, q) }, "👤 ", shownAs, " — their card"), h("a", { href: `#/u/${q}`, textContent: "🏠 Their space", onclick: e => (e.preventDefault(), go(`#/u/${q}`)) })]]);
      } else if (/^(space:)?[0-9a-f]{64}$/.test(q)) {
        // A SPACE's id: opened (yours, or a public one).
        const id = q.replace(/^space:/, "");
        const known = [...(await space.mine()), ...(await items.publicSpaces())].find(s => s.id === id);
        groups.push(["Space", [h("a", { href: `#/s/${id}`, textContent: `🏠 ${known?.name ?? "This space"}`, onclick: e => (e.preventDefault(), go(`#/s/${id}`)) })]]);
      } else if (/^#[^\s#]+$/.test(q)) {
        groups.push(["Tag", [h("a", { href: `#/tag/${encodeURIComponent(q.slice(1).toLowerCase())}`, textContent: `# ${q.slice(1).toLowerCase()}`, onclick: e => (e.preventDefault(), go(`#/tag/${encodeURIComponent(q.slice(1).toLowerCase())}`)) })]]);
      } else {
        const t = q.replace(/^@/, "").toLowerCase();
        const apps = ctx.apps.filter(a => (a.views ?? []).includes("personal") && a.name.toLowerCase().includes(t));
        if (apps.length) groups.push(["Apps", apps.slice(0, 5).map(a => h("a", { href: ctx.entryOf(a), textContent: `${a.icon ?? ""} ${a.name}`, onclick: e => (e.preventDefault(), go(`#${a.route}`)) }))]);
        const mine = (await space.mine()).filter(s => s.kind === "server" && !s.group);
        const pub = await items.publicSpaces();
        const seen = new Set();
        const sps = [...mine, ...pub].filter(s => s?.name && s.name.toLowerCase().includes(t) && !seen.has(s.id) && seen.add(s.id)).slice(0, 5);
        if (sps.length) groups.push(["Spaces", sps.map(s => h("a", { href: `#/s/${s.id}`, textContent: `🏠 ${s.name}`, onclick: e => (e.preventDefault(), go(`#/s/${s.id}`)) }))]);
        if (n !== asked) return;
        const people = (await person.mentionable(t)).slice(0, 5);
        if (people.length) groups.push(["People", people.map(p => h("button", { type: "button", textContent: `👤 ${p.shown}`, onclick: e => person.open(e.currentTarget, p.did) }))]);
        out.replaceChildren(...groups.flatMap(([head, rows]) => [h("div", { className: "head", textContent: head }), ...rows]), h("p", { className: "none", textContent: "Looking in items…" }));
        out.hidden = false;
        const shown = kinds.all().filter(k => !["channel", "folder", "delegation"].includes(k));
        itemsHeld ??= Promise.all([
          items.list({ feed: true }, "new", shown, { window: "all" }),
          items.list({ discover: true }, "new", shown, { window: "all" }),
        ]).then(([a, b]) => {
          const seen = new Set();
          return [...a, ...b].filter(it => !seen.has(it.ref) && seen.add(it.ref));
        });
        const found = (await itemsHeld).filter(it => !(it.meta?.expires < Date.now()) && `${it.title ?? ""} ${it.body ?? ""}`.toLowerCase().includes(t)).slice(0, 8);
        if (n !== asked) return;
        if (found.length) groups.push(["Items", found.map(it => h("a", { href: items.pageOf(it.ref, it.kind), textContent: `${ctx.apps.find(a => a.route === `/${items.appOf(it.kind)}`)?.icon ?? "📄"} ${it.title || String(it.body ?? "").slice(0, 60) || "untitled"}`, onclick: e => (e.preventDefault(), go(items.pageOf(it.ref, it.kind))) }))]);
      }
      if (n !== asked) return;
      out.replaceChildren(...(groups.length ? groups.flatMap(([head, rows]) => [h("div", { className: "head", textContent: head }), ...rows]) : [h("p", { className: "none", textContent: `Nothing found for “${q}”.` })]));
      out.hidden = false;
    }
    let timer = null;
    input.addEventListener("input", () => (clearTimeout(timer), (timer = setTimeout(() => ctx.background("header: find", find()), 250))));
    input.addEventListener("focus", () => ((itemsHeld = null), input.value.trim() && ctx.background("header: find", find())));
    input.addEventListener("keydown", e => {
      if (e.key === "Escape") return close(), input.blur();
      if (e.key === "Enter") out.querySelector("a, button")?.click();
    });
    addEventListener("mousedown", e => !out.hidden && !box.contains(e.target) && !e.target.closest?.(".cw-person") && close(), true);
  }
  // THE APP's NAME (ARCHITECTURE §1, "The desktop": no space dropdown, no app dropdown — apps are opened from Home, and
  // where an app is, its spaces, is the APP's to show: `places`, as its manifest says).
  function switcher() {
    const box = el.querySelector(".name");
    const app = ctx.apps.find(a => a.route === ctx.route);
    const here = ctx.route === "/" || ctx.route === "/space" ? "Home" : ctx.route === "/settings" ? "Settings" : (app?.name ?? "");
    box.replaceChildren(app ? `${app.icon ?? ""} ${here}` : here);
  }

  // THE SPACE you are in: its name (logged in); a click opens the spaces panel. What is unread in your other spaces on it.
  // THE HEADER's HEIGHT, as it is (two rows on a phone): what panels below it place by —
  // `--cw-bar-now`, never `--cw-bar` (the bar's own height: 10-15, measured while a remount had it collapsed, it wrote
  // `--cw-bar: 1px` and the bar locked itself at 1px — the header cut off). A detached bar (an older mount) stops.
  const barEl = el.querySelector(".bar");
  if (barEl && typeof ResizeObserver === "function") {
    const ro = new ResizeObserver(() => {
      if (!barEl.isConnected) return void ro.disconnect();
      if (barEl.offsetHeight > 0) document.documentElement.style.setProperty("--cw-bar-now", `${barEl.offsetHeight}px`);
    });
    ro.observe(barEl);
  }
  switcher();
  addEventListener("craftworks:route", switcher);
  addEventListener("craftworks:auth", () => switcher());
  // THE BACKGROUND — on every page, AFTER it: started one at a time, each once the node is idle (the page's own reads
  // answered), never on a clock that lands among them. ACTIVITY (what is new: notified, whichever app is open);
  // UPKEEP (spaces told of, entered); FILE KEYS (every file on the key its access calls for); KEEP (this person's
  // data kept on the network); VIDEO STUDIO (renditions still to make); MEMBERS (every space's keys kept to its list).
  ctx.background("header: async", (async () => {
    const node = await ctx.require("node");
    for (const name of ["activity", "upkeep", "members", "file-keys", "keep", "video-studio"]) {
      await node.idle();
      await ctx.background("header: ctx.require", ctx.require(name));
    }
  })());
}
