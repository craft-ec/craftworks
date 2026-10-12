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
  const [space, items, roles] = await Promise.all(["space", "items", "roles"].map(n => ctx.require(n)));
  const personOf = () => (ctx.space ? null : (/^u\/(did:[^/]+)/.exec(ctx.sub ?? "")?.[1] ?? null));

  // A SAVED item, read where it is (a space's this person is not in: from outside, by its public description).
  async function getSaved(ref) {
    const outside = await items.outsideOf(ref).catch(() => null);
    if (String(ref).startsWith("space:") && !outside && !(await items.boardOf(ref.slice(6, ref.indexOf("/"))))) return null;
    return items.get(ref, { outside }).catch(() => null);
  }

  async function of({ kind, kinds = [kind], app = ctx.route.replace(/^\//, ""), yours = "Yours", saves = true } = {}) {
    const me = await space.me();
    const discover = ctx.space === "discover";
    const mineSp = ctx.space && !discover ? ((await space.mine()).find(s => s.id === ctx.space) ?? null) : null;
    // A space this person is NOT in (or anyone, for a guest): opened FROM OUTSIDE where the space lets anyone read this
    // app — its public description, its public acts' rules (the app on, its kind's `read` policy "anyone"); else said.
    let sp = mineSp;
    if (ctx.space && !discover && !sp) {
      const desc = await items.publicSpace(ctx.space).catch(() => null);
      const pr = desc ? await roles.ofPublic(desc) : null;
      if (pr) await pr.settled;
      const domain = (await ctx.require("kinds")).policyDomain(kind);
      // (Both refusals are ANSWERS, not failures: `expected` — the loader says them, never as a fault.)
      if (!desc) throw Object.assign(new Error("you are not in that space (left, or not joined yet)"), { expected: true });
      // Why not, said (its public rules: the app not on there, or who reads it).
      const on = pr.apps().includes(app);
      const reads = pr.policyIn(domain, "read");
      if (!on || reads !== "anyone") throw Object.assign(new Error(`${space.shown(desc)}'s ${app} is for its members (${on ? `read by: ${reads ?? "members"}` : "not public there"}): join it to open it`), { expected: true });
      sp = { ...desc, outside: true };
    }
    // An app the space TURNED OFF: its Home instead (where its apps are added) — only on that certainty, an `app` act
    // turning it off that counted. Never because it looks unused while the acts are still arriving (a member's acts
    // merge in after "settled": the app would be sent away from a space that uses it).
    const app_ = ctx.apps.find(a => a.route === `/${app}`);
    if (sp)
      roles
        .of(sp)
        .then(async r => (await r.settled, r))
        .then(r => {
          const last = r.acts("app").filter(a => a.app === app).at(-1);
          if (last && !last.on && !r.apps().includes(app) && ctx.space === sp.id && ctx.route === `/${app}`) location.replace(`#/s/${sp.id}`);
        }, () => {});
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
    // THE READ: the place's items — a list's window, or (whole) every one; your Saved: what you saved of its kinds.
    const read = ({ kinds: ks = kinds, sort = "new", window = "all", whole = false, ...options } = {}) => {
      if (saved)
        return ctx
          .require("actions")
          .then(a => Promise.all(a.saved().map(getSaved)))
          // NSFW as every list (`items.visible`).
          .then(list => items.visible(list.filter(it => it && ks.includes(it.kind))));
      if (who === "discover") return items.list({ discover: true }, sort, ks, { window, ...options });
      if (whole) return items.inPlaces(sp ? { spaces: [sp] } : { people: [person ?? me] }, ks, { withVotes: false });
      return items.list(sp?.outside ? { outside: sp } : sp ? { board: sp.id } : { by: person ?? me }, sort, ks, { window, ...options });
    };
    // THE TABS, in the same order in every app: in your space, the app's own page ("Your …"), then the app's extras,
    // then Discover (an app with a public side); elsewhere the extras alone.
    const tabs = (extra = [], { yoursOn = true, before = [], create = null } = {}) => {
      const home = who === "mine" || who === "discover";
      // A GUEST: nothing of theirs (Yours, Saved) and nothing to make — Log in instead; once in, the page starts again
      // (as at Log out: nothing made for the guest reaches the session).
      ctx.actions[ctx.route] = !me
        ? [
            ...(home && pub ? [{ label: "Discover", href: `#/discover/${app}`, on: who === "discover" }] : []),
            ...extra.filter(e => e.public),
            { label: "Log in", run: async () => (await (await ctx.require("login")).session()) && location.reload(), end: true },
          ]
        : [
            ...(home ? before : []),
            ...(home ? [{ label: yours, href: `#/${app}`, on: who === "mine" && !saved && yoursOn }] : []),
            ...extra,
            ...(home && saves ? [{ label: "Saved", href: `#/${app}/saved`, on: saved }] : []),
            ...(home && pub ? [{ label: "Discover", href: `#/discover/${app}`, on: who === "discover" }] : []),
            ...(create && who !== "person" && !sp?.outside ? [{ ...create, label: `＋ ${create.label}`, end: true }] : []),
          ];
      dispatchEvent(new CustomEvent("craftworks:actions"));
    };
    return { who, saved, outside: !!sp?.outside, others: who === "person" || who === "discover" || saved || !!sp?.outside, discover: who === "discover", guest: !me, space: sp, person, me, sub, base, href: s => (s ? `${base}/${s}` : base), read, tabs };
  }
  return { of, personOf, key: () => `${ctx.space ?? ""}|${ctx.sub ?? ""}`, placeKey: () => `${ctx.space ?? ""}|${personOf() ?? ""}` };
}
