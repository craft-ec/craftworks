// ROLES, a capability: who holds what authority IN A SPACE (a server, a direct conversation — any space; a channel is
// its parent's). Roles are per space: holding one never crosses spaces. Built in, for now:
//   owner   everything; first the one who made the space (its id proves it: `space.owner`), then whom they hand it to
//   admin   invite (by id or code), make and delete channels, hide others' items, remove members
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
//   await r.act({ act: "grant", did, role })   // an act, as this person (refused here if it would not count)
//   await r.grant(did, role)  r.onChange(fn)  r.settled
export async function start(ctx) {
  const [space, storage, keys, node, directory] = await Promise.all(["space", "storage", "keys", "node", "directory"].map(n => ctx.require(n)));

  const CAN = {
    owner: new Set(["post", "invite", "channels", "moderate", "remove", "grant"]),
    admin: new Set(["post", "invite", "channels", "moderate", "remove"]),
    member: new Set(["post", "invite"]),
  };
  const RANK = { owner: 3, admin: 2, member: 1 };
  // An invite code in force at `at`: not revoked, not expired, and uses left (0: no limit).
  const live = (inv, at = Date.now()) => !inv.revoked && (!inv.expires || at < inv.expires) && (!inv.uses || inv.admitted.length < inv.uses);
  // A member's credential (hex): `CWMB ‖ did ‖ signer ‖ writer ‖ MLS key ‖ signature` (the identity's format; MLS
  // checked the signature when it admitted it).
  const didOf = h => new Uint8Array(h.match(/../g).slice(4, 36).map(x => parseInt(x, 16)));
  const signerOf = h => h.slice(72, 136);
  const newId = () => [...crypto.getRandomValues(new Uint8Array(8))].map(x => x.toString(16).padStart(2, "0")).join("");

  const spaces = new Map();
  function of(sp) {
    sp = sp.parent ?? sp;
    if (!spaces.has(sp.id)) spaces.set(sp.id, open(sp));
    return spaces.get(sp.id);
  }

  async function open(sp) {
    const [first, me, t] = await Promise.all([space.owner(sp), space.account(), storage.table(space.tableOf(sp, "acts"), sp)]);
    const g = keys.group(sp);
    // WRITERS: each member of the group (one per DID) → its DID, where the credential is really that DID's: signed by
    // the data key its key log names (a credential names any DID it likes; the key log says whose it is). Members who
    // left stay known (what they wrote is still theirs): learned from the group as it was when seen, and from the
    // removals that name them.
    const writers = new Map();
    let group = [];
    let left = false;
    const learn = async st => {
      left = !!st?.removed;
      const all = (st?.members ?? []).filter(m => m.cred).map(m => ({ key: m.key, index: m.index, did: node.glue.did_of(didOf(m.cred)), signer: signerOf(m.cred) }));
      const real = await Promise.all(all.map(async m => (await directory.dataKey(m.did)) === m.signer));
      for (const [i, m] of all.entries()) if (!real[i]) ctx.log("roles", { what: `a member claims ${m.did.slice(12, 20)}… with a key that is not its: left out` });
      group = all.filter((_, i) => real[i]);
      for (const m of group) writers.set(m.key, m.did);
    };
    await learn(await g.ready().catch(() => null));
    const author = row => (row?.id ? (writers.get(row.id.slice(0, 64)) ?? null) : null);

    // THE REPLAY: the acts in order, each kept only if its signer could.
    let roles = new Map();
    let counted = [];
    let owner = first;
    let invites = new Map();
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
          (a.act === "added" && CAN[r]?.has("invite") && a.did && !banned.has(a.did)) ||
          (a.act === "hide" && CAN[r]?.has("moderate")) ||
          (a.act === "transfer" && r === "owner" && a.did && a.did !== a.by && !removed.has(a.did)) ||
          (a.act === "invite" && CAN[r]?.has("invite") && typeof a.code === "string" && a.code && !invites.has(a.code)) ||
          (a.act === "revoke-invite" && inv && (inv.by === a.by || CAN[r]?.has("moderate"))) ||
          (a.act === "admitted" && CAN[r]?.has("invite") && inv && live(inv, a.at) && a.did && !banned.has(a.did));
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
        if (a.act === "admitted" && !inv.admitted.includes(a.did)) inv.admitted.push(a.did);
        if (a.act === "admitted") removed.delete(a.did);
        counted.push(a);
      }
      for (const d of removed) roles.set(d, null);
      bans = banned;
    }
    replay();

    const changed = [];
    t.onChange(() => {
      replay();
      for (const f of changed) f();
    });
    const role = did => {
      if (!did || (left && did === me?.id)) return null;
      if (roles.has(did)) return roles.get(did);
      return group.some(m => m.did === did) ? "member" : null;
    };
    const can = (did, what) => !!CAN[role(did)]?.has(what);

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
        await learn(await g.ready().catch(() => null));
        replay();
      },
      async act(a) {
        if (!me) throw new Error("nobody is logged in");
        const need = { grant: "grant", remove: "remove", ban: "remove", unban: "remove", hide: "moderate", transfer: "grant", invite: "invite", "revoke-invite": "invite", admitted: "invite", added: "invite" }[a.act];
        if (!need || !can(me.id, need)) throw new Error(`as ${role(me.id) ?? "nobody here"}, you cannot ${a.act} in this space`);
        await t.put(newId(), JSON.stringify({ ...a, at: Date.now() }));
      },
      grant: (did, to) => r.act({ act: "grant", did, role: to }),
      onChange: f => changed.push(f),
      settled: t.settled ?? Promise.resolve(),
    };
    return r;
  }

  return { of, can: (role, what) => !!CAN[role]?.has(what), names: Object.keys(CAN) };
}
