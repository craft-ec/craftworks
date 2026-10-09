//! The write pipeline as a pure state machine.
//!
//! `step(event) -> Vec<Effect>`: no sockets, no clock, no randomness. Every
//! interleaving a live node can produce — responses out of order, a PUT that
//! never acknowledges, a duplicate confirmation, a restart mid-commit — is an
//! ordinary deterministic test here instead of a flake against a real network.
//!
//! **Effects carry dependencies, never a batch size.** How many operations fit
//! in one delegate round is a platform fact that has already changed once
//! (F15, F21) and is the shell's business. The core says what must happen
//! before what; the shell decides how many at a time.
//!
//! The states a write moves through, and what each one promises:
//!
//! | state | promise |
//! |---|---|
//! | `Accepted` | applied to the tree, not yet shipped. Survives a tab close, NOT a node restart. |
//! | `Stalled` | the commit carrying it cannot publish, and is not moving. Non-terminal, reported once. |
//! | `Published` | packs read back, THEN the head read back. Survives a restart, and what a UI may call "saved". |
//! | `ParityComplete` | the redundancy the new nodes promise actually exists. |
//! | `Busy` | TERMINAL, and nothing was applied: a commit was already in flight, or a cap was reached. The client still holds the write and may re-submit it. |
//! | `Failed` | TERMINAL. The edit is NOT in the tree and never will be, so a re-submit applies it once. Never reported for a write whose edit IS in the tree. |
//! | `Lost` | TERMINAL. The commit carrying it will not publish and the engine no longer has it; the client is the only thing that does. |
//!
//! Between `Published` and `ParityComplete` the tree is correct and its groups
//! have no redundancy. That is *absent redundancy, not an error*
//! (ARCHITECTURE §7): a keeper's repair and the writer's late put are the same
//! bytes under the same id, so nothing has to be flagged, only done.
//!
//! # THE RULE FOR ADDING ANYTHING HERE
//!
//! **The engine is LONG-LIVED in the page (READ-STATE, design B).** The page holds one `Engine` for its life
//! and drives it call by call, so a field keeps its value across calls, in memory. A race's and a repair's state
//! live there (sdk#303), as the reads' own bookkeeping does.
//!
//! It was not always so. In the delegate era every `process()` call got a fresh linear memory (F32): the engine
//! was rebuilt from a CONTEXT the last call wrote, thrown away at the end of the call, and only what that
//! context carried survived. Nothing re-hydrates any more, and the carry was deleted (#305); the context
//! BUDGET it left behind (`keep_saveable`, `context_len`) is sdk#385's question.
//!
//! In that era a field held its default at the top of every call but the one that set it, and four defects of
//! exactly that shape were invisible to every test that drove one process:
//!
//! * `in_flight_since` (sdk#81) — the stall timer. `None` at the top of every
//!   call but the one that started the commit, so `Stalled` could never be
//!   reported at all;
//! * `told_stalled` (sdk#81) — which writes had been told. Without it the
//!   notice repeats on every tick, which is noise a caller learns to ignore;
//! * `in_flight_parity` (sdk#83) — the map from a parity block to its group.
//!   The ack arrives in a LATER call, so no group ever settled and the same
//!   three blocks went out on every tick for ever;
//! * the delegate scheduler's `confirmed` set (sdk#83) — not engine state,
//!   but the same rule one layer out. It learns what the node holds from
//!   acks in THIS call, so a parity put gated `after: [published_root]` was
//!   held on a dependency nothing would confirm, and dropped. Every call.
//!
//! The question that survives them, for any new field, effect dependency or deadline: **what does this hold
//! after the page RELOADS, and is that the right answer?** A page's memory is gone then; what must outlive it
//! lives on the network (the head, the blocks), and is derived from there -- two copies of one fact can
//! disagree, and a derived one cannot.
//!
//! And an EMISSION is not a fact (sdk#150). An effect the engine returns can
//! be held and dropped by the scheduler, cut by a per-return limit, or never
//! answered; "I asked, therefore it happened" made each of those a permanent
//! loss. What is owed is derived from answers; `asks` only paces re-asking.
//!
//! And the test has to span the RELOAD. An assertion that passes on an engine driven straight through proves
//! nothing about what a fresh page recovers.

use freenet_prolly::apply::{apply_with, ApplyError, Edit as TreeEdit, Options as ApplyOptions};
use freenet_prolly::chunk::empty_leaf;
use freenet_prolly::node::Node;
use freenet_prolly::store::{Blocks, ReadError};
use freenet_prolly::Cid;
use std::collections::{BTreeMap, BTreeSet};

pub mod asks;
pub mod pack;
pub mod read;
pub mod race_get;
pub mod repair;
pub mod subs;
mod wanted;
use wanted::{Reader, Wanted};
mod commit;
use commit::{CommitLife, End};

/// Which client a write came from. Two tabs are two clients.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct ClientId(pub u64);

/// The client's own id for a write. Echoed in every state change, so a caller
/// never has to guess which of its writes a notification is about.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct WriteId(pub u64);

/// What a client asked for. Mirrors the tree's own edit vocabulary; the engine
/// never looks inside a value.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Op {
    Put(Vec<u8>),
    Delete,
}

/// What a write READ, stated with it: the key as the writer saw it
/// (sdk#148's `Commit{reads, writes}`). The engine checks every expectation
/// against the tree it is about to apply ON, inside that same apply (R0), and
/// a write whose read no longer holds applies NOTHING and ends `Conflict`.
/// That is the only place the check can live: between an outside check and
/// the apply another commit can land, and only the engine knows which root it
/// applies on.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Expect {
    /// The key is not in the tree (`create_at`: nothing is there yet).
    Absent,
    /// The key is in the tree, whatever its value (a child needs its parent
    /// to EXIST, not to be unchanged).
    Present,
    /// The key holds exactly this value: [`leaf_hash`] of the bytes read
    /// (`update` over the record it patched, a delete of what was read).
    Value([u8; 32]),
    /// "I write this key whatever it holds": the FORCED form, named so it can
    /// be counted (sdk#235, W8). TRANSITIONAL — only a store-level batch that
    /// genuinely cannot read first builds one, an app's write never does, and
    /// sdk#281 removes it once the count is zero. It holds against any tree,
    /// so it reads nothing.
    Any,
}

/// The first key a write changes without having read it, if any (sdk#235,
/// W8). Pure and syntactic: the write's own op keys against its own read
/// keys. A plain `Write` has no reads, so any op it carries is unread.
fn unread_key(ops: &[(Vec<u8>, Op)], reads: &[(Vec<u8>, Expect)]) -> Option<Vec<u8>> {
    ops.iter()
        .map(|(k, _)| k)
        .find(|k| !reads.iter().any(|(r, _)| r == *k))
        .cloned()
}

/// A value as the tree STORES it: inline bytes, or a reference to the block
/// that holds it. What a `Conflict` reports as the key's current value, so the
/// app can merge without reading a copy that may lag the tree.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LeafForm {
    Inline(Vec<u8>),
    Ref { cid: Cid, len: u32 },
}

impl LeafForm {
    fn of(v: freenet_prolly::node::Value<'_>) -> LeafForm {
        match v {
            freenet_prolly::node::Value::Inline(b) => LeafForm::Inline(b.to_vec()),
            freenet_prolly::node::Value::Ref { cid, len } => LeafForm::Ref { cid, len },
        }
    }

    /// The hash an [`Expect::Value`] compares.
    pub fn hash(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        match self {
            LeafForm::Inline(b) => {
                h.update(&[0]);
                h.update(b);
            }
            LeafForm::Ref { cid, len } => {
                h.update(&[1]);
                h.update(cid);
                h.update(&len.to_le_bytes());
            }
        }
        *h.finalize().as_bytes()
    }
}

/// The [`Expect::Value`] for a value the writer READ as `bytes`.
///
/// It hashes the value's STORED FORM (`Value::for_bytes`, the format's one
/// decision of inline vs reference), never the bytes themselves: a value
/// stored by reference is then compared by its reference, and the engine never
/// fetches a large value just to compare it (the architect's attack on the
/// design). Same stored form ⇔ same value.
pub fn leaf_hash(bytes: &[u8]) -> [u8; 32] {
    LeafForm::of(freenet_prolly::node::Value::for_bytes(bytes).0).hash()
}

/// How far along a write is.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum State {
    Accepted,
    /// Still held, not yet saved, and the engine cannot publish it right now.
    ///
    /// NON-TERMINAL, and reported once. The write is still tracked, its edit
    /// is still in the tree, and it reaches `Published` normally when the
    /// commit blocking it confirms. It tells a caller that "saving..." has
    /// stopped being routine -- not that anything has been given up.
    ///
    /// It exists because reporting `Failed` here was a FALSE statement: the
    /// edit stayed in the tree and published with the next commit, so a
    /// client that re-submitted would apply it twice, and could overwrite a
    /// newer write someone else made in between.
    Stalled,
    Published,
    ParityComplete,
    /// TERMINAL: this edit is not in the tree and never will be, for the cause
    /// named (sdk#500: one list, [`FailWhy`]).
    Failed {
        why: FailWhy,
    },
    /// TERMINAL: this engine has never heard of that write.
    ///
    /// After a restart, what the head does not name was never published and
    /// is gone. A client holding the id in its outbox is TOLD so, and
    /// re-submits. Silence would leave it waiting on a write nobody will ever
    /// finish -- the one outcome a client cannot recover from.
    Lost,
    /// TERMINAL: refused without being accepted, for now -- a commit is in
    /// flight, or a cold write was declined while its blocks are fetched.
    /// Re-sending later can succeed.
    Busy,
    /// TERMINAL: refused without being accepted, and NEVER acceptable as it
    /// is: it is over `limit` of `bound` by `got` (craftworks-sdk#136). Unlike
    /// `Busy`, re-sending it is refused the same way every time.
    TooLarge {
        bound: WriteBound,
        limit: usize,
        got: usize,
    },
    /// TERMINAL, nothing applied: a key the write READ no longer holds what
    /// it read ([`Expect`]). The key and its current value ride beside it in
    /// [`Effect::Conflicted`]. NEVER re-sent as it is — the same write
    /// conflicts the same way; what to do next is the app's (sdk#148).
    Conflict,
    /// TERMINAL, nothing applied: the write changes `key` and did not read it
    /// (sdk#235, W8). Judged AT THE DOOR on the write's own bytes — op keys ⊆
    /// read keys — before any other answer and reading no state, so it can
    /// never follow `Accepted`. NEVER re-sent: the same write is refused the
    /// same way every time. Distinct from `Conflict` on purpose: "what you
    /// read moved" and "you never read it" are different facts. WHICH key
    /// rides beside it in [`Effect::Unread`], as `Conflict`'s does.
    Unread,
    /// TERMINAL: its commit died by the engine's lights and whether it
    /// LANDED cannot be known -- the new head's ledger no longer lists this
    /// page (its entry evicted, the list at its bound) or cannot be read
    /// (COMMIT-LIFE ⁵). Never `Lost` (it may have landed) and never re-applied
    /// (it may be there, and a re-judge would call its own value a conflict):
    /// the app is told to CHECK.
    Unknown,
    /// Never admitted, NOW: the page's write queue holds `bytes` of its
    /// `limit` and this session already has a write in it (R-b; COMMIT-LIFE
    /// K1). BACKPRESSURE, not a verdict on the write: the SDK waits for room
    /// and makes it again, and tells the app only at its own deadline. Never
    /// `Busy`. Nothing was applied.
    QueueFull { bytes: usize, limit: usize },
}

/// Which bound a [`State::TooLarge`] write is over.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum WriteBound {
    /// Distinct blocks one commit would write ([`Params::max_commit_blocks`]).
    CommitBlocks,
    /// Bytes in one write ([`Params::max_write_bytes`]).
    WriteBytes,
}

// `Event` is not `Eq`: a scan carries a `Range`, whose bounds are
// `Bound<Vec<u8>>` and which the library does not make `Eq`. Tests compare
// what an event PRODUCES, which is what matters.
#[derive(Clone, Debug)]
pub enum Event {
    /// The page cannot confirm a changed group's other member at once (sdk#416): it is asking the node
    /// (`Held`), and the engine releases one more of that group's parity meanwhile. See [`Effect::ConfirmHeld`].
    HeldUnknown(Cid),
    /// The node's Block contract REFUSED this block as invalid (sdk#433): a final answer -- the same
    /// content-addressed bytes are refused again, for ever (an encoding defect, or a node on another contract
    /// epoch). Never re-put. A commit that needs it and has not sent its head ends its writes `Failed` (never
    /// `Lost`: re-sent, they would re-derive the same block); a published commit's straggler stays owed (never
    /// BACKED_UP) and is recorded ([`Engine::rejected_blocks`]). [`Event::PutFailed`] keeps its meaning: put again.
    PutRejected(Cid),
    /// The SIGNER finally refused the head of commit `seq` (PUBLISH-LIFE ⁵, E8; COMMIT-LIFE's Heading x head refused).
    /// A real answer (rule 8), so it ENDS the commit: every write of its cut `Failed`, the tree back to the published
    /// root, the writes behind it re-derived there. Any other seq is a stale answer and does nothing (the head it named
    /// is already dead). The engine carries no reason: it sits below signer-proto, and the page -- which holds the
    /// signer's `Why` -- names it on these writes' `Failed` (one owner of the reason).
    HeadRefused { seq: u64 },
    Write {
        client: ClientId,
        write_id: WriteId,
        ops: Vec<(Vec<u8>, Op)>,
        /// What it read: checked against the tree the ops land on, in the same
        /// apply. Every op key must be here (W8, sdk#235) — a write with an op
        /// key outside its reads is refused at the door as `Unread`; a forced
        /// write says so per key with [`Expect::Any`] ([`Event::forced_write`]).
        reads: Vec<(Vec<u8>, Expect)>,
        /// Commit only IN COMPANY (sdk#350): held, `Accepted`, until the queue
        /// holds a non-deferred write that is applied; then it rides that
        /// write's cut. Never committed alone ([`Event::deferred`]).
        deferred: bool,
    },
    /// A block was READ BACK from our own node. Never an ack: W1 says an
    /// acknowledgement is not evidence the block is there.
    PutConfirmed(Cid),
    PutFailed(Cid),
    HeadConfirmed(u64),
    Tick(u64),
    /// The client has gone. Ship what is waiting; there will be no more ticks.
    ///
    /// The shell sends this on disconnect. `Tick` comes only from a connected
    /// client (W4: the node fires no wake-ups), so nothing the page-closed
    /// promise depends on may need one — a commit in flight advances on its
    /// confirmations, and everything else that a tick would have got round to
    /// happens here instead.
    Flush,

    // ---- the read path ----
    Get {
        client: ClientId,
        req_id: read::ReqId,
        key: Vec<u8>,
    },
    Scan {
        client: ClientId,
        req_id: read::ReqId,
        range: Box<freenet_prolly::range::Range>,
    },
    /// A scan AT A GIVEN ROOT (READ-STATE open item 3): the root a reader's
    /// walk stopped at, so the blocks it fetches are that tree's. The page's
    /// store sends it when its own walk found blocks missing; the reply is
    /// the read's end (its TICKET), and the store walks again at that root.
    ScanAt {
        client: ClientId,
        req_id: read::ReqId,
        root: Cid,
        range: Box<freenet_prolly::range::Range>,
    },
    /// What changed in this range since the root the reader last saw?
    ///
    /// **No subscription is involved, and that is the point.** A delta is a
    /// capability of the TREE — equal ids mean equal subtrees, so the
    /// difference between two roots costs the changes and not the size — and
    /// it is available to any reader holding a root, live or not. An ordinary
    /// reload uses it: the client remembers the root it last read at and asks
    /// only for what moved.
    ///
    /// Being TOLD that something changed is the separate, declared thing. It
    /// triggers this; it is not required for it.
    ChangesSince {
        client: ClientId,
        req_id: read::ReqId,
        /// The root the reader is standing on.
        from: Cid,
        range: subs::SubRange,
        max_entries: usize,
    },
    /// Advisory: bring these roots' blocks nearer, within the budget. It may
    /// make later reads SOONER; it may never make them more or wider.
    Preload {
        client: ClientId,
        roots: Vec<Cid>,
    },
    /// A fetched block came back. Hash-checked before it is believed.
    BlockArrived {
        id: Cid,
        bytes: Vec<u8>,
    },
    /// A bounded attempt ended without an answer. Not a failure of the read:
    /// the next attempt is issued, until the budget runs out.
    BlockMissed(Cid),

    // ---- recovery ----
    /// A new engine value, holding only its device key and the epoch table.
    ///
    /// Where the authority came from is STATED here, never inferred: an
    /// engine that invented its own would be a second writer for the same
    /// device (sdk#14).
    Start {
        key: KeySource,
        /// Code epochs to try, newest first. An engine's own head may still
        /// sit under the previous epoch after an upgrade.
        epochs: Vec<Epoch>,
    },
    HeadRead {
        epoch: Epoch,
        seq: u64,
        root: Cid,
    },
    /// No head under any epoch: a brand-new device, empty tree, seq 0.
    HeadMissing,
    /// Whether this page can SIGN a head now, stepped by the page on each
    /// CHANGE of its one signer decision. While `false` no commit is CUT:
    /// writes queue and apply to the warm root (reads see them), and the
    /// moment it turns `true` ONE commit carries the queue, in arrival order.
    /// Before any is stepped the engine can sign (every existing path).
    CanSign(bool),
    /// Another engine holds this device key and is ahead. The loser re-reads
    /// and rebases; it never publishes a fork.
    HeadConflict {
        seq: u64,
        root: Cid,
    },
    /// A client asking after a write it still holds in its outbox.
    AskWrite {
        client: ClientId,
        write_id: WriteId,
    },
    /// Block `id`'s bytes LANDED LOCALLY: the page already holds them (a rebuild it kept, its own write), so it serves a
    /// fetch from memory, not from the node (WANTED-LIFE E2r). It serves the reads, repairs and the parked write like
    /// `BlockArrived` -- and NO audit: an audit waits for the node's answer (W7).
    BlockLanded {
        id: Cid,
        bytes: Vec<u8>,
    },
    /// The assets audit's pass `pass` wants block `id` from the NODE (sdk#530, WANTED-LIFE E1): a reader of the one
    /// (block, GET) key. Asked even when `id` is held (W7): the audit measures the node, not the page.
    AuditWant {
        id: Cid,
        pass: PassId,
    },
    /// Pass `pass` no longer waits on `id` (replaced, or ended: sdk#532) -- only ITS reader drops (WANTED-LIFE E4).
    AuditDrop {
        id: Cid,
        pass: PassId,
    },

    // ---- subscriptions ----
    /// Tell me when anything between these keys changes.
    SubscribeRange {
        client: ClientId,
        sub_id: u64,
        range: subs::SubRange,
    },
    Unsubscribe {
        client: ClientId,
        sub_id: u64,
    },
    /// This client is gone. Everything it was watching goes with it.
    ///
    /// Separate from `Flush`, which is about shipping work: a tab that closed
    /// still has writes worth publishing, and none worth notifying.
    ClientGone {
        client: ClientId,
    },
}

impl Event {
    /// A FORCED write: every op key read as [`Expect::Any`] — "I write this
    /// key whatever it holds" (sdk#235, W8). The ONE way a write that does not
    /// depend on what was there is built, so it is named and counted, never
    /// an implicit reads-less write (which the engine refuses as `Unread`).
    pub fn forced_write(client: ClientId, write_id: WriteId, ops: Vec<(Vec<u8>, Op)>) -> Event {
        let reads = ops.iter().map(|(k, _)| (k.clone(), Expect::Any)).collect();
        Event::Write { client, write_id, ops, reads, deferred: false }
    }

    /// The same write, DEFERRED: it commits only in company (sdk#350). Any
    /// other event is returned as it is.
    pub fn deferred(self) -> Event {
        match self {
            Event::Write { client, write_id, ops, reads, .. } => Event::Write { client, write_id, ops, reads, deferred: true },
            other => other,
        }
    }
}

/// Which code epoch a head was written under.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct Epoch(pub u32);

/// Where this engine's authority came from.
///
/// Stated at `Start` and never inferred. The engine does not mint authority:
/// it is handed some, or it has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum KeySource {
    SecretStore,
    Reissued,
    /// Handed in from outside and kept in the secret store, unmodified.
    ///
    /// The delegate derives NOTHING: it is given a key and it uses that key.
    /// Where a real device key comes from is sdk#14's, and this is the
    /// parameter that lets it slot in without the shell changing — the shell
    /// only ever knows that it was handed one.
    ///
    /// Today the only provisioner is a test harness, which generates a fresh
    /// key per run and removes it afterwards. `Test` says so IN THE TYPE, so
    /// a key that is not for real use cannot be mistaken for one that is by
    /// anything reading this.
    Provisioned(Provisioned),
}

/// Who provisioned a key, and whether it is real.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Provisioned {
    /// Generated for one run by a driver, and removed at the end of it.
    /// Never read from disk, never a real key.
    Test,
}

/// Why a write's reads did not all hold.
enum ReadsCheck {
    /// Their paths are not held yet: park, fetch, check again.
    Need(Vec<Cid>),
    /// This key does not hold what the write read.
    Differs { key: Vec<u8>, current: Option<LeafForm> },
    /// The tree cannot be read (a corrupt block): nothing applied.
    Unreadable,
}

/// A group's parity ids, [`PARITY`] of them. The unit redundancy comes in:
/// they are one group's protection and are worth nothing separately.
pub type ParityIds = [Cid; PARITY];

/// Parity blocks per group: the TREE's number (freenet-prolly `parity::PARITY`), its
/// one owner. Never written by hand here -- `tests/one_parity.rs` fails the build
/// on a hand-written count.
pub use freenet_prolly::parity::PARITY;
/// Why a write ended `Failed` (sdk#500): the ONE list, from layer 0.
pub use core_types::fail::FailWhy;

/// An apply that stopped for anything but a missing block: the tree's own
/// damage, or the edit itself refused.
fn apply_failed(e: &ApplyError) -> FailWhy {
    match e {
        ApplyError::Read(_) => FailWhy::TreeDamaged,
        ApplyError::NotSorted | ApplyError::KeyTooLong | ApplyError::ValueTooLong | ApplyError::Node(_) => FailWhy::EditRefused,
    }
}


#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// A block of a QUEUED write's warm apply (R-b; COMMIT-LIFE § A write's
    /// stage): kept by the page so a read of the warm root finds it, and
    /// NEVER put — its commit, rebuilt on the published root, puts the same
    /// bytes (content-addressed) when it is that write's turn.
    Keep {
        id: Cid,
        bytes: Vec<u8>,
    },
    /// A block REBUILT from its sibling group (sdk#303, the owner's rule 11: repair from parity), PUT back so the
    /// network is whole again. The same PUT as a commit's, through the page's one sender -- but no commit's: its
    /// answer confirms nothing a commit or a head waits on. Emitted only for a block a reader wanted and whose
    /// bytes are its id ([`Engine::rebuilt`]).
    PutRepaired {
        id: Cid,
        bytes: Vec<u8>,
    },
    PutPack {
        id: Cid,
        bytes: Vec<u8>,
        after: Vec<Cid>,
    },
    /// A member of a group the published commit changed that is not the commit's own block nor its own straggler
    /// (safety gap class 2, the architect's ruling): BACKED_UP ("all k+m of every group the commit changed",
    /// rule 10) waits for the node to hold it. A block in page memory is not one on the node -- a READ may have
    /// REBUILT it from its group, and another page's straggler may still be in flight. The engine keeps no copy
    /// of what the node confirmed: the page (its `confirmed`, the one holder) answers `PutConfirmed(id)` at once
    /// when it knows, or asks the node until it does (rule 7).
    ConfirmHeld {
        id: Cid,
    },
    /// A value too large to ride in a pack.
    PutBlock {
        id: Cid,
        bytes: Vec<u8>,
        after: Vec<Cid>,
    },
    /// Stop putting this block: a later root move SUPERSEDED it (COMMIT-LIFE
    /// §P). Withdrawal of a PUT nobody needs, never a cut-off of one that is.
    Withdraw {
        id: Cid,
    },
    /// Stop FETCHING this block: it was asked, and nobody wants it any more (WANTED-LIFE; sdk#303's withdrawal). The
    /// page ends its GET (`drop_get`) and does not keep it if it arrives late. Computed NET at the end of the call that
    /// dropped its last reader -- never at the drop, so a block dropped and wanted again in one call is not told
    /// unwanted (the architect's T3). Not [`Effect::Withdraw`]: that is a superseded PUT, a different fact.
    Unwanted {
        id: Cid,
    },
    /// The NODE answered block `id`'s GET for the audit passes `passes` (sdk#530, WANTED-LIFE A2/A3), with its
    /// [`AuditVerdict`]. Only the node's answer: a rebuild from parity, or bytes the page already holds, never answer
    /// an audit (W7). The audit's readers are taken with it.
    AuditAnswered {
        id: Cid,
        passes: Vec<PassId>,
        verdict: AuditVerdict,
    },
    UpdateHead {
        seq: u64,
        root: Cid,
        /// The root this commit was BUILT ON: its head's prev is
        /// `(seq - 1, base)` and nothing else. A head signed with any other
        /// prev claims to contain a tree it does not (a foreign winner
        /// adopted after the commit was built: two devices, same seq).
        base: Cid,
        after: Vec<Cid>,
    },
    Notify {
        client: ClientId,
        write_id: WriteId,
        state: State,
    },
    /// Beside a `Notify{Conflict}`: WHICH read no longer holds, and what the
    /// key holds now (`None`: absent) — the value the engine just read, so the
    /// app can merge without re-reading a copy that may lag.
    Conflicted {
        client: ClientId,
        write_id: WriteId,
        key: Vec<u8>,
        current: Option<LeafForm>,
        /// A CASCADE NAMES ITS CAUSE (R-b; COMMIT-LIFE footnote 3): the
        /// earlier write, of the same re-derivation, that wrote `key` and
        /// itself conflicted or fell. `None`: this write's own read moved.
        after: Option<(ClientId, WriteId)>,
        /// Tries this write had spent (ONE budget, sdk#265): a `Db` re-run of
        /// it draws on from here, never from a count of its own.
        tries: u32,
    },

    /// Beside a `Notify{Unread}`: the key the write changes and did not read
    /// (sdk#235). The first such key in the write's own op order.
    Unread {
        client: ClientId,
        write_id: WriteId,
        key: Vec<u8>,
    },

    // ---- the read path ----
    FetchBlock {
        id: Cid,
        via: read::Via,
        /// Which try this is. The shell may race or widen on a later attempt;
        /// the core only says the earlier one did not answer.
        attempt: u32,
    },
    Reply {
        client: ClientId,
        req_id: read::ReqId,
        result: read::ReadResult,
    },
    Progress {
        client: ClientId,
        req_id: read::ReqId,
        levels_done: usize,
        levels_total: usize,
    },

    // ---- recovery ----
    /// Read this device's own head, under this epoch.
    ReadHead { epoch: Epoch },

    // ---- subscriptions ----
    /// A subscribe was taken, or refused and WHY.
    ///
    /// Answered either way. A client that believes it is subscribed and is
    /// not waits for ever for a notification nobody is going to send, and
    /// there is nothing in what it can see that would tell it so.
    Subscribed {
        client: ClientId,
        sub_id: u64,
        accepted: subs::Accepted,
    },
    /// Something in a subscribed range changed.
    ///
    /// **The new root ONLY.** No key list, no previous root. The CLIENT owns
    /// "the last root I saw" and passes it to `ChangesSince`, and that is what
    /// makes a dropped notification self-healing: the next one, or the
    /// client's own backstop, diffs across the whole gap in a single call. An
    /// engine that carried the previous root would be asserting where the
    /// client stood, and it does not know — the client may have missed three.
    ///
    /// Delivery is lossy by construction (the node's notification channel
    /// drops when full, and a subscription can be evicted without telling
    /// anyone), so "the client missed some" is the ordinary case rather than
    /// the exceptional one, and the design has to be right for it.
    Changed {
        client: ClientId,
        sub_id: u64,
        new_root: Cid,
        seq: u64,
        why: subs::Why,
    },
}

/// A block as the commit makes it: its id and bytes.
type Block = (Cid, Vec<u8>);

/// Everything tunable, in one place, so nothing downstream reads a literal.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// Ceiling on a pack body. A parameter because the engine's default is
    /// policy and has already moved once; the FORMAT's limit is elsewhere.
    pub max_pack: usize,
    /// A value at or under this rides inside a pack; above it, its own PUT.
    pub max_packed_value: usize,
    /// Whether a commit ships a PACK as well as its blocks.
    ///
    /// FALSE in Phase 3, and the reason is arithmetic rather than taste: a
    /// pack is transient, so the network holds pack + members either way and
    /// the pack only moves who pays. With no keepers, that is the writer
    /// both times.
    ///
    /// Kept as a parameter, not deleted, because Phase 4 turns it back on
    /// with the other half in place — keepers putting the members, and reads
    /// resolving through the head's packs.
    pub pack_on_write: bool,
    /// The largest ONE write, in bytes (keys plus values).
    ///
    /// Named for what it bounds. It was `max_backlog`, but a write is refused
    /// `Busy` whenever a commit is pending or a write is parked, so by the time
    /// this is checked there is no backlog at all: it bounds a single write
    /// (craftworks-sdk#136), and a write over it is `TooLarge`, not `Busy`.
    pub max_write_bytes: usize,
    /// Ticks an ask goes unanswered before it is asked again (sdk#150).
    ///
    /// An ask is not a fact: the effect may have been stranded, cut by a
    /// return's limit or never answered, and nothing reports that. So an
    /// unanswered ask is re-made after this long. Every ask is idempotent,
    /// so a slow answer costs one more put, never a wrong state.
    pub reask_after: u64,
    /// The most asks outstanding at once. Unit: one ask, at most
    /// `asks::ASK_BYTES` of context. Default: one return's worth (`max_puts`
    /// 128 + `max_gets` 4). A full table defers a NEW ask -- what it asks for
    /// stays owed and goes when an answer frees a place -- and never drops
    /// one already made. Checked at construction against
    /// `asks::CONTEXT_SHARE` of `max_context_bytes`.
    pub max_asks: usize,
    /// What of `max_context_bytes` the SHELL's own state may take beside the
    /// engine's: its read-backs (up to one commit's blocks), the head, the
    /// tracing flags, and the wrapper. Checked by the shell's tests.
    pub shell_context_reserve: usize,
    /// The least the parked reads may be left, once every fixed cap is
    /// counted. Construction refuses params that leave less.
    pub min_parked_read_bytes: usize,

    // ---- the read path ----
    /// How many blocks one attempt may ask for. `Page::need` can name the
    /// whole remainder of a range, and asking for all of it is how one scan
    /// becomes an unbounded fan-out. This is a bound on the CORE's appetite,
    /// not a batch size: the shell still decides how many it issues at once.
    ///
    /// And never MORE than one return carries (the shell's `max_gets`,
    /// refused at the shell's construction otherwise): what a return cannot
    /// carry is dropped at the end of the call and nothing asks for it again,
    /// so a read that needed more than that in one round never came back
    /// (sdk#150; live on two private nodes: 0 of 60 rows, 0 of 300).
    pub max_fetch_per_round: usize,
    /// The most GETs ONE client request may cause (sdk#174): a scan answers
    /// a short page and a cursor at it, a parked write waits for the client's
    /// AskWrite to continue. The node runs a request as one delegate chain
    /// under exclusion and truncates it, invisibly, past 100 iterations
    /// (~400 GETs); this keeps a chain an order of magnitude under that and
    /// releases the delegate between pages.
    pub max_gets_per_request: u32,
    /// A DELTA's no-progress fetch cycles before it answers `FullReloadRequired` (sdk#135, the architect's K = 2):
    /// cycles of answered RE-fetches only, below the node's retention floor. See `read::DeltaProgress`.
    pub delta_stall_cycles: u8,
    /// A block the node answered NotFound is ALSO rebuilt from its sibling
    /// group (`repair`: any k of the group's k+m, verified by hash) while the
    /// page keeps asking for it (Phase 4, gap 1): an alternative source,
    /// never a reason to stop. Off only as the control.
    pub repair_reads: bool,
    /// RACE PUT (COMMIT-LIFE §P): the head is signed once the root is in and
    /// every changed group has ANY k of its k+m blocks on the network; the
    /// rest go on to BACKED_UP. Off only as the control: wait for every PUT.
    pub race_put: bool,
    /// THE FIRST WAVE (#378 P1-hybrid, rule 10): a commit's first send is its data and ONE parity per changed group
    /// (so one stall per group is still raced: k of k + 1); the other m - 1 follow the Sign, sent when the head lands. Every parity PUT is
    /// still sent and retried until acked; BACKED_UP is unchanged. Off only as the control: all parity at once.
    /// Applies only with `race_put` (waiting for every PUT cannot hold any back).
    pub first_wave_parity: bool,
    /// RACE GET (the owner's rule 11, sdk#303): the FIRST time a read wants a block of a sibling group, every
    /// other block of that group -- members and parity -- is asked at once, and the read finishes on the member
    /// itself or on any `k` of the group (rebuilt, verified), whichever comes first. Point reads too. Off only as
    /// the control (today's one-at-a-time, which harness#13 measures against). Needs `repair_reads`.
    pub race_get: bool,
    /// Two requests needing one block share its fetch. Off = the control.
    pub share_fetches: bool,
    /// Ceilings on an advisory preload: roots, blocks, bytes. A preload may
    /// make reads SOONER, never more or wider, so a hostile manifest costs
    /// the budget and not what it asked for.
    pub preload_roots: usize,
    pub preload_blocks: usize,
    pub preload_bytes: usize,
    /// Ask again for blocks the node already holds.
    ///
    /// Kept, but it is NOT a control for the cost bound and must not be sold
    /// as one: fetches come from `Need`, and `Need` only ever names blocks
    /// that are missing, so there is nothing already-held to re-ask for. It
    /// measured 3 fetches against a bound of 4 — inert.
    pub refetch_held: bool,
    /// Remember which blocks are already requested, across calls.
    ///
    /// Carried in the context, and it does suppress a duplicate fetch when two
    /// READS want the same block. It is NOT what stops a waiting read
    /// re-asking on every entry — nothing re-descends a parked read except
    /// its own block arriving — so it is not a control for the cost bound,
    /// and measuring it as one gives the same number either way.
    pub dedupe_in_flight: bool,
    /// Re-drive every parked read on every entry.
    ///
    /// The plausible wrong implementation, and the honest control for "a cold
    /// lookup costs the same however often the engine is entered": it is the
    /// SIMPLER thing to write — resume everything and let each read work out
    /// whether it can progress — and it makes the cost grow with how busy the
    /// node is rather than with the depth of the tree.
    pub redescend_on_entry: bool,
    /// On a miss, also ask for every child of every branch already readable.
    ///
    /// A plausible "fetch ahead" a reader might write, and the control that
    /// keeps the depth bound honest: it asks for a level where the descent
    /// needs one block.
    pub fetch_ahead: bool,
    /// The context budget. The platform caps a delegate's context at exactly
    /// 400 KiB (`DelegateContext::MAX_SIZE` = 4096*10*10), so this sits below
    /// it with headroom: exceeding the platform's cap is a refusal the engine
    /// never sees coming, and exceeding its own is a `Busy` it can report.
    pub max_context_bytes: usize,
    /// Count the nodes a descent would touch. It is a second walk, for the
    /// cost gate and nothing else, so it is off unless a test asks.
    pub count_descent: bool,
    /// Ticks a write may sit merely ACCEPTED — in memory, in no commit —
    /// before the engine stops promising anything about it.
    ///
    /// `accepted` survives a refresh and not a restart, so it is exposure,
    /// and exposure has to be bounded rather than journalled: only the head
    /// makes anything findable, and a write not yet in a commit is not on its
    /// way to a head. One commit is in flight at a time, so a write arriving
    /// behind a commit that cannot publish would otherwise wait for ever.
    /// At T it is reported `Failed`, which is a reply the SDK can re-submit
    /// from its outbox — unlike silence.
    pub max_accept_age: u64,
    /// Off = the control: writes fold for ever behind a stuck commit.
    pub bound_accept_age: bool,
    /// Whether the context BUDGET counts the commit in flight
    /// (`context_len`). Always true in production. It was the control for the
    /// Live-vs-Rehydrate comparison, which went with the carry (#305); what is
    /// left is sdk#385's.
    pub context_carries_pending: bool,
    /// Rounds a write may spend waiting for a cold tree path before it is
    /// refused. Each round is one fetch-and-retry of the whole apply.
    pub max_apply_rounds: u32,
    /// The page's write queue (R-b, K1): BYTES of writes taken and not yet
    /// published. No count cap. Past it a session that already has a write
    /// queued is told `QueueFull` and its SDK waits for room.
    pub max_queue_bytes: usize,
    /// The page's block store budget, in bytes (sdk#411; the architect: a Param, default 64 MiB). Past it, at the
    /// end of a page step, the least recently used UNPINNED blocks are evicted ([`Engine::pinned`]). Pinned blocks
    /// are outside it by construction. It bounds the PEAK: wasm memory never shrinks.
    pub max_page_block_bytes: usize,
    /// Times a write whose commit died (a foreign move) goes again at the
    /// front before it falls `Lost` (COMMIT-LIFE footnote 5; the client's
    /// `WRITE_TRIES`, moved into the engine with go-back-N).
    pub max_write_tries: u32,
    /// How many reads may be waiting on the network at once.
    ///
    /// Each costs the context about 130 B, measured. Unbounded, ten thousand
    /// of them make a 1.3 MB context -- three times what the platform will
    /// store -- and in the delegate era saving it failed, which lost the
    /// IN-FLIGHT COMMIT as well. A read burst must not be able to destroy a write, so reads
    /// get a slice of the budget and are refused at its edge.
    pub max_parked_reads: usize,
    /// Blocks one commit may name.
    ///
    /// TWO bounds meet here and they are one number.
    ///
    /// The context: a commit's bookkeeping carries a Cid per block it waits
    /// on, ~54 B each (measured), and `max_write_bytes` bounds a write in BYTES
    /// which says nothing about how many blocks those bytes become.
    ///
    /// The PLATFORM: a pack's bytes exist only in the return that made them
    /// (`Commit::packs` is `#[serde(skip)]`, deliberately — a pack is the
    /// largest thing the engine touches and the context is 400 KiB), so the
    /// shell cannot hold a put back for the next entry. Whatever a commit
    /// emits must go out in ONE `process()` return.
    ///
    /// 128 is the largest k MEASURED to work: k = 1..128 PUTs from one
    /// return, each acknowledged, zero errors, on a private network-mode
    /// node. The limit was NOT reached — 128 is where the search stopped,
    /// not where the node refused. F21's "8" was a probe's `--max-k` default
    /// that had been read as a platform constant; this is a measurement, and
    /// it stays a measurement rather than becoming the next inherited number.
    pub max_commit_blocks: usize,
    /// Emit the head as soon as the commit is planned, without waiting for
    /// its packs to be read back. The control for (c): a head that names a
    /// root whose blocks are not all there is a tree no reader can walk, and
    /// a crash in that window leaves one behind for ever.
    pub head_before_packs: bool,

    // ---- subscriptions ----
    /// Standing range subscriptions this engine will hold at once.
    ///
    /// They live in the 400 KiB context beside everything else, so this is a
    /// slice of the same budget the reads and the in-flight commit come out
    /// of. Over it a subscribe is REFUSED and the client told — a subscriber
    /// that thinks it is watching and is not waits for ever, and nothing it
    /// can see would tell it otherwise.
    pub max_subscriptions: usize,
    /// Standing subscriptions ONE client — one page load — may hold (sdk#146),
    /// so one page cannot take every other's room. Half the engine-wide cap:
    /// two live tabs fit side by side without either evicting the other.
    pub max_subscriptions_per_client: usize,
    /// The longest bound a subscription may carry.
    ///
    /// The bounds are client-chosen byte strings. Without this one
    /// subscription is an allocation whose size the sender picks — which is
    /// the thing the wire's `MAX_MESSAGE` exists to prevent, one layer in.
    pub max_sub_key: usize,
    /// Consecutive `BlockMissing` answers after which a subscribed range is
    /// declared `Stale` and stops being compared.
    ///
    /// A missing block on the OLD root is usually durable — superseded,
    /// unpinned, evicted — so a range in that state would otherwise notify on
    /// every commit for ever with a `why` the client can do nothing about. It
    /// is told once and then left quiet; a reload re-subscribes and resets it.
    /// A declared degradation beats an undeclared storm.
    pub stale_after_missing: u32,
    /// Changes one delta page may carry.
    ///
    /// A delta is a READ and is bounded like one. Unbounded, "what changed
    /// since the root I saw a week ago" is the whole tree, held in memory at
    /// once, in a delegate with 400 KiB of context and one 5-second call.
    pub max_delta_entries: usize,
    /// Answer every subscriber on every root move without comparing the
    /// trees.
    ///
    /// OFF. Two jobs. It is the CONTROL for the range filter — with it on, a
    /// commit that touched nothing a subscriber watches still notifies it, so
    /// a test asserting "no notification for an untouched range" has
    /// something that can make it fail. And it is the shape a DOWNGRADE takes:
    /// an engine under pressure that stops comparing still tells its
    /// subscribers, honestly, that it did not look.
    pub notify_without_diff: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            max_pack: 1024 * 1024,
            max_packed_value: 64 * 1024,
            pack_on_write: false,
            max_write_bytes: 8 * 1024 * 1024,
            reask_after: 16,
            max_asks: 132,
            shell_context_reserve: 12 * 1024,
            min_parked_read_bytes: 32 * 1024,
            max_fetch_per_round: 4,
            max_gets_per_request: 64,
            delta_stall_cycles: 2,
            repair_reads: true,
            race_put: true,
            first_wave_parity: true,
            race_get: true,
            share_fetches: true,
            preload_roots: 4,
            preload_blocks: 256,
            preload_bytes: 4 * 1024 * 1024,
            refetch_held: false,
            dedupe_in_flight: true,
            redescend_on_entry: false,
            fetch_ahead: false,
            max_context_bytes: 320 * 1024,
            count_descent: false,
            max_accept_age: 64,
            bound_accept_age: true,
            context_carries_pending: true,
            max_apply_rounds: 256,
            // 32 MiB: page memory, beside the warm blocks it makes.
            max_queue_bytes: 32 * 1024 * 1024,
            max_page_block_bytes: 64 * 1024 * 1024,
            max_write_tries: 3,
            max_parked_reads: 1000,
            max_commit_blocks: 128,
            head_before_packs: false,
            stale_after_missing: 3,
            max_delta_entries: 256,
            max_subscriptions: 32,
            max_subscriptions_per_client: 16,
            max_sub_key: 256,
            notify_without_diff: false,
        }
    }
}


/// Who wants a block, as [`Engine::readers_of`] derives it: a parked read waits on it, a repair needs it as a slot,
/// the parked write needs it, or an assets-audit pass waits on its GET (sdk#530). Nothing wants it when none does:
/// the ONE holder set of the (block, GET) key (WANTED-LIFE W1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Readers {
    pub reads: bool,
    pub repairs: bool,
    pub parked_write: bool,
    pub audits: bool,
}

impl Readers {
    /// Somebody wants it.
    pub fn any(self) -> bool {
        self.reads || self.repairs || self.parked_write || self.audits
    }
}

/// What the NODE said of a block an audit asked (sdk#530; KEEPER's C2): its bytes (the audit's Fetched), or its
/// NotFound (the audit's Absent). Never a bool: the audit matches on it exhaustively.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditVerdict {
    Present,
    Absent,
}

/// An assets-audit PASS, by its number (sdk#530, sdk#532): a replaced pass drops only ITS readers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub struct PassId(pub u64);

/// REPAIR's own pass (sdk#479): while [`Engine::set_repair_pass`] is on, every race keeps each parity slot's GET out
/// until the NODE answers, under this audit reader -- a real node answers a block it lacks with silence as often as
/// NotFound, and a race that withdrew its parity GETs once the member arrived never learned a parity block was gone.
pub const REPAIR_PASS: PassId = PassId(u64::MAX);


/// What an engine shed to keep its context saveable (sdk#162), per call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shed {
    /// Parity groups NOT CODED because `owed` was at its cap: left to the
    /// scrub (sdk#181, #119), never forgotten from what was already owed.
    pub uncoded: usize,
    /// Writes that will no longer hear `ParityComplete`.
    pub waits: usize,
    /// Parked reads answered `Unavailable` to make room.
    pub reads: usize,
    /// Parked writes answered `Busy` to make room.
    pub writes: usize,
}

/// What a foreign head's ledger says about THIS page's commit (COMMIT-LIFE ⁵).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Witness {
    /// Its `through` for this page: the last arrival number of this page's
    /// writes it carries.
    Through(u64),
    /// This page is not in it, and the list was never at its bound (nothing
    /// ever evicted): this page's commit is not in this head's chain.
    NotThere,
    /// This page is not in it and the list is at its bound (an entry may have
    /// been evicted), or the ledger could not be read: it cannot be known.
    Unknown,
}

/// A commit's stage (COMMIT-LIFE rev 5): `Idle`, `Racing` (its PUTs out, the head not yet signed) or `Heading` (its
/// `UpdateHead` sent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitStage {
    Idle,
    Racing,
    Heading,
}

/// A queued write's stage (R-b; COMMIT-LIFE § A write's stage in the page's
/// queue): the non-terminal half of a fate. Terminal fates are the verdicts
/// the engine emits (`Effect::Notify`), kept by the page's Server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Taken; its path's blocks are being fetched, or an earlier write is
    /// still applying. In no root yet.
    Applying,
    /// In the warm root; not in a commit yet.
    Queued,
    /// The commit in flight carries it.
    Committing,
}

/// A write the engine has TAKEN and not yet committed (R-b; COMMIT-LIFE §
/// A write's stage in the page's queue).
/// A block being rebuilt from its group (`repair`): what is held of the
/// group, and how many times each block still missing has been asked for.
#[derive(Clone, Debug)]
struct Repair {
    group: repair::Group,
    have: BTreeMap<usize, Vec<u8>>,
    asked: BTreeMap<usize, u32>,
    /// The missing block itself was answered NotFound: a REPAIR, whose slots are asked until answered with their
    /// bytes. Until then it is a RACE (sdk#303) beside a read still asking the block, and a slot answered NotFound
    /// or with wrong bytes is DROPPED, not re-asked -- a healthy read must not spend GETs re-asking parity.
    missed: bool,
    /// Slots a race dropped: asked again the moment it becomes a repair.
    dropped: BTreeSet<usize>,
    /// Slots ANSWERED NotFound (or with bytes that are not the block), the missing block's own included once it is:
    /// what a reader counts as NOT present (sdk#524). A slot whose bytes land leaves it. Gone with the repair.
    absent: BTreeSet<usize>,
}

/// A read waiting on a block of a DAMAGED group ([`Engine::damaged`]): of its group's `k + m` slots, `j < k` are
/// not answered NotFound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Damaged {
    pub block: Cid,
    pub j: usize,
    pub k: usize,
}

/// Why a block's page copy may not be dropped (sdk#411's table): each names the code that needs the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pin {
    /// W: a queued write's warm-apply block (`Effect::Keep`), in no commit yet -- the only copy.
    Warm,
    /// C: the pending commit's block, not yet confirmed: on the Wrapper path its `Held` re-put (after HELD_ABSENTS
    /// absents, [`Engine::reput_bytes`]) reads it from page memory -- and a block whose PUT is not yet acked when the
    /// commit publishes becomes a straggler (B) whose re-put needs the SAME bytes. No `Held` ask is out for it yet,
    /// so HF does not hold it.
    InFlight,
    /// B: a published commit's block not yet acked (`backing.remaining`): its re-put reads page memory.
    Backing,
    /// P: a parked read holds it (`Parked::held`) or waits on it (`reads.waiting`).
    ParkedRead,
    /// PW: the parked write's fetched path (`ParkedWrite::held`) or what it waits on.
    ParkedWrite,
    /// HF: a FOREIGN member of a group this page's commit changed, whose bytes this page holds and whose `Held` ask
    /// (#402's ConfirmHeld) is still out: after `HELD_ABSENTS` the page re-puts it FROM THE STORE -- its self-heal
    /// of another page's straggler (the architect, sdk#411). Released at `Held { present }` or when the ask ends.
    HeldForeign,
}

/// The page's fact for [`Engine::pins`]: its outstanding `Held` asks (HF). W, the warm-apply blocks, is the
/// engine's own record since sdk#450 (one owner of "the bytes the queue pins").
///
/// One class the design proposed is NOT a pin (sdk#411, measured): R, a repaired block whose PUT is out -- its
/// re-send carries its own bytes (`Op::Put`, `put_again`), no reader takes them from the store, so its "unpinned"
/// mutant could never go red. H is kept only as HF (a foreign member with bytes and an ask out): for this page's
/// own block C or B already pins it, and a foreign one with no bytes is simply asked again.
pub struct PagePins<'a> {
    /// Ids with a `Held` ask out (a deadline or a backed-off re-ask): HF, for those this page holds bytes of.
    pub held_asks: &'a BTreeSet<Cid>,
}

/// One of a merge's writes for [`Engine::merge_front`]: its id, ops and reads.
pub type MergeWrite = (WriteId, Vec<(Vec<u8>, Op)>, Vec<(Vec<u8>, Expect)>);

#[derive(Clone, Debug)]
struct Queued {
    client: ClientId,
    write_id: WriteId,
    ops: Vec<(Vec<u8>, Op)>,
    reads: Vec<(Vec<u8>, Expect)>,
    size: usize,
    /// Times its commit died (a foreign move) and it went again at the front.
    tries: u32,
    /// `Accepted` has been said (at arrival before recovery, or when it first
    /// joined the warm root): a re-application after a foreign move is not a
    /// new acceptance.
    told_accepted: bool,
    /// `None`: APPLYING — not in the warm root yet (its path is being fetched,
    /// or an earlier write is still applying: arrival order is apply order).
    /// `Some(r)`: QUEUED — the warm root just after it was applied.
    warm_after: Option<Cid>,
    /// Already told `Stalled` (once per stay; reset when it joins a cut). It leaves with the write (COMMIT-LIFE C4,
    /// A5): no engine-level set to forget to clear.
    stalled_told: bool,
    /// Its place in arrival order, numbered by this engine (K9): what a head's
    /// ledger records as this page's `through`, the witness that a group
    /// landed.
    arrival: u64,
    /// Commits only in company (sdk#350): see [`Event::Write`].
    deferred: bool,
}

impl Queued {
    /// UNSAVED: taken and not yet published, except a DEFERRED write no cut
    /// has taken (sdk#350) -- THE one statement of it.
    /// `committing`: the commit in flight carries it ([`Engine::carries`]).
    fn is_unsaved(&self, committing: bool) -> bool {
        !(self.deferred && !committing)
    }
}

/// A write whose apply stopped on a block the node does not hold.
///
/// The tree is a persistent structure over content-addressed blocks, so an
/// apply reads the path it is about to rewrite. A delegate reads through a
/// SYNC host call that returns what the node happens to hold right now (F14),
/// and that call does not register demand (F33) -- so a path can simply be
/// cold, with nothing wrong and nothing to blame.
///
/// Reporting `Failed` there was a false statement about a transient miss: it
/// tells a client its edit will never be in the tree, when fetching one block
/// would have applied it. So the write is PARKED, the blocks are fetched,
/// and the apply runs again from the start -- it is a pure function of
/// (root, ops), so re-running it costs reads and cannot produce a different
/// tree. Its ops are in the page's queue (R-b): this is only the FETCH state
/// of the first `Applying` write, so no context carries client bytes and
/// there is no size a write can be too big to park at.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct ParkedWrite {
    client: ClientId,
    write_id: WriteId,
    /// Rounds spent waiting. Bounded, because the node may simply not have
    /// the block: F14 reads what is held and registers no demand, so "fetch
    /// and try again" can be true for ever.
    rounds: u32,
    // Its NEEDS are held with every other reader in `Engine::wanted` (WANTED-LIFE: one writer, by type).
    /// GETs this write's CURRENT chain has caused (sdk#174). At
    /// `max_gets_per_request` it asks for nothing more until its client
    /// asks after it (`AskWrite`), which starts a new chain.
    #[serde(default)]
    gets: u32,
    /// Ticks RUN since this write last heard anything -- a block it asked
    /// for, or its client asking after it. Counted in ticks that ran, never
    /// as `now - stamp`: the engine's clock moves only when a tick runs, and
    /// none runs while the delegate is parked on a chain, so a stamp is stale
    /// by exactly the park and the first tick after it measured the PARK as
    /// the write's silence (the architect's sdk#196 review, executed: a 64-GET
    /// chain at ~0.75 s a GET is ~48 s -- the ordinary cold write). At
    /// `3 * reask_after` it is released `Failed` (R-b): nothing of it is
    /// applied, and the next `Applying` write is tried.
    #[serde(default)]
    idle_ticks: u64,
    /// The engine time of the last tick counted, so two tabs ticking the same
    /// second count once.
    #[serde(default)]
    idle_at: u64,
    /// Every block that has LANDED for this write while parked, PINNED until it leaves the park (sdk#411): its
    /// re-apply walks the path it already fetched, and a block evicted from under it would be fetched again,
    /// successfully, for ever -- the livelock `read::Parked::held` guards against, for a write.
    ///
    /// BOUNDED by construction, with no count cap of its own: it grows only by blocks fetched for this CHAIN,
    /// which `max_gets_per_request` caps (the same bound as a read's `held`); a new chain (`on_ask`) clears it.
    /// GONE ON EVERY EXIT, because it lives in the `ParkedWrite` and every exit drops that: applied
    /// (`warm_apply`), ended in the queue (`end_queued`: Conflict, Failed, TooLarge, ...), its rounds spent
    /// (`park_write`), idle past its bound (`Failed`), or no longer the first `Applying` write (a queue re-cut).
    #[serde(default)]
    held: BTreeSet<Cid>,
}

/// A commit in flight: one apply, one head bump.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Commit {
    seq: u64,
    root: Cid,
    /// Pack and block ids this commit must land before its head may move.
    data: BTreeSet<Cid>,
    /// The root's parity ids (sdk#335), for its head's ledger. In the page's
    /// memory, not the context (nothing rehydrates in production): a
    /// rehydrated commit signs an EMPTY list, and its root is then fetched
    /// singly, as before.
    #[serde(skip)]
    root_parity: Vec<Cid>,
    /// The pack bodies, for THIS call only.
    ///
    /// A pack is never carried in the context — it is the largest thing the
    /// engine touches and the context is 400 KiB. So a pack that fails within
    /// the call that made it is re-sent from here; one that fails after the
    /// engine has been re-hydrated cannot be, and the commit's writes are
    /// reported `Failed` instead. That is TRUE now in a way it was not
    /// before: the core holds no tree, so nothing of that write is anywhere.
    #[serde(skip)]
    packs: BTreeMap<Cid, Vec<u8>>,
    confirmed: BTreeSet<Cid>,
    /// The writes folded into it, in arrival order.
    writes: Vec<(ClientId, WriteId)>,
    /// Bytes of the accepted-but-not-durable writes, for the backlog bound.
    bytes: usize,
    /// The root the commit's ops were applied to: the published root when it
    /// started (the head's `prev`).
    base: Cid,
    /// The last arrival number in this commit's group (K9): the `through`
    /// its head's ledger records for this page.
    #[serde(default)]
    through: u64,
    /// What must be on the network before the head is signed (§P).
    #[serde(default)]
    race: Race,
    /// THE PARITY HELD BACK for after the Sign (#378 P1-hybrid): every changed group's parity but the ONE in the
    /// first wave. Sent when the head lands (`follow_ups`), behind the Sign and the head's UPDATE on the node's one
    /// queue (F61). Held WITH their bytes, so nothing is re-derived and nothing can fail to be sent. Page memory,
    /// like `packs`: skipped by the state walk (`context_len`, `state_digest`), where m - 1 blocks per group would
    /// be counted as bookkeeping; their ids are in `data`, which the walk covers, and each id is its bytes' hash.
    #[serde(skip)]
    deferred: Vec<Block>,
    /// The changed groups' other members the NODE confirmed (sdk#416): counted toward k.
    #[serde(default)]
    held: BTreeSet<Cid>,
    /// Other members the page could not confirm at once (`Event::HeldUnknown`), each having released one more of
    /// its group's parity into the first wave.
    #[serde(default)]
    unknown: BTreeSet<Cid>,
    /// Each pack's member ids: a pack's ack is its members' ack (the node
    /// holds a pack's members under their own ids), so they count toward k.
    #[serde(default)]
    pack_members: BTreeMap<Cid, Vec<Cid>>,
}

/// The ROOT's parity blocks for a commit that emitted `root` (sdk#335): its
/// group of one, coded by [`repair::root_parity`]. None when the commit did not
/// emit the root. The one derivation, for a commit's first send and for a
/// re-put from its carried ops alike.
fn root_parity_blocks(root: Cid, emitted: &[(Cid, Vec<u8>)]) -> Vec<(Cid, Vec<u8>)> {
    emitted.iter().find(|(id, _)| *id == root).and_then(|(_, bytes)| repair::root_parity(bytes)).unwrap_or_default()
}

/// RACE PUT's accounting for one commit (COMMIT-LIFE §P): the blocks that
/// must be acked whatever (anything in no group: a root coded without
/// parity), and each
/// changed group's count toward k.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct Race {
    must: BTreeSet<Cid>,
    groups: Vec<RaceGroup>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct RaceGroup {
    k: usize,
    /// Members this commit did not change that are not an own straggler: on the network only once the NODE has
    /// said so (sdk#416, the architect's ruling: SAVED at k needs the node's word too). They count toward k only
    /// when confirmed (`Commit::held`), asked at `start_commit` (`Effect::ConfirmHeld`) and answered by the page
    /// from what it confirmed, at once when it knows. One entry per SLOT.
    #[serde(default)]
    others: Vec<Cid>,
    /// Members this commit did not change but an EARLIER commit is still
    /// putting (its stragglers): not known to be on the network until acked
    /// (the architect, rule 10's count). One entry per SLOT.
    #[serde(default)]
    earlier: Vec<Cid>,
    /// The group's blocks this commit puts (new members and new parity), one
    /// entry per SLOT: the same value under two keys of one leaf is ONE
    /// block filling TWO slots, and counted as a set it filled one -- a
    /// group of repeated values could never reach k, and its commit never
    /// published.
    new: Vec<Cid>,
}

impl Race {
    /// The accounting for a commit putting `data` (its new blocks) and
    /// `parity` (its coded parity). Groups are read from the NODES being put
    /// (the format states the grouping), and the ROOT is a group of one with
    /// `root.1` its parity (sdk#335: any 1 of its 1 + m); a block in no group
    /// -- a root coded without parity, or anything not found -- must be acked
    /// itself.
    fn of(data: &[(Cid, Vec<u8>)], parity: &[(Cid, Vec<u8>)], unacked: &BTreeSet<Cid>, root: (Cid, &[Cid])) -> Race {
        let new_ids: BTreeSet<Cid> = data.iter().chain(parity).map(|(c, _)| *c).collect();
        let mut covered: BTreeSet<Cid> = BTreeSet::new();
        let mut groups = Vec::new();
        for (_, bytes) in data {
            let Ok(node) = Node::parse(bytes) else { continue };
            let ids: Vec<Cid> = node.parity().collect();
            for (g, (_, members)) in freenet_prolly::parity::group_members(&node).into_iter().enumerate() {
                let Some(par) = ids.get(PARITY * g..PARITY * (g + 1)) else { continue };
                let new: Vec<Cid> = members.iter().chain(par).copied().filter(|c| new_ids.contains(c)).collect();
                if new.is_empty() {
                    continue;
                }
                let k = members.len();
                let earlier: Vec<Cid> = members.iter().copied().filter(|m| !new_ids.contains(m) && unacked.contains(m)).collect();
                let others: Vec<Cid> = members.iter().copied().filter(|m| !new_ids.contains(m) && !unacked.contains(m)).collect();
                covered.extend(new.iter().copied());
                groups.push(RaceGroup { k, others, earlier, new });
            }
        }
        let (root, root_parity) = root;
        if !root_parity.is_empty() && new_ids.contains(&root) {
            let new: Vec<Cid> = std::iter::once(root).chain(root_parity.iter().copied()).collect();
            covered.extend(new.iter().copied());
            groups.push(RaceGroup { k: 1, others: Vec::new(), earlier: Vec::new(), new });
        }
        let must = new_ids.into_iter().filter(|c| !covered.contains(c)).collect();
        Race { must, groups }
    }

    /// Every block that must be in is, and every changed group has k: its
    /// present members, what of this commit is acked, and what of an earlier
    /// commit's stragglers has been acked since (`unacked` is what still is
    /// not).
    fn recoverable(&self, confirmed: &BTreeSet<Cid>, unacked: &BTreeSet<Cid>, held: &BTreeSet<Cid>) -> bool {
        self.must.is_subset(confirmed)
            && self.groups.iter().all(|g| {
                g.others.iter().filter(|c| held.contains(*c)).count()
                    + g.new.iter().filter(|c| confirmed.contains(*c)).count()
                    + g.earlier.iter().filter(|c| !unacked.contains(*c)).count()
                    >= g.k
            })
    }

    /// Can every changed group still reach k, and every must-have block land, with `rejected` never on the network?
    /// (COMMIT-LIFE Heading x E4: a group fallen below k can never be recoverable.)
    fn reachable(&self, rejected: &BTreeSet<Cid>) -> bool {
        self.must.is_disjoint(rejected)
            && self.groups.iter().all(|g| g.others.len() + g.earlier.len() + g.new.iter().filter(|c| !rejected.contains(*c)).count() >= g.k)
    }

    /// Every changed group's other members (`RaceGroup::others`), each once.
    fn others(&self) -> BTreeSet<Cid> {
        self.groups.iter().flat_map(|g| g.others.iter().copied()).collect()
    }

}

impl Commit {
    /// A commit owing `data` (none confirmed), for CommitLife's own tests.
    #[cfg(test)]
    fn for_test(data: BTreeSet<Cid>) -> Commit {
        Commit {
            seq: 1,
            root: [0; 32],
            data,
            root_parity: Vec::new(),
            packs: BTreeMap::new(),
            confirmed: BTreeSet::new(),
            writes: Vec::new(),
            bytes: 0,
            base: [0; 32],
            through: 0,
            race: Race::default(),
            deferred: Vec::new(),
            held: BTreeSet::new(),
            unknown: BTreeSet::new(),
            pack_members: BTreeMap::new(),
        }
    }

    /// THE ONE "still owed" (COMMIT-LIFE C4, A4): this commit's blocks not yet confirmed. A rejected one stays owed
    /// (never BACKED_UP) and is never put again: the callers that put filter `rejected`.
    fn owed(&self) -> impl Iterator<Item = &Cid> + '_ {
        self.data.difference(&self.confirmed)
    }

    /// Is `id` one of [`Commit::owed`]?
    fn owes(&self, id: &Cid) -> bool {
        self.data.contains(id) && !self.confirmed.contains(id)
    }

    /// What the page holds the head for (`UpdateHead::after`): the blocks the
    /// engine COUNTED when it judged the head ready -- this commit's acked
    /// blocks -- and nothing else. Handing the page every data block instead
    /// made it wait for all of them (the page holds a head until every id in
    /// `after` is acked), which silently undid race put at the page while the
    /// engine looked done.
    fn race_set(&self) -> Vec<Cid> {
        self.data.iter().copied().filter(|c| self.confirmed.contains(c)).collect()
    }

    /// May the head be signed? (§P: race put; `all` is the control.)
    fn ready(&self, race: bool, unacked: &BTreeSet<Cid>) -> bool {
        if race {
            self.race.recoverable(&self.confirmed, unacked, &self.held)
        } else {
            self.confirmed.len() >= self.data.len()
        }
    }
}

/// A published commit's blocks still being put (§P): BACKED_UP when none is
/// left. Kept by the engine for this page's life; the page keeps the bytes.
/// Kept in the PAGE's engine memory only, never in the delegate-era context
/// (the architect, rules check): page memory and the keeper cover a reload
/// (rule 12).
#[derive(Clone, Debug)]
struct Backing {
    writes: Vec<(ClientId, WriteId)>,
    remaining: BTreeSet<Cid>,
}

/// The write pipeline.
/// A block source that also answers for the EMPTY LEAF.
///
/// A brand-new device's tree is one empty leaf, and until its first commit is
/// published nothing on the network holds it. That is not a block the engine
/// "keeps": `empty_leaf()` is a pure function of the format, so knowing it is
/// knowing a constant, the same way the engine knows what a pack header looks
/// like. Everything else goes to the real source.
struct WithEmptyLeaf<'a, B: Blocks> {
    inner: &'a B,
    empty_cid: Cid,
    empty_bytes: &'a [u8],
    /// Blocks handed to the engine THIS CALL and not yet visible through
    /// `inner` (sdk#34).
    ///
    /// A `BlockArrived` event carries the block's bytes. Until now they were
    /// checked and dropped, because the node that served them also holds them,
    /// so the next call could read them back through `Blocks`. That is true of
    /// a node that RETAINS. A node is free not to — a sync read does not
    /// refresh hosting (F33) — and then the bytes the engine was handed are
    /// the only copy in existence, and it had thrown them away.
    ///
    /// So they are used for the rest of THIS call and then dropped. Nothing is
    /// stored: this map never reaches the context, and the next call starts
    /// without it.
    arrived: &'a BTreeMap<Cid, Vec<u8>>,
}

impl<B: Blocks> Blocks for WithEmptyLeaf<'_, B> {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        // The real source first: once the leaf is published the node's copy is
        // the one to use, and it is byte-identical anyway.
        self.inner
            .get(cid)
            .or_else(|| self.arrived.get(cid).map(|b| b.as_slice()))
            .or_else(|| {
                if *cid == self.empty_cid {
                    Some(self.empty_bytes)
                } else {
                    None
                }
            })
    }
}

/// A parked delta's resume key and established prefix (`Engine::delta_prefix_for_test`, sdk#135).
pub type DeltaPrefix = (Option<Vec<u8>>, read::Changes);

pub struct Engine<B: Blocks> {
    /// The empty leaf, which a device with no head has as its root and which
    /// nothing on the network holds until the first commit publishes it.
    /// A constant of the format, not a cache.
    empty: freenet_prolly::chunk::Closed,
    /// Where blocks come from. The core owns NO block bytes: a delegate's
    /// memory is fresh on every call, so anything it "kept" would be gone by
    /// the next one. What it can do is ASK — and on a node that is the
    /// synchronous local read (F14), which sees what this node already holds.
    blocks: B,
    /// Blocks handed to this engine during the CURRENT call, and dropped when
    /// it ends (sdk#34). Never in the context — see [`WithEmptyLeaf::arrived`].
    arrived: BTreeMap<Cid, Vec<u8>>,
    /// Blocks the node's Block contract rejected (`Event::PutRejected`, sdk#433): never put again.
    rejected: BTreeSet<Cid>,
    /// Re-puts that found the block's bytes GONE from the store (sdk#411): a commit or a straggler that can never be
    /// put again. The pin rule's model property: always 0.
    reput_missing: u64,
    /// THE W PIN's one record (sdk#450, the architect): every block a queued write's warm apply emitted
    /// (`Effect::Keep`) that the node has NOT confirmed, with its bytes -- each id ONCE (a content-addressed block
    /// two writes emit is held once). The only copy while any write is in flight: it leaves when the node confirms
    /// it (then the node holds it) and all of it is forgotten at the end of a step with no write in flight.
    kept: BTreeMap<Cid, usize>,
    kept_bytes: usize,
    /// Blocks being REBUILT from their groups, by the block's id (`repair`).
    repairs: BTreeMap<Cid, Repair>,
    /// WHO WANTS each block (WANTED-LIFE): the parked reads, the repair slots and the parked write's needs, in one type
    /// only its four transitions can change.
    wanted: Wanted,
    /// Blocks whose LAST reader dropped during THIS call (WANTED-LIFE): settled at the call's end into
    /// [`Effect::Unwanted`] for those still unwanted and not held ([`Engine::settle_unwanted`]), and empty between
    /// calls -- the withdrawal is an effect, never stored state.
    dropped: BTreeSet<Cid>,
    /// Asks that went out, by why (sdk#303): `.0` WANTED (a read waits on the block), `.1` RACED (only a race or a
    /// repair asks it). A read's cap counts the wanted; its race rides inside its one fetch.
    fetch_counts: (usize, usize),
    /// This step's asks, by who pushed them: `true` a read (or a write, or a preload), `false` only a race or a
    /// repair. Counted into `fetch_counts` where the step's asks leave (`one_ask_each`), then cleared.
    step_asks: BTreeMap<Cid, bool>,
    /// Blocks REBUILT this step, to land like arrivals once the step's own work is done (sdk#303). Landed by a
    /// loop, never from inside the repair that rebuilt them: one arrival can finish many races, each landing wakes
    /// reads that race and rebuild again, and done recursively that chain is as deep as a level is wide (a
    /// fetch-ahead lookup overflowed the stack).
    landing: Vec<(Cid, Vec<u8>)>,
    /// Repairs started, finished (verified, kept), and given up.
    repair_counts: (u64, u64, u64),
    /// PARITY the node answered NotFound in a race or repair (sdk#479, the owner: a group whose parity is never
    /// replaced erodes to its last copy), with its group: re-encoded and PUT back once the page holds all `k` members
    /// ([`Engine::put_owed_parity`]). Bounded by the NotFounds seen.
    parity_owed: BTreeMap<Cid, repair::Group>,
    /// REPAIR mode ([`Engine::set_repair_pass`]): each parity slot a race asked, with its group, until the node answers
    /// it -- NotFound makes it owed ([`Engine::parity_owed`]).
    repair_pass: bool,
    parity_watch: BTreeMap<Cid, repair::Group>,
    /// Put back what a read rebuilds (true, every read and REPAIR) or only COUNT it (false: a CHECK pass, sdk#479 --
    /// nothing a read rebuilds or re-encodes is PUT; [`Engine::set_put_back`]).
    put_back: bool,
    /// Every block a read found LOST: a member rebuilt from its group, a parity block the node answered NotFound
    /// (sdk#479). The page's report counts it missing unless the node served it before any repair.
    lost: BTreeSet<Cid>,
    /// Parity found missing, put back (re-encoded, id-verified), and re-encoded to a DIFFERENT id (never put).
    parity_counts: (u64, u64, u64),
    /// Why the last repair was given up, in words (the read is answered
    /// `Unavailable` naming the block; this says why the group could not).
    repair_failed: Option<String>,
    params: Params,
    /// The warm tree: every block this engine has written. In slice 1 it is
    /// also the only place they exist, because there is no node yet.
    root: Cid,
    /// The last head the network has confirmed, and its root. Recovery starts
    /// from here, never from the warm tree.
    published_seq: u64,
    published_root: Cid,
    next_seq: u64,
    /// THE COMMIT'S LIFE (COMMIT-LIFE rev 5): the one commit in flight, as one stage, and the published commits
    /// still putting toward BACKED_UP. Its transitions are its only mutators ([`commit`]).
    life: CommitLife,
    /// Whether every group of the published tree is known recoverable
    /// (sdk#119): an engine that tracked the tree from empty, through its own
    /// race-put commits (§P). Not carried in the context.
    parity_scan: ParityScan,
    /// Unanswered asks, for pacing only (`asks`).
    asks: asks::Asks,
    /// Shed this call to stay saveable (`keep_saveable`); not carried.
    shed: Shed,
    /// A write whose tree path is not held, waiting for the blocks it needs.
    ///
    /// At most ONE, and it costs the context its ops. That is affordable only
    /// because one commit at a time already means a second write is refused:
    /// a queue of parked writes would put an unbounded number of client values
    /// in a 400 KiB context.
    parked_write: Option<ParkedWrite>,
    /// Where this engine's authority came from, as STATED at `Start`.
    key: Option<KeySource>,
    /// Epochs still to try when looking for this device's head.
    epochs: Vec<Epoch>,
    head_epoch: Option<Epoch>,
    recovered: bool,
    /// The last `Event::CanSign` (true until told otherwise): the consequence
    /// of the events it was told, never a polled or copied field. Page memory
    /// only, never in the context: the page steps it again on each change.
    can_sign: bool,
    /// Reads that arrived before the head was recovered, in arrival order
    /// (sdk#223). The tree before recovery is the EMPTY tree, and answering
    /// from it said "complete, and there is nothing here" about data the head
    /// had not been read to find. They wait here, and are answered — in order —
    /// the moment `HeadRead` or `HeadMissing` recovers the root.
    before_head: Vec<(ClientId, read::ReqId, read::Want)>,
    /// THE PAGE'S WRITE QUEUE (R-b; COMMIT-LIFE § A write's stage in the
    /// page's queue): writes taken and not yet committed, in arrival order.
    /// The FRONT may be the one the commit in flight carries (`committing`).
    /// Engine-local, never in the context: the page keeps its engine, and a
    /// reload loses the queue as it lost the outbox (named in the table).
    queue: std::collections::VecDeque<Queued>,
    /// Rebuilt commits whose root differed from the warm root the write was
    /// applied into, with no foreign move between (K9). Must stay 0; the
    /// model reads it.
    rebuild_differs: u64,
    /// Warm-root re-derivations done on this page's OWN publish (footnote 4:
    /// must stay 0 — re-deriving there is O(queue²) in a burst).
    own_publish_rederived: u64,
    /// Set for the length of `on_head` (OWN publish), so a re-derivation
    /// there is counted where it is made.
    in_own_publish: bool,
    /// The next arrival number (K9).
    next_arrival: u64,
    /// Sessions told `QueueFull` that have not been taken since (review §2 on
    /// sdk#295): each later write of theirs is `QueueFull` too, until the
    /// session has nothing queued, so a smaller later write never lands before
    /// the refused one. Transient: not in a context (a restored engine holds
    /// no queue to overtake).
    queue_blocked: BTreeSet<ClientId>,
    /// Tries the NEXT write taken has already spent: set by a `Db` re-run
    /// just before it makes the write (ONE budget, sdk#265), taken by that
    /// write, cleared by the call that set it.
    next_tries: Option<u32>,
    /// Writes told `Published` by a NO-OP group (#164): no commit carried
    /// them, the tree already held them.
    noop_published: u64,
    /// Nothing is applied or cut while set (review §1 on sdk#295): a same-seq
    /// race is being MERGED, and the displaced group must go at the FRONT,
    /// before any later write is judged or commits. Set by the page before the displacing head is
    /// stepped; cleared by [`Engine::merge_front`] or [`Engine::release_cut`].
    cut_held: bool,
    /// The cut being shipped (K9 §1, §4, §6): the arrival number of its last
    /// write, until every write of it has published. A later piece, and a
    /// re-send after `Lost`, take no write past it.
    cut_until: Option<u64>,
    /// The `through` the head about to be adopted records for this page (its
    /// ledger), set by the page before it steps the foreign move: the WITNESS
    /// that a dead commit's group LANDED (COMMIT-LIFE ⁵). Consumed by that
    /// step.
    witness: Option<Witness>,
    /// The next adopted head's §P mark and root parity (see
    /// [`Engine::set_head_mark`]).
    head_mark: Option<Vec<Cid>>,
    /// Each known root's parity ids (sdk#335): what makes a root a group of
    /// one for a read. Kept for the published root and the roots reads are
    /// pinned to, nothing else.
    root_parity: BTreeMap<Cid, Vec<Cid>>,
    /// Writes that ended in THIS step with keys a later write may have read:
    /// a later write's conflict on one of those keys names it as `after`
    /// (footnote 3). Cleared at the end of every step, like `arrived`.
    cascade: Vec<(ClientId, WriteId, BTreeSet<Vec<u8>>)>,
    /// Writes that fell `Lost` in the engine: forced (`Expect::Any`, never
    /// re-applied, WRITE-PATH ⁷) and tries spent.
    forced_lost: u64,
    tries_spent_lost: u64,
    /// Stage moves the table calls impossible (footnote 1). Must stay 0.
    impossible_transitions: u64,
    /// Groups a dead commit's witness showed LANDED (⁵): told Published.
    landed_by_witness: u64,
    /// Dead commits' writes whose landing could not be known (⁵): `Unknown`.
    unknown_fates: u64,
    /// Own commits published, and the queued writes they carried: writes per
    /// commit, what group commit is measured by (K9).
    commits_published: u64,
    writes_published: u64,
    /// Writes TAKEN with at least one `Expect::Any` read: forced past their
    /// reads (sdk#235). Shown to a person, so the transitional form is a
    /// number someone can act on (sdk#281 removes `Any` at zero).
    forced_writes: u64,
    /// Blocks written since the last commit shipped. Accumulated as each
    /// write emits them rather than recovered later by comparing two trees:
    /// the emitting is where the answer is already known, and walking for it
    /// afterwards is the same answer computed again, more expensively and
    /// with a chance of disagreeing.
    unpublished: Vec<(Cid, Vec<u8>)>,
    /// Everything the read path is waiting on.
    reads: read::Reads,
    /// Who is watching which range.
    subs: subs::Subs,
    /// Nodes parsed on the write path. A cost counter, not a statistic: the
    /// whole point of the diff walk is that this stays proportional to the
    /// tree's DEPTH, and a test that does not measure it would not notice the
    /// day it goes back to being proportional to its SIZE.
    nodes_parsed: usize,
    now: u64,
}

impl<B: Blocks> Engine<B> {
    /// Anything a caller can set must not be able to panic the core later.
    /// `max_packed_value` above the limit describes a value that must ride in
    /// a pack and cannot; the planner would reach an `unreachable!` three steps
    /// away, where nothing points back at the setting that caused it. Refused
    /// here instead, naming both numbers.
    ///
    /// **The limit is the PER-MEMBER one, not the pack's.** This bounded
    /// `max_packed_value` against `max_pack` — the container — and packed
    /// members are `RAW`, so every setting in [262,209, 1,048,565] passed here
    /// and produced packs the contract refuses per member: a 786,357-wide band
    /// the engine accepted and the network would not. That is worse than a
    /// panic, because the refusal happens REMOTELY, where a node can log
    /// nothing at the moment it refuses (F48). Both numbers are needed: a
    /// member must fit its kind's limit AND the pack must be able to hold one
    /// (craftworks-sdk#117).
    pub fn new(params: Params, blocks: B) -> Self {
        assert!(
            params.max_packed_value <= pack::max_body(core_types::kind::BlockKind::Raw),
            "max_packed_value ({}) is above what the network accepts for one \
             member of its kind ({}): the pack would be built here and refused \
             THERE, per member, where nothing local says why",
            params.max_packed_value,
            pack::max_body(core_types::kind::BlockKind::Raw)
        );
        assert!(
            params.max_packed_value + pack::member_cost(0) + pack::PACK_HEADER <= params.max_pack,
            "max_packed_value ({}) cannot fit in a pack of max_pack ({}): a value \
             that must be packed and cannot be is a commit that can never ship",
            params.max_packed_value,
            params.max_pack
        );
        assert!(
            params.max_pack > pack::PACK_HEADER,
            "max_pack holds no members"
        );
        // EVERY CAP HAS A BYTE MEANING, AND THEY SUM UNDER THE BOUND (sdk#162).
        // Everything the context carries that is not a parked read is capped;
        // its worst case, from each unit's measured size, must leave the
        // parked reads at least `min_parked_read_bytes` -- they take the rest,
        // and are the first thing shed when the bound is reached.
        //
        // `max_context_bytes: usize::MAX` is an engine whose context is never
        // saved -- an in-process writer building a fixture, with no returns
        // and no bound -- and has no sum to keep.
        let fixed = worst_case_fixed_bytes(&params);
        let room = params
            .max_context_bytes
            .saturating_sub(params.shell_context_reserve + CONTEXT_HEADER);
        assert!(
            params.max_context_bytes == usize::MAX
                || fixed.saturating_add(params.min_parked_read_bytes) <= room,
            "the context's caps do not fit its bound: {fixed} B fixed + {} B for parked reads \
             > {room} B (max_context_bytes {} less the shell's {} and the header)",
            params.min_parked_read_bytes,
            params.max_context_bytes,
            params.shell_context_reserve
        );
        // The asks table is carried, so its worst case is part of the
        // context's. This bounds ITS share; whether every carried cap sums
        // under the bound is sdk#162's question, not answered here.
        assert!(
            params.max_asks.saturating_mul(asks::ASK_BYTES)
                <= params.max_context_bytes / asks::CONTEXT_SHARE,
            "max_asks ({}) x {} B is over 1/{} of max_context_bytes ({})",
            params.max_asks,
            asks::ASK_BYTES,
            asks::CONTEXT_SHARE,
            params.max_context_bytes
        );
        let empty = empty_leaf();
        let root = empty.cid;
        Engine {
            params,
            empty,
            blocks,
            arrived: BTreeMap::new(),
            rejected: BTreeSet::new(),
            reput_missing: 0,
            kept: BTreeMap::new(),
            kept_bytes: 0,
            repairs: BTreeMap::new(),
            wanted: Wanted::default(),
            dropped: BTreeSet::new(),
            fetch_counts: (0, 0),
            step_asks: BTreeMap::new(),
            landing: Vec::new(),
            repair_counts: (0, 0, 0),
            parity_owed: BTreeMap::new(),
            repair_pass: false,
            parity_watch: BTreeMap::new(),
            put_back: true,
            lost: BTreeSet::new(),
            parity_counts: (0, 0, 0),
            repair_failed: None,
            root,
            published_seq: 0,
            published_root: root,
            next_seq: 1,
            life: CommitLife::default(),
            parity_scan: ParityScan::NotScanned,
            asks: asks::Asks::default(),
            shed: Shed::default(),
            parked_write: None,
            key: None,
            epochs: Vec::new(),
            head_epoch: None,
            recovered: false,
            can_sign: true,
            before_head: Vec::new(),
            queue: std::collections::VecDeque::new(),
            rebuild_differs: 0,
            own_publish_rederived: 0,
            in_own_publish: false,
            next_arrival: 1,
            queue_blocked: BTreeSet::new(),
            next_tries: None,
            noop_published: 0,
            cut_held: false,
            cut_until: None,
            witness: None,
            head_mark: None,
            root_parity: BTreeMap::new(),
            cascade: Vec::new(),
            forced_lost: 0,
            tries_spent_lost: 0,
            impossible_transitions: 0,
            landed_by_witness: 0,
            unknown_fates: 0,
            commits_published: 0,
            writes_published: 0,
            forced_writes: 0,
            unpublished: Vec::new(),
            reads: read::Reads::default(),
            subs: subs::Subs::default(),
            nodes_parsed: 0,
            now: 0,
        }
    }

    /// The warm tree's current root, including writes not yet published.
    pub fn root(&self) -> Cid {
        self.root
    }

    /// The root other readers can see.
    /// The seq of the head this engine has published, or 0 if it has none.
    ///
    /// 0 means no head exists ANYWHERE for this key — not merely that this
    /// engine has not written one — because a fresh start re-reads the head
    /// before anything else and adopts whatever is there.
    pub fn published_seq(&self) -> u64 {
        self.published_seq
    }

    pub fn published_root(&self) -> Cid {
        self.published_root
    }

    /// Groups whose redundancy does not exist yet.
    /// The parameters this engine runs under.
    pub fn params(&self) -> &Params {
        &self.params
    }

    /// Whether [`Engine::owed_groups`] covers the whole published tree
    /// (sdk#119). A zero is "all put" ONLY under `Done`; under `NotScanned`
    /// it means nothing is known beyond what this engine coded itself — and
    /// every surface that reports redundancy must say so, never "0 owed".
    pub fn parity_scan(&self) -> &ParityScan {
        &self.parity_scan
    }

    /// Writes taken with an `Expect::Any` read: forced past their reads.
    pub fn forced_writes(&self) -> u64 {
        self.forced_writes
    }

    /// Writes in the page's queue (R-b): `Applying`, `Queued` or `Committing`.
    pub fn queued_writes(&self) -> usize {
        self.queue.len()
    }

    /// Writes taken and not yet published -- WHAT IS UNSAVED, its one owner
    /// (sdk#350): every queued write but a DEFERRED one still held (a define
    /// no commit has taken is nothing the person made; it is unsaved only
    /// once a cut carries it).
    pub fn unsaved_writes(&self) -> usize {
        self.queue.iter().filter(|q| q.is_unsaved(self.carries(q))).count()
    }

    /// The client of every UNSAVED write ([`Engine::unsaved_writes`]'s rule,
    /// per write): what a session counts as its own unsaved writes -- the
    /// unsaved-changes guard and "saving N…" -- so a held define never makes
    /// a viewer's close say "unsaved changes" (sdk#350).
    pub fn unsaved_clients(&self) -> impl Iterator<Item = ClientId> + '_ {
        self.queue.iter().filter(|q| q.is_unsaved(self.carries(q))).map(|q| q.client)
    }

    /// May a commit be cut (sdk#350)? Only with a NON-deferred write applied
    /// in the queue: judged on the queue, not per cut piece, so a byte-limit
    /// split whose first piece is defines only still goes. A non-deferred
    /// write still applying does not count -- it may yet be refused, and a
    /// define must never commit alone.
    fn in_company(&self) -> bool {
        self.queue.iter().any(|q| !q.deferred && q.warm_after.is_some())
    }

    /// Every write in the queue, in order, with its stage (R-b).
    pub fn queue_stages(&self) -> impl Iterator<Item = (ClientId, WriteId, Stage)> + '_ {
        self.queue.iter().map(|q| {
            let stage = match (q.warm_after, self.carries(q)) {
                (None, _) => Stage::Applying,
                (Some(_), false) => Stage::Queued,
                (Some(_), true) => Stage::Committing,
            };
            (q.client, q.write_id, stage)
        })
    }

    /// Does a write still APPLYING (in no root yet) write a key in
    /// `[lo, hi)`? A read of that range waits for it (R-b: reads and
    /// `Applying`), or it would miss this page's own write.
    pub fn applying_touches(&self, lo: &[u8], hi: &[u8]) -> bool {
        self.queue
            .iter()
            .filter(|q| q.warm_after.is_none())
            .any(|q| q.ops.iter().any(|(k, _)| k.as_slice() >= lo && k.as_slice() < hi))
    }

    /// The stage of the LAST queued write that writes `key`: what that key's
    /// own write is doing now. `None`: no write in the queue touches it.
    pub fn stage_of_key(&self, key: &[u8]) -> Option<Stage> {
        self.queue_stages()
            .zip(self.queue.iter())
            .filter(|(_, q)| q.ops.iter().any(|(k, _)| k.as_slice() == key))
            .last()
            .map(|((_, _, s), _)| s)
    }

    /// Writes queued and bytes of them: what `QueueFull` is measured against.
    pub fn queue_load(&self) -> (usize, usize) {
        (self.queue.len(), self.queued_bytes())
    }

    /// K9 (COMMIT-LIFE footnote 2): rebuilt commits whose root differed from
    /// the warm root their write made, with no foreign move between. Must be 0.
    pub fn rebuild_differs(&self) -> u64 {
        self.rebuild_differs
    }

    /// Re-derivations of the warm root on OWN publish (footnote 4). Must be 0.
    pub fn own_publish_rederived(&self) -> u64 {
        self.own_publish_rederived
    }

    /// Stage moves the table calls impossible (footnote 1). Must be 0.
    pub fn impossible_transitions(&self) -> u64 {
        self.impossible_transitions + self.life.in_flight_supersedes()
    }

    /// THE COMMIT'S STAGE (COMMIT-LIFE rev 5, C1): what the commit table's rows name, read by its model.
    pub fn commit_stage(&self) -> CommitStage {
        self.life.stage()
    }

    /// Writes recorded as already told `Stalled` (COMMIT-LIFE C4, A5): a write that has LEFT the queue must not be
    /// among them.
    pub fn stalled_told(&self) -> usize {
        self.queue.iter().filter(|q| q.stalled_told).count()
    }

    /// Own commits published and the queued writes they carried (K9).
    pub fn commits_and_writes(&self) -> (u64, u64) {
        (self.commits_published, self.writes_published)
    }

    /// Dead commits' writes the witness showed landed (told Published, ⁵).
    /// THE ONE BUDGET (sdk#265): the next write taken is a `Db` re-run of a
    /// write that had spent `tries` -- it goes on from there, and falls
    /// `Lost` past [`Params::max_write_tries`] as any write does.
    pub fn carry_tries(&mut self, tries: u32) {
        self.next_tries = Some(tries);
    }

    pub fn clear_carried_tries(&mut self) {
        self.next_tries = None;
    }

    pub fn max_write_tries(&self) -> u32 {
        self.params.max_write_tries
    }

    /// Writes told `Published` by a no-op group (see the field).
    pub fn noop_published(&self) -> u64 {
        self.noop_published
    }

    /// Published commits still putting blocks toward BACKED_UP (§P).
    pub fn backing(&self) -> usize {
        self.life.backing_len()
    }

    /// Blocks published commits are still putting (not yet acked).
    fn unacked(&self) -> BTreeSet<Cid> {
        self.life.unacked()
    }


    /// Read repairs through parity: `(started, rebuilt, given up)`.
    pub fn repair_counts(&self) -> (u64, u64, u64) {
        self.repair_counts
    }

    /// THE READS WAITING ON A DAMAGED GROUP (sdk#524): each repair whose missing block was answered NotFound and whose
    /// group has fewer than `k` blocks NOT answered NotFound -- [`repair::GroupHealth::of`] DAMAGED, the audit's own
    /// derivation and word. "Present" here is OPTIMISTIC (a slot not yet answered, or silent, may be there; the
    /// audit's is what it verified), so DAMAGED is named only when even that count is below `k`. DERIVED on each
    /// call, never stored: a state, not an end -- the block and its slots are still asked, and a late answer still
    /// lands (rule 8).
    pub fn damaged(&self) -> Vec<Damaged> {
        self.repairs
            .values()
            // Named only for a block a live READ waits on (Codex on sdk#542): two sibling repairs each wanting the
            // other's block outlive their superseded reads, and nobody is waiting on them.
            .filter(|r| r.missed && self.readers_of(&r.group.missing).reads)
            .filter_map(|r| {
                let slots = r.group.slots.len();
                let j = slots - r.absent.len().min(slots);
                let m = slots - r.group.k;
                (repair::GroupHealth::of(j, r.group.k, m) == repair::GroupHealth::Damaged).then_some(Damaged { block: r.group.missing, j, k: r.group.k })
            })
            .collect()
    }

    /// REPAIR mode on or off (sdk#479): on, every race keeps its group's parity GETs out until the NODE answers each
    /// ([`REPAIR_PASS`]), so a parity block the node lacks is learned even when it answers with silence. Off (every
    /// normal read): a race withdraws what it no longer needs.
    pub fn set_repair_pass(&mut self, on: bool) {
        self.repair_pass = on;
    }

    /// PUT back what reads rebuild (true: every read, and REPAIR) or only count it (false: a CHECK pass, sdk#479).
    /// Gates BOTH put paths at their source: a rebuilt member's [`Effect::PutRepaired`] and an owed parity's.
    pub fn set_put_back(&mut self, on: bool) {
        self.put_back = on;
    }

    /// Every block a read found lost: members rebuilt, parity answered NotFound (sdk#479).
    pub fn lost(&self) -> &BTreeSet<Cid> {
        &self.lost
    }

    /// Parity the reads found missing, put back, and re-encoded to a different id (sdk#479): `(missing, put, mismatched)`.
    pub fn parity_counts(&self) -> (u64, u64, u64) {
        self.parity_counts
    }

    /// Why the last read repair was given up, if one was.
    pub fn repair_failed(&self) -> Option<&str> {
        self.repair_failed.as_deref()
    }

    pub fn landed_by_witness(&self) -> u64 {
        self.landed_by_witness
    }

    /// The `through` the head about to be adopted records for this page
    /// (its ledger's entry), before the foreign move is stepped (⁵).
    pub fn set_witness(&mut self, witness: Option<Witness>) {
        self.witness = witness;
    }

    /// The head about to be adopted: does it carry race put's §P mark (every
    /// group it lists was recoverable when signed), and its ROOT's parity ids
    /// (sdk#335; empty when the mark lists none)? `None`: unmarked. Told by
    /// the shell before the step, like the witness; read once by `adopt`.
    pub fn set_head_mark(&mut self, mark: Option<Vec<Cid>>) {
        self.head_mark = mark;
    }

    /// The ROOT's parity ids for a head naming `root` (sdk#335): the root is
    /// a group of ONE, coded like any group (k = 1, [`PARITY`] parity), and
    /// its ids ride in the head's ledger, signed with it -- the one place a
    /// reader of that head learns them. The commit in flight's, or a known
    /// root's; empty for a root this engine did not code or learn.
    pub fn root_parity_of(&self, root: &Cid) -> Vec<Cid> {
        match self.life.commit() {
            Some(c) if c.root == *root => c.root_parity.clone(),
            _ => self.root_parity.get(root).cloned().unwrap_or_default(),
        }
    }

    /// The last arrival number of the commit in flight: what its head's
    /// ledger records as this page's `through`.
    pub fn committing_through(&self) -> Option<u64> {
        self.life.commit().map(|c| c.through).filter(|t| *t > 0)
    }

    /// HOLD THE QUEUE (review §1 on sdk#295): until [`Engine::merge_front`]
    /// or [`Engine::release_cut`], writes are taken but none is applied or
    /// committed -- a merge's displaced group is about to go at the front.
    pub fn hold_cut(&mut self) {
        self.cut_held = true;
    }

    pub fn cut_held(&self) -> bool {
        self.cut_held
    }

    /// The hold ends with nothing to merge: the queue moves on.
    pub fn release_cut(&mut self) -> Vec<Effect> {
        self.cut_held = false;
        self.advance()
    }

    /// THE MERGE'S WRITES GO AT THE FRONT (sdk#225b; review §1 on sdk#295):
    /// the displaced group is re-applied onto the winner BEFORE every write
    /// that came after it, in its order -- the same path as a dead cut
    /// (`requeue_from(0)`). Queued behind, a later own write would commit
    /// first and the older value land over it (or be told superseded by the
    /// page's own write). Its bytes were counted when first taken: no bound.
    /// Arrival numbers are given again from here, in the new order (none of
    /// these writes is in a head: the cut was held), so arrival order stays
    /// apply order and the witness's `through` stays one number.
    pub fn merge_front(&mut self, client: ClientId, writes: Vec<MergeWrite>) -> Vec<Effect> {
        self.cut_held = false;
        let at = self.queue.iter().take_while(|q| self.carries(q)).count();
        if at > 0 {
            // The hold was not in place when the cut was made.
            self.impossible_transitions += 1;
        }
        for (n, (write_id, ops, reads)) in writes.into_iter().enumerate() {
            let size = ops.iter().map(|(k, o)| k.len() + if let Op::Put(v) = o { v.len() } else { 0 }).sum::<usize>()
                + reads.iter().map(|(k, _)| k.len() + 33).sum::<usize>();
            // Never deferred (sdk#350): a displaced group was cut, so it was
            // already in company, and it must land again on its own.
            self.queue.insert(at + n, Queued { client, write_id, ops, reads, size, tries: 0, told_accepted: true, warm_after: None, stalled_told: false, arrival: 0, deferred: false });
        }
        for q in self.queue.iter_mut().skip(at) {
            q.arrival = self.next_arrival;
            self.next_arrival += 1;
        }
        if at == 0 {
            self.cut_until = None;
        }
        self.root = self.queue.iter().take(at).next_back().and_then(|q| q.warm_after).unwrap_or(self.published_root);
        self.requeue_from(at);
        self.advance()
    }

    /// The seq the commit in flight would land at: what a head's `through`
    /// list is compared against to tell an evicted entry from one never
    /// there (review §3 on sdk#295).
    pub fn committing_seq(&self) -> Option<u64> {
        self.life.commit().map(|c| c.seq)
    }

    /// Does the commit in flight carry `q`? DERIVED from its cut (COMMIT-LIFE C4, A3): no per-write flag to disagree.
    fn carries(&self, q: &Queued) -> bool {
        self.life.commit().is_some_and(|c| c.writes.contains(&(q.client, q.write_id)))
    }

    /// Writes recorded as told `Stalled` -- the state walk's form of the per-write flag.
    fn stalled_told_writes(&self) -> BTreeSet<(ClientId, WriteId)> {
        self.queue.iter().filter(|q| q.stalled_told).map(|q| (q.client, q.write_id)).collect()
    }

    /// Writes that fell `Lost` in the engine: `(forced, tries spent)`.
    pub fn lost_fell(&self) -> (u64, u64) {
        (self.forced_lost, self.tries_spent_lost)
    }

    /// Groups with parity not yet on the network: under race put (§P) a
    /// commit's parity goes with it and nothing is owed, so what is left is
    /// the published commits still putting toward BACKED_UP.
    pub fn owed_groups(&self) -> usize {
        self.life.backing_len()
    }

    /// Nodes parsed on the write path since the last [`Engine::reset_cost`].
    ///
    /// Exposed so a test can assert the write path is proportional to the
    /// tree's depth and not to its size. It is the only honest way to keep
    /// that true: the O(tree) version was correct, passed every test, and
    /// would have cost roughly 10^5 node parses per keystroke at a million
    /// keys, inside a 5 s delegate call.
    pub fn nodes_parsed(&self) -> usize {
        self.nodes_parsed
    }

    pub fn reset_cost(&mut self) {
        self.nodes_parsed = 0;
    }

    pub fn step(&mut self, event: Event) -> Vec<Effect> {
        if self.params.redescend_on_entry {
            // The control: every entry re-drives every parked read, whether
            // or not anything it waits on has changed.
            let parked: Vec<read::ReqId> = self.reads.parked.keys().copied().collect();
            let mut out = Vec::new();
            for r in parked {
                out.extend(self.drive(r));
            }
            out.extend(self.step_inner(event));
            out.extend(self.land_rebuilt());
            out.extend(self.put_owed_parity());
            out.extend(self.keep_saveable());
            out.extend(self.settle_unwanted());
            self.cascade.clear();
            self.arrived.clear();
            self.forget_kept_if_idle();
            self.one_ask_each(&mut out);
            return out;
        }
        let mut out = self.step_inner(event);
        out.extend(self.land_rebuilt());
        out.extend(self.put_owed_parity());
        out.extend(self.keep_saveable());
        out.extend(self.settle_unwanted());
        self.cascade.clear();
        self.arrived.clear();
        self.forget_kept_if_idle();
        self.one_ask_each(&mut out);
        out
    }

    /// W is the only copy only while a write is queued or a commit pending: with neither, forgotten.
    fn forget_kept_if_idle(&mut self) {
        if !self.has_writes_in_flight() {
            self.kept.clear();
            self.kept_bytes = 0;
        }
    }

    /// THE ONE SUM of what the write queue holds (sdk#450, rule 9): its writes' OPS bytes plus the bytes the W pin
    /// holds for them (their warm-apply blocks, each once). `QueueFull` is measured against it, and so is
    /// `queue_load`: no second form of "queued bytes".
    pub fn queued_bytes(&self) -> usize {
        self.queue.iter().map(|q| q.size).sum::<usize>() + if self.has_writes_in_flight() { self.kept_bytes } else { 0 }
    }

    /// ONE ask per block per step (sdk#303). A block can be wanted for several reasons in one step -- a read, a
    /// race for its sibling, a repair re-asking a slot -- and each reason used to push its own `FetchBlock`. The
    /// page sends one GET per block while one is out anyway; this keeps the engine's own asks, and its `fetches`
    /// count, equal to what goes out.
    fn one_ask_each(&mut self, out: &mut Vec<Effect>) {
        let mut seen = BTreeSet::new();
        let before = out.len();
        out.retain(|e| match e {
            Effect::FetchBlock { id, .. } => seen.insert(*id),
            _ => true,
        });
        self.reads.fetches = self.reads.fetches.saturating_sub(before - out.len());
        for id in &seen {
            match self.step_asks.get(id) {
                Some(false) => self.fetch_counts.1 += 1,
                _ => self.fetch_counts.0 += 1,
            }
        }
        self.step_asks.clear();
    }

    /// What this engine shed to stay saveable, since the last `take_shed`.
    /// Per call, never carried: it is what the call report says.
    pub fn take_shed(&mut self) -> Shed {
        std::mem::take(&mut self.shed)
    }

    /// KEEP THE CONTEXT SAVEABLE, at the end of every step (sdk#162).
    ///
    /// A context over `max_context_bytes` was not saved at all: the node kept
    /// the PREVIOUS call's while this call's effects had already gone, and the
    /// request behind it was told nothing, ever. Checking requests at the
    /// door cannot prevent it -- the context also grows on the node's
    /// ANSWERS (a parked scan's `held`, waiters moving at publish) -- so the
    /// bound is kept here, after whatever the step did, by shedding work in a
    /// NAMED order, each with its own verdict:
    ///   1. parity waiters past `max_parity_waiting_refs`: the OLDEST waiter,
    ///      which will not hear ParityComplete (counted);
    ///   2. past the byte bound: the newest parked READ (`Unavailable`, as the
    ///      count cap answers), then the parked WRITE (`Busy`: it may re-send).
    ///      Never the commit in flight, and never OWED parity: it is not
    ///      re-derivable from the tree today (sdk#181), so it is bounded where
    ///      it grows -- past its cap a write's parity is left uncoded
    ///      (`record_owed`) -- and never forgotten here.
    ///
    /// If none of that fits it, the backstop below is a loud debug assert.
    /// No context is written any more (#305): whether this budget still
    /// earns its verdicts is sdk#385.
    ///
    /// ITS COST, MEASURED (sdk#187 review), so nobody optimises it blind: it
    /// sizes the live state by reference each step (`context_len`). Release,
    /// the burst test that parks ~10,000 reads in ONE engine
    /// (`context.rs`): 0.08 s on main, 0.15 s here -- about 7 us a step,
    /// noise beside a delegate's handful of steps a call. Debug, the same
    /// test: 0.30 -> 10.4 s; every other engine suite within +0.8 s.
    fn keep_saveable(&mut self) -> Vec<Effect> {
        let mut out = Vec::new();
        let limit = self
            .params
            .max_context_bytes
            .saturating_sub(self.params.shell_context_reserve);
        while self.context_len() > limit {
            if let Some(req_id) = self.reads.parked.keys().next_back().copied() {
                let p = self.reads.parked.remove(&req_id).expect("just listed");
                let result = self.gave_up(&p.want, p.root);
                self.forget_waiting(req_id);
                self.shed.reads += 1;
                out.push(Effect::Reply {
                    client: p.client,
                    req_id,
                    result,
                });
            } else if let Some(pw) = self.release_write() {
                // A BACKSTOP NO HONEST TEST REACHES: `Engine::new` asserts the
                // fixed worst case -- the parked write at its cap included --
                // plus `min_parked_read_bytes` fits the room, so once every
                // parked read is shed the context is under the limit and this
                // arm is not reached. Reaching it means
                // `worst_case_fixed_bytes` UNDERESTIMATED: loud in every debug
                // run, a counted Busy in release.
                debug_assert!(
                    false,
                    "worst_case_fixed_bytes underestimated: the context is over its bound with every parked read shed"
                );
                self.shed.writes += 1;
                match self.queue.iter().position(|q| (q.client, q.write_id) == (pw.client, pw.write_id)) {
                    Some(i) => out.extend(self.end_queued(i, State::Failed { why: FailWhy::ContextFull })),
                    None => out.push(Effect::Notify { client: pw.client, write_id: pw.write_id, state: State::Failed { why: FailWhy::ContextFull } }),
                }
                out.extend(self.advance());
            } else {
                break;
            }
        }
        out
    }


    fn step_inner(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::Write {
                client,
                write_id,
                ops,
                reads,
                deferred,
            } => self.on_write(client, write_id, ops, reads, deferred),
            Event::PutConfirmed(id) => self.on_confirmed(id),
            Event::HeldUnknown(id) => self.on_held_unknown(id),
            Event::PutFailed(id) => self.on_failed(id),
            Event::PutRejected(id) => self.on_rejected(id),
            Event::HeadRefused { seq } => self.on_head_refused(seq),
            Event::HeadConfirmed(seq) => self.on_head(seq),
            Event::Tick(now) => self.on_tick(now),
            Event::Flush => self.on_flush(),
            Event::Get {
                client,
                req_id,
                key,
            } => self.read_or_wait(client, req_id, read::Want::Get(key)),
            Event::Scan {
                client,
                req_id,
                range,
            } => self.read_or_wait(
                client,
                req_id,
                read::Want::Scan(Box::new(range.as_ref().into())),
            ),
            Event::ScanAt {
                client,
                req_id,
                root,
                range,
            } => {
                let want = read::Want::Scan(Box::new(range.as_ref().into()));
                if self.recovered {
                    self.on_read_at(client, req_id, want, root)
                } else {
                    // Only a recovered engine has a root a reader could name.
                    self.read_or_wait(client, req_id, want)
                }
            }
            Event::ChangesSince {
                client,
                req_id,
                from,
                range,
                max_entries,
            } => self.read_or_wait(
                client,
                req_id,
                read::Want::Delta(Box::new(read::DeltaSpec {
                    from,
                    lo: range.lo,
                    hi: range.hi,
                    // Clamped here, where the engine's own appetite is
                    // decided. A caller asking for a million changes is
                    // asking the engine to hold a million changes.
                    max_entries: max_entries.clamp(1, self.params.max_delta_entries),
                })),
            ),
            Event::Preload { client, roots } => self.on_preload(client, roots),
            Event::SubscribeRange {
                client,
                sub_id,
                range,
            } => {
                let (accepted, evicted) = self.subs.add(
                    client,
                    sub_id,
                    range,
                    subs::SubLimits {
                        max_subs: self.params.max_subscriptions,
                        max_per_client: self.params.max_subscriptions_per_client,
                        max_key: self.params.max_sub_key,
                    },
                );
                // The oldest client's subscriptions are dropped UNTOLD, and on
                // purpose. A `Changed` names no session and is delivered to
                // whichever connection's call produced it — this one, the NEW
                // page's — so a `Stale` for the evicted ids would reach the
                // wrong tab, under sub ids it may hold itself (sdk#166: reads
                // and notifications are not yet sessioned). A victim that is
                // still live loses push and keeps its tick backstop — slower,
                // never wrong — and re-asserts its subscriptions on reconnect.
                // Almost always the victim is a page that reloaded away.
                let _ = evicted;
                vec![Effect::Subscribed {
                    client,
                    sub_id,
                    accepted,
                }]
            }
            Event::Unsubscribe { client, sub_id } => {
                self.subs.remove(client, sub_id);
                Vec::new()
            }
            Event::ClientGone { client } => {
                self.subs.drop_client(client);
                Vec::new()
            }
            Event::BlockArrived { id, bytes } => self.on_arrived(id, bytes, true),
            Event::BlockLanded { id, bytes } => self.on_arrived(id, bytes, false),
            Event::BlockMissed(id) => self.on_missed(id),
            Event::Start { key, epochs } => self.on_start(key, epochs),
            Event::HeadRead { epoch, seq, root } => self.on_head_read(epoch, seq, root),
            Event::HeadMissing => self.on_head_missing(),
            Event::CanSign(can) => {
                self.can_sign = can;
                // Turned ON: the held queue is cut now, as one commit.
                if can {
                    self.advance()
                } else {
                    Vec::new()
                }
            }
            Event::HeadConflict { seq, root } => self.on_head_conflict(seq, root),
            Event::AskWrite { client, write_id } => self.on_ask(client, write_id),
            Event::AuditWant { id, pass } => self.on_audit_want(id, pass),
            Event::AuditDrop { id, pass } => {
                self.drop_reader(id, Reader::Audit(pass));
                Vec::new()
            }
        }
    }

    /// A new engine value, holding only its device key.
    ///
    /// NOTHING IS REPLAYED. What the head does not name was never published:
    /// a pack's key is its content hash, so an engine that restarts with only
    /// its device key cannot enumerate packs its head does not name, and a
    /// separate journal would not remove the crash window — it would move it,
    /// at a PUT and a read-back on every commit. So the head is the journal,
    /// and recovery is reading it.
    fn on_start(&mut self, key: KeySource, epochs: Vec<Epoch>) -> Vec<Effect> {
        self.key = Some(key);
        self.epochs = epochs;
        self.recovered = false;
        // Newest epoch first: this engine's own head may still sit under the
        // previous one after an upgrade, and the first write after recovery
        // goes to the current one either way.
        match self.epochs.first().copied() {
            Some(epoch) => vec![Effect::ReadHead { epoch }],
            None => self.on_head_missing(),
        }
    }

    fn on_head_read(&mut self, epoch: Epoch, seq: u64, root: Cid) -> Vec<Effect> {
        self.head_epoch = Some(epoch);
        // WITH A COMMIT IN FLIGHT, a head read is that commit's question:
        // its own head is the confirmation, an older one says its head never
        // landed, and anything newer is another writer's. Adopting whatever
        // came back would move the tree out from under the commit.
        if let Some(c) = self.life.commit() {
            return if seq == c.seq && root == c.root {
                self.on_head(seq)
            } else if seq < c.seq {
                self.head_not_landed()
            } else {
                self.on_head_conflict(seq, root)
            };
        }
        let mut changed = self.adopt(seq, root);
        self.recovered = true;
        changed.extend(self.release_before_head());
        changed.extend(self.release_before_head_write());
        // The tree is not walked here. Reads warm it lazily (slice 2), which
        // is also what makes a restart cheap: the engine is usable the moment
        // it knows its root.
        //
        // This is the path a SECOND client learns on: its own engine never
        // wrote anything, it re-read the head, and the root moved.
        changed
    }

    /// No head under the newest epoch: try the one before it, and only when
    /// they are exhausted is this a device that has never written.
    fn on_head_missing(&mut self) -> Vec<Effect> {
        // A commit in flight whose head is not there: it never landed.
        if self.life.commit().is_some() {
            return self.head_not_landed();
        }
        if !self.epochs.is_empty() {
            self.epochs.remove(0);
        }
        if let Some(epoch) = self.epochs.first().copied() {
            return vec![Effect::ReadHead { epoch }];
        }
        // A device with no head starts from the empty tree. Its root is a
        // constant, not something to fetch.
        let mut changed = self.adopt(0, self.empty.cid);
        self.recovered = true;
        // A device that has never written: its tree IS empty, so the reads
        // that waited are answered from it — empty and complete, now truly.
        changed.extend(self.release_before_head());
        changed.extend(self.release_before_head_write());
        changed
    }

    /// Another engine holds this device key and is ahead.
    ///
    /// The Register refuses the lower or equal seq, so the loser here is the
    /// one that must give way: it takes the winner's head and rebases what it
    /// had accepted on top. It never publishes a fork — there is only ever one
    /// head, and the writes that were in flight are re-applied to the tree
    /// that won.
    fn on_head_conflict(&mut self, seq: u64, root: Cid) -> Vec<Effect> {
        // An OLDER head is not a conflict: this commit's head never landed —
        // unless it is the head this commit was BUILT ON shown under ANOTHER
        // root: that head was displaced (another device of the same identity
        // won the Register's tie-break, sdk#225), so the commit is dead.
        let displaced = seq == self.published_seq() && root != self.published_root();
        if self.life.commit().is_some_and(|c| seq < c.seq) && !displaced {
            return self.head_not_landed();
        }
        let witness = self.witness.take();
        if self.life.commit().is_none() {
            // Idle (COMMIT-LIFE Idle x E8): adopt, and the queue is re-derived on it.
            self.unpublished.clear();
            let mut out = self.adopt(seq, root);
            out.extend(self.rederive_after_foreign_move(&[], 0, witness, true));
            return out;
        }
        // A FOREIGN MOVE (R-b) ends the commit in flight: E8, one teardown.
        self.end_commit(End::Dead { witness, head: (seq, root) })
    }

    /// A client asking after a write this engine has never heard of.
    fn on_ask(&mut self, client: ClientId, write_id: WriteId) -> Vec<Effect> {
        // A PARKED WRITE ITS CLIENT ASKS AFTER: the next chain (sdk#174). Its
        // GETs start again from nothing and the apply runs again -- which
        // asks for its next round, or applies. The TICK does not also try:
        // one mechanism. (A parked write was not among the writes this engine
        // "knew", so asking after one was answered `Lost`.)
        if let Some(p) = self.parked_write.as_mut() {
            if p.client == client && p.write_id == write_id {
                p.gets = 0;
                // A new chain holds only what IT fetches: `held` stays within one chain's GETs (sdk#411).
                p.held.clear();
                p.idle_ticks = 0;
                if !self.wanted.write_needs().is_empty() {
                    // ITS ROUND IS STILL OUT -- and cannot be: the node runs
                    // no ask while this write's chain is live, so a block
                    // still needed when its client asks is an answer that
                    // chain LOST (sdk#173's event). Asked again HERE, in the
                    // writer's own chain: returning nothing left the write
                    // neither fetched nor released for as long as its client
                    // kept asking, and let another tab's chain fetch the
                    // block and carry the write's verdicts where the writer
                    // never hears them (F49; the architect's sdk#196 v4-bridge
                    // review, executed). Paced by the asks themselves: at most
                    // one unanswered per session, a second apart.
                    p.gets = self.wanted.write_needs().len() as u32;
                    let attempt = p.rounds;
                    return self
                        .wanted
                        .write_needs()
                        .iter()
                        .map(|id| Effect::FetchBlock {
                            id: *id,
                            via: read::Via::Direct,
                            attempt,
                        })
                        .collect();
                }
                return self.advance();
            }
        }
        // A WRITE THIS ENGINE DOES NOT KNOW IS NOT A LOST WRITE. It may be
        // one it published and forgot, whose `Published` was dropped or went
        // to another tab (F39, F49): `Lost` here rolled back a write that IS
        // in the tree (the architect's sdk#196 review, executed). `Lost` is
        // said only on positive evidence about THIS (client, write id) -- a
        // commit released lost -- and not knowing is not evidence: no verdict.
        // The client's own timeout decides what it does not hear about. (A
        // ledger in the head value, WRITE-PATH rev 3, is what will let an
        // engine answer "published" for a write it has forgotten.)
        let _ = client;
        Vec::new()
    }

    /// Take a root as the published one, and tell whoever was watching.
    ///
    /// The notification is INSIDE this rather than beside each caller. Every
    /// way the published root can move goes through here or through a commit
    /// landing, and a subscriber that heard about only some of them would
    /// miss exactly the changes another writer made — which is most of what a
    /// second tab is for. Making it structural means a fourth caller added
    /// later cannot forget.
    #[must_use = "a root move that tells nobody is a subscription that misses it"]
    fn adopt(&mut self, seq: u64, root: Cid) -> Vec<Effect> {
        let was = self.published_root;
        self.root = root;
        self.published_root = root;
        // A root taken from OUTSIDE — a head read, another writer's head —
        // was made by commits this engine did not code. The empty tree is
        // known in full (it lists none); a head carrying race put's §P mark
        // had every group it lists recoverable when it was signed
        // (COMMIT-LIFE §P); anything else is pre-§P, and NotScanned.
        let mark = self.head_mark.take();
        let marked = mark.is_some();
        if let Some(ids) = mark.filter(|ids| !ids.is_empty()) {
            self.root_parity.insert(root, ids);
        }
        self.parity_scan = if root == self.empty.cid || marked {
            ParityScan::Done { root }
        } else {
            ParityScan::NotScanned
        };
        self.published_seq = seq;
        self.next_seq = seq + 1;
        let mut out = self.supersede(was, root);
        self.keep_root_parity();
        out.extend(self.notify_subs(was));
        out
    }

    /// Forget the parity of roots nobody stands on: kept for the published
    /// root and every root a parked read is pinned to.
    fn keep_root_parity(&mut self) {
        let keep: BTreeSet<Cid> = std::iter::once(self.published_root).chain(self.reads.parked.values().map(|p| p.root)).collect();
        self.root_parity.retain(|r, _| keep.contains(r));
    }

    /// The group `id` is rebuilt from under `root`: its sibling group in a
    /// held node, or -- `id` BEING the root -- the root's group of one
    /// (sdk#335), when its parity is known.
    pub(crate) fn group_for(&self, root: Cid, id: Cid) -> Option<repair::Group> {
        repair::find_group(&self.source(), root, id)
            .or_else(|| (id == root).then(|| self.root_parity.get(&root)).flatten().map(|p| repair::root_group(root, p)))
    }

    /// Tell every subscriber whose range a root move touched.
    ///
    /// Called wherever `published_root` moves, with the root it moved FROM.
    /// There are exactly two such places — this engine's own commit landing,
    /// and a head read adopting someone else's root — and a subscriber that
    /// only hears about the first would miss every change another writer
    /// made, which is most of what a second tab is for.
    ///
    /// `a == b` reads nothing, so calling this on a move that moved nothing
    /// is free rather than merely harmless.
    fn notify_subs(&mut self, from: Cid) -> Vec<Effect> {
        if self.subs.is_empty() {
            return Vec::new();
        }
        let to = self.published_root;
        let seq = self.published_seq;
        let stale_after = self.params.stale_after_missing;
        if self.params.notify_without_diff {
            // The CONTROL. Every subscriber hears about every move, with no
            // comparison at all — which is what the range filter is supposed
            // to improve on, and a filter nothing can make fail is not a
            // filter. `Unknown` is the honest `why`: nothing was compared.
            if from == to {
                return Vec::new();
            }
            return self
                .subs
                .touched_without_diff(&from, &to)
                .into_iter()
                .map(|(client, sub_id, why)| Effect::Changed {
                    client,
                    sub_id,
                    new_root: to,
                    seq,
                    why,
                })
                .collect();
        }
        // Through `source()`, not `blocks`, and the shell test is what found
        // it. A brand-new device's root is the EMPTY LEAF, which is a constant
        // of the format that nothing on the network holds until the first
        // commit publishes it — so a diff from it against the node's store
        // alone stops immediately on a block that is not missing at all. Every
        // subscriber then got a spurious `BlockMissing` on the very first
        // commit, on any range, including ranges the commit came nowhere near.
        //
        // The fields are disjoint, so this is a borrow split rather than a
        // clone: `source()` reads `blocks` and `empty`, `touched` mutates
        // `subs`.
        let Engine {
            subs,
            blocks,
            empty,
            ..
        } = self;
        let source = WithEmptyLeaf {
            inner: blocks,
            empty_cid: empty.cid,
            empty_bytes: &empty.bytes,
            arrived: &BTreeMap::new(),
        };
        subs.touched(&source, &from, &to, stale_after)
            .into_iter()
            .map(|(client, sub_id, why)| Effect::Changed {
                client,
                sub_id,
                new_root: to,
                seq,
                why,
            })
            .collect()
    }

    /// What a read that has GIVEN UP answers.
    ///
    /// One place, because there are four ways a read ends without an answer
    /// and a delta must come out of all of them saying the same thing. Its
    /// degraded answer is `FullReloadRequired` — a defined outcome the client
    /// acts on by doing a plain range read — rather than `Unavailable`, which
    /// is a failure the client would have to invent a recovery for. The old
    /// root's blocks being gone is ORDINARY for a reader that was away, so
    /// the path that handles it has to be the ordinary one.
    fn gave_up(&self, want: &read::Want, blocked: Cid) -> read::ReadResult {
        match want {
            read::Want::Delta(_) => read::ReadResult::FullReloadRequired {
                new_root: self.published_root,
            },
            _ => read::ReadResult::Unavailable(blocked),
        }
    }

    /// Try a read, reply if it is answerable now, park it if it is not.
    ///
    /// Rule 1: a warm hit replies in the same `step`, with no other effect. A
    /// read that is already answerable must not cost a round trip, because
    /// most reads in a live app are answerable.
    /// A read before the head is recovered WAITS (sdk#223): the only tree an
    /// unrecovered engine has is the empty one, and an answer from it is a
    /// false "complete, nothing here". Bounded like parked reads: past
    /// `max_parked_reads` a read is answered as one that could not be served
    /// (`gave_up`), never as empty.
    fn read_or_wait(&mut self, client: ClientId, req_id: read::ReqId, want: read::Want) -> Vec<Effect> {
        if self.recovered {
            return self.on_read(client, req_id, want);
        }
        if self.before_head.len() >= self.params.max_parked_reads {
            let result = self.gave_up(&want, self.published_root);
            return vec![Effect::Reply { client, req_id, result }];
        }
        self.before_head.push((client, req_id, want));
        Vec::new()
    }

    /// The head is recovered (or another head adopted with no commit in
    /// flight): the queue is re-derived on the root now published -- the
    /// writes that waited for recovery apply onto it, in arrival order.
    fn release_before_head_write(&mut self) -> Vec<Effect> {
        let witness = self.witness.take();
        self.rederive_after_foreign_move(&[], 0, witness, true)
    }

    /// The head is recovered: answer every read that waited for it, in the
    /// order they arrived.
    fn release_before_head(&mut self) -> Vec<Effect> {
        let waiting = std::mem::take(&mut self.before_head);
        let mut out = Vec::new();
        for (client, req_id, want) in waiting {
            out.extend(self.on_read(client, req_id, want));
        }
        out
    }

    fn on_read(&mut self, client: ClientId, req_id: read::ReqId, want: read::Want) -> Vec<Effect> {
        let root = self.published_root;
        self.on_read_at(client, req_id, want, root)
    }

    /// A read at `root`: the published head for a protocol read, the root a
    /// reader's own walk stopped at for [`Event::ScanAt`].
    fn on_read_at(&mut self, client: ClientId, req_id: read::ReqId, want: read::Want, root: Cid) -> Vec<Effect> {
        self.reads.parked.insert(
            req_id,
            read::Parked {
                client,
                want,
                root,
                frontier: None,
                delta: read::DeltaProgress::default(),
                levels_done: 0,
                rounds: 0,
                held: BTreeSet::from([root]),
                gets: 0,
                arrived: 0,
            },
        );
        let out = self.drive(req_id);
        // The cap is checked AFTER the attempt, not before it. A read the
        // node can already answer is parked and unparked within this call and
        // costs the context nothing, so refusing it at the door would refuse
        // the cheap case to protect a budget it never touches. What is
        // bounded is the reads still WAITING when the call ends.
        if self.reads.parked.len() > self.params.max_parked_reads
            && self.reads.parked.contains_key(&req_id)
        {
            let blocked = out
                .iter()
                .find_map(|f| match f {
                    Effect::FetchBlock { id, .. } => Some(*id),
                    _ => None,
                })
                .unwrap_or(root);
            let result = self
                .reads
                .parked
                .remove(&req_id)
                .map(|p| self.gave_up(&p.want, blocked))
                .unwrap_or(read::ReadResult::Unavailable(blocked));
            self.forget_waiting(req_id);
            // A reply, not silence. The caller can retry when the burst
            // clears; a read that is simply dropped leaves it waiting for an
            // answer that is never coming.
            return vec![Effect::Reply {
                client,
                req_id,
                result,
            }];
        }
        out
    }

    /// Advance one parked read as far as what is warm allows.
    fn drive(&mut self, req_id: read::ReqId) -> Vec<Effect> {
        let Some(p) = self.reads.parked.get(&req_id).cloned() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        // The counter is taken OUT for the call: `source()` borrows the
        // engine, and a cost counter is not worth an interior-mutability cell.
        let mut parsed = self.nodes_parsed;
        // Continue from the deepest node this read reached, not from the root
        // (sdk#34). A read that starts over needs every upper level again, so
        // a node evicting what it serves can make it impossible rather than
        // merely slow.
        let start = p.frontier.unwrap_or(p.root);
        let outcome = match &p.want {
            // A DELTA resumes after the prefix it has established and KEEPS every prefix a page gives it (sdk#135): a
            // node that evicts what it serves would otherwise make it re-fetch its whole working set for ever.
            read::Want::Delta(spec) => {
                let room = spec.max_entries.saturating_sub(p.delta.acc.len());
                let step = if room == 0 {
                    // The page limit, reached by prefixes: answered with a cursor, as a page limit always is.
                    read::DeltaStep::Done { changes: Vec::new(), cursor: p.delta.after.clone() }
                } else {
                    let source = self.source();
                    read::delta_step(&source, &self.params, spec, &p.root, p.delta.after.as_deref(), room)
                };
                let q = self.reads.parked.get_mut(&req_id).expect("parked");
                match step {
                    read::DeltaStep::Done { changes, cursor } => {
                        let mut all = std::mem::take(&mut q.delta.acc);
                        all.extend(changes);
                        read::Attempt::Done(read::ReadResult::Delta { changes: all, cursor, new_root: p.root })
                    }
                    read::DeltaStep::Prefix { changes, after, need } => {
                        q.delta.acc.extend(changes);
                        // Never backwards (I2): a page's `next` is where it has decided up to.
                        if after.is_some() && after != q.delta.after {
                            q.delta.after = after;
                            // A new resume position: what arrived for the old one no longer tells a re-fetch.
                            q.delta.seen.clear();
                        }
                        read::Attempt::Need(need)
                    }
                    read::DeltaStep::Need(ids) => read::Attempt::Need(ids),
                    read::DeltaStep::Broken(cid) => read::Attempt::Broken(cid),
                    read::DeltaStep::Reload => read::Attempt::Done(read::ReadResult::FullReloadRequired { new_root: p.root }),
                }
            }
            _ => {
                let source = self.source();
                read::attempt(&source, &self.params, &p.want, &start, &mut parsed)
            }
        };
        self.nodes_parsed = parsed;
        // THE DELTA'S STALL RULE (sdk#135; `read::DeltaProgress`): at the end of a fetch cycle -- every block it
        // asked has ARRIVED, none still waited on -- a cycle that moved nothing and asked only blocks that had
        // already arrived at this resume position is a livelock of answered re-fetches. Two in a row: the full read.
        if matches!(p.want, read::Want::Delta(_)) && matches!(outcome, read::Attempt::Need(_) | read::Attempt::NeedFrom { .. }) {
            let waiting = !self.wanted.waited_by(req_id).is_empty();
            if !waiting {
                let q = self.reads.parked.get_mut(&req_id).expect("parked");
                let marker = (q.delta.after.clone(), q.delta.acc.len());
                if let Some(began) = q.delta.cycle_start.take() {
                    if began == marker && !q.delta.cycle_new {
                        q.delta.stalled += 1;
                    } else {
                        q.delta.stalled = 0;
                    }
                }
                if q.delta.stalled >= self.params.delta_stall_cycles {
                    self.reads.parked.remove(&req_id);
                    self.forget_waiting(req_id);
                    return vec![Effect::Reply {
                        client: p.client,
                        req_id,
                        result: read::ReadResult::FullReloadRequired { new_root: p.root },
                    }];
                }
                q.delta.cycle_start = Some(marker);
                q.delta.cycle_new = false;
            }
        }
        // Only a missing CHILD is a resume point; see `Attempt::NeedFrom`.
        // Read from a borrow, so the two Need shapes can share one arm below
        // rather than needing a branch that cannot happen.
        let advance_to = match &outcome {
            read::Attempt::NeedFrom { node, .. } => Some(*node),
            _ => None,
        };
        match outcome {
            read::Attempt::Done(result) => {
                self.reads.parked.remove(&req_id);
                self.forget_waiting(req_id);
                out.push(Effect::Reply {
                    client: p.client,
                    req_id,
                    result,
                });
            }
            read::Attempt::Broken(cid) => {
                // The tree names a block whose content is not that block. No
                // number of fetches fixes it, and answering "absent" would be
                // a wrong answer rather than a missing one.
                let result = self.gave_up(&p.want, cid);
                self.reads.parked.remove(&req_id);
                self.forget_waiting(req_id);
                out.push(Effect::Reply {
                    client: p.client,
                    req_id,
                    result,
                });
            }
            read::Attempt::Need(ids) | read::Attempt::NeedFrom { ids, .. } => {
                // A round that asks for something is a round. There is NO cap
                // on them (the owner's rule 7: retry until answered): a read
                // ends when its blocks arrive, never because it has taken
                // many rounds. A slow node is shown as "not answering for N
                // s" by the page's sender, not turned into `Unavailable`.
                {
                    let q = self.reads.parked.get_mut(&req_id).expect("parked");
                    q.levels_done += 1;
                    q.rounds += 1;
                    if let Some(node) = advance_to {
                        q.frontier = Some(node);
                    }
                }
                // THIS REQUEST'S GETS ARE SPENT (sdk#174): answer what has been
                // reached, with a cursor, and let the next page be a new
                // request -- the node's exclusion is released between them.
                // A read that has reached nothing yet has no key to put a
                // cursor after; it continues, bounded by the tree's depth.
                let used = self.reads.parked.get(&req_id).map_or(0, |q| q.gets);
                let room = self.params.max_gets_per_request.saturating_sub(used) as usize;
                if room == 0 {
                    let short = {
                        let source = self.source();
                        read::short_page(&source, &p.want, &p.root)
                    };
                    if let Some(result) = short {
                        self.reads.parked.remove(&req_id);
                        self.forget_waiting(req_id);
                        return vec![Effect::Reply {
                            client: p.client,
                            req_id,
                            result,
                        }];
                    }
                }
                let levels_done = self.reads.parked.get(&req_id).map_or(0, |q| q.levels_done);
                out.push(Effect::Progress {
                    client: p.client,
                    req_id,
                    levels_done,
                    // Not known until the walk ends; one more than what is
                    // done is the honest floor rather than a guessed height.
                    levels_total: levels_done + 1,
                });
                let mut ids = ids;
                // Never ask for what the node already holds. `Page::need` can
                // name blocks an earlier round brought in, and a reader that
                // re-fetches them makes no progress at all.
                //
                // `refetch_held` is the control for the cost bound: with it on
                // the engine asks again for everything, which is a plausible
                // implementation and blows the bound many times over. The old
                // control enumerated the whole warm set, which a core that
                // owns no blocks cannot do.
                if !self.params.refetch_held {
                    ids.retain(|id| self.blocks.get(id).is_none());
                }
                // A round asks for no more than one return carries, and no more
                // than the request has left -- so the cap is met exactly.
                let cap = if room == 0 {
                    self.params.max_fetch_per_round
                } else {
                    self.params.max_fetch_per_round.min(room)
                };
                if self.params.fetch_ahead {
                    // Everything the readable branches name, whether or not
                    // this descent needs it.
                    let mut ahead: Vec<Cid> = Vec::new();
                    let mut stack = vec![p.root];
                    let mut seen: BTreeSet<Cid> = BTreeSet::new();
                    while let Some(cid) = stack.pop() {
                        if !seen.insert(cid) || ahead.len() > 128 {
                            continue;
                        }
                        let Some(bytes) = self.blocks.get(&cid) else {
                            continue;
                        };
                        let Ok(n) = Node::parse(bytes) else { continue };
                        if !n.is_leaf() {
                            for i in 0..n.len() {
                                let c = n.child(i).0;
                                ahead.push(c);
                                stack.push(c);
                            }
                        }
                    }
                    ids.extend(ahead);
                    // The ahead ids go through the same already-held filter as
                    // the rest: without it the control re-asks for what just
                    // arrived and never settles, which is a hang rather than a
                    // cost and measures nothing.
                    if !self.params.refetch_held {
                        ids.retain(|id| self.blocks.get(id).is_none());
                    }
                }
                for id in ids.into_iter().take(cap) {
                    // The waiter is ALWAYS recorded — that is what wakes the
                    // read when the block lands, and skipping it would make
                    // the control a hang rather than a cost. What the control
                    // turns off is only the SUPPRESSION: with dedupe off the
                    // fetch is emitted again every time the engine re-descends
                    // and finds the same block missing.
                    // A read is the FIRST to want it when no other READ does (a repair's in-flight slot is joined
                    // below as `raced`; the parked write's is joined as a read's own ask, unchanged).
                    let first = !self.want(id, Reader::Read(req_id)).reads || !self.params.share_fetches;
                    // A DELTA asking a block that has not arrived at this resume position is descending (progress);
                    // one it had and lost is a re-fetch (sdk#135's stall rule).
                    if let Some(q) = self.reads.parked.get_mut(&req_id) {
                        if matches!(q.want, read::Want::Delta(_)) && !q.delta.seen.contains(&id) {
                            q.delta.cycle_new = true;
                        }
                    }
                    // A race for a sibling asked this block already (sdk#303): that GET answers this read too.
                    let raced = self.params.dedupe_in_flight && self.wanted.is_slot(&id);
                    if raced {
                        out.extend(self.race(id, p.root));
                    } else if first || !self.params.dedupe_in_flight {
                        let attempt = *self.reads.attempts.entry(id).or_insert(0);
                        let via = self
                            .reads
                            .in_pack
                            .get(&id)
                            .copied()
                            .map_or(read::Via::Direct, read::Via::Pack);
                        self.reads.fetches += 1;
                        if let Some(q) = self.reads.parked.get_mut(&req_id) {
                            q.gets += 1;
                        }
                        self.step_asks.insert(id, true);
                        out.push(Effect::FetchBlock { id, via, attempt });
                        // RACE (rule 11, sdk#303): the rest of this block's group, at once.
                        out.extend(self.race(id, p.root));
                    }
                }
            }
        }
        out
    }

    /// SUPERSEDE a parked read (#330 ruling) — the caller has seen a head
    /// move past its root — ONLY IF it has made no progress: no block has
    /// arrived for it since it started, and it waits on at least one (a GET
    /// unanswered or answered NotFound). Then it is FORGOTTEN, never
    /// answered, and each block only it waited on is WITHDRAWN (its group's
    /// repair ended with it), so the page ends that GET instead of re-asking
    /// it for ever. `false`: it is not parked, or it is making progress — it
    /// finishes on its own tree (READ-STATE inv. 2). An answer-driven end:
    /// the head moved; no clock is read (rule 8).
    pub fn supersede_read(&mut self, req_id: read::ReqId) -> Option<Vec<Effect>> {
        let p = self.reads.parked.get(&req_id)?;
        let blocks: Vec<Cid> = self.wanted.waited_by(req_id);
        if p.arrived > 0 || blocks.is_empty() {
            return None;
        }
        self.reads.parked.remove(&req_id);
        self.forget_waiting(req_id);
        for id in blocks {
            // Still wanted by another read, a repair, or the parked write: its fetch -- and its repair -- stand.
            if self.readers_of(&id).any() {
                continue;
            }
            if self.repairs.contains_key(&id) {
                self.end_repair(id);
            }
        }
        // Its own call's end: the withdrawal, net (WANTED-LIFE; a call outside `step`, so it settles here).
        Some(self.settle_unwanted())
    }

    fn forget_waiting(&mut self, req_id: read::ReqId) {
        let waited: Vec<Cid> = self.wanted.waited_by(req_id);
        for id in waited {
            self.drop_reader(id, Reader::Read(req_id));
        }
        // A block nobody waits on any more has no attempts to count. Left
        // behind, one entry per block asked for and never answered outlived
        // the read that asked -- shed, given up or capped -- in no cap and in
        // no sum, and a context with ZERO parked reads went over its bound
        // (sdk#187 review, executed).
        let wanted = &self.wanted;
        self.reads.attempts.retain(|id, _| wanted.read_waits_on(id));
    }

    fn on_write(
        &mut self,
        client: ClientId,
        write_id: WriteId,
        ops: Vec<(Vec<u8>, Op)>,
        reads: Vec<(Vec<u8>, Expect)>,
        deferred: bool,
    ) -> Vec<Effect> {
        let tries = self.next_tries.take().unwrap_or(0);
        // AT THE DOOR (sdk#235, W8): a write names what it read. SYNTACTIC —
        // the write's own op keys against its own read keys — so it reads no
        // state and comes before every other answer: before Busy, before the
        // pre-head park (which answers Accepted at once), before the order
        // rule and the size. Nothing is consumed, moved, parked or applied.
        if let Some(key) = unread_key(&ops, &reads) {
            return vec![
                Effect::Notify { client, write_id, state: State::Unread },
                Effect::Unread { client, write_id, key },
            ];
        }
        let size: usize = ops
            .iter()
            .map(|(k, o)| k.len() + if let Op::Put(v) = o { v.len() } else { 0 })
            .sum::<usize>()
            + reads.iter().map(|(k, _)| k.len() + 33).sum::<usize>();
        // One write against its own bound: over it every time, `TooLarge`.
        if size > self.params.max_write_bytes {
            return vec![Effect::Notify {
                client,
                write_id,
                state: State::TooLarge {
                    bound: WriteBound::WriteBytes,
                    limit: self.params.max_write_bytes,
                    got: size,
                },
            }];
        }
        // THE PAGE'S QUEUE (R-b; COMMIT-LIFE K1, K9). A write arriving while a
        // commit is in flight is TAKEN: it joins the warm root now and rides
        // the next cut. NO COUNT CAP: the one bound is the BYTES held, and past
        // it this says `QueueFull` so the SDK WAITS for room (backpressure,
        // K1) -- a refusal only at the app's own deadline. A session with
        // nothing queued is always taken (its fair share: one tab's burst
        // cannot starve another's write).
        //
        // ORDER WITHIN A SESSION (review §2 on sdk#295): `QueueFull` is not
        // terminal -- the refused write comes again. So once a session is told
        // it, every later write of that session is told it too, even one that
        // would fit, until the session has nothing queued: a smaller later
        // write never lands before the older one it followed. With nothing
        // queued the session is taken (its fair share) and unblocked.
        let bytes = self.queued_bytes();
        let has_one = self.queue.iter().any(|q| q.client == client);
        if has_one && (self.queue_blocked.contains(&client) || bytes + size > self.params.max_queue_bytes) {
            self.queue_blocked.insert(client);
            return vec![Effect::Notify { client, write_id, state: State::QueueFull { bytes, limit: self.params.max_queue_bytes } }];
        }
        self.queue_blocked.remove(&client);
        self.queue.push_back(Queued {
            client,
            write_id,
            ops,
            reads,
            size,
            tries,
            told_accepted: false,
            warm_after: None,
            stalled_told: false,
            arrival: self.next_arrival,
            deferred,
        });
        self.next_arrival += 1;
        let mut out = Vec::new();
        // BEFORE THE HEAD IS RECOVERED the tree is the empty one: a write
        // applied now would commit a head blind to the real one. It waits,
        // APPLYING, and is ANSWERED, not silently held: `Accepted` is what it
        // is -- the engine has the write and nothing is on the network yet.
        if !self.recovered {
            self.queue.back_mut().expect("just pushed").told_accepted = true;
            out.push(Effect::Notify { client, write_id, state: State::Accepted });
        }
        out.extend(self.advance());
        out
    }

    /// Move the queue as far as it can go now (R-b): apply the FIRST
    /// `Applying` write onto the warm root (arrival order is apply order,
    /// footnote 7), and commit the front when the commit lane is idle. Every
    /// exit of the first `Applying` write tries the next (footnote 8).
    fn advance(&mut self) -> Vec<Effect> {
        let mut out = Vec::new();
        // Held (a merge's group is about to go at the front): NOTHING is
        // judged -- a later write re-judged on the winner now would be judged
        // without the displaced writes it came after.
        if !self.recovered || self.cut_held {
            return out;
        }
        loop {
            let mut moved = false;
            if let Some(i) = self.queue.iter().position(|q| q.warm_after.is_none()) {
                let q = &self.queue[i];
                let fetching = self
                    .parked_write
                    .as_ref()
                    .is_some_and(|p| (p.client, p.write_id) == (q.client, q.write_id)) && !self.wanted.write_needs().is_empty();
                if !fetching {
                    let (fx, applied_or_gone) = self.warm_apply(i);
                    out.extend(fx);
                    moved |= applied_or_gone;
                }
            }
            // THE GATE: no commit is cut while this page cannot sign, nor while
            // nothing but DEFERRED writes is ready (sdk#350: they commit only
            // in company); the writes keep applying to the warm root above.
            if self.can_sign && self.in_company() {
                let (fx, popped) = self.commit_front();
                out.extend(fx);
                moved |= popped;
            }
            if !moved {
                break;
            }
        }
        out
    }

    /// The keys a write writes: what a later write's conflict may name it for.
    fn written_keys(ops: &[(Vec<u8>, Op)]) -> BTreeSet<Vec<u8>> {
        ops.iter().map(|(k, _)| k.clone()).collect()
    }

    /// Take queue entry `i` out as terminal, telling its client `state`.
    /// Its keys join the cascade: a later write that read one names it.
    fn end_queued(&mut self, i: usize, state: State) -> Vec<Effect> {
        let q = self.queue.remove(i).expect("a queued write");
        if self.parked_write.as_ref().is_some_and(|p| (p.client, p.write_id) == (q.client, q.write_id)) {
            self.release_write();
        }
        self.cascade.push((q.client, q.write_id, Self::written_keys(&q.ops)));
        vec![Effect::Notify { client: q.client, write_id: q.write_id, state }]
    }

    /// Apply the first `Applying` write (entry `i`; every entry before it is
    /// in the warm root) onto the warm root. `true` when it moved: applied,
    /// or ended. `false` when it parked on its path's blocks.
    ///
    /// The apply is the SAME pure function the commit runs (`apply_with`), so
    /// the root it makes here is the root its commit makes on the published
    /// root when its turn comes, with no foreign move between (K9). Its
    /// blocks are KEPT by the page for reads, never put: the commit puts the
    /// same bytes.
    fn warm_apply(&mut self, i: usize) -> (Vec<Effect>, bool) {
        let q = self.queue[i].clone();
        match self.check_reads(&q.reads, &q.ops) {
            Ok(()) => {}
            Err(ReadsCheck::Need(need)) => return self.park_applying(i, need),
            Err(ReadsCheck::Differs { key, current }) => {
                // The LATEST earlier write of that key that ended: the one
                // whose value this write read.
                let after = self
                    .cascade
                    .iter()
                    .rev()
                    .find(|(_, _, keys)| keys.contains(&key))
                    .map(|(c, w, _)| (*c, *w));
                let mut out = self.end_queued(i, State::Conflict);
                out.push(Effect::Conflicted { client: q.client, write_id: q.write_id, key, current, after, tries: q.tries });
                return (out, true);
            }
            Err(ReadsCheck::Unreadable) => return (self.end_queued(i, State::Failed { why: FailWhy::TreeDamaged }), true),
        }
        let batch = batch_of(&q.ops);
        let mut emitted: Vec<(Cid, Vec<u8>)> = Vec::new();
        let applied = match apply_with(ApplyOptions::default(), &self.source(), &self.root, &batch, |c, b: &[u8]| {
            emitted.push((c, b.to_vec()))
        }) {
            Ok(a) => a,
            Err(ApplyError::Read(ReadError::Need(need))) => return self.park_applying(i, need),
            Err(e) => return (self.end_queued(i, State::Failed { why: apply_failed(&e) }), true),
        };
        if emitted.len() > self.params.max_commit_blocks {
            let got = emitted.len();
            let state = State::TooLarge { bound: WriteBound::CommitBlocks, limit: self.params.max_commit_blocks, got };
            return (self.end_queued(i, state), true);
        }
        if self.parked_write.as_ref().is_some_and(|p| (p.client, p.write_id) == (q.client, q.write_id)) {
            self.release_write();
        }
        self.root = applied.root;
        let e = &mut self.queue[i];
        e.warm_after = Some(applied.root);
        // Readable for the REST OF THIS STEP too: the next write in the queue
        // may apply onto this one's root before the page has kept a byte, and
        // asking the network for a block that exists only here would ask for
        // something nobody has put (`arrived` is cleared when the step ends).
        for (id, bytes) in &emitted {
            self.arrived.insert(*id, bytes.clone());
            if !self.kept.contains_key(id) {
                self.kept.insert(*id, bytes.len());
                self.kept_bytes += bytes.len();
            }
        }
        let mut out: Vec<Effect> = emitted.into_iter().map(|(id, bytes)| Effect::Keep { id, bytes }).collect();
        if !e.told_accepted {
            e.told_accepted = true;
            out.push(Effect::Notify { client: q.client, write_id: q.write_id, state: State::Accepted });
        }
        (out, true)
    }

    /// The first `Applying` write's path is not held: fetch it (the parked
    /// write's chain, GET budget and round bound). `park_write` ending it --
    /// rounds spent -- is an exit, and the next is tried.
    fn park_applying(&mut self, i: usize, need: Vec<Cid>) -> (Vec<Effect>, bool) {
        let q = self.queue[i].clone();
        let keys = Self::written_keys(&q.ops);
        let out = self.park_write(q.client, q.write_id, need);
        if self.parked_write.as_ref().is_some_and(|p| (p.client, p.write_id) == (q.client, q.write_id)) {
            return (out, false);
        }
        // Ended by `park_write`, which told it: out of the queue, untold again.
        self.queue.remove(i);
        self.cascade.push((q.client, q.write_id, keys));
        (out, true)
    }

    /// The parked write `(client, write_id)` may have been ended by
    /// `park_write` (its rounds spent): if so it leaves the queue and the
    /// next `Applying` write is tried (footnote 8).
    fn after_park(&mut self, client: ClientId, write_id: WriteId) -> Vec<Effect> {
        if self.parked_write.as_ref().is_some_and(|p| (p.client, p.write_id) == (client, write_id)) {
            return Vec::new();
        }
        if let Some(i) = self.queue.iter().position(|q| (q.client, q.write_id) == (client, write_id)) {
            let q = self.queue.remove(i).expect("found");
            self.cascade.push((q.client, q.write_id, Self::written_keys(&q.ops)));
        }
        self.advance()
    }

    /// COMMIT THE CUT when the lane is idle (R-b; COMMIT-LIFE K9): GROUP
    /// COMMIT. The cut is an arrival-order prefix of the queue that stops at
    /// the first `Applying` write, and -- while one cut is being shipped in
    /// pieces, or re-sent after `Lost` -- at that cut's last write (§1, §4,
    /// §6). The group is REBUILT in order on the just-published root, each
    /// write re-judged by `check_reads` where it lands (§5; `uncoded` decided
    /// here); a write that no longer holds drops out, named, and its
    /// dependants cascade. Invariant 4 per commit (§3): the committed root is
    /// replay(base, the survivors) -- with nothing dropped, the warm root at
    /// the cut (a difference is counted, `rebuild_differs`, and the rest
    /// re-derived). Split only at write boundaries (§4). `true` when writes
    /// ended without a commit (a no-op group is `Published` at once, #164),
    /// so the next may go.
    fn commit_front(&mut self) -> (Vec<Effect>, bool) {
        if self.life.commit().is_some() || self.cut_held {
            return (Vec::new(), false);
        }
        // A dead cut's writes can leave the queue while being re-applied (a
        // conflict, a failed fetch): with none of them left the cut is over,
        // or its bound would hold every later write back for ever.
        self.close_cut();
        let until = self.cut_until;
        let cut: Vec<(ClientId, WriteId)> = self
            .queue
            .iter()
            .take_while(|q| q.warm_after.is_some() && until.is_none_or(|u| q.arrival <= u))
            .map(|q| (q.client, q.write_id))
            .collect();
        if cut.is_empty() {
            return (Vec::new(), false);
        }
        if self.cut_until.is_none() {
            self.cut_until = self.queue.get(cut.len() - 1).map(|q| q.arrival);
        }
        let tip = self.root;
        let base = self.published_root;
        self.root = base;
        let mut out = Vec::new();
        let mut taken = 0usize;
        let mut dropped = false;
        let mut stopped_cold: Option<Vec<Cid>> = None;
        let mut blocks = 0usize;
        let mut last_warm = None;
        let mut through = 0u64;
        let mut cut_parity: Vec<(Cid, Vec<u8>)> = Vec::new();
        // The cut's writes as they are taken, and their bytes: handed to `start_commit` (the old `folded` field held
        // them for the length of this call only, COMMIT-LIFE A7).
        let mut writes: Vec<(ClientId, WriteId)> = Vec::new();
        let mut bytes = 0usize;
        for id in cut {
            // Its index now: every earlier write of the cut is either taken
            // (in front) or dropped (gone).
            let idx = taken;
            let q = self.queue[idx].clone();
            debug_assert_eq!((q.client, q.write_id), id);
            match self.check_reads(&q.reads, &q.ops) {
                Ok(()) => {}
                Err(ReadsCheck::Need(need)) => {
                    stopped_cold = Some(need);
                    break;
                }
                Err(ReadsCheck::Differs { key, current }) => {
                    let after = self.cascade.iter().rev().find(|(_, _, keys)| keys.contains(&key)).map(|(c, w, _)| (*c, *w));
                    out.extend(self.end_queued(idx, State::Conflict));
                    out.push(Effect::Conflicted { client: q.client, write_id: q.write_id, key, current, after, tries: q.tries });
                    dropped = true;
                    continue;
                }
                Err(ReadsCheck::Unreadable) => {
                    out.extend(self.end_queued(idx, State::Failed { why: FailWhy::TreeDamaged }));
                    dropped = true;
                    continue;
                }
            }
            let batch = batch_of(&q.ops);
            let mut emitted: Vec<(Cid, Vec<u8>)> = Vec::new();
            let applied = match apply_with(ApplyOptions::default(), &self.source(), &self.root, &batch, |c, b: &[u8]| {
                emitted.push((c, b.to_vec()))
            }) {
                Ok(a) => a,
                Err(ApplyError::Read(ReadError::Need(need))) => {
                    stopped_cold = Some(need);
                    break;
                }
                Err(e) => {
                    out.extend(self.end_queued(idx, State::Failed { why: apply_failed(&e) }));
                    dropped = true;
                    continue;
                }
            };
            // THE COMMIT'S LIMIT, at a write boundary (§4): this write goes in
            // the next piece, built on this one.
            if taken > 0 && blocks + emitted.len() > self.params.max_commit_blocks {
                break;
            }
            self.root = applied.root;
            // THIS WRITE'S PARITY, coded by the apply: it goes out with the
            // commit (§P). A group a later write of the cut recodes is
            // dropped below, by what the final tree lists.
            cut_parity.extend(applied.parity.iter().cloned());
            blocks += emitted.len();
            self.unpublished.extend(emitted.iter().cloned());
            for (id, bytes) in &emitted {
                self.arrived.insert(*id, bytes.clone());
            }
            if q.reads.iter().any(|(_, e)| *e == Expect::Any) {
                self.forced_writes += 1;
            }
            writes.push((q.client, q.write_id));
            bytes += q.size;
            through = through.max(q.arrival);
            last_warm = q.warm_after;
            taken += 1;
        }
        // A write whose path on the published root was evicted: it and every
        // write behind it go back to `Applying` (arrival order), fetching.
        let cold = |e: &mut Self, need: Vec<Cid>| -> Vec<Effect> {
            e.requeue_from(taken);
            let q = e.queue[taken].clone();
            e.park_write(q.client, q.write_id, need)
        };
        if taken == 0 {
            self.root = base;
            if let Some(need) = stopped_cold {
                out.extend(cold(self, need));
                return (out, false);
            }
            // Every write of the cut dropped: the rest re-derived.
            self.requeue_from(0);
            return (out, true);
        }
        // A NO-OP GROUP (#164): the tree it asks for IS the published one.
        if self.root == base && self.unpublished.is_empty() {
            self.noop_published += writes.len() as u64;
            // Nothing new was put: the tree it asks for is the published one,
            // so SAVED at once. BACKED_UP only when that tree IS: while a
            // published commit's stragglers are still out (race put, §P) the
            // no-op joins every pending Backing and hears ParityComplete when
            // they drain -- the architect's #164 finding, not re-opened by
            // race put.
            for w in &writes {
                out.push(Effect::Notify { client: w.0, write_id: w.1, state: State::Published });
                if !self.life.join_open(*w) {
                    out.push(Effect::Notify { client: w.0, write_id: w.1, state: State::ParityComplete });
                }
            }
            for _ in 0..taken {
                let q = self.queue.pop_front().expect("taken");
                self.cascade.push((q.client, q.write_id, Self::written_keys(&q.ops)));
            }
            self.close_cut();
            if dropped || stopped_cold.is_some() {
                self.requeue_from(0);
                if let Some(need) = stopped_cold {
                    let q = self.queue[0].clone();
                    out.extend(self.park_write(q.client, q.write_id, need));
                }
            } else {
                self.root = tip;
            }
            return (out, true);
        }
        let committed = self.root;
        let to_ship = self.take_unpublished();
        // Only the parity the FINAL tree lists: a group a later write of the
        // cut recoded superseded the earlier coding.
        let listed: BTreeSet<Cid> = to_ship.iter().filter_map(|(_, b)| Node::parse(b).ok()).flat_map(|n| n.parity().collect::<Vec<_>>()).collect();
        let mut seen_parity = BTreeSet::new();
        let parity: Vec<(Cid, Vec<u8>)> = cut_parity.into_iter().filter(|(c, _)| listed.contains(c) && seen_parity.insert(*c)).collect();
        out.extend(self.start_commit(to_ship, parity, writes, bytes));
        if let Some(c) = self.life.commit_mut() {
            c.through = through;
        }
        if !dropped && stopped_cold.is_none() {
            if last_warm == Some(committed) {
                // Invariant 4: the committed root IS the warm root the cut's
                // last write made; the warm tip stands.
                self.root = tip;
            } else {
                self.rebuild_differs += 1;
                self.root = committed;
                self.requeue_from(taken);
                out.extend(self.advance());
            }
        } else {
            // A drop, or a cold path: the warm root is re-derived on the
            // committed root, without what dropped (§5).
            self.root = committed;
            match stopped_cold {
                Some(need) => out.extend(cold(self, need)),
                None => {
                    self.requeue_from(taken);
                    out.extend(self.advance());
                }
            }
        }
        (out, false)
    }

    /// The cut is shipped when no write of it is left in the queue.
    fn close_cut(&mut self) {
        if let Some(u) = self.cut_until {
            if !self.queue.iter().any(|q| q.arrival <= u) {
                self.cut_until = None;
            }
        }
    }

    /// Entries `from..` go back to `Applying`, re-judged in order when
    /// `advance` re-applies them onto the warm root as it is now (`self.root`,
    /// set by the caller to the tip before `from`). A foreign move's
    /// re-derivation; counted when it happens on OWN publish (footnote 4).
    fn requeue_from(&mut self, from: usize) {
        if self.in_own_publish {
            self.own_publish_rederived += 1;
        }
        for q in self.queue.iter_mut().skip(from) {
            q.warm_after = None;
        }
        // A fetch for a write that is no longer the FIRST `Applying` one is
        // moot: that write re-parks when its turn comes.
        let first = self.queue.get(from).map(|q| (q.client, q.write_id));
        if self.parked_write.as_ref().is_some_and(|p| Some((p.client, p.write_id)) != first) {
            self.release_write();
        }
    }

    /// The commit in flight is DEAD by the engine's lights (a foreign move).
    /// Its GROUP's fate is read from the WITNESS, the new head's ledger
    /// (COMMIT-LIFE ⁵): its `through` for this page at or past the group's
    /// last arrival means the group LANDED unheard -- every write of it is
    /// `Published`, never `Lost` (told `Lost`, the app rolls back a write
    /// that happened: sdk#293). Otherwise it did not land: back to the FRONT,
    /// `Queued` again and re-applied first, one try spent each (go-back-N, in
    /// the engine, once); a write out of tries, or forced (`Expect::Any`),
    /// falls `Lost`, named, never re-applied (WRITE-PATH ⁷).
    /// `dead` is the dead commit's cut; `spend` is false for a commit whose head never left (COMMIT-LIFE A8: a try
    /// counts an attempt to PUBLISH), whose witness the caller does not pass.
    fn dead_front(&mut self, witness: Option<Witness>, group_through: u64, dead: &[(ClientId, WriteId)], spend: bool) -> Vec<Effect> {
        let n = self.queue.iter().take_while(|q| dead.contains(&(q.client, q.write_id))).count();
        if n == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        // ABSENT IS NOT BELOW: an evicted entry (the list at its bound) cannot
        // say the group did not land. Told `Unknown`, named, never re-applied.
        if witness == Some(Witness::Unknown) {
            for _ in 0..n {
                let q = self.queue.pop_front().expect("committing");
                self.unknown_fates += 1;
                self.cascade.push((q.client, q.write_id, Self::written_keys(&q.ops)));
                out.push(Effect::Notify { client: q.client, write_id: q.write_id, state: State::Unknown });
            }
            self.close_cut();
            return out;
        }
        if matches!(witness, Some(Witness::Through(t)) if group_through > 0 && t >= group_through) {
            for _ in 0..n {
                let q = self.queue.pop_front().expect("committing");
                self.landed_by_witness += 1;
                out.push(Effect::Notify { client: q.client, write_id: q.write_id, state: State::Published });
            }
            self.close_cut();
            return out;
        }
        let mut i = 0;
        for _ in 0..n {
            let q = &mut self.queue[i];
            if spend {
                q.tries += 1;
            }
            let forced = q.reads.iter().any(|(_, e)| *e == Expect::Any);
            if !forced && q.tries <= self.params.max_write_tries {
                i += 1;
                continue;
            }
            if forced {
                self.forced_lost += 1;
            } else {
                self.tries_spent_lost += 1;
            }
            out.extend(self.end_queued(i, State::Lost));
        }
        self.close_cut();
        out
    }

    /// A foreign move landed (`self.published_root` is the new root): the
    /// dead commit's write is handled, and the WHOLE queue is re-derived on
    /// the new root, in order. `dead` is the dead commit's writes: one NOT
    /// in the queue (a commit this engine did not queue -- restored from a
    /// context) cannot go again from here, so it is told `Lost`, as before
    /// the queue: its client still holds it.
    fn rederive_after_foreign_move(&mut self, dead: &[(ClientId, WriteId)], group_through: u64, witness: Option<Witness>, spend: bool) -> Vec<Effect> {
        let queued: Vec<(ClientId, WriteId)> = self.queue.iter().take_while(|q| dead.contains(&(q.client, q.write_id))).map(|q| (q.client, q.write_id)).collect();
        let mut out: Vec<Effect> = dead
            .iter()
            .filter(|w| !queued.contains(w))
            .map(|w| Effect::Notify { client: w.0, write_id: w.1, state: State::Lost })
            .collect();
        out.extend(self.dead_front(witness, group_through, dead, spend));
        self.root = self.published_root;
        self.requeue_from(0);
        out.extend(self.advance());
        out
    }

    /// Park a write whose path is cold, and ask for what it needs.
    ///
    /// The bound is on ROUNDS, not on failures: an arrival that lets the
    /// apply get one level deeper and stop again is a SUCCESS, and a bound
    /// that counts only misses would never stop a descent that makes progress
    /// for ever. A tree this engine wrote is bounded in depth, but the root
    /// it was handed is not necessarily one it wrote.
    fn park_write(&mut self, client: ClientId, write_id: WriteId, need: Vec<Cid>) -> Vec<Effect> {
        let rounds = self.parked_write.as_ref().map_or(0, |p| p.rounds) + 1;
        if rounds > self.params.max_apply_rounds {
            // Nothing was applied, so `Failed` is true: the client still has
            // the write and re-submitting it applies it once.
            self.release_write();
            return vec![Effect::Notify {
                client,
                write_id,
                state: State::Failed { why: FailWhy::FetchBudget },
            }];
        }
        // ONE ROUND AT A TIME, AND NO MORE THAN THE CHAIN HAS LEFT (sdk#174).
        // Asking for a whole path at once stranded past a return's GETs
        // (matrix W3); a chain past the node's iteration cap is truncated
        // without a word. So a round asks for what one return carries, and a
        // chain whose GETs are spent asks for nothing until the client asks
        // after its write -- `on_ask` -- which is a new chain.
        let gets = self
            .parked_write
            .as_ref()
            .filter(|p| p.write_id == write_id)
            .map_or(0, |p| p.gets);
        let room = self.params.max_gets_per_request.saturating_sub(gets) as usize;
        let held = self.parked_write.as_ref().filter(|p| p.write_id == write_id).map(|p| p.held.clone()).unwrap_or_default();
        let needs: BTreeSet<Cid> = need
            .iter()
            .copied()
            .take(self.params.max_fetch_per_round.min(room))
            .collect();
        // The write it replaces (itself, a round on) gives up its needs first; a block it still needs is wanted again
        // below, in this same call, so nothing is told `Unwanted` for it (net at the call's end).
        self.release_write();
        self.parked_write = Some(ParkedWrite {
            client,
            write_id,
            rounds,
            gets: gets + needs.len() as u32,
            idle_ticks: 0,
            idle_at: self.now,
            held,
        });
        // ONE ask per block (W5): a need a read or a repair already has in flight rides that GET.
        let mut out = Vec::new();
        for id in needs {
            if !self.want(id, Reader::Write).any() {
                out.push(Effect::FetchBlock { id, via: read::Via::Direct, attempt: rounds });
            }
        }
        out
    }

    /// A block arrived: if the parked write was waiting on it, apply again.
    ///
    /// Only on a block it actually asked for. An engine that re-ran the apply
    /// on every arrival would redo the whole descent for each of a commit's
    /// own confirmations, and the cost of a cold write would be the number of
    /// blocks moving on the node rather than the depth of the tree.
    fn resume_parked_write(&mut self, id: Cid) -> Vec<Effect> {
        if self.parked_write.is_none() || !self.wanted.write_needs().contains(&id) {
            return Vec::new();
        }
        // Arrived: the write's need of it is met (the block is held, so never `Unwanted`).
        self.drop_reader(id, Reader::Write);
        let Some(p) = self.parked_write.as_mut() else {
            return Vec::new();
        };
        p.held.insert(id);
        p.idle_ticks = 0;
        if !self.wanted.write_needs().is_empty() {
            // Still waiting on the rest of this round's blocks. Running now
            // would spend a round to stop at the very next one.
            return Vec::new();
        }
        // Its round is in: it applies (or asks for its next round) THROUGH
        // THE QUEUE, on the warm root as it is now -- a tree that moved under
        // it is simply the tree it is judged on (R-b).
        self.advance()
    }

    /// Every read against the tree this apply lands on.
    /// Every read against the tree this apply lands on. A read that no
    /// longer holds is still SATISFIED when this write's own op for that key
    /// leaves exactly what the tree holds now — a write that is already there
    /// (main's ruling on sdk#148: two tabs defining one schema from one
    /// Absent base both succeed; deleting what is already gone is not a
    /// conflict). Only a key the write WRITES can be satisfied that way; a
    /// read of anything else that moved is a Conflict.
    fn check_reads(&self, reads: &[(Vec<u8>, Expect)], ops: &[(Vec<u8>, Op)]) -> Result<(), ReadsCheck> {
        let source = WithEmptyLeaf {
            inner: &self.blocks,
            empty_cid: self.empty.cid,
            empty_bytes: &self.empty.bytes,
            arrived: &self.arrived,
        };
        let mut need = Vec::new();
        for (key, want) in reads {
            if *want == Expect::Any {
                continue; // holds against any tree, so it reads nothing
            }
            let found = match freenet_prolly::read::get(&source, &self.root, key) {
                Ok(v) => v.map(LeafForm::of),
                Err(ReadError::Need(n)) => {
                    need.extend(n);
                    continue;
                }
                Err(_) => return Err(ReadsCheck::Unreadable),
            };
            let holds = match (want, &found) {
                (Expect::Absent, None) => true,
                (Expect::Present, Some(_)) => true,
                (Expect::Value(h), Some(f)) => f.hash() == *h,
                _ => false,
            };
            // The write's LAST op on this key decides what it leaves there.
            let already_there = || match ops.iter().rev().find(|(k, _)| k == key) {
                Some((_, Op::Put(v))) => found.as_ref().is_some_and(|f| f.hash() == leaf_hash(v)),
                Some((_, Op::Delete)) => found.is_none(),
                None => false,
            };
            if !holds && !already_there() {
                return Err(ReadsCheck::Differs { key: key.clone(), current: found });
            }
        }
        if need.is_empty() {
            Ok(())
        } else {
            need.sort();
            need.dedup();
            Err(ReadsCheck::Need(need))
        }
    }





    /// Plan the packs for what a commit emitted, and send them.
    fn start_commit(
        &mut self,
        emitted: Vec<(Cid, Vec<u8>)>,
        parity: Vec<(Cid, Vec<u8>)>,
        writes: Vec<(ClientId, WriteId)>,
        bytes: usize,
    ) -> Vec<Effect> {
        // THE ROOT IS A GROUP OF ONE (sdk#335): its parity is coded here, put
        // in the same round as everything else, counted by the race like any
        // group (any 1 of its 1 + m), and listed in the head. A root this
        // commit did not emit (none new) has none.
        let mut parity = parity;
        let blocks = root_parity_blocks(self.root, &emitted);
        let root_parity: Vec<Cid> = blocks.iter().map(|(id, _)| *id).collect();
        parity.extend(blocks);
        let race = Race::of(&emitted, &parity, &self.unacked(), (self.root, &root_parity));
        // THE FIRST WAVE (#378 P1-hybrid): one parity per changed group goes now, the rest when the head lands --
        // so the Sign, sent at k, queues behind at most ~1 extra PUT per group on the node's one queue (F61), not m.
        let (parity, deferred): (Vec<Block>, Vec<Block>) = if self.params.race_put && self.params.first_wave_parity {
            let ids: BTreeSet<Cid> = parity.iter().map(|(c, _)| *c).collect();
            let first: BTreeSet<Cid> = race.groups.iter().filter_map(|g| g.new.iter().find(|c| ids.contains(*c)).copied()).collect();
            parity.into_iter().partition(|(c, _)| first.contains(c))
        } else {
            (parity, Vec::new())
        };
        let seq = self.next_seq;
        // In a commit now, so no longer stalled: if it stalls again later that
        // is a new fact and deserves a new notice.
        for q in self.queue.iter_mut() {
            if writes.contains(&(q.client, q.write_id)) {
                q.stalled_told = false;
            }
        }

        // PHASE 3 WRITES MEMBERS, NOT PACKS.
        //
        // A pack is a transport and it is inherently TRANSIENT: a head names
        // at most a few, and one leaves the network as soon as it is
        // unpacked. So the total is always pack PLUS members — a pack only
        // changes WHO pays for the members, and with no keepers in Phase 3
        // the writer pays for them anyway. Sending both cost ~188 KiB per
        // commit for a pack nothing reads.
        //
        // What makes members-only need no new read path is F35: a delegate's
        // own PUT is HOSTED, so a member it wrote is readable at its own key.
        //
        // The pack FORMAT stays — kind, pack.rs, its vectors — because Phase
        // 4 (#39) is where it earns its place: the writer puts the pack only,
        // keepers put the members, and reads resolve through the head's
        // packs. This is the write path, not the format.
        let (packable, direct): (Vec<_>, Vec<_>) = if self.params.pack_on_write {
            emitted
                .into_iter()
                .partition(|(_, b)| b.len() <= self.params.max_packed_value)
        } else {
            (Vec::new(), emitted)
        };

        let manifest = pack::Manifest {
            prev_seq: self.published_seq,
            prev_root: self.published_root,
            seq,
            root: self.root,
        }
        .encode();

        let mut out: Vec<Effect> = Vec::new();
        let mut data: BTreeSet<Cid> = BTreeSet::new();

        for (id, b) in &direct {
            data.insert(*id);
            out.push(Effect::PutBlock {
                id: *id,
                bytes: b.clone(),
                after: Vec::new(),
            });
        }
        // THE FIRST WAVE'S PARITY (§P, #378 P1-hybrid): one per changed group, computed before the send and put with
        // the data. The rest (`deferred`) are this commit's blocks too -- in `data`, counted toward k if they land,
        // needed for BACKED_UP -- and are put when the head lands (`follow_ups`).
        for (id, _) in &deferred {
            data.insert(*id);
        }
        for (id, b) in &parity {
            data.insert(*id);
            out.push(Effect::PutBlock {
                id: *id,
                bytes: b.clone(),
                after: Vec::new(),
            });
        }

        // Every pack carries the manifest, so recovery can read any one of a
        // commit's packs and know the whole commit.
        let mut current: Vec<(u8, Vec<u8>)> = vec![(kind_raw(), manifest.clone())];
        let mut used = pack::PACK_HEADER + pack::member_cost(manifest.len());
        let mut packs: Vec<Vec<(u8, Vec<u8>)>> = Vec::new();
        for (_, b) in packable {
            let cost = pack::member_cost(b.len());
            if used + cost > self.params.max_pack && current.len() > 1 {
                packs.push(std::mem::take(&mut current));
                current = vec![(kind_raw(), manifest.clone())];
                used = pack::PACK_HEADER + pack::member_cost(manifest.len());
            }
            used += cost;
            current.push((pack::member_kind(&b).byte(), b));
        }
        // A commit that emitted nothing packable still ships its manifest: the
        // journal entry is the point, not the payload. With packing off the
        // write path there is no pack at all — the head IS the journal, and
        // a manifest nobody will read is bytes nobody should pay for.
        if self.params.pack_on_write {
            packs.push(current);
        }

        let mut pack_bodies: BTreeMap<Cid, Vec<u8>> = BTreeMap::new();
        let mut pack_members: BTreeMap<Cid, Vec<Cid>> = BTreeMap::new();
        for members in packs {
            let body = match pack::build(&members) {
                Ok(b) => b,
                // Planning is what keeps a pack within its bounds, and
                // `Engine::new` refuses the settings that could make one
                // unplannable, so a refusal here is a bug in the planner.
                Err(e) => unreachable!("the engine planned an unbuildable pack: {e:?}"),
            };
            let id = pack::pack_id(&body);
            data.insert(id);
            pack_members.insert(id, pack::members(&body).into_iter().map(|(mid, _)| mid).collect());
            pack_bodies.insert(id, body.clone());
            out.push(Effect::PutPack {
                id,
                bytes: body,
                after: Vec::new(),
            });
        }

        self.next_seq += 1;
        let commit = Commit {
            seq,
            root: self.root,
            data,
            packs: pack_bodies,
            confirmed: BTreeSet::new(),
            writes,
            bytes,
            base: self.published_root,
            through: 0,
            race,
            deferred,
            held: BTreeSet::new(),
            unknown: BTreeSet::new(),
            pack_members,
            root_parity,
        };
        // Idle -> Racing (E1). The cut is taken only in Idle (`commit_front`), so a refusal here is an impossible cell.
        if !self.life.start(commit, self.now) {
            self.impossible_transitions += 1;
        }
        // SAVED NEEDS THE NODE'S WORD (sdk#416): every other member of a changed group is asked about. The page
        // answers at once for what it confirmed (`PutConfirmed`), and says `HeldUnknown` for the rest while it asks
        // the node -- each of those releases one more of its group's parity, so k stays reachable from this page's
        // own PUTs whatever the other page's block does.
        let others = self.life.commit().map(|c| c.race.others()).unwrap_or_default();
        out.extend(others.into_iter().map(|id| Effect::ConfirmHeld { id }));
        out
    }

    /// The page cannot confirm `id`, another member of a changed group, at once (sdk#416): it counts ABSENT toward
    /// k until the node says it holds it, and ONE more of its group's parity goes now, from the parity held back
    /// for after the head (#378). Once per member.
    fn on_held_unknown(&mut self, id: Cid) -> Vec<Effect> {
        let Some(c) = self.life.commit_mut() else { return Vec::new() };
        if c.held.contains(&id) || !c.unknown.insert(id) {
            return Vec::new();
        }
        let Some(g) = c.race.groups.iter().find(|g| g.others.contains(&id)) else { return Vec::new() };
        let Some(at) = c.deferred.iter().position(|(d, _)| g.new.contains(d)) else { return Vec::new() };
        let (id, bytes) = c.deferred.remove(at);
        vec![Effect::PutBlock { id, bytes, after: Vec::new() }]
    }

    /// THE HEAD QUESTION (COMMIT-LIFE Heading x E12): a Heading commit that has heard nothing for its pace
    /// (`reask_after`, doubling per round) READS the head -- its seq and root are known, so nothing is re-sent blind --
    /// and the answer is E6, E7 or E8. A QUESTION, never an end (C5, rule 8). A Racing commit has no such round: its
    /// PUTs re-send in `Page::send` until answered, and silence never ends it (A6); only answers do.
    fn ask_head(&mut self, now: u64) -> Vec<Effect> {
        if !self.life.head_question_due(now, self.params.reask_after) {
            return Vec::new();
        }
        vec![Effect::ReadHead { epoch: self.head_epoch.unwrap_or(Epoch(1)) }]
    }

    /// The head read back shows the commit's head is NOT there (an older seq,
    /// or none): the write of it never landed. Re-issue it -- the same seq and
    /// root, which the Register takes or refuses on its own terms.
    fn head_not_landed(&mut self) -> Vec<Effect> {
        // Heading x E7: RE-ISSUE, never back to Racing (COMMIT-LIFE C2, A2). Race-ready only grows while the commit is
        // in flight (held evidence is not retracted), so the same head is still one the race rule signs. Racing x E7:
        // its head is not out; nothing.
        match self.life.stage() {
            CommitStage::Heading => self.life.commit().map(|c| vec![self.head_of(c)]).unwrap_or_default(),
            CommitStage::Racing | CommitStage::Idle => Vec::new(),
        }
    }

    /// The commit's `UpdateHead`: what the page holds it for is the race set (the blocks counted), or -- in the
    /// `head_before_packs` control arm, which sends it before race-ready -- every data block.
    fn head_of(&self, c: &Commit) -> Effect {
        Effect::UpdateHead {
            seq: c.seq,
            root: c.root,
            base: c.base,
            after: if self.params.head_before_packs { c.data.iter().copied().collect() } else { c.race_set() },
        }
    }

    /// These writes' Backing drained: each is BACKED_UP unless it still sits
    /// in another Backing (a write carried onto a newer commit waits for that
    /// one too) or in the carry (waiting for the next own commit).
    fn backed_up(&mut self, writes: Vec<(ClientId, WriteId)>) -> Vec<Effect> {
        let mut out = Vec::new();
        let mut told = BTreeSet::new();
        for w in writes {
            if !told.insert(w) || self.life.waits_for(&w) {
                continue;
            }
            out.push(Effect::Notify { client: w.0, write_id: w.1, state: State::ParityComplete });
        }
        out
    }

    /// The nodes of `was` that `now` does not have (`diff(now, was)`'s
    /// `new_blocks`, paged and unioned), or `None` if the diff could not be
    /// completed from what is held -- then nothing is superseded, the
    /// conservative side (a withdrawn block that was still needed is a hole
    /// nothing tracks; a kept one costs only its re-sends).
    fn removed_nodes(&self, was: Cid, now: Cid) -> Option<BTreeSet<Cid>> {
        use freenet_prolly::range::Range;
        use std::ops::Bound;
        let src = self.source();
        let r = Range { lo: Bound::Unbounded, hi: Bound::Unbounded, reverse: false, after: None, max_entries: usize::MAX, max_bytes: usize::MAX };
        let mut removed = BTreeSet::new();
        let mut resume = None;
        for _ in 0..1_000_000 {
            let page = freenet_prolly::diff::diff(&src, &now, &was, &r, resume.as_ref()).ok()?;
            if !page.need.is_empty() {
                return None;
            }
            removed.extend(page.new_blocks.iter().copied());
            match page.next {
                Some(next) => resume = Some(next),
                None => return Some(removed),
            }
        }
        None
    }

    /// COMMIT-LIFE §P, superseded stragglers, on EVERY published-root move
    /// (own commit or foreign/merge) from `was` to `now`: a straggler that
    /// is a NODE removed by the move, or the PARITY of a removed branch node
    /// (a group over nodes), is garbage -- WITHDRAWN, and its commit's writes
    /// carried onto the next own commit's Backing. Node bytes include their
    /// keys, so a node is unique to its position and a removed one is needed
    /// by nobody. VALUE blocks and the parity of VALUE groups (a leaf's) are
    /// NEVER withdrawn: the same value under two keys is one block, and
    /// telling it apart would need a whole-tree scan -- they retry until acked
    /// (the architect's ruling).
    fn supersede(&mut self, was: Cid, now: Cid) -> Vec<Effect> {
        if was == now || self.life.backing_len() == 0 {
            return Vec::new();
        }
        let Some(removed) = self.removed_nodes(was, now) else {
            return Vec::new();
        };
        let mut gone: BTreeSet<Cid> = BTreeSet::new();
        {
            let src = self.source();
            for n in &removed {
                gone.insert(*n);
                // The ROOT's parity (sdk#335) is listed in its head, not a node.
                if let Some(p) = self.root_parity.get(n) {
                    gone.extend(p.iter().copied());
                }
                if let Some(bytes) = src.get(n) {
                    if let Ok(node) = Node::parse(bytes) {
                        if !node.is_leaf() {
                            gone.extend(node.parity());
                        }
                    }
                }
            }
        }
        let (withdrawn, drained) = self.life.supersede(&gone);
        let mut out: Vec<Effect> = withdrawn.into_iter().map(|id| Effect::Withdraw { id }).collect();
        out.extend(self.backed_up(drained));
        out
    }

    fn on_confirmed(&mut self, id: Cid) -> Vec<Effect> {
        // The node holds it now: a warm-apply block leaves the W pin, and its bytes the queue's sum (sdk#450).
        // (A block a LATER warm apply re-emits after the node confirmed it stays counted until the queue idles --
        // an over-count, never an under-count: the confirmation is not repeated.)
        if let Some(n) = self.kept.remove(&id) {
            self.kept_bytes -= n;
        }
        let head_early = self.params.head_before_packs;
        let mut out = Vec::new();
        // A published commit's straggler landing: one closer to BACKED_UP.
        let backed = self.life.acked(&id);
        out.extend(self.backed_up(backed));

        let unacked = self.unacked();
        let heading = self.life.stage() == CommitStage::Heading;
        let race_put = self.params.race_put;
        let Some(c) = self.life.commit_mut() else {
            // A confirmation for something no commit is waiting on. Duplicates
            // and stragglers are normal on a network; they are not errors and
            // they are not events.
            return out;
        };
        // This commit's block, or an EARLIER commit's straggler that a group
        // of this one was counting on (rule 10: it counts once it is acked).
        let mine = c.data.contains(&id) && c.confirmed.insert(id);
        if mine {
            if let Some(members) = c.pack_members.get(&id).cloned() {
                c.confirmed.extend(members);
            }
        }
        let earlier = c.race.groups.iter().any(|g| g.earlier.contains(&id));
        // Another member of a changed group, confirmed by the node (sdk#416): it counts toward k now.
        let other = c.race.groups.iter().any(|g| g.others.contains(&id)) && c.held.insert(id);
        if !mine && !earlier && !other {
            return out;
        }
        if heading || (!head_early && !c.ready(race_put, &unacked)) {
            return out;
        }
        // Every block of this commit has been READ BACK from our own node.
        // Only now may the head move: a head naming a root whose blocks are
        // not all there is a tree readers cannot walk.
        //
        // No state is reported here. `Durable` used to be, and it promised a
        // recovery nobody can perform: a pack's key is its content hash, so a
        // restarted engine holding only its device key cannot enumerate packs
        // its head does not name. Only the HEAD makes anything findable, so
        // the head is the journal and `published` is the first state that
        // survives a restart.
        // Racing -> Heading (the one forward move, C2).
        self.life.send_head(self.now);
        if let Some(c) = self.life.commit() {
            out.push(self.head_of(c));
        }
        out
    }

    /// `id` was REJECTED by the node's Block contract (`Event::PutRejected`, sdk#433; the architect's rulings).
    fn on_rejected(&mut self, id: Cid) -> Vec<Effect> {
        self.rejected.insert(id);
        let Some(c) = self.life.commit() else { return Vec::new() };
        let own = c.data.contains(&id) || c.packs.contains_key(&id);
        let ends = match self.life.stage() {
            // Racing x E4: any block of its own -- re-sent, the writes would re-derive the same block.
            CommitStage::Racing => own,
            // Heading x E4 (the architect on rev 5, A9): the ROOT -- the signer refuses a head whose root is not on
            // the node (`RootNotHeld`, retryable), and a rejected root never will be -- or a block without which a
            // group falls below k. Any other block: the commit stays (the head may land at k); its Backing is never
            // BACKED_UP.
            CommitStage::Heading => own && (id == c.root || !c.race.reachable(&self.rejected)),
            CommitStage::Idle => false,
        };
        if !ends {
            // A published commit's straggler (or one whose head is out and may land): the Backing stays, so
            // BACKED_UP never lies; nothing is put again.
            return Vec::new();
        }
        // A REAL END (rule 8): the commit's writes are Failed.
        self.end_commit(End::Failed(FailWhy::BlockRejected))
    }

    /// The signer finally refused commit `seq`'s head (PUBLISH-LIFE ⁵): if `seq` is the commit in flight, it ends
    /// `Failed`; any other seq is stale -- that head is already dead -- and nothing happens.
    fn on_head_refused(&mut self, seq: u64) -> Vec<Effect> {
        if !self.life.commit().is_some_and(|c| c.seq == seq) {
            return Vec::new();
        }
        self.end_commit(End::Failed(FailWhy::SignerRefused))
    }

    /// THE ONE TEARDOWN (COMMIT-LIFE rev 5, C3): every end of the commit in flight -- its head showed, a foreign head,
    /// a final refusal -- comes here, in one order: what the dead commit still has on the way is withdrawn, its writes
    /// are told, the published root moves (own, or the foreign head adopted), `next_seq` follows it, the queue is
    /// re-derived (never on own publish, ⁴), and the next cut is taken (K1). The per-end part is one exhaustive match.
    fn end_commit(&mut self, end: End) -> Vec<Effect> {
        let unacked = self.unacked();
        let Some((c, headed)) = self.life.end() else { return Vec::new() };
        let mut out = Vec::new();
        self.unpublished.clear();
        // Did a dead commit's group LAND unheard (⁵)? Then its blocks are in the published tree: never withdrawn, they
        // are owed toward BACKED_UP like any published commit's. An `Unknown` witness may have landed: not withdrawn.
        let landed = |w: &Option<Witness>| matches!(w, Some(Witness::Through(t)) if c.through > 0 && *t >= c.through);
        let withdraw = match &end {
            End::Published => false,
            End::Failed(_) => true,
            End::Dead { witness, .. } => !headed || !(landed(witness) || *witness == Some(Witness::Unknown)),
        };
        if withdraw {
            let mut gone: BTreeSet<Cid> = c.owed().copied().collect();
            gone.extend(c.race.others().into_iter().filter(|m| !c.held.contains(m)));
            gone.retain(|id| !unacked.contains(id));
            out.extend(gone.into_iter().map(|id| Effect::Withdraw { id }));
        }
        match end {
            End::Published => out.extend(self.publish(c)),
            End::Failed(why) => {
                self.next_seq = self.published_seq + 1;
                // Re-applied they would meet the same refusal: they leave the queue.
                self.queue.retain(|q| !c.writes.contains(&(q.client, q.write_id)));
                out.extend(c.writes.iter().map(|w| Effect::Notify { client: w.0, write_id: w.1, state: State::Failed { why } }));
                self.root = self.published_root;
                self.requeue_from(0);
                out.extend(self.advance());
            }
            End::Dead { witness, head } => {
                if headed && landed(&witness) {
                    let remaining: BTreeSet<Cid> = c.owed().copied().collect();
                    if !remaining.is_empty() {
                        self.life.back(c.writes.clone(), remaining);
                    }
                }
                out.extend(self.adopt(head.0, head.1));
                // A head that never left reads no witness and spends no try (A8).
                let witness = if headed { witness } else { None };
                out.extend(self.rederive_after_foreign_move(&c.writes, c.through, witness, headed));
            }
        }
        out
    }

    /// Blocks the node's Block contract rejected (sdk#433): what the assets dashboard reads as DAMAGED -- a repair
    /// would recompute the same bytes and be rejected again.
    pub fn rejected_blocks(&self) -> &BTreeSet<Cid> {
        &self.rejected
    }

    fn on_failed(&mut self, id: Cid) -> Vec<Effect> {
        // A block the node's contract REJECTED is never put again (sdk#433).
        if self.rejected.contains(&id) {
            return Vec::new();
        }
        // A published commit's straggler the node refused: put it again from
        // the page's blocks (the page keeps them until BACKED_UP).
        if self.life.backing_owes(&id) {
            if let Some(bytes) = self.blocks.get(&id) {
                return vec![Effect::PutBlock { id, bytes: bytes.to_vec(), after: Vec::new() }];
            }
            // Its bytes are gone: the straggler can never be put again (sdk#411's B pin exists to make this 0).
            self.reput_missing += 1;
            return Vec::new();
        }
        // Re-emit exactly what is missing, and nothing else. A retry that
        // re-sends the whole commit pays for every block again, and a retry
        // that re-sends nothing stalls it for ever.
        let Some(c) = self.life.commit() else {
            return Vec::new();
        };
        if !c.owes(&id) {
            return Vec::new();
        }
        // A pack first: its body exists only here, so if this does not send it
        // again nothing will, and the commit waits for ever on a put that has
        // already failed.
        if let Some(body) = c.packs.get(&id) {
            return vec![Effect::PutPack {
                id,
                bytes: body.clone(),
                after: Vec::new(),
            }];
        }
        // A value block: it is in the warm tree.
        if let Some(bytes) = self.blocks.get(&id) {
            return vec![Effect::PutBlock {
                id,
                bytes: bytes.to_vec(),
                after: Vec::new(),
            }];
        }
        // Its bytes are gone: the commit waits for ever on it (the pack path's; off in this phase).
        self.reput_missing += 1;
        Vec::new()
    }

    /// THE RE-PUT FROM PAGE MEMORY (sdk#411): the bytes of `id` for a PUT again -- the Wrapper path's `Held` rule,
    /// after HELD_ABSENTS absents in a row. `None` if the page does not hold them; and when the block is one this
    /// engine's commits PUT (the pending commit's, or a published one's straggler) its bytes were the page's, so
    /// their absence is COUNTED ([`Engine::reput_missing`]: the pin rule's model property, always 0). A foreign member
    /// the page never held is not counted.
    pub fn reput_bytes(&mut self, id: &Cid) -> Option<Vec<u8>> {
        if let Some(bytes) = self.blocks.get(id) {
            return Some(bytes.to_vec());
        }
        let own = self.life.commit().is_some_and(|c| c.data.contains(id)) || self.life.backing_owes(id);
        if own {
            self.reput_missing += 1;
        }
        None
    }


    /// THE REST OF THE PARITY (#378 P1-hybrid), sent when the head LANDS: behind the Sign AND the head's UPDATE,
    /// which would otherwise queue behind them on the node's one queue (F61; live: `PPSPPPPPPPU` put the UPDATE
    /// seven PUTs back). Each is put and retried until acked like any PUT; the commit is BACKED_UP only when they are.
    fn follow_ups(&mut self) -> Vec<Effect> {
        let mut out = Vec::new();
        let Some(c) = self.life.commit_mut() else {
            return out;
        };
        for (id, bytes) in std::mem::take(&mut c.deferred) {
            if c.owes(&id) {
                out.push(Effect::PutBlock { id, bytes, after: Vec::new() });
            }
        }
        out
    }

    fn on_head(&mut self, seq: u64) -> Vec<Effect> {
        if !self.life.commit().is_some_and(|c| c.seq == seq) {
            return Vec::new();
        }
        // Racing x E6 is impossible: no head was sent for this seq. The head showing it is a fact, so it publishes;
        // the cell is counted.
        if self.life.stage() == CommitStage::Racing {
            self.impossible_transitions += 1;
        }
        let mut out = self.follow_ups();
        out.extend(self.end_commit(End::Published));
        out
    }

    /// OWN PUBLISH, `end_commit`'s Published part: the published root moves to `c`'s, its writes are `Published`,
    /// its Backing opens (or BACKED_UP now), the front leaves the queue and the next cut goes, with nothing re-derived.
    fn publish(&mut self, c: Commit) -> Vec<Effect> {
        let mut out = Vec::new();
        let was = self.published_root;
        self.published_seq = c.seq;
        self.published_root = c.root;
        // THIS engine's commit: every group it coded is in `owed`, so a tree
        // known in full stays known in full. `NotScanned` stays `NotScanned`:
        // one commit says nothing about the parity of the tree beneath it.
        if matches!(self.parity_scan, ParityScan::Done { .. }) {
            self.parity_scan = ParityScan::Done { root: c.root };
        }
        // ONE per commit: this runs where the head is confirmed, which
        // happens once per commit, rather than per write or per block.
        out.extend(self.notify_subs(was));
        for (client, write_id) in &c.writes {
            out.push(Effect::Notify {
                client: *client,
                write_id: *write_id,
                state: State::Published,
            });
        }
        // SAVED is `Published`, just said. BACKED_UP (`ParityComplete`) when
        // ALL k+m of every group the commit changed are acked (§P): its own
        // blocks AND an earlier commit's stragglers in those groups (the
        // members it counted in `earlier`). Counting only its own let a write
        // be BACKED_UP while a group it changed still missed a block (the page
        // model: "ParityComplete, but the root it was published at is not
        // WHOLE"). Now, or as the rest land. The stragglers stay in `send()`
        // with their re-sends (stall retry is still the standard); the engine
        // only counts.
        // SUPERSEDED stragglers first: this commit may re-code a group an
        // earlier one is still putting; those old blocks are withdrawn and the
        // earlier writes are carried onto THIS commit's Backing.
        if !c.root_parity.is_empty() {
            self.root_parity.insert(c.root, c.root_parity.clone());
        }
        out.extend(self.supersede(was, c.root));
        self.keep_root_parity();
        let still_out = self.unacked();
        let mut remaining: BTreeSet<Cid> = c.data.difference(&c.confirmed).copied().collect();
        remaining.extend(c.race.groups.iter().flat_map(|g| g.earlier.iter()).filter(|m| still_out.contains(*m)).copied());
        // THE OTHER MEMBERS of the groups this commit changed (class 2): asked at `start_commit` (sdk#416), and
        // BACKED_UP waits for the node's word on each one not confirmed yet; the page is still asking.
        remaining.extend(c.race.others().into_iter().filter(|m| !c.held.contains(m) && !still_out.contains(m)));
        let mut writes = c.writes.clone();
        for w in self.life.take_carry() {
            if !writes.contains(&w) {
                writes.push(w);
            }
        }
        if remaining.is_empty() {
            out.extend(self.backed_up(writes));
        } else {
            self.life.back(writes, remaining);
        }
        // OWN PUBLISH (R-b, footnote 4): the front leaves the queue
        // `Published`; the warm root is UNCHANGED -- it already is the
        // published root plus the rest -- so nothing is re-derived here, and
        // the next front commits, rebuilt on the root just published.
        let mut popped = 0;
        while self.queue.front().is_some_and(|f| c.writes.contains(&(f.client, f.write_id))) {
            self.queue.pop_front();
            popped += 1;
        }
        if popped != c.writes.len() && !c.writes.is_empty() && popped > 0 {
            self.impossible_transitions += 1;
        }
        self.commits_published += 1;
        self.writes_published += popped as u64;
        self.close_cut();
        self.in_own_publish = true;
        out.extend(self.advance());
        self.in_own_publish = false;
        out
    }

    /// The blocks written since the last commit shipped, each once.
    ///
    /// This used to be recovered by walking the published tree and the warm
    /// tree and taking the difference — two whole-tree walks per follow-on
    /// commit, to recompute something the write path already knew. A write
    /// that emits a block is the moment the answer exists; collecting it then
    /// costs nothing and cannot disagree with itself.
    ///
    /// De-duplicated by id, because two writes in one commit can touch the
    /// same node and a pack is a set.
    fn take_unpublished(&mut self) -> Vec<(Cid, Vec<u8>)> {
        let mut seen: BTreeSet<Cid> = BTreeSet::new();
        std::mem::take(&mut self.unpublished)
            .into_iter()
            .filter(|(c, _)| seen.insert(*c))
            .collect()
    }

    /// The client is going away: do now what a tick would eventually do.
    ///
    /// There is no clock. `Tick` arrives only from a connected client, so
    /// once the tab closes nothing else will ever ask the engine to get on
    /// with it — and everything the page-closed promise covers must therefore
    /// be event-driven or happen HERE.
    ///
    /// Two things. Start a commit for anything applied but not yet shipped,
    /// and put ALL owed parity regardless of age: coalescing trades a little
    /// redundancy-latency for fewer puts while someone is watching, and there
    /// is nothing left to coalesce WITH once they are gone. A commit already
    /// in flight is left alone; it advances on its own confirmations, which
    /// is exactly the part that does not need a clock.
    fn on_flush(&mut self) -> Vec<Effect> {
        let mut out = Vec::new();
        // No `can_sign` check here: `unpublished` is filled only by the cut in
        // `advance`, the ONE place the gate is read, so while the page cannot
        // sign there is nothing here to ship.
        if self.life.commit().is_none() && !self.unpublished.is_empty() {
            let to_ship = self.take_unpublished();
            out.extend(self.start_commit(to_ship, Vec::new(), Vec::new(), 0));
        }
        out
    }

    fn on_tick(&mut self, now: u64) -> Vec<Effect> {
        // THE FIRST CLOCK THIS ENGINE SEES. `now` is 0 until a tick arrives, so
        // a commit started before any tick recorded `in_flight_since = 0` --
        // and against an epoch-seconds tick that is 56 years old, which told
        // every write `Stalled` on the first tick (sdk#150, W4). A start taken
        // before there was a clock is anchored to the first clock, not
        // measured from zero. A real tick is never 0, so 0 means "unknown".
        // ENGINE TIME IS THE MOST IT HAS SEEN. Every tab and every device
        // ticks from its own clock, so two of them a second -- or five minutes
        // -- apart alternate forwards and backwards. A step back within
        // `CLOCK_RESET_TICKS` is that: time has not moved, and the tick's work
        // runs at the time already reached. Taken as a reset, it re-anchored
        // every deadline on every other tick and nothing was ever due again
        // (21 parity puts in 600 s against 126 on one clock, executed).
        let now = if self.now != 0 && now < self.now && self.now - now <= CLOCK_RESET_TICKS {
            self.now
        } else {
            now
        };
        if self.now == 0 {
            self.life.anchor(now, false);
            self.asks.anchor(now);
        } else if now < self.now || now - self.now > CLOCK_RESET_TICKS {
            // A CLOCK RESET (sdk#150 review B). `now` is whatever the client
            // sends, and every deadline is measured against it: one tick ten
            // years ahead froze every re-ask for good (0 in 600 ticks, executed)
            // and the stall timer and owed ages with it. A context lives 600 s
            // (F32), so a jump of more than that, either way, cannot be the
            // same clock: every date is re-anchored to it. Nothing
            // becomes due early for it, and nothing waits on a clock that is
            // gone.
            self.life.anchor(now, true);
            self.asks.reanchor(now);
            if let Some(p) = self.parked_write.as_mut() {
                p.idle_at = now;
            }
        }
        self.now = now;
        let mut out = self.age_out_accepted(now);
        out.extend(self.release_silent_parked_write(now));
        out.extend(self.ask_head(now));
        out
    }

    /// A parked write that has heard NOTHING -- no block it asked for, no
    /// client asking after it -- for `3 * reask_after` TICKS RUN is released
    /// `Busy`: nothing of it is in the tree, and `Busy` is the verdict a
    /// client re-sends (`Failed` is rolled back and shown to the person).
    /// Waiting for a continuation that never comes would otherwise keep the
    /// engine `Busy` to every writer for good (sdk#174; the audit's E7 in
    /// general, PR 6).
    ///
    /// A tick that RAN is the only honest unit: the node runs one request at
    /// a time, so a tick running means no chain of this write is being
    /// served at that moment. A tick carrying the same time as the last one
    /// counted (another tab, the same second) is not another second.
    fn release_silent_parked_write(&mut self, now: u64) -> Vec<Effect> {
        let limit = 3 * self.params.reask_after;
        let Some(p) = self.parked_write.as_mut() else {
            return Vec::new();
        };
        if now <= p.idle_at {
            return Vec::new();
        }
        p.idle_at = now;
        p.idle_ticks += 1;
        if p.idle_ticks < limit {
            return Vec::new();
        }
        // `Failed`, named (COMMIT-LIFE: a fetch past its budget), never
        // `Busy`; and the next `Applying` write is tried (footnote 8).
        let p = self.release_write().expect("checked");
        let mut out = match self.queue.iter().position(|q| (q.client, q.write_id) == (p.client, p.write_id)) {
            Some(i) => self.end_queued(i, State::Failed { why: FailWhy::SilentRelease }),
            None => vec![Effect::Notify { client: p.client, write_id: p.write_id, state: State::Failed { why: FailWhy::SilentRelease } }],
        };
        out.extend(self.advance());
        out
    }

    /// Say so when a write has sat merely accepted too long.
    ///
    /// What this reports is a commit that cannot publish, with its own writes
    /// sitting `Accepted` and nothing moving.
    ///
    /// They are NOT dropped. Their edits are in the tree and publish when the
    /// commit confirms, so telling a client `Failed` would be a false
    /// statement with teeth: the client re-submits, the original publishes
    /// anyway, and a write someone else made in between is overwritten by the
    /// re-submission. `Stalled` says what is true -- still held, not saved,
    /// not moving. Memory stays bounded by NEW writes being refused with
    /// `Busy`, never by forgetting ones already accepted.
    /// # What the age is measured AGAINST
    ///
    /// A delegate has no clock. `now` is whatever the last `Tick` carried,
    /// and `in_flight_since` is the `now` of the tick current when the commit
    /// started. So "how long" is measured in the time the OUTSIDE has sent,
    /// and nothing else.
    ///
    /// If no tick has ever arrived, both are 0 and the difference is 0: a
    /// commit CANNOT BE STALLED YET, however long it has really been sitting.
    /// That is the honest answer rather than a guess — the engine has not
    /// been told that any time has passed, and inventing some would make the
    /// report a fiction with teeth.
    fn age_out_accepted(&mut self, now: u64) -> Vec<Effect> {
        if !self.params.bound_accept_age {
            return Vec::new();
        }
        let Some(since) = self.life.started() else {
            return Vec::new();
        };
        if now.saturating_sub(since) < self.params.max_accept_age {
            return Vec::new();
        }
        // Once per write, its flag on the write itself (C4): a notice repeated every tick is noise a caller learns to
        // ignore, and this one matters. Every write of the commit, and every write QUEUED behind the stuck commit too
        // (R-b): taken, not saved, and not moving either.
        let cut: Vec<(ClientId, WriteId)> = self.life.commit().map(|c| c.writes.clone()).unwrap_or_default();
        let mut out = Vec::new();
        for q in self.queue.iter_mut() {
            let w = (q.client, q.write_id);
            if (cut.contains(&w) || q.told_accepted) && !q.stalled_told {
                q.stalled_told = true;
                out.push(Effect::Notify {
                    client: w.0,
                    write_id: w.1,
                    state: State::Stalled,
                });
            }
        }
        out
    }





}

/// Whether an engine's owed parity covers the WHOLE published tree (sdk#119).
///
/// A zero from [`Engine::owed_groups`] means "every group is put" only under
/// `Done`. Under `NotScanned` the engine knows the groups it coded itself and
/// nothing about the tree beneath them — which is every engine that read a
/// non-empty head, adopted another writer's, or was rehydrated. Re-deriving
/// what such a tree owes needs a presence probe; until it lands (sdk#119 PR2)
/// `NotScanned` is the honest answer, and a surface must say it as such.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParityScan {
    /// Nothing is known beyond what this engine coded.
    NotScanned,
    /// This engine has tracked the tree since it was empty: every group of
    /// `root` is confirmed or in `owed`.
    Done { root: Cid },
}



/// The worst case, in context bytes, of everything carried that is not a
/// parked read, at `params`' caps. Each unit's size is MEASURED by encoding
/// one, with the context's own options, rather than written down, so a field
/// added to a unit moves the bound with it.
pub(crate) fn worst_case_fixed_bytes(params: &Params) -> usize {
    use bincode::Options;
    let enc = |n: Result<u64, bincode::Error>| n.map_or(usize::MAX / 16, |n| n as usize);
    let o = || bincode::DefaultOptions::new().with_fixint_encoding();
    let cid: Cid = [0u8; 32];
    let group: ParityIds = [cid; PARITY];

    // The commit in flight, at its caps: data and confirmed ids, one group per
    // block at most, and its few writes and scalars.
    let m = |a: usize, b: usize| a.saturating_mul(b);
    let commit = m(m(2, params.max_commit_blocks), enc(o().serialized_size(&cid)))
        .saturating_add(m(params.max_commit_blocks, enc(o().serialized_size(&group))))
        .saturating_add(512);
    // The parked write: the path it waits on (its ops are in the page's
    // queue, R-b).
    let parked_write = 64 * enc(o().serialized_size(&cid)) + 128;
    // Subscriptions: two keys at their cap, a cid and a few scalars each.
    let subs = m(params.max_subscriptions, m(2, params.max_sub_key.saturating_add(8)).saturating_add(128));
    let asks = m(params.max_asks, asks::ASK_BYTES).saturating_add(8);
    // Scalars, epochs, the told-stalled list for a commit's writes.
    let scalars = 1024;
    [
        commit,
        parked_write,
        subs,
        asks,
        scalars,
    ]
    .into_iter()
    .fold(0usize, usize::saturating_add)
}

/// A write's ops as the tree takes them: a SET in key order, the LAST op on
/// a key winning -- the client's sequence, collapsed the way the SDK does.
fn batch_of(ops: &[(Vec<u8>, Op)]) -> Vec<(Vec<u8>, TreeEdit)> {
    ops.iter()
        .map(|(k, o)| {
            (
                k.clone(),
                match o {
                    Op::Put(v) => TreeEdit::Put(v.clone()),
                    Op::Delete => TreeEdit::Delete,
                },
            )
        })
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .collect()
}

/// A block id's first bytes, for a message: the one short form every message uses.
pub fn short_id(id: &Cid) -> String {
    core_types::hex::encode(&id[..4])
}

fn kind_raw() -> u8 {
    core_types::kind::BlockKind::Raw.byte()
}

impl<B: Blocks> Engine<B> {
    /// A fetched block came back.
    ///
    /// Rule 3: hash-checked against the id it was asked for BEFORE it touches
    /// the warm tree. A block that fails is treated exactly as a miss — it is
    /// not cached, not parsed, and not an error the caller sees, because a
    /// node that sends rubbish must not be able to break a reader that asked
    /// for something real.
    /// `by_node`: the NODE's answer to the GET (`Event::BlockArrived`, WANTED-LIFE E2n); `false` for a LOCAL landing
    /// (a rebuild from parity, `land_rebuilt`: E2r), which serves no audit (W7).
    fn on_arrived(&mut self, id: Cid, bytes: Vec<u8>, by_node: bool) -> Vec<Effect> {
        // A block of a group being repaired (a parity block among them, which
        // no read asks for by itself).
        let mut out = Vec::new();
        if by_node {
            self.parity_watch.remove(&id); // the node HAS it
        }
        if self.wanted.is_slot(&id) {
            out.extend(self.on_repair_block(id, Some(&bytes)));
            if !self.wanted.read_waits_on(&id) && !(by_node && self.wanted.audit_waits_on(&id)) {
                return out;
            }
        }
        if !read::matches_id(&id, &bytes) {
            out.extend(self.on_missed(id));
            return out;
        }
        let mut landed: Vec<Cid> = vec![id];
        // A pack carries many blocks, and one fetch of it can answer several
        // parked reads at once. Its members are checked the same way: a pack
        // is a transport, and the blocks inside are the same blocks with the
        // same ids.
        //
        // The engine does not STORE any of it. The node holds what it fetched
        // (a GET populates its store), and the engine reads through `Blocks`
        // on the next call. All this does is check what arrived and wake
        // whoever was waiting — the checking still matters, because a block
        // that is not what was asked for must not be treated as an answer.
        if freenet_prolly::block_id(pack::PACK_KIND, &bytes) == id {
            for (mid, mbytes) in pack::members(&bytes) {
                if read::matches_id(&mid, &mbytes) {
                    self.reads.in_pack.insert(mid, id);
                    landed.push(mid);
                }
            }
        }

        // The bytes just handed over, usable for the rest of this call. A
        // node that evicts what it serves leaves these as the only copy, and
        // a read that must re-read them through `Blocks` next call cannot
        // finish at all (sdk#34).
        self.arrived.insert(id, bytes.clone());
        if freenet_prolly::block_id(pack::PACK_KIND, &bytes) == id {
            for (mid, mbytes) in pack::members(&bytes) {
                if read::matches_id(&mid, &mbytes) {
                    self.arrived.insert(mid, mbytes.to_vec());
                }
            }
        }

        let mut woken: BTreeSet<read::ReqId> = BTreeSet::new();
        // The audit passes this arrival answers (the node's answer only, A2).
        let mut audited: Vec<Effect> = Vec::new();
        for l in &landed {
            self.reads.attempts.remove(l);
            // The block itself came: a repair rebuilding it is no longer
            // needed, and its group's asks end with it -- otherwise a slot
            // that is also missing would be re-asked for ever for a block
            // already here.
            if self.repairs.contains_key(l) {
                self.end_repair(*l);
            }
            // Only the id the node ANSWERED carries its authority: a member expanded from a pack landed locally (E2r) --
            // the node's answer was for the pack, not for it (the second reviewer on #538).
            let served = self.served(*l, by_node && *l == id);
            if !served.audits.is_empty() {
                audited.push(Effect::AuditAnswered { id: *l, passes: served.audits.into_iter().collect(), verdict: AuditVerdict::Present });
            }
            if let Some(reqs) = served.reads {
                for r in &reqs {
                    // Pinned for as long as this read is parked: it will
                    // re-descend through this block on its next attempt.
                    if let Some(p) = self.reads.parked.get_mut(r) {
                        p.held.extend(landed.iter().copied());
                        if matches!(p.want, read::Want::Delta(_)) {
                            p.delta.seen.extend(landed.iter().copied());
                        }
                        p.arrived = p.arrived.saturating_add(1);
                    }
                }
                woken.extend(reqs);
            }
        }
        out.extend(audited);
        for req in woken {
            // A read answered by eviction above is gone; driving it is a no-op.
            out.extend(self.drive(req));
        }
        // A write parked on a cold path takes the same arrivals, including a
        // block that came inside a pack: the node holds a pack's members
        // under their own ids, so the apply can read them.
        for l in &landed {
            out.extend(self.resume_parked_write(*l));
        }
        // The call is over for these: the engine owns no block bytes across
        // calls, and this is what keeps that true.
        self.arrived.clear();
        out
    }

    /// An attempt ended without an answer.
    ///
    /// Rule 5: re-issued, not waited on. Only when the budget runs out does
    /// the read get an answer — `Unavailable`, which is a reply. A read that
    /// never answers is indistinguishable from a wedged node, and the caller
    /// can do nothing about either.
    fn on_missed(&mut self, id: Cid) -> Vec<Effect> {
        let mut out = Vec::new();
        // REPAIR mode: a watched parity slot the node answered NotFound is owed back (sdk#479).
        if let Some(g) = self.parity_watch.remove(&id) {
            if !self.parity_owed.contains_key(&id) {
                self.parity_counts.0 += 1;
                self.parity_owed.insert(id, g);
                self.lost.insert(id);
            }
        }
        // E3 BY READER KIND: the audits are ANSWERED (absent: their verdict) and taken; every other reader keeps
        // waiting (rule 8).
        let passes = self.wanted.missed_audits(id);
        if !passes.is_empty() {
            if !self.wanted.readers_of(&id).any() {
                self.dropped.insert(id);
            }
            out.push(Effect::AuditAnswered { id, passes: passes.into_iter().collect(), verdict: AuditVerdict::Absent });
        }
        if self.wanted.is_slot(&id) {
            out.extend(self.on_repair_block(id, None));
        }
        out.extend(self.on_missed_parked(id));
        out
    }

    fn on_missed_parked(&mut self, id: Cid) -> Vec<Effect> {
        // A miss is the round the parked write spent. Re-ask rather than wait:
        // the same block may be held by the time the next call runs, and the
        // round bound is what stops this, not the node's willingness to answer.
        //
        // `parked_write` is deliberately NOT cleared first: `park_write` reads
        // the round count off it, so clearing would reset the count on every
        // miss and the bound would never be reached.
        let mut out = Vec::new();
        if let Some(p) = self.parked_write.clone() {
            if self.wanted.write_needs().contains(&id) {
                let needs: Vec<Cid> = self.wanted.write_needs().iter().copied().collect();
                out.extend(self.park_write(p.client, p.write_id, needs));
                out.extend(self.after_park(p.client, p.write_id));
                // RULE 11 FOR A WRITE (sdk#405; rule 4, one read path): the group is an alternative source here as
                // for a read -- a write never waits on a straggler its group can rebuild. The rebuilt block lands
                // like an arrival, and the write resumes on it (`resume_parked_write`).
                out.extend(self.repair_for_write(id));
            }
        }
        // ...and the read path still gets the miss: one block can be the one a
        // parked READ is waiting on as well, and an early return would leave
        // that read waiting on an attempt that already came back.
        out.extend(self.on_missed_read(id));
        out
    }

    /// Start rebuilding `id`, a block a parked WRITE needs, from its group under the tree the write lands on (the
    /// warm root, else the published one) -- the same repair a read starts on a NotFound.
    fn repair_for_write(&mut self, id: Cid) -> Vec<Effect> {
        if !self.params.repair_reads || self.repairs.contains_key(&id) {
            return Vec::new();
        }
        let group = [self.root, self.published_root].into_iter().find_map(|root| self.group_for(root, id));
        group.map(|g| self.start_repair(g, true)).unwrap_or_default()
    }

    fn on_missed_read(&mut self, id: Cid) -> Vec<Effect> {
        let Some(reqs) = self.wanted.reads_on(&id).cloned() else {
            return Vec::new();
        };
        // The node answered NotFound. The readers keep waiting and the block
        // is ASKED AGAIN -- with no count that ends it (the owner's rule 7:
        // retry until answered). The PAGE paces the re-asks on its backoff
        // (the one sender), so a node that answers NotFound at once is not
        // asked at the speed of its answers. A slow or forgetful node is
        // shown as "not answering", never answered `Unavailable`.
        let attempt = {
            let a = self.reads.attempts.entry(id).or_insert(0);
            *a += 1;
            *a
        };
        // A DELTA is an optimisation over a reload, not a read of its own:
        // a block it needs that the node answers NotFound (an answer, never
        // silence -- the page no longer turns silence into a miss) is most
        // likely an old root's, gone for good. It is answered
        // `FullReloadRequired` at once -- the client reloads from the current
        // root, so nothing is lost and live updates do not wait on a vanished
        // root. Every other read keeps waiting.
        let mut out = Vec::new();
        for req in &reqs {
            let is_delta = self.reads.parked.get(req).is_some_and(|p| matches!(p.want, read::Want::Delta(_)));
            if is_delta {
                let p = self.reads.parked.remove(req).expect("just read");
                let result = self.gave_up(&p.want, id);
                self.forget_waiting(*req);
                out.push(Effect::Reply { client: p.client, req_id: *req, result });
            }
        }
        if !self.wanted.read_waits_on(&id) {
            return out;
        }
        let via = self.reads.in_pack.get(&id).copied().map_or(read::Via::Direct, read::Via::Pack);
        self.reads.fetches += 1;
        self.step_asks.insert(id, true);
        out.push(Effect::FetchBlock { id, via, attempt });
        // Meanwhile the group is an ALTERNATIVE source (Phase 4, gap 1): a
        // verified rebuild lands exactly like an arrival.
        if self.params.repair_reads && !self.repairs.contains_key(&id) {
            let roots: Vec<Cid> = reqs.iter().filter_map(|r| self.reads.parked.get(r).map(|p| p.root)).collect();
            let group = roots.into_iter().find_map(|root| self.group_for(root, id));
            if let Some(group) = group {
                out.extend(self.start_repair(group, true));
            }
        } else {
            // The block a race was racing is answered NotFound: the race is a REPAIR now.
            out.extend(self.race_missed(id));
        }
        out
    }

    /// Would a `PutConfirmed(id)` count for anything NOW (sdk#455)? A block of the pending commit not yet confirmed,
    /// an earlier commit's straggler a group of it counts on, another member of a changed group not yet held
    /// (sdk#416), or a published commit's block still owed for BACKED_UP -- read from those records, the ones
    /// `on_confirmed` reads. The page stops asking `Held` about a block nobody waits on.
    pub fn awaits_confirmation(&self, id: &Cid) -> bool {
        self.life.backing_owes(id)
            || self.life.commit().is_some_and(|c| {
                c.owes(id)
                    || c.race.groups.iter().any(|g| g.earlier.contains(id) || (g.others.contains(id) && !c.held.contains(id)))
            })
    }

    /// WHO WANTS a block (sdk#480, the structure sweep's Tier-1 #1): derived from the engine's own indexes, never
    /// stored, and THE one answer to "is this block still wanted" -- a withdrawal, a superseded read, a repair's
    /// "already in flight", a rebuild's PUT-back and the page's parked GET all read it. Before it, two definitions
    /// disagreed (`awaits_block` counted all three; `withdraw_if_unwanted` only reads), so a block the parked write
    /// needed could be withdrawn when a read on it was superseded.
    pub fn readers_of(&self, id: &Cid) -> Readers {
        self.wanted.readers_of(id)
    }

    /// Start rebuilding `group.missing`: what is held already counts, and
    /// every other block of the group is asked for at once (the first `k` to
    /// arrive are enough), through the same `FetchBlock` as any read.
    fn start_repair(&mut self, group: repair::Group, missed: bool) -> Vec<Effect> {
        self.repair_counts.0 += 1;
        let missing = group.missing;
        let mut r = Repair { group, have: BTreeMap::new(), asked: BTreeMap::new(), missed, dropped: BTreeSet::new(), absent: BTreeSet::new() };
        // The missing block's OWN slots -- every one holding its id -- are not asked as group slots, and a miss marks
        // them all absent.
        let own: BTreeSet<usize> = r.group.slots_of(&missing).collect();
        if missed {
            r.absent.extend(&own);
        }
        let mut out = Vec::new();
        for i in 0..r.group.slots.len() {
            if own.contains(&i) {
                continue;
            }
            let slot = r.group.slots[i];
            match self.source().get(&slot) {
                Some(b) if r.group.fits(i, b) => {
                    r.have.insert(i, r.group.stored(i, b));
                }
                _ => {
                    r.asked.insert(i, 0);
                    // ONE ask per block (sdk#303): a slot another member's race, or a read, has in flight already
                    // is not asked twice -- racing every member of a group would otherwise ask each slot k times.
                    let in_flight = self.want(slot, Reader::Repair(missing)).any();
                    if !in_flight {
                        self.reads.fetches += 1;
                        self.step_asks.entry(slot).or_insert(false);
                        out.push(Effect::FetchBlock { id: slot, via: read::Via::Direct, attempt: 0 });
                    }
                }
            }
        }
        self.repairs.insert(missing, r);
        out.extend(self.try_repair(missing));
        out
    }

    /// A group block came back (`Some`, hash-checked here against its slot)
    /// or an attempt at it ended empty (`None`).
    fn on_repair_block(&mut self, slot: Cid, bytes: Option<&[u8]>) -> Vec<Effect> {
        let mut out = Vec::new();
        let for_: Vec<Cid> = self.wanted.repairs_on(&slot);
        let mut still_asked = false;
        for missing in for_ {
            // A repair nobody wants any more ENDS here, rather than asking its group for ever: a write that started
            // it (sdk#405) applied or was refused, and nothing else waits on the block.
            if !self.readers_of(&missing).any() && self.repairs.contains_key(&missing) {
                self.end_repair(missing);
                continue;
            }
            let Some(r) = self.repairs.get_mut(&missing) else { continue };
            // EVERY slot holding this id: one block fills (or misses) them all (sdk#542).
            let ixs: Vec<usize> = r.group.slots_of(&slot).collect();
            for i in ixs {
                match bytes {
                    Some(b) if r.group.fits(i, b) => {
                        let st = r.group.stored(i, b);
                        r.have.insert(i, st);
                        r.asked.remove(&i);
                        r.absent.remove(&i);
                    }
                    // A RACE drops a slot the node does not have (or answered wrong): the read's own block is
                    // still asked, and re-asking parity beside a healthy read is GETs for nothing (sdk#303).
                    _ if !r.missed => {
                        r.asked.remove(&i);
                        r.dropped.insert(i);
                        r.absent.insert(i);
                        // A PARITY slot the node does NOT HAVE is owed back (sdk#479): the read saw it gone.
                        if bytes.is_none() && r.group.is_parity(i) && !self.parity_owed.contains_key(&slot) {
                            self.parity_counts.0 += 1;
                            self.parity_owed.insert(slot, r.group.clone());
                            self.lost.insert(slot);
                        }
                    }
                    _ => {
                        r.absent.insert(i);
                        // Still asked, and asked AGAIN: a group block that is slow
                        // is not a group block that is gone, so no count here
                        // ever gives up on it (the page paces the re-ask).
                        *r.asked.entry(i).or_insert(0) += 1;
                        still_asked = true;
                        // A PARITY slot answered NotFound in a repair is owed back too (sdk#479).
                        if bytes.is_none() && r.group.is_parity(i) && !self.parity_owed.contains_key(&slot) {
                            self.parity_counts.0 += 1;
                            self.parity_owed.insert(slot, r.group.clone());
                            self.lost.insert(slot);
                        }
                    }
                }
            }
            out.extend(self.try_repair(missing));
        }
        if still_asked && self.wanted.is_slot(&slot) {
            self.reads.fetches += 1;
            self.step_asks.entry(slot).or_insert(false);
            out.push(Effect::FetchBlock { id: slot, via: read::Via::Direct, attempt: 1 });
        } else if !still_asked {
            // Nobody asks for this block any more (it came, or it is spent).
            let spent: Vec<Cid> = self
                .wanted
                .repairs_on(&slot)
                .into_iter()
                .filter(|m| !self.repairs.get(m).is_some_and(|r| r.asked.keys().any(|i| r.group.slots[*i] == slot)))
                .collect();
            for m in spent {
                self.drop_reader(slot, Reader::Repair(m));
            }
        }
        out
    }

    /// With `k` of the group held, rebuild and VERIFY. A rebuild that does
    /// not verify ends the REPAIR, named in `repair_failed`, and never the
    /// read: the block itself is still asked for, and the readers wait.
    fn try_repair(&mut self, missing: Cid) -> Vec<Effect> {
        let Some(r) = self.repairs.get(&missing) else { return Vec::new() };
        let k = r.group.k;
        if r.have.len() >= k {
            let have: Vec<Option<Vec<u8>>> = (0..r.group.slots.len()).map(|i| r.have.get(&i).cloned()).collect();
            let rebuilt = repair::rebuild(&r.group, &have);
            self.end_repair(missing);
            return match rebuilt {
                Ok(body) => {
                    self.repair_counts.1 += 1;
                    self.lost.insert(missing);
                    // The page KEEPS it (the node lost it; reads go on from
                    // the page's blocks), PUTs it back, and it lands like any
                    // arrival -- asked BEFORE the arrival, which ends the wait.
                    let out = self.rebuilt(missing, &body);
                    self.landing.push((missing, body));
                    out
                }
                Err(why) => {
                    self.repair_counts.2 += 1;
                    self.repair_failed = Some(format!("block {} could not be repaired: {why}", short_id(&missing)));
                    Vec::new()
                }
            };
        }
        Vec::new()
    }

    fn end_repair(&mut self, missing: Cid) {
        self.repairs.remove(&missing);
        // Its slots lose this reader; a slot nobody wants any more is a candidate for `Unwanted` (sdk#303).
        let slots: Vec<Cid> = self.wanted.slots_of(&missing);
        for slot in slots {
            self.drop_reader(slot, Reader::Repair(missing));
        }
    }

    /// THE WANT (WANTED-LIFE, sdk#480 part 3): `reader` wants `id`. THE one writer, with [`drop_reader`],
    /// [`served`] and [`release_write`], of the three reader indexes (`reads.waiting`, `repair_slots`,
    /// `parked_write.needs`): a source test holds that nothing else writes them. Returns who wanted `id` BEFORE --
    /// each caller's own ask rule reads it (one ask per block, W5).
    ///
    /// [`drop_reader`]: Self::drop_reader
    /// [`served`]: Self::served
    /// [`release_write`]: Self::release_write
    fn want(&mut self, id: Cid, reader: Reader) -> Readers {
        self.wanted.want(id, reader)
    }

    /// THE DROP: `reader` no longer wants `id`. If nobody does now, `id` is a candidate for [`Effect::Unwanted`] at
    /// the end of this call -- decided then, net, never here (the architect's T3).
    fn drop_reader(&mut self, id: Cid, reader: Reader) {
        if self.wanted.drop_reader(id, reader) {
            self.dropped.insert(id);
        }
        // A block no read waits on any more has no attempts to count. Left behind, one entry per block asked for and
        // never answered outlived the read that asked (sdk#187 review, executed).
        if !self.wanted.read_waits_on(&id) {
            self.reads.attempts.remove(&id);
        }
    }

    /// `id` ARRIVED: the readers it serves are TAKEN (WANTED-LIFE A2) -- the reads (they re-descend) and, when the NODE
    /// answered (`by_node`), the audit passes. Not a drop: the block is here, so it is never `Unwanted`.
    fn served(&mut self, id: Cid, by_node: bool) -> wanted::Served {
        self.wanted.served(id, by_node)
    }

    /// An audit pass wants `id` (WANTED-LIFE A1): its reader joins the one (block, GET) key, and the ONE `FetchBlock`
    /// goes when no GET of it is in flight -- EVEN IF `id` is held (W7: the audit measures the node).
    fn on_audit_want(&mut self, id: Cid, pass: PassId) -> Vec<Effect> {
        if self.want(id, Reader::Audit(pass)).any() {
            return Vec::new();
        }
        self.reads.fetches += 1;
        self.step_asks.entry(id).or_insert(false);
        vec![Effect::FetchBlock { id, via: read::Via::Direct, attempt: 0 }]
    }

    /// The parked write is RELEASED (applied, refused, re-queued, out of rounds or room): its needs lose their reader,
    /// and each block nobody else wants is a candidate for `Unwanted` -- before, its GETs ran on until the page's
    /// due-time check. The released write, for its caller.
    fn release_write(&mut self) -> Option<ParkedWrite> {
        self.dropped.extend(self.wanted.release_write());
        self.parked_write.take()
    }

    /// THE WITHDRAWAL, NET, at the end of a call (WANTED-LIFE): every block whose last reader dropped during the call
    /// and that is STILL unwanted ([`readers_of`](Self::readers_of)) and not held is one [`Effect::Unwanted`]. A block
    /// dropped and wanted again within the call (a race frees a slot the parked write then parks on) is not told.
    fn settle_unwanted(&mut self) -> Vec<Effect> {
        let dropped = std::mem::take(&mut self.dropped);
        dropped.into_iter().filter(|id| !self.readers_of(id).any() && self.blocks.get(id).is_none()).map(|id| Effect::Unwanted { id }).collect()
    }

    /// Rule 7: preload is advisory and budgeted.
    ///
    /// It may make a later read SOONER; it may never make one more or wider.
    /// So it is truncated to the budget rather than refused, and it only
    /// walks roots this session already holds — a manifest naming a million
    /// roots costs the budget, not the manifest.
    fn on_preload(&mut self, _client: ClientId, roots: Vec<Cid>) -> Vec<Effect> {
        let mut out = Vec::new();
        // WORK, NOT MISSES.
        //
        // This counted only blocks it had to FETCH, so a block the node
        // already held fell through to `Node::parse`, pushed its children,
        // and cost the budget nothing. On a tree the node holds, the budget
        // could never fire and the walk ran to completion — so the preload
        // did its most work in exactly the case where it had nothing to do,
        // and that case is the common one: every open after the first,
        // against the user's own tree.
        //
        // Measured before this: 8,972 nodes parsed against a budget of 256,
        // in ONE `process()` call, in a wasm delegate, for ZERO fetches
        // (sdk#120). A bound that only the cold path could reach was a bound
        // on the path that did not need one.
        //
        // Both a parse and a fetch are one unit of work now. The cold path is
        // unchanged — a miss still costs one and still stops at the guard.
        let mut work = 0usize;
        let mut bytes = 0usize;
        for root in roots.into_iter().take(self.params.preload_roots) {
            // Only over a root the session already holds: a preload is a hint
            // about what is already ours, not an invitation to fetch a
            // stranger's tree.
            if self.blocks.get(&root).is_none() {
                continue;
            }
            let mut stack = vec![root];
            while let Some(cid) = stack.pop() {
                if work >= self.params.preload_blocks || bytes >= self.params.preload_bytes {
                    return out;
                }
                let Some(b) = self.blocks.get(&cid) else {
                    // Not held: this is what a preload is FOR.
                    if self.wanted.read_waits_on(&cid) {
                        continue;
                    }
                    work += 1;
                    self.reads.fetches += 1;
                    out.push(Effect::FetchBlock {
                        id: cid,
                        via: read::Via::Direct,
                        attempt: 0,
                    });
                    continue;
                };
                // A PARSE IS WORK, and its BYTES are the dimension that
                // actually costs: a few large nodes can be more work than
                // many small ones, which a count alone cannot see.
                // `preload_bytes` was declared, defaulted and read NOWHERE —
                // a parameter nobody reads is a claim, not a bound, and it
                // sat in a settings surface as though it constrained
                // something.
                work += 1;
                bytes += b.len();
                self.nodes_parsed += 1;
                let Ok(n) = Node::parse(b) else { continue };
                if !n.is_leaf() {
                    for i in 0..n.len() {
                        stack.push(n.child(i).0);
                    }
                }
            }
        }
        out
    }

    /// A parked DELTA's established prefix and resume key (sdk#135): only for tests that check every step is a
    /// correct prefix of the true diff.
    pub fn delta_prefix_for_test(&self, req_id: read::ReqId) -> Option<DeltaPrefix> {
        self.reads.parked.get(&req_id).map(|p| (p.delta.after.clone(), p.delta.acc.clone()))
    }

    /// Start from a published root this engine did not write.
    ///
    /// What a cold reader has: a head, and nothing else. Only for tests — a
    /// real engine learns its root by reading its own head.
    pub fn adopt_root_for_test(&mut self, root: Cid) {
        self.root = root;
        self.published_root = root;
        // A reader that HAS its head is recovered: its reads are answered, not
        // held for a head read that already happened (sdk#223).
        self.recovered = true;
    }

    /// The block source this engine reads through.
    pub fn blocks(&self) -> &B {
        &self.blocks
    }

    /// The block store, to add to or evict from BETWEEN steps (sdk#411): the engine holds no borrow of it across
    /// calls, and with an owned store that is the borrow checker's fact, not a comment's.
    /// Re-puts that found their block's bytes gone from the store (sdk#411): always 0 when the pin rule holds.
    pub fn reput_missing(&self) -> u64 {
        self.reput_missing
    }

    /// The parked write's owner is LIVE: it is still the first `Applying` write in the queue (sdk#411, the
    /// architect's twin property "no pin outlives its owner": `ParkedWrite::held` pins only while this holds).
    pub fn parked_write_is_live(&self) -> bool {
        let Some(p) = self.parked_write.as_ref() else { return true };
        self.queue.iter().find(|q| q.warm_after.is_none()).is_some_and(|q| (q.client, q.write_id) == (p.client, p.write_id))
    }

    /// Is a write queued or a commit pending? (The page's warm-apply blocks are the only copy while one is.)
    pub fn has_writes_in_flight(&self) -> bool {
        self.life.commit().is_some() || !self.queue.is_empty()
    }

    pub fn blocks_mut(&mut self) -> &mut B {
        &mut self.blocks
    }

    /// THE ONE PIN RULE (sdk#411): every block whose page copy may NOT be dropped, and why -- the engine's own facts
    /// and the page's (`page`). Everything else in the store may be: its bytes are on the node, or nothing
    /// references them, and a later need fetches it. The states are the design's table (sdk#411): W, C, B, P, PW,
    /// HF (R was measured to need no pin: [`PagePins`]). Each class's reader, and the mutant that proves it, are
    /// named on [`Pin`]. Listed once per eviction pass, never per block.
    pub fn pins(&self, page: &PagePins<'_>) -> BTreeMap<Cid, Pin> {
        let mut out = BTreeMap::new();
        // The later a class is listed, the more it says: a block in two classes keeps the first reason.
        let mut pin = |id: &Cid, why: Pin| {
            out.entry(*id).or_insert(why);
        };
        if let Some(c) = self.life.commit() {
            for id in c.owed() {
                pin(id, Pin::InFlight);
            }
        }
        for id in &self.life.unacked() {
            pin(id, Pin::Backing);
        }
        for id in self.wanted.read_blocks().chain(self.reads.parked.values().flat_map(|p| p.held.iter())) {
            pin(id, Pin::ParkedRead);
        }
        if let Some(p) = self.parked_write.as_ref() {
            for id in p.held.iter().chain(self.wanted.write_needs().iter()) {
                pin(id, Pin::ParkedWrite);
            }
        }
        if self.has_writes_in_flight() {
            for id in self.kept.keys() {
                pin(id, Pin::Warm);
            }
        }
        for id in page.held_asks {
            pin(id, Pin::HeldForeign);
        }
        out
    }

    /// [`Engine::pins`] for one block.
    pub fn pinned(&self, id: &Cid, page: &PagePins<'_>) -> Option<Pin> {
        self.pins(page).get(id).copied()
    }

    /// What the engine reads through: the node, plus the one constant it can
    /// answer for itself.
    fn source(&self) -> WithEmptyLeaf<'_, B> {
        WithEmptyLeaf {
            inner: &self.blocks,
            empty_cid: self.empty.cid,
            empty_bytes: &self.empty.bytes,
            arrived: &self.arrived,
        }
    }

    /// Has the head been recovered? Before it, the only tree is the empty
    /// one, and a walk of it would answer "nothing here" for a tree nobody
    /// has read (sdk#223).
    pub fn recovered(&self) -> bool {
        self.recovered
    }

    /// WALK the tree at `root` over this engine's blocks, now, fetching
    /// nothing (READ-STATE, design B). The descent is `read::attempt`, the
    /// same one a parked read makes, so a reader and the engine cannot
    /// disagree about what a tree holds.
    pub fn walk(&self, root: &Cid, walk: &read::Walk) -> read::Walked {
        let want = match walk {
            read::Walk::Get(k) => read::Want::Get(k.clone()),
            read::Walk::Scan(s) => read::Want::Scan(s.clone()),
            read::Walk::Delta(d) => read::Want::Delta(d.clone()),
        };
        let mut parsed = 0;
        match read::attempt(&self.source(), &self.params, &want, root, &mut parsed) {
            read::Attempt::Done(r) => read::Walked::Done(r),
            read::Attempt::Need(ids) | read::Attempt::NeedFrom { ids, .. } => read::Walked::Need(ids),
            read::Attempt::Broken(c) => read::Walked::Broken(c),
        }
    }

    /// Fetches emitted, for the cost gate.
    pub fn fetches(&self) -> usize {
        self.reads.fetches
    }

    /// Asks that went out since this engine began: `(wanted, raced)` ([`Engine`]'s `fetch_counts`).
    pub fn fetch_counts(&self) -> (usize, usize) {
        self.fetch_counts
    }
}

/// A tick more than this after the last one, or before it, is a different
/// clock, not the same one moving on: a delegate context lives 600 s (F32),
/// so no engine sees a real gap longer than that. A smaller step BACK is
/// another tab's or device's clock, and is ignored. Ticks are seconds
/// (`protocol::tick_of`).
const CLOCK_RESET_TICKS: u64 = 600;

/// The header the delegate-era context began with (magic 4 + version 2 +
/// checksum 8). No context is written any more (#305); the context BUDGET
/// still counts it until sdk#385 decides that budget.
const CONTEXT_HEADER: usize = 4 + 2 + 8;

/// Visits each field of [`Engine::state_fields`]: sized by `context_len`,
/// hashed by `state_digest`.
trait StateVisitor {
    fn field<T: serde::Serialize>(&mut self, v: &T);
}

impl<B: Blocks> Engine<B> {
    /// The size the delegate-era context would have been, header included
    /// (#305 deleted the carry; `keep_saveable` still bounds by it, sdk#385).
    ///
    /// Sized field by field FROM THE LIVE STATE, never by building the
    /// `Context` -- that clones every collection (a commit's pack bodies
    /// among them) and, run at the end of every step, made the engine's own
    /// suites ~4x slower (sdk#162 review, measured). With fixint bincode a
    /// map encodes exactly as a sequence of its pairs and a set as a sequence,
    /// so the sum was exactly the length the carry wrote (checked every step
    /// while the carry existed).
    pub fn context_len(&self) -> usize {
        struct Size(usize);
        impl StateVisitor for Size {
            fn field<T: serde::Serialize>(&mut self, v: &T) {
                use bincode::Options;
                let o = bincode::DefaultOptions::new().with_fixint_encoding();
                self.0 += o.serialized_size(v).map_or(usize::MAX / 64, |n| n as usize);
            }
        }
        let mut size = Size(0);
        self.state_fields(&mut size);
        CONTEXT_HEADER + 2 + size.0
    }

    /// BLAKE3 over the bytes of every field [`Engine::state_fields`] walks, in
    /// its order, each prefixed with its length: the bookkeeping, BYTE FOR
    /// BYTE, so a refusal test can assert nothing moved -- whatever it grows
    /// later, since a new field joins the one walk. Only those tests call it;
    /// `context_len` sizes the same walk every step without encoding it.
    #[doc(hidden)]
    pub fn state_digest(&self) -> [u8; 32] {
        struct Digest(blake3::Hasher);
        impl StateVisitor for Digest {
            fn field<T: serde::Serialize>(&mut self, v: &T) {
                use bincode::Options;
                let o = bincode::DefaultOptions::new().with_fixint_encoding();
                match o.serialize(v) {
                    Ok(b) => {
                        self.0.update(&(b.len() as u64).to_le_bytes());
                        self.0.update(&b);
                    }
                    // Never for these types; a field that cannot encode is
                    // still a DIFFERENT digest from one that can.
                    Err(_) => {
                        self.0.update(&u64::MAX.to_le_bytes());
                    }
                }
            }
        }
        let mut d = Digest(blake3::Hasher::new());
        self.state_fields(&mut d);
        *d.0.finalize().as_bytes()
    }

    /// THE ONE LIST of the engine's bookkeeping fields, in one fixed order:
    /// what the delegate-era context carried. `context_len` (the budget,
    /// sdk#385) sizes it and `state_digest` hashes it, so the two cannot
    /// disagree about what the state is.
    fn state_fields(&self, v: &mut impl StateVisitor) {
        let carries = self.params.context_carries_pending;
        v.field(&(self.published_seq, self.published_root, self.root, self.next_seq));
        if carries {
            v.field(&self.life.commit());
        } else {
            v.field(&None::<Commit>);
        }
        v.field(&self.asks.to_vec());
        v.field(&self.reads.parked);
        v.field(self.wanted.waiting());
        v.field(&self.reads.attempts);
        v.field(&self.head_epoch);
        v.field(&self.parked_write);
        v.field(self.wanted.write_needs());
        v.field(&self.subs);
        if carries {
            v.field(&self.life.started());
            v.field(&self.stalled_told_writes());
        } else {
            v.field(&None::<u64>);
            v.field(&Vec::<(ClientId, WriteId)>::new());
        }
        v.field(&self.now);
    }
}
