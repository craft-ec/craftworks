// TOKENS, a capability: what every token action on a page shares (TOKEN §7c) — the set-up, the token's code, READ a
// contract into the core, PUT what a step built, RUN a step to its end (read what it wants, have this node's identity
// delegate sign what it asks: `tokenSign`), and NOW (the beacon's height, every chain entry's stamp). The core
// (`tokens` in it) builds every entry; nothing here decides a token rule. Used by `editions` and the Wallet app.
//
//   const tokens = await ctx.require("tokens");
//   tokens.on                    // the page has a beacon: false, no token action anywhere
//   tokens.beacon  tokens.pay    // bytes: the beacon (the one `beacon` extends: one clock); the token editions are paid
//                                // in (null: none)
//   tokens.code  tokens.idlogCode
//   await tokens.read(idHex)  await tokens.putAll(puts)  await tokens.run(sigs => core.…(…, sigs))  await tokens.now()
//   await tokens.device()        // { member, creds }: this node and the account's credentials (what a step signs with)
//
// ON when the page has a beacon (`beacon`, from its `beacon-config`); the `tokens-config` package (`{ pay }`, a hex
// id) names the token editions are paid in.
export async function start(ctx) {
  let cfg = {};
  try {
    cfg = JSON.parse(new TextDecoder().decode(await ctx.require("tokens-config")));
  } catch {}
  const clock = (await ctx.require("beacon")).status();
  if (clock.off || !clock.id) return { on: false };
  const [node, auth, membership, code, idlogCode] = await Promise.all(["node", "auth", "membership", "token-wasm", "idlog-wasm"].map(n => ctx.require(n)));
  const { core, ask } = node;
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const hexOf = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  const beacon = bytes(clock.id);

  // READ a contract so the core holds it (`node.read`; a contract the node does not have: "read, not there").
  const read = idHex => node.read(bytes(idHex), "reading a token contract", { absent: true });
  // PUT what a step built, each answered; then FORGOTTEN by the core (read again, fresh, when next asked).
  async function putAll(puts) {
    for (const p of puts ?? []) {
      const said = await ask(p.frames, s => (s.kind === "put" && s.key === p.id) || s.kind === "refused", "putting a token contract", 60000);
      if (said.kind !== "put") throw new Error(`the node refused it (${said.said})`);
      core.take_got(hexOf(p.idBytes));
    }
  }
  // RUN a step to its end: read what it wants, have the delegate sign what it asks (one entry a round), until done.
  async function run(step) {
    const sigs = [];
    for (let round = 0; round < 16; round++) {
      const r = step(sigs);
      if (r.done) return r.done;
      if (r.want) {
        await Promise.all(r.want.map(read));
        continue;
      }
      if (r.sign) {
        const a = await auth.identity.tokenSign(r.sign);
        if (!a?.signed) throw new Error(`this node did not sign (${a?.refused ?? "no answer"})`);
        sigs.push([r.sign, bytes(a.signed)]);
        continue;
      }
      throw new Error("a step with no answer");
    }
    throw new Error("too many rounds");
  }
  // NOW: the beacon's height (every chain entry is stamped by it).
  async function now() {
    await read(clock.id);
    const n = core.beacon_height(beacon);
    if (n == null) throw new Error("the beacon is not there");
    return n;
  }
  // THIS DEVICE: the node (its member key) and the account's credentials.
  async function device() {
    const me = await auth.check();
    return { me, member: bytes(me.member), creds: await membership.credentials() };
  }

  return { on: true, beacon, pay: cfg.pay ? bytes(cfg.pay) : null, code, idlogCode, read, putAll, run, now, device, bytes, hexOf };
}
