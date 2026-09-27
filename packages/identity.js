// IDENTITY, a service: this page's side of the identity delegate on the node. The delegate holds the members (device
// keys) on this device and signs for them; this page holds only a SESSION token, random, in memory, gone with the page.
// Every call answers with the delegate's own answer (`{ unlocked }`, `{ wrongPin }`, `{ refused }`, ...).
export async function start(ctx) {
  const { core, glue, ask } = await ctx.require("node");
  const reg = await ask(core.frames_register_identity(), s => s.kind === "registered" || s.kind === "refused", "registering the identity delegate");
  if (reg.kind !== "registered") throw new Error(`the node refused the identity delegate: ${reg.said}`);
  ctx.log("identity ready", { what: core.identity_key().slice(0, 16) + "…" });

  const session = crypto.getRandomValues(new Uint8Array(32));
  const call = async ([id, frames], what) => {
    const said = await ask(frames, s => s.kind === "identity" && s.answers.some(a => a.id === id), what);
    return said.answers.find(a => a.id === id).answer;
  };
  return {
    // A new member on this device: its key minted here, handed to the delegate once, never kept.
    provision: (seed, did, pin) => call(core.frames_provision(seed, did, pin, session), "logging in with this device"),
    unlock: pin => call(core.frames_unlock(pin, session), "unlocking with the PIN"),
    lock: () => call(core.frames_lock(), "logging out"),
    who: () => call(core.frames_who(session), "asking who is logged in"),
    sign: (params, seq, valueHash) => call(core.frames_sign(session, params, BigInt(seq), valueHash), "signing"),
    exportKey: () => call(core.frames_export(session), "exporting the key"),
    publicOf: seed => glue.CraftworksCore.public_of(seed),
  };
}
