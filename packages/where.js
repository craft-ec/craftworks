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
//   where.personOf()  // the person whose space is open, else null
//   where.key()       // the address as a key (an app redraws when it changes)
//   where.placeKey()  // the PLACE as a key (yours, a space, a person, Discover): another place, the app opened afresh
export async function start(ctx) {
  const { Fault } = await ctx.require("fault");
  const [space, items, roles] = await Promise.all(["space", "items", "roles"].map(n => ctx.require(n)));
  const personOf = () => (ctx.space ? null : (/^u\/(did:[^/]+)/.exec(ctx.sub ?? "")?.[1] ?? null));

  // A SAVED item, read where it is (a space's this person is not in: from outside, by its public description).
  async function getSaved(ref) {
    const outside = await items.outsideOf(ref);
    if (String(ref).startsWith("space:") && !outside && !(await items.boardOf.strict(ref.slice(6, ref.indexOf("/"))))) return null;
    return items.get(ref, { outside });
  }

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
      // Why not, said (its public rules: the app not on there, or who reads it).
      // (Its roles read with `await`: the same here and in an app's frame, where they cross the bridge.)
      const on = (await pr.apps()).includes(app);
      const reads = await pr.policyIn(domain, "read");
      if (!on || reads !== "anyone") throw new Fault("refused-expected", "members only", { message: `${space.shown(desc)}'s ${app} is for its members (${on ? `read by: ${reads ?? "members"}` : "not public there"}): join it to open it` });
      sp = { ...desc, outside: true };
    }
    // An app the space TURNED OFF: its Home instead (where its apps are added) — only on that certainty, an `app` act
    // turning it off that counted. Never because it looks unused while the acts are still arriving (a member's acts
    // merge in after "settled": the app would be sent away from a space that uses it).
    const app_ = ctx.apps.find(a => a.route === `/${app}`);
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
    // THE READ: the place's items — a list's window, or (whole) every one; your Saved: what you saved of its kinds.
    const read = ctx.total("where.read", () => [], ({ kinds: ks = kinds, sort = "new", window = "all", whole = false, ...options } = {}) => {
      if (saved)
        return ctx
          .require("actions")
          .then(async a => Promise.all((await a.saved()).map(getSaved)))
          // NSFW as every list (`items.visible`).
          .then(list => items.visible(list.filter(it => it && ks.includes(it.kind))));
      if (shared) return ctx.require("share").then(async sh => items.visible(await items.inPlaces.strict({ spaces: await sh.groups() }, ks, { withVotes: false })));
      if (who === "discover") return items.list.strict({ discover: true }, sort, ks, { window, ...options });
      if (whole) return items.inPlaces.strict(sp ? { spaces: [sp] } : { people: [person ?? me] }, ks, { withVotes: false });
      return items.list.strict(sp?.outside ? { outside: sp } : sp ? { board: sp.id } : { by: person ?? me }, sort, ks, { window, ...options });
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
    let mayConfigure = false;
    // Each entry's ROLE, where the side panel puts it (owner 10-15, YouTube's: Home · Subscriptions, then You): `discover`,
    // `feed` (what you follow — `feed: true` where the app's own page IS its feed, as Videos'), `you` (yours, saved,
    // the app's extras unless they say otherwise), `create`.
    const tabs = (extra = [], { yoursOn = true, before = [], create = null, feed = false } = {}) => {
      settingsRead.then(ok => ok && ((mayConfigure = true), draw()));
      const draw = () => {
      const home = who === "mine" || who === "discover";
      // A GUEST: nothing of theirs (Yours, Saved) and nothing to make — Log in instead; once in, the page starts again
      // (as at Log out: nothing made for the guest reaches the session).
      ctx.actions[ctx.route] = !me
        ? [
            ...(home && pub ? [{ label: "Discover", href: `#/discover/${app}`, on: who === "discover", role: "discover" }] : []),
            ...extra.filter(e => e.public).map(e => ({ role: "you", ...e })),
            { label: "Log in", run: async () => (await (await ctx.require("login")).session()) && location.reload(), end: true, role: "create" },
          ]
        : [
            ...(home ? before.map(e => ({ role: "you", ...e })) : []),
            ...(home || sp ? [{ label: yours, href: `#/${app}`, on: who === "mine" && !saved && yoursOn, role: feed && !sp ? "feed" : "you" }] : []),
            ...extra.map(e => ({ role: "you", ...e })),
            ...(mayConfigure ? [{ label: "Settings", href: `${base}/settings`, on: sub === "settings", role: "you" }] : []),
            ...(home && saves ? [{ label: "Saved", href: `#/${app}/saved`, on: saved, role: "you" }] : []),
            ...(home && saves ? [{ label: "Shared", href: `#/${app}/shared`, on: shared, role: "you" }] : []),
            ...(home && pub ? [{ label: "Discover", href: `#/discover/${app}`, on: who === "discover", role: "discover" }] : []),
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
    return { settings, who, saved, shared, outside: !!sp?.outside, others: who === "person" || who === "discover" || saved || !!sp?.outside, discover: who === "discover", guest: !me, space: sp, person, me, sub, base, href: s => (s ? `${base}/${s}` : base), read, tabs };
  }
  // A MENU of an OS page's own (Settings' sections, Mail's folders, the token pages): the ONE door for an app's entries
  // besides `tabs` — each its ROLE (`you`, or `create` for an ending action), told to the header and the side panel.
  // `route`: the page's (default: the one open).
  function menu(entries, route = ctx.route) {
    ctx.actions[route] = (entries ?? []).map(e => ({ role: e.end ? "create" : "you", ...e }));
    dispatchEvent(new CustomEvent("craftworks:actions"));
  }
  return { of, menu, personOf, key: () => `${ctx.space ?? ""}|${ctx.sub ?? ""}`, placeKey: () => `${ctx.space ?? ""}|${personOf() ?? ""}` };
}
