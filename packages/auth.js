// AUTH, a capability (no UI: the dialog is the `login` component): who is logged in, and the ways in. What an account
// IS (its key log, which account words hold, changing them) is the `identity` capability's; who its nodes are is the
// `membership` capability's.
// - `check()`: who is logged in, or null. Never a dialog.
// - `unlock(pin)`, `join(entropy, pin, { fresh })`: the two ways in (a PIN on this node; recovery words, which make this
//   node a member); `accept(answer)` turns an answer that opened a member into the session.
// - `logout()`: ends this app's session and starts the app again at home.
// Every change is announced as a `craftworks:auth` event (detail: the person, or null), so a page can switch views.
// - A MEMBER is one NODE's key in the account, kept by the identity delegate on that node and opened by a PIN. Every
//   browser on the node shares its members and sessions. One node can hold several members, of one person or of
//   several. Registering makes the recovery words and shows them ONCE; no node keeps them.
export async function start(ctx) {
  const id = await ctx.require("identity");
  const { glue, hex, bytes: hexBytes } = await ctx.require("node");
  // THE SESSION's lifecycle (FOUNDATION.md §3: `session`): recorded here (shadow — nothing gated on it yet).
  const life = await ctx.require("lifecycle");
  const fault = await ctx.require("fault");
  let current = null;
  let made = null; // the DID (hex) of an account this page registered

  const announce = () => dispatchEvent(new CustomEvent("craftworks:auth", { detail: current }));
  const opened = a => {
    current = {
      member: a.unlocked.member,
      did: glue.did_of(hexBytes(a.unlocked.did)),
      didBytes: hexBytes(a.unlocked.did),
      data: a.unlocked.data, // the account's data key (public, hex), or null
      // Made by this page just now: nothing of it is on the network yet, so there is nothing to read, only to create.
      fresh: made === a.unlocked.did,
    };
    ctx.log("logged in", { what: `${current.did.slice(0, 24)}… member ${current.member.slice(0, 12)}…` });
    if (life.state("session", "me").s !== "OPEN") life.transition("session", "me", "open", { fresh: current.fresh });
    announce();
    return current;
  };

  // Whether this page STARTED with nobody logged in (its first answer): a session opened later was a login made HERE
  // (a PIN, recovery words, a registration) — the loader's resume is for that alone, never for a visit.
  let startedOut = null;
  async function check() {
    if (current) return current;
    const w = await id.who();
    startedOut ??= !w.unlocked;
    return w.unlocked ? opened(w) : null;
  }

  async function logout() {
    await id.lock();
    current = null;
    life.transition("session", "me", "lock", {});
    ctx.log("logged out", {});
    announce();
    // The app starts again, at home: every capability holds per-session things (grants asked, tables and their keys,
    // the account's group), and none of them may reach the next person to log in on this page.
    location.hash = "#/";
    location.reload();
  }

  // Log in with the PIN. A member made by an EARLIER build of the identity delegate is not in this one (a delegate's
  // secrets stay with its build): on a PIN this build does not know, the earlier builds are asked, newest first, and
  // the member the PIN opens there is moved here, so an update never costs anyone their login.
  async function unlock(pin) {
    const r = await unlock_(pin);
    // The EARLIER builds retired and their keys gathered BEFORE the login returns: a login loads the page again at
    // once (`login`), which ended this work when it ran behind (10-18, fx: no earlier build retired, epoch 0 never
    // recovered — the space's acts and channels unreadable after an update). Bounded: a build silent past it is left.
    if (r.unlocked) {
      // THE ACCOUNT's GENERATION TO WRITE WITH marked first (FOUNDATION "WHO RECEIVES IS DECIDED BY THE ONE WHO SEALS":
      // an earlier build's keys are only HELD here — its writes, the retirement's own moves among them, would wait on the
      // account's first reconcile; 10-28 gate: the "retired" move waited past the bound below and was never kept).
      await ctx.optional("auth: the account ready to write", async () => (await (await ctx.require("members")).account()).readyToWrite(), () => null);
      let timer;
      await Promise.race([
        retireEarlier(pin, r.movedFrom ?? null).catch(e => ctx.log("earlier build", { what: `not finished: ${e?.message ?? e}` })),
        new Promise(res => (timer = setTimeout(() => (ctx.log("earlier build", { what: "still working after 120 s: the rest left" }), res()), 120_000))),
      ]).finally(() => clearTimeout(timer));
      // Its moves kept before the login reloads the page.
      await life.keepNow().catch(e => ctx.log("earlier build", { what: `moves not kept: ${e?.message ?? e}` }));
    }
    return r;
  }
  // The EARLIER builds' upkeep for this member, retired (an empty mandate each): every build stays on the node and
  // wakes every minute, and one the member left would act on its last mandate — admitting by stale rules, re-keying
  // under its own. Newest first, up to the first that does not hold this PIN: a member is in every build from the one
  // that made it (a handover keeps the PIN), and each wrong PIN counts against a build's tries — at most one here. A
  // build that answers nothing (still loading, or one that never runs) counted no try: passed over. In the background.
  // On the way, every earlier build holding the member hands the EPOCH KEYS this build lacks (`epochsOnly`): the member
  // may have been moved from an older one (a newer build slow to answer), and a key only a newer build got — a space's
  // later epoch — would otherwise be lost (seen: "no key here for epoch 17").
  async function retireEarlier(pin, movedFrom) {
    // One earlier build: retired, its epoch keys this build lacks gathered — the handover's (`epochsOnly`), and every
    // space's MLS epochs asked by number (`epochsFrom`: an earlier build's handover walked only the numbers just below
    // its latest epoch — once a space's epochs were BeeKEM generations, its MLS epochs were left behind: v401 → v403,
    // 10-18, a channel unreadable after the PIN).
    const one = async prior => {
      const r = await id.retireFrom(prior, pin).catch(e => ({ said: e.message }));
      // Each outcome a lifecycle move of the build (reported, journaled): `retired` / `silent` / `not-here` / `stopped`.
      const key = prior.split(":")[0];
      const move = (event, d) => {
        try {
          life.transition("build", key, event, d);
        } catch (e) {
          ctx.log("earlier build", { what: `${key.slice(0, 12)}…: ${event} not recorded: ${e?.message ?? e}` });
        }
      };
      move(r.retired ? "retired" : r.missing ? "not-here" : r.silent ? "silent" : "stopped", r.retired ? { witnessed: r.witnessed ?? 0 } : { why: r.said ?? null });
      if (r.retired && prior !== movedFrom) {
        const k = await id.moveKeysFrom(prior, pin, { epochsOnly: true }).catch(e => ({ said: e.message }));
        move(k.said ? "keys-not-read" : "keys-gathered", k.said ? { why: k.said } : { epochs: k.moved, spaces: k.spaces });
      }
      if (r.retired) {
        let kept = 0;
        let spaces = 0;
        for (const sp of (await ctx.optional("auth: spaces", (await ctx.require("space")).mine.strict(), () => [])).filter(s => s.kind !== "account")) {
          const e = await id.epochsFrom(prior, sp.idBytes, 2 ** 24).catch(e => ({ said: e.message }));
          kept += e.kept ?? 0;
          spaces += 1;
        }
        move("epochs-kept", { kept, spaces });
      }
      return r;
    };
    // Newest first, up to the first that does not hold this PIN. A SILENT one (an overloaded or still loading build —
    // exactly the one whose rounds must stop) is asked again by the fault policy, never passed over for good (10-18:
    // v401's build, overloaded, answered nothing, and kept running its rounds on the owner's node after the login).
    // FROM THE BUILD IT CAME FROM (`movedFrom`, the handover's): the builds newer than it never held this member — the
    // handover asked them first — and one of them saying "not here" ended the walk before the member's own build was
    // retired (10-20: a build published only to a test node, newer than the release before, stopped every retirement).
    let silent = [];
    const from = movedFrom ? ctx.identityPrior.indexOf(movedFrom) : -1;
    for (const prior of from > 0 ? ctx.identityPrior.slice(from) : ctx.identityPrior) {
      const r = await one(prior);
      if (r.silent) silent.push(prior);
      else if (!r.retired && !r.missing) break;
    }
    if (silent.length)
      await fault
        .attempt("earlier build", "retiring the earlier builds not answering yet", async () => {
          const still = [];
          for (const prior of silent) if ((await one(prior)).silent) still.push(prior);
          silent = still;
          if (silent.length) throw new fault.Fault("not-yet", `${silent.length} earlier build(s) not answering yet`, { d: { silent: silent.length } });
          return null;
        }, { policy: { "not-yet": { tries: 30, wait: () => 20000, report: "never" } } })
        .catch(fault.absentOnly);
    // Keys gathered: what reads with them is read again.
    dispatchEvent(new CustomEvent("craftworks:keys"));
  }
  async function unlock_(pin) {
    const r = await id.unlock(pin);
    if (!r.wrongPin) return r;
    // ONE ASK of an earlier build: what it said (`silent`: not answering yet — a build the node is still loading).
    const askOnce = prior => ctx.optional("earlier build", id.handoverFrom(prior, pin), null);
    // The member an earlier build hands over, moved here with its keys (the account's group, the epochs it could read)
    // before anything reads a table.
    const moveIn = async (prior, h) => {
      const bytes = x => (x ? hexBytes(x) : new Uint8Array(0));
      // The account's seeds: its NAMES seed first, where that build keeps one apart from the data seed (K8.7, after a
      // rotation) — `names ‖ data`; else the one seed.
      const handed = h.handed.data ? await id.namesFrom(prior, pin) : null;
      if (handed?.silent || handed?.refused) throw new Error(`the earlier build did not give the account's names seed (${handed.refused ?? "no answer"}): log in again in a moment`);
      const names = handed?.names ?? null;
      const seeds = names ? new Uint8Array([...bytes(names), ...bytes(h.handed.data)]) : bytes(h.handed.data);
      const p = await id.provision(bytes(h.handed.seed), bytes(h.handed.did), pin, seeds);
      // (The data seeds it replaced: what was sealed to them before a rotation still opens here.)
      if (p.unlocked && handed?.old?.length) await ctx.optional("auth: dataOldKeep", id.dataOldKeep(handed.old), () => null);
      ctx.log("member moved", { what: `from the earlier build ${prior.slice(0, 12)}…: ${p.unlocked ? "logged in" : JSON.stringify(p)}` });
      p.movedFrom = prior;
      if (p.unlocked) {
        const k = await id.moveKeysFrom(prior, pin).catch(e => ({ moved: 0, said: e.message }));
        ctx.log("member moved", { what: `its keys: ${k.group ? "the account's group and " : ""}${k.moved} epoch(s), ${k.spaces ?? 0} space(s)${k.said ? ` (${k.said})` : ""}` });
      }
      return p;
    };
    // Newest first, each once; the SILENT ones asked again, 10 s apart, for a minute — a slow node compiles a build in
    // about that (10-12: every earlier build of the owner's node was passed over at once, and the login went to the
    // recovery words — a new member without the account's keys).
    let silent = [];
    for (const prior of ctx.identityPrior) {
      const h = await askOnce(prior);
      if (h?.handed) return moveIn(prior, h);
      if (h?.silent) silent.push(prior);
    }
    // The silent ones asked again by the fault POLICY (one retry mechanism: `attempt`) — `not-yet`, 10 s apart, a minute.
    if (silent.length) {
      const got = await fault
        .attempt("earlier build", "asking the earlier builds not answering yet", async () => {
          const still = [];
          for (const prior of silent) {
            const h = await askOnce(prior);
            if (h?.handed) return { prior, h };
            if (h?.silent) still.push(prior);
          }
          silent = still;
          if (silent.length) throw new fault.Fault("not-yet", `${silent.length} earlier build(s) not answering yet`, { d: { silent: silent.length } });
          return null;
        }, { policy: { "not-yet": { tries: 7, wait: () => 10000, report: "never" } } })
        .catch(fault.absentOnly);
      if (got) return moveIn(got.prior, got.h);
    }
    // Not "wrong": an earlier build never answered — the PIN may well be right.
    if (silent.length) return { slow: true };
    return r;
  }

  // This node as a member of the account the words hold. Kept across a retry with another PIN, so one attempt
  // mints one member key.
  //
  // WHICH account: new words (`fresh`, just made) hold their own inception's. Typed words hold either their own
  // inception's (an account's original words) or the one their whoami names (words rotated in): both are asked AT
  // ONCE and the first that answers wins, so the one that does not exist never holds the login up. Neither: refused
  // (no account has them — a typo never makes an empty account).
  let joining = null;
  const joined = [];
  async function join(entropy, pin, { fresh = false, own = false } = {}) {
    if (!joining || joining.entropyHex !== hex(entropy)) {
      const member = crypto.getRandomValues(new Uint8Array(32));
      const a = await id.accountOf(entropy, { fresh, own });
      joining = { entropyHex: hex(entropy), entropy, did: a.didBytes, data: a.data, member };
    }
    const r = await id.provision(joining.member, joining.did, pin, joining.data);
    if (r.unlocked) {
      // Only a REGISTRATION makes an account that has nothing on the network yet; a words login opens one that does.
      if (fresh) made = r.unlocked.did;
      // While the words are still here: whoever needs them to act for the account (the `keys` capability: this node
      // joins the account's MLS group). A failure there is reported, never a failed login.
      const node = id.publicOf(joining.member);
      // …and to the health bag (`account-recovery`): a recovery that failed is a report, never only a trace line.
      for (const f of joined)
        await f({ entropy: joining.entropy, did: joining.did, node, fresh }).catch(e => {
          const why = e?.message ?? String(e);
          ctx.log("after login", { what: why });
          ctx.background("auth: ctx.require", ctx.require("index").then(ix => ix.health({ k: "account-recovery", s: "", why: `failed: ${why}`.slice(0, 160), d: { fresh: !!fresh } })));
        });
      joining.member.fill(0);
      joining.entropy.fill(0);
      joining.data.fill(0);
      joining = null;
    }
    return r;
  }

  return {
    loggedInHere: () => !!current && startedOut === true,
    check: ctx.total("auth.check", () => null, check), accept: opened, unlock, join, logout, current: () => current, identity: id,
    // `fn({ entropy, did, node, fresh })`, called after a words login or a registration, before the words are wiped.
    onJoined: f => joined.push(f),
  };
}

