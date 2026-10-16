// ORDERING, a capability: ONE AGREED ORDER of entries for an object — the consensus every shared thing rests on. A log
// has POSITIONS 0, 1, 2, …; an entry goes only at the next position; if two writers go for the same position, one
// wins by the log's rule and the other is told so, reads what won, and tries again after it. A position is what the
// other capabilities count by: an MLS epoch, a snapshot number, a membership version.
//
// Several TYPES of ordering, one interface — the type is the rule of who may write and how a tie is decided:
//   `tail`       writers who share one key: the table's own write sequence decides — the first write of a position is
//                accepted, the other is refused. `owner` (a key, hex) names whose tail: the account's MLS commits keep
//                one per EPOCH, under a key from that epoch's secret, so only the nodes in the group then can write it.
//   (A space's acts — people with their own keys writing at once, each naming what it had seen — are a DAG ordered
//   by the replay itself: `Gov::stamp` / `Gov::missing` in the core, the one the node writes acts by too.)
//   (later) `witnessed` k of a known witness set co-sign each position.
//
//   const log = await ordering.open({ type: "tail", table: "mls", prefix: "c/", owner });
//   log.from(n)               // [{ position, entry }] in order, from n while contiguous
//   await log.append(n, hex)  // { ok: true } | { ok: false, taken: true } (someone else wrote n: read, apply, retry)
//   log.onAppend(fn)
//
//   ordering.turn(id, dids, me)    // WHO DOES A THING (no claim, no message): every member ranks the members for `id`
//                                  // the same way — 0 does it now, k takes it over after k × ordering.TAKEOVER of no
//                                  // movement (a member away); -1: not among them
export async function start(ctx) {
  const storage = await ctx.require("storage");
  // The core: the one ranking (`turn`) and the one row rule of an ordered log (`rules`).
  const { glue } = await ctx.require("node");

  // THE ONE ORDER of a position's entries (`rules::ordering::order`, the node's too): lowest first, each once — entries
  // gathered from several places (an epoch log and its door) merged here.
  const order = values => JSON.parse(glue.ordering_entries(JSON.stringify(values)));

  const types = {
    // TAIL: rows `<prefix><position, 12 digits>~<tag>` in one of the account's tables — EACH WRITER'S ENTRY UNDER ITS
    // OWN KEY: two writers racing for one position both land (a write refused for racing is written again by
    // storage, and under one shared key it would overwrite the other's), and the position's entry is the LOWEST of
    // them — the same one on every node once it has read both.
    async tail({ table, prefix, owner, known = null, sealWith = null, space = null }) {
      const t = await (owner ? storage.log(table, owner, { known, sealWith, space }) : storage.table(table));
      // THE ROW RULE is `rules::ordering`'s (through the core — the identity delegate writes and reads by it too).
      const rowsOf = n => glue.ordering_position_prefix(prefix, n);
      // A position's ENTRIES in the one order (`order`); its entry where nothing is checked: the first.
      const entriesAt = n => {
        const p = rowsOf(n);
        return order(t.rows().filter(r => r.value && r.key.startsWith(p)).map(r => r.value));
      };
      const at = n => entriesAt(n)[0];
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
            await t.put(`${rowsOf(n)}${tag()}`, entry);
          } catch (e) {
            if (at(n) !== undefined) return { ok: false, taken: true };
            throw e;
          }
          // Written; the network read again — a writer racing for the same position shows now: the lowest entry is it.
          await ctx.background("ordering: t.reread", t.reread?.());
          return at(n) === entry ? { ok: true } : { ok: false, taken: true };
        },
        // Every entry of position n, in the one order (a reader that checks entries takes the first valid one).
        entries: entriesAt,
        onAppend: f => t.onChange(f),
        // Read again from the network (what another writer appended since).
        reread: () => t.reread?.(),
      };
    },
  };

  async function open(spec) {
    const make = types[spec.type];
    if (!make) throw new Error(`no ordering of type “${spec.type}”`);
    return make(spec);
  }

  // TURNS: `rules::admission::turn` through the core — the ONE ranking (the identity delegate runs it too).
  // The one TAKEOVER (`guards::TAKEOVER_MS`, the node's too).
  const TAKEOVER = glue.takeover_ms();
  const turn = (id, dids, me) => glue.admission_turn(String(id), JSON.stringify([...dids]), String(me));

  return { open, types: Object.keys(types), order, turn, TAKEOVER };
}
