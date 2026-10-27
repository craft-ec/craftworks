// MODERATION, a capability: acts on OBJECTS (hide: a message, a channel, any row of any table in a space) and on
// ACTORS (remove a person from a space). It applies to every object alike: an object is named by its table and its
// key. Who may is `roles`' (moderate, remove); the acts are roles' acts, so every reader agrees on what counted.
// Nothing is deleted from anyone's feed (nobody writes another's): readers leave out what is hidden.
//
//   const m = await moderation.of(sp)
//   m.hidden(table)          // the Set of keys hidden in that table
//   await m.hide(table, key) // hidden for everyone (a moderator's act)
//   await m.remove(did)      // that person off the list (their keys follow: a new epoch they cannot read); back by an invite
//   await m.ban(did)         // the same, and never back (no invite or code lets them in) until `unban`
//   m.onChange(fn)
//
// MODERATION LISTS — the PUBLIC network's (Discover has no owner, so nobody's acts count there): a person's list is
// their public tail `modlist` (only their account writes it: signed by them), entries `<kind>:<ref>` → { reason, at },
// kind person | post | space. Each reader applies their own list and the lists of whom they chose (`edge` relation
// "modlist"): what is on any of them is left out of Discover.
//   const lists = await moderation.lists()
//   lists.flagged({ by, id, ref, space }) // not seen by this reader: hidden by them (a person, a post, a space —
//                                         // `edge`'s hide/block, private) or on a list they apply; every read asks it
//   await lists.hide({ by, id } | { person } | { space }, on)   // hidden for this person only (an edge of theirs)
//   await lists.flag(kind, ref, reason)   await lists.unflag(kind, ref)   lists.mine()  lists.followed()  lists.onChange(fn)
export async function start(ctx) {
  const [roles, directory, storage, space] = await Promise.all(["roles", "directory", "storage", "space"].map(n => ctx.require(n)));

  // DEPARTED — the one record of a writer that stopped being a member's: a member put out (before the commit takes
  // their nodes out), OR a device dropped from its account (`nodes`: those; owner 10-16). Where each of its feeds stands
  // — so what it wrote still counts, up to there, for everyone who opens the space later, never after (the space's
  // table `departed`, read by `storage.writersOf`; its public copy `pub-departed`, for whoever reads the space from
  // outside — Discover, a guest: its public acts' authors).
  async function departed(sp, did, nodes = null) {
    nodes ??= await directory.devices.strict(did);
    if (!nodes.length) return 0;
    const heads = await storage.headsOf(sp, nodes);
    const [t, pub] = await Promise.all(["departed", "pub-departed"].map(n => storage.table(space.tableOf(sp, n), sp)));
    await Promise.all([t.settled, pub.settled]);
    let n = 0;
    for (const [node, h] of Object.entries(heads)) {
      if (!Object.keys(h).length) continue;
      const row = JSON.stringify({ did, heads: h, at: Date.now() });
      await t.put(node, row);
      await pub.put(node, row);
      n += 1;
    }
    return n;
  }

  // Whether where `did`'s feeds stood is recorded in the space already (`departed`).
  async function recorded(sp, did) {
    const t = await storage.table(space.tableOf(sp, "departed"), sp);
    await t.settled;
    return t.rows().some(x => {
      try {
        return x.value && JSON.parse(x.value)?.did === did;
      } catch {
        return false;
      }
    });
  }

  async function of(sp) {
    const r = await roles.of.strict(sp);
    // STILL IN THE KEYS (K8.2): the BeeKEM tree holds them.
    const holdsKeys = async did => await (await ctx.require("members")).group(r.space).holds(did);
    // OUT: the act — naming their devices, so what they wrote stays theirs after they leave — and where their feeds
    // stand recorded; the KEYS FOLLOW the list (FOUNDATION "Members and keys": the node's follower takes their entries
    // out — asked to run now). A ban of someone already out is the act alone.
    async function out(did, act) {
      await r.refresh();
      // ON THE LIST (the replay's roster) — whether or not they hold keys yet.
      if (!r.roster().includes(did) && act !== "ban") throw new Error("they are not in this space");
      await r.act({ act, did, nodes: await directory.devices.strict(did) });
      if (await holdsKeys(did)) await departed(r.space, did).catch(e => ctx.failed("moderation", e, `where their feeds stand: ${e.message}`));
      await ctx.background("moderation: followNow", (await ctx.require("auth")).identity.followNow());
      await r.refresh();
    }
    return {
      hidden: table => new Set(r.acts("hide").filter(a => a.table === table).map(a => a.item)),
      hide: (table, item) => r.act({ act: "hide", table, item }),
      remove: did => out(did, "remove"),
      ban: did => out(did, "ban"),
      unban: did => r.act({ act: "unban", did }),
      // Anyone the acts put out (banned, removed, left) still in the group: where their feeds stand recorded (what they
      // wrote stays theirs); their entries are the node's follower's to take out. How many.
      enforce: async () => {
        await r.refresh();
        const all = [...new Set([...r.bannedList(), ...r.goneList()])];
        const held = await Promise.all(all.map(holdsKeys));
        const out = all.filter((_, i) => held[i]);
        // ONCE per member (where their feeds stood when they went out — a later pass would move it on, counting what
        // they wrote after as theirs).
        for (const did of out) if (!(await recorded(r.space, did))) await departed(r.space, did).catch(e => ctx.failed("moderation", e, `where their feeds stand: ${e.message}`));
        return out.length;
      },
      onChange: f => r.onChange(f),
      settled: r.settled,
    };
  }

  let listsOpen = null;
  function lists() {
    return (listsOpen ??= (async () => {
      // An item's reference: the one builder (`items.itemRef`).
      const { itemRef } = await ctx.require("items");
      const [space, edge] = await Promise.all(["space", "edge"].map(n => ctx.require(n)));
      const people = await edge.people();
      const me = await space.me.strict();
      const LIST = "modlist";
      const mineT = await directory.publicOf.strict(me, LIST);
      const tails = new Map(); // did → tail
      const changed = [];
      const fire = () => changed.forEach(f => f());
      mineT.onChange(fire);
      const follow = async () => {
        for (const did of people.list("modlist"))
          if (!tails.has(did)) {
            const t = await directory.publicOf(did, LIST);
            if (t) (tails.set(did, t), t.onChange(fire));
          }
      };
      // The lists chosen arrive as they are read (what is hidden by this person's own needs none of them).
      ctx.background("moderation: follow", follow().then(fire));
      people.onChange(() => follow().then(fire));
      const entries = () => {
        const on = new Set();
        for (const t of [mineT, ...[...tails.entries()].filter(([d]) => people.is("modlist", d)).map(([, t]) => t)])
          for (const r of t.rows()) if (r.value) on.add(r.key);
        return on;
      };
      return {
        // THE ONE CHECK of what this person does not see, wherever they read (every app's content, Discover): a
        // POST is its author + its key (`<did>/<key>`: only its author writes it, the same wherever it is read); a
        // person, a space by its id. YOURS (private, `edge`: hide · block) and the LISTS applied (yours, chosen).
        flagged: ({ by = null, id = null, ref = null, space: sid = null } = {}) => {
          const post = by && id ? itemRef(by, id) : null;
          if (by && (people.is("hide", by) || people.is("block", by))) return true;
          if ((post && people.is("hide", post)) || (ref && people.is("hide", ref)) || (sid && people.is("hide", sid))) return true;
          const on = entries();
          return (by && on.has(`person:${by}`)) || (post && on.has(`post:${post}`)) || (ref && on.has(`post:${ref}`)) || (sid && on.has(`space:${sid}`));
        },
        // HIDE, for this person only (an `edge` from them: a post — its author + key —, a person, a space).
        hide: ({ by = null, id = null, person = null, space: sid = null }, on = true) => people.set("hide", person ?? sid ?? itemRef(by, id), on),
        flag: (kind, ref, reason = "") => mineT.put(`${kind}:${ref}`, JSON.stringify({ reason: String(reason).slice(0, 200), at: Date.now() })),
        unflag: (kind, ref) => mineT.remove(`${kind}:${ref}`),
        // THE SIX, one way everywhere (the item bar, a person's card, a space's card): HIDE (private, everywhere) and
        // FLAG (public: your list, Discover) — each of a POST (`{ by, id }`), an AUTHOR (`{ person }`) or a SPACE
        // (`{ space }`). `isHidden` / `isFlagged` say which are on; `setHidden` / `setFlagged` turn one on or off.
        isHidden: ({ by = null, id = null, person = null, space: sid = null }) => people.is("hide", person ?? sid ?? itemRef(by, id)),
        setHidden: ({ by = null, id = null, person = null, space: sid = null }, on) => people.set("hide", person ?? sid ?? itemRef(by, id), on),
        // What this person HID (private): [{ kind: "post"|"person"|"space", ref }] — to show again.
        hiddenList: () =>
          people.list("hide").map(k => ({ kind: k.startsWith("did:") && !k.includes("/") ? "person" : /^[0-9a-f]{64}$/.test(k) ? "space" : "post", ref: k })),
        isFlagged: (kind, ref) => mineT.rows().some(r => r.key === `${kind}:${ref}` && r.value),
        setFlagged: (kind, ref, on) => (on ? mineT.put(`${kind}:${ref}`, JSON.stringify({ reason: "", at: Date.now() })) : mineT.remove(`${kind}:${ref}`)),
        mine: () => mineT.rows().filter(r => r.value).length,
        // This person's own entries: [{ kind, ref }].
        entries: () =>
          mineT
            .rows()
            .filter(r => r.value)
            .map(r => ({ kind: r.key.slice(0, r.key.indexOf(":")), ref: r.key.slice(r.key.indexOf(":") + 1) })),
        followed: () => people.list("modlist"),
        onChange: f => changed.push(f),
      };
    })().catch(e => ((listsOpen = null), Promise.reject(e))));
  }

  // DEVICES DROPPED from this account (owner 10-16): each space it is in records them as DEPARTED — where they stood —
  // so what they wrote stays its own, up to there (a space's rules written by a device since dropped read as none,
  // and the space left Discover). After a device is removed (`membership.remove`), and now and then for any found not
  // recorded yet (a device dropped before this, or while a space was not open). How many recorded.
  async function retire() {
    const me = (await space.account())?.id;
    if (!me) return 0;
    const [ever, now] = await Promise.all([directory.devicesEver.strict(me), directory.devices.strict(me)]);
    const gone = ever.filter(n => !now.includes(n));
    if (!gone.length) return 0;
    let n = 0;
    for (const sp of (await space.mine.strict()).filter(s => s.kind !== "account")) {
      const w = await ctx.optional("moderation: writersOf", storage.writersOf(sp), () => null);
      const todo = gone.filter(node => !w?.byNode.get(node)?.caps);
      if (!w || !todo.length) continue;
      const did = await ctx.optional("moderation: departed", departed(sp, me, todo), () => 0);
      if (did) ctx.log("moderation", { what: `${sp.name ?? sp.id.slice(0, 8)}: ${did} device(s) dropped from this account recorded as departed` });
      n += did;
    }
    return n;
  }
  (await ctx.require("lifecycle")).job("departed devices", { every: 6 * 3600e3, delay: 60e3, needs: async () => !!(await space.account()) || "nobody is logged in", run: () => retire() });

  return { of, lists, retire };
}
