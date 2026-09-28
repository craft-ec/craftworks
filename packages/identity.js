// IDENTITY, a service: this page's side of the identity delegate on the node. The delegate holds the members (device
// keys) on this device and signs for them, and keeps each app's SESSION until it logs out: a reload, another tab or a
// later visit is still logged in. The node tells the delegate which app asks, so no token lives in the page.
// Every call answers with the delegate's own answer (`{ unlocked }`, `{ wrongPin }`, `{ refused }`, ...).
export async function start(ctx) {
  const { core, glue, ask } = await ctx.require("node");
  const reg = await ask(core.frames_register_identity(), s => s.kind === "registered" || s.kind === "refused", "registering the identity delegate");
  if (reg.kind !== "registered") throw new Error(`the node refused the identity delegate: ${reg.said}`);
  ctx.log("identity ready", { what: core.identity_key().slice(0, 16) + "…" });

  const call = async ([id, frames], what, ms) => {
    const said = await ask(frames, s => s.kind === "identity" && s.answers.some(a => a.id === id), what, ms);
    return said.answers.find(a => a.id === id).answer;
  };
  return {
    // A new member on this node: its key minted here and the account's data key, handed to the delegate once, never
    // kept by the page.
    provision: (seed, did, pin, data) => call(core.frames_provision(seed, did, pin, data), "adding this node"),
    unlock: pin => call(core.frames_unlock(pin), "unlocking with the PIN"),
    lock: () => call(core.frames_lock(), "logging out"),
    who: () => call(core.frames_who(), "asking who is logged in"),
    sign: (params, seq, valueHash) => call(core.frames_sign(params, BigInt(seq), valueHash), "signing"),
    exportKey: () => call(core.frames_export(), "exporting the key"),
    // Leave to write one of the account's tables. The node may ask the person (its own prompt, which waits up to a
    // minute), so this waits longer than any other call.
    grant: tables => call(core.frames_grant(tables), `asking for ${tables.map(t => `“${t}”`).join(", ")}`, 90000),
    grants: () => call(core.frames_grants(), "listing the apps with access"),
    // The key that seals a table (generation 0): only for a site the person allowed that table.
    tableKey: table => call(core.frames_table_key(table, 0), `the key of “${table}”`),
    // The account's MLS group on this node: its state and the current epoch's secret, kept by the delegate (home only).
    mlsSave: (state, epoch, secret) => call(core.frames_mls_save(state, epoch, secret), "keeping the account's keys"),
    mlsLoad: () => call(core.frames_mls_load(), "reading the account's keys"),
    epochKeep: (epoch, secret) => call(core.frames_epoch_keep(epoch, secret), "keeping an earlier epoch's key"),
    // A table's key in an MLS epoch (-1: the newest this node holds).
    tableKeyAt: (table, epoch = -1) => call(core.frames_table_key_at(table, epoch), `the key of “${table}”`),
    revoke: (app, table) => call(core.frames_revoke(app, table), "removing an app's access"),
    publicOf: seed => glue.CraftworksCore.public_of(seed),
    // HANDOVER: ask an earlier build (`<key>:<code hash>`) for the member `pin` opens there. `{ handed }` with its keys,
    // or its refusal, or `{ missing: true }` when this node never ran that build.
    handoverFrom: async (prior, pin) => {
      const [id, frames] = core.frames_handover_from(prior, pin);
      const key = prior.split(":")[0];
      const said = await ask(
        frames,
        s => (s.kind === "identity" && s.answers.some(a => a.id === id)) || (s.kind === "delegate-missing" && String(s.delegate).includes(key)),
        "asking an earlier identity build",
        20000,
      );
      return said.kind === "identity" ? said.answers.find(a => a.id === id).answer : { missing: true };
    },
  };
}
