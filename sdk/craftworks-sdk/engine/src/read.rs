//! The read path as a pure state machine.
//!
//! Same shape as the write path: `step(event) -> Vec<Effect>`, no sockets, no
//! clock. A cold read of someone else's key is a head and then one block per
//! level, and every one of those is a round-trip a delegate is re-entered for
//! — so the core parks a continuation and resumes it, rather than blocking.
//!
//! **The descent is not reimplemented here.** `freenet_prolly::read::get` and
//! `range::range` already walk the tree and report exactly which blocks they
//! were missing (`ReadError::Need`, `Page::need`). Writing a second walk in
//! the engine would be a second answer to "which child holds this key", and
//! the one way a reader and the library can come to disagree about what a tree
//! contains. So the core's whole job is: try, park on what came back, fetch,
//! try again.

use crate::{ClientId, Params};
use freenet_prolly::node::Value;
use freenet_prolly::range::{range_with, Options as RangeOptions, PageEnd, Range, RangeError};
use freenet_prolly::read::get;
use freenet_prolly::store::{Blocks, ReadError};
use freenet_prolly::{block_id, Cid};
use std::collections::{BTreeMap, BTreeSet};

/// A client's own id for a read, echoed in its reply.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct ReqId(pub u64);

/// How a block is being fetched.
///
/// A pack is tried first for a fresh commit: one fetch can carry every block
/// of a level, and the blocks inside are the same blocks with the same ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Direct,
    Pack(Cid),
}

/// What a read came back with.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReadResult {
    /// The value, or its absence. Absence is an answer.
    Value(Option<Vec<u8>>),
    Page {
        entries: Vec<(Vec<u8>, Vec<u8>)>,
        /// Pass back as `after` to continue. `None` means the scan reached the
        /// end of the range, not that the page was full.
        cursor: Option<Vec<u8>>,
        complete: bool,
    },
    /// What changed in a range between two roots.
    ///
    /// `None` as a value is a REMOVAL, not an empty value: a reader told an
    /// empty value keeps a key the writer deleted.
    Delta {
        changes: Vec<(Vec<u8>, Option<Vec<u8>>)>,
        /// Continue after this key. `None` means the delta is complete.
        cursor: Option<Vec<u8>>,
        /// The root the changes bring the reader TO. Carried so the client can
        /// record where it now stands without a second question, and so two
        /// deltas answered across a moving tree cannot be mistaken for one.
        new_root: Cid,
    },
    /// The delta cannot be computed, and a full range read is the answer.
    ///
    /// **Not an error, and this is a decision rather than a convenience.** The
    /// old root's blocks may simply be gone — superseded, holding no demand,
    /// evicted — and that is an ORDINARY outcome for a reader that was away
    /// long enough, not a failure of anything. An error here would make every
    /// caller invent the same fallback, and some of them would invent it
    /// wrongly or not at all; saying what to do instead makes the degraded
    /// path the one that gets tested.
    FullReloadRequired { new_root: Cid },
    /// A block could not be had within the attempt budget. A REPLY, not a
    /// hang: a read that never answers is indistinguishable from a wedged
    /// node, and the caller can do nothing about either.
    Unavailable(Cid),
    /// This read needs to hold more at once than the warm bound allows.
    ///
    /// Distinct from `Unavailable`, which means the network would not answer.
    /// Here the network answered and the engine cannot keep what it was
    /// given — a caller can act on that (raise the bound, narrow the query),
    /// and could do nothing about the other. It is a reply either way: the
    /// alternative is evicting what the read needs and asking for it again
    /// for ever.
    OutOfWarmSpace,
}

/// A walk a READER makes itself, over the engine's own blocks (READ-STATE,
/// design B): the page's store walks the tree from the owner's root instead of
/// keeping rows. The descent is the engine's (`attempt`), never a second one.
#[derive(Clone, Debug)]
pub enum Walk {
    Get(Vec<u8>),
    Scan(Box<ScanSpec>),
    /// What changed in a range between `from` and the walk's root: a LIVE
    /// binding's diff from the root it rendered at (READ-STATE inv. 5).
    Delta(Box<DeltaSpec>),
}

/// How a walk ended: answered, or stopped on blocks this page does not hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Walked {
    Done(ReadResult),
    /// These blocks are missing; a parked read at the same root fetches them.
    Need(Vec<Cid>),
    /// The tree names a block whose content is not that block.
    Broken(Cid),
}

/// What a client asked for, kept so the request can be retried as blocks land.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum Want {
    Get(Vec<u8>),
    Scan(Box<ScanSpec>),
    /// What changed in a range between a root the reader last saw and the
    /// root now.
    ///
    /// **Per TREE.** A delta is the difference between two roots of ONE tree.
    /// A view assembled over several device trees is NOT the union of their
    /// per-tree deltas — a tombstone in one tree can REVEAL an older value in
    /// another, which no per-tree diff mentions because nothing in that tree
    /// changed. Phase 3 has one tree per identity so the distinction does not
    /// bite yet; it is stated here so that nothing later assumes the union is
    /// valid.
    ///
    /// A READ like the other two, and deliberately so: it descends, it can
    /// find a block missing, and it must then park and resume when the block
    /// lands. Answering a delta from whatever happened to be warm would give
    /// a SHORTER list of changes than the truth — which is a wrong answer
    /// shaped exactly like a right one, and the reader would apply it and
    /// believe it was up to date.
    Delta(Box<DeltaSpec>),
}

/// A delta request, in the engine's own encodable representation.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DeltaSpec {
    /// The root the reader last saw. Where it is standing, not where it wants
    /// to be.
    pub from: Cid,
    pub lo: std::ops::Bound<Vec<u8>>,
    pub hi: std::ops::Bound<Vec<u8>>,
    pub max_entries: usize,
}

/// A scan, in the engine's OWN representation.
///
/// `freenet_prolly::range::Range` is the library's type and is not encodable;
/// the context is a wire format that leaves this process, so what goes into it
/// has to be the engine's, converted at the boundary. Mirroring the fields
/// rather than wrapping keeps the conversion total — a field added to `Range`
/// is a compile error here, not a silently dropped bound.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ScanSpec {
    pub lo: std::ops::Bound<Vec<u8>>,
    pub hi: std::ops::Bound<Vec<u8>>,
    pub reverse: bool,
    pub after: Option<Vec<u8>>,
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl From<&ScanSpec> for Range {
    fn from(s: &ScanSpec) -> Range {
        Range {
            lo: s.lo.clone(),
            hi: s.hi.clone(),
            reverse: s.reverse,
            after: s.after.clone(),
            max_entries: s.max_entries,
            max_bytes: s.max_bytes,
        }
    }
}

impl From<&Range> for ScanSpec {
    fn from(r: &Range) -> ScanSpec {
        ScanSpec {
            lo: r.lo.clone(),
            hi: r.hi.clone(),
            reverse: r.reverse,
            after: r.after.clone(),
            max_entries: r.max_entries,
            max_bytes: r.max_bytes,
        }
    }
}

/// A parked DELTA's own progress (sdk#135, apply-and-resume): the prefix it has established, and where it resumes.
///
/// In MEMORY only (the architect's ruling): the engine Context is test-only, delegate-era code slated for deletion,
/// and a delta rehydrated without this simply restarts from `from` -- correct, only slower.
///
/// **The stall rule** (K = 2, the architect's ruling with engineer4's correction): a delta below the node's
/// retention floor livelocks on ANSWERED re-fetches -- the node evicts what it served before the next attempt can use
/// it. A fetch CYCLE (from the asks made while no earlier ask was outstanding, until every one of them has arrived)
/// is NO PROGRESS when `(after, acc.len())` did not move AND every block it asked had already arrived at this resume
/// position (`seen`). Two in a row answer `FullReloadRequired`, the delta's defined answer (the reload is a scan,
/// which resumes by `frontier` and finishes under eviction). A descent asks NEW blocks, so it never counts; a cycle
/// with an unanswered ask never ends (silence is re-asked, rule 8), so a slow network never trips it.
#[derive(Clone, Debug, Default)]
pub(crate) struct DeltaProgress {
    /// Resume strictly after this key; `None`: from the start.
    pub after: Option<Vec<u8>>,
    /// The established prefix, in key order: final, never re-delivered or rewritten.
    pub acc: Changes,
    /// Blocks that have arrived for this read at the current resume position. Cleared when `after` advances, so it
    /// is bounded by the descent it tracks (a few spines and a page's values), never by the delta's size.
    pub seen: BTreeSet<Cid>,
    /// This cycle asked at least one block not in `seen`: a descent, so progress.
    pub cycle_new: bool,
    /// `(after, acc.len())` when the current cycle began; `None` before the first.
    pub cycle_start: Option<(Option<Vec<u8>>, usize)>,
    /// No-progress cycles in a row.
    pub stalled: u8,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Parked {
    pub client: ClientId,
    pub want: Want,
    pub root: Cid,
    /// The deepest NODE this read has actually reached, if any (sdk#34).
    ///
    /// The next attempt continues from here instead of walking from the root
    /// again. `None` means "start at the root" — which is what a context
    /// written before this field existed decodes to, and what a range still
    /// does.
    ///
    /// **Why one cid is enough for a point read.** A descent is a single path,
    /// so its position IS a point. A SCAN's position is a path, not a point,
    /// and a stack is a different trade — so ranges keep re-descending until
    /// that trade is made deliberately.
    ///
    /// **Why continuing does not weaken the chain.** The id being waited on
    /// came from a parent the engine had already verified, and the arriving
    /// block is checked against that id before it is parsed
    /// ([`matches_id`]). The verification is transitive across calls because
    /// what the context carries is an id that was trusted when it was written
    /// down.
    ///
    /// **Snapshot consistency is untouched.** `root` is still fixed when the
    /// read parks, and the frontier is a node of that same tree, so a head
    /// that moves underneath changes nothing about what this read answers.
    pub frontier: Option<Cid>,
    /// A DELTA's prefix and resume point (sdk#135). Not in the Context (see [`DeltaProgress`]).
    #[serde(skip)]
    pub delta: DeltaProgress,
    pub levels_done: usize,
    /// Fetch rounds this read has made without finishing.
    ///
    /// The bound has to be PER REQUEST, not per block. A per-block attempt
    /// counter counts FAILURES, and when the node serves a block and then
    /// evicts it before the next call — which it may, because a sync read
    /// does not refresh hosting — every attempt SUCCEEDS and the counter
    /// never trips. The read then re-descends for ever, making progress on
    /// paper and none in fact.
    pub rounds: u32,
    /// Every block this read has been handed, PINNED until it replies.
    ///
    /// A read re-descends from the root on each resume, so it needs the whole
    /// path it has already paid for. Evicting any of it sends the read back
    /// for a block it just had — and because that fetch SUCCEEDS, the attempt
    /// budget never trips and nothing ever ends. A tight warm set turned a
    /// cold read into a livelock exactly that way.
    pub held: BTreeSet<Cid>,
    /// GETs this read has caused, across its rounds (sdk#174). To the node
    /// one client request is ONE delegate chain however many rounds it
    /// takes, held under exclusion throughout and truncated, invisibly, past
    /// 100 iterations; a read stops at `max_gets_per_request` and answers what
    /// it has with a cursor, and the next page is a new request.
    #[serde(default)]
    pub gets: u32,
    /// Blocks that have ARRIVED for this read since it started (#330
    /// ruling): its progress, which decides whether a head that moves past
    /// its root may supersede it ([`crate::Engine::supersede_read`]). A read
    /// receiving blocks finishes on its tree; one stuck on a silent or
    /// absent block does not hold the reader on a superseded root.
    #[serde(default)]
    pub arrived: u32,
}

/// The read side of the engine's state.
#[derive(Default)]
pub(crate) struct Reads {
    pub parked: BTreeMap<ReqId, Parked>,
    // Who waits on each block is in `Engine::wanted` (WANTED-LIFE: one writer, by type).
    /// Attempts already made per block, for the bounded re-issue.
    pub attempts: BTreeMap<Cid, u32>,
    /// Which pack is known to carry a block, newest first.
    pub in_pack: BTreeMap<Cid, Cid>,
    pub fetches: usize,
}

/// Does this block's content match the id it was fetched under -- under ANY kind a block can be
/// ([`BlockKind::ALL`](core_types::kind::BlockKind::ALL), the one list)?
///
/// Checked BEFORE it touches the warm tree. A block that fails this is not cached and not parsed: a
/// content-addressed store whose contents are not their addresses is a store where one bad answer poisons every
/// later read of the same key. It listed three kinds by hand and missed PARITY, so a fetched parity block was
/// judged "not its id" and never kept; the list is no longer written here.
pub fn matches_id(id: &Cid, bytes: &[u8]) -> bool {
    kind_of(id, bytes).is_some()
}

/// The one list of block kinds, re-exported for the engine's callers (the page), which read it through the engine.
pub use core_types::kind::BlockKind;

/// The kind under which `bytes` ARE the block `id`, or `None` when they are not it under any kind.
pub fn kind_of(id: &Cid, bytes: &[u8]) -> Option<core_types::kind::BlockKind> {
    core_types::kind::BlockKind::ALL.into_iter().find(|k| block_id(k.byte(), bytes) == *id)
}

/// Try a request against what is warm, and say what it still needs.
pub(crate) enum Attempt {
    Done(ReadResult),
    Need(Vec<Cid>),
    /// Blocks are needed AND the next attempt may continue from `node`.
    ///
    /// Separate from [`Attempt::Need`] because not every missing block is a
    /// place to resume from. A missing VALUE block is named by a leaf that has
    /// already been reached, and it is a raw block, not a node — resuming
    /// "from" it would hand the descent something that does not parse as a
    /// tree node. Only a missing CHILD is a resume point.
    NeedFrom {
        ids: Vec<Cid>,
        node: Cid,
    },
    /// The tree is damaged in a way no fetch can fix.
    Broken(Cid),
}

pub(crate) fn attempt<B: Blocks>(
    blocks: &B,
    params: &Params,
    want: &Want,
    root: &Cid,
    nodes_parsed: &mut usize,
) -> Attempt {
    match want {
        Want::Get(key) => {
            if params.count_descent {
                count_nodes(blocks, root, key, nodes_parsed);
            }
            match get(blocks, root, key) {
                // A value stored by REFERENCE is a block of its own, and the
                // descent that found the leaf does not fetch it. Answering
                // with what is warm would return an EMPTY value for a key
                // that has one — a wrong answer, not a missing one, and
                // indistinguishable downstream from a value that really is
                // empty. So a missing value block is something to fetch, like
                // any other.
                Ok(Some(Value::Ref { cid, .. })) if blocks.get(&cid).is_none() => {
                    Attempt::Need(vec![cid])
                }
                Ok(v) => Attempt::Done(ReadResult::Value(v.map(|v| materialise(blocks, v)))),
                Err(ReadError::Need(ids)) => match ids.first().copied() {
                    // The missing child is where the descent stopped, so it is
                    // where the next one starts.
                    Some(node) => Attempt::NeedFrom { ids, node },
                    None => Attempt::Need(ids),
                },
                Err(ReadError::Corrupt(cid, _)) | Err(ReadError::Mismatch(cid)) => {
                    Attempt::Broken(cid)
                }
            }
        }
        // The walk's delta (and a delta's first attempt) is `delta_step` from the start; a PREFIX it establishes is
        // not an answer to the whole question, so here it is only a need. A parked delta keeps the prefix instead
        // (`Engine::drive`, sdk#135).
        Want::Delta(spec) => match delta_step(blocks, params, spec, root, None, spec.max_entries) {
            DeltaStep::Done { changes, cursor } => Attempt::Done(ReadResult::Delta { changes, cursor, new_root: *root }),
            DeltaStep::Prefix { need, .. } | DeltaStep::Need(need) => Attempt::Need(need),
            DeltaStep::Broken(cid) => Attempt::Broken(cid),
            DeltaStep::Reload => Attempt::Done(ReadResult::FullReloadRequired { new_root: *root }),
        },
        Want::Scan(spec) => {
            let opts = RangeOptions::default();
            let r: Range = spec.as_ref().into();
            match range_with(opts, blocks, root, &r) {
                Ok(page) => {
                    if !page.need.is_empty() && page.end == PageEnd::Blocked {
                        // Only the blocks it actually stopped on, bounded by
                        // the parameter: `need` can name the whole rest of the
                        // range, and asking for all of it is how one scan
                        // turns into an unbounded fan-out.
                        return Attempt::Need(
                            page.need
                                .into_iter()
                                .take(params.max_fetch_per_round)
                                .collect(),
                        );
                    }
                    // Same for a page: an entry whose value block is not warm
                    // would come back empty.
                    let missing: Vec<Cid> = page
                        .entries
                        .iter()
                        .filter_map(|(_, v)| match v {
                            Value::Ref { cid, .. } if blocks.get(cid).is_none() => Some(*cid),
                            _ => None,
                        })
                        .collect();
                    if !missing.is_empty() {
                        return Attempt::Need(
                            missing
                                .into_iter()
                                .take(params.max_fetch_per_round)
                                .collect(),
                        );
                    }
                    let entries = page
                        .entries
                        .into_iter()
                        .map(|(k, v)| (k, materialise(blocks, v)))
                        .collect();
                    Attempt::Done(ReadResult::Page {
                        entries,
                        cursor: page.next,
                        complete: matches!(page.end, PageEnd::EndOfRange | PageEnd::EndOfTree),
                    })
                }
                Err(RangeError::Read(ReadError::Need(ids))) => Attempt::Need(ids),
                Err(RangeError::Read(ReadError::Corrupt(cid, _)))
                | Err(RangeError::Read(ReadError::Mismatch(cid))) => Attempt::Broken(cid),
                // `max_entries == 0` is a caller mistake the SDK screens; here
                // it is an empty, complete page rather than a panic.
                Err(RangeError::NoLimit) => Attempt::Done(ReadResult::Page {
                    entries: Vec::new(),
                    cursor: None,
                    complete: true,
                }),
            }
        }
    }
}

/// A SHORT page for a scan that has used its request's GETs (sdk#174): the
/// entries it has reached, in order, up to the first whose value is not
/// warm, and a cursor after the last -- the next page is a new request.
/// `None` for anything but a scan, or when nothing has been reached yet
/// (there is no key to put a cursor after).
pub(crate) fn short_page<B: Blocks>(blocks: &B, want: &Want, root: &Cid) -> Option<ReadResult> {
    let Want::Scan(spec) = want else {
        return None;
    };
    let r: Range = spec.as_ref().into();
    let page = range_with(RangeOptions::default(), blocks, root, &r).ok()?;
    let entries: Vec<(Vec<u8>, Vec<u8>)> = page
        .entries
        .into_iter()
        .take_while(|(_, v)| !matches!(v, Value::Ref { cid, .. } if blocks.get(cid).is_none()))
        .map(|(k, v)| (k, materialise(blocks, v)))
        .collect();
    let last = entries.last()?.0.clone();
    Some(ReadResult::Page {
        entries,
        cursor: Some(last),
        complete: false,
    })
}

/// A delta's changes as the reply carries them: key, and the new value (`None`: removed).
pub type Changes = Vec<(Vec<u8>, Option<Vec<u8>>)>;

/// One attempt of a DELTA, resumed after `after` (sdk#135, apply-and-resume).
pub(crate) enum DeltaStep {
    /// Finished for the range, or its page limit (`cursor`).
    Done { changes: Changes, cursor: Option<Vec<u8>> },
    /// A correct PREFIX was established (freenet_prolly::diff's page contract, prolly#51) and the diff stopped on
    /// blocks it does not hold: apply `changes`, resume after `after`, fetch `need`.
    Prefix { changes: Changes, after: Option<Vec<u8>>, need: Vec<Cid> },
    /// Nothing established: fetch these (a changed value not warm, or a block before any decided key).
    Need(Vec<Cid>),
    Broken(Cid),
    /// Not answerable by fetching (another pair of roots, an instruction a diff has no answer for): the full read.
    Reload,
}

/// The diff from `spec.from` to `root`, resumed strictly after `after`, at most `room` changes.
///
/// A page is a correct PREFIX of the full diff (freenet_prolly::diff, "Paging, and what a page means"): every change
/// up to where it stopped is final, and `next` never passes the first key it could not establish. So a page that
/// stopped on missing blocks is KEPT, not discarded -- which is what lets a delta finish on a node that evicts what
/// it serves (sdk#135: discarding needed the diff's whole working set co-resident, and never got it). A prefix is
/// taken only once every changed value in it is warm: materialising a value that is not yields an EMPTY value for a
/// key that has one.
pub(crate) fn delta_step<B: Blocks>(
    blocks: &B,
    params: &Params,
    spec: &DeltaSpec,
    root: &Cid,
    after: Option<&[u8]>,
    room: usize,
) -> DeltaStep {
    use freenet_prolly::diff::{Change, DiffError, Resume};
    let r = Range {
        lo: spec.lo.clone(),
        hi: spec.hi.clone(),
        reverse: false,
        after: None,
        max_entries: room,
        max_bytes: usize::MAX,
    };
    // Both roots travel with the resume key, so a resume against another pair is refused, never mixed.
    let resume = after.map(|a| Resume { a: spec.from, b: *root, after: a.to_vec() });
    match freenet_prolly::diff::diff(blocks, &spec.from, root, &r, resume.as_ref()) {
        Ok(page) => {
            let missing: Vec<Cid> = page
                .changes
                .iter()
                .filter_map(|c| match c {
                    Change::Added { new, .. } | Change::Changed { new, .. } => match new {
                        Value::Ref { cid, .. } if blocks.get(cid).is_none() => Some(*cid),
                        _ => None,
                    },
                    Change::Removed { .. } => None,
                })
                .collect();
            if !missing.is_empty() {
                return DeltaStep::Need(missing.into_iter().take(params.max_fetch_per_round).collect());
            }
            let next = page.next.as_ref().map(|n| n.after.clone());
            let changes: Changes = page
                .changes
                .into_iter()
                .map(|c| match c {
                    Change::Added { key, new } | Change::Changed { key, new, .. } => (key, Some(materialise(blocks, new))),
                    // A removal carries no value. `None` IS the change -- a reader that dropped the key is correct,
                    // and one told an empty value is not.
                    Change::Removed { key, .. } => (key, None),
                })
                .collect();
            if page.need.is_empty() {
                return DeltaStep::Done { changes, cursor: next };
            }
            let need: Vec<Cid> = page.need.into_iter().take(params.max_fetch_per_round).collect();
            if changes.is_empty() && next.is_none() {
                DeltaStep::Need(need)
            } else {
                DeltaStep::Prefix { changes, after: next, need }
            }
        }
        Err(DiffError::Read(ReadError::Need(ids))) => DeltaStep::Need(ids),
        Err(DiffError::Read(ReadError::Corrupt(cid, _))) | Err(DiffError::Read(ReadError::Mismatch(cid))) => {
            DeltaStep::Broken(cid)
        }
        // A resume token from another pair of roots, or an instruction a diff has no answer for. Neither is fixed by
        // fetching, and an EMPTY delta would tell the reader it is up to date when nobody checked -- so it is sent
        // to the full read, which is always correct.
        Err(_) => DeltaStep::Reload,
    }
}

/// The bytes of a value, wherever the format put it.
/// Only called once the value's block is known to be warm: the callers above
/// turn a missing one into a fetch, because defaulting it to empty would
/// answer a question wrongly rather than not answering it.
fn materialise<B: Blocks>(blocks: &B, v: Value<'_>) -> Vec<u8> {
    match v {
        Value::Inline(b) => b.to_vec(),
        Value::Ref { cid, .. } => blocks
            .get(&cid)
            .map(<[u8]>::to_vec)
            .expect("the caller fetched it first"),
    }
}

/// How many nodes a descent for `key` would touch, added to the cost counter.
///
/// A MEASUREMENT, not a second answer: it decides nothing and nothing reads
/// its result but the gate. The descent that answers the read is
/// `freenet_prolly::read::get`, and duplicating that decision here is exactly
/// what this module exists not to do — so this walks the same path only to
/// count it, and if it ever disagreed with the real descent the cost number
/// would be wrong while every answer stayed right.
fn count_nodes<B: Blocks>(blocks: &B, root: &Cid, key: &[u8], out: &mut usize) {
    let mut cur = *root;
    while let Some(bytes) = blocks.get(&cur) {
        let Ok(node) = freenet_prolly::node::Node::parse(bytes) else {
            return;
        };
        *out += 1;
        if node.is_leaf() {
            return;
        }
        let i = match node.search(key) {
            Ok(i) => i,
            Err(0) => return,
            Err(i) => i - 1,
        };
        cur = node.child(i).0;
    }
}

#[cfg(test)]
mod matches_id_tests {
    use super::*;
    use core_types::kind::BlockKind;

    /// **EVERY kind matches its own id** -- a PARITY block included (a race get, a repair and the audit all GET
    /// parity; the hand-written list missed it) -- and the kind it is found under is its own. CONTROL: no kind lets
    /// bytes pass under ANOTHER block's id. Mutant "a kind missing from BlockKind::ALL" -> red.
    #[test]
    fn every_kind_matches_its_own_id_and_no_other() {
        let bytes = vec![7u8; 64];
        for k in BlockKind::ALL {
            let id = block_id(k.byte(), &bytes);
            assert!(matches_id(&id, &bytes), "a {k:?} block did not match its own id");
            assert_eq!(kind_of(&id, &bytes), Some(k), "a {k:?} block was found under another kind");
            let other = block_id(k.byte(), &[9u8; 64]);
            assert!(!matches_id(&other, &bytes), "bytes matched another {k:?} block's id");
        }
    }
}
