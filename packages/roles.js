// ROLES, a capability: who holds what authority IN A SPACE (a server, a direct conversation — any space; a channel is
// its parent's). Roles are per space: holding one never crosses spaces. Built in, for now:
//   owner   everything; first the one who made the space (its id proves it: `space.owner`), then whom they hand it to
//   admin   invite (by id or code), make and delete channels, hide others' items, remove members, add and remove apps
//   member  post, invite (by id or code)
// Authority changes by ACTS in the space's table `acts` (each member's own feed). An act counts only if its signer's
// role allowed it at that point: every reader replays the acts in one order (when made, then id) from the owner — the
// same answer on every node. The signer of an act, or of any row, is the WRITER of the feed it is in (a feed's rows
// carry their writer, which nobody else can write as); a writer is a node, and the space's group says whose.
//
//   const r = await roles.of(sp)
//   r.role(did)              // "owner" | "admin" | "member" | null (not a member, or removed)
//   r.can(did, "moderate")   // may that person do it: post | invite | channels | moderate | remove | grant
//   r.author(row)            // the DID that wrote a row (from its version id), or null for a node not known here
//   r.members()              // [{ did, role }], this space's people as its group has them now
//   r.left                   // this node was removed from the space
//   r.acts("hide")           // the acts of a kind that COUNTED, in order (`r.acts()`: all of them — the space's log)
//   r.owner                  // who owns it now (a `transfer` act hands it on)
//   r.invites()              // the invite codes in force: [{ code, by, at, expires, uses, admitted }]
//   r.apps()                 // the APPS the space uses (chat, board, notes): an `app` act ({ app, on }) adds or
//                            // removes one; a new space has none (its Home and settings only)
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
  const [space, storage, keys, node, directory, K] = await Promise.all(["space", "storage", "keys", "node", "directory", "kinds"].map(n => ctx.require(n)));

  // THE REPLAY and its rules are the core's (`Governance`, the one implementation: the identity delegate reads a
  // space by it too). What is gathered here: the acts' rows, the group's members, each member's devices.
  const G = node.glue.Governance;
  const ACTIONS = G.actions();
  // WHICH POLICY GOVERNS a domain (THE ONE resolver, for every app and every action): its domain's own (`kinds`:
  // text, video, audio, note, file …), else the setting made before domains (the app's name: board, videos …), else the
  // space's. `DOMAINS`: every domain a policy can name.
  const LEGACY = { text: "board", video: "videos", audio: "audio", image: "images", subtitle: "subtitles", note: "notes", file: "drive" };
  const DOMAINS = Object.keys(LEGACY);
  // A member's credential (hex): `CWMB ‖ did ‖ signer ‖ writer ‖ MLS key ‖ signature` (the identity's format; MLS
  // checked the signature when it admitted it).
  const didOf = h => new Uint8Array(h.match(/../g).slice(4, 36).map(x => parseInt(x, 16)));
  const signerOf = h => h.slice(72, 136);
  const newId = () => [...crypto.getRandomValues(new Uint8Array(8))].map(x => x.toString(16).padStart(2, "0")).join("");

  const spaces = new Map();
  const outside = new Map();
  function ofPublic(desc) {
    if (!outside.has(desc.id)) outside.set(desc.id, open(desc, { outside: true }));
    return outside.get(desc.id);
  }
  function of(sp) {
    sp = sp.parent ?? sp;
    if (!spaces.has(sp.id)) spaces.set(sp.id, open(sp));
    return spaces.get(sp.id);
  }

  async function open(sp, { outside: out = false } = {}) {
    // THE ACTS: the sealed ones and the PUBLIC ones (the same act in both — a copy — counts once: by its key). From
    // outside, only the public ones, read from the writers found so far.
    const pubName = space.tableOf(sp, "pub-acts");
    const index = await ctx.require("index");
    const [first, me, sealedActs, pubActs] = await Promise.all([
      space.owner(sp),
      space.account(),
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
    };
    const g = out ? null : keys.group(sp);
    // MEMBERS: each member of the group (one per DID), where its credential is really that DID's: signed by the data
    // key its key log names (a credential names any DID it likes; the key log says whose it is). WRITERS: each of their
    // devices → the DID. Members who left stay known (what they wrote is still theirs): learned from the group as it
    // was when seen, and from the removals that name them.
    const writers = new Map();
    let group = [];
    let left = false;
    const learn = async st => {
      left = !!st?.removed;
      const all = (st?.members ?? []).filter(m => m.cred).map(m => ({ key: m.key, index: m.index, did: node.glue.did_of(didOf(m.cred)), signer: signerOf(m.cred) }));
      const real = await Promise.all(all.map(async m => (await directory.dataKey(m.did)) === m.signer));
      for (const [i, m] of all.entries()) if (!real[i]) ctx.log("roles", { what: `a member claims ${m.did.slice(12, 20)}… with a key that is not its: left out` });
      group = all.filter((_, i) => real[i]);
      // Each member's DEVICES write on its behalf (their feeds): device key → the DID (the DID's card, checked
      // against its key log).
      const sets = await Promise.all(group.map(m => directory.devices(m.did)));
      group.forEach((m, i) => sets[i].forEach(k => writers.set(k, m.did)));
    };
    if (!out) await learn(await g.ready().catch(() => null));
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
    }
    replay();
    // FROM OUTSIDE: the members are the roster; each one's devices are writers, whose public acts are read too — until
    // no new member turns up.
    const known = new Set();
    // ITS PUBLIC ACTS' WRITERS (`index`: the space's public acts bag — whoever wrote a public act, the owner and admins,
    // listed once): read from outside instead of every member's public acts. Empty (a space from before the bag): every
    // member, as before, until one of its writers opens it with the bag.
    const ACTS = `acts ${sp.id}`;
    const actWriters = out ? await index.pointers(ACTS).then(ps => [...new Set(ps.map(p => p?.w).filter(w => typeof w === "string"))], () => []) : [];
    async function widen() {
      if (!out) return;
      for (;;) {
        const fresh = [...roster].filter(d => !known.has(d));
        if (!fresh.length) return;
        fresh.forEach(d => known.add(d));
        const sets = await Promise.all(fresh.map(d => directory.devices(d).catch(() => [])));
        fresh.forEach((d, i) => sets[i].forEach(k => writers.set(k, d)));
        await pubActs.add(actWriters.length ? actWriters : sets.flat());
        replay();
      }
    }
    if (out) {
      await widen();
      group = [...roster].map(did => ({ did }));
    }

    const changed = [];
    t.onChange(() => {
      replay();
      if (out) widen().then(() => ((group = [...roster].map(did => ({ did }))), changed.forEach(f => f())));
      for (const f of changed) f();
    });
    // PUBLIC: something of the space is anyone's — an app read by anyone, or joining open to anyone (an open space is
    // listed in Discover: whoever looks must see who is in it and how to join).
    const isPublic = () => gv.is_public();
    const role = did => {
      if (!did || (left && did === me?.id)) return null;
      return gv.role(did, group.some(m => m.did === did)) ?? null;
    };
    // Inviting is the space's policy (`invite`); everything else its role's.
    const can = (did, what) => (what === "invite" ? G.passes(gv.effective("", "invite", Infinity), role(did)) : G.can_role(role(did), what));

    // This device listed as a writer of the space's public acts (once a page): after it writes one, and on opening a
    // space whose public acts it wrote before the bag.
    let actListed = false;
    async function listActWriter() {
      if (out || actListed || !sp.self) return;
      actListed = true;
      const ws = await index.pointers(ACTS).catch(() => []);
      if (!ws.some(p => p?.w === sp.self)) await index.point(ACTS, { w: sp.self }).catch(() => (actListed = false));
    }
    const r = {
      space: sp,
      get owner() {
        return owner;
      },
      role,
      can,
      author,
      acts: kind => (kind ? counted.filter(a => a.act === kind) : [...counted]),
      invites: () => JSON.parse(gv.invites(Date.now())),
      apps: () => gv.apps(),
      config: (app, key, dflt = null) => {
        const c = gv.config(app, key);
        return (c === undefined ? null : JSON.parse(c)) ?? dflt;
      },
      policy: (path, action, at = Infinity) => gv.effective(path, action, at),
      // The policies set at exactly this path (not inherited): { action: who }.
      policiesAt: path => Object.fromEntries(ACTIONS.map(x => [x, gv.policy_at(path, x, Infinity)]).filter(([, w]) => w)),
      allows: (action, did, path = "", at = Infinity) => {
        const who = gv.effective(path, action, at);
        return who === "anyone" || G.passes(who, role(did));
      },
      // A DOMAIN's policy for an action (`kinds.policyDomain(kind)`: what an item is decides, not the app showing it).
      policyIn: (domain, action, at = Infinity) =>
        gv.policy_at(domain, action, at) || (LEGACY[domain] && gv.policy_at(LEGACY[domain], action, at)) || gv.effective("", action, at),
      allowsIn: (action, did, domain, at = Infinity) => {
        const own = gv.policy_at(domain, action, at) || (LEGACY[domain] && gv.policy_at(LEGACY[domain], action, at));
        const who = own || gv.effective("", action, at);
        return who === "anyone" || G.passes(who, role(did));
      },
      domains: () => DOMAINS,
      banned: did => bans.has(did),
      bannedList: () => [...bans],
      // Out by the acts — removed, banned, left — and not added back: their nodes leave the group (`moderation`).
      goneList: () => [...gone],
      // This node was removed from the space's group: it reads nothing newer.
      get left() {
        return left;
      },
      // The people of the space now: its group's accounts, with their roles.
      members: () => [...new Set(group.map(m => m.did))].map(did => ({ did, role: role(did) })).filter(m => m.role),
      // A person's member in the group (to remove them): one per DID.
      nodesOf: did => group.filter(m => m.did === did),
      // Brought current with the group (a member added or removed).
      refresh: async () => {
        if (out) return widen();
        await learn(await g.ready().catch(() => null));
        replay();
      },
      isPublic,
      // (A MIGRATION — `upkeep`.) This device listed as a writer of the public acts it wrote before their bag.
      migrate: async () => {
        if (out) return;
        await pubActs.settled;
        if (pubActs.rows().some(x => x.id?.startsWith(sp.self))) await listActWriter();
      },
      // All writers known here (their devices' keys): whose public tails a reader outside reads.
      writerKeys: () => [...writers.keys()],
      // PUBLISH (the owner): the counted acts so far, and a `member` act for each member, into the public acts.
      async publish() {
        if (out || !me || role(me.id) !== "owner") throw new Error("only the owner makes the space's acts public");
        const have = new Set(pubActs.rows().map(x => x.key));
        const ids = new Set(counted.map(a => a.id));
        for (const row of sealedActs.rows()) if (ids.has(row.key) && !have.has(row.key)) await pubActs.put(row.key, row.value);
        const listed = new Set(counted.filter(a => ["member", "added", "admitted"].includes(a.act)).map(a => a.did));
        for (const m of r.members()) if (m.did !== owner && !listed.has(m.did)) await pubActs.put(newId(), JSON.stringify({ act: "member", did: m.did, at: Date.now() }));
        await listActWriter();
      },
      async act(a) {
        if (!me) throw new Error("nobody is logged in");
        if (out) throw new Error("not a member of this space");
        const need = { grant: "grant", remove: "remove", ban: "remove", unban: "remove", hide: "moderate", app: "apps", config: "apps", policy: "apps", transfer: "grant", invite: "invite", "revoke-invite": "invite", admitted: "invite", added: "invite", member: "invite" }[a.act];
        // Leaving is any member's own act (the owner hands the space on first).
        const leaving = a.act === "leave" && role(me.id) && role(me.id) !== "owner";
        if (!leaving && (!need || !can(me.id, need))) throw new Error(`as ${role(me.id) ?? "nobody here"}, you cannot ${a.act} in this space`);
        // A public space's acts are public (readers outside must know them); a private one's sealed.
        const toPublic = isPublic() || (a.act === "policy" && (a.action === "read" || a.action === "join") && a.who === "anyone");
        // Its time: now — or when it happened (upkeep let someone in while no page ran: the act says when).
        await (toPublic ? pubActs : sealedActs).put(newId(), JSON.stringify({ at: Date.now(), ...a }));
        if (toPublic) await listActWriter();
      },
      grant: (did, to) => r.act({ act: "grant", did, role: to }),
      onChange: f => changed.push(f),
      settled: t.settled,
    };
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
  const ruleOf = (item, action, r, at) => item?.meta?.write?.[action] ?? (r ? r.policyIn(K.policyDomain(item.kind), action, at) : "anyone");
  const credChecks = new Map(); // `${author}|${token}` → { v: row | null | undefined }
  const checked = [];
  function credFor(author, token) {
    const k = `${author}|${token}`;
    if (!credChecks.has(k)) {
      const c = { v: undefined };
      credChecks.set(k, c);
      directory
        .publicOf(author, `cred-${token}`, { unlisted: true })
        .then(async t => (t ? (await t.answer(), JSON.parse(t.rows().find(x => x.key === "for")?.value ?? "null")) : null))
        .catch(() => null)
        .then(v => ((c.v = v), checked.forEach(f => f())));
    }
    return credChecks.get(k).v;
  }
  function mayWrite({ action, item, writer, cred = null, r = null, at = Infinity }) {
    const rule = ruleOf(item, action, r, at);
    if (rule === "anyone" || writer === item.by) return true;
    if (rule === "author" || rule === "nobody") return false;
    if (CIRCLES_OF[rule]) {
      if (!cred || !/^[0-9a-f]{32}$/.test(cred)) return false;
      const v = credFor(item.by, cred);
      return v === undefined ? null : !!v && v.did === writer && CIRCLES_OF[rule].includes(v.circle);
    }
    return r ? rule === "anyone" || G.passes(rule, r.role(writer)) : false;
  }
  // The credential a writer cites on an item whose rule is its author's friends or followers (null: none needed / held).
  async function credToCite(item, action, r = null) {
    const rule = ruleOf(item, action, r, Infinity);
    if (!CIRCLES_OF[rule] || item.by === (await space.account())?.id) return null;
    const people = await (await ctx.require("edge")).people();
    for (const w of CIRCLES_OF[rule]) {
      const token = people.about(`credin-${w}`, item.by)?.token;
      if (token) return token;
    }
    throw new Error(`only ${directory.shown(item.by)}'s ${rule} ${action === "vote" ? "vote" : "comment"} here`);
  }

  return { of, ofPublic, mayWrite, credToCite, onChecked: f => checked.push(f), can: (role, what) => G.can_role(role, what), names: ["owner", "admin", "member"] };
}
