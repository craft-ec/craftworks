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
  const MACHINES = {
    // THE SESSION (who this page is for). OPEN_KEYLESS: logged in, but the account's keys did not come — it must not
    // write to the card, key packages, welcomes or asks (gated from phase 4), and says "keys not recovered".
    session: {
      initial: "GUEST",
      events: {
        open: { from: ["GUEST", "LOCKED", "OPEN", "OPEN_KEYLESS"], to: "OPEN" },
        keyless: { from: ["GUEST", "LOCKED", "OPEN", "OPEN_KEYLESS"], to: "OPEN_KEYLESS" },
        lock: { from: "*", to: "LOCKED" },
        removed: { from: "*", to: "REMOVED" },
      },
    },
    // THE ACCOUNT'S KEY GROUP, on this device. Only a REGISTRATION makes a group, and only where the node answered the
    // account's channel is not there; a words login walks the channel's history and joins.
    account: {
      initial: "NONE",
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
    ctx
      .require("index")
      .then(ix => ix.health({ k: `lifecycle-${machine}`, s: String(key).slice(0, 12), to: `${from}>${to}`, why: `${event}: ${from} → ${to}`, d: detail }))
      .catch(() => {});

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
    ctx.log(`lifecycle ${machine}`, { what: `${String(key).slice(0, 12)}: ${from} → ${to} (${event})` });
    report(machine, key, from, to, event, detail);
    dispatchEvent(new CustomEvent("craftworks:lifecycle", { detail: { machine, key, from, to, event } }));
    for (const f of listeners.get(machine) ?? []) Promise.resolve(f(key, to, from, event, detail)).catch(e => met("lifecycle", e));
    return to;
  }

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

  return { MACHINES, state, transition, on, snapshot };
}
