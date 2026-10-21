// WHERE, a capability: the ONE answer every app asks of its address — whose items it shows, how they are read, how
// its links stay there, and its standard tabs — so every app behaves the same for free:
//   YOURS      `#/<app>`                 your personal space (yours to change)
//   A SPACE's  `#/s/<space>/<app>`       a shared space you are in (its policies say what you may do)
//   A PERSON's `#/<app>/u/<did>`         someone's personal space, from outside (read only: what they let you read)
//   DISCOVER   `#/discover/<app>`        the public network — your space's Discover tab (read only)
// Its read is the ONE items read for that place (`items`: access control and privacy decide what loads); its tabs are
// the same in every app (yours: "Your …" and Discover), the app's own after them.
//
//   const where = await ctx.require("where");
//   const at = await where.of({ kind: "note", app: "note", yours: "Your notes" })
//   at.who            // "mine" | "space" | "person" | "discover"
//   at.saved          // your SAVED of this app (`#/<app>/saved`: what you saved, any place) — every app has it
//   at.shared         // SHARED with chosen people (`#/<app>/shared`: `share`'s item groups) — every app has it
//   at.others         // someone else's (a person's, Discover's): read only
//   at.space  at.person  at.me
//   at.sub            // the rest of the address, after the place (`f/Photos`, `p/<ref>`)
//   at.href(sub)      // a link that stays in this place
//   await at.read({ kinds, sort, window, whole })   // its items (whole: every one, not a list's window)
//   at.tabs(extra, { yoursOn, before, create })   // the header's tabs, in every app's ONE order: `before` (views
//                                 // ahead of yours), Yours, the extras, Saved, Discover — then `create` ("＋ …"),
//                                 // always last and shown wherever the tabs are (Saved, Discover too): it makes yours
//   where.spacesPage(host, app)   // DISCOVER: the public spaces that have the app (the loader draws it)
//   where.personOf()  // the person whose space is open, else null
//   where.key()       // the address as a key (an app redraws when it changes)
//   where.onMove(el, draw)   // the app drawn again for each new address of its own, while `el` is on the page
//   where.placeKey()  // the PLACE as a key (yours, a space, a person, Discover): another place, the app opened afresh
export async function start(ctx) {
  const { Fault } = await ctx.require("fault");
  const [space, items, roles] = await Promise.all(["space", "items", "roles"].map(n => ctx.require(n)));
  const personOf = () => (ctx.space ? null : (/^u\/(did:[^/]+)/.exec(ctx.sub ?? "")?.[1] ?? null));

  async function of({ kind, kinds = [kind], app = ctx.route.replace(/^\//, ""), yours = "Yours", saves = true } = {}) {
    const me = await space.me.strict();
    const discover = ctx.space === "discover";
    const mineSp = ctx.space && !discover ? ((await space.mine.strict()).find(s => s.id === ctx.space) ?? null) : null;
    // A space this person is NOT in (or anyone, for a guest): opened FROM OUTSIDE where the space lets anyone read this
    // app — its public description, its public acts' rules (the app on, its kind's `read` policy "anyone"); else said.
    let sp = mineSp;
    if (ctx.space && !discover && !sp) {
      const desc = await items.publicSpace(ctx.space);
      const pr = desc ? await roles.ofPublic.strict(desc) : null;
      if (pr) await pr.settled;
      const domain = (await ctx.require("kinds")).policyDomain(kind);
      // (Both refusals are ANSWERS, not failures: `expected` — the loader says them, never as a fault.)
      if (!desc) throw new Fault("refused-expected", "not in that space", { message: "you are not in that space (left, or not joined yet)" });
      // Why not, said (its public rules: the app not on there, or who reads it) — the one rule (`opensFromOutside`).
      const { on, reads } = await outsideRule(pr, app, domain);
      if (!on || reads !== "anyone") throw new Fault("refused-expected", "members only", { message: `${space.shown(desc)}'s ${app} is for its members (${on ? `read by: ${reads ?? "members"}` : "not public there"}): join it to open it` });
      sp = { ...desc, outside: true };
    }
    // An app the space TURNED OFF: its Home instead (where its apps are added) — only on that certainty, an `app` act
    // turning it off that counted. Never because it looks unused while the acts are still arriving (a member's acts
    // merge in after "settled": the app would be sent away from a space that uses it).
    const app_ = ctx.apps.find(a => a.route === `/${app}`);
    // ITS "YOURS", one name on every page (the manifest's: the app's own words — owner 10-22: "Yours" on one page, "Your
    // votes" on another, as each page named it).
    yours = app_?.yours ?? yours;
    if (sp)
      ctx.background("where: roles.of", roles
        .of.strict(sp)
        .then(async r => (await r.settled, r)).then(async r => {
          // (Awaited: in an app's frame a space's roles answer through its door, each a promise.)
          const last = (await r.acts("app")).filter(a => a.app === app).at(-1);
          if (last && !last.on && !(await r.apps()).includes(app) && ctx.space === sp.id && ctx.route === `/${app}`) location.replace(`#/s/${sp.id}`);
        }));
    const p = personOf();
    const person = p && p !== me ? p : null;
    // A GUEST (nobody logged in) has no "yours": an app with a public side opens its Discover, the address made so.
    const pub = (app_?.views ?? []).includes("public");
    const guestDiscover = !me && !discover && !sp && !person && pub;
    const who = discover || guestDiscover ? "discover" : sp ? "space" : person ? "person" : "mine";
    const base = who === "discover" ? `#/discover/${app}` : sp ? `#/s/${sp.id}/${app}` : person ? `#/${app}/u/${person}` : `#/${app}`;
    const sub = (p ? (ctx.sub ?? "").replace(/^u\/did:[^/]+\/?/, "") : (ctx.sub ?? "")).replace(/^\/+/, "");
    if (guestDiscover) history.replaceState(history.state, "", `#/discover/${app}${sub ? `/${sub}` : ""}`), dispatchEvent(new CustomEvent("craftworks:address"));
    const saved = saves && who === "mine" && /^saved(\/|$)/.test(sub);
    // SHARED: items shared with chosen people (`share`: each in its own group) — by this person or with them.
    const shared = saves && who === "mine" && /^shared(\/|$)/.test(sub);
    // WHO YOU FOLLOW, YOUR FRIENDS: their items of this app (the one feed's sources).
    const following = !!me && who === "mine" && /^following(\/|$)/.test(sub);
    const friends = !!me && who === "mine" && /^friends(\/|$)/.test(sub);
    // THE READ: the place's items — a list's window, or (whole) every one; your Saved: what you saved of its kinds.
    const read = ctx.total("where.read", () => [], ({ kinds: ks = kinds, sort = "new", window = "all", whole = false, ...options } = {}) => {
      // THE ONE FEED (`items.feed`): each page its source — Saved, Shared, Following, Friends, Home (the public).
      const src = saved ? { saved: true } : shared ? { shared: true } : following ? { following: true } : friends ? { friends: true } : who === "discover" ? { home: true } : null;
      if (src) return items.feed.strict(src, { kinds: ks, sort, window, ...options });
      if (whole) return items.inPlaces.strict(sp ? { spaces: [sp] } : { people: [person ?? me] }, ks, { withVotes: false });
      // YOURS (owner 10-20): your items — your personal space's and what you wrote in the spaces you are in.
      return items.feed.strict(sp?.outside ? { space: sp, outside: true } : sp ? { space: sp.id } : person ? { person } : { yours: true }, { kinds: ks, sort, window, ...options });
    });
    // THE TABS, in the same order in every app: in your space, the app's own page ("Your …"), then the app's extras,
    // then Discover (an app with a public side); elsewhere the extras alone.
    // THE APP's PLACES (You, each space with the app, Discover) are the SIDE PANEL's (`places.side`, owner 10-15) —
    // never listed in the menu as well.
    // THE APP's OWN SETTINGS in a space (ARCHITECTURE §1: an app defines its settings and shows them itself) — for who
    // may manage the space's apps, where the app has any (`app-settings`: Board's rules, Blog's approval, Chat's channels).
    const settingsRead = sp && !sp.outside && me
      ? ctx.optional("where: app settings", (async () => {
          const st = await ctx.require("app-settings");
          return (await st.has(app)) === true && (await (await roles.of.strict(sp)).can(me, "apps")) === true;
        })(), () => false)
      : Promise.resolve(false);
    // Each entry's ROLE, where the side panel puts it (owner 10-15, YouTube's: Home · Subscriptions, then You): `discover`,
    // `feed` (what you follow — `feed: true` where the app's own page IS its feed, as Videos'), `you` (yours, saved,
    // the app's extras unless they say otherwise), `create`.
    const tabs = (extra = [], { yoursOn = true, before = [], create = null, feed = false } = {}) => {
      const draw = () => {
      const home = who === "mine" || who === "discover";
      // THE SIDE PANEL, the same in every app (owner 10-21): HOME (the public items), DISCOVER (the public spaces with
      // this app), YOU (yours, following, friends, saved, shared, the app's own), then the spaces (`places.side`).
      const top = [
        { label: "Home", href: `#/discover/${app}`, on: who === "discover" && !/^spaces(\/|$)/.test(sub), role: "home" },
        { label: "Discover", href: `#/discover/${app}/spaces`, on: who === "discover" && /^spaces(\/|$)/.test(sub), role: "discover" },
      ];
      // A GUEST: nothing of theirs (Yours, Saved) and nothing to make — Log in instead; once in, the page starts again
      // (as at Log out: nothing made for the guest reaches the session).
      ctx.actions[ctx.route] = !me
        ? [
            ...top,
            ...extra.filter(e => e.public).map(e => ({ role: "you", ...e })),
            { label: "Log in", run: async () => (await (await ctx.require("login")).session()) && location.reload(), end: true, role: "create" },
          ]
        : [
            ...top,
            ...before.map(e => ({ role: "you", ...e })),
            { label: yours, href: `#/${app}`, on: who === "mine" && !saved && !shared && !following && !friends && yoursOn, role: "you" },
            { label: "Following", href: `#/${app}/following`, on: following, role: "you" },
            { label: "Friends", href: `#/${app}/friends`, on: friends, role: "you" },
            ...(saves ? [{ label: "Saved", href: `#/${app}/saved`, on: saved, role: "you" }] : []),
            ...(saves ? [{ label: "Shared", href: `#/${app}/shared`, on: shared, role: "you" }] : []),
            ...extra.map(e => ({ role: "you", ...e })),
            ...(create && who !== "person" && !sp?.outside ? [{ ...create, label: `＋ ${create.label}`, end: true, role: "create" }] : []),
          ];
      dispatchEvent(new CustomEvent("craftworks:actions"));
      };
      draw();
    };
    // ITS SETTINGS PAGE, drawn into `host` when the address is `…/settings` in a space: true when drawn (the app draws
    // nothing else then).
    const settings = async host => {
      if (sub !== "settings" || !sp || sp.outside) return false;
      // IN AN APP's FRAME: a SYSTEM page — the host draws it in the app's place (its rules are the space's governance,
      // never the app's to draw; FOUNDATION §3).
      if (ctx.system) return void (await ctx.system("settings", { kind, app, yours })), true;
      // (Only for who may manage the space's apps — the menu offers it to them alone, and so does the address.)
      if (!(await settingsRead)) return void host.replaceChildren(Object.assign(document.createElement("p"), { textContent: "Its owner and admins keep this app's settings." })), true;
      const page = await (await ctx.require("app-settings")).appPage(sp, app).catch(e => Object.assign(document.createElement("p"), { textContent: ctx.say(e) }));
      host.replaceChildren(page ?? Object.assign(document.createElement("p"), { textContent: "This app has no settings of its own." }));
      return true;
    };
    return { settings, who, saved, shared, following, friends, outside: !!sp?.outside, others: who === "person" || who === "discover" || saved || following || friends || !!sp?.outside, discover: who === "discover", guest: !me, space: sp, person, me, sub, base, href: s => (s ? `${base}/${s}` : base), read, tabs };
  }
  // A SPACE OPENED FROM OUTSIDE in an app (the one rule — opening it here, and Discover listing it): the app on there
  // and its domain read by anyone (10-22: Board's panel listed spaces by their posts' reading, Discover by the app — each
  // showed one the other refused).
  async function outsideRule(pr, app, domain) {
    await pr.settled;
    return { on: (await pr.apps()).includes(app), reads: await pr.policyIn(domain, "read") };
  }
  async function opensFromOutside(desc, app) {
    const pr = await roles.ofPublic.strict(desc);
    const a = ctx.apps.find(x => x.route === `/${app}`);
    const domains = a?.needs?.length ? a.needs : ["text"];
    for (const d of domains) {
      const { on } = await outsideRule(pr, app, d);
      if (on && (await pr.readsAnyone(d))) return true;
    }
    return false;
  }
  // THE PUBLIC SPACES AN APP OPENS IN (the one list — Discover's page and any panel listing them draw it: 10-22, Board's
  // panel and Discover each kept a list of their own): `[{ d, pr }]`.
  async function spacesFor(app) {
    const all = await items.publicSpaces();
    const ok = await Promise.all(all.map(d => ctx.optional("where: opens from outside", opensFromOutside(d, app), () => false)));
    const prs = await Promise.all(all.map((d, i) => (ok[i] ? roles.ofPublic.strict(d) : null)));
    const out = all.map((d, i) => ok[i] && { d, pr: prs[i] }).filter(Boolean);
    // (Still arriving as the list it is drawn from is: some spaces not decided yet.)
    if (all.arriving) Object.defineProperty(out, "arriving", { value: true });
    return out;
  }
  // AN APP's DISCOVER PAGE (`#/discover/<app>/spaces`): the public spaces that have this app on — each opened, joined
  // or followed there (`join-button`). The same page for every app (the loader draws it in the app's place).
  async function spacesPage(host, app) {
    const h = (await ctx.require("theme")).h;
    const shown = ctx.apps.find(a => a.route === `/${app}`);
    // ITS SIDE PANEL, as on every page of the app (`tabs`: Home · Discover · You · Spaces).
    (await of({ kind: (await ctx.require("kinds")).inDomain?.(shown?.needs?.[0] ?? "")?.[0] ?? "post", app })).tabs([]);
    host.classList.add("cw-discover-spaces");
    const css = h("style", { textContent: `@layer components {
      .cw-discover-spaces .ds { display: grid; gap: var(--cw-space-3); padding: var(--cw-space-3) var(--cw-gutter); }
      .cw-discover-spaces .ds h2 { margin: 0; }
      .cw-discover-spaces .ds .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: var(--cw-space-3); }
      .cw-discover-spaces .ds .card { display: grid; gap: 8px; padding: var(--cw-space-3); border: 1px solid var(--cw-line); border-radius: var(--cw-radius); background: var(--cw-surface); }
      .cw-discover-spaces .ds .card a.name { font-weight: 600; color: var(--cw-fg); text-decoration: none; }
      .cw-discover-spaces .ds .about { color: var(--cw-muted); font-size: var(--cw-text-sm); margin: 0; }
    }` });
    const box = h("div", { className: "ds" });
    host.replaceChildren(css, box);
    ctx.view(box, {
      read: async () => {
        // (The one list: `spacesFor`.)
        return spacesFor(app);
      },
      empty: list => !list.length,
      draw: async list => {
        const join = await ctx.require("join-button");
        const cards = await Promise.all(list.map(async ({ d, pr }) => h("div", { className: "card" },
          h("a", { className: "name", href: `#/s/${d.id}/${app}`, textContent: d.name || `A space #${d.id.slice(0, 6)}` }),
          d.about ? h("p", { className: "about", textContent: String(d.about).slice(0, 200) }) : null,
          await ctx.optional("where: join", join.control(d, { open: `#/s/${d.id}/${app}`, policy: pr.policy("", "join") }), () => null))));
        return h("div", { style: "display:contents" }, h("h2", { textContent: `Spaces with ${shown?.name ?? app}` }), h("div", { className: "grid" }, ...cards));
      },
      looks: { empty: () => h("p", { textContent: `No public space has ${shown?.name ?? app} yet.` }) },
    });
  }
  // A MENU of an OS page's own (Settings' sections, Mail's folders, the token pages): the ONE door for an app's entries
  // besides `tabs` — each its ROLE (`you`, or `create` for an ending action), told to the header and the side panel.
  // `route`: the page's (default: the one open).
  function menu(entries, route = ctx.route) {
    ctx.actions[route] = (entries ?? []).map(e => ({ role: e.end ? "create" : "you", ...e }));
    dispatchEvent(new CustomEvent("craftworks:actions"));
  }
  const key = () => `${ctx.space ?? ""}|${ctx.sub ?? ""}`;
  // AN APP DRAWN AGAIN for another address of its own (a link inside it: an item opened, one just made, another tab of
  // it): `onMove(el, draw)` — while `el` is on the page and the route is still the app's, once per new address. THE one
  // rule (every app had its own copy, some redrawing on any route event; Docs had none: 10-20, Create did nothing).
  function onMove(el, draw, route = ctx.route) {
    let drawn = key();
    const moved = () => {
      if (!el.isConnected) return removeEventListener("craftworks:route", moved);
      if (ctx.route !== route || key() === drawn) return;
      drawn = key();
      draw();
    };
    addEventListener("craftworks:route", moved);
    return () => removeEventListener("craftworks:route", moved); // (stop: an app mounted again in the same element)
  }
  return { of, menu, spacesPage, spacesFor, opensFromOutside, personOf, onMove, key, placeKey: () => `${ctx.space ?? ""}|${personOf() ?? ""}` };
}
