// FAULTS, a service (FOUNDATION.md §4): what a failure IS, once, for every package — the failure edge of every
// lifecycle. A fault has a KIND (what happened), a stable WHAT (what was being done: the same words every time, so a
// report keeps one identity however its details change) and its details `d` (the diagnosis: ms, epochs, held …).
// ONE table (`POLICY`) decides, by kind, whether it is tried again and whether it is reported; nobody else does.
//
//   const fault = await ctx.require("fault");
//   throw new fault.Fault("transient", "reading notes", { message: "…", d: { ms } });
//   await fault.attempt("files", "putting a piece", () => put(p))     // tried again by kind, reported once
//   rows = await read().catch(fault.absentOnly)                         // null for absent / not yet; the rest thrown
//   el.textContent = fault.say(e)                                       // what a person reads, by kind
export async function start(ctx) {
  const KINDS = ["refused-expected", "login-needed", "not-yet", "absent", "sealed", "transient", "refused", "cancelled", "extension", "bug"];

  class Fault extends Error {
    // `message`: what the trace says (the details), defaulting to `what`. `cause`: the error this one classifies.
    constructor(kind, what, { message, cause, d = {} } = {}) {
      super(message ?? what);
      if (!KINDS.includes(kind)) throw new Error(`fault: no kind "${kind}"`);
      this.name = "Fault";
      this.kind = kind;
      this.what = String(what);
      this.d = d;
      if (cause !== undefined) this.cause = cause;
    }
  }

  // RETRIES AND REPORTS, by kind. `tries`: how many times `attempt` runs its work in all; `wait(n)`: before try n+1.
  // `report`: "always" (every one, once an hour per kind|source|what), "final" (only when the last try failed), "never".
  const POLICY = {
    transient: { tries: 4, wait: n => Math.min(5e3 * 2 ** n, 3e5), report: "final" },
    "not-yet": { tries: 6, wait: () => 1e4, report: "final" },
    sealed: { tries: 1, report: "always" },
    refused: { tries: 1, report: "always" },
    bug: { tries: 1, report: "always" },
    "refused-expected": { tries: 1, report: "never" },
    "login-needed": { tries: 1, report: "never" },
    extension: { tries: 1, report: "never" },
    absent: { tries: 1, report: "never" },
    cancelled: { tries: 1, report: "never" },
  };

  // A BROWSER EXTENSION's own error (its stack wholly in the extension: a wallet extension failing to connect).
  const extensionStack = e => {
    const lines = String(e?.stack ?? "").split("\n").slice(1).filter(l => l.trim());
    return lines.length > 0 && lines.every(l => /(chrome|moz|safari(-web)?)-extension:\/\//.test(l));
  };
  const text = e => e?.message ?? (typeof e === "string" ? e : (() => { try { return JSON.stringify(e); } catch { return String(e); } })());

  // ANYTHING THROWN, as a fault. What is not one yet is a `bug` (a thrown Error nobody classified) — unless it is an
  // extension's, an abort, or an older `expected` refusal.
  function classify(e) {
    if (e instanceof Fault) return e;
    if (e?.expected) return new Fault("refused-expected", text(e), { cause: e });
    // A KIND it carries already (a fault that crossed an app's frame comes back an Error with its kind): kept.
    if (KINDS.includes(e?.kind)) return new Fault(e.kind, text(e), { cause: e });
    if (extensionStack(e)) return new Fault("extension", "a browser extension", { message: text(e), cause: e });
    if (e?.name === "AbortError") return new Fault("cancelled", text(e), { cause: e });
    return new Fault("bug", text(e), { cause: e });
  }

  // THE REPORTER: the loader's (it tags each report with the route, the app, what was starting). Until it is set,
  // nothing is reported — and said so in the trace.
  let reporter = null;
  const reportTo = f => (reporter = f);
  function report(source, f) {
    if (!reporter) return ctx.log("fault", { what: `${f.kind} not reported (no reporter yet): ${f.what}` });
    reporter(f.kind, source, f.what, { ...f.d, said: f.message === f.what ? undefined : String(f.message).slice(0, 160) });
  }
  // A fault met at its end (nobody tries it again): reported if its kind says so — ONCE, however many places it
  // reaches (one rejection is met by every package awaiting it, and by the page).
  const REPORTED = Symbol("reported");
  // `extra`: what the reporter adds to its details (where it was met).
  const met = (source, e, extra = {}) => {
    const f = classify(e);
    const mark = e !== null && typeof e === "object" ? e : f;
    if (mark[REPORTED]) return f;
    mark[REPORTED] = true;
    if (POLICY[f.kind].report !== "never") report(source, Object.assign(f, { d: { ...f.d, ...extra } }));
    return f;
  };

  // RUN `fn`, tried again by its fault's kind (`policy` overrides per call: `{ transient: { tries: 10 } }`), reported
  // once — the last failure, with how many tries it took — then thrown as the fault.
  async function attempt(source, what, fn, { policy = {} } = {}) {
    for (let n = 0; ; n++) {
      try {
        return await fn(n);
      } catch (e) {
        const f = classify(e);
        const p = { ...POLICY[f.kind], ...(policy[f.kind] ?? {}) };
        if (n + 1 < (p.tries ?? 1)) {
          ctx.log("retry", { what: `${source}: ${what} (${f.kind}, try ${n + 2} of ${p.tries}): ${f.message}` });
          await new Promise(r => setTimeout(r, p.wait?.(n) ?? 0));
          continue;
        }
        if (p.report !== "never") report(source, Object.assign(f, { d: { ...f.d, tries: n + 1, doing: what } }));
        throw f;
      }
    }
  }

  // ABSENT IS AN ANSWER: `.catch(absentOnly)` gives null for a read that found nothing (or nothing yet), and throws
  // every other failure on — never "nothing there" for a refusal, a sealed table or a bug.
  const absentOnly = e => {
    const f = classify(e);
    if (f.kind === "absent" || f.kind === "not-yet") return null;
    throw f;
  };

  // WHAT A PERSON READS, by kind: one wording each, never a stack or a contract id.
  const WORDS = {
    "login-needed": "Log in to do that.",
    "not-yet": "Still arriving — try again in a moment.",
    transient: "Your node did not answer in time — try again in a moment.",
    sealed: "This is sealed with keys this device does not have yet.",
    absent: "Not found.",
    cancelled: "Stopped.",
  };
  const say = e => {
    const f = classify(e);
    // (A failure with no words of its kind says its own: the refusals apps throw as plain errors read as they were.)
    return WORDS[f.kind] ?? f.message;
  };
  const present = (el, e) => ((el.textContent = say(e)), el);

  return { KINDS, Fault, POLICY, classify, met, attempt, absentOnly, say, present, reportTo };
}
