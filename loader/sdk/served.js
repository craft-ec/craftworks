// THE ONE FETCH, and the race of the load pieces (split from artefacts.js, sdk#347): what a published app's
// STARTER carries, because it runs before the SDK exists. The content-hash CACHE of the SDK's artefacts is
// artefacts.js, which fetches through this.
//
// # A node that does not hold it YET has not answered (sdk#312)
//
// A 404, a 503, a 400 or a network error for an artefact the app names is NOT
// an end: a node serves a web container only once it holds it, and one that
// is still fetching or re-storing it answers "not found" in the meantime (the
// owner saw an app fail on a 404 and open on a reload). This fetch is a path
// to the node like any other, so it keeps the owner's rules 7 and 8: re-asked
// until answered, on the PAGE'S back-off (`rto.js`, generated from
// page/src/rto.rs — not a second one written here), with no give-up timer.
// While it waits it says so (`onWait`). What ends it: bytes that do not hash
// to the name (a MISMATCH, refused by name), or a person cancelling
// (`signal`).

import { RTO_SCHEDULE_MS, STILL_FETCHING_AFTER_MS, STILL_FETCHING_REASK_MS, WINDOW_AFTER_ANSWERS } from "./rto.js";
import { COARSEN_BANDS, COARSEN_LAST_GRAIN, EVENT, HTTP_CLASSES, KEY, OUTCOME, SITE, STATUS, STRIDE } from "./instrument-vocab.js";

/** A time, coarsened by the recording vocabulary's own rule (generated from craftworks-instrument's bands). */
export const coarsenMs = ms => {
  for (const [below, grain] of COARSEN_BANDS) if (ms < below) return Math.floor(ms / grain) * grain;
  return Math.floor(ms / COARSEN_LAST_GRAIN) * COARSEN_LAST_GRAIN;
};

/** An HTTP status's class (generated from craftworks-instrument's table). */
export const statusClassOf = status => {
  for (const [lo, hi, cls] of HTTP_CLASSES) if (status >= lo && status <= hi) return cls;
  return STATUS.OtherHttp;
};

/**
 * THE LOADER'S RECORDING (sdk#386's instrument work, the architect's design): every fetch round the loader makes
 * before the SDK exists, as NUMBERS from the generated vocabulary -- `[kind, site, a, b, c]` per event, never a URL,
 * a hash or a key. The spine's rules: BOUNDED (the last `capacity` events kept; older ones overwritten and COUNTED),
 * a per-ring SEQUENCE (the fetch's ordinal), times only as COARSENED offsets from the ring's start, and no read-back
 * into the loader's logic -- the loader only writes. When the SDK exists, `take()` hands the ring to the page
 * ONCE (`Session.adopt_loader`), where Rust validates every number; after it, the ring records nothing.
 */
export class LoaderRing {
  constructor(capacity = 1024, now = () => Date.now()) {
    this.cap = capacity;
    this.buf = new Array(capacity * STRIDE).fill(0);
    this.n = 0;
    this.at = 0;
    this.dropped = 0;
    this.fetches = 0;
    this.now = now;
    this.startMs = now();
    this.taken = false;
  }

  push(kind, a, b, c) {
    if (this.taken) return;
    this.buf.splice(this.at * STRIDE, STRIDE, kind, SITE["loader::fetch"], a, b, c);
    this.at = (this.at + 1) % this.cap;
    if (this.n < this.cap) this.n += 1;
    else this.dropped += 1;
  }

  /** A fetch round ASKED: its ordinal (the ring's own sequence), its attempt, and when. */
  asked(round) {
    this.fetches += 1;
    const o = this.fetches;
    this.push(EVENT.EDGE, 0, o, 0);
    this.push(EVENT.COUNTER, o, KEY.Attempts, round + 1);
    this.push(EVENT.COUNTER, o, KEY.OffsetMs, coarsenMs(this.now() - this.startMs));
    return o;
  }

  /** An HTTP ANSWER, any status: the class of it. */
  answered(o, status) {
    this.push(EVENT.EDGE, 1, o, 0);
    this.push(EVENT.COUNTER, o, KEY.StatusClass, statusClassOf(status));
  }

  /** No HTTP answer: withdrawn on this side (a cancel), or none at all (a network error). */
  ended(o, cancelled) {
    this.push(EVENT.EXIT, o, cancelled ? OUTCOME.Withdrawn : OUTCOME.Timeout, 0);
    this.push(EVENT.COUNTER, o, KEY.StatusClass, cancelled ? STATUS.Abort : STATUS.NetworkError);
  }

  /** The events, oldest first. */
  events() {
    const full = this.n === this.cap;
    const out = [];
    for (let k = 0; k < this.n; k += 1) {
      const i = ((full ? this.at : 0) + k) % this.cap;
      out.push(...this.buf.slice(i * STRIDE, i * STRIDE + STRIDE));
    }
    return out;
  }

  /** The handover, ONCE: when the loader started, its events, and how many the ring lost. It records nothing after. */
  take() {
    const seg = { startMs: this.startMs, events: this.events(), dropped: this.dropped };
    this.taken = true;
    return seg;
  }
}

/** The loader's one ring: every `served()` records into it unless handed another. */
export const loaderRing = new LoaderRing();

/**
 * The loader's rounds, rendered for a person -- ONLY for when the SDK never loaded (otherwise they are in the page's
 * recording, `Session.page_trace`). Names from the generated vocabulary; nothing else is in the ring to name.
 */
export function loaderTrace(ring = loaderRing) {
  const name = (table, v) => Object.keys(table).find(k => table[k] === v) ?? `#${v}`;
  const ev = ring.events();
  const lines = [`loader: ${ev.length / STRIDE} events, ${ring.dropped} dropped`];
  for (let i = 0; i < ev.length; i += STRIDE) {
    const [kind, , a, b, c] = ev.slice(i, i + STRIDE);
    if (kind === EVENT.EDGE) lines.push(`  fetch#${b} ${a === 0 ? "asked" : "answered"}`);
    else if (kind === EVENT.EXIT) lines.push(`  fetch#${a} ${name(OUTCOME, b)}`);
    else lines.push(`  fetch#${a} ${name(KEY, b)}=${b === KEY.StatusClass ? name(STATUS, c) : c}`);
  }
  return lines.join("\n");
}

const hex = bytes =>
  [...new Uint8Array(bytes)].map(b => b.toString(16).padStart(2, "0")).join("");

/** Storage (and digesting) may throw for reasons that are not this app's fault. */
export async function best(effort, fallback = null) {
  try {
    return await effort();
  } catch {
    return fallback;
  }
}

/** THE one wait between rounds: `ms`, or less if a person cancels (`sig`). */
export const sleepFor = (ms, sig) =>
  new Promise(ok => {
    const t = setTimeout(ok, ms);
    sig?.addEventListener?.("abort", () => { clearTimeout(t); ok(); }, { once: true });
  });

/**
 * THE ONE FETCH (#126 ruling): every file a page or a tool fetches from the
 * node over HTTP comes through here — a web container's own files, the SDK's
 * artefacts, a builder's files. Re-asked until answered on the PAGE'S back-off
 * (`rto.js`, generated from page/src/rto.rs and pinned to it by a test), with
 * no give-up of its own: a 404, a 5xx or a network error is "not held yet",
 * never an end (rules 7, 8). While it waits it says so (`onWait`, naming the
 * file). What ends it: a person's cancel (`signal`), or — only where the
 * caller passes a `check` — every source ANSWERING with bytes the check
 * refuses as final.
 *
 * THE BOOTSTRAP EXCEPTION to rule 5 (architect, #126): node DATA goes through
 * the page's one sender, `Page::send`, which does not exist until the SDK's
 * wasm is loaded. The files that load it — and the code and web files a node
 * serves over HTTP — can only be fetched here. Nothing fetched AFTER load that
 * is node data may come this way.
 *
 * `{ url }` or `{ urls }` (never both: taking either silently would drop a
 * source the caller supplied). `check(bytes, from)`: `null` to accept, or
 * `{ says, answer }` — `answer` true when this source has given its final word
 * (then, from EVERY source, the fetch is refused, by `refusal`).
 */
export async function served(
  { url, urls },
  {
    fetch: fetchWith = typeof fetch === "function" ? fetch : null,
    onWait = null,
    signal = null,
    sleep = sleepFor,
    now = () => Date.now(),
    // The loader's recording (above): every round asked and how it ended, as codes.
    rec = loaderRing,
    check = null,
    name = null,
    refusal = "a refusal",
    // Options for every request (e.g. `{ cache: "no-store" }` for a file that
    // must be read fresh). Passed as given; never a reason to end.
    init = null,
  } = {},
) {
  const label = name ?? (url ?? (urls ?? []).join(", "));
  if (url && urls) {
    throw new Error(`${label} was given both \`url\` and \`urls\`; pass one. Taking either silently would drop a source the caller supplied.`);
  }
  const sources = urls ?? (url ? [url] : []);
  if (sources.length === 0) throw new Error(`${label} has no url to fetch it from`);
  const started = now();
  // THE NODE IS STILL FETCHING IT (sdk#447; main's re-rule after batch 4's realnet): a non-200 that took the node's
  // web bound (its 503 at ~30 s) leaves the node's GET RUNNING, and asking again starts ANOTHER network GET of the same
  // key beside it (the node shares none: the stall review, V20's four concurrent streams). So such a source is not
  // re-asked on the RTO: after its n-th slow answer it waits `STILL_FETCHING_REASK_MS[n]` (the last repeating). The
  // first wait is one RTO floor, because a node that has FINISHED answers a re-ask from its own store at once, and a
  // hold of B took a piece the node already had up to B late (V at "still fetching (49 s)"). The waits then double up
  // to B, so a node truly still fetching gets few duplicates. A FAST non-200 (a node still joining, a refusal) left no
  // GET running: re-asked on the RTO as before. One classification: builder#153's waiting page takes it from here.
  const heldUntil = new Map();
  const slowAnswers = new Map();
  for (let round = 0; ; round += 1) {
    // Every failure is kept, so a wait can say what each source actually did.
    const failures = [];
    // The HTTP statuses of the answers that were not the file (a node that does not hold it answers one): told to
    // `onWait`, so a caller can tell "answered, not held" from "no answer yet" (the load pieces' repair, sdk#347).
    const statuses = [];
    let answered = 0;
    for (const from of sources) {
      const hold = heldUntil.get(from);
      if (hold !== undefined && now() < hold) {
        failures.push(`${from}: the node is still fetching it`);
        continue;
      }
      heldUntil.delete(from);
      const sentAt = now();
      let res;
      // THE ONE SITE of the loader's recording: every request this fetch makes, and its end.
      const asked = rec?.asked(round);
      try {
        res = await (init ? fetchWith(from, init) : fetchWith(from));
      } catch (e) {
        rec?.ended(asked, signal?.aborted === true);
        failures.push(`${from}: ${e?.message ?? e}`);
        continue;
      }
      rec?.answered(asked, res.status);
      if (!res.ok) {
        // NOT AN ANSWER about the file: the node does not hold it yet.
        failures.push(`${from}: ${res.status}`);
        statuses.push(res.status);
        if (now() - sentAt >= STILL_FETCHING_AFTER_MS) {
          const n = slowAnswers.get(from) ?? 0;
          slowAnswers.set(from, n + 1);
          heldUntil.set(from, now() + STILL_FETCHING_REASK_MS[Math.min(n, STILL_FETCHING_REASK_MS.length - 1)]);
        } else {
          slowAnswers.delete(from);
        }
        continue;
      }
      // `fetch` resolves on the HEADERS; the body can still fail, and that is
      // this source's failure too — the next source is asked.
      let got;
      try {
        got = new Uint8Array(await res.arrayBuffer());
      } catch (e) {
        failures.push(`${from}: ${e?.message ?? e}`);
        continue;
      }
      const refused = check ? await check(got, from) : null;
      if (!refused) return got;
      if (refused.answer) answered += 1;
      failures.push(`${from}: ${refused.says}`);
    }
    // NAMES WHICH FILE AND WHAT EACH SOURCE DID: the first thing a person can
    // send when an app will not open.
    const what = failures.join("\n  ");
    if (answered === sources.length) {
      throw new Error(`${label} refused: ${refusal} from every source:\n  ${what}`);
    }
    const waitedMs = now() - started;
    if (signal?.aborted) {
      throw new Error(`${label}: cancelled after ${Math.round(waitedMs / 1000)} s unanswered:\n  ${what}`);
    }
    // The wait still TICKS on the RTO while every source is held (main's (B)): `onWait` keeps counting the seconds
    // unanswered, and the next ask is never later than the earliest hold's end.
    const holds = sources.map(s => heldUntil.get(s)).filter(h => h !== undefined && h > now());
    const stillFetching = holds.length === sources.length;
    const rto = RTO_SCHEDULE_MS[Math.min(round, RTO_SCHEDULE_MS.length - 1)];
    const nextMs = stillFetching ? Math.min(rto, Math.min(...holds) - now()) : rto;
    onWait?.({
      waitedMs,
      nextMs,
      failures,
      statuses,
      stillFetching,
      says: stillFetching
        ? `loading the app… the node is still fetching it (${Math.round(waitedMs / 1000)} s)`
        : `loading the app… not available on this node yet (${Math.round(waitedMs / 1000)} s)`,
    });
    await sleep(nextMs, signal);
    if (signal?.aborted) {
      throw new Error(`${label}: cancelled after ${Math.round((now() - started) / 1000)} s unanswered:\n  ${what}`);
    }
  }
}

/**
 * RACE THE LOAD PIECES (sdk#347, the owner's rule 11 applied to loading the SDK): `spec.pieces` is the `k + m`
 * pieces of the SDK's bundle (`{ url | urls, sha256 }`, data first), and the bundle needs ANY `k` of them. Each
 * piece is asked through `served()` (the one fetch: re-asked on the RTO, never ended by a status) and checked by its
 * sha256; a piece that verifies counts, one with the wrong bytes does not. At most `WINDOW_AFTER_ANSWERS[answers]`
 * are in flight (the page's own window, generated from page/src/rto.rs: no second pacing rule). At the FIRST `k`
 * verified it resolves and ABORTS every piece still asked: a stalled piece never holds the page.
 *
 * No time cut-off (rule 8): while fewer than `k` have verified, it waits and `onWait` says so. It rejects only
 * when every piece has REFUSED (wrong bytes from every source), or on the person's cancel (`signal`).
 *
 * Resolves `{ pieces, verified, asked, notHeld }`: `notHeld` the pieces a source answered NOT FOUND (404) and that
 * never arrived -- the only ones a repair may PUT back (a piece cancelled at k was not missing). `pieces[i]` the
 * verified bytes of piece `i` or `null`; `asked` the
 * indices that were asked (the rest were never needed). What the loader's repair later re-PUTs is what was asked
 * and did not arrive.
 */
export async function raceK(
  spec,
  {
    fetch: fetchWith = typeof fetch === "function" ? fetch : null,
    subtle = typeof crypto === "object" ? crypto.subtle : null,
    onWait = null,
    signal = null,
    sleep = sleepFor,
    now = () => Date.now(),
  } = {},
) {
  const { k, m, pieces } = spec;
  if (!Array.isArray(pieces) || pieces.length !== k + m || k < 1) {
    throw new Error(`raceK: ${pieces?.length} pieces, not k + m = ${k + m}`);
  }
  if (!subtle) throw new Error("no crypto.subtle: cannot verify a piece");
  const stop = new AbortController();
  const cancel = () => stop.abort();
  signal?.addEventListener?.("abort", cancel, { once: true });
  const got = new Array(k + m).fill(null);
  const asked = [];
  // Pieces the node ANSWERED "not held" (404) at least once: the only ones the repair may PUT back. A piece
  // still in flight when the k-th verified was cancelled by this race, not missing, and is never repaired.
  const notHeld = new Set();
  let verified = 0;
  let refused = 0;
  let answers = 0;
  let inFlight = 0;
  let next = 0;
  const window = () => WINDOW_AFTER_ANSWERS[Math.min(answers, WINDOW_AFTER_ANSWERS.length - 1)];
  try {
    return await new Promise((resolve, reject) => {
      const settle = () => {
        if (verified >= k) {
          stop.abort();
          resolve({ pieces: got, verified, asked, notHeld: [...notHeld].filter(i => !got[i]).sort((a, b) => a - b) });
        } else if (stop.signal.aborted) {
          reject(new Error(`the SDK's pieces: cancelled with ${verified} of ${k} verified`));
        } else if (refused > m) {
          stop.abort();
          reject(new Error(`the SDK's pieces refused: ${refused} of ${k + m} gave bytes that do not hash to their name, so fewer than k = ${k} can verify`));
        } else {
          launch();
        }
      };
      const launch = () => {
        while (next < k + m && inFlight < window()) {
          const i = next;
          next += 1;
          inFlight += 1;
          asked.push(i);
          const p = pieces[i];
          served(p.urls ? { urls: p.urls } : { url: p.url }, {
            // The abort reaches the request itself, not only the next round: a piece no longer needed stops now.
            fetch: (u, init) => fetchWith(u, { ...(init ?? {}), signal: stop.signal }),
            signal: stop.signal,
            sleep,
            now,
            name: `SDK piece ${i}`,
            refusal: "bytes that do not hash to the piece's sha256",
            check: async bytes => ((await matches(bytes, p.sha256, subtle)) ? null : { answer: true, says: "the wrong bytes" }),
            onWait: w => {
              // A source's own NOT FOUND (404) only: a 503 is "not ready" (a node still joining), and silence or a
              // network error is no answer at all (rule 8: re-asked, never taken for "missing").
              if (w.statuses?.includes(404)) notHeld.add(i);
              onWait?.({ ...w, piece: i, verified, k });
            },
          }).then(
            bytes => {
              got[i] = bytes;
              verified += 1;
              answers += 1;
            },
            () => {
              // Cancelled (the race is over) or refused (the wrong bytes from every source): not counted.
              if (!stop.signal.aborted) refused += 1;
            },
          ).finally(() => {
            inFlight -= 1;
            settle();
          });
        }
      };
      launch();
    });
  } finally {
    signal?.removeEventListener?.("abort", cancel);
  }
}

/** A file's text, through the one fetch. */
export async function servedText(source, deps) {
  return new TextDecoder().decode(await served(source, deps));
}

export async function digestOf(bytes, subtle) {
  if (!subtle) throw new Error("no crypto.subtle: cannot verify an artefact");
  return best(async () => hex(await subtle.digest("SHA-256", bytes)));
}

export async function matches(bytes, sha256, subtle) {
  return (await digestOf(bytes, subtle)) === sha256;
}
