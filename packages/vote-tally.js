// VOTE TALLY, a service: what Vote's records ARE and how every reader counts them the same way (ARCHITECTURE §7, Vote)
// — no store of its own: every record is an ITEM (`items`, kinds in `kinds`), written in its writer's own feed (a
// space's room: each member's own; a person's: their own tail, pointed in the item's bag).
// - A POLL is an item of kind `poll`: its question the title; in `meta` its OPTIONS, `multiple` (several may be
//   chosen), `open` (`closedAt` when closed), `results` ("live" or "after": shown once closed).
// - A PROPOSAL is an item of kind `proposal`: its title and text; in `meta` an optional OUTCOME (`action`: one act the
//   space's acts already have — grant a role, assign a composed role, set a policy) and a `quorum` (votes needed).
// - A BALLOT (kind `ballot`, attached to a poll or a proposal): `meta.seq`, `meta.choice` — a poll's options chosen
//   (a list; empty = withdrawn), a proposal's "yes" | "no" | "abstain" (null = withdrawn: back to the delegation).
// - A DELEGATION (kind `delegation`, standing on its own in a SPACE: global per space in v1): `meta.seq`, `meta.to`
//   (a member's DID; null = revoked).
// - A TALLY (kind `tally`, attached to a proposal): its FINALIZATION — `meta.final` = { result, electorate, inputs,
//   hash }: the result, who could vote, the hashes of the records it counted, and the hash over all of them.
// Every record is APPEND-ONLY: a change is a new record with a higher `seq`; per account the HIGHEST seq counts, a tie
// (two devices of one account at once) going to the LOWER record hash. A record's hash is SHA-256 over what it says
// and its reference, computed by each reader from what it holds — never trusted as written.
//
// RESOLUTION (liquid democracy): each member of the electorate votes DIRECTLY (their newest ballot), or follows their
// DELEGATION transitively to the first member on the chain who voted directly — a direct vote always overrides one's
// own delegation; a chain that comes back on itself before reaching a direct vote (a CYCLE), that ends at someone who
// neither voted nor delegated, that leaves the electorate, or that is longer than DEPTH hops contributes NOTHING.
// Records of accounts outside the electorate count for nothing (they neither vote nor carry others' votes).
// FINALIZED (by the proposer or an admin): the result as it was over exactly the records named in its inputs;
// readers show that result whatever is written later, and check it (`verifyFinal`).
//
//   const V = await ctx.require("vote-tally");
//   const place = await V.of(at)     // `where.of(…)`'s place: yours, a person's, a space's
//   await place.polls()  await place.proposals()          // the items, newest first
//   await place.pollView(poll)       // { meta, tally, mine, may, records }
//   await place.vote(poll, choice)   // a ballot: a poll's options (a list) or a proposal's "yes" | "no" | "abstain" | null
//   await place.proposalView(p)      // { tally, resolution, final, verified, mine, participation, outcome, … }
//   await place.delegate(did | null) // this person's delegation in the space (null: revoked)
//   await place.finalize(p)  await place.applyOutcome(p)
// Pure parts (exported; `node --test tools/vote-tally.test.mjs`): normalizePoll, recordOf, recordHash, withHashes,
// newest, nextSeq, pollTally, resolve, proposalTally, snapshot, verifyFinal, pickFinal, outcomeProblem, outcomeNeed,
// outcomeText, appliedAct, participation, graphOf.

export const DEPTH = 10;
export const POLL = "poll";
export const PROPOSAL = "proposal";
export const DELEGATION = "delegation";
export const BALLOT = "ballot";
export const TALLY = "tally";
export const CHOICES = Object.freeze(["yes", "no", "abstain"]);
export const LIMITS = Object.freeze({ question: 300, options: 20, option: 200, inputs: 3000 });
const DID = /^did:[a-z]+:[A-Za-z0-9]{1,80}$/;
const HASH = /^[0-9a-f]{64}$/;
// What a finalization names of each input: its hash's first 32 hex (128 bits) — what one row holds for thousands.
const SHORT = 32;

// ── POLLS ───────────────────────────────────────────────────────────────────────────────────────────────────────────
// A poll's settings, cleaned: options trimmed and without repeats (two to LIMITS.options), the rest defaulted.
export function normalizePoll(meta = {}) {
  const seen = new Set();
  const options = (Array.isArray(meta.options) ? meta.options : [])
    .map(o => String(o ?? "").trim().slice(0, LIMITS.option))
    .filter(o => o && !seen.has(o) && seen.add(o))
    .slice(0, LIMITS.options);
  return {
    options,
    multiple: !!meta.multiple,
    open: meta.open !== false,
    closedAt: Number(meta.closedAt) || null,
    results: meta.results === "after" ? "after" : "live",
  };
}
// Why a poll cannot be made (null: it can).
export function pollProblem(question, meta) {
  const q = String(question ?? "").trim();
  if (!q) return "a poll needs a question";
  if (q.length > LIMITS.question) return `a question of at most ${LIMITS.question} characters`;
  if (normalizePoll(meta).options.length < 2) return "a poll needs at least two different options";
  return null;
}
// Is `choice` a valid ballot on poll settings `p` (a list of its options: one for single choice, one or more for
// multiple, each once)? An empty list is a WITHDRAWN vote (valid, counts nothing).
export function validPollChoice(p, choice) {
  if (!Array.isArray(choice)) return false;
  if (!choice.length) return true;
  if (new Set(choice).size !== choice.length) return false;
  if (!p.multiple && choice.length !== 1) return false;
  return choice.every(c => p.options.includes(c));
}

// ── RECORDS ─────────────────────────────────────────────────────────────────────────────────────────────────────────
// The item a record is about: a space's attached item names its target's id (`in`), a person's its whole reference.
export const targetOf = it => {
  const inn = String(it?.in ?? "");
  if (!inn) return null;
  if (inn.includes("/")) return inn;
  const ref = String(it?.ref ?? "");
  return ref.includes("/") ? `${ref.slice(0, ref.lastIndexOf("/"))}/${inn}` : null;
};
// An item (`items`' shape) read as a RECORD, or null (not one, or malformed). `on`: what a ballot or a tally is about.
export function recordOf(it) {
  const kind = it?.kind;
  if (![BALLOT, DELEGATION, TALLY].includes(kind)) return null;
  const m = it.meta ?? {};
  const seq = Number.isSafeInteger(m.seq) && m.seq >= 0 ? m.seq : null;
  if (seq === null || typeof it.by !== "string" || !it.by) return null;
  const r = { ref: String(it.ref ?? ""), kind, by: it.by, at: Number(it.at) || 0, seq, on: kind === DELEGATION ? null : targetOf(it), cred: typeof m.cred === "string" ? m.cred : null };
  if (kind === BALLOT) {
    if (!r.on) return null;
    r.choice = Array.isArray(m.choice) ? m.choice.map(String) : typeof m.choice === "string" ? m.choice : null;
  } else if (kind === DELEGATION) {
    if (m.to !== null && !(typeof m.to === "string" && DID.test(m.to))) return null;
    r.to = m.to;
    if (r.to === r.by) r.to = null; // to oneself: no delegation
  } else {
    const f = m.final;
    if (!r.on || !f || typeof f !== "object" || !HASH.test(String(f.hash)) || !Array.isArray(f.inputs) || !Array.isArray(f.electorate) || !f.result || typeof f.result !== "object") return null;
    r.final = { hash: f.hash, inputs: f.inputs.map(String), electorate: f.electorate.map(String), result: f.result };
  }
  return r;
}
const hex = buf => [...new Uint8Array(buf)].map(b => b.toString(16).padStart(2, "0")).join("");
export const sha256 = async s => hex(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(s)));
// What a record SAYS, in one canonical form (its reference included: two records never share a hash).
const said = r => JSON.stringify([r.kind, r.ref, r.by, r.on ?? null, r.seq, r.at, r.kind === BALLOT ? (r.choice ?? null) : r.kind === DELEGATION ? (r.to ?? null) : r.final.hash]);
export const recordHash = r => sha256(said(r));
export const withHashes = async records => Promise.all(records.filter(Boolean).map(async r => ({ ...r, h: await recordHash(r) })));

// NEWEST per account: the highest `seq`; a tie to the lower hash. → Map(by → record). Records need their hash.
export function newest(records) {
  const out = new Map();
  for (const r of records) {
    if (!r || !r.by || r.seq == null) continue;
    if (typeof r.h !== "string") throw new Error("a record without its hash: withHashes first");
    const had = out.get(r.by);
    if (!had || r.seq > had.seq || (r.seq === had.seq && r.h < had.h)) out.set(r.by, r);
  }
  return out;
}
// The seq this account's next record (of these) takes: one above its highest.
export const nextSeq = (records, by) => 1 + Math.max(0, ...records.filter(r => r && r.by === by && Number.isSafeInteger(r.seq)).map(r => r.seq));

// ── A POLL's TALLY ──────────────────────────────────────────────────────────────────────────────────────────────────
// One per account, the newest counting; only ELIGIBLE writers (`eligible(by, record)`: true — counted; false — not;
// null — not known yet, counted when it is); nothing written after the poll CLOSED (`closedAt`).
// → { counts: { option: n }, voters, choices: Map(by → [options]), ineligible, pending, late, invalid }
export function pollTally(poll, records, { eligible = () => true } = {}) {
  const p = normalizePoll(poll?.meta);
  const ref = poll?.ref;
  const cut = !p.open && p.closedAt ? p.closedAt : Infinity;
  let ineligible = 0;
  let pending = 0;
  let late = 0;
  const kept = [];
  for (const r of records) {
    if (!r || r.kind !== BALLOT || r.on !== ref) continue;
    if (r.at > cut) {
      late++;
      continue;
    }
    const ok = eligible(r.by, r);
    if (ok === null) pending++;
    else if (ok !== true) ineligible++;
    else kept.push(r);
  }
  const counts = Object.fromEntries(p.options.map(o => [o, 0]));
  const choices = new Map();
  let invalid = 0;
  for (const [by, r] of newest(kept)) {
    if (!validPollChoice(p, r.choice)) {
      invalid++;
      continue;
    }
    if (!r.choice.length) continue;
    choices.set(by, r.choice);
    for (const c of r.choice) counts[c]++;
  }
  return { counts, voters: choices.size, choices, ineligible, pending, late, invalid };
}

// ── DELEGATION ──────────────────────────────────────────────────────────────────────────────────────────────────────
// RESOLVE every member of the electorate: `direct` Map(did → choice) — their own vote; `delegations` Map(did → did |
// null). → Map(did → { choice, how, path }) — how: "direct" | "delegated" (path: the delegator to the voter whose
// choice it carries) | "none" (no vote, no delegation) | "stuck" (the chain ends at someone who neither voted nor
// delegated) | "outside" (it leaves the electorate) | "cycle" | "depth" (more than `depth` hops). Only "direct" and
// "delegated" carry a choice. Order-independent: each member walked alone.
export function resolve({ electorate, direct, delegations, depth = DEPTH }) {
  const E = new Set(electorate);
  const out = new Map();
  for (const v of [...E].sort()) {
    if (direct.has(v)) {
      out.set(v, { choice: direct.get(v), how: "direct", path: [v] });
      continue;
    }
    const path = [v];
    const seen = new Set(path);
    let cur = v;
    let res = null;
    for (let hops = 1; !res; hops++) {
      const to = delegations.get(cur) ?? null;
      if (!to) res = { how: cur === v ? "none" : "stuck" };
      else if (!E.has(to)) res = { how: "outside", path: [...path, to] };
      else if (seen.has(to)) res = { how: "cycle", path: [...path, to] };
      else if (hops > depth) res = { how: "depth" };
      else {
        path.push(to);
        seen.add(to);
        if (direct.has(to)) res = { how: "delegated", choice: direct.get(to) };
        cur = to;
      }
    }
    out.set(v, { choice: res.choice ?? null, how: res.how, path: res.path ?? path });
  }
  return out;
}

// A PROPOSAL's TALLY over records already hashed: the electorate (eligible DIDs), each one's newest ballot on the
// proposal (a valid choice: their direct vote; null: withdrawn) and newest delegation. → { result, resolution,
// direct, delegations, inputs } — `inputs`: the records that decided it (each account's newest of each).
export function proposalTally({ proposal, records, electorate, quorum = null }) {
  const E = [...new Set(electorate)].sort();
  const inE = new Set(E);
  const ref = proposal?.ref;
  const ballots = newest(records.filter(r => r && r.kind === BALLOT && r.on === ref && inE.has(r.by)));
  const dels = newest(records.filter(r => r && r.kind === DELEGATION && inE.has(r.by)));
  const direct = new Map([...ballots].filter(([, r]) => CHOICES.includes(r.choice)).map(([by, r]) => [by, r.choice]));
  const delegations = new Map([...dels].map(([by, r]) => [by, r.to ?? null]));
  const resolution = resolve({ electorate: E, direct, delegations });
  const result = { yes: 0, no: 0, abstain: 0, electorate: E.length };
  for (const x of resolution.values()) if (x.choice) result[x.choice]++;
  const q = Number(quorum ?? proposal?.meta?.quorum) || 1;
  result.quorum = q;
  result.passed = result.yes > result.no && result.yes + result.no + result.abstain >= q;
  const inputs = [...ballots.values(), ...dels.values()];
  return { result, resolution, direct, delegations, inputs };
}

// THE SNAPSHOT a finalization records: the result, the electorate, its inputs' hashes (short, sorted) and one hash over
// them all and the proposal. → { result, electorate, inputs, hash }
export async function snapshot(proposalRef, t, electorate) {
  const E = [...new Set(electorate)].sort();
  const inputs = [...new Set(t.inputs.map(r => r.h.slice(0, SHORT)))].sort();
  if (inputs.length > LIMITS.inputs) throw new Error(`more than ${LIMITS.inputs} records to finalize: not in v1`);
  const result = { ...t.result };
  return { result, electorate: E, inputs, hash: await snapshotHash(proposalRef, result, E, inputs) };
}
const RESULT_KEYS = ["yes", "no", "abstain", "electorate", "quorum", "passed"];
const snapshotHash = (ref, result, E, inputs) => sha256(JSON.stringify([ref, RESULT_KEYS.map(k => result[k] ?? null), E, inputs]));
const sameResult = (a, b) => RESULT_KEYS.every(k => (a?.[k] ?? null) === (b?.[k] ?? null));

// A FINALIZATION checked against what this reader holds: its hash over what it names ("forged" if not); every input
// it names held? ("partial": `missing` of them not here); the result recomputed over exactly those inputs and its
// electorate ("verified" | "mismatch"). Later records never enter: only the named inputs are counted.
export async function verifyFinal(final, proposal, records) {
  const f = final?.final ?? final;
  if ((await snapshotHash(proposal.ref, f.result, f.electorate, f.inputs)) !== f.hash) return { status: "forged" };
  const byShort = new Map(records.filter(r => r?.h).map(r => [r.h.slice(0, SHORT), r]));
  const held = f.inputs.map(h => byShort.get(h)).filter(Boolean);
  if (held.length < f.inputs.length) return { status: "partial", missing: f.inputs.length - held.length };
  const t = proposalTally({ proposal, records: held, electorate: f.electorate, quorum: f.result.quorum });
  return sameResult(t.result, f.result) ? { status: "verified", result: t.result } : { status: "mismatch", result: t.result };
}
// THE FINALIZATION that stands: the earliest (then lowest hash) tally record on the proposal by one who may finalize
// (`mayFinalize(by, at)`: the proposer, an admin) whose hash checks. Later ones change nothing.
export async function pickFinal(proposal, records, mayFinalize) {
  const cands = records.filter(r => r && r.kind === TALLY && r.on === proposal.ref && mayFinalize(r.by, r.at)).sort((a, b) => a.at - b.at || (a.h < b.h ? -1 : 1));
  for (const r of cands) if ((await snapshotHash(proposal.ref, r.final.result, r.final.electorate, r.final.inputs)) === r.final.hash) return r;
  return null;
}

// ── OUTCOMES ────────────────────────────────────────────────────────────────────────────────────────────────────────
// What a passed proposal may DO: one act the space's acts already have (`roles.act`, replayed by `gov`), and the
// permission that act needs — whoever holds it applies it (no new authority).
export const OUTCOMES = Object.freeze({ grant: "grant", assign: "roles", policy: "apps" });
// (A grant, and a policy on who READS, are the owner's alone in the replay: "owner".)
const POLICY_ACTIONS = ["read", "post", "comment", "vote", "edit", "join", "invite", "react", "save", "share", "tag"];
const POLICY_WHO = ["anyone", "members", "admins", "owner", "nobody"];
export function outcomeProblem(a) {
  if (a == null) return null;
  if (typeof a !== "object" || !OUTCOMES[a.act]) return "an outcome is a role granted, a role assigned or a policy set";
  if (a.act === "grant") return DID.test(String(a.did)) && ["admin", "member"].includes(a.role) ? null : "a grant names a member and admin or member";
  if (a.act === "assign") return DID.test(String(a.did)) && /^[\w-]{1,40}$/.test(String(a.role)) && typeof a.on === "boolean" ? null : "an assignment names a member, a role and on/off";
  if (typeof a.path !== "string" || a.path.length > 120) return "a policy names a path (an app's domain, or \"\" for the space)";
  if (!POLICY_ACTIONS.includes(a.action)) return `a policy's action is one of ${POLICY_ACTIONS.join(", ")}`;
  if (!POLICY_WHO.includes(a.who) && !/^role:[\w-]{1,40}$/.test(String(a.who))) return "a policy's who is anyone, members, admins, owner, nobody or a role";
  return null;
}
export const outcomeNeed = a => (a?.act === "grant" || (a?.act === "policy" && a.action === "read") ? "owner" : (OUTCOMES[a?.act] ?? null));
export function outcomeText(a, name = d => d) {
  if (!a) return "no outcome (a decision only)";
  if (a.act === "grant") return `make ${name(a.did)} ${a.role === "admin" ? "an admin" : "a member (not admin)"}`;
  if (a.act === "assign") return `${a.on ? "give" : "take back"} the role “${a.role}” ${a.on ? "to" : "from"} ${name(a.did)}`;
  return `set who may ${a.action} ${a.path ? `in “${a.path}”` : "in the space"} to ${a.who}`;
}
// The act that applied a proposal's outcome (an act carries `proposal`: its reference), or null.
export const appliedAct = (acts, proposalRef) => acts.find(a => a?.proposal === proposalRef) ?? null;

// ── WHAT A PERSON SEES ──────────────────────────────────────────────────────────────────────────────────────────────
const WORD = { yes: "YES", no: "NO", abstain: "ABSTAIN" };
// This person's part in a proposal, in words: "You voted NO directly", "Your vote is delegated to Alice; Alice votes
// YES", or why it counts for nothing.
export function participation(me, resolution, name = d => d) {
  const x = resolution.get(me);
  if (!x) return "You are not in this proposal's electorate.";
  const chain = p => p.slice(1).map(name).join(" → ");
  switch (x.how) {
    case "direct":
      return `You voted ${WORD[x.choice]} directly.`;
    case "delegated":
      return `Your vote is delegated to ${chain(x.path)}; ${name(x.path.at(-1))} votes ${WORD[x.choice]}.`;
    case "stuck":
      return `Your vote is delegated to ${chain(x.path)}; ${name(x.path.at(-1))} has not voted (yet) — it counts for nothing until then.`;
    case "outside":
      return `Your vote is delegated to ${chain(x.path)}, who is not in this electorate: it counts for nothing.`;
    case "cycle":
      return `Your delegation runs in a circle (${chain(x.path)}) and nobody on it voted directly: it counts for nothing.`;
    case "depth":
      return `Your delegation chain is longer than ${DEPTH}: it counts for nothing.`;
    default:
      return "You have not voted, and your vote is not delegated.";
  }
}
// THE DELEGATION GRAPH, laid out: each person who voted or delegated a NODE — `col` 0 for a direct voter, n for one
// whose vote reaches a voter in n steps, -1 for one whose vote counts for nothing — `row` their place in the column
// (by DID); an EDGE per delegation (from → to). Deterministic.
export function graphOf(resolution, delegations) {
  const nodes = [];
  for (const [did, x] of resolution) {
    if (x.how === "none") continue;
    nodes.push({ did, how: x.how, choice: x.choice, col: x.how === "direct" ? 0 : x.how === "delegated" ? x.path.length - 1 : -1 });
  }
  const shown = new Set(nodes.map(n => n.did));
  const edges = [...delegations].filter(([from, to]) => to && shown.has(from)).map(([from, to]) => ({ from, to })).sort((a, b) => (a.from < b.from ? -1 : 1));
  for (const e of edges) if (!shown.has(e.to)) (shown.add(e.to), nodes.push({ did: e.to, how: "outside", choice: null, col: -1 }));
  const rows = new Map();
  nodes.sort((a, b) => a.col - b.col || (a.did < b.did ? -1 : 1));
  for (const n of nodes) {
    n.row = rows.get(n.col) ?? 0;
    rows.set(n.col, n.row + 1);
  }
  return { nodes, edges, cols: Math.max(0, ...nodes.map(n => n.col)) + 1 };
}

// THE SPACE's DELEGATIONS alone (no proposal): each chain walked to its END — a member delegated to who delegates to
// nobody (a node in column 0, no choice) — so the graph shows who carries whose vote before anyone votes.
export function delegationChains(electorate, delegations) {
  const targets = new Set([...delegations.values()].filter(Boolean));
  const ends = new Map([...new Set(electorate)].filter(d => !delegations.get(d) && targets.has(d)).map(d => [d, null]));
  return resolve({ electorate, direct: ends, delegations });
}

// ── THE SERVICE ─────────────────────────────────────────────────────────────────────────────────────────────────────
export async function start(ctx) {
  const [items, space, roles] = await Promise.all(["items", "space", "roles"].map(n => ctx.require(n)));
  const me = async () => (await space.account()).id;
  // The space an item is in, this person a member (null: a person's own item, or a space read from outside).
  const spaceOf = async ref => {
    const sid = items.spaceIdOf(ref);
    return sid ? ((await space.mine()).find(s => s.id === sid) ?? null) : null;
  };
  const rolesOf = async sp => {
    const r = await roles.of(sp);
    await r.settled;
    return r;
  };
  const hashed = list => withHashes(list.map(recordOf));

  // ELIGIBLE to vote on `item` (a poll, a proposal): in a space, its member who passes the item's `vote` rule (its own,
  // within the space's: `roles.mayWrite`, the one check); a person's item, who passes its rule (a circle's by the
  // credential they cite). true | false | null (a rule not read yet).
  const eligibility = (item, r) => (by, rec) => {
    if (r && !r.role(by)) return false;
    if (r?.banned?.(by)) return false;
    return roles.mayWrite({ action: "vote", item, writer: by, cred: rec?.cred ?? null, r, at: rec?.at ?? Infinity });
  };

  // A POLL: its settings, its ballots (where they are: `items.attached` — a space's room, or each voter's own feed
  // pointed in its bag), its tally, this person's own newest, whether they may vote now.
  async function pollView(poll) {
    const sp = await spaceOf(poll.ref);
    const r = sp ? await rolesOf(sp) : null;
    if (!sp) await roles.personal(poll.by).ready?.catch?.(() => {});
    const records = await hashed(await items.attached(poll.ref, BALLOT).catch(() => []));
    const self = await me();
    const tally = pollTally(poll, records, { eligible: eligibility(poll, r) });
    const mineRec = newest(records.filter(x => x.by === self && x.on === poll.ref)).get(self) ?? null;
    const meta = normalizePoll(poll.meta);
    const may = meta.open && (await items.mayWriteOn(poll, "vote").catch(() => false));
    return { meta, tally, mine: mineRec?.choice ?? null, may, records };
  }
  // A BALLOT written: a new record in this person's own feed, its seq one above their last on this item.
  async function vote(item, choice) {
    const self = await me();
    if (item.kind === POLL) {
      const p = normalizePoll(item.meta);
      if (!p.open) throw new Error("this poll is closed");
      if (!validPollChoice(p, choice)) throw new Error(p.multiple ? "choose one or more of its options" : "choose one of its options");
    } else if (choice !== null && !CHOICES.includes(choice)) throw new Error("a proposal's vote is yes, no or abstain");
    if (!(await items.mayWriteOn(item, "vote").catch(() => false))) throw new Error("you may not vote on this");
    const mine = (await hashed(await items.attached(item.ref, BALLOT).catch(() => []))).filter(x => x.by === self);
    const sp = await spaceOf(item.ref);
    const cred = sp ? null : await roles.credToCite(item, "vote").catch(() => null);
    return items.attach(item.ref, BALLOT, "", { meta: { seq: nextSeq(mine, self), choice, ...(cred ? { cred } : {}) } });
  }

  // A SPACE's proposals and delegations, read once (every record of the vote kinds in its room), hashed.
  async function inSpace(sp) {
    const r = await rolesOf(sp);
    const all = await items.inPlaces({ spaces: [sp] }, [POLL, PROPOSAL, DELEGATION, BALLOT, TALLY], { withVotes: false });
    const records = await hashed(all);
    const self = await me();
    const members = () => r.members().map(m => m.did);
    // The ELECTORATE of a proposal: the space's members (one account each) its `vote` rule lets vote.
    const electorate = p => members().filter(d => eligibility(p, r)(d) === true);
    // WHO MAY FINALIZE, as it was WHEN the finalization was written (a role changed later never unseats one): its
    // proposer, the owner then (the space's first owner, then each `transfer` act), an admin then (their last `grant`
    // act before it). Now (`at` Infinity): as the roles are now.
    const ownerAt = at => r.acts("transfer").filter(a => (a.at ?? 0) <= at).reduce((o, a) => a.did ?? o, sp.governance?.owner ?? r.owner);
    const adminAt = (did, at) => r.acts("grant").filter(a => a.did === did && (a.at ?? 0) <= at).at(-1)?.role === "admin";
    const mayFinalize = p => (by, at = Infinity) => (at === Infinity ? by === p.by || ["owner", "admin"].includes(r.role(by)) : by === p.by || by === ownerAt(at) || adminAt(by, at));
    async function proposalView(p) {
      const E = electorate(p);
      const t = proposalTally({ proposal: p, records, electorate: E });
      const fin = await pickFinal(p, records, mayFinalize(p));
      const verified = fin ? await verifyFinal(fin.final, p, records) : null;
      const outcome = p.meta?.action ?? null;
      const applied = outcome ? appliedAct(r.acts(outcome.act), p.ref) : null;
      const need = outcomeNeed(outcome);
      return {
        tally: t,
        final: fin,
        verified,
        result: fin ? fin.final.result : t.result,
        electorate: E,
        mine: t.direct.get(self) ?? null,
        myDelegate: t.delegations.get(self) ?? null,
        participation: name => participation(self, t.resolution, name),
        outcome,
        applied,
        mayApply: !!(outcome && need && (need === "owner" ? r.role(self) === "owner" : r.can(self, need))),
        mayFinalize: !fin && mayFinalize(p)(self),
        mayVote: !fin && E.includes(self),
      };
    }
    // This person's delegation now (latest), and a new one written (null: revoked) — a member's, to a member.
    const delegation = () => newest(records.filter(x => x.kind === DELEGATION && x.by === self)).get(self)?.to ?? null;
    async function delegate(to) {
      if (!r.role(self)) throw new Error("only a member delegates here");
      if (to !== null && (!members().includes(to) || to === self)) throw new Error("delegate to another member of this space");
      const seq = nextSeq(records.filter(x => x.kind === DELEGATION), self);
      return items.submit({ board: sp.id, kind: DELEGATION, title: "", body: "", audience: "members", meta: { seq, to } });
    }
    // FINALIZE: the tally now, its inputs and its hash, as a `tally` record on the proposal (the proposer, an admin).
    async function finalize(p) {
      const v = await proposalView(p);
      if (v.final) throw new Error("this proposal is finalized already");
      if (!mayFinalize(p)(self)) throw new Error("only its proposer or an admin finalizes a proposal");
      const snap = await snapshot(p.ref, v.tally, v.electorate);
      return items.attach(p.ref, TALLY, "", { meta: { seq: 1, final: snap } });
    }
    // THE OUTCOME applied: its act, as this person (the space's own rule decides whether it counts), naming the
    // proposal — only once finalized as PASSED, and only by one who may do that act.
    async function applyOutcome(p) {
      const v = await proposalView(p);
      if (!v.outcome) throw new Error("this proposal has no outcome to apply");
      if (!v.final || !v.final.final.result.passed) throw new Error("only a finalized, passed proposal is applied");
      if (v.verified?.status === "forged" || v.verified?.status === "mismatch") throw new Error(`its finalization does not check (${v.verified.status})`);
      if (v.applied) return v.applied;
      if (!v.mayApply) throw new Error("you may not do this act in this space");
      await r.act({ ...v.outcome, proposal: p.ref });
      return null;
    }
    return {
      r,
      sp,
      records,
      members,
      electorate,
      polls: () => all.filter(x => x.kind === POLL).sort((a, b) => b.at - a.at),
      proposals: () => all.filter(x => x.kind === PROPOSAL).sort((a, b) => b.at - a.at),
      proposalView,
      delegation,
      delegations: () => new Map([...newest(records.filter(x => x.kind === DELEGATION && r.role(x.by)))].map(([by, x]) => [by, x.to ?? null])),
      delegate,
      finalize,
      applyOutcome,
      mayPropose: () => r.allowsIn("post", self, "vote"),
      async propose({ title, body, action = null, quorum = 1, audience = "members", write = null }) {
        if (!String(title ?? "").trim()) throw new Error("a proposal needs a title");
        const why = outcomeProblem(action);
        if (why) throw new Error(why);
        return items.submit({ board: sp.id, kind: PROPOSAL, title, body, audience, write, meta: { quorum: Math.max(1, Number(quorum) || 1), ...(action ? { action } : {}) } });
      },
    };
  }
  async function createPoll({ board = null, question, options, multiple = false, results = "live", audience = "public", write = null }) {
    const meta = { ...normalizePoll({ options, multiple, results }), open: true, closedAt: null };
    const why = pollProblem(question, meta);
    if (why) throw new Error(why);
    return items.submit({ board, kind: POLL, title: String(question).trim(), body: "", audience, write, meta });
  }
  // OPEN or CLOSE a poll (its maker): its meta as it was, `open` and `closedAt` changed.
  const setOpen = (poll, open) => items.editItem(poll.ref, poll.body ?? "", { meta: { ...poll.meta, open, closedAt: open ? null : Date.now() } });
  return { pollView, vote, inSpace, createPoll, setOpen, spaceOf, normalizePoll, participation, graphOf, delegationChains, outcomeText, outcomeProblem, DEPTH, CHOICES };
}
