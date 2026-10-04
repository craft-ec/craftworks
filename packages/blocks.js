// BLOCKS, a service: the ONE door for tree blocks — fetching them (raced, and rebuilt from parity when they are
// missing) and putting them. Every table's tree goes through here; nothing else asks the node for a block.
//
// A FETCH RACES (the owner's rule 11, sdk#303; the SDK engine's `race_get`): a block and every other block of its
// sibling group are asked AT ONCE, and whichever comes first ends it — the block itself, or any k of its group, from
// which the core rebuilds it and keeps it only if it hashes to the id asked for. A slow or silent block costs nothing
// extra, and a lost one is not waited on. Each block is asked ONCE however many reads want it. A block no held node
// names a group for (a root written before root parity) is asked alone.
//
//   const blocks = await ctx.require("blocks");
//   await blocks.fetch(tailIdHex, [blockContractHex, …], "notes")   // all held, or throws
//   await blocks.put([[name, frames], …], "notes")                   // all accepted, or throws
//   await blocks.probe(contractHex, "notes")                          // on the network? (keeping counts them)
// A contract's address as the NODE's log writes it (base58 of its 32 bytes): a failure here found there, one to one.
const B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const base58 = hex => {
  let n = BigInt(`0x${hex}`);
  let out = "";
  while (n > 0n) (out = B58[Number(n % 58n)] + out), (n /= 58n);
  for (let i = 0; i < hex.length && hex.slice(i, i + 2) === "00"; i += 2) out = `1${out}`;
  return out;
};
export async function start(ctx) {
  const { core, ask, WAIT } = await ctx.require("node");
  const bytes = hex => new Uint8Array(hex.match(/../g).map(b => parseInt(b, 16)));

  // What this page did: blocks read, and how many of them were rebuilt from their group.
  const stats = { read: 0, rebuilt: 0 };

  // One GET per block in flight: its answer is the table's next view (tagged with the block), or `get-failed`.
  const inflight = new Map();
  function get(b, what) {
    if (!inflight.has(b)) {
      const [, frames] = core.frames_get(bytes(b));
      const p = ask(frames, x => x.block === b || (x.kind === "get-failed" && x.id === b), what, WAIT.ask)
        .catch(() => ({ kind: "get-failed", id: b }))
        .finally(() => inflight.delete(b));
      inflight.set(b, p);
    }
    return inflight.get(b);
  }

  // A block LOST (neither it nor enough of its group came back): not asked again for a while — every read of a tree
  // that needs it fails at once (its table shown without that writer's part), never another wait on the same loss.
  const LOST_FOR = 5 * 60 * 1000;
  const lost = new Map(); // block → when it was found lost
  // One block, raced against its group. Resolves "direct" or "rebuilt"; rejects when neither can happen.
  function race(tail, b, what) {
    const at = lost.get(b);
    if (at && Date.now() - at < LOST_FOR) return Promise.reject(new Error(`${what}: block ${b.slice(0, 12)}… is not on the network (found lost ${Math.round((Date.now() - at) / 1000)} s ago: not asked again yet)`));
    let group = [];
    try {
      group = Array.from(core.tail_group(bytes(tail), b));
    } catch {
      // No group names it: asked alone.
    }
    return new Promise((resolve, reject) => {
      let open = 1 + group.length;
      let answered = 0; // of its group: how many came
      let done = false;
      const settle = () => {
        if (done) return;
        let held = false;
        try {
          held = group.length ? core.tail_rebuild(b) : false;
        } catch {}
        if (held) {
          done = true;
          resolve("rebuilt");
        } else if (open === 0) {
          done = true;
          lost.set(b, Date.now());
          // In FULL — the node's address (base58) too — and its group: what can be followed in the node's log.
          ctx.log("block lost", { what: `${what}: block ${b} (node: ${base58(b)}) — its group of ${group.length}: ${answered} answered; asked ${group.map(g => base58(g)).join(", ") || "(none: asked alone)"}` });
          reject(new Error(`${what}: block ${b.slice(0, 12)}… is not on the network, and too little of its group is to rebuild it`));
        }
      };
      get(b, what).then(a => {
        open--;
        if (done) return;
        if (a.kind !== "get-failed") {
          done = true;
          // Forget the race: the block itself is held. Another read of the same block (two tails sharing a tree: a
          // table and its moved copy) may have forgotten it already — never a throw here, or this read waits forever.
          if (group.length)
            try {
              core.tail_rebuild(b);
            } catch {}
          resolve("direct");
        } else settle();
      });
      for (const g of group)
        get(g, what).then(a => {
          open--;
          if (a?.kind !== "get-failed") answered += 1;
          settle();
        });
    });
  }

  async function fetch(tail, ids, what) {
    const t0 = performance.now();
    const how = await Promise.all(ids.map(b => race(tail, b, `reading ${what}'s tree`)));
    const rebuilt = how.filter(h => h === "rebuilt").length;
    stats.read += ids.length;
    stats.rebuilt += rebuilt;
    ctx.log("tree read", {
      what: `${what}: ${ids.length} block(s), ${ids.length - rebuilt} arrived first, ${rebuilt} from their group first (rebuilt, verified)`,
      ms: Math.round(performance.now() - t0),
    });
  }

  // PUT a flush's blocks: every one accepted, or the first refusal thrown.
  async function put(puts, what) {
    const said = await Promise.all(
      puts.map(([key, frames]) => ask(frames, x => (x.kind === "put" && x.key === key) || x.kind === "refused", `putting ${what}'s tree`, 60000)),
    );
    const refused = said.find(s => s.kind === "refused");
    if (refused) throw new Error(`a tree block was refused: ${refused.said}`);
  }

  // KEEP (phase 4): is the block in contract `c` (hex) on the network? Its GET answered with a state (true) or not
  // found (false). The state is not kept: a probe only counts.
  async function probe(c, what) {
    const [, frames] = core.frames_get(bytes(c));
    const a = await ask(frames, x => (x.kind === "got" && x.id === c) || x.block === c || (x.kind === "get-failed" && x.id === c), what, WAIT.ask).catch(() => ({ kind: "get-failed" }));
    core.take_got(c);
    return a.kind !== "get-failed";
  }

  return { fetch, put, probe, stats: () => ({ ...stats }) };
}
