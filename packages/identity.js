// IDENTITY, a capability: who an account IS, and this node's part in it.
// - The ACCOUNT: its DID names its KEY LOG (the id of its first event); the log's keys rotate under it, the recovery
//   words hold its current key. `accountOf(words)` says which account words hold (their own inception's, or the one
//   their whoami names) and puts its log; `readKeyLog(did)`; `changeWords(did, old, new)`.
// - The IDENTITY DELEGATE on this node: it holds the members (node keys) and signs for them, keeps each app's
//   SESSION until it logs out, and keeps the account's keys (MLS state, epoch secrets). The node tells it which app
//   asks, so no token lives in the page. Every call answers with the delegate's own answer (`{ unlocked }`, …).
export async function start(ctx) {
  const { core, glue, ask } = await ctx.require("node");
  const reg = await ask(core.frames_register_identity(), s => s.kind === "registered" || s.kind === "refused", "registering the identity delegate");
  if (reg.kind !== "registered") throw new Error(`the node refused the identity delegate: ${reg.said}`);
  ctx.log("identity ready", { what: core.identity_key().slice(0, 16) + "…" });

  // Put one of the account's contracts (its key log, a whoami); read one by its 32-byte id (true: the core holds it).
  async function put(p, what) {
    const said = await ask(p.frames, s => (s.kind === "put" && s.key === p.id) || s.kind === "refused", what, 60000);
    if (said.kind !== "put") throw new Error(`the node refused ${what} (${said.said})`);
  }
  async function get(id, what) {
    const [hexId, frames] = core.frames_get(id);
    const said = await ask(frames, s => (s.kind === "got" || s.kind === "get-failed") && s.id === hexId, what, 30000);
    return said.kind === "got";
  }
  const codes = () => Promise.all(["idlog-wasm", "register-wasm"].map(n => ctx.require(n)));
  const logOf = async did => glue.CraftworksCore.idlog_id((await codes())[0], did);

  // The account's key log, read so the core holds it: true if the network has it.
  const readKeyLog = async did => get(await logOf(did), "reading the account's key log");

  // `{ events, changes }` of the account's key log (read now).
  async function keyLogInfo(did) {
    if (!(await readKeyLog(did))) return null;
    return core.idlog_info((await codes())[0], did);
  }

  // WHICH ACCOUNT words hold. New words (`fresh`) hold their own inception's. Typed words hold either their own
  // inception's (an account's original words) or the one their whoami names (words rotated in): both are asked AT
  // ONCE and the first that answers wins, so the one that does not exist never holds the way in up. The log is put
  // (again: re-publishing a signed log keeps it on the network). `{ did, didBytes, data }` — data: the data key's seed.
  async function accountOf(entropy, { fresh = false } = {}) {
    const [idlogCode, registerCode] = await codes();
    const plan = core.words_plan(registerCode, entropy);
    let did = plan.inceptionDid;
    if (!fresh) {
      const own = readKeyLog(plan.inceptionDid).then(ok => (ok ? plan.inceptionDid : Promise.reject()));
      const named = get(plan.whoamiId, "asking which account these words hold").then(async ok => {
        const d = ok && core.whoami_did(registerCode, entropy);
        if (!d || !(await readKeyLog(d))) throw new Error("none");
        return d;
      });
      did = await Promise.any([own, named]).catch(() => plan.inceptionDid);
    }
    const a = core.join_account(idlogCode, entropy, did);
    await put(a.log, "the account's key log");
    ctx.log("account", { what: a.did });
    return { did: a.did, didBytes: a.didBytes, data: a.data };
  }

  // CHANGE THE RECOVERY WORDS: `old` (the current words, typed again) hands the account to `fresh` (new words, shown
  // once). The log gets its two rotations, the new words their whoami. Whoever holds something sealed for the old words
  // seals it again for the new ones first (`onWordsChanged`: the `keys` capability's escrows). Afterwards the old words
  // open nothing; the account's nodes stay its members.
  const wordsChanged = [];
  async function changeWords(did, old, fresh) {
    const [idlogCode, registerCode] = await codes();
    if (!(await readKeyLog(did))) throw new Error("this account has no key log yet: log in once with its recovery words first");
    const c = core.change_words(idlogCode, registerCode, did, old, fresh);
    for (const f of wordsChanged) await f({ old, fresh });
    await put(c.log, "the account's key log");
    await put(c.whoami, "the new words' account");
    ctx.log("recovery words changed", {});
  }

  const call = async ([id, frames], what, ms) => {
    const said = await ask(frames, s => s.kind === "identity" && s.answers.some(a => a.id === id), what, ms);
    return said.answers.find(a => a.id === id).answer;
  };
  return {
    accountOf,
    readKeyLog,
    keyLogInfo,
    changeWords,
    // `fn({ old, fresh })`, called while the words change, both in hand; an error stops the change.
    onWordsChanged: f => wordsChanged.push(f),
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
