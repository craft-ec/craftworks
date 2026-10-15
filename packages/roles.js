// ROLES, a capability: who holds what authority IN A SPACE (a server, a direct conversation — any space; a channel is
// its parent's). Roles are per space: holding one never crosses spaces. Built in, for now:
//   owner   everything; first the one who made the space (its id proves it: `space.owner`), then whom they hand it to
//   admin   invite (by id or code), make and delete channels, hide others' items, remove members, add and remove apps
//   member  post, invite (by id or code)
// Authority changes by ACTS in the space's table `acts` (each member's own feed). An act counts only if its signer's
// role allowed it at that point: every reader replays the acts in one order (when made, then id) from the owner — the
// same answer on every node. The acts are a DAG (`ordering`'s `dag`): each names the acts its writer had seen and is
// timed after them, so the order is causal whoever's clock is behind, and a reader knows when it lacks one. The signer of an act, or of any row, is the WRITER of the feed it is in (a feed's rows
// carry their writer, which nobody else can write as); a writer is a node, and the space's group says whose.
//
//   const r = await roles.of(sp)
//   r.role(did)              // "owner" | "admin" | "member" | null (not a member, or removed)
//   r.can(did, "moderate")   // may that person do it: post | invite | channels | moderate | remove | apps | roles |
//                            // grant — by their base role and every COMPOSED role they hold
//   r.roles()                // the roles composed here: [{ id, name, perms }] (a `role` act: { role, name, perms };
//                            // { role, on: false } deletes it) — by who may `roles`, carrying only what they may
//   r.held(did)              // the composed roles a person holds (an `assign` act: { did, role, on })
//   r.passes(who, did)       // does that person pass a rule's who ("role:<id>": its holders, the owner and admins)
//   r.author(row)            // the DID that wrote a row (from its version id), or null for a node not known here
//   r.members()              // [{ did, role }], this space's people as its group has them now
//   r.left                   // this node was removed from the space
//   r.acts("hide")           // the acts of a kind that COUNTED, in order (`r.acts()`: all of them — the space's log)
//   r.missing()              // acts other acts name that this reader does not have (its view of the space is short)
//   r.owner                  // who owns it now (a `transfer` act hands it on)
//   r.invites()              // the invite codes in force: [{ code, by, at, expires, uses, admitted }]
//   r.enabled()              // the CAPABILITIES its owner enabled (ARCHITECTURE §1, "The desktop"): its domains
//                            // (text, video, chat, wiki, vote …) and features (comments, reactions, mail) — the one
//                            // list the space keeps; `r.enable(cap, on)` changes it (an `app` act naming the capability)
//   r.apps()                 // the APPS that show it: each whose manifest `needs` it enables (none: Home and settings)
//   ACCESS, like row-level security: POLICIES `{ path, action, who }` (a `policy` act, by who may `apps`; `read` by
//   the owner only), INHERITED along the path — an item (`board/p/<id>`), a container (`chat/<channel>`), an app
//   (`board`), the space (""): the most specific wins, else its parent's, else the default. Actions: read, post,
//   comment, vote, edit, join, invite. Who: anyone | members | admins | owner | nobody. Defaults: every action `members`
//   (join `members`: by an invite from one). Time-aware: an item is judged by the policy in force when it was made.
//   r.policy(path, action, at?)   // the effective `who`
//   r.allows(action, did, path, at?)   // may that person do it there (then)
//   r.config(app, key, dflt) // an APP's content setting (a `config` act; e.g. Board's rules) — not access
//                            // Chat `post`, Board `post` and `rules`, Notes `edit`
//   r.allows(app, did, key, at) // may that person do it in that app here — as the setting was at `at` (an item's time:
//                            // a change never hides what came before it): "everyone" (a member) or "admins"
//   await r.act({ act: "grant", did, role })   // an act, as this person (refused here if it would not count)
//   await r.grant(did, role)  r.onChange(fn)  r.settled
//   r.isPublic()             // an app of the space reads in public (`config` read: "public"): its acts are public too
//   await r.publish()        // (the owner) the acts so far and the roster (a `member` act per member) into the PUBLIC
//                            // acts (`pub-acts`, in the clear): what a reader outside needs to know whose posts count
//   await roles.ofPublic(desc)  // a space seen from OUTSIDE (`desc`: { id, name, governance }): its public acts only —
//                            // its writers found from its owner (the id proves them) and the roster, each DID's devices
//                            // from its card; the same `r` to read by (role, author, config, allows, members, acts)
export async function start(ctx) {
  const [space, storage, keys, node, directory, K, S, ordering] = await Promise.all(["space", "storage", "keys", "node", "directory", "kinds", "signals", "ordering"].map(n => ctx.require(n)));

  // THE REPLAY and its rules are the core's (`Governance`, the one implementation: the identity delegate reads a
  // space by it too). What is gathered here: the acts' rows, the group's members, each member's devices.
  const G = node.glue.Governance;
  const ACTIONS = G.actions();
  // WHICH POLICY GOVERNS a domain (THE ONE resolver, for every app and every action): its domain's own (`kinds`:
  // text, video, audio, note, caption, file …), else the space's. `DOMAINS`: every domain a policy can name.
  const DOMAINS = ["text", "video", "audio", "image", "book", "caption", "note", "file", "chat", "form", "wiki", "vote", "bundle"];
  // A space's CAPABILITIES: what its owner enables — its domains and the features on them. Apps compose from these:
  // an app shows the space when the space enables what the app `needs` (its manifest entry), and draws only what is on.
  const FEATURES = ["comments", "reactions", "mail", "contact"];
  const CAPABILITIES = [...DOMAINS, ...FEATURES];
  // BASE (owner 10-13): what every space has from its first member and never turns off — its people (contact: the
  // members), chat and mail (the basic ways to reach it; Form's answers go by its mail).
  const BASE = ["chat", "mail", "contact"];
  // The apps a list of enabled names opens: each app whose `needs` are all on (and, until the reset, an app turned on
  // by its own name by an older act).
  // An OLDER act that turned on an APP by its name (before capabilities): what that app needs counts as enabled.
  const legacyCaps = on => (ctx.apps ?? []).filter(a => on.includes(a.route.slice(1)) && !CAPABILITIES.includes(a.route.slice(1))).flatMap(a => a.needs ?? []);
  const appsFor = on => {
    const caps = new Set(on);
    return (ctx.apps ?? [])
      .filter(a => ((a.needs ?? []).length && a.needs.every(n => caps.has(n))) || caps.has(a.route.slice(1)))
      .map(a => a.route.slice(1))
      .sort();
  };
  // A member's credential (hex): `CWMB ‖ did ‖ signer ‖ writer ‖ MLS key ‖ signature` (the identity's format; MLS
  // checked the signature when it admitted it).
  const didOf = h => new Uint8Array(h.match(/../g).slice(4, 36).map(x => parseInt(x, 16)));
  const signerOf = h => h.slice(72, 136);
  const newId = () => [...crypto.getRandomValues(new Uint8Array(8))].map(x => x.toString(16).padStart(2, "0")).join("");

  const spaces = new Map();
  const outside = new Map();
  // (A view that failed to open is not kept: R4.)
  const ofPublic = desc => ctx.cache(outside, desc.id, () => open(desc, { outside: true }));
  function of(sp) {
    // A space READ FROM OUTSIDE (`where`'s, `outside: true`: its public description): its public acts only.
    if (sp.outside) return ofPublic(sp);
    sp = sp.parent ?? sp;
    return ctx.cache(spaces, sp.id, () => open(sp));
  }

  async function open(sp, { outside: out = false } = {}) {
    // THE ACTS: the sealed ones and the PUBLIC ones (the same act in both — a copy — counts once: by its key). From
    // outside, only the public ones, read from the writers found so far.
    const pubName = space.tableOf(sp, "pub-acts");
    const index = await ctx.require("index");
    const [first, me, sealedActs, pubActs] = await Promise.all([
      space.owner.strict(sp),
      space.account.strict(),
      out ? null : storage.table(space.tableOf(sp, "acts"), sp),
      out ? storage.readOnly(pubName, []) : storage.table(pubName, sp),
    ]);
    const t = {
      rows: () => {
        const byKey = new Map();
        for (const r of [...(sealedActs?.rows() ?? []), ...pubActs.rows()]) if (!byKey.has(r.key)) byKey.set(r.key, r);
        return [...byKey.values()];
      },
      onChange: f => (sealedActs?.onChange(f), pubActs.onChange(f)),
      settled: Promise.all([sealedActs?.settled, pubActs.settled]),
      // Read or not (R5): the narrowest of its tables' states — "unknown" while any is, never "none" for unread.
      get state() {
        const ss = [sealedActs, pubActs].filter(Boolean).map(x => x.state ?? { s: "present" });
        return ss.find(x => x.s === "unknown") ?? ss.find(x => x.s === "unreadable") ?? { s: "present" };
      },
      decided: async () => (await Promise.all([sealedActs?.decided?.(), pubActs.decided?.()]), t.state),
    };
    const g = out ? null : keys.group(sp);
    // THE ACTS' DAG: what a new act names and when it is timed; which named acts are not here.
    const dag = await ordering.open({ type: "dag", source: () => counted });
    // MEMBERS: each member of the group (one per DID), where its credential is really that DID's: signed by the data
    // key its key log names (a credential names any DID it likes; the key log says whose it is). WRITERS: each of their
    // devices → the DID. Members who left stay known (what they wrote is still theirs): learned from the group as it
    // was when seen, and from the removals that name them.
    const writers = new Map();
    let group = [];
    let left = false;
    let lastSaid = null;
    // The group READ at least once (owner 10-08: a space showed "0 members" while its keys were still arriving).
    let groupRead = false;
    const learn = async st => {
      if (st) groupRead = true;
      left = !!st?.removed;
      const all = (st?.members ?? []).filter(m => m.cred).map(m => ({ key: m.key, index: m.index, did: node.glue.did_of(didOf(m.cred)), signer: signerOf(m.cred) }));
      const real = await Promise.all(all.map(async m => (await directory.dataKey(m.did)) === m.signer));
      for (const [i, m] of all.entries()) if (!real[i]) ctx.log("roles", { what: `a member claims ${m.did.slice(12, 20)}… with a key that is not its: left out` });
      group = all.filter((_, i) => real[i]);
      // SAID when it changes (owner 10-07: a member's space showed 0 members): what the group gave, and what was kept.
      const said = `${st ? `${st.members?.length ?? 0} member(s) at epoch ${st.epoch ?? "?"}${st.removed ? " (this node removed)" : ""}` : "not ready"}, ${all.length} with a credential, ${group.length} kept`;
      if (said !== lastSaid) (lastSaid = said), ctx.log("roles", { what: `${sp.name ?? sp.id.slice(0, 8)}: its group — ${said}` });
      // Each member's DEVICES write on its behalf (their feeds): device key → the DID (the DID's card, checked
      // against its key log).
      const sets = await Promise.all(group.map(m => directory.devices.strict(m.did)));
      group.forEach((m, i) => sets[i].forEach(k => writers.set(k, m.did)));
    };
    if (!out) await learn(await g.ready());
    const author = row => (row?.id ? (writers.get(row.id.slice(0, 64)) ?? null) : null);

    // THE REPLAY: the acts in order, each kept only if its signer could (the core's `Governance`).
    let gv = null;
    let counted = [];
    let owner = first;
    let roster = new Set(first ? [first] : []);
    let bans = new Set();
    let gone = new Set();
    function replay() {
      const rows = t.rows().map(r => [r.key, r.value, r.id ? r.id.slice(0, 64) : null]);
      const next = G.replay(JSON.stringify(rows), JSON.stringify(Object.fromEntries(writers)), first ?? undefined, Date.now());
      // A removed member's nodes, named by the removal: their rows stay theirs.
      for (const [n, d] of JSON.parse(next.learned())) if (!writers.has(n)) writers.set(n, d);
      gv?.free();
      gv = next;
      counted = JSON.parse(gv.counted());
      owner = gv.owner() ?? null;
      roster = new Set(gv.roster());
      bans = new Set(gv.bans());
      gone = new Set(gv.gone());
      // SHORT: an act names one this reader has not got (a feed not read yet, or lost) — said when it changes. From
      // outside, public acts may name sealed ones: not counted.
      if (!out) {
        const miss = dag.missing(new Set(rows.map(([k]) => k)));
        const causal = rows.filter(([, v]) => /"deps":\[/.test(v ?? "")).length;
        const said = `acts ${rows.length} (${causal} naming what they saw), ${counted.length} counted; ${miss.length ? `${miss.length} named by others NOT HERE yet (${miss.slice(0, 3).join(", ")})` : "none missing"}`;
        if (said !== missSaid) ctx.log("roles", { what: `${sp.name ?? sp.id.slice(0, 8)}: ${said}` });
        missSaid = said;
      }
    }
    let missSaid = null;
    replay();
    // FROM OUTSIDE: the members are the roster; each one's devices are writers, whose public acts are read too — until
    // no new member turns up.
    const known = new Set();
    // ITS PUBLIC ACTS' WRITERS (`index`: the space's public acts bag — whoever wrote a public act, the owner and admins,
    // listed once): read from outside instead of every member's public acts.
    const ACTS = `acts ${sp.id}`;
    const actWriters = out ? await ctx.optional("roles: index.pointers", index.pointers(ACTS).then(ps => [...new Set(ps.map(p => p?.w).filter(w => typeof w === "string"))]), () => []) : [];
    async function widen() {
      if (!out) return;
      for (;;) {
        const fresh = [...roster].filter(d => !known.has(d));
        if (!fresh.length) return;
        fresh.forEach(d => known.add(d));
        const sets = await Promise.all(fresh.map(d => directory.devices(d)));
        fresh.forEach((d, i) => sets[i].forEach(k => writers.set(k, d)));
        await pubActs.add(actWriters);
        replay();
      }
    }
    if (out) {
      await widen();
      group = [...roster].map(did => ({ did }));
    }

    const changed = [];
    // A MEMBER's group learned AGAIN (owner 10-07: a fresh load opened a space before its group had caught up — 0
    // members, no apps, no channels, until a reload, and a reload raced the same way): on every change of its acts and
    // every key handed over, and — while it still has no members — every few seconds (3 s, doubling, to a minute, for
    // ten minutes). A changed group: the acts replayed (their writers known now) and everyone told.
    const sig = () => group.map(m => m.key).sort().join(",");
    let relearning = null;
    const relearn = () =>
      (relearning ??= (async () => {
        try {
          if (out) return;
          const before = sig();
          await learn(await g.ready());
          if (sig() !== before) {
            replay();
            for (const f of changed) f();
          }
        } finally {
          relearning = null;
        }
      })());
    if (!out) {
      addEventListener("craftworks:keys", () => ctx.background("roles: relearn", relearn()));
      let wait = 3000;
      const until = Date.now() + 600000;
      const again = () => !group.length && Date.now() < until && setTimeout(() => relearn().finally(again), (wait = Math.min(wait * 2, 60000)));
      if (!group.length) setTimeout(() => relearn().finally(again), wait);
    }
    t.onChange(() => {
      replay();
      if (out) widen().then(() => ((group = [...roster].map(did => ({ did }))), changed.forEach(f => f())));
      for (const f of changed) f();
      if (!out) ctx.background("roles: relearn", relearn());
    });
    // PUBLIC: something of the space is anyone's — an app read by anyone, or joining open to anyone (an open space is
    // listed in Discover: whoever looks must see who is in it and how to join).
    const isPublic = () => gv.is_public();
    const role = did => {
      if (!did || (left && did === me?.id)) return null;
      return gv.role(did, group.some(m => m.did === did)) ?? null;
    };
    // What a person may: inviting is the space's policy (`invite`); everything else their roles' — their base role
    // and every role composed in the space they hold (`gov`).
    const inGroup = did => group.some(m => m.did === did) && !(left && did === me?.id);
    const passes = (who, did) => who === "anyone" || (!!role(did) && gv.passes_did(who, did, inGroup(did)));
    const can = (did, what) => (!role(did) ? false : what === "invite" ? passes(gv.effective("", "invite", Infinity), did) : gv.may(did, inGroup(did), what));

    // This device listed as a writer of the space's public acts (once a page): after it writes one, and on opening a
    // space whose public acts it wrote before the bag.
    let actListed = false;
    async function listActWriter() {
      if (out || actListed || !sp.self) return;
      actListed = true;
      const ws = await index.pointers(ACTS);
      if (!ws.some(p => p?.w === sp.self)) await ctx.optional("roles: index.point", index.point(ACTS, { w: sp.self }), () => actListed = false);
    }
    // The policy set at exactly a path.
    const policyAt = (path, action, at) => gv.policy_at(path, action, at) || null;
    // Set at a path or any path above it (a channel, its app, the space).
    const setAlong = (path, action, at) => {
      for (let p = path; ; p = p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : "") {
        const w = policyAt(p, action, at);
        if (w || p === "") return w;
      }
    };
    // THE DEFAULT where nothing is set: what anyone may READ, anyone may comment on and vote on (public participation:
    // outsiders' part kept in their own profiles, `items`); everything else as the core's default (members).
    // (`signals`: the interactions open where anyone reads — comment, vote.)
    const PUBLIC_WRITES = new Set(S.interactions().filter(a => a.openWhereRead).map(a => a.action));
    const fallback = (read, action, at) => (PUBLIC_WRITES.has(action) && read === "anyone" ? "anyone" : gv.effective("", action, at));
    // A SIGNAL's own action with nothing set: its parent's (`signals`: save, share → read; react, tag → vote).
    const policyOf = (path, action, at) =>
      setAlong(path, action, at) || (S.parentOf(action) ? policyOf(path, S.parentOf(action), at) : fallback(action === "read" ? null : policyOf(path, "read", at), action, at));
    const policyInOf = (domain, action, at) =>
      policyAt(domain, action, at) ||
      policyAt("", action, at) ||
      (S.parentOf(action) ? policyInOf(domain, S.parentOf(action), at) : fallback(action === "read" ? null : policyInOf(domain, "read", at), action, at));
    const r = {
      space: sp,
      get owner() {
        return owner;
      },
      role,
      can,
      author,
      acts: kind => (kind ? counted.filter(a => a.act === kind) : [...counted]),
      missing: () => (out ? [] : dag.missing(new Set(t.rows().map(x => x.key)))),
      invites: () => JSON.parse(gv.invites(Date.now())),
      enabled: () => [...new Set([...BASE, ...gv.apps().filter(x => CAPABILITIES.includes(x)), ...legacyCaps(gv.apps())])],
      base: () => BASE,
      enable: (cap, on) => {
        if (!CAPABILITIES.includes(cap)) throw new Error(`"${cap}" is not a capability a space enables`);
        if (BASE.includes(cap)) throw new Error(`${cap} is always on in a space`);
        // Off: also every OLDER app act that turned it on by an app's name.
        const older = on ? [] : (ctx.apps ?? []).filter(a => gv.apps().includes(a.route.slice(1)) && !CAPABILITIES.includes(a.route.slice(1)) && (a.needs ?? []).includes(cap)).map(a => a.route.slice(1));
        return (async () => {
          for (const app of older) await r.act({ act: "app", app, on: false });
          return r.act({ act: "app", app: cap, on: !!on });
        })();
      },
      capabilities: () => CAPABILITIES,
      apps: () => appsFor([...BASE, ...gv.apps()]),
      // LISTS of named people (`{ id: { by, people } }`): audiences by name (`list:<id>`), any member's to make.
      lists: () => {
        try {
          return JSON.parse(gv.lists());
        } catch {
          return {};
        }
      },
      config: (app, key, dflt = null) => {
        const c = gv.config(app, key);
        return (c === undefined ? null : JSON.parse(c)) ?? dflt;
      },
      policy: (path, action, at = Infinity) => policyOf(path, action, at),
      // The policies set at exactly this path (not inherited): { action: who }.
      policiesAt: path => Object.fromEntries(ACTIONS.map(x => [x, policyAt(path, x, Infinity)]).filter(([, w]) => w)),
      allows: (action, did, path = "", at = Infinity) => passes(policyOf(path, action, at), did),
      // Does a person pass a rule's `who` (a composed role: by holding it).
      passes,
      // THE ROLES composed in this space ([{ id, name, perms }]) and whose they are.
      roles: () => JSON.parse(gv.roles_defined()),
      held: did => JSON.parse(gv.roles_held())[did] ?? [],
      // A DOMAIN's policy for an action (`kinds.policyDomain(kind)`: what an item is decides, not the app showing it).
      policyIn: (domain, action, at = Infinity) => policyInOf(domain, action, at),
      allowsIn: (action, did, domain, at = Infinity) => passes(policyInOf(domain, action, at), did),
      domains: () => DOMAINS,
      banned: did => bans.has(did),
      bannedList: () => [...bans],
      // Out by the acts — removed, banned, left — and not added back: their nodes leave the group (`moderation`).
      goneList: () => [...gone],
      // This node was removed from the space's group: it reads nothing newer.
      get left() {
        return left;
      },
      // Whether its members are KNOWN yet (its group read once — or, from outside, its roster).
      get membersKnown() {
        return groupRead || !!out;
      },
      // Its member count as every view says it: "Loading members…" until the group is read, never a false 0.
      membersText: () => {
        if (!(groupRead || out)) return "Loading members…";
        const n = [...new Set(group.map(m => m.did))].filter(did => role(did)).length;
        return `${n} member${n === 1 ? "" : "s"}`;
      },
      // The people of the space now: its group's accounts, with their roles.
      members: () => [...new Set(group.map(m => m.did))].map(did => ({ did, role: role(did) })).filter(m => m.role),
      // A person's member in the group (to remove them): one per DID.
      nodesOf: did => group.filter(m => m.did === did),
      // Brought current with the group (a member added or removed).
      refresh: async () => {
        if (out) return widen();
        await learn(await g.ready());
        replay();
      },
      isPublic,
      // WHO THE ACTS SAY IS IN (added, admitted or listed, not removed, banned or left since) — the group carries it out.
      roster: () => [...roster],
      // All writers known here (their devices' keys): whose public tails a reader outside reads.
      writerKeys: () => [...writers.keys()],
      // PUBLISH (the owner): the counted acts so far, and a `member` act for each member, into the public acts.
      async publish() {
        if (out || !me || role(me.id) !== "owner") throw new Error("only the owner makes the space's acts public");
        const have = new Set(pubActs.rows().map(x => x.key));
        const ids = new Set(counted.map(a => a.id));
        for (const row of sealedActs.rows()) if (ids.has(row.key) && !have.has(row.key)) await pubActs.put(row.key, row.value);
        const listed = new Set(counted.filter(a => ["member", "added", "admitted"].includes(a.act)).map(a => a.did));
        for (const m of r.members()) if (m.did !== owner && !listed.has(m.did)) await pubActs.put(newId(), JSON.stringify(dag.stamp({ act: "member", did: m.did })));
        await listActWriter();
      },
      async act(a) {
        if (!me) throw space.loginNeeded();
        if (out) throw new Error("not a member of this space");
        const need = { role: "roles", assign: "roles", grant: "grant", remove: "remove", ban: "remove", unban: "remove", hide: "moderate", app: "apps", config: "apps", policy: "apps", transfer: "grant", invite: "invite", "revoke-invite": "invite", admitted: "invite", added: "invite", member: "invite" }[a.act];
        // Leaving is any member's own act (the owner hands the space on first).
        const leaving = a.act === "leave" && role(me.id) && role(me.id) !== "owner";
        // A list of named people: any member's own (only its maker changes it — the acts' check).
        const listing = a.act === "list" && !!role(me.id);
        // A LAPSE (a level group): any member's, as the replay rules it (`gov`: level groups only, never the owner or an
        // admin) — owner 10-07: this check had no entry for it, so no page could write one.
        const lapsing = a.act === "lapsed" && !!role(me.id) && /^level:(pro|vip)$/.test(policyOf("", "join", Infinity) ?? "");
        if (!leaving && !listing && !lapsing && (!need || !can(me.id, need))) throw new Error(`as ${role(me.id) ?? "nobody here"}, you cannot ${a.act} in this space`);
        // A public space's acts are public (readers outside must know them); a private one's sealed. An OPEN space's
        // admissions too (10-11: sealed, the asker — reading only the public acts — never learned it was let in, so a
        // welcome that failed was never asked for again: "Requested" for days while listed as a member).
        const admits = ["admitted", "added", "member"].includes(a.act) && policyOf("", "join", Infinity) === "anyone";
        const toPublic = isPublic() || admits || (a.act === "policy" && (a.action === "read" || a.action === "join") && a.who === "anyone");
        // Its time: after every act seen (`dag`: now, or just past the latest) — or when it happened (upkeep let someone
        // in while no page ran: the act says when); and the acts it builds on.
        await (toPublic ? pubActs : sealedActs).put(newId(), JSON.stringify(dag.stamp(a)));
        if (toPublic) await listActWriter();
      },
      grant: (did, to) => r.act({ act: "grant", did, role: to }),
      onChange: f => changed.push(f),
      settled: t.settled,
      // Its acts read or not (R5): what a decision on "no acts" waits for.
      get state() {
        return t.state;
      },
      decided: () => t.decided(),
    };
    // ITS RETENTION handed to `storage` (its tables' history kept that long): the `space`/`history` setting, in days
    // ("" — the plan's; "0": off).
    const retain = () => {
      const v = r.config("space", "history", "");
      storage.setRetention(sp.id, v === "" || v == null ? null : Number(v));
    };
    ctx.background("roles: t.settled.then", t.settled.then(retain));
    changed.push(retain);
    return r;
  }

  // MAY WRITE — THE ONE CHECK of who may comment, vote or post on an item, in a shared space and a personal one alike,
  // made by EVERY READER (no server checks a write: what a reader does not count does not count). Its RULE: the item's
  // own (`meta.write[action]`), else its space's policy for the item's domain (`policyIn`), else — a personal item —
  // anyone. The rule's GROUP: anyone · the item's author · a role in the space (members, admins, owner: the space's
  // credentials) · the author's FRIENDS or FOLLOWERS (the writer's credential: an unlisted table of the author's
  // account at the token it cites, naming the writer — `circles` issues them). True, false, or null: not checked yet
  // (a credential being read: `onChecked` fires when it is).
  const CIRCLES_OF = { followers: ["followers", "friends"], friends: ["friends"] };
  // A PERSONAL SPACE, governed as a shared one is: its POLICY ACTS replayed by the core's `Governance` (the one
  // replay — inherited along the path, TIME-AWARE: an item is judged by the policy in force when it was made). Its acts
  // are rows of its owner's CARD (`act:<id>` → { at, act: "policy", path, action, who }) — public, the person's own
  // record (only their devices write it: every row is theirs), so every reader applies them; written on their
  // Settings (`app-settings`). Who: anyone · followers · friends · author (only they) — checked by credential
  // (`mayWrite`). Its default: anyone. Rows of the first shape (`policy:<path>|<action>` → who) count as acts made
  // before anything. Not read yet: `loaded()` false.
  const personals = new Map();
  function personal(did) {
    if (!personals.has(did)) {
      let t = null;
      let loaded = false;
      let gv = null;
      const replay = () => {
        const rows = (t?.rows() ?? []).flatMap(x => {
          if (!x.value) return [];
          if (x.key.startsWith("act:")) return [[x.key, x.value, "card"]];
          const m = x.key.match(/^policy:(.*)\|([a-z]+)$/);
          return m ? [[x.key, JSON.stringify({ at: 0, act: "policy", path: m[1], action: m[2], who: x.value }), "card"]] : [];
        });
        const next = G.replay(JSON.stringify(rows), JSON.stringify({ card: did }), did, Date.now());
        gv?.free();
        gv = next;
      };
      const ready = ctx.background("roles: directory.publicOf", directory
        .publicOf.strict(did, "card")
        .then(async x => ((t = x), x && (await x.answer(), replay(), x.onChange(() => (replay(), checked.forEach(f => f())))))))
        .finally(() => ((loaded = true), checked.forEach(f => f())));
      const policyAt = (path, action, at) => gv?.policy_at(path, action, at) || null;
      // Walked up the path (`board/p/<id>` → `board` → the space), else anyone.
      const along = (path, action, at) => {
        for (let p = path; ; p = p.includes("/") ? p.slice(0, p.lastIndexOf("/")) : "") {
          const w = policyAt(p, action, at);
          if (w || p === "") return w ?? "anyone";
        }
      };
      personals.set(did, {
        personal: true,
        ready,
        loaded: () => loaded,
        policiesAt: path => Object.fromEntries(ACTIONS.map(a => [a, policyAt(path, a, Infinity)]).filter(([, w]) => w)),
        policy: (path, action, at = Infinity) => along(path, action, at),
        policyIn: (domain, action, at = Infinity) => along(domain, action, at),
        role: d => (d === did ? "owner" : null),
        can: d => d === did,
        config: (_a, _k, dflt = null) => dflt,
        // (The personal space is the person's: every capability on.)
        enabled: () => CAPABILITIES,
        capabilities: () => CAPABILITIES,
        apps: () => ["board", "video", "audio", "image", "book"],
        act: async ({ act, path, action, who }) => {
          if (act !== "policy") throw new Error("a personal space keeps policies only");
          await ready;
          if (!t) throw new Error("your card is not readable here");
          await t.put(`act:${newId()}`, JSON.stringify({ at: Date.now(), act, path, action, who }));
          replay();
        },
      });
    }
    return personals.get(did);
  }
  // An item's own rule for the action — a signal's own, else (none set on it) its parent's own — else its place's.
  const ruleOf = (item, action, r, at) =>
    item?.meta?.write?.[action] ?? (S.parentOf(action) && S.parentOf(action) !== "read" ? item?.meta?.write?.[S.parentOf(action)] : undefined) ?? (r ?? personal(item.by)).policyIn(K.policyDomain(item.kind), action, at);
  const credChecks = new Map(); // `${author}|${token}` → { v: row | null | undefined }
  const checked = [];
  function credFor(author, token) {
    const k = `${author}|${token}`;
    if (!credChecks.has(k)) {
      const c = { v: undefined };
      credChecks.set(k, c);
      ctx.optional("roles: directory.credTable", directory
        .credTable(author, token)
        .then(async t => (t ? (await t.answer(), JSON.parse(t.rows().find(x => x.key === "for")?.value ?? "null")) : null)), () => null)
        .then(v => ((c.v = v), checked.forEach(f => f())));
    }
    return credChecks.get(k).v;
  }
  function mayWrite({ action, item, writer, cred = null, r = null, at = Infinity }) {
    if (writer === item.by) return true;
    // A personal item's author's policy not read yet: not checked yet (`onChecked` fires once it is).
    if (!r && !item?.meta?.write?.[action] && item?.by && !personal(item.by).loaded()) return null;
    const judge = rule => {
      if (rule === "anyone") return true;
      if (rule === "author" || rule === "nobody") return false;
      if (CIRCLES_OF[rule]) {
        if (!cred || !/^[0-9a-f]{32}$/.test(cred)) return false;
        const v = credFor(item.by, cred);
        return v === undefined ? null : !!v && v.did === writer && CIRCLES_OF[rule].includes(v.circle);
      }
      return r ? r.passes ? r.passes(rule, writer) : G.passes(rule, r.role(writer)) : false;
    };
    const v = judge(ruleOf(item, action, r, at));
    // A SPACE item's own rule is WITHIN its space's (the ceiling): its writer passes both — an item narrows who may,
    // never widens it (a public post with members-only comments; never anyone's comments where the space keeps them).
    const own = item?.meta?.write?.[action] ?? (S.parentOf(action) && S.parentOf(action) !== "read" ? item?.meta?.write?.[S.parentOf(action)] : undefined);
    if (v !== true || own === undefined || !r?.policyIn || r.personal) return v;
    return judge(r.policyIn(K.policyDomain(item.kind), action, at));
  }
  // The credential a writer cites on an item whose rule is its author's friends or followers (null: none needed / held).
  async function credToCite(item, action, r = null) {
    if (!r && item?.by) await personal(item.by).ready;
    const rule = ruleOf(item, action, r, Infinity);
    if (!CIRCLES_OF[rule] || item.by === (await space.me.strict())) return null;
    const people = await (await ctx.require("edge")).people();
    for (const w of CIRCLES_OF[rule]) {
      const token = people.about(`credin-${w}`, item.by)?.token;
      if (token) return token;
    }
    throw new Error(`only ${directory.shown(item.by)}'s ${rule} may ${action === "follow" ? "follow them" : `${action} here`}`);
  }
  // A PERSON, as what is FOLLOWED: their card's rule (`policy:profile|follow` — anyone · friends · nobody), checked as
  // a comment on their post is (`mayWrite`, its credential `credToCite`).
  async function followable(did) {
    const p = personal(did);
    await p.ready;
    return { kind: "profile", by: did, meta: { write: { follow: p.policy("profile", "follow") } } };
  }

  // `perms`: what a composed role may carry.
  // A WRITER's RIGHT DECIDED WHOLE (async — one call, as an app's frame asks it): its place's roles read first (a
  // space's, from outside as its public acts say), an author's personal rule waited for, a ban, then `mayWrite`.
  // true | false | null (not decidable yet).
  // (`sp`: a space this person is in; `place`: one read from outside, by its public description.)
  async function allows({ action, item, writer, cred = null, place = null, sp = null }) {
    const r = sp ? await of(sp) : place ? await ofPublic(place) : null;
    await r?.settled;
    if (!r && item?.by) await personal(item.by).ready;
    if (r?.banned?.(writer)) return false;
    return mayWrite({ action, item, writer, cred, r });
  }
  // A person's PERSONAL rule for a domain's action, once read: { who, loaded } (what a refusal's report says).
  async function personalPolicy(did, domain, action) {
    const p = personal(did);
    await p.ready;
    return { who: p.policyIn(domain, action), loaded: p.loaded() };
  }
  return { allows, personalPolicy, of: ctx.total("roles.of", () => null, of), ofPublic: ctx.total("roles.ofPublic", () => null, ofPublic), personal, mayWrite, credToCite, followable, onChecked: f => checked.push(f), can: (role, what) => G.can_role(role, what), names: ["owner", "admin", "member"], perms: G.perms() };
}
