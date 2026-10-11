// VOTE's pure logic (`packages/vote-tally.js`: records, newest-wins, poll tally, delegation resolution, finalization) —
// run with no node and no page:   node --test tools/vote-tally.test.mjs
// `VOTE_TALLY_JS`: another copy of the file to test (the negative controls: a copy with one rule broken must fail).
import test from "node:test";
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";

const src = process.env.VOTE_TALLY_JS ?? new URL("../packages/vote-tally.js", import.meta.url).href;
const V = await import(src.startsWith("file:") ? src : pathToFileURL(src).href);

const D = x => `did:craftec:${x}`;
const SP = `space:${"a".repeat(64)}`;
const PROP = { ref: `${SP}/tprop0000001`, kind: "proposal", by: D("P"), at: 1, meta: { quorum: 1 } };
let n = 0;
// Records as the space's room gives them (`items`' shape): a ballot attached to the proposal (its id in `in`), a
// delegation standing on its own.
const ballot = (by, choice, seq, { on = PROP, at = 100 + n } = {}) => ({ kind: "ballot", ref: `${SP}/tb${String(++n).padStart(10, "0")}`, by: D(by), at, in: on.ref.slice(on.ref.lastIndexOf("/") + 1), meta: { seq, choice } });
const deleg = (by, to, seq, at = 100 + n) => ({ kind: "delegation", ref: `${SP}/td${String(++n).padStart(10, "0")}`, by: D(by), at, meta: { seq, to: to === null ? null : D(to) } });
const recs = items => V.withHashes(items.map(V.recordOf));
const tally = async (items, electorate, proposal = PROP) => V.proposalTally({ proposal, records: await recs(items), electorate: electorate.map(D) });
const how = (t, x) => t.resolution.get(D(x));
const shuffle = (a, seed) => {
  const b = [...a];
  let s = seed;
  for (let i = b.length - 1; i > 0; i--) {
    s = (s * 1103515245 + 12345) % 2147483648;
    const j = s % (i + 1);
    [b[i], b[j]] = [b[j], b[i]];
  }
  return b;
};

test("records: a ballot names its proposal; a malformed record is none; a delegation to oneself is none", () => {
  const b = V.recordOf(ballot("A", "yes", 1));
  assert.equal(b.on, PROP.ref);
  assert.equal(V.recordOf({ kind: "ballot", ref: `${SP}/tx`, by: D("A"), in: "tprop0000001", meta: { choice: "yes" } }), null, "no seq: not a record");
  assert.equal(V.recordOf({ kind: "delegation", ref: `${SP}/tx`, by: D("A"), meta: { seq: 1, to: "bob" } }), null, "not a DID");
  assert.equal(V.recordOf(deleg("A", "A", 1)).to, null);
});

test("single delegation: A → B, B votes YES — A's vote is B's", async () => {
  const t = await tally([deleg("A", "B", 1), ballot("B", "yes", 1)], ["A", "B", "C"]);
  assert.deepEqual(how(t, "A"), { choice: "yes", how: "delegated", path: [D("A"), D("B")] });
  assert.equal(how(t, "C").how, "none");
  assert.deepEqual([t.result.yes, t.result.no, t.result.abstain], [2, 0, 0]);
  assert.equal(t.result.passed, true);
});

test("chain: A → B → C, C votes NO — both carry C's NO, with the path", async () => {
  const t = await tally([deleg("A", "B", 1), deleg("B", "C", 1), ballot("C", "no", 1)], ["A", "B", "C"]);
  assert.deepEqual(how(t, "A").path, [D("A"), D("B"), D("C")]);
  assert.equal(how(t, "A").choice, "no");
  assert.equal(how(t, "B").choice, "no");
  assert.equal(t.result.no, 3);
  assert.equal(t.result.passed, false);
});

test("direct override: A delegated to B but voted YES directly — A counts YES whatever B votes", async () => {
  const t = await tally([deleg("A", "B", 1), ballot("A", "yes", 1), ballot("B", "no", 1)], ["A", "B"]);
  assert.deepEqual(how(t, "A"), { choice: "yes", how: "direct", path: [D("A")] });
  assert.deepEqual([t.result.yes, t.result.no], [1, 1]);
  // Withdrawn (a newer ballot with no choice): back to the delegation.
  const w = await tally([deleg("A", "B", 1), ballot("A", "yes", 1), ballot("A", null, 2), ballot("B", "no", 1)], ["A", "B"]);
  assert.equal(how(w, "A").how, "delegated");
  assert.equal(how(w, "A").choice, "no");
});

test("revocation and replacement: the highest seq counts — null revokes, a later one re-delegates", async () => {
  const base = [ballot("B", "yes", 1), ballot("C", "no", 1)];
  const revoked = await tally([...base, deleg("A", "B", 1), deleg("A", null, 2)], ["A", "B", "C"]);
  assert.equal(how(revoked, "A").how, "none");
  const again = await tally([...base, deleg("A", "B", 1), deleg("A", null, 2), deleg("A", "C", 3)], ["A", "B", "C"]);
  assert.equal(how(again, "A").choice, "no");
  // Written in any order, the seq decides (not the order read, not the time claimed).
  const late = await tally([...base, deleg("A", "C", 3, 1), deleg("A", "B", 1, 999)], ["A", "B", "C"]);
  assert.equal(how(late, "A").choice, "no");
});

test("cycles: A → B → C → A with no direct vote contributes nothing", async () => {
  const t = await tally([deleg("A", "B", 1), deleg("B", "C", 1), deleg("C", "A", 1)], ["A", "B", "C"]);
  for (const x of ["A", "B", "C"]) assert.equal(how(t, x).how, "cycle", x);
  assert.deepEqual([t.result.yes, t.result.no, t.result.abstain], [0, 0, 0]);
  assert.equal(t.result.passed, false);
});

test("cycles broken by a direct vote: B in the circle votes YES — A and C reach B", async () => {
  const t = await tally([deleg("A", "B", 1), deleg("B", "C", 1), deleg("C", "A", 1), ballot("B", "yes", 1)], ["A", "B", "C"]);
  assert.equal(how(t, "B").how, "direct");
  assert.deepEqual(how(t, "A"), { choice: "yes", how: "delegated", path: [D("A"), D("B")] });
  assert.deepEqual(how(t, "C").path, [D("C"), D("A"), D("B")]);
  assert.equal(t.result.yes, 3);
});

test("depth limit: 10 hops to a voter count, 11 do not", async () => {
  const chain = len => [...Array(len)].map((_, i) => deleg(`N${i}`, `N${i + 1}`, 1));
  const who = len => [...Array(len + 1)].map((_, i) => `N${i}`);
  const ten = await tally([...chain(10), ballot("N10", "yes", 1)], who(10));
  assert.equal(how(ten, "N0").how, "delegated");
  assert.equal(how(ten, "N0").path.length, 11);
  const eleven = await tally([...chain(11), ballot("N11", "yes", 1)], who(11));
  assert.equal(how(eleven, "N0").how, "depth");
  assert.equal(how(eleven, "N1").how, "delegated");
  assert.equal(eleven.result.yes, 11);
});

test("only the electorate counts: an outsider's ballot is nothing, a chain leaving the electorate is nothing", async () => {
  const t = await tally([ballot("X", "yes", 1), deleg("A", "X", 1)], ["A", "B"]);
  assert.equal(t.result.yes, 0);
  assert.equal(how(t, "A").how, "outside");
});

test("deterministic under input order: any shuffle gives the same resolution, result and snapshot hash", async () => {
  const items = [
    deleg("A", "B", 1), deleg("A", "C", 2), deleg("B", "D", 1), deleg("E", "F", 1), deleg("F", "E", 1), deleg("G", "D", 1),
    ballot("D", "abstain", 1), ballot("C", "yes", 1), ballot("C", "no", 2), ballot("H", "yes", 1),
    // Two devices of one account at the same seq: the lower hash wins, whichever is read first.
    ballot("H", "no", 2), ballot("H", "yes", 2),
  ];
  const E = ["A", "B", "C", "D", "E", "F", "G", "H"];
  const first = await tally(items, E);
  const snap = await V.snapshot(PROP.ref, first, E.map(D));
  const shape = t => JSON.stringify([...t.resolution].sort());
  for (let seed = 1; seed <= 25; seed++) {
    const t = await tally(shuffle(items, seed), shuffle(E, seed + 7));
    assert.equal(shape(t), shape(first), `seed ${seed}`);
    assert.deepEqual(t.result, first.result);
    assert.equal((await V.snapshot(PROP.ref, t, shuffle(E, seed).map(D))).hash, snap.hash);
  }
});

test("finalized snapshot: later records never change it; a forged or incomplete one is told", async () => {
  const items = [deleg("A", "B", 1), ballot("B", "yes", 1), ballot("C", "no", 1)];
  const E = ["A", "B", "C"].map(D);
  const t = V.proposalTally({ proposal: PROP, records: await recs(items), electorate: E });
  const final = await V.snapshot(PROP.ref, t, E);
  assert.equal(final.result.passed, true);
  // LATER: B changes to NO, A votes NO directly, a newcomer... the live tally flips — the finalized one does not.
  const later = await recs([...items, ballot("B", "no", 2), ballot("A", "no", 1)]);
  const live = V.proposalTally({ proposal: PROP, records: later, electorate: E });
  assert.equal(live.result.passed, false);
  const v = await V.verifyFinal(final, PROP, later);
  assert.equal(v.status, "verified");
  assert.equal(v.result.passed, true);
  assert.equal(v.result.yes, 2);
  // Its result changed by hand: the hash no longer checks.
  assert.equal((await V.verifyFinal({ ...final, result: { ...final.result, passed: false } }, PROP, later)).status, "forged");
  // An input not held here: partial, never a recount over what is held.
  assert.deepEqual(await V.verifyFinal(final, PROP, later.slice(1)), { status: "partial", missing: 1 });
  // The earliest valid finalization stands; one by someone who may not finalize is passed over.
  const tallyItem = (by, at, f) => ({ kind: "tally", ref: `${SP}/tt${String(++n).padStart(10, "0")}`, by: D(by), at, in: "tprop0000001", meta: { seq: 1, final: f } });
  const t2 = await V.snapshot(PROP.ref, live, E);
  const fins = await recs([tallyItem("P", 50, final), tallyItem("Z", 10, t2), tallyItem("P", 60, t2)]);
  const may = by => by === D("P");
  assert.equal((await V.pickFinal(PROP, fins, may)).final.hash, final.hash);
});

const POLL = { ref: `${D("M")}/tpoll000001`, kind: "poll", by: D("M"), at: 1, meta: { options: ["Red", "Green", "Blue"], multiple: false, open: true } };
const pb = (by, choice, seq, at = 100 + n) => ({ kind: "ballot", ref: `${D(by)}/tb${String(++n).padStart(10, "0")}`, by: D(by), at, in: POLL.ref, meta: { seq, choice } });

test("poll: one per account, the newest counts — changed, withdrawn, invalid, ineligible, late", async () => {
  const items = [pb("A", ["Red"], 1), pb("A", ["Blue"], 2), pb("B", ["Green"], 1), pb("B", [], 2), pb("C", ["Red", "Blue"], 1), pb("D", ["Red"], 1), pb("E", ["Green"], 1)];
  const records = await recs(items);
  const t = V.pollTally(POLL, records, { eligible: by => (by === D("D") ? false : by === D("E") ? null : true) });
  assert.deepEqual(t.counts, { Red: 0, Green: 0, Blue: 1 });
  assert.equal(t.voters, 1);
  assert.equal(t.invalid, 1, "two options on a single-choice poll");
  assert.equal(t.ineligible, 1);
  assert.equal(t.pending, 1);
  // Shuffled: the same.
  for (let seed = 1; seed <= 10; seed++) assert.deepEqual(V.pollTally(POLL, shuffle(records, seed), { eligible: by => by !== D("D") && by !== D("E") }).counts, t.counts);
  // CLOSED at 150: a ballot written after it is not counted (the one before stands).
  const closed = { ...POLL, meta: { ...POLL.meta, open: false, closedAt: 150 } };
  const c = V.pollTally(closed, await recs([pb("F", ["Red"], 1, 120), pb("F", ["Green"], 2, 200)]));
  assert.deepEqual(c.counts, { Red: 1, Green: 0, Blue: 0 });
  assert.equal(c.late, 1);
  // MULTIPLE choice: several options, each counted.
  const multi = { ...POLL, meta: { ...POLL.meta, multiple: true } };
  assert.deepEqual(V.pollTally(multi, records).counts, { Red: 2, Green: 1, Blue: 2 });
});

test("participation in words, the graph laid out, outcomes checked", async () => {
  const t = await tally([deleg("A", "B", 1), ballot("B", "yes", 1), ballot("C", "no", 1), deleg("E", "F", 1), deleg("F", "E", 1)], ["A", "B", "C", "D", "E", "F"]);
  const name = d => d.slice(12);
  assert.equal(V.participation(D("A"), t.resolution, name), "Your vote is delegated to B; B votes YES.");
  assert.equal(V.participation(D("C"), t.resolution, name), "You voted NO directly.");
  assert.match(V.participation(D("D"), t.resolution, name), /not voted/);
  assert.match(V.participation(D("E"), t.resolution, name), /circle/);
  assert.match(V.participation(D("X"), t.resolution, name), /not in this proposal's electorate/);
  const g = V.graphOf(t.resolution, t.delegations);
  const node = x => g.nodes.find(nd => nd.did === D(x));
  assert.equal(node("B").col, 0);
  assert.equal(node("A").col, 1);
  assert.equal(node("E").col, -1);
  assert.equal(node("D"), undefined);
  assert.deepEqual(g.edges.map(e => [name(e.from), name(e.to)]), [["A", "B"], ["E", "F"], ["F", "E"]]);
  // The space's delegations alone: chains walked to their end (F ← … none; B ← A).
  const chains = V.delegationChains([..."ABCD"].map(D), new Map([[D("A"), D("B")], [D("C"), D("A")]]));
  assert.deepEqual(chains.get(D("C")), { choice: null, how: "delegated", path: [D("C"), D("A"), D("B")] });
  assert.equal(chains.get(D("B")).how, "direct");
  assert.equal(chains.get(D("D")).how, "none");
  assert.equal(V.outcomeProblem({ act: "grant", did: D("A"), role: "admin" }), null);
  assert.equal(V.outcomeProblem({ act: "policy", path: "text", action: "post", who: "admins" }), null);
  assert.match(V.outcomeProblem({ act: "ban", did: D("A") }), /role granted/);
  assert.match(V.outcomeProblem({ act: "policy", path: "", action: "fly", who: "admins" }), /action is one of/);
  assert.equal(V.outcomeNeed({ act: "assign" }), "roles");
  assert.equal(V.outcomeNeed({ act: "grant" }), "owner");
  assert.equal(V.appliedAct([{ act: "grant", proposal: "x" }, { act: "grant", proposal: PROP.ref }], PROP.ref).proposal, PROP.ref);
});
