// LIFECYCLES, a service (FOUNDATION.md §1, §3): every process the system runs has its states written down HERE, as
// data — what it may move to, from where, and on what condition — and moves only through `transition`. Nobody keeps
// their own flags: they ask `state`. A move from a state the table does not allow is a `bug` fault; a guard that says
// no is a `refused` one — both thrown, so nothing happens on a wrong assumption. Every move is one health report
// (`lifecycle-<machine>`: from, to, the event and its details), and what each machine ended in rides in the visit.
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
    // A SPACE's membership on this device, as upkeep finds it each round: HELD (its keys current, on the owner's
    // branch), BEHIND (an older epoch), OFF_BRANCH, NOT_HELD (no group here), UNREADABLE (tables it cannot read),
    // WELCOME_FAILED{reason}, REMOVED. Moved only when it changes (`become`).
    space: {
      initial: "UNKNOWN",
      events: {
        held: { from: "*", to: "HELD" },
        behind: { from: "*", to: "BEHIND" },
        "off-branch": { from: "*", to: "OFF_BRANCH" },
        "not-held": { from: "*", to: "NOT_HELD" },
        unreadable: { from: "*", to: "UNREADABLE" },
        "welcome-failed": { from: "*", to: "WELCOME_FAILED" },
        removed: { from: "*", to: "REMOVED" },
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
      progress: { REQUESTED: (_, d) => `Deleting${d?.what ? ` ${String(d.what).slice(0, 40)}` : ""}…` },
      ended: { REFUSED: { text: "Not deleted (the trace says why)", ms: 15000 } },
      events: {
        request: { from: "*", to: "REQUESTED" },
        hidden: { from: ["REQUESTED"], to: "HIDDEN" },
        gone: { from: ["REQUESTED"], to: "GONE" },
        refuse: { from: ["REQUESTED"], to: "REFUSED" },
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
      .then(ix => ix.health({ k: `lifecycle-${machine}`, s: String(key).slice(0, 12), to: `${from}>${to}`, why: `${event}: ${from} → ${to}`, d: detail })));

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
      const show = () => (lines.set(id, textOf(now, key, detail)), say());
      if (m.after) timers.set(id, setTimeout(show, m.after));
      else show();
      if (was) say();
      return;
    }
    if (end && (!end.from || end.from.includes(from))) {
      lines.set(id, textOf(end.text, key, detail));
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
    const to = ev.to;
    states.set(`${machine}|${key}`, { s: to, since: Date.now(), event, detail });
    if (!m.quiet?.includes(event)) {
      ctx.log(`lifecycle ${machine}`, { what: `${String(key).slice(0, 12)}: ${from} → ${to} (${event})` });
      report(machine, key, from, to, event, detail);
    }
    dispatchEvent(new CustomEvent("craftworks:lifecycle", { detail: { machine, key, from, to, event } }));
    progressOf(machine, key, from, to, detail);
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
  const become = (machine, key, event, detail = {}) => (MACHINES[machine]?.events[event]?.to === at(machine, key).s ? at(machine, key).s : transition(machine, key, event, detail));

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

  return { MACHINES, state, transition, become, on, snapshot, job, jobs, deleting };
}
