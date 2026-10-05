// IDENTITY, a capability: who an account IS, and this node's part in it.
// - The ACCOUNT: its DID names its KEY LOG (the id of its first event); the log's keys rotate under it, the recovery
//   words hold its current key. `accountOf(words)` says which account words hold (their own inception's, or the one
//   their whoami names) and puts its log; `readKeyLog(did)`; `changeWords(did, old, new)`.
// - The IDENTITY DELEGATE on this node: it holds the members (node keys) and signs for them, keeps each app's
//   SESSION until it logs out, and keeps the account's keys (MLS state, epoch secrets). The node tells it which app
//   asks, so no token lives in the page. Every call answers with the delegate's own answer (`{ unlocked }`, …).
export async function start(ctx) {
  // No space named: the account's own group.
  const NONE = new Uint8Array(0);
  // Ask an EARLIER build (`<key>:<code hash>`) one of the handover questions: its answer, or null (a build from before
  // the question answers nothing readable; a node that never ran it answers nothing at all).
  async function askPrior(prior, [id, frames], what) {
    const key = prior.split(":")[0];
    const said = await ask(
      frames,
      s => (s.kind === "identity" && s.answers.some(a => a.id === id || a.id === 0)) || (s.kind === "delegate-missing" && String(s.delegate).includes(key)),
      `asking an earlier identity build for ${what}`,
      30000,
    ).catch(() => ({ kind: "silent" }));
    // No answer in time (the node still loading that build — a compile can take a minute): said so, never taken for
    // "it does not know this PIN".
    if (said.kind === "silent") return { silent: true };
    return said.kind === "identity" ? said.answers.find(a => a.id === id)?.answer ?? null : null;
  }
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

  // WHICH ACCOUNT words hold: either their own inception's (an account's original words) or the one their whoami names
  // (words rotated in) — both asked AT ONCE, the first that answers wins, so the one that does not exist never holds
  // the way in up. `null`: no account has these words.
  async function heldBy(entropy) {
    const [, registerCode] = await codes();
    const plan = core.words_plan(registerCode, entropy);
    const own = readKeyLog(plan.inceptionDid).then(ok => (ok ? plan.inceptionDid : Promise.reject()));
    const named = get(plan.whoamiId, "asking which account these words hold").then(async ok => {
      const d = ok && core.whoami_did(registerCode, entropy);
      if (!d || !(await readKeyLog(d))) throw new Error("none");
      return d;
    });
    return Promise.any([own, named]).catch(() => null);
  }
  // The ACCOUNT words open: new words (`fresh`: a registration) their own inception's, made now — words the person
  // BROUGHT (`own`) refused if some account has them already (words drawn here are 128 random bits: never asked);
  // typed words the account that holds them — refused if none does (a typo never makes an empty account). The log is put (again: re-publishing a signed log keeps it on the network). `{ did, didBytes,
  // data }` — data: the data key's seed.
  async function accountOf(entropy, { fresh = false, own = false } = {}) {
    const [idlogCode, registerCode] = await codes();
    const plan = core.words_plan(registerCode, entropy);
    const held = fresh && !own ? null : await heldBy(entropy);
    if (fresh && held) throw new Error("these words already open an account: log in with them instead");
    if (!fresh && !held) throw new Error("no account has these recovery words: check them, or register to make a new account with them");
    const did = fresh ? plan.inceptionDid : held;
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

  // An epoch's key kept here (the account's group or a space's): `craftworks:keys`, so what was sealed with it and
  // could not be opened before is opened now (`storage`).
  const keysChanged = r => {
    if (r?.mlsSaved) dispatchEvent(new CustomEvent("craftworks:keys"));
    return r;
  };
  const call = async ([id, frames], what, ms) => {
    const said = await ask(frames, s => s.kind === "identity" && s.answers.some(a => a.id === id), what, ms);
    return said.answers.find(a => a.id === id).answer;
  };
  return {
    accountOf,
    readKeyLog,
    keyLogInfo,
    changeWords,
    // Whether these are the account's CURRENT words; whether any account has these words.
    wordsOpen: async (did, entropy) => (await readKeyLog(did)) && core.words_open((await codes())[0], did, entropy),
    wordsTaken: async entropy => !!(await heldBy(entropy)),
    // `fn({ old, fresh })`, called while the words change, both in hand; an error stops the change.
    onWordsChanged: f => wordsChanged.push(f),
    // A new member on this node: its key minted here and the account's data key, handed to the delegate once, never
    // kept by the page.
    provision: (seed, did, pin, data) => call(core.frames_provision(seed, did, pin, data), "adding this node"),
    unlock: pin => call(core.frames_unlock(pin), "unlocking with the PIN"),
    lock: () => call(core.frames_lock(), "logging out"),
    // This node's member forgotten (key, PIN, grants, group state, every epoch's secret): for a removed node.
    forget: () => call(core.frames_forget(), "forgetting this node's member"),
    who: () => call(core.frames_who(), "asking who is logged in"),
    sign: (params, seq, valueHash, space = NONE, table = "") => call(core.frames_sign(params, BigInt(seq), valueHash, space ?? NONE, table), "signing"),
    // Leave to write one of the account's tables. The node may ask the person (its own prompt, which waits up to a
    // minute), so this waits longer than any other call.
    grant: tables => call(core.frames_grant(tables), `asking for ${tables.map(t => `“${t}”`).join(", ")}`, 90000),
    grants: () => call(core.frames_grants(), "listing the apps with access"),
    // The key that seals a table (generation 0): only for a site the person allowed that table.
    tableKey: table => call(core.frames_table_key(table, 0), `the key of “${table}”`),
    // The account's MLS group on this node: its state and the current epoch's secret, kept by the delegate (home only).
    // `space`: a space's id (bytes) — its own group — or none: the account's.
    mlsSave: (state, epoch, secret, space = NONE) => call(core.frames_mls_save(state, epoch, secret, space), "keeping the group's keys").then(keysChanged),
    mlsLoad: (space = NONE) => call(core.frames_mls_load(space), "reading the group's keys"),
    // The space's EARLIER epochs' secrets held here (below `below`): `[[epoch, secretHex]…]` — for a member let in later.
    epochSecrets: (space, below) => call(core.frames_epoch_secrets(space, below), "the space's earlier keys").then(r => r?.epochs ?? []),
    epochKeep: (epoch, secret, space = NONE) => call(core.frames_epoch_keep(epoch, secret, space), "keeping an earlier epoch's key").then(keysChanged),
    // A table's key in an MLS epoch (-1: the newest this node holds).
    tableKeyAt: (table, epoch = -1, space = NONE) => call(core.frames_table_key_at(table, epoch, space), `the key of “${table}”`),
    revoke: (app, table) => call(core.frames_revoke(app, table), "removing an app's access"),
    // The account's INBOX key (its public half, for its card), and opening items sealed to it (the home site only).
    inboxKey: () => call(core.frames_inbox_key(), "reading the inbox key"),
    // The DID's member for spaces (the same on every device of the account): its MLS seed and keys, its credential.
    spaceMember: () => call(core.frames_space_member(), "the account's member for spaces"),
    // UPKEEP with no page open: the inbox the delegate watches at each wake-up, and what it has done.
    // With it, the page's randomness and time: the delegate has neither of its own.
    upkeepWatch: inbox => call(core.frames_upkeep_watch(inbox, crypto.getRandomValues(new Uint8Array(32)), Date.now()), "handing upkeep the inbox"),
    upkeepStatus: () => call(core.frames_upkeep_status(), "upkeep's status"),
    upkeepCodes: (bag, tail, idlog, sealed, piece, block) => call(core.frames_upkeep_codes(bag, tail, idlog, sealed, piece, block), "handing upkeep the contracts"),
    upkeepMandate: (me, spaces, spent = []) => call(core.frames_upkeep_mandate(me, JSON.stringify(spaces), JSON.stringify(spent)), "handing upkeep the mandate"),
    upkeepAck: admitted => call(core.frames_upkeep_ack(JSON.stringify(admitted)), "telling upkeep what was written"),
    inboxOpen: items => call(core.frames_inbox_open(items), "opening the inbox"),
    publicOf: seed => glue.CraftworksCore.public_of(seed),
    // HANDOVER: ask an earlier build (`<key>:<code hash>`) for the member `pin` opens there. `{ handed }` with its keys,
    // or its refusal, or `{ missing: true }` when this node never ran that build.
    // A member's KEYS from an earlier build (its group state, every epoch's secret), kept here: after `handoverFrom`,
    // so an update keeps its place in the account's group. A build from before this answers nothing it can read.
    // A member's KEYS from an earlier build, kept here: its account group (state, every epoch's secret), then each of
    // its SPACES' groups — so an update keeps its place in every group. A build from before either answers nothing it
    // can read: that part is simply not moved.
    // `{ epochsOnly }`: an OLDER build's keys GATHERED (the member came from another): only the epoch secrets this
    // build lacks — never its group state, which may be older than the one held here (a group rolled back).
    moveKeysFrom: async (prior, pin, { epochsOnly = false } = {}) => {
      const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
      // One group into this build: every epoch's secret, then its state with the newest one.
      const keepGroup = async (mls, epochs, space) => {
        for (const [e, secret] of epochs) await call(core.frames_epoch_keep(e, bytes(secret), space), "keeping an epoch's key");
        const newest = epochs[epochs.length - 1];
        if (mls && newest && !epochsOnly) await call(core.frames_mls_save(bytes(mls), newest[0], bytes(newest[1]), space), "keeping a group's keys");
      };
      const k = (await askPrior(prior, core.frames_handover_keys_from(prior, pin), "the member's keys"))?.handedKeys;
      if (k) await keepGroup(k.mls, k.epochs, NONE);
      const sp = (await askPrior(prior, core.frames_handover_spaces_from(prior, pin), "the member's spaces"))?.handedSpaces ?? [];
      for (const x of sp) await keepGroup(x.mls, x.epochs, bytes(x.space));
      return { moved: k?.epochs.length ?? 0, group: !!k?.mls, spaces: sp.length };
    },
    // RETIRE an earlier build's upkeep for this member: its PIN there, then an empty mandate — a build the member left
    // still wakes every minute and would act on the last mandate it held. `{ retired }`, or why not (`silent`: it
    // answered nothing, so no try was counted there).
    retireFrom: async (prior, pin) => {
      const u = await askPrior(prior, core.frames_unlock_from(prior, pin), "a session");
      if (!u?.unlocked) return { silent: !!u?.silent || !u, said: u?.silent || !u ? "no answer" : u?.wrongPin ? "not a member there" : JSON.stringify(u) };
      const m = await askPrior(prior, core.frames_retire_upkeep_from(prior), "retiring its upkeep");
      return m?.upkeep ? { retired: true } : { said: m?.silent ? "no answer" : JSON.stringify(m) };
    },
    handoverFrom: async (prior, pin) => (await askPrior(prior, core.frames_handover_from(prior, pin), "the member")) ?? { missing: true },
  };
}
