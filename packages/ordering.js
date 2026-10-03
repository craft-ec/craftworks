// ORDERING, a capability: ONE AGREED ORDER of entries for an object — the consensus every shared thing rests on. A log
// has POSITIONS 0, 1, 2, …; an entry goes only at the next position; if two writers go for the same position, one
// wins by the log's rule and the other is told so, reads what won, and tries again after it. A position is what the
// other capabilities count by: an MLS epoch, a snapshot number, a membership version.
//
// Several TYPES of ordering, one interface — the type is the rule of who may write and how a tie is decided:
//   `tail`       writers who share one key: the table's own write sequence decides — the first write of a position is
//                accepted, the other is refused. `owner` (a key, hex) names whose tail: the account's MLS commits keep
//                one per EPOCH, under a key from that epoch's secret, so only the nodes in the group then can write it.
//   (next) `log`       people with their own keys (a space's members): a Log contract, lowest hash wins a tie.
//   (later) `witnessed` k of a known witness set co-sign each position.
//
//   const log = await ordering.open({ type: "tail", table: "mls", prefix: "c/", owner });
//   log.from(n)               // [{ position, entry }] in order, from n while contiguous
//   await log.append(n, hex)  // { ok: true } | { ok: false, taken: true } (someone else wrote n: read, apply, retry)
//   log.onAppend(fn)
export async function start(ctx) {
  const storage = await ctx.require("storage");

  const types = {
    // TAIL: rows `<prefix><position, 12 digits>~<tag>` in one of the account's tables — EACH WRITER'S ENTRY UNDER ITS
    // OWN KEY: two writers racing for one position both land (a write refused for racing is written again by
    // storage, and under one shared key it would overwrite the other's), and the position's entry is the LOWEST of
    // them — the same one on every node once it has read both. (`<prefix><position>` alone: one written before.)
    async tail({ table, prefix, owner, known = null, sealWith = null, space = null }) {
      const t = await (owner ? storage.log(table, owner, { known, sealWith, space }) : storage.table(table));
      const key = n => `${prefix}${String(n).padStart(12, "0")}`;
      const at = n => {
        const k = key(n);
        const all = t.rows().filter(r => r.value && (r.key === k || r.key.startsWith(`${k}~`))).map(r => r.value);
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
  };

  async function open(spec) {
    const make = types[spec.type];
    if (!make) throw new Error(`no ordering of type “${spec.type}”`);
    return make(spec);
  }

  return { open, types: Object.keys(types) };
}
