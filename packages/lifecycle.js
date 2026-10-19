// LIFECYCLES, a service (FOUNDATION.md §1, §3): every process the system runs has its states written down HERE, as
// data — what it may move to, from where, and on what condition — and moves only through `transition`. Nobody keeps
// their own flags: they ask `state`. A move from a state the table does not allow is a `bug` fault; a guard that says
// no is a `refused` one — both thrown, so nothing happens on a wrong assumption. Every move is one health report
// (`lifecycle-<machine>`: from, to, the event and its details), and what each machine ended in rides in the visit.
// THE ONE RECORD (owner 10-14: "it should not be a separate instrument… the structure should support it for all
// processes"): a machine marked `journal` also keeps every move in the account's `journal` table — this device, the
// version, when, the details — read back with `journal(machine, key)` (the account's keys' critical actions, 10-07,
// were a table of their own: `keyevents`, now these moves). A STEP (`to` left out) is a move that keeps the state: a
// thing done, recorded and reported, nothing changed.
//
//   const life = await ctx.require("lifecycle");
//   life.transition("account", "me", "walk", { channel: "present", info: true })
//   life.state("account", "me").s                       // "WALKING"
//   life.on("account", (key, to, from, event) => …)
export async function start(ctx) {
  const { Fault, met } = await ctx.require("fault");

  // THE MACHINES. `events`: event → { from: [states] | "*", to, guard?(detail) → true | why-not }.
  // PROGRESS, the ONE way a running process shows on the page (owner 10-13: "can the progress bar become one
  // implementation and standard for all lifecycle?"): `progress: { STATE: text | (key, detail) => text }` — said while
  // the process is in that state; `after` (ms): only once it has lasted that long (a quick step never flickers);
  // `ended: { STATE: { text, ms } }` — how it finished, said for `ms`. The page's pill lists every process in progress.
  const MACHINES = {
    // THE SESSION (who this page is for). OPEN_KEYLESS: logged in, but the account's keys did not come — it must not
    // write to the card, key packages, welcomes or asks (gated from phase 4), and says "keys not recovered".
    session: {
      initial: "GUEST",
      events: {
        open: { from: ["GUEST", "LOCKED", "OPEN", "OPEN_KEYLESS", "REMOVED"], to: "OPEN" },
        keyless: { from: ["GUEST", "LOCKED", "OPEN", "OPEN_KEYLESS"], to: "OPEN_KEYLESS" },
        lock: { from: "*", to: "LOCKED" },
        removed: { from: "*", to: "REMOVED" },
      },
    },
    // THE ACCOUNT'S KEY GROUP, on this device. Only a REGISTRATION makes a group, and only where the node answered the
    // account's channel is not there; a words login walks the channel's history and joins.
    account: {
      initial: "NONE",
      journal: true,
      progress: {
        CONFIRMING_CHANNEL: "Recovering your account (1 of 4): finding your account's keys…",
        WALKING: "Recovering your account (2 of 4): opening your keys…",
        JOINING: "Recovering your account (3 of 4): adding this device…",
        CREATING: "Setting up your account's keys…",
      },
      ended: {
        MEMBER: { from: ["WALKING", "JOINING", "CREATING"], text: "Recovering your account (4 of 4): opening your spaces and tables — keep this page open a minute", ms: 90000 },
        JOIN_FAILED: { text: "Recovery stopped — try the recovery words again (the trace says why)", ms: 30000 },
      },
      events: {
        login: { from: "*", to: "CONFIRMING_CHANNEL" },
        walk: {
          from: ["CONFIRMING_CHANNEL"],
          to: "WALKING",
          guard: d => (!d.fresh && d.channel === "present" && d.info) || "a words login walks only a channel read with its group info",
        },
        create: {
          from: ["CONFIRMING_CHANNEL"],
          to: "CREATING",
          guard: d => (d.fresh && d.channel === "absent") || `only a registration makes the account's group, on a channel the node answered is not there (fresh ${!!d.fresh}, channel ${d.channel})`,
        },
        join: { from: ["WALKING"], to: "JOINING" },
        joined: { from: ["JOINING"], to: "MEMBER" },
        made: { from: ["CREATING"], to: "MEMBER" },
        fail: { from: ["CONFIRMING_CHANNEL", "WALKING", "JOINING", "CREATING"], to: "JOIN_FAILED" },
        loaded: { from: ["NONE", "MEMBER", "JOIN_FAILED"], to: "MEMBER" },
        removed: { from: "*", to: "REMOVED" },
      },
    },
    // A GROUP (a space's, on this device — FOUNDATION §3 `group`): where its keys stand as upkeep finds it each round —
    // HELD (current, on the owner's branch), BEHIND, OFF_BRANCH, NOT_HELD, UNREADABLE, STUCK (behind its own commit),
    // REMOVED — and every STEP its keys take (an entry by itself, a commit and why, one skipped by the list check, a
    // log written or mended, history handed), each kept and reported: what moved an epoch is never unknown.
    group: {
      initial: "UNKNOWN",
      journal: true,
      // Steps that are routine (still kept in the journal): not each a health report.
      quiet: ["log-open-written", "log-open-left", "log-back-added", "log-mended", "history-received", "history-handed", "writers-found", "follow-info", "follow-keys"],
      events: {
        held: { from: "*", to: "HELD" },
        // Ahead of the owner's announcement (it reads: HELD).
        ahead: { from: "*", to: "HELD" },
        behind: { from: "*", to: "BEHIND" },
        "off-branch": { from: "*", to: "OFF_BRANCH" },
        "not-held": { from: "*", to: "NOT_HELD" },
        unreadable: { from: "*", to: "UNREADABLE" },
        removed: { from: "*", to: "REMOVED" },
        "stuck-own-commit": { from: "*", to: "STUCK" },
        commit: { from: "*" },
        applied: { from: "*" },
        "apply-failed": { from: "*" },
        // ENTERED BY ITSELF (FOUNDATION "Members and keys" 1): an external commit put in its epoch's door — or no group
        // info a listed member published to enter from yet.
        enter: { from: "*" },
        "enter-no-info": { from: "*" },
        "enter-not-listed": { from: "*" },
        // A NEW WRITER of one of its tables found (`storage`): read now, or waiting to be listed in its writers bag.
        "writers-found": { from: "*" },
        "writers-waiting": { from: "*" },
        // Its epoch and group info NOT announced (what a device with no key enters from): why.
        "announce-failed": { from: "*" },
        // An entry SKIPPED by the list check (`mls::entry_ok`: every reader skips it), and a catch-up DEFERRED until
        // the list is read whole (I6).
        "entry-skipped": { from: "*" },
        "catch-up-deferred": { from: "*" },
        "adopt-delegate": { from: "*" },
        "pending-published": { from: "*" },
        "log-open-written": { from: "*" },
        "log-open-left": { from: "*" },
        "log-back-added": { from: "*" },
        "log-mended": { from: "*" },
        "log-mend-cannot": { from: "*" },
        "history-received": { from: "*" },
        "history-handed": { from: "*" },
        // THE NODE's FOLLOWER over this space (`by: node`: removals only), each step its own event with its why: its
        // plan; its commit; and why a round did not finish.
        "follow-plan": { from: "*" },
        "follow-commit": { from: "*" },
        "follow-stalled": { from: "*" },
        "follow-short": { from: "*" },
        "follow-entry-skipped": { from: "*" },
        "follow-unloadable": { from: "*" },
        "follow-behind": { from: "*" },
        "follow-log-refused": { from: "*" },
        "follow-commit-refused": { from: "*" },
        "follow-next-refused": { from: "*" },
        "follow-no-commit": { from: "*" },
        // The node's group info for an epoch: published, or waiting for randomness (a page stirs the pool).
        "follow-info": { from: "*" },
        "follow-info-held": { from: "*" },
        // Steps of the node lost to its limit before a page read them: how many.
        "follow-dropped": { from: "*" },
        // MEMBERS AND KEYS ON BEEKEM (K3–K7, `members`): its TREE made (this account placed itself); a reconcile that
        // CHANGED it (added, removed — one key change) or found it IN STEP with the list; one that decided nothing (the
        // list not read whole — I6); OPERATIONS refused by our check (why) or waiting on their predecessors; a listed
        // account whose card names no member we can check; the HISTORY BRIDGE written and crossed.
        "keys-tree-made": { from: "*" },
        "keys-reconciled": { from: "*" },
        "keys-in-step": { from: "*" },
        "keys-short": { from: "*" },
        // This page's account NOT in the space's tree (a tree there, placing others): it cannot add anyone — said, with
        // whom the tree places (10-19: Craftworks' tree placed one member nobody could name; nobody was ever added, and
        // no report said why).
        "keys-not-placed": { from: "*" },
        "keys-refused": { from: "*" },
        "keys-waiting": { from: "*" },
        "keys-unmapped": { from: "*" },
        "keys-bridged": { from: "*" },
        "keys-crossed": { from: "*" },
        // …and the NODE's (`keys_round`, every page closed): in step; its plan (added, removed); not changed — the list
        // short, this account not placed, no randomness yet, no data key here; its write failed or was refused.
        "follow-keys": { from: "*" },
        "follow-keys-plan": { from: "*" },
        "follow-keys-short": { from: "*" },
        "follow-keys-not-placed": { from: "*" },
        "follow-keys-wait": { from: "*" },
        "follow-keys-no-member": { from: "*" },
        "follow-keys-write": { from: "*" },
        "follow-keys-refused": { from: "*" },
      },
    },
    // A REQUEST TO JOIN (owner 10-17: "requests are not hanging"), keyed by the space: asked; then CANCELLED by the asker,
    // REJECTED by one who may invite, or MEMBER — let in, or in by a code, the open door or a level, entered by itself.
    // Every move reported.
    join: {
      initial: "NONE",
      journal: true,
      events: {
        ask: { from: "*", to: "REQUESTED" },
        cancelled: { from: "*", to: "CANCELLED" },
        rejected: { from: "*", to: "REJECTED" },
        joined: { from: "*", to: "MEMBER" },
      },
    },
    // AN OPEN APP (a body component): opening → open → closed, or failed (reported; the rest are quiet — they ride in
    // the visit's snapshot).
    app: {
      initial: "CLOSED",
      progress: { OPENING: key => `Opening ${key}…` },
      after: 4000,
      quiet: ["opening", "open", "closed"],
      events: {
        opening: { from: "*", to: "OPENING" },
        open: { from: ["OPENING"], to: "OPEN" },
        failed: { from: "*", to: "FAILED" },
        closed: { from: "*", to: "CLOSED" },
      },
    },
    // A DELETE (an item, a space left, a device removed): one path — asked, then HIDDEN (still on the network, no
    // longer shown: a tail can hide, not erase) or GONE (forgotten), or REFUSED with why.
    delete: {
      initial: "NONE",
      journal: true,
      progress: { REQUESTED: (_, d) => `Deleting${d?.what ? ` ${String(d.what).slice(0, 40)}` : ""}…` },
      ended: { REFUSED: { text: "Not deleted (the trace says why)", ms: 15000 } },
      events: {
        request: { from: "*", to: "REQUESTED" },
        hidden: { from: ["REQUESTED"], to: "HIDDEN" },
        gone: { from: ["REQUESTED"], to: "GONE" },
        refuse: { from: ["REQUESTED"], to: "REFUSED" },
      },
    },
    // A SAVE (FOUNDATION §3 `save`, one per change): OWED while it is on its way, HELD once the network took it, REFUSED
    // with its fault. THE ONE PLACE "Saving…" and "Not saved" are said (the pill) — for the person's own saves (`user`);
    // background writes (drafts, records, upkeep) move it quietly. A refusal is reported; a save held is forgotten.
    // `owed()`: what an update waits for (page `UPDATING`).
    save: {
      initial: "NONE",
      quiet: ["owe", "held"],
      after: 600,
      progress: { OWED: (_, d) => (d?.user ? "Saving…" : null) },
      ended: { REFUSED: { text: (_, d) => (d?.user ? `Not saved: ${String(d.why ?? "").slice(0, 100)}` : null), ms: 8000 } },
      forget: ["HELD"],
      events: {
        owe: { from: "*", to: "OWED" },
        held: { from: ["OWED"], to: "HELD" },
        refuse: { from: ["OWED"], to: "REFUSED" },
      },
    },
    // THE PAGE (FOUNDATION §3 `page`): no shutdown state — closed at any time. RESUMING puts back what the last page
    // left (drafts); HIDDEN sends what is owed; UPDATING (a newer version) waits for `save` to owe nothing.
    page: {
      initial: "BOOTING",
      quiet: ["resume", "run", "hide", "update"],
      events: {
        resume: { from: ["BOOTING"], to: "RESUMING" },
        run: { from: ["BOOTING", "RESUMING", "HIDDEN", "RUNNING"], to: "RUNNING" },
        hide: { from: ["RUNNING", "RESUMING"], to: "HIDDEN" },
        update: { from: "*", to: "UPDATING" },
        // THE VERSION BEFORE ran (this one's packages not fetchable in time from this node): reported — a page on the
        // previous code once read in every report as the new one (10-18: the owner's login ran v403's code).
        previous: { from: "*" },
      },
    },
    // AN EARLIER IDENTITY BUILD on this node, as a login finds it (`auth`): RETIRED (its upkeep stopped; what it held
    // of this account's keys gathered here — each a step), SILENT (not answering yet: asked again), NOT_HERE (this
    // node never ran it: quiet, kept in the journal), STOPPED (it answered, and holds no member on this PIN: the walk
    // ends there). 10-18: which builds a login retired was in no report.
    build: {
      initial: "UNKNOWN",
      journal: true,
      quiet: ["not-here"],
      events: {
        retired: { from: "*", to: "RETIRED" },
        silent: { from: "*", to: "SILENT" },
        "not-here": { from: "*", to: "NOT_HERE" },
        stopped: { from: "*", to: "STOPPED" },
        "keys-gathered": { from: "*" },
        "keys-not-read": { from: "*" },
        "epochs-kept": { from: "*" },
      },
    },
    // A BACKGROUND JOB (upkeep, sync, the usage flush, keeping): runs every `every` ms only while what it NEEDS holds;
    // PAUSED (with why) while it does not. Its moves are QUIET (10-13: every guest page reported upkeep paused, "nobody
    // is logged in" — ×25 of noise): where each job stands rides in the visit's snapshot; only a stop is reported.
    job: {
      initial: "IDLE",
      quiet: ["run", "done", "pause", "resume"],
      events: {
        run: { from: ["IDLE"], to: "RUNNING" },
        done: { from: ["RUNNING"], to: "IDLE" },
        pause: { from: ["IDLE", "RUNNING"], to: "PAUSED" },
        resume: { from: ["PAUSED"], to: "IDLE" },
        stop: { from: "*", to: "STOPPED" },
      },
    },
  };

  // THE JOURNAL: moves of `journal` machines, batched into the account's table (newest 500 kept).
  const pending = [];
  let flushing = null;
  async function flush() {
    flushing = null;
    const batch = pending.splice(0);
    if (!batch.length) return;
    try {
      // (`device`: which of the account's devices made the move — never a move's own fields, e.g. a decider's `by`.)
      const device = await thisDevice();
      const t = await (await ctx.require("storage")).table("journal");
      for (const e of batch) await t.put(`${e.at.toString(36)}-${Math.random().toString(36).slice(2, 8)}`, JSON.stringify({ ...e, device }));
      const rows = t.rows().filter(r => r.value).map(r => r.key).sort();
      for (const k of rows.slice(0, Math.max(0, rows.length - 500))) await t.remove(k);
    } catch (e) {
      ctx.log("lifecycle", { what: `${batch.length} move(s) not kept in the journal: ${e?.message ?? e}` });
    }
  }
  // THE JOURNAL WRITTEN NOW (not 3 s later): before what reloads the page — a login loads it again as soon as the
  // unlock returns, and the moves of its last 3 s were lost (10-18, fx: a login's `build` moves never in the journal).
  async function keepNow() {
    clearTimeout(flushing);
    await flush();
  }
  // The journal's moves (`machine`; `key`: one process's), oldest first.
  // `fresh`: the table read again from the network first (a guard deciding by it — never a short read).
  async function journal(machine = null, key = null, { fresh = false } = {}) {
    const t = await (await ctx.require("storage")).table("journal");
    await t.settled;
    if (fresh) await t.reread?.();
    return t
      .rows()
      .filter(r => r.value)
      .map(r => {
        try {
          return JSON.parse(r.value);
        } catch {
          return null;
        }
      })
      // …and the moves not written yet (they are flushed a few seconds after): this device's own, as it made them.
      .concat(await pendingMoves())
      .filter(e => e && (!machine || e.m === machine) && (!key || e.key === key))
      .sort((a, b) => a.at - b.at);
  }
  // THIS DEVICE (its member key, the first 16 hex): what a journal row names as the one that made a move.
  async function thisDevice() {
    return String((await ctx.optional("lifecycle: auth.check", (await ctx.require("auth")).check(), () => null))?.member ?? "").slice(0, 16);
  }
  async function pendingMoves() {
    const device = await thisDevice();
    return pending.map(e => ({ ...e, device }));
  }

  const states = new Map(); // `${machine}|${key}` → { s, since, event, detail }
  const listeners = new Map(); // machine → [fn]
  const at = (m, k) => states.get(`${m}|${k}`) ?? { s: MACHINES[m].initial, since: null };

  function state(machine, key) {
    if (!MACHINES[machine]) throw new Fault("bug", `lifecycle: no machine "${machine}"`);
    return at(machine, key);
  }

  // ONE health report per move (never content: a short key, the states, the event's details).
  const report = (machine, key, from, to, event, detail) =>
    ctx.background("lifecycle: index", ctx
      .require("index")
      .then(ix => ix.health({ k: `lifecycle-${machine}`, s: String(key).slice(0, 12), to: `${from}>${to}`, why: `${event}: ${from} → ${to}${detail?.why ? ` — ${String(detail.why).slice(0, 160)}` : ""}`, d: detail })));

  // THE PILL'S LINES: one per process in progress (`machine|key` → text), sent whole on every change
  // (`craftworks:progress` { lines }: the loader draws them).
  const lines = new Map();
  const timers = new Map();
  const say = () => dispatchEvent(new CustomEvent("craftworks:progress", { detail: { lines: [...lines.values()] } }));
  const textOf = (t, key, detail) => (typeof t === "function" ? t(key, detail) : t);
  function progressOf(machine, key, from, to, detail) {
    const m = MACHINES[machine];
    if (!m.progress && !m.ended) return;
    const id = `${machine}|${key}`;
    clearTimeout(timers.get(id));
    timers.delete(id);
    const was = lines.delete(id);
    const now = m.progress?.[to];
    const end = m.ended?.[to];
    if (now) {
      // (A text function may say nothing for this one: a background save.)
      const show = () => {
        const t = textOf(now, key, detail);
        if (t) lines.set(id, t), say();
      };
      if (m.after) timers.set(id, setTimeout(show, m.after));
      else show();
      if (was) say();
      return;
    }
    const endText = end && (!end.from || end.from.includes(from)) ? textOf(end.text, key, detail) : null;
    if (endText) {
      lines.set(id, endText);
      timers.set(id, setTimeout(() => (lines.delete(id), say()), end.ms ?? 10000));
    }
    say();
  }

  function transition(machine, key, event, detail = {}) {
    const m = MACHINES[machine];
    if (!m) throw met("lifecycle", new Fault("bug", `lifecycle: no machine "${machine}"`));
    const ev = m.events[event];
    const from = at(machine, key).s;
    if (!ev) throw met("lifecycle", new Fault("bug", `lifecycle: ${machine} has no event "${event}"`, { d: { from } }));
    if (ev.from !== "*" && !ev.from.includes(from))
      throw met("lifecycle", new Fault("bug", `lifecycle: ${machine} ${event} from ${from}`, { message: `${machine}: "${event}" is not allowed from ${from}`, d: { from, event } }));
    const ok = ev.guard ? ev.guard(detail) : true;
    if (ok !== true) throw met("lifecycle", new Fault("refused", `lifecycle: ${machine} ${event} refused`, { message: `${machine}: ${ok}`, d: { from, event } }));
    const to = ev.to ?? from;
    states.set(`${machine}|${key}`, { s: to, since: Date.now(), event, detail });
    if (m.journal) {
      pending.push({ m: machine, key: String(key), from, to, event, at: Date.now(), v: String(ctx.shaOf?.(machine === "group" ? "keys" : "lifecycle") ?? "").slice(0, 12), ...detail });
      flushing ??= setTimeout(flush, 3000);
    }
    if (!m.quiet?.includes(event)) {
      ctx.log(`lifecycle ${machine}`, { what: `${String(key).slice(0, 12)}: ${from} → ${to} (${event})` });
      report(machine, key, from, to, event, detail);
    }
    dispatchEvent(new CustomEvent("craftworks:lifecycle", { detail: { machine, key, from, to, event } }));
    progressOf(machine, key, from, to, detail);
    // A finished one-per-change machine (a save HELD) is not kept: the page may make thousands.
    if (m.forget?.includes(to)) states.delete(`${machine}|${key}`);
    for (const f of listeners.get(machine) ?? []) Promise.resolve(f(key, to, from, event, detail)).catch(e => met("lifecycle", e));
    return to;
  }

  // A JOB: `run()` every `every` ms (and once at the start, after `delay`), only while `needs()` says true; its why-not
  // otherwise (a string: the job is PAUSED with it, and resumes when it holds again). One run at a time; a run that
  // throws is met as a fault and the job goes on. `stop()` ends it for good. The page's jobs, as they stand: `jobs()`.
  const running = new Map();
  function job(name, { every, needs = async () => true, run, delay = 0 }) {
    if (running.has(name)) return running.get(name);
    let timer = null;
    let stopped = false;
    let ticking = false; // ONE tick at a time (a tick awaits its needs: another must not move the state meanwhile)
    const tick = async () => {
      if (stopped || ticking) return;
      ticking = true;
      try {
        await tickOnce();
      } finally {
        ticking = false;
      }
    };
    const tickOnce = async () => {
      if (at("job", name).s === "RUNNING") return;
      const ok = await Promise.resolve()
        .then(needs)
        .catch(e => `its needs could not be read: ${e?.message ?? e}`);
      if (stopped) return;
      const s = at("job", name).s;
      if (ok !== true) {
        if (s !== "PAUSED") transition("job", name, "pause", { why: String(ok).slice(0, 120) });
        return;
      }
      if (s === "PAUSED") transition("job", name, "resume", {});
      transition("job", name, "run");
      try {
        await run();
      } catch (e) {
        met(`job ${name}`, e);
      } finally {
        if (at("job", name).s === "RUNNING") transition("job", name, "done");
      }
    };
    const start = () => {
      tick();
      timer = setInterval(tick, every);
    };
    if (delay) setTimeout(() => !stopped && start(), delay);
    else start();
    const handle = {
      now: tick,
      stop: () => {
        stopped = true;
        clearInterval(timer);
        transition("job", name, "stop", {});
        running.delete(name);
      },
    };
    running.set(name, handle);
    return handle;
  }
  // DELETE `what` (a kind: "item", "space", "device") through its machine: `fn()` does it; `ends` says how it ends
  // ("hidden" or "gone"). A refusal is recorded with why, and thrown on.
  async function deleting(what, id, fn, ends = "gone") {
    const key = `${what}:${String(id).slice(0, 24)}`;
    transition("delete", key, "request", { what });
    try {
      const r = await fn();
      transition("delete", key, ends, { what });
      return r;
    } catch (e) {
      transition("delete", key, "refuse", { what, why: String(e?.message ?? e).slice(0, 120) });
      throw e;
    }
  }
  const jobs = () => [...running.keys()].map(n => ({ name: n, s: at("job", n).s }));

  // The move `event` names, only if it changes the state (a state found again each round is not a move).
  const become = (machine, key, event, detail = {}) => {
    const now = state(machine, key).s; // (an unknown machine: the table's bug fault)
    return MACHINES[machine].events[event]?.to === now ? now : transition(machine, key, event, detail);
  };

  const on = (machine, fn) => listeners.set(machine, [...(listeners.get(machine) ?? []), fn]);
  // Where every process stands now (for the visit report): `{ machine: { key: state } }`.
  const snapshot = () => {
    const out = {};
    for (const [k, v] of states) {
      const [m, key] = k.split("|");
      (out[m] ??= {})[String(key).slice(0, 12)] = v.s;
    }
    return out;
  };

  // How many of a machine's processes stand in `s` now (a save OWED: what an update waits for).
  const count = (machine, s) => [...states].filter(([k, v]) => k.startsWith(`${machine}|`) && v.s === s).length;
  return { MACHINES, state, count, transition, become, on, snapshot, job, jobs, deleting, journal, keepNow };
}
