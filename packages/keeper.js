// KEEPER, a service (TOKEN §7a: keepers are paid on PROVEN keeping): while a page is open, the STANDING RECORD of this
// device's keeper — named by its MEMBER KEY (hex): the node refuses node queries from web apps, so a page never learns
// its peer id or what its node hosts, and proves only what CRAFTWORKS keeps — in its standing contract. Each standing
// epoch its HOLDINGS: the pieces of the files `keep` keeps whole (`keep.holdings`, fixed for the epoch); at each beacon
// height SAMPLED for it, its ANSWER to the beacon's challenge: the asked pieces read (a read is keeping: Freenet evicts
// what is read least), sealed in the page's worker (`beacon.sealed`), signed. Every record is signed by the member key
// through its identity delegate (`tokenSign`, the standing domain). (Answering with the page CLOSED is the delegate's.)
//
//   (await ctx.require("keeper")).status()   // { peer, epoch, files, pieces, opened, answered, last } — what this page did
export async function start(ctx) {
  const beaconSvc = await ctx.require("beacon");
  if (beaconSvc.status().off) return { status: () => ({ off: true }) };
  const [node, auth, standingCode, beaconCode] = await Promise.all(["node", "auth", "standing-wasm", "beacon-wasm"].map(n => ctx.require(n)));
  const { core } = node;
  const Core = node.glue.CraftworksCore;
  const state = { peer: null, epoch: null, files: 0, pieces: 0, opened: 0, answered: 0, last: null };
  let self = null; // { epoch, hosted, open } — this epoch's holdings ([id hex, size]) and whether its record is open
  let busy = false;
  let upTo = -1; // the last height handled
  // How far back a catch-up looks (the beacon's state keeps its latest steps' outputs; older ones are null).
  const BACK = 1024;

  // The epoch's HOLDINGS: every piece of the files `keep` keeps whole, fixed for the epoch.
  async function holdingsOf(epoch) {
    // (Like the token service, `keep` and `files` need an account: asked for once one is logged in.)
    const [keep, files] = await Promise.all(["keep", "files"].map(n => ctx.require(n)));
    const fs = await keep.holdings(epoch);
    const hosted = [];
    for (const { sp, row } of fs) hosted.push(...(await files.pieces(sp, row).catch(e => (ctx.log("keeper", { what: `${row.id.slice(0, 8)}…: its pieces not listed (${e?.message ?? e})` }), []))));
    state.files = fs.length;
    state.pieces = hosted.length;
    return hosted;
  }

  async function tick(h) {
    if (busy) return;
    busy = true;
    try {
      const me = await auth.check().catch(() => null);
      if (!me) return;
      // (The token service needs an account: asked for once one is logged in.)
      const tokens = await ctx.require("tokens");
      if (!tokens.on) return;
      const { beacon, run, putAll, bytes, hexOf } = tokens;
      const public_ = bytes(me.member);
      const peer = me.member.toLowerCase();
      const epoch = Core.keeper_epoch(h);
      // (None yet — `keep`'s passes still running: asked again 64 heights on.)
      if (!self || self.epoch !== epoch || (!self.hosted.length && h - self.at >= 64)) self = { epoch, at: h, hosted: await holdingsOf(epoch), open: false };
      if (!self.hosted.length) return;
      Object.assign(state, { peer, epoch });
      // OPEN the epoch: binding and holdings.
      if (!self.open) {
        const r = await run(sigs => core.keeper_open(standingCode, beaconCode, beacon, public_, peer, epoch, self.hosted, sigs));
        if (r?.puts) {
          await putAll(r.puts);
          state.opened++;
          ctx.log("keeper", { what: `epoch ${epoch}: holdings of ${state.files} file(s), ${self.hosted.length} piece(s) committed` });
        }
        self.open = true;
      }
      // ANSWER every height of this epoch since the last one handled that is sampled for this keeper (one that came
      // while it was sealing is not missed), while the beacon's state still holds its output. The beacon read into the
      // core first (the beacon service takes its own reads out of it).
      await tokens.read(hexOf(beacon));
      for (let x = Math.max(upTo + 1, h - BACK); x <= h; x++) {
        if (Core.keeper_epoch(x) !== epoch) continue;
        const out = core.beacon_out(beacon, x);
        if (!out) continue;
        const asked = await run(() => core.keeper_asked(standingCode, beaconCode, beacon, public_, peer, x, out, self.hosted));
        if (!asked?.length) continue;
        const hashes = [];
        for (const id of asked) {
          await tokens.read(id);
          const piece = core.take_got(id) ?? new Uint8Array(0);
          hashes.push([id, await beaconSvc.sealed(public_, bytes(id), piece, out, peer)]);
        }
        const r = await run(sigs => core.keeper_answer(standingCode, beaconCode, beacon, public_, peer, x, out, self.hosted, hashes, sigs));
        if (r?.puts) {
          await putAll(r.puts);
          state.answered++;
          state.last = x;
          ctx.log("keeper", { what: `height ${x}: answered the beacon's challenge (${asked.length} piece(s))` });
        }
      }
      upTo = h;
    } catch (e) {
      ctx.log("keeper", { what: e.message ?? String(e) });
    } finally {
      busy = false;
    }
  }
  beaconSvc.onHeight(tick);
  return { status: () => ({ ...state }) };
}
