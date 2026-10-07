// ORDERING, a capability: ONE AGREED ORDER of entries for an object — the consensus every shared thing rests on. A log
// has POSITIONS 0, 1, 2, …; an entry goes only at the next position; if two writers go for the same position, one
// wins by the log's rule and the other is told so, reads what won, and tries again after it. A position is what the
// other capabilities count by: an MLS epoch, a snapshot number, a membership version.
//
// Several TYPES of ordering, one interface — the type is the rule of who may write and how a tie is decided:
//   `tail`       writers who share one key: the table's own write sequence decides — the first write of a position is
//                accepted, the other is refused. `owner` (a key, hex) names whose tail: the account's MLS commits keep
//                one per EPOCH, under a key from that epoch's secret, so only the nodes in the group then can write it.
//   `dag`        people with their own keys (a space's members), each writing in their own feed, AT ONCE: nobody waits
//                for a position and nobody loses a tie. Each entry NAMES THE ENTRIES ITS WRITER HAD SEEN (`deps`: the
//                heads then — those no other entry names) and carries a time PAST every one it saw (`at`: its clock, or
//                just after the latest seen). So the time order (`at`, then id) is a CAUSAL order — an entry always
//                after what it built on, whoever's clock was behind — and a reader that sees an entry naming one it
//                does not have KNOWS its view is short (`missing`). Old entries (no `deps`) keep their own time.
//                (Replaces the planned `log`: concurrent writers both count, merged by the replay's rules.)
//   (later) `witnessed` k of a known witness set co-sign each position.
//
//   const log = await ordering.open({ type: "tail", table: "mls", prefix: "c/", owner });
//   log.from(n)               // [{ position, entry }] in order, from n while contiguous
//   await log.append(n, hex)  // { ok: true } | { ok: false, taken: true } (someone else wrote n: read, apply, retry)
//   log.onAppend(fn)
//
//   const d = await ordering.open({ type: "dag", source })   // source: () => the entries that COUNT, [{ id, at, deps }]
//   d.stamp({ act: "grant", … })   // the value to write: `deps` (the heads now) and `at` (kept when the caller gives one)
//   d.heads()                      // ids no entry names
//   d.missing(have)                // ids entries name that are not among `have` (a Set: every id written, counted or
//                                  // not) — this reader's view is short
// Only entries that COUNT time and head a new one (a refused act — a non-member's, a time pushed far ahead by one
// with no right to act — moves nobody's clock).
export async function start(ctx) {
  const storage = await ctx.require("storage");

  const types = {
    // TAIL: rows `<prefix><position, 12 digits>~<tag>` in one of the account's tables — EACH WRITER'S ENTRY UNDER ITS
    // OWN KEY: two writers racing for one position both land (a write refused for racing is written again by
    // storage, and under one shared key it would overwrite the other's), and the position's entry is the LOWEST of
    // them — the same one on every node once it has read both.
    async tail({ table, prefix, owner, known = null, sealWith = null, space = null }) {
      const t = await (owner ? storage.log(table, owner, { known, sealWith, space }) : storage.table(table));
      const key = n => `${prefix}${String(n).padStart(12, "0")}`;
      const at = n => {
        const k = key(n);
        const all = t.rows().filter(r => r.value && r.key.startsWith(`${k}~`)).map(r => r.value);
        return all.length ? all.sort()[0] : undefined;
      };
      const tag = () => [...crypto.getRandomValues(new Uint8Array(6))].map(x => x.toString(16).padStart(2, "0")).join("");
      return {
        from(n) {
          const out = [];
          for (let p = n; ; p++) {
            const entry = at(p);
            if (entry === undefined) return out;
            out.push({ position: p, entry });
          }
        },
        async append(n, entry) {
          if (at(n) !== undefined) return { ok: false, taken: true };
          try {
            await t.put(`${key(n)}~${tag()}`, entry);
          } catch (e) {
            if (at(n) !== undefined) return { ok: false, taken: true };
            throw e;
          }
          // Written; the network read again — a writer racing for the same position shows now: the lowest entry is it.
          await t.reread?.().catch(() => {});
          return at(n) === entry ? { ok: true } : { ok: false, taken: true };
        },
        onAppend: f => t.onChange(f),
        // Read again from the network (what another writer appended since).
        reread: () => t.reread?.(),
      };
    },

    // DAG: over the entries that count (a space's acts as the replay kept them) — nothing written here, the caller
    // writes what `stamp` gives under a new id.
    async dag({ source }) {
      return dag(source);
    },
  };

  // A CLOCK AHEAD by more than this is not followed (one writer's wrong clock would push every later entry's time —
  // and with it when policies take effect — into the future): its entry stays where its own time puts it.
  const AHEAD = 3600000;
  const DEPS = 8;
  function dag(source) {
    const entries = () => source().filter(e => e && typeof e.id === "string");
    const named = es => new Set(es.flatMap(e => (Array.isArray(e.deps) ? e.deps : [])));
    const at = e => (Number.isFinite(e.at) ? e.at : 0);
    const heads = (es = entries()) => {
      const n = named(es);
      return es.filter(e => !n.has(e.id));
    };
    return {
      heads: () => heads().map(e => e.id),
      missing: have => [...named(entries())].filter(id => !have.has(id)),
      stamp(value) {
        const es = entries();
        const now = Date.now();
        // The latest time seen, less any from a clock too far ahead.
        const latest = es.reduce((m, e) => (at(e) <= now + AHEAD ? Math.max(m, at(e)) : m), 0);
        const deps = heads(es)
          .sort((a, b) => at(b) - at(a) || (a.id < b.id ? -1 : 1))
          .slice(0, DEPS)
          .map(e => e.id);
        return { ...value, at: value.at ?? Math.max(now, latest + 1), deps };
      },
    };
  }


  async function open(spec) {
    const make = types[spec.type];
    if (!make) throw new Error(`no ordering of type “${spec.type}”`);
    return make(spec);
  }

  return { open, types: Object.keys(types) };
}
