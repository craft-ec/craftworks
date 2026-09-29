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
  const [space, storage, keys, node, directory] = await Promise.all(["space", "storage", "keys", "node", "directory"].map(n => ctx.require(n)));

  const CAN = {
    owner: new Set(["post", "invite", "channels", "moderate", "remove", "grant", "apps"]),
    admin: new Set(["post", "invite", "channels", "moderate", "remove", "apps"]),
    member: new Set(["post", "invite"]),
  };
  const RANK = { owner: 3, admin: 2, member: 1 };
  const ACTIONS = ["read", "post", "comment", "vote", "edit", "join", "invite"];
  const WHO = ["anyone", "members", "admins", "owner", "nobody", "inherit"];
  // The settings from before policies (`config` acts), as policies — their defaults ("everyone", "members",
  // "invite") as INHERIT (no policy of their own), never as an override.
  const OLD = {
    "chat/post": { path: "chat", action: "post", who: v => (v === "admins" ? "admins" : null) },
    "board/post": { path: "board", action: "post", who: v => (v === "admins" ? "admins" : null) },
    "board/read": { path: "board", action: "read", who: v => (v === "public" ? "anyone" : null) },
    "notes/edit": { path: "notes", action: "edit", who: v => (v === "admins" ? "admins" : null) },
    "space/join": { path: "", action: "join", who: v => (v === "open" ? "anyone" : null) },
  };
  // The apps a shared space may use (each app's key: its route).
  const APPS = ["chat", "board", "notes"];
  // An invite code in force at `at`: not revoked, not expired, and uses left (0: no limit).
  const live = (inv, at = Date.now()) => !inv.revoked && (!inv.expires || at < inv.expires) && (!inv.uses || inv.admitted.length < inv.uses);
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

    // THE REPLAY: the acts in order, each kept only if its signer could.
    let roles = new Map();
    let counted = [];
    let owner = first;
    let invites = new Map();
    let apps = new Map();
    let configs = new Map();
    let history = new Map(); // `${path}|${action}` → [{ at, who }], in the order they counted
    let roster = new Set(); // the DIDs the acts name as members (the owner, added, admitted, rostered)
    // A POLICY set at a path (its history: time-aware), and the EFFECTIVE one — walked up the path to the space.
    const setPolicy = (path, action, who, at) => {
      const k = `${path}|${action}`;
      (history.get(k) ?? history.set(k, []).get(k)).push({ at: Number(at) || 0, who });
    };
    const policyAt = (path, action, at = Infinity) => (history.get(`${path}|${action}`) ?? []).filter(x => x.at <= at).pop()?.who ?? null;
    // Does a role pass a policy's `who` (anyone here means any member: only members act in a space).
    const passes = (who, r) => (who === "nobody" || !r ? false : who === "owner" ? r === "owner" : who === "admins" ? RANK[r] >= RANK.admin : true);
    // INVITING (codes, adding by id, letting askers in): the space's `invite` policy as it was then (default: members).
    const mayInvite = (r, at) => passes(effective("", "invite", at), r);
    const effective = (path, action, at = Infinity) => {
      const parts = String(path ?? "").split("/").filter(Boolean);
      for (let i = parts.length; i >= 0; i--) {
        const who = policyAt(parts.slice(0, i).join("/"), action, at);
        if (who) return who;
      }
      return "members";
    };
    let bans = new Set();
    function replay() {
      const acts = [];
      for (const r of t.rows()) {
        try {
          const v = JSON.parse(r.value);
          for (const n of v.nodes ?? []) if (v.act === "remove" && v.did) writers.set(n, writers.get(n) ?? v.did);
          acts.push({ ...v, id: r.key, by: author(r) });
        } catch {}
      }
      acts.sort((a, b) => (Number(a.at) || 0) - (Number(b.at) || 0) || (a.id < b.id ? -1 : 1));
      owner = first;
      roles = new Map(owner ? [[owner, "owner"]] : []);
      invites = new Map();
      apps = new Map();
      configs = new Map();
      history = new Map();
      roster = new Set(first ? [first] : []);
      const removed = new Set();
      const banned = new Set();
      const roleAt = d => (removed.has(d) ? null : (roles.get(d) ?? "member"));
      counted = [];
      for (const a of acts) {
        if (!a.by || removed.has(a.by)) continue;
        const r = roleAt(a.by);
        const inv = a.code && invites.get(a.code);
        const ok =
          (a.act === "grant" && r === "owner" && a.did !== owner && ["admin", "member"].includes(a.role)) ||
          ((a.act === "remove" || a.act === "ban") && CAN[r]?.has("remove") && a.did !== a.by && RANK[r] > RANK[roleAt(a.did) ?? "member"]) ||
          (a.act === "unban" && CAN[r]?.has("remove") && banned.has(a.did)) ||
          (a.act === "added" && mayInvite(r, a.at) && a.did && !banned.has(a.did)) ||
          (a.act === "hide" && CAN[r]?.has("moderate")) ||
          (a.act === "transfer" && r === "owner" && a.did && a.did !== a.by && !removed.has(a.did)) ||
          (a.act === "invite" && mayInvite(r, a.at) && typeof a.code === "string" && a.code && !invites.has(a.code)) ||
          (a.act === "revoke-invite" && inv && (inv.by === a.by || CAN[r]?.has("moderate"))) ||
          (a.act === "admitted" && mayInvite(r, a.at) && inv && live(inv, a.at) && a.did && !banned.has(a.did)) ||
          (a.act === "app" && CAN[r]?.has("apps") && APPS.includes(a.app)) ||
          // Who may READ an app (members, or anyone) is the owner's: making it public publishes the space's acts.
          // The space's own settings are app "space" (who may join: "invite" or "open").
          (a.act === "config" && CAN[r]?.has("apps") && (APPS.includes(a.app) || a.app === "space") && typeof a.key === "string" && a.key.length <= 32 && (a.key !== "read" || r === "owner")) ||
          // A POLICY: who may do an action at a path (reading is the owner's: public reading publishes the space's acts).
          (a.act === "policy" && CAN[r]?.has("apps") && typeof a.path === "string" && a.path.length <= 120 && ACTIONS.includes(a.action) && WHO.includes(a.who) && (a.action !== "read" || r === "owner")) ||
          // Admitted by asking, while the space was OPEN (no code).
          (a.act === "admitted" && a.code === "open" && mayInvite(r, a.at) && policyAt("", "join") === "anyone" && a.did && !banned.has(a.did)) ||
          (a.act === "member" && mayInvite(r, a.at) && a.did && !banned.has(a.did));
        if (!ok) continue;
        if (a.act === "grant") roles.set(a.did, a.role);
        // Removed: out, and back only by an invite or a code (which clears it). Banned: out, and never back until unbanned.
        if (a.act === "remove" || a.act === "ban") {
          removed.add(a.did);
          roles.delete(a.did);
        }
        if (a.act === "ban") banned.add(a.did);
        if (a.act === "unban") banned.delete(a.did);
        if (a.act === "added" || a.act === "admitted") removed.delete(a.did);
        if (a.act === "transfer") {
          roles.set(owner, "admin");
          owner = a.did;
          roles.set(owner, "owner");
        }
        if (a.act === "invite") invites.set(a.code, { code: a.code, by: a.by, at: a.at, expires: Number(a.expires) || 0, uses: Number(a.uses) || 0, admitted: [] });
        if (a.act === "revoke-invite") inv.revoked = a.at;
        if (a.act === "admitted" && inv && !inv.admitted.includes(a.did)) inv.admitted.push(a.did);
        if (a.act === "admitted") removed.delete(a.did);
        if (a.act === "member") removed.delete(a.did);
        if (a.act === "added" || a.act === "admitted" || a.act === "member") roster.add(a.did);
        if (a.act === "app") apps.set(a.app, !!a.on);
        if (a.act === "config") {
          configs.set(`${a.app}/${a.key}`, a.value);
          // The settings from before policies, read as the policies they are.
          const as = OLD[`${a.app}/${a.key}`];
          if (as) setPolicy(as.path, as.action, as.who(a.value), a.at);
        }
        // `inherit`: the override removed (the parent's policy applies again).
        if (a.act === "policy") setPolicy(a.path, a.action, a.who === "inherit" ? null : a.who, a.at);
        counted.push(a);
      }
      for (const d of removed) roles.set(d, null);
      for (const d of removed) roster.delete(d);
      bans = banned;
    }
    replay();
    // FROM OUTSIDE: the members are the roster; each one's devices are writers, whose public acts are read too — until
    // no new member turns up.
    const known = new Set();
    async function widen() {
      if (!out) return;
      for (;;) {
        const fresh = [...roster].filter(d => !known.has(d));
        if (!fresh.length) return;
        fresh.forEach(d => known.add(d));
        const sets = await Promise.all(fresh.map(d => directory.devices(d).catch(() => [])));
        fresh.forEach((d, i) => sets[i].forEach(k => writers.set(k, d)));
        await pubActs.add(sets.flat());
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
    // Public: an app of the space reads in public.
    // PUBLIC: something of the space is anyone's — an app read by anyone, or joining open to anyone (an open space is
    // listed in Discover: whoever looks must see who is in it and how to join).
    const isPublic = () => policyAt("", "join") === "anyone" || [...history.keys()].some(k => k.endsWith("|read") && policyAt(k.slice(0, -5), "read") === "anyone");
    const role = did => {
      if (!did || (left && did === me?.id)) return null;
      if (roles.has(did)) return roles.get(did);
      return group.some(m => m.did === did) ? "member" : null;
    };
    // Inviting is the space's policy (`invite`); everything else its role's.
    const can = (did, what) => (what === "invite" ? passes(effective("", "invite"), role(did)) : !!CAN[role(did)]?.has(what));

    const r = {
      space: sp,
      get owner() {
        return owner;
      },
      role,
      can,
      author,
      acts: kind => (kind ? counted.filter(a => a.act === kind) : [...counted]),
      invites: () => [...invites.values()].filter(i => live(i)),
      apps: () => APPS.filter(x => apps.get(x) === true),
      config: (app, key, dflt = null) => configs.get(`${app}/${key}`) ?? dflt,
      policy: (path, action, at = Infinity) => effective(path, action, at),
      // The policies set at exactly this path (not inherited): { action: who }.
      policiesAt: path => Object.fromEntries(ACTIONS.filter(x => policyAt(path, x)).map(x => [x, policyAt(path, x)])),
      allows: (action, did, path = "", at = Infinity) => {
        const who = effective(path, action, at);
        if (who === "anyone") return true;
        if (who === "nobody") return false;
        const r = role(did);
        if (!r) return false;
        return who === "owner" ? r === "owner" : who === "admins" ? RANK[r] >= RANK.admin : true;
      },
      banned: did => bans.has(did),
      bannedList: () => [...bans],
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
      },
      async act(a) {
        if (!me) throw new Error("nobody is logged in");
        if (out) throw new Error("not a member of this space");
        const need = { grant: "grant", remove: "remove", ban: "remove", unban: "remove", hide: "moderate", app: "apps", config: "apps", policy: "apps", transfer: "grant", invite: "invite", "revoke-invite": "invite", admitted: "invite", added: "invite", member: "invite" }[a.act];
        if (!need || !can(me.id, need)) throw new Error(`as ${role(me.id) ?? "nobody here"}, you cannot ${a.act} in this space`);
        // A public space's acts are public (readers outside must know them); a private one's sealed.
        const toPublic = isPublic() || (a.act === "policy" && (a.action === "read" || a.action === "join") && a.who === "anyone");
        await (toPublic ? pubActs : sealedActs).put(newId(), JSON.stringify({ ...a, at: Date.now() }));
      },
      grant: (did, to) => r.act({ act: "grant", did, role: to }),
      onChange: f => changed.push(f),
      settled: t.settled,
    };
    return r;
  }

  return { of, ofPublic, can: (role, what) => !!CAN[role]?.has(what), names: Object.keys(CAN) };
}
