// KEYS, a capability: the account's MLS GROUP on this node — its members are the account's nodes, and its epoch secret
// is where every table key comes from. The protocol runs here, in the page's core; the identity delegate keeps this
// node's member state and each epoch's secret, and gives table keys to granted sites.
//
// THE COMMITS keep ONE order (MLS epochs admit no forks), through the `ordering` capability: the commit that moves the
// group FROM epoch e is written in EPOCH e's LOG — its own tail, under a key from e's secret, so only the nodes in the
// group at e can write it (a removed node writes no later epoch's). A commit's row carries the group info after it and
// the next epoch's secret in ESCROW (sealed to the account's encryption key: the recovery words alone reopen every
// epoch). The node that moves the group into an epoch makes that epoch's log, so every reached epoch's log exists.
//
// THE CHANNEL (the space's shared tail, sealed with a words-derived key: read before any epoch) keeps the POINTER to
// the first epoch — its escrow (`e/<epoch>`) and group info (`info`). A node joining with the words starts there and walks: each log's commit, its escrowed next secret,
// the next log — to the present.
//
// - After a registration or a words login (`auth.onJoined`, while the words are in hand): the group is made (no group
//   info yet) or joined by an external commit — no other node needs to be online.
// - After a PIN login, and whenever the page opens: the kept state is loaded and every newer commit applied.
// Only the account's home site runs it (the delegate keeps the state for the home site only).
//
//   const keys = await ctx.require("keys");
//   await keys.ready()        // { epoch, members, me } once current; null if not the home site or no group yet
export async function start(ctx) {
  const auth = await ctx.require("auth");
  const storage = await ctx.require("storage");
  // THE ACCOUNT'S GROUP on this device moves only through its lifecycle (FOUNDATION.md §3: `account`): made only by a
  // registration on a channel answered absent, joined only by walking a channel read with its group info.
  const life = await ctx.require("lifecycle");
  const { Fault, attempt } = await ctx.require("fault");
  const ME = "me";
  const lifeIs = s => life.state("account", ME).s === s;
  // The account's keys here at last: a session that was KEYLESS is open.
  life.on("account", (_, to) => to === "MEMBER" && life.state("session", ME).s === "OPEN_KEYLESS" && life.transition("session", ME, "open", { keys: "arrived" }));
  // RECORDS OF CRITICAL ACTIONS (owner 10-07: a space's key log broke and nothing said who wrote it, why or when — "why
  // are we not keeping enough log for critical action?"). PROVENANCE: what every key-log row and commit this page writes
  // carries — this device (its member key), why, when, the `keys` code's version.
  const provOf = async why => ({ by: String((await auth.check())?.member ?? "").slice(0, 16), why, at: Date.now(), v: String(ctx.shaOf?.("keys") ?? "").slice(0, 12) });
  // KEY EVENTS: each critical action of this account's keys is a STEP of its group's lifecycle (`group`: kept in the
  // account's journal, reported unless routine — one record for every process, owner 10-14), keyed by the space (the
  // account's own group: `account`).
  const keyEvent = (what, detail = {}) => life.transition("group", detail.space ?? "account", what, detail);
  const keyEvents = (spaceHex = null) => life.journal("group", spaceHex);
  // The space whose group this is (the account): its channel's name.
  const space = await ctx.require("space");
  const CHANNEL = space.tables.channel;
  // The group's commits in one agreed order (the `ordering` capability's `tail`): each in its epoch's own log.
  const ordering = await ctx.require("ordering");
  const { core, glue, newHex } = await ctx.require("node");
  const idlogCode = await ctx.require("idlog-wasm");
  // MLS is its own wasm package, loaded here only: no other page pays for it.
  const mlsGlue = await ctx.require("mls-glue");
  await mlsGlue.default({ module_or_path: await ctx.require("mls-wasm") });
  const mls = new mlsGlue.Mls();
  // The account's key log, as the core read it (mls verifies it again against the DID).
  const keyLog = async did => {
    if (!(await auth.identity.readKeyLog(did))) throw new Error("the account's key log is not on the network");
    return core.idlog_state(idlogCode, did);
  };
  const hexOf = (await ctx.require("node")).hex;
  const bytes = (await ctx.require("node")).bytes;
  const escrowKey = e => `e/${String(e).padStart(12, "0")}`;
  const parse = e => JSON.parse(e);

  let status = null; // the group as this page holds it
  const watchers = []; // told the group's status whenever it changes (`membership` gossips its members from it)
  const told = st => {
    for (const f of watchers) Promise.resolve(f(st)).catch(e => ctx.log("account keys", { what: e?.message ?? String(e) }));
  };
  let busy = Promise.resolve();

  // A GROUP's LOGS — the account's, or a space's; one set of rules for both. `g`: `mls()` (the group here), `space`
  // (its id bytes; undefined: the account), `channel` (its logs' table), `seal(epoch)` (a space's logs are sealed with that
  // epoch's own key; the account's with its channel's: undefined), `label` (for the trace).
  function logsOf(g) {
    const open = async (e, secret, known = null) =>
      ordering.open({ type: "tail", table: g.channel, prefix: "c/", owner: glue.epoch_log_public(secret), known, sealWith: await g.seal(e), space: g.space });
    // The commit that moved the group FROM epoch e (`fresh`: the log read again from the network, not as last seen) —
    // its epoch log's first entry (members' commits; where no entry is checked: the account's).
    async function commitFrom(e, secret, fresh = false) {
      const log = await open(e, secret);
      if (fresh) await ctx.background("keys: log.reread", log.reread?.());
      return log.from(e)[0]?.entry ?? null;
    }
    // EVERY ENTRY of position e, in THE ONE ORDER (`rules::ordering::order`): the epoch log's ∪ its DOOR's (a device
    // that entered by itself: `g.door`, a space's) — a reader takes the first that applies and the list allows.
    async function candidates(e, secret, fresh = false) {
      const log = await open(e, secret);
      if (fresh) await ctx.background("keys: log.reread", log.reread?.());
      const door = g.door ? await ctx.optional("keys: g.door", g.door(e, fresh), () => []) : [];
      return ordering.order([...log.entries(e), ...door]);
    }
    // COMMIT from epoch e: into e's log, with the group info after it (and, for the account, the next secret in escrow).
    // (Its entry made from the group AFTER the commit applied here — and published apart, so a space can keep it first.)
    // Its PROVENANCE names what the commit is for (`why`: `commit <add|readd|remove|refresh|join>`): whoever applies it
    // says which (the group's `applied` step) — never a bare "commit".
    async function entryOf(commit, intent = "add") {
      const st = g.mls().status();
      return JSON.stringify({ commit: hexOf(commit), info: hexOf(st.info), ...(st.escrow ? { next: hexOf(st.escrow) } : {}), ...(await provOf(`commit ${intent}`)) });
    }
    const publish = async (e, secret, entry) => ({ ...(await (await open(e, secret)).append(e, entry)), entry });
    async function commitAt(e, secret, commit, intent) {
      return publish(e, secret, await entryOf(commit, intent));
    }
    // This epoch's LOG exists: made now by the node that made the epoch (`made`: known new, nothing to ask), else found —
    // or, where that node never made it, made the first time it is looked for. `prev`: the epoch before's secret
    // (a space's HISTORY: whoever holds this epoch opens every earlier one, walking back — the account has escrow).
    // MORE THAN ONE WAY BACK (owner 10-07: one row without its prev cut a space's whole history): beside `prev` (the
    // epoch before), an epoch's open row carries `back` — the secrets of the BACK epochs before that (e-2 … e-1-BACK),
    // taken from the epoch before's own row (its prev and its back): on this branch by construction (never the
    // identity's keys, which may hold a lost fork's for the same epoch). A walk then crosses up to BACK broken rows.
    const BACK = 8;
    const isSecret = x => typeof x === "string" && x.length >= 32 && x.length % 2 === 0 && /^[0-9a-f]+$/i.test(x);
    async function backFor(e, prev) {
      if (!g.space || !prev || e < 2) return null;
      const before = await storage.log(g.channel, glue.epoch_log_public(bytes(prev)), { sealWith: glue.epoch_table_key(bytes(prev), g.channel), space: g.space });
      await before.answer?.();
      const v = parse(before.rows().find(r => r.key === "open")?.value ?? "{}");
      const links = { ...(v.back ?? {}), ...(v.prev ? { [e - 2]: v.prev } : {}) };
      const out = Object.fromEntries(Object.entries(links).filter(([k, x]) => Number(k) >= Math.max(0, e - 1 - BACK) && Number(k) <= e - 2 && isSecret(x)));
      return Object.keys(out).length ? out : null;
    }
    async function ensure(st, made, prev = null, why = made ? "made" : "looked-for") {
      const log = await storage.log(g.channel, glue.epoch_log_public(st.secret), { known: made ? false : null, sealWith: await g.seal(st.epoch), space: g.space });
      await log.answer?.();
      // NEVER A BROKEN LINK (owner 10-07: "don't let it happen in the first place"): an epoch's open row is written only
      // WITH its prev — the way back every later member walks — save epoch 0 (nothing before it). A node that does not
      // hold the epoch before (a newcomer joining, a page reading) writes nothing: the row is the maker's, or a
      // member's who caught up from the epoch before.
      // (A SPACE's only: the account's history is its escrow — its logs open as they always did.)
      if (log.absent && (prev || st.epoch === 0 || !g.space)) {
        const back = await ctx.optional("keys: backFor", backFor(st.epoch, prev), () => null);
        await log.put("open", JSON.stringify({ info: hexOf(st.info), ...(prev ? { prev } : {}), ...(back ? { back } : {}), ...(await provOf(why)) }));
        if (g.space) keyEvent("log-open-written", { space: hexOf(g.space), epoch: st.epoch, prev: !!prev, made: !!made, reason: why });
      } else if (log.absent && g.space) keyEvent("log-open-left", { space: hexOf(g.space), epoch: st.epoch, reason: why });
      return log;
    }
    // Every EARLIER epoch this group's log hands on, from `st` back: each secret kept here, so rows sealed before this
    // node joined open too. How many were kept.
    // Walked once per device: it stops at the first epoch below the top already held (an earlier walk went on from
    // there) — each step is a log read and a delegate call.
    const held = async e => !!(await ctx.optional("keys: auth.identity.tableKeyAt", auth.identity.tableKeyAt(g.channel, e, g.space), () => null))?.tableKey;
    // `whole`: walked to the start whatever is held — a node that joined another BRANCH (a repair) holds its old
    // branch's keys for those epochs, not this one's.
    async function history(st, { whole = false } = {}) {
      if (!whole && st.epoch > 0 && (await held(st.epoch - 1)) && (await held(0))) return 0;
      // Every epoch whose secret is known, from the top down: its log's open row names the epoch before (`prev`) and
      // those before that (`back`) — an epoch no row above names is a GAP, the walk going on below it where it can.
      // The NEAREST row naming an epoch decides its secret (its own epoch after's `prev` over a `back` from higher up):
      // walking down, a later link overwrites — an epoch is kept only when the walk reaches it.
      const known = new Map([[st.epoch, st.secret]]);
      let n = 0;
      let gap = null;
      for (let e = st.epoch; e >= 0; e--) {
        const secret = known.get(e);
        if (!secret) {
          gap ??= e;
          continue;
        }
        // HELD ALREADY: a walk keeps every epoch it reaches, top down — one that met this one went on below it as far
        // as the links reach. Nothing under it is read again (10-15, owner's trace: Craftworks' history has a gap at
        // epoch 15, so epoch 0 is never held and the check above never passed — 43 log reads, ~85 s, every load).
        if (!whole && e < st.epoch && (await held(e))) break;
        if (e < st.epoch) {
          await auth.identity.epochKeep(e, secret, g.space);
          n += 1;
        }
        if (e === 0) break;
        // Sealed with ITS key, derived here (the identity may hold a lost fork's key for the same epoch).
        const log = await storage.log(g.channel, glue.epoch_log_public(secret), { sealWith: g.space ? glue.epoch_table_key(secret, g.channel) : await g.seal(e), space: g.space });
        // Its ANSWER, not what is shown before it (a log shown empty while the node is still asked ends the walk there:
        // a member who joined late never reached epoch 0 — the writers bag's — and read none of the space).
        await log.answer?.();
        const v = parse(log.rows().find(r => r.key === "open")?.value ?? "{}");
        // Farthest first, `prev` last: the nearer link wins.
        const links = [...Object.entries(v.back ?? {}).map(([k, x]) => [Number(k), x]).sort((a, b) => a[0] - b[0]), [e - 1, v.prev]];
        for (const [k, x] of links) if (Number.isInteger(k) && k >= 0 && k < e && isSecret(x)) known.set(k, bytes(x));
      }
      if (gap !== null) {
        ctx.log(`${g.name ?? "space"} keys`, { what: `its history stops at epoch ${gap}: no epoch above names it — asking its admins for the rest` });
        // A member who holds it hands it over (`conversation`: a history ask to the space's admins).
        if (g.space) dispatchEvent(new CustomEvent("craftworks:history-gap", { detail: { space: hexOf(g.space), below: gap + 1 } }));
      }
      return n;
    }
    // Apply every commit newer than this node's epoch, in order, keeping every epoch passed (rows sealed then must open
    // here too). `reload(st)` first, before the first (the account: its key log as it is NOW).
    // The secrets of THIS node's branch from epoch `from` up to `st` (its own epoch logs: each one's `open` row names
    // the epoch before's secret) — kept before leaving the branch, so nothing sealed on it becomes unreadable.
    async function branch(st, from) {
      const out = [{ epoch: st.epoch, secret: hexOf(st.secret) }];
      let [e, secret] = [st.epoch, st.secret];
      while (e > from) {
        // A space's log is sealed with its epoch's key of the channel — derived from the secret here (a branch just
        // joined: the identity holds none of its keys yet).
        const sealWith = g.space ? glue.epoch_table_key(secret, g.channel) : await g.seal(e);
        const log = await storage.log(g.channel, glue.epoch_log_public(secret), { sealWith, space: g.space });
        await log.answer?.();
        const prev = parse(log.rows().find(r => r.key === "open")?.value ?? "{}").prev;
        if (!prev) break;
        e -= 1;
        secret = bytes(prev);
        out.push({ epoch: e, secret: prev });
      }
      return out.filter(x => x.epoch >= from);
    }
    // A FORK HEALED. Two nodes committing from one epoch at once each see only their own write at first (the log is
    // read where each writes); the log keeps the entries (`ordering`: each under its own key, the LOWEST is the
    // position's). Only what THIS node committed is ever reconsidered — its own changes, kept with the group BEFORE
    // them (`g.snaps`) — never a commit it received: a member who held an old epoch's key and writes into its log
    // later rolls nobody back. Lost (the log's entry is another): the branch's secrets kept (`g.keepLost`: what was
    // sealed on it stays readable, sealed over to the winner's), the group as it was before, the winner applied
    // next. Nothing is redone: what the change was for is reconciled (the acts; a refresh's mark). True when it went back.
    async function heal() {
      if (!g.snaps) return false;
      for (const s of await g.snaps()) {
        // The position's WINNER: the first entry that applies to the group as it was (a space's: by the list check too).
        const won = g.winner ? await ctx.optional("keys: g.winner", g.winner(s), () => null) : await ctx.optional("keys: commitFrom", commitFrom(s.epoch, bytes(s.secret), true), () => null);
        if (!won || won === s.entry) continue;
        ctx.log(g.label, { what: `epoch ${s.epoch}: this node's commit lost the race for it — the winner's applied, the change made again where it still applies` });
        await g.keepLost(await ctx.optional("keys: branch", branch(g.mls().status(), s.epoch + 1), () => [{ epoch: g.mls().status().epoch, secret: hexOf(g.mls().status().secret) }]));
        await g.restore(s);
        return true;
      }
      return false;
    }
    // ONE ENTRY TRIED (`g.apply`: a space's — the list check, `mls::entry_ok`; the account's group: MLS's own rule).
    // `{ applied }`, `{ skipped: why }` (every reader skips it), `{ defer: why }` (the list not read whole: nothing
    // decided from a short read — I6); throws where MLS would not apply it at all.
    const tryEntry = (m, commit) => (g.apply ? g.apply(m, commit) : (m.process(commit), { applied: true }));
    async function catchUp(reload, fresh = false) {
      const healed = fresh ? await heal() : false;
      const m = g.mls();
      if (m.status().removed) return 0; // removed: nothing after that applies
      let n = 0;
      for (;;) {
        const st = m.status();
        const cands = await candidates(st.epoch, st.secret, fresh);
        if (!cands.length) break;
        if (!n && reload) await reload(st);
        // ANOTHER's commit applied — a STEP of the group (10-14: a member stayed an epoch behind another's commit and
        // nothing said whether it read it): who wrote it and why (its row's provenance), or why it would not apply.
        const space = g.space ? hexOf(g.space) : undefined;
        let row = null;
        let failed = null;
        let deferred = null;
        for (const entry of cands) {
          const c = parse(entry);
          if (!c?.commit) continue;
          try {
            const r = tryEntry(m, bytes(c.commit));
            if (r.applied) {
              row = c;
              break;
            }
            if (r.defer) {
              deferred = r.defer;
              break;
            }
            keyEvent("entry-skipped", { space, epoch: st.epoch, writer: String(c.by ?? "").slice(0, 16), why: String(r.skipped ?? "").slice(0, 160) });
          } catch (e) {
            failed ??= e;
            keyEvent("apply-failed", { space, epoch: st.epoch, writer: String(c.by ?? "").slice(0, 16), why: String(e?.message ?? e).slice(0, 160) });
          }
        }
        if (deferred) {
          keyEvent("catch-up-deferred", { space, epoch: st.epoch, why: deferred });
          break;
        }
        if (!row) {
          if (failed) throw failed;
          break;
        }
        keyEvent("applied", { space, from: st.epoch, to: m.status().epoch, writer: String(row.by ?? "").slice(0, 16), cause: row.why ?? null });
        n += 1;
        if (m.status().removed) {
          ctx.log(g.label, { what: "this node was removed: it keeps what it could read, and gets nothing newer" });
          break;
        }
        const now = m.status();
        await auth.identity.epochKeep(now.epoch, now.secret, g.space);
      }
      if (n) ctx.log(g.label, { what: `${n} commit(s) applied: epoch ${m.status().epoch}` });
      // Gone back (a lost commit undone) counts as a change: the group as it is now is saved.
      return n || (healed ? 1 : 0);
    }
    return { commitFrom, candidates, commitAt, entryOf, publish, ensure, catchUp, history, heal, branch, backFor };
  }

  // THE ACCOUNT's group.
  // (No space: the account's group is the account's own — every identity call and tail defaults to it.)
  const account = logsOf({ mls: () => mls, space: undefined, channel: CHANNEL, seal: async () => undefined, label: "account keys" });
  const { commitFrom } = account;
  const catchUp = () =>
    account.catchUp(async st => {
      const s = await auth.check.strict();
      mls.load(s.didBytes, await keyLog(s.didBytes), st.state);
    });

  // ENROLLED with the words (K8.6 — no MLS group made or joined): a REGISTRATION or a WORDS LOGIN opens every generation
  // of the account's tree from its escrow (a words login), then writes THIS DEVICE's CREDENTIAL — its node key and its
  // member in the tree (the seed its delegate keeps), signed by the key log's current owner key — into the account's
  // devices contract. The tree then places it: the maker of the root (a registration), a placed device's reconcile, or
  // it enters by itself (`members.account`).
  auth.onJoined(({ entropy, did, node, fresh }) =>
    (busy = busy.then(async () => {
      // Only the home site keeps this device's member: anywhere else, nothing to do.
      const home = await auth.identity.deviceMember(crypto.getRandomValues(new Uint8Array(32))).catch(e => ({ refused: e?.message ?? String(e) }));
      if (!home?.deviceMember) return;
      life.transition("account", ME, "login", { fresh: !!fresh });
      try {
        await enroll({ entropy, did, node, fresh, seed: bytes(home.deviceMember.seed) });
      } catch (e) {
        if (COMING.some(lifeIs)) life.transition("account", ME, "fail", { why: String(e?.message ?? e).slice(0, 120) });
        throw e;
      }
    })));
  async function enroll({ entropy, did, node, fresh, seed }) {
    const channel = await storage.table(CHANNEL);
    // The CHANNEL as the NETWORK has it (a words login: its escrows), read to a decision first — for a words login read
    // again for up to a minute while it still shows nothing.
    const chState = (await channel.decided?.())?.s ?? "unknown";
    if (!fresh && !channel.rows().length)
      await ctx.optional("keys: attempt", attempt(
        "account keys",
        "reading the account's key channel",
        async n => {
          if (n) await ctx.background("keys: channel.reread", channel.reread?.());
          if (!channel.rows().length) throw new Fault("not-yet", "the account's key channel still empty", { d: { state: channel.state?.s ?? chState } });
        },
        { policy: { "not-yet": { tries: 7, report: "never" } } },
      ), () => null);
    life.transition("account", ME, fresh ? "create" : "walk", { fresh: !!fresh, channel: chState });
    // THE ACCOUNT TREE's GENERATIONS FROM THE WORDS (K8.5): every escrow in the channel (`g/<n>`, sealed to the
    // account's encryption key) opened with the words and kept as the account's epochs — BEFORE the walk, so every table
    // a device sealed with a generation reads here at once (fx 10-26: a second device joined by MLS held only epoch 0, and
    // the account's directory, resealed with the tree's generation, did not read: "tables: not saved").
    if (!fresh) {
      let n = 0;
      for (const r of channel.rows()) {
        const num = Number(r.key.startsWith("g/") ? r.key.slice(2) : NaN);
        if (!Number.isInteger(num) || !r.value) continue;
        let s = null;
        try {
          s = glue.CraftworksCore.account_escrow_open(entropy, bytes(r.value));
        } catch (e) {
          ctx.log("account keys", { what: `the escrow of generation ${num} does not open with these words: ${String(e?.message ?? e).slice(0, 80)}` });
          continue;
        }
        await auth.identity.epochKeep(num, s);
        n += 1;
      }
      if (n) (ctx.log("account keys", { what: `${n} generation(s) of the account's tree recovered from their escrows` }), dispatchEvent(new CustomEvent("craftworks:keys")));
    }
    // THIS DEVICE's CREDENTIAL, signed with the words.
    if (!fresh) life.transition("account", ME, "join", { channel: chState });
    await keyLog(did);
    const member = (await ctx.require("members")).deviceMember(seed);
    const cred = core.device_credential(idlogCode, did, entropy, typeof node === "string" ? bytes(node) : node, bytes(String(member)));
    await auth.identity.addDevices(did, [cred]);
    readyShared = null;
    dispatchEvent(new CustomEvent("craftworks:devices"));
    life.transition("account", ME, fresh ? "made" : "joined", { member: String(member).slice(0, 12) });
    ctx.log("account keys", { what: `this device's credential written (member ${String(member).slice(0, 12)}…): the account's tree places it` });
    ctx.background("keys: ctx.require", ctx.require("index").then(ix => ix.health({ k: "account-recovery", s: "", e: 0, why: `${fresh ? "registered" : "credentialed"}: member ${String(member).slice(0, 12)}`, d: { fresh: !!fresh, channelRows: channel.rows().length } })));
    // PLACED by the account's tree (after this turn of the queue): its reconcile.
    setTimeout(() => ctx.require("members").then(m => m.account()).then(t => t.reconcile()).catch(e => ctx.failed("account keys", e, `placing this device: ${e.message}`)));
    if (!fresh) dispatchEvent(new CustomEvent("craftworks:keys"));
    // Every account has a CARD from the start (its inbox, its devices).
    setTimeout(() => ctx.require("directory").then(d => d.publish()).catch(e => ctx.failed("account keys", e, `the card: ${e.message}`)));
  }

  // NEW WORDS: every escrow sealed again for them (the old words open nothing after the change), and from now on escrows
  // are sealed for them.
  auth.identity.onWordsChanged(({ old, fresh }) =>
    (busy = busy.then(async () => {
      const home = await auth.identity.mlsLoad();
      if (home.refused) return;
      const channel = await storage.table(CHANNEL);
      const reseal = hex => hexOf(mlsGlue.Mls.reseal_escrow(old, fresh, bytes(hex)));
      const rows = channel.rows().filter(x => x.key.startsWith("e/"));
      // The walk starts from the first escrow AS IT WAS (sealed for the old words) — read before it is sealed again —
      // at the channel's START (a repaired account: the epoch the walk of a words login begins at, `walk`).
      let e = Math.max(0, Number(channel.rows().find(x => x.key === "start")?.value ?? 0) || 0);
      const first = rows.find(x => x.key === escrowKey(e))?.value;
      for (const row of rows) await channel.put(row.key, reseal(row.value));
      // And every epoch log's escrowed next secret, walking with the old words.
      let n = rows.length;
      let secret = first ? mlsGlue.Mls.open_escrow(old, bytes(first)) : null;
      while (secret) {
        const entry = await commitFrom(e, secret);
        if (!entry) break;
        const c = parse(entry);
        // RESEALED IN PLACE: the row the entry was read from (`rules::ordering`: `c/<e>~<tag>`) — 10-15: this wrote a
        // bare `c/<e>`, which no reader takes, so the escrow stayed sealed for the old words.
        const log = await storage.log(CHANNEL, glue.epoch_log_public(secret));
        const rowKey = log.rows().find(x => x.value === entry && x.key.startsWith(glue.ordering_position_prefix("c/", e)))?.key;
        if (!rowKey) throw new Error(`epoch ${e}'s commit row not found to reseal`);
        await log.put(rowKey, JSON.stringify({ ...c, next: reseal(c.next) }));
        secret = mlsGlue.Mls.open_escrow(old, bytes(c.next));
        e += 1;
        n += 1;
      }
      mls.escrow_to(fresh);
      ctx.log("account keys", { what: `${n} escrow(s) sealed again for the new words` });
    })));

  // THE ACCOUNT ON ITS TREE (K8.6 — no MLS group): WHO ITS DEVICES ARE is the DEVICES LIST (credentials on the key
  // log's CURRENT key: `account_devices_now`); this device is the account's when its node holds one, and WRITES for it
  // once the account's tree places it (`writable`). REMOVED: its node held a credential once and holds none on the
  // current key (the key rotated without it) — it forgets everything. Only the home site holds this device's member
  // (`deviceMember` refuses elsewhere). `epoch`: the key log's `seq` (it moves with every removal).
  const devicesCode = await ctx.require("devices-wasm");
  const COMING = ["CONFIRMING_CHANNEL", "WALKING", "JOINING", "CREATING"];
  async function devicesNow(didB) {
    if (!(await auth.identity.readKeyLog(didB))) return null;
    if (!(await auth.identity.readDevices(didB))) return null;
    const r = core.account_devices_now(devicesCode, idlogCode, didB);
    return r ? JSON.parse(r) : null;
  }
  // READY, SHARED: one check, every read waiting on it, until the devices list changes (`craftworks:keys`, a login).
  let readyShared = null;
  let toldAs = "";
  let followed = null;
  const carried = new Set(); // nodes whose MLS group's credentials were carried into the devices list (once a page)
  function ready() {
    if (readyShared) return readyShared;
    const p = (busy = busy.then(async () => {
      const s = await auth.check.strict();
      if (!s) return null;
      const home = await auth.identity.deviceMember(crypto.getRandomValues(new Uint8Array(32))).catch(e => ({ refused: e?.message ?? String(e) }));
      if (!home?.deviceMember) return null; // not the home site
      // READ AGAIN when the account's devices change (another device credentialed or removed: its card moves) — fx/fy
      // 10-27, a device added after this page read the list was never a writer here, and its rows were never merged.
      if (followed !== s.did) {
        followed = s.did;
        ctx.background("keys: devices followed", ctx.require("directory").then(d => d.onDevices(s.did, () => ((readyShared = null), ctx.background("keys: devices changed", ready())))));
      }
      const d = await devicesNow(s.didBytes);
      if (!d) return null; // the devices list not read here yet: asked again on the next read
      const node = String(s.member).toLowerCase();
      let me = d.devices.findIndex(x => x.node === node);
      // A DEVICE FROM BEFORE THE TREE: its credential is in the account's MLS group (made at its join), maybe never put
      // in the devices contract (fx/fy 10-27, a v446 device: no credential there, so never current, never placed) —
      // every credential its group holds carried there once (the contract counts only those on the current key).
      if (me < 0 && !carried.has(node) && (await mlsNow())) {
        carried.add(node);
        const creds = mls.status().members.map(m => m.cred).filter(Boolean);
        if (creds.length) {
          await auth.identity.addDevices(s.didBytes, creds.map(bytes));
          ctx.log("account keys", { what: `${creds.length} credential(s) of the account's MLS group carried into its devices list` });
          const again = await devicesNow(s.didBytes);
          if (again) (d.devices = again.devices), (d.seq = again.seq), (me = d.devices.findIndex(x => x.node === node));
        }
      }
      if (me < 0) {
        const ever = [...core.account_devices_ever(devicesCode, idlogCode, s.didBytes)].map(x => String(x).toLowerCase());
        if (ever.includes(node)) return forget();
        // Not credentialed here (yet): the session KEYLESS — said, never left looking logged in.
        if (!COMING.some(lifeIs) && life.state("session", ME).s !== "OPEN_KEYLESS") life.transition("session", ME, "keyless", { why: "this device holds no credential of the account" });
        return null;
      }
      if (!COMING.some(lifeIs) && !lifeIs("MEMBER")) life.transition("account", ME, "loaded", { epoch: d.seq });
      status = { epoch: d.seq, me, members: d.devices.map((x, index) => ({ index, key: x.node, member: x.member, cred: x.cred })), removed: false };
      const as = JSON.stringify(status);
      if (as !== toldAs) (toldAs = as), told(status);
      return status;
    }));
    readyShared = p;
    // A null ("not here yet") is never kept as the answer: the next read asks again (as a space's group's, below).
    ctx.optional("keys: p.then", p.then(s => !s && readyShared === p && (readyShared = null)), () => readyShared === p && (readyShared = null));
    return p;
  }
  addEventListener("craftworks:devices", () => (readyShared = null));

  // WRITES FOR THE ACCOUNT — its card, its acts, its entries into spaces — only from a device whose
  // session is OPEN and whose account group is a MEMBER (FOUNDATION.md §3: a KEYLESS session writes none of them; on
  // 10-12 one re-put the card's key packages and asked members to welcome it again, which dropped the original devices).
  // `writable()`: true, or why not.
  async function writable() {
    if (!(await auth.check())) return "nobody is logged in";
    await ctx.optional("keys: ready", ready(), () => null);
    const a = life.state("account", ME).s;
    const s = life.state("session", ME).s;
    if (a !== "MEMBER" || s === "OPEN_KEYLESS") return `the account's keys are not held on this device (account ${a}, session ${s})`;
    // PLACED by the account's tree (K8.6): a device credentialed but not yet placed reads, and writes nothing.
    const tree = await ctx.optional("keys: the account's tree", ctx.require("members").then(m => m.account()), () => null);
    if (tree && !tree.placed()) await ctx.optional("keys: the account's tree read", tree.sync(), () => null);
    if (!tree?.placed()) return "this device is not yet placed in the account's tree";
    return true;
  }
  async function mustWrite(what) {
    const ok = await writable();
    if (ok === true) return;
    const a = life.state("account", ME).s;
    const coming = ["CONFIRMING_CHANNEL", "WALKING", "JOINING", "CREATING"].includes(a);
    throw new Fault(coming ? "not-yet" : "refused", `${what}: not written, the account's keys not held here`, { message: `${what}: not written — ${ok}`, d: { account: a, session: life.state("session", ME).s } });
  }

  // REMOVED: the group says this node is out of the account. It FORGETS its member at once — key, PIN, grants, the
  // group's state and every epoch's secret — so a lost or stolen node that comes online again gives nothing away,
  // and the app starts again logged out. The account is untouched: the recovery words bring the node back.
  async function forget() {
    life.transition("account", ME, "removed", {});
    life.transition("session", ME, "removed", {});
    storage.refuse("this node was removed from your account: it cannot change it");
    const r = await auth.identity.forget().catch(e => ({ refused: e?.message ?? String(e) }));
    ctx.log("account keys", { what: r.loggedOut ? "this node was removed from the account: it has forgotten everything it held" : `this node was removed, and could not forget: ${r.refused ?? JSON.stringify(r)}` });
    status = { removed: true, members: [], epoch: status?.epoch ?? 0 };
    if (r.loggedOut) {
      await (await ctx.require("theme")).ask(null, { text: "This node was removed from your account, so it has forgotten your account's keys. To use it again, log in with your recovery words.", ok: "Log out" });
      await auth.logout();
    }
    return status;
  }

  // REMOVE a node (lost, stolen, retired), after `membership` rotated the key log with the words and credentialed the
  // devices that stay (K8.6 — no MLS commit): the devices list read again (it no longer holds the removed one), and the
  // account's tree takes it out with one key change (its reconcile) — it reads nothing written after.
  // (Through the DELETE machine: a device removed is gone from the account's tree.)
  const remove = index => life.deleting("device", `account#${index}`, () => removeNow(index));
  async function removeNow(index) {
    const before = await ready();
    if (!before) throw new Error("the account's keys are not held on this site");
    const gone = before.members.find(m => m.index === index);
    readyShared = null;
    dispatchEvent(new CustomEvent("craftworks:devices"));
    const st = await ready();
    if (!st) throw new Error("the account's devices did not read after the removal");
    const tree = await (await ctx.require("members")).account();
    await tree.reconcile();
    ctx.log("account keys", { what: `a node removed (${String(gone?.key ?? "?").slice(0, 12)}…): key log at ${st.epoch}, ${st.members.length} node(s); the account's tree took it out` });
    // Writes onto the newest generation first (the removed node holds the old ones), then the card: the removed node no
    // longer listed as the account's (what it writes counts for nothing). (Its spaces' trees take it out as their lists
    // say: the account's devices are its writers — K8.6, no MLS group to refresh.)
    setTimeout(async () => {
      await ctx.background("keys: storage.sealNewest", storage.sealNewest());
      await ctx.require("directory").then(d => d.publish()).catch(e => ctx.failed("account keys", e, `the card: ${e.message}`));
    });
    return st;
  }

  // How many epochs are in escrow for the words.
  // Every epoch is escrowed: the first from the channel, each later one with the commit that made it.
  async function escrowed() {
    const st = await ctx.optional("keys: ready", ready(), () => null);
    return st && !st.removed ? st.epoch + 1 : 0;
  }

  // Once the account's keys are ready here: its CARD is current — its inbox and its devices' credentials.
  const cardReady = () =>
    ctx
      .require("directory")
      .then(async d => {
        const me = await space.account.strict();
        if (me) await d.publish(); // writes only what is missing or changed: the inbox, the devices' credentials
      })
      .catch(e => ctx.failed("account keys", e, `the card: ${e.message}`));
  const start = () =>
    ready()
      .then(st => st && !st.removed && setTimeout(cardReady))
      .catch(e => ctx.log("account keys", { what: e?.message ?? String(e) }));
  start();
  addEventListener("craftworks:auth", e => {
    if (!e.detail) status = null;
    else start();
  });

  // THIS DEVICE's ACCOUNT-TREE MEMBER SEED (K8.6): kept by the identity — its MLS signer's where it has one (the key its
  // credential names), else one drawn here once.
  const deviceSeed = async () => {
    const r = await auth.identity.deviceMember(crypto.getRandomValues(new Uint8Array(32)));
    if (!r?.deviceMember) throw new Error(`this device has no member in the account's tree: ${r?.refused ?? JSON.stringify(r)}`);
    return bytes(r.deviceMember.seed);
  };
  // THE ACCOUNT's LAST MLS EPOCH and its secret (K8.6: frozen — no device commits to the MLS group any more): what each
  // generation of the account's tree is also sealed with (`m/`, `members.account`), so a device from before the tree,
  // without the words, opens it. Only where this device holds the account's MLS state: loaded once, brought to the last
  // commit any device made (read only — nothing written), never saved.
  let mlsLoaded = null;
  const mlsNow = async () => {
    mlsLoaded ??= (async () => {
      const s = await auth.check.strict();
      const r = s && (await auth.identity.mlsLoad());
      if (!r?.mlsState) return false;
      mls.load(s.didBytes, await keyLog(s.didBytes), bytes(r.mlsState));
      await catchUp();
      return !mls.status().removed;
    })().catch(e => (ctx.log("account keys", { what: `the account's MLS epoch: ${e?.message ?? e}` }), (mlsLoaded = null), false));
    if (!(await mlsLoaded)) return null;
    const st = mls.status();
    return { epoch: st.epoch, secret: st.secret };
  };
  return { deviceSeed, mlsNow, ready: ctx.total("keys.ready", () => null, ready), writable, mustWrite, remove, escrowed: ctx.total("keys.escrowed", () => null, escrowed), onChange: f => watchers.push(f) };
}
