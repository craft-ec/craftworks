// EDGE, a capability: *me → a thing*, a small value — the shape of pins, labels (private tags), and later likes, votes,
// follows, saves. Each kind is one table of the account (sealed, like all its tables), and names the thing it points at
// by a ref (`notes:<id>`, `app:/notes`, …). No UI here: the pin button and the label menu are components.
//
//   const edge = await ctx.require("edge");
//   const pins = await edge.pins();      pins.has(ref)  pins.refs(prefix)  await pins.set(ref, on)  pins.onChange(fn)
//   const labels = await edge.labels();  labels.list()  labels.of(ref)  labels.refs(id, prefix)  labels.onChange(fn)
//     await labels.create(name)  labels.rename(id, name)  labels.remove(id)  labels.set(ref, id, on)  labels.clear(ref)
//   await edge.adoptPinnedField(table, prefix)   // rows saved with an old `pinned: true` field → pins, field dropped
//   const people = await edge.people(); people.is("follow", did)  people.list("friend")  await people.set("hide", did, on)
//     people.onChange(fn) — me → a PERSON (or, for `follow`, any SPACE: a person's DID is their personal space's id; a
//     shared space's id follows it, `people.about("follow", id)` its public description): follow · friend · asked (a friend request sent) · answered (their request,
//     when: yes, no or ended — a request older than it is not asked again) · hide (their items unseen
//     here) · block (hidden, and their welcomes, mail and requests refused) · modlist (their moderation list applied
//     to what this person sees in Discover: `moderation.lists`)
//
// PEOPLE: table `people`, a row `<relation>/<did>` per link (one table: the relations are one mechanism).
// PINS: table `pins`, a row per pinned ref. LABELS: table `tags` (its name from before; private tags):
//   `l/<id>` { name } a label;  `a/<id>/<ref>` { at } that label on a thing.
export async function start(ctx) {
  const storage = await ctx.require("storage");

  let pinsOpen = null;
  function pins() {
    return (pinsOpen ??= storage.table("pins").then(t => {
      const has = ref => t.rows().some(r => r.key === ref);
      const refs = prefix => t.rows().map(r => r.key).filter(k => !prefix || k.startsWith(prefix));
      const set = (ref, on) => (on ? t.put(ref, JSON.stringify({ at: Date.now() })) : t.remove(ref));
      return { has, refs, set, onChange: t.onChange };
    }));
  }

  let labelsOpen = null;
  function labels() {
    return (labelsOpen ??= storage.table("tags").then(t => {
      const parse = v => {
        try {
          return JSON.parse(v) ?? {};
        } catch {
          return {};
        }
      };
      const byName = (a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
      const list = () =>
        t.rows()
          .filter(r => r.key.startsWith("l/"))
          .map(r => ({ id: r.key.slice(2), name: String(parse(r.value).name ?? "") }))
          .filter(l => l.name)
          .sort(byName);
      const assigned = () => t.rows().filter(r => r.key.startsWith("a/")).map(r => {
        const rest = r.key.slice(2);
        const i = rest.indexOf("/");
        return { id: rest.slice(0, i), ref: rest.slice(i + 1) };
      });
      const of = ref => {
        const ids = new Set(assigned().filter(a => a.ref === ref).map(a => a.id));
        return list().filter(l => ids.has(l.id));
      };
      const refs = (id, prefix = "") => assigned().filter(a => a.id === id && a.ref.startsWith(prefix)).map(a => a.ref);
      const find = name => list().find(l => l.name.localeCompare(name.trim(), undefined, { sensitivity: "base" }) === 0);

      const newId = () => `${Date.now().toString(36)}${[...crypto.getRandomValues(new Uint8Array(3))].map(b => b.toString(16).padStart(2, "0")).join("")}`;
      async function create(name) {
        name = name.trim().slice(0, 50);
        if (!name) throw new Error("a label needs a name");
        const same = find(name);
        if (same) return same.id;
        const id = newId();
        await t.put(`l/${id}`, JSON.stringify({ name }));
        return id;
      }
      const rename = (id, name) => {
        name = name.trim().slice(0, 50);
        const same = find(name);
        if (!name || (same && same.id !== id)) return Promise.reject(new Error(name ? `there is already a label “${same.name}”` : "a label needs a name"));
        return t.put(`l/${id}`, JSON.stringify({ name }));
      };
      // A label goes with every use of it.
      async function remove(id) {
        for (const ref of refs(id)) await t.remove(`a/${id}/${ref}`);
        await t.remove(`l/${id}`);
      }
      const set = (ref, id, on) => (on ? t.put(`a/${id}/${ref}`, JSON.stringify({ at: Date.now() })) : t.remove(`a/${id}/${ref}`));
      // A thing that is gone takes its labels with it.
      async function clear(ref) {
        for (const l of of(ref)) await t.remove(`a/${l.id}/${ref}`);
      }
      return { list, of, refs, create, rename, remove, set, clear, onChange: t.onChange };
    }));
  }

  // Rows saved with an old `pinned: true` field (notes, before pins were their own table): into the pins, and saved
  // again without the field. Once per row; harmless when there are none.
  async function adoptPinnedField(table, prefix) {
    const p = await pins();
    for (const r of table.rows()) {
      let j;
      try {
        j = JSON.parse(r.value);
      } catch {
        continue;
      }
      if (j?.pinned !== true) continue;
      const { pinned, ...rest } = j;
      await (p.has(`${prefix}${r.key}`) ? Promise.resolve() : p.set(`${prefix}${r.key}`, true));
      await table.put(r.key, JSON.stringify(rest));
    }
  }

  let peopleOpen = null;
  // `follower`: someone who follows this person (from their notice: `circles`).
  // `cred-friends` / `cred-followers`: a write credential this person issued (its token in `about`); `credin-…`: one
  // they hold from someone (`circles`).
  const RELATIONS = new Set(["follow", "follower", "friend", "asked", "answered", "hide", "block", "modlist", "cred-friends", "cred-followers", "credin-friends", "credin-followers"]);
  function people() {
    return (peopleOpen ??= storage.table("people").then(t => {
      const is = (rel, did) => t.rows().some(r => r.key === `${rel}/${did}`);
      // When a link was made (ms; 0: none).
      const at = (rel, did) => {
        try {
          return Number(JSON.parse(t.rows().find(r => r.key === `${rel}/${did}`)?.value ?? "{}").at) || 0;
        } catch {
          return 0;
        }
      };
      const list = rel => t.rows().filter(r => r.key.startsWith(`${rel}/`)).map(r => r.key.slice(rel.length + 1));
      // `at`: when the link holds from (default now; what it answers may be older). `about`: what is needed to read
      // the thing it points at — a followed SHARED space's public description (a person needs none: their DID is their
      // personal space, found by it).
      const set = async (rel, did, on, at = Date.now(), about = null) => {
        if (!RELATIONS.has(rel)) throw new Error(`no relation “${rel}”`);
        // A FOLLOW of a person by their rule (`roles.followable`): where only their friends may, it cites the
        // credential they handed (a friend's); where nobody may, it is not made.
        let cred = null;
        if (rel === "follow" && on && String(did).startsWith("did:craftec:")) {
          const roles = await ctx.require("roles");
          const item = await roles.followable(did);
          if (item.meta.write.follow === "author") throw new Error(`${(await ctx.require("directory")).shown(did)} is not taking followers`);
          cred = await roles.credToCite(item, "follow");
        }
        await (on ? t.put(`${rel}/${did}`, JSON.stringify({ at, ...(about ? { about } : {}) })) : t.remove(`${rel}/${did}`));
        // A FOLLOW of a person tells them (a notice in their inbox): they count the follower (`circles`), unasked.
        if (rel === "follow" && String(did).startsWith("did:craftec:"))
          ctx
            .require("index")
            .then(async index => index.send(did, { kind: on ? "follow" : "unfollow", from: (await (await ctx.require("space")).account()).id, at, ...(cred ? { cred } : {}) }))
            .catch(e => ctx.log("edge", { what: `telling ${String(did).slice(12, 20)}… of the follow: ${e?.message ?? e}` }));
      };
      const about = (rel, id) => {
        try {
          return JSON.parse(t.rows().find(r => r.key === `${rel}/${id}`)?.value ?? "{}").about ?? null;
        } catch {
          return null;
        }
      };
      return { is, at, list, set, about, onChange: t.onChange, settled: t.settled };
    }));
  }

  return { pins, labels, people, adoptPinnedField };
}
