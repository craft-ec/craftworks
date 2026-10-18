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
  const { core, glue } = await ctx.require("node");
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
  const hexOf = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const escrowKey = e => `e/${String(e).padStart(12, "0")}`;
  const parse = e => JSON.parse(e);

  let status = null; // the group as this page holds it
  const watchers = []; // told the group's status whenever it changes (`membership` gossips its members from it)
  const told = st => {
    for (const f of watchers) Promise.resolve(f(st)).catch(e => ctx.log("account keys", { what: e?.message ?? String(e) }));
  };
  let busy = Promise.resolve();

  // Keep the state and the epoch's secret (the delegate); make this epoch's log (its first row: the group info), so
  // every node reading it finds it; a new group's first epoch is pointed to from the channel.
  async function keep(channel, { made = false } = {}) {
    readyShared = null; // the group moved here: the next read checks it again
    const st = mls.status();
    const r = await auth.identity.mlsSave(st.state, st.epoch, st.secret);
    if (!r.mlsSaved) throw new Error(`the identity would not keep the account's keys: ${r.refused ?? JSON.stringify(r)}`);
    // The group's FIRST escrow and info: written only while this device is CREATING the group (a registration) —
    // never because the channel, as read here, happens to lack them.
    if (st.epoch === 0 && lifeIs("CREATING") && !channel.rows().some(x => x.key === escrowKey(0))) {
      await channel.put(escrowKey(0), hexOf(st.escrow));
      await channel.put("info", hexOf(st.info));
    }
    await ensureLog(st, made);
    status = { epoch: st.epoch, me: st.me, members: st.members, removed: st.removed };
    told(status);
    return status;
  }

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
  const { commitFrom, commitAt } = account;
  const ensureLog = account.ensure;
  const catchUp = () =>
    account.catchUp(async st => {
      const s = await auth.check.strict();
      mls.load(s.didBytes, await keyLog(s.didBytes), st.state);
    });

  // MADE or JOINED with the words. A join whose epoch someone else moved first is refused by the channel's order:
  // read again and join from the newer group info.
  auth.onJoined(({ entropy, did, node, fresh }) =>
    (busy = busy.then(async () => {
      // Only the home site keeps the account's keys: anywhere else, nothing to do (and no table to ask for).
      const home = await auth.identity.mlsLoad();
      if (home.refused) return;
      life.transition("account", ME, "login", { fresh: !!fresh });
      try {
        await joinOrMake({ entropy, did, node, fresh });
      } catch (e) {
        if (["CONFIRMING_CHANNEL", "WALKING", "JOINING", "CREATING"].includes(life.state("account", ME).s)) life.transition("account", ME, "fail", { why: String(e?.message ?? e).slice(0, 120) });
        throw e;
      }
    })));
  async function joinOrMake({ entropy, did, node, fresh }) {
    const channel = await storage.table(CHANNEL);
    // The CHANNEL as the NETWORK has it, before anything is decided (10-12, the owner's node: its rows were read before
    // the node had fetched them — 0 rows — and the page made a SECOND group for the account): its first read awaited,
    // and for a words login (an account that exists) read again for up to a minute while it still shows nothing.
    // DECIDED first (FOUNDATION.md §2: asked again with the long wait while the node is silent).
    let chState = (await channel.decided?.())?.s ?? "unknown";
    // (A words login, the channel still empty: read again — the fault policy's `not-yet` tries, 10 s apart, a minute.)
    if (!fresh && !channel.rows().length)
      await ctx.background("keys: attempt", attempt(
        "account keys",
        "reading the account's key channel",
        async n => {
          if (n) await ctx.background("keys: channel.reread", channel.reread?.());
          chState = channel.state?.s ?? chState;
          if (!channel.rows().length) throw new Fault("not-yet", "the account's key channel still empty", { d: { state: chState } });
        },
        { policy: { "not-yet": { tries: 7, report: "never" } } },
      ));
    const kept = new Set();
    // How the walk went — reported whatever the outcome (`account-recovery`, to the health bag: the account's own
    // tables may still be sealed here): where it started, how far it got, why it stopped (10-12: the owner's node
    // recovered the account but not its storage, and nothing said how far the walk had come).
    const walked = { channelRows: channel.rows().length, fresh: !!fresh, info: false, escrow0: false, epochs: 0, stop: null };
    const recovery = (k, why, d = {}) =>
      ctx.background("keys: ctx.require", ctx.require("index").then(ix => ix.health({ k, s: "", e: walked.epochs, why: String(why).slice(0, 160), d: { ...walked, ...d } })));
    // The WALK, with the words: from the channel's pointer, every epoch's commit and escrowed next secret, to the
    // present — each epoch's secret kept here on the way (this node then reads what was written before it joined).
    async function walk() {
      // From the channel's START (an account repaired after a second group sealed over its first epoch's pointer: the
      // epoch its keys were made the account's again — every table sealed under it since), else the first epoch.
      let e = Math.max(0, Number(channel.rows().find(x => x.key === "start")?.value ?? 0) || 0);
      let info = channel.rows().find(x => x.key === "info")?.value;
      walked.info = !!info;
      const first = channel.rows().find(x => x.key === escrowKey(e))?.value;
      walked.escrow0 = !!first;
      walked.start = e;
      if (!first) return ((walked.stop = `no escrow for epoch ${e} in the channel`), { e, info, secret: null });
      let secret = mlsGlue.Mls.open_escrow(entropy, bytes(first));
      for (;;) {
        if (!kept.has(e) && (await auth.identity.epochKeep(e, secret)).mlsSaved) kept.add(e);
        walked.epochs = e;
        // Each epoch's log read FROM THE NETWORK (not as this new node happens to hold it: nothing yet), and an empty
        // one asked twice more before it counts as the newest — a walk cut short joins at an old epoch.
        let entry = await commitFrom(e, secret, true);
        if (!entry)
          entry = await ctx.optional("keys: attempt", attempt(
            "account keys",
            "reading an epoch's log",
            async () => (await commitFrom(e, secret, true)) ?? Promise.reject(new Fault("not-yet", "an epoch's log still empty", { d: { epoch: e } })),
            { policy: { "not-yet": { tries: 2, wait: () => 5000, report: "never" } } },
          ), () => null);
        if (!entry) return ((walked.stop = `no commit after epoch ${e} (the newest)`), { e, info, secret });
        const c = parse(entry);
        info = c.info ?? info;
        // A commit without its escrowed next secret (made before escrows, or by a build that did not write one):
        // the chain of keys breaks here — said, never a bare throw.
        if (!c.next) return ((walked.stop = `the commit after epoch ${e} carries no escrowed secret: later epochs not recoverable from the words`), { e, info, secret, broken: true });
        secret = mlsGlue.Mls.open_escrow(entropy, bytes(c.next));
        e += 1;
      }
    }
    // THE LIFECYCLE decides which way (its guards): a REGISTRATION makes the group, only on a channel the node
    // answered is not there; a WORDS LOGIN walks, only a channel read with its group info — never a second group.
    if (fresh) life.transition("account", ME, "create", { fresh: true, channel: chState });
    else {
      const info = channel.rows().some(x => x.key === "info");
      try {
        life.transition("account", ME, "walk", { fresh: false, channel: chState, info });
      } catch (e) {
        const why = `the account's key channel read ${channel.rows().length} row(s) (${chState}), no group info: not joined (never a second group)`;
        await recovery("account-recovery", `failed: ${why}`, { kind: "refused" });
        throw e;
      }
    }
    for (let round = 0; round < 4; round++) {
      const at = await walk();
      if (!fresh && !at.info) {
        const why = `the account's key channel read ${walked.channelRows} row(s), no group info: not joined (never a second group)`;
        await recovery("account-recovery", `failed: ${why}`, { kind: "refused" });
        throw new Error(why);
      }
      const [kind, commit] = mls.with_words(did, await keyLog(did), entropy, node, at.info ? bytes(at.info) : new Uint8Array(0));
      if (kind === "joined") {
        // The join's commit, in the log of the epoch it moved from: if another node moved the group first, walk on.
        if (!at.secret) throw new Error("the account's group has no escrow to join from");
        if (lifeIs("WALKING")) life.transition("account", ME, "join", { epoch: at.e });
        const r = await commitAt(at.e, at.secret, commit, "join (words)");
        if (!r.ok) continue;
      } else if (!fresh) throw new Error(`a words login was answered "${kind}", not joined: never a second group`);
      const st = await keep(channel, { made: true });
      life.transition("account", ME, kind === "joined" ? "joined" : "made", { epoch: st.epoch, nodes: st.members.length, recovered: kept.size });
      ctx.log("account keys", { what: `this node ${kind} the account's group: epoch ${st.epoch}, ${st.members.length} node(s)` });
      await recovery("account-recovery", `${kind}: group epoch ${st.epoch}, ${st.members.length} node(s), ${kept.size} earlier epoch(s) recovered; walk: ${walked.stop ?? "?"}`, { kind, groupEpoch: st.epoch, nodes: st.members.length, recovered: kept.size, broken: !!at.broken });
      if (kept.size) ctx.log("account keys", { what: `${kept.size} earlier epoch(s) recovered from escrow` });
      // JOINED with the words (a node new to the account): every table this page opened before the keys were here
      // read sealed — the name, the spaces. A FULL recovery, never half a page (10-12: the name and the spaces stayed
      // empty until a reload): they open again, and the page shows itself again, with the keys in place.
      if (kind === "joined") {
        // No reload (FOUNDATION.md §2, R3): the account turning MEMBER reopens every table that did not read here, and
        // the page shows itself again with the keys in place.
        ctx.log("account keys", { what: "recovered: every table that did not read here opens again with this node's keys in place" });
        dispatchEvent(new CustomEvent("craftworks:keys"));
      }
      // Every account has a CARD from the start (its inbox, its devices): nobody who wants to write to it ever waits on
      // one that does not exist. After this turn of the queue.
      setTimeout(() => ctx.require("directory").then(d => d.publish()).catch(e => ctx.failed("account keys", e, `the card: ${e.message}`)));
      return;
    }
    throw new Error("the account's group kept moving: joining it again next time");
  }

  // REPAIR (owner 10-13): THIS LOGIN's keys made the account's again. 10-12 a words login on a slow node read the
  // account's channel empty and made a SECOND group: it wrote its own first escrow and group info over the channel's,
  // so every words login since joins that second group — and it sealed the account's directory over with its own key,
  // so not even the first group reads it. Run where the first group's keys are (the original login, its own PIN):
  // the directory made again (`storage.repairDirectory`), every table this node writes sealed under the current epoch
  // (`storage.resealOwn`), and the channel pointed at this group from that epoch (`start`, its escrow, its group
  // info) — a words login anywhere then joins this group and reads everything. `say(line)`: each step, as it goes.
  async function repairAccount(say = () => {}) {
    const held = await ready();
    if (!held) throw new Error("this login holds no keys of the account's group here: log in where its keys are (the original login, its own PIN)");
    // ONLY on the login that HOLDS THE ACCOUNT'S DATA — its spaces read here (a second group's logins read none of
    // them). Whether the directory reads does not tell them apart (the owner's account, 10-13: its original login
    // reads it, the second group's do not; a fork that sealed it over the other way round on fx).
    const list = await (await ctx.require("space")).mine.strict();
    if (list.complete === false) throw new Error("your spaces are still being read here: try again in a moment");
    if (!list.some(s => s.kind === "server")) throw new Error("no space of yours reads on this login: run it on the login where your spaces show (the original one)");
    const st = mls.status();
    say(`this login's group: epoch ${st.epoch}, ${st.members.length} node(s); ${list.filter(s => s.kind === "server").length} space(s) read here`);
    // Every node of this group keeps its feeds listed (a node's key is in its credential: did ‖ owner ‖ node).
    const nodes = st.members.map(m => (m.cred ? JSON.parse(glue.credential_parts(bytes(m.cred)) ?? "null")?.node : null)).filter(Boolean);
    // THE DIRECTORY: made again only where it does not read here (sealed over by the other group); where it reads, kept,
    // and every node of this group listed in it.
    const dir = await storage.directoryState();
    if (dir.s === "unreadable") say(`the directory made again: ${(await storage.repairDirectory(nodes)).length} row(s)`);
    else if (dir.s === "present") say(`the directory reads here: kept (${(await storage.listNodes(nodes)).length} node(s) listed)`);
    else throw new Error("your account's directory is not read yet here: try again in a moment");
    const sealed = await storage.resealOwn();
    say(`${sealed.length} table(s) of this node sealed under epoch ${st.epoch}`);
    const channel = await storage.table(CHANNEL);
    await channel.put(escrowKey(st.epoch), hexOf(st.escrow));
    await channel.put("info", hexOf(st.info));
    await channel.put("start", String(st.epoch));
    say(`the account's channel points at this group from epoch ${st.epoch}: a words login now joins it`);
    await keyEvent("repaired", { epoch: st.epoch, tables: sealed.length });
    dispatchEvent(new CustomEvent("craftworks:keys"));
    return { epoch: st.epoch, tables: sealed.length };
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

  // Loaded (after a PIN login) and brought current.
  // READY, SHARED as a space's group's is: one check, every read waiting on it, until the group moves (its current
  // epoch's log changes, or this page changes it: `keep`).
  let readyShared = null;
  let readyLog = null;
  function ready() {
    if (readyShared) return readyShared;
    const p = (busy = busy.then(async () => {
      const s = await auth.check.strict();
      if (!s) return null;
      if (!status) {
        const r = await auth.identity.mlsLoad();
        // Not joined here (logged in, the keys not held): the session is KEYLESS — said, never left looking logged in.
        if (!r.mlsState && !r.refused && !lifeIs("CONFIRMING_CHANNEL") && !lifeIs("WALKING") && !lifeIs("JOINING") && !lifeIs("CREATING"))
          if (life.state("session", ME).s !== "OPEN_KEYLESS") life.transition("session", ME, "keyless", { why: "the account's keys are not held on this node" });
        if (!r.mlsState) return null; // not the home site, or this node has not joined yet
        mls.load(s.didBytes, await keyLog(s.didBytes), bytes(r.mlsState));
      }
      if (!lifeIs("MEMBER") && !lifeIs("REMOVED")) life.transition("account", ME, "loaded", { epoch: mls.status().epoch });
      const channel = await storage.table(CHANNEL);
      const applied = await catchUp();
      if (mls.status().removed) return forget();
      if (applied) await keep(channel);
      else {
        const st = mls.status();
        const log = await ensureLog(st, false);
        if (log && readyLog !== log) (readyLog = log), log.onChange?.(() => readyLog === log && (readyShared = null));
        status ??= (({ epoch, me, members, removed }) => ({ epoch, me, members, removed }))(st);
      }
      return status;
    }));
    readyShared = p;
    // A null ("not here yet") is never kept as the answer: the next read asks again (as a space's group's, below).
    ctx.optional("keys: p.then", p.then(s => !s && readyShared === p && (readyShared = null)), () => readyShared === p && (readyShared = null));
    return p;
  }

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
    status = { removed: true, members: [], epoch: mls.status().epoch };
    if (r.loggedOut) {
      await (await ctx.require("theme")).ask(null, { text: "This node was removed from your account, so it has forgotten your account's keys. To use it again, log in with your recovery words.", ok: "Log out" });
      await auth.logout();
    }
    return status;
  }

  // REMOVE a node (lost, stolen, retired): an MLS removal, committed in the group's order. The group moves to a new
  // epoch; the removed node reads nothing written from then on. If another node moved the group first, this node's
  // view is taken back to what it kept and brought current, and the removal must be asked again.
  // (Through the DELETE machine: a device removed is gone from the account's group.)
  const remove = index => life.deleting("device", `account#${index}`, () => removeNow(index));
  async function removeNow(index) {
    return (busy = busy.then(async () => {
      if (!status) throw new Error("the account's keys are not held on this site");
      const channel = await storage.table(CHANNEL);
      const from = mls.status();
      const commit = mls.remove(Uint32Array.from([index]));
      const r = await commitAt(from.epoch, from.secret, commit, "remove (device)");
      if (!r.ok) {
        const kept = await auth.identity.mlsLoad();
        const s = await auth.check.strict();
        mls.load(s.didBytes, await keyLog(s.didBytes), bytes(kept.mlsState));
        await catchUp();
        await keep(channel);
        throw new Error("the account's group moved meanwhile: look again and remove it again");
      }
      const st = await keep(channel, { made: true });
      ctx.log("account keys", { what: `a node removed: epoch ${st.epoch}, ${st.members.length} node(s)` });
      // Every space this account is in: its member's keys refreshed, so the removed node — which held them — follows
      // nothing there from now on (after this turn: each space's queue, not the account's).
      setTimeout(async () => {
        // Writes onto the new epoch first (the removed node holds the old ones), then the card: the removed node no
        // longer listed as the account's (what it writes counts for nothing).
        await ctx.background("keys: storage.sealNewest", storage.sealNewest());
        await ctx.require("directory").then(d => d.publish()).catch(e => ctx.failed("account keys", e, `the card: ${e.message}`));
        for (const sp of await space.mine()) {
          const g = group(sp);
          if (!(await g.ready())) continue;
          await g.refresh().catch(e => ctx.log(`${sp.name ?? "space"} keys`, { what: `refreshing after the removal: ${e.message}` }));
        }
      });
      return st;
    }));
  }

  // THE DID's MEMBER in spaces (ARCHITECTURE: a DID is the member, a device only signs in): its keys from the identity
  // (the same on every device of the account); each space's group state in the account's table `spacekeys` (`s/<space>`,
  // the newest any device saved).
  const SPACEKEYS = "spacekeys";
  const newId = () => hexOf(crypto.getRandomValues(new Uint8Array(8)));
  let memberKeys = null;
  const keysOfMember = () =>
    (memberKeys ??= auth.identity.spaceMember().then(r => {
      if (!r.spaceMember) throw new Error(`no member for spaces here: ${r.refused ?? JSON.stringify(r)}`);
      return r.spaceMember;
    })).catch(e => {
      memberKeys = null;
      throw e;
    });
  // THE TABLE IN USE: `spacekeys`, or the one the account's catalog NAMES (`upkeep` mark `spacekeys`) — set by hand
  // only, never by a reading; `spacekeys-2` … `-9` (the site declares them all: a move needs no site change) (owner
  // 10-07: the owner's `spacekeys` lost a tree block; a fresh table for that account,
  // filled again as its devices enter each space again). Read once per page.
  let keysName = null;
  const spacekeys = async () => {
    keysName ??= ctx.optional("keys: storage.upkeepMark", storage.upkeepMark("spacekeys").then(n => (typeof n === "string" && /^spacekeys(-[2-9])?$/.test(n) ? n : SPACEKEYS)), () => SPACEKEYS);
    const t = await storage.table(await keysName);
    await t.settled;
    return t;
  };
  // THE DID's MEMBER for spaces (its signing key and credential: the same on every device of the account).
  const memberWith = async () => {
    const k = await keysOfMember();
    return new mlsGlue.SpaceMember(bytes(k.seed), bytes(k.credential));
  };

  // A SPACE's GROUP, as this DID's member (a server, a chat): made by its first member, or ENTERED by itself; its
  // state the ACCOUNT's (the table `spacekeys`, row `s/<space>`: the newest any device saved), brought current through
  // the space's epoch logs — each sealed with its epoch's own key. This device keeps each epoch's secret it holds (its
  // identity signs and seals with them). A commit this DID made on another device is not processed here (MLS: a
  // member's own): the state that device saved is loaded instead.
  const groups = new Map();
  // EACH SPACE's LIST as its replay has it (`roles` hands it here — never read by the group itself: the acts are read
  // with the group's keys): `{ roster: [did], whole, admits(did, aadHex) }`. What every entry is checked against.
  const lists = new Map();
  function setList(id, l) {
    const had = lists.get(id);
    lists.set(id, l);
    const changed = !had || had.whole !== l.whole || had.roster.join() !== l.roster.join() || had.gen !== l.gen;
    if (changed) groups.get(id)?.listChanged();
  }
  function group(sp) {
    if (groups.has(sp.id)) return groups.get(sp.id);
    let m = null;
    let st = null;
    let queue = Promise.resolve();
    const ch = sp.tables.channel;
    const at = `s/${sp.id}`;
    // SNAPSHOTS: the group BEFORE each commit applied here, by epoch — `<at>@<epoch>` in the account's `spacekeys`, the
    // last few kept (a commit that lost the race for its epoch is undone from one: `logs.heal`).
    const KEEP = 6;
    const snapKey = e => `${at}@${String(e).padStart(12, "0")}`;
    let refreshTried = false; // a refresh owed: tried once a page (it may fail until the group is current)
    const keepSnap = async (from, entry) => {
      const t = await spacekeys();
      await t.put(snapKey(from.epoch), JSON.stringify({ epoch: from.epoch, secret: hexOf(from.secret), state: hexOf(from.state), entry }));
      for (const k of t.rows().filter(r => r.key.startsWith(`${at}@`) && r.value).map(r => r.key).sort().slice(0, -KEEP)) await t.remove(k);
    };
    // The LOST branches' epoch secrets (`<at>~lost`): read with, as alternates (`storage`), never written with.
    // The LOST branches' epoch secrets (`<at>~lost`): read with, as alternates (`storage`), never written with.
    // A BRANCH's FINGERPRINT at epoch `e`: its log's public key (from that epoch's secret) — the same on every node of
    // one branch, another on another. This node's, from its own logs (null: an epoch it has not reached).
    async function fingerprint(e) {
      const s = m?.status();
      if (!s || e > s.epoch) return null;
      const list = e === s.epoch ? [{ epoch: s.epoch, secret: hexOf(s.secret) }] : await logs.branch(s, e);
      const x = list.find(l => l.epoch === e);
      return x ? glue.epoch_log_public(bytes(x.secret)) : null;
    }
    // ANNOUNCED in the space's writers list (`{ w, epoch, branch }`, once per epoch): which branch this device is on —
    // what a member's state is judged against (the owner's branch), and the info a device with no key enters from.
    // Once: the epoch claimed before anything is awaited (many reads at once announce nothing twice), and nothing
    // written when the list holds that very entry already (a page opened again).
    // …with the GROUP INFO of that epoch (`ih`: its hash) put in the epoch's door in parts (a bag item holds 16 KiB): what
    // a listed device with no current key enters from (FOUNDATION "Members and keys" 4).
    let announced = -1;
    const PART = 12000; // hex characters per part
    const digest = async b => hexOf(new Uint8Array(await crypto.subtle.digest("SHA-256", b)));
    // Bounded (a door holds 256 KiB, review 10-17): the OWNER's always (the info a device with no key enters from —
    // its owner proven by the space's id); another member's only where no whole info is there yet.
    async function publishInfo(index, s) {
      const info = s.info;
      const ih = await digest(info);
      const items = await index.spacePointers(sp, doorName(s.epoch));
      const have = items.filter(x => x?.k === "info" && x.d === ih);
      const whole = d => { const ps = items.filter(x => x?.k === "info" && x.d === d); return ps.length && new Set(ps.map(x => x.i)).size === ps[0].n; };
      const owner = (await space.account())?.id === sp.governance?.owner;
      if (!owner && !have.length && items.some(x => x?.k === "info" && whole(x.d))) return null;
      const hex = hexOf(info);
      const n = Math.ceil(hex.length / PART);
      for (let i = 0; i < n; i++)
        if (!have.some(x => x.i === i && x.n === n)) await index.spacePoint(sp, doorName(s.epoch), { k: "info", e: s.epoch, d: ih, i, n, x: hex.slice(i * PART, (i + 1) * PART) });
      return ih;
    }
    async function announce() {
      const s = m?.status();
      if (!s || s.removed || s.epoch === announced || !sp.self) return;
      announced = s.epoch;
      const branch = glue.epoch_log_public(s.secret);
      const index = await ctx.require("index");
      try {
        const ih = await publishInfo(index, s);
        // (Only an entry every member reads counts: one sealed with an epoch is read by those who hold it alone.)
        const had = (await index.spacePointers(sp, "writers")).some(p => p?.w === sp.self && p.epoch === s.epoch && p.branch === branch && (p.ih ?? null) === ih && p.sealedAt === index.OPEN);
        if (!had) await index.spacePoint(sp, "writers", { w: sp.self, epoch: s.epoch, branch, ...(ih ? { ih } : {}), at: Date.now() });
      } catch (e) {
        announced = -1;
        throw e;
      }
    }
    // THE GROUP INFO of epoch `e` whose hash is `ih`, from its door's parts (null: not all there).
    async function infoAt(e, ih) {
      const parts = (await doorItems(e)).filter(x => x?.k === "info" && x.e === e && x.d === ih && Number.isInteger(x.i) && typeof x.x === "string");
      const n = parts[0]?.n;
      if (!n) return null;
      const byI = new Map(parts.filter(x => x.n === n).map(x => [x.i, x.x]));
      if (byI.size !== n) return null;
      const info = bytes([...Array(n).keys()].map(i => byI.get(i)).join(""));
      return (await digest(info)) === ih ? info : null;
    }
    // EVERY SWITCH of the group keeps the branch it leaves: its epoch secrets that the group now on does not share
    // (the epochs where the two differ) — kept as lost, read with as alternates. Nothing is ever sealed under a key
    // no device keeps.
    const keepLeft = async wasList => {
      if (!wasList.length || !m) return;
      const from = Math.min(...wasList.map(l => l.epoch));
      const now = await ctx.optional("keys: logs.branch", logs.branch(m.status(), from), () => [{ epoch: m.status().epoch, secret: hexOf(m.status().secret) }]);
      const left = wasList.filter(l => !now.some(n => n.epoch === l.epoch && n.secret === l.secret) && now.some(n => n.epoch === l.epoch));
      if (left.length) await keepLostHere(left);
    };
    const keepLostHere = async list => {
      const t = await spacekeys();
      const had = JSON.parse(t.rows().find(r => r.key === `${at}~lost`)?.value ?? "[]");
      const all = [...had, ...list.filter(l => !had.some(h => h.secret === l.secret))];
      await t.put(`${at}~lost`, JSON.stringify(all));
      ctx.log(`${sp.name ?? "space"} keys`, { what: `${list.length} epoch key(s) of a lost branch kept (what was written there stays readable)` });
    };
    // THE DOOR of epoch e (FOUNDATION "Members and keys" 4): the entries of devices that entered by itself there.
    const doorName = e => `door-${e}`;
    const doorItems = async e => ctx.require("index").then(ix => ix.spacePointers(sp, doorName(e)));
    // THE LIST CHECK of one entry (`mls::entry_ok` through the MLS package): with the list the replay handed over.
    const apply = (mm, commit) => {
      const l = lists.get(sp.id);
      const r = mm.apply_checked(commit, l?.check ?? { listed: () => false, admits: () => false, mayTakeOut: () => false });
      if (r.applied) return r;
      if (!l || !l.whole) {
        // (The list asked for: `roles` reads the acts with the keys held so far and hands it here.)
        if (!l) ctx.background("keys: roles.of", ctx.require("roles").then(x => x.of(sp)));
        return { defer: l ? "the list was not read whole: nothing decided from a short read" : "the list not read yet" };
      }
      return { skipped: r.why };
    };
    const logs = logsOf({
      mls: () => m,
      space: sp.idBytes,
      channel: ch,
      door: async e => (await doorItems(e)).filter(x => x?.k === "enter" && x.e === e && typeof x.entry === "string").map(x => x.entry),
      apply,
      // The WINNER of the position this device committed at (`logs.heal`): the first entry that applies to its group then.
      winner: async s => {
        for (const entry of await logs.candidates(s.epoch, bytes(s.secret), true)) {
          const copy = mlsGlue.Mls.load_space(sp.idBytes, bytes(s.state));
          try {
            const r = apply(copy, bytes(parse(entry).commit));
            if (r.applied) return entry;
            if (r.defer) return null;
          } catch {}
        }
        return null;
      },
      seal: async e => (await auth.identity.tableKeyAt(ch, e, sp.idBytes)).tableKey,
      label: `${sp.name ?? "space"} keys`,
      keepLost: list => keepLostHere(list),
      snaps: async () =>
        (await spacekeys())
          .rows()
          .filter(r => r.key.startsWith(`${at}@`) && r.value)
          .map(r => JSON.parse(r.value))
          .sort((a, b) => a.epoch - b.epoch),
      // Back to the group before epoch `s.epoch`'s commit; every snapshot after it dropped (that branch lost). Its
      // change not redone (reconciled: the acts, a refresh's mark).
      restore: async s => {
        m = mlsGlue.Mls.load_space(sp.idBytes, bytes(s.state));
        const t = await spacekeys();
        for (const r of t.rows().filter(r => r.key.startsWith(`${at}@`) && r.value)) if (JSON.parse(r.value).epoch >= s.epoch) await t.remove(r.key);
        status(m.status());
      },
    });
    // The shared check (`ready`), and the epoch log it follows; any change of the group's state ends the shared one.
    let shared = null;
    let followed = null;
    const credsOf = x => (x?.members ?? []).map(m => m.cred ?? "").join();
    const status = s => {
      const changed = !st || st.epoch !== s.epoch || st.removed !== s.removed || st.members?.length !== s.members?.length;
      if (changed) shared = null;
      const moved = !!st && credsOf(st) !== credsOf(s);
      st = { epoch: s.epoch, me: s.me, members: s.members, removed: s.removed };
      // ITS MEMBERS CHANGED (someone entered, someone taken out): said AFTER the status above is the group's — every
      // table of the space finds its new writers by it (fx 10-17: the identity's signal came inside the save, before
      // this; the tables read the old members and a new member's writing stayed unread for minutes).
      if (moved) setTimeout(() => dispatchEvent(new CustomEvent("craftworks:keys", { detail: { space: sp.id } })));
      return st;
    };
    // SAVE: this epoch's secret on this device, the state for the account's devices, the epoch's log.
    async function save(made, prev = null, why = null) {
      const s = m.status();
      const k = await auth.identity.epochKeep(s.epoch, s.secret, sp.idBytes);
      if (!k.mlsSaved) throw new Error(`the identity would not keep the space's key: ${k.refused ?? JSON.stringify(k)}`);
      await (await spacekeys()).put(at, JSON.stringify({ epoch: s.epoch, state: hexOf(s.state) }));
      await logs.ensure(s, made, prev, why ?? (made ? "made" : "looked-for"));
      return status(s);
    }
    // LOAD the account's newest state of this space, where it is newer than this device's (or this device has none):
    // its secret kept here, and — on a device that never held this space — every earlier epoch its log hands on.
    async function load() {
      const v = (await spacekeys()).rows().find(r => r.key === at)?.value;
      if (!v) return false;
      const r = JSON.parse(v);
      if (m && m.status().epoch >= r.epoch) return false;
      const first = !m;
      // The branch this device is on, before it is replaced: a newer state saved by another device of the account
      // (or by upkeep) may be ANOTHER branch — what was sealed on this one must stay readable.
      const was = m ? m.status() : null;
      const wasList = was ? await ctx.optional("keys: logs.branch", logs.branch(was, Math.max(0, was.epoch - 8)), () => [{ epoch: was.epoch, secret: hexOf(was.secret) }]) : [];
      m = mlsGlue.Mls.load_space(sp.idBytes, bytes(r.state));
      const s = m.status();
      await auth.identity.epochKeep(s.epoch, s.secret, sp.idBytes);
      await keepLeft(wasList);
      if (first) {
        const n = await ctx.optional("keys: logs.history", logs.history(s), () => 0);
        if (n) ctx.log(`${sp.name ?? "space"} keys`, { what: `${n} earlier epoch(s) kept on this device` });
      }
      status(s);
      return true;
    }
    // A COMMIT KEPT BUT NOT KNOWN TO BE IN THE LOG (`change`'s write-ahead; the page died between): the log's entry
    // for its epoch read again — none: published now; this one: done; another's: this one lost — the group before it
    // taken back (`logs.heal`) and the winner applied.
    async function finishPending() {
      const t = await spacekeys();
      const v = t.rows().find(r => r.key === `${at}~pending`)?.value;
      if (!v || !m) return;
      const p = JSON.parse(v);
      const snap = t.rows().find(r => r.key === snapKey(p.epoch))?.value;
      const before = snap ? JSON.parse(snap) : null;
      if (!before?.secret) return void (await t.remove(`${at}~pending`));
      const won = await ctx.background("keys: logs.commitFrom", logs.commitFrom(p.epoch, bytes(before.secret), true));
      if (won === undefined) return; // not read: asked again next time
      if (won === null) {
        const r = await logs.publish(p.epoch, bytes(before.secret), p.entry);
        if (!r.ok) return void (await logs.heal());
        ctx.log(`${sp.name ?? "space"} keys`, { what: `epoch ${p.epoch}: this device's change, kept but not sent when the page closed, sent now` });
        keyEvent("pending-published", { space: hexOf(sp.idBytes), epoch: p.epoch });
      } else if (won !== p.entry) {
        await logs.heal();
        await t.remove(`${at}~pending`);
        return;
      }
      if (m.status().epoch === p.epoch + 1) await save(true, before.secret, "commit");
      await t.remove(`${at}~pending`);
    }
    // BROUGHT CURRENT: every newer commit applied (tried again by the fault policy — its table read again first — when
    // it would not: `fault.attempt`).
    async function current(fresh = false) {
      await finishPending().catch(e => ctx.log(`${sp.name ?? "space"} keys`, { what: `finishing a kept change: ${e?.message ?? e}` }));
      return attempt(
        `${sp.name ?? "space"} keys`,
        "bringing the space's keys current",
        async n => {
          if (n) await (await spacekeys()).reread?.();
          for (;;) {
            try {
              // CAUGHT UP one epoch: the epoch before's secret held here is the new log's PREV (owner 10-07: an open row
              // with no prev breaks every later member's way back). More than one: the logs between were read, not made here.
              const before = m?.status();
              if (await logs.catchUp(undefined, fresh)) {
                const now = m.status();
                return save(false, before && now.epoch === before.epoch + 1 ? hexOf(before.secret) : null, "catch-up");
              }
              return status(m.status());
            } catch (e) {
              // STUCK behind its OWN commit (10-07, Ivvor: a commit this device wrote reached the log, its state after
              // it was never saved — the tab closed): MLS never applies a member's own message. Its own NODE's commit (the
              // follower) is taken; anything else: this device ENTERS AGAIN by itself (FOUNDATION "Members and keys" 1).
              if (/CantProcessMessageFromSelf/.test(String(e?.message ?? e))) {
                const mine = ((await ctx.optional("keys: upkeepStatus", auth.identity.upkeepStatus(), () => null))?.upkeep?.groups ?? []).find(x => x.space === hexOf(sp.idBytes));
                if (mine && (await adoptNow(mine.epoch, mine.state))) continue;
                keyEvent("stuck-own-commit", { space: hexOf(sp.idBytes), epoch: m.status().epoch });
                return enterNow("stuck");
              }
              // A newer state another device of this account saved: taken, and on.
              if (await load()) continue;
              throw new Fault("transient", "the space's keys would not come current", { message: String(e?.message ?? e), cause: e });
            }
          }
        },
        { policy: { transient: { tries: 4, wait: () => 1500, report: "never" } } },
      );
    }
    // A change of the group (add, remove): its commit in the log of the epoch it moves from, then saved. Lost to
    // another change first: this device takes the newer group and says so.
    // WRITE-AHEAD (10-07, Ivvor: a commit reached the log and the state after it was never saved — the tab closed —
    // and MLS never applies a member's own commit, so that device could never catch up): BEFORE the commit goes out,
    // the state after it, the group before it (`keepSnap`) and the commit itself (`~pending`) are kept; the pending
    // mark goes once it is in the log. A page that dies between finishes it on its next load (`finishPending`).
    async function change(make, intent = null) {
      if (!m) throw new Error("this account is not in the space's group");
      const from = m.status();
      const [commit, out] = make();
      const entry = await logs.entryOf(commit, intent?.kind ?? "add");
      const t = await spacekeys();
      await keepSnap(from, entry);
      await t.put(`${at}~pending`, JSON.stringify({ epoch: from.epoch, entry }));
      await t.put(at, JSON.stringify({ epoch: m.status().epoch, state: hexOf(m.status().state) }));
      const r = await logs.publish(from.epoch, from.secret, entry);
      if (!r.ok) {
        // Lost the race for the epoch: back to the group before it, saved as it was.
        await t.put(at, JSON.stringify({ epoch: from.epoch, state: hexOf(from.state) }));
        await t.remove(`${at}~pending`);
        await t.remove(snapKey(from.epoch));
        m = null;
        await load();
        await ctx.background("keys: current", current());
        throw new Error("the group moved meanwhile: try again");
      }
      await save(true, hexOf(from.secret), "commit");
      await t.remove(`${at}~pending`);
      keyEvent("commit", { space: hexOf(sp.idBytes), from: from.epoch, to: m.status().epoch, intent: intent?.kind ?? "add", ...(intent?.q ? { q: intent.q } : {}) });
      return out;
    }
    // THE GROUP THIS DEVICE's NODE MOVED (its follower: the same device, the same entry — MLS never applies a member's
    // own commit, so it is TAKEN, never processed): kept here, unqueued (called inside the queue too).
    async function adoptNow(epoch, stateHex) {
      // Taken only if it leads past this device's own copy (the guard "leads past").
      if (m && glue.guard_leads_past(epoch, m.status().epoch)) return false;
      // The group as it was here, kept with the commit upkeep wrote from it (its epoch log's): undone like any lost
      // commit if the log's winner turns out another (`logs.heal`).
      if (m) {
        const was = m.status();
        const entry = await ctx.optional("keys: logs.commitFrom", logs.commitFrom(was.epoch, was.secret, true), () => null);
        if (entry) await keepSnap(was, entry);
      }
      await (await spacekeys()).put(at, JSON.stringify({ epoch, state: stateHex }));
      await load();
      ctx.log(`${sp.name ?? "space"} keys`, { what: `the group upkeep moved (someone let in): epoch ${epoch}` });
      keyEvent("adopt-delegate", { space: hexOf(sp.idBytes), epoch });
      return true;
    }
    // ENTER BY ITSELF (FOUNDATION "Members and keys" 1) — the ONE way a device of a LISTED account gets the space's
    // keys, whenever it cannot read (never held, another branch, stuck): an MLS external commit from the newest group
    // info a listed member published, its own old entry out in the same commit, put in that epoch's door. Nobody answers;
    // nobody else need be online. `proof`: a self-entry's (`{ join: "code", p } | { join: "open" } | { join: "level" }`
    // — `gov::Gov::admits`), for one not on the list yet. Every reader checks it against the list.
    async function enterNow(why, proof = null) {
      const index = await ctx.require("index");
      const l = lists.get(sp.id);
      const owner = sp.governance?.owner ?? null;
      // NEVER ON THE LIST (read whole), never held here, and no proof: nothing to enter — every reader would skip it. (A
      // device that held the group enters again whatever its own view of the list says: its own `join` act may not be
      // counted here yet — fx 10-17.)
      const me = (await space.account())?.id;
      if (!proof && !m && l?.whole && me && !l.check.listed(me)) {
        keyEvent("enter-not-listed", { space: hexOf(sp.idBytes), why });
        throw new Error(`${sp.name ?? "this space"}: not on its list — nothing to enter`);
      }
      // The newest epochs announced with their info. AUTHENTIC only (review 10-17: anyone who knows the space's id can
      // write its open bags): signed by its OWNER (proven by the space's id) — or, the list read whole, by a listed
      // member; NEVER this account's own (its own fork, entered again and again).
      const authentic = signer => signer !== me && (signer === owner || (!!l?.whole && l.check.listed(signer)));
      const anns = (await index.spacePointers.strict(sp, "writers")).filter(p => p?.ih && Number.isInteger(p.epoch));
      const tried = new Set();
      for (const a of anns.sort((x, y) => y.epoch - x.epoch)) {
        if (tried.has(`${a.epoch}|${a.ih}`) || tried.size >= 6) continue;
        tried.add(`${a.epoch}|${a.ih}`);
        const info = await ctx.optional("keys: infoAt", infoAt(a.epoch, a.ih), () => null);
        if (!info) continue;
        const f = mlsGlue.Mls.info_facts(info);
        if (f.epoch !== a.epoch || !authentic(f.signer)) continue;
        const was = m && !m.status().removed ? m.status() : null;
        if (was) await keepLostHere(await ctx.optional("keys: logs.branch", logs.branch(was, Math.max(0, was.epoch - 8)), () => [{ epoch: was.epoch, secret: hexOf(was.secret) }]));
        const aad = new TextEncoder().encode(JSON.stringify({ why, ...(proof ?? {}) }));
        const [m2, commit] = (await memberWith()).enter(sp.idBytes, info, aad);
        const entry = JSON.stringify({ commit: hexOf(commit), ...(await provOf(`commit enter (${why})`)) });
        await index.spacePoint(sp, doorName(f.epoch), { k: "enter", e: f.epoch, entry });
        m = m2;
        const kept = await save(true, null, "enter");
        keyEvent("enter", { space: hexOf(sp.idBytes), from: f.epoch, to: m.status().epoch, why, proof: proof?.join ?? null });
        ctx.log(`${sp.name ?? "space"} keys`, { what: `entered by itself (${why}) at epoch ${f.epoch}: no one else needed` });
        announce().catch(e => keyEvent("announce-failed", { space: hexOf(sp.idBytes), why: String(e?.message ?? e).slice(0, 160) }));
        historyOwed = true;
        return kept;
      }
      keyEvent("enter-no-info", { space: hexOf(sp.idBytes), why, announced: anns.length });
      throw new Error(`${sp.name ?? "this space"}: no group info a listed member published yet — entered when one is`);
    }
    let historyOwed = false;
    const g = {
      // ENTER by itself (`enterNow`), in the group's queue.
      enter: (why, proof = null) => (queue = ctx.background("keys: queue", queue).then(() => enterNow(why, proof))),
      // The space's LIST changed (`roles`): what was deferred for it is decided now.
      listChanged: () => {
        shared = null;
        // What waited for it (a catch-up deferred until the list read whole) goes on now.
        ctx.background("keys: listChanged", g.ready());
      },
      // REFRESH this DID's own keys in the space (a device of the account was removed: what it held follows nothing
      // after this commit).
      // A TARGET, not a one-off (ARCHITECTURE §5, reconciling): the mark `~refresh` stays until a refresh commit is in
      // the log — one that lost its race, or a page closed first, is made again on the next load (`ready`).
      refresh: () =>
        (queue = ctx.background("keys: queue", queue).then(async () => {
          const t = await spacekeys();
          if (!t.rows().some(r => r.key === `${at}~refresh` && r.value)) await t.put(`${at}~refresh`, String(Date.now()));
          await change(() => [m.update(), null], { kind: "refresh" });
          await t.remove(`${at}~refresh`);
          ctx.log(`${sp.name ?? "space"} keys`, { what: `this account's keys refreshed: epoch ${m.status().epoch}` });
          return st;
        })),
      // MADE by this DID, its first member.
      create: () =>
        (queue = ctx.background("keys: queue", queue).then(async () => {
          m = (await memberWith()).create_space(sp.idBytes);
          ctx.log(`${sp.name ?? "space"} keys`, { what: "made the space's group: epoch 0" });
          return save(true, null, "create");
        })),
      // UPKEEP's view (`upkeep`): the group's epoch and state, for the mandate the identity delegate admits by while no
      // page runs; and a NEWER group it made meanwhile (it let someone in), ADOPTED here (kept for the account's
      // devices, loaded).
      snapshot: () =>
        (queue = ctx.background("keys: queue", queue).then(async () => {
          if (!m && !(await load())) return null;
          const s = m.status();
          return s.removed ? null : { epoch: s.epoch, state: hexOf(s.state) };
        })),
      // REMOVED: this account taken out of the space's group (a removal or a ban by its acts, a commit read since) —
      // the group brought current first. Not joined yet (no group here): not removed.
      removed: () =>
        (queue = ctx.background("keys: queue", queue).then(async () => {
          if (!m && !(await load())) return false;
          await ctx.optional("keys: current", current(false), () => null);
          return !!m?.status().removed;
        })),
      adopt: (epoch, stateHex) => (queue = ctx.background("keys: queue", queue).then(() => adoptNow(epoch, stateHex))),

      // Loaded and brought current; null where this account is not in the space's group. `fresh`: its log read again
      // from the network first (before a write: sealed with the newest epoch, never one a removed member holds).
      // READY, SHARED: a read takes the group as it stands — one check per space, every table of it waiting on the
      // same — until the group changes (its state saved again, or its current epoch's log moving: someone committed).
      // A write asks `fresh` (read again from the network: sealed with the newest epoch).
      ready: ({ fresh = false } = {}) => {
        if (!fresh && shared) return shared;
        const p = (queue = ctx.background("keys: queue", queue).then(async () => {
          if (!m && !(await load())) return null;
          const s = await current(fresh);
          if (!s) return null;
          const log = await logs.ensure(m.status(), false, null, "ready");
          // ENTERED BY ITSELF: its history from the logs, once a member wrote the epoch's way back.
          if (historyOwed && (await ctx.optional("keys: logs.history", logs.history(m.status()), () => 0))) historyOwed = false;
          if (log && followed !== log) (followed = log), log.onChange?.(() => followed === log && (shared = null));
          announce().catch(e => keyEvent("announce-failed", { space: hexOf(sp.idBytes), why: String(e?.message ?? e).slice(0, 160) }));
          // RECONCILED, never redone: removals follow the acts (`moderation.enforce`, every pass), admissions too
          // (devices entering by themselves); a REFRESH still owed (its mark: a lost race, a page closed first) made now.
          if (!refreshTried && (await spacekeys()).rows().some(r => r.key === `${at}~refresh` && r.value) && (refreshTried = true))
            queueMicrotask(() => g.refresh().catch(e => ctx.log(`${sp.name ?? "space"} keys`, { what: `refresh owed: ${e.message}` })));
          return s;
        }));
        shared = p;
        // Finished: current (every change of the group runs in this same queue) — what reads share from now on. NOT a
        // null ("not here yet": a fresh load's `spacekeys` still arriving when a space asked first — owner 10-07: kept
        // as the answer, a member's space showed 0 members and no apps for the whole session): the next read asks again.
        ctx.optional("keys: p.then", p.then(s => (s ? shared === null && (shared = p) : shared === p && (shared = null))), () => shared === p && (shared = null));
        return p;
      },
      // MEND (owner 10-07: "don't let it happen in the first place" — and heal what already did): this epoch's log
      // opened WITHOUT its prev (written before the rule above) has it put back by a member who holds the epoch before —
      // the same row, its prev added, for every member at once (not a handover to one asker at a time). One that does
      // not hold it writes nothing: "cannot".
      mend: () =>
        (queue = ctx.background("keys: queue", queue).then(async () => {
          if (!m && !(await load())) return "no group";
          const s = m.status();
          if (s.removed || s.epoch === 0) return "nothing to mend";
          const k = await ctx.optional("keys: auth.identity.tableKeyAt", auth.identity.tableKeyAt(ch, s.epoch, sp.idBytes), () => null);
          if (!k?.tableKey) return "its key not held";
          const lg = await storage.log(ch, glue.epoch_log_public(s.secret), { known: null, sealWith: k.tableKey, space: sp.idBytes });
          await lg.answer?.();
          const row = lg.absent ? null : lg.rows().find(r => r.key === "open");
          if (!row?.value) return "no open row";
          let v = null;
          try {
            v = JSON.parse(row.value);
          } catch {
            return "open row unreadable";
          }
          // WHOLE, or whole but for its other ways back (written before them, or while the epoch before's log had not
          // answered): those added now.
          if (v?.prev) {
            if (v.back || s.epoch < 2) return "whole";
            const back = await ctx.optional("keys: logs.backFor", logs.backFor(s.epoch, v.prev), () => null);
            if (!back) return "whole (no way back further to add yet)";
            await lg.put("open", JSON.stringify({ ...v, back, ...(await provOf("mend")) }));
            keyEvent("log-back-added", { space: hexOf(sp.idBytes), epoch: s.epoch, back: Object.keys(back).length });
            return `back added (${Object.keys(back).length})`;
          }
          const before = (await ctx.optional("keys: auth.identity.epochSecrets", auth.identity.epochSecrets(sp.idBytes, s.epoch), () => [])).find(([e]) => e === s.epoch - 1)?.[1];
          if (!before) {
            keyEvent("log-mend-cannot", { space: hexOf(sp.idBytes), epoch: s.epoch });
            return "cannot: the epoch before is not held here";
          }
          const back = await ctx.optional("keys: logs.backFor", logs.backFor(s.epoch, before), () => null);
          await lg.put("open", JSON.stringify({ ...v, prev: before, ...(back ? { back } : {}), ...(await provOf("mend")) }));
          keyEvent("log-mended", { space: hexOf(sp.idBytes), epoch: s.epoch });
          ctx.log(`${sp.name ?? "space"} keys`, { what: `its epoch ${s.epoch} log had no way back: mended (its prev put back)` });
          return "mended";
        })),
      // AUDIT (log only): the epoch this node's group state is at, and whether its identity holds that epoch's key.
      diag: () =>
        (queue = ctx.background("keys: queue", queue).then(async () => {
          if (!m && !(await load())) return { state: "none" };
          const s = m.status();
          const k = await auth.identity.tableKeyAt(ch, s.epoch, sp.idBytes).catch(e => ({ why: e?.message ?? String(e) }));
          // Its epoch LOG as it stands (read, never written here): each "open" row, its writer, and whether it names prev.
          let opens = "unread";
          try {
            const lg = await storage.log(ch, glue.epoch_log_public(s.secret), { known: null, sealWith: k?.tableKey, space: sp.idBytes });
            await lg.answer?.();
            opens = lg.absent ? "log absent" : lg.rows().filter(r => r.key === "open").map(r => { try { const v = JSON.parse(r.value); return `${v.prev ? "prev" : "NO prev"}${v.back ? `+${Object.keys(v.back).length} back` : ""}${v.by ? ` by ${v.by.slice(0, 8)} (${v.why}) v${v.v} ${new Date(v.at).toISOString().slice(0, 16)}` : " (no provenance: written before 10-07)"}`; } catch { return "?"; } }).join(",") || "no open row";
          } catch (e) {
            opens = `error ${e?.message ?? e}`;
          }
          return { opens, state: "held", epoch: s.epoch, removed: !!s.removed, members: s.members?.length ?? 0, keyHeld: !!k?.tableKey, why: k?.tableKey ? null : (k?.why ?? k?.refused ?? null) };
        })),
      lost: async () => JSON.parse((await spacekeys()).rows().find(r => r.key === `${at}~lost`)?.value ?? "[]"),
      // Another branch's epoch keys, kept as LOST here (read with as alternates, never written with): what another
      // device hands over the bridge (`members`).
      keepLost: list => keepLostHere(list),
      // WHETHER THIS DEVICE HOLDS the group at all — its state row, the table read AGAIN from the network first: a
      // definite answer (`ready()` is null too while the table is still arriving — never a reason to ask for keys).
      holds: async () => {
        const t = await spacekeys();
        await t.reread?.();
        return t.rows().some(r => r.key === at && r.value);
      },
      fingerprint: e => (queue = ctx.background("keys: queue", queue).then(() => fingerprint(e))),
    };
    // (Its READS total — `ctx.total`: never thrown at a caller; a group not ready here reads as null.)
    const readyRaw = g.ready;
    const fingerprintRaw = g.fingerprint;
    g.ready = ctx.total("keys.group.ready", () => null, readyRaw);
    g.fingerprint = ctx.total("keys.group.fingerprint", () => null, fingerprintRaw);
    groups.set(sp.id, g);
    return g;
  }

  // How many epochs are in escrow for the words.
  // Every epoch is escrowed: the first from the channel, each later one with the commit that made it.
  async function escrowed() {
    const st = await ctx.optional("keys: ready", ready(), () => null);
    return st && !st.removed ? st.epoch + 1 : 0;
  }

  // AUDIT (log only), once a page: each space's epoch log — one opened without its way back mended (owner 10-07).
  let audited = false;
  async function auditChains() {
    if (audited) return;
    audited = true;
    for (const sp of await ctx.optional("keys: mine", (await ctx.require("space")).mine(), () => [])) {
      if (sp.kind === "account") continue;
      const d = await group(sp).diag().catch(e => ({ state: `error ${e?.message ?? e}` }));
      const broken = d.state === "held" && d.epoch > 0 && /NO prev/.test(String(d.opens));
      // THIN: a way back but none further (`back`): mended quietly too.
      const thin = d.state === "held" && d.epoch > 1 && /(^|,)prev(?!\+)/.test(String(d.opens));
      const mended = broken || thin ? await group(sp).mend().catch(e => `mend failed: ${e?.message ?? e}`) : null;
      if (thin && mended && !/^whole/.test(mended)) ctx.log("account keys", { what: `${sp.name ?? sp.id.slice(0, 8)}: its epoch ${d.epoch} log — ${mended}` });
      const evs = broken ? (await ctx.optional("keys: keyEvents", keyEvents(sp.id), () => [])).slice(-5).map(e => `${new Date(e.at).toISOString().slice(5, 16)} ${e.event}${e.epoch != null ? ` @${e.epoch}` : ""}${e.reason ? ` (${e.reason})` : ""} by ${String(e.device).slice(0, 8)} v${e.v}`).join("; ") : "";
      ctx.log("account keys", { what: `AUDIT space ${broken ? "BROKEN CHAIN — " : ""}${sp.name ?? sp.id.slice(0, 8)}${broken ? ` [mend: ${mended}; its key events: ${evs || "none recorded"}]` : ""}: group ${d.state}${d.state === "held" ? ` at epoch ${d.epoch}, ${d.members} member(s)${d.removed ? ", REMOVED" : ""}, its epoch key ${d.keyHeld ? "HELD" : `NOT held${d.why ? ` (${d.why})` : ""}`}, its log's open: ${d.opens}` : ""}` });
    }
    const allEv = await ctx.optional("keys: keyEvents", keyEvents(), () => []);
    ctx.log("account keys", { what: `AUDIT key events — ${allEv.length} kept; last: ${allEv.slice(-3).map(e => `${e.event}${e.epoch != null ? ` @${e.epoch}` : ""}`).join(", ")}` });
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
      .then(st => st && !st.removed && (setTimeout(cardReady), setTimeout(() => auditChains().catch(e => ctx.failed("account keys", e, `AUDIT failed: ${e?.message ?? e}`)), 20000)))
      .catch(e => ctx.log("account keys", { what: e?.message ?? String(e) }));
  start();
  addEventListener("craftworks:auth", e => {
    if (!e.detail) status = null;
    else start();
  });

  return { repairAccount, ready: ctx.total("keys.ready", () => null, ready), writable, mustWrite, remove, escrowed: ctx.total("keys.escrowed", () => null, escrowed), group, setList, onChange: f => watchers.push(f) };
}
