//! What the SDK and the engine say to each other.
//!
//! **Every message starts with a version, from the very first one.** Not
//! because v2 is planned, but because a protocol that has to be versioned
//! LATER cannot be: the messages already in flight have nowhere to put the
//! number, and the only way to tell them apart is to guess from their shape.
//! ARCHITECTURE §19 — every published version keeps working — is a promise
//! that can only be kept by something that could always tell versions apart.
//!
//! Three rules follow, and each has a test:
//!
//! 1. A version this build does not know is answered [`Reply::Unsupported`],
//!    listing what it does know. Never silence, and never a guess at what the
//!    sender meant.
//! 2. Decoding is EXACT. `bincode::deserialize` accepts trailing bytes — it
//!    decodes the type it was asked for and ignores the rest — which is how
//!    one message is read as another. A wrapped value decoded as its wrapper
//!    returns the wrapper's first variant and throws the answer away.
//! 3. Records are append-only and unknown fields are PRESERVED byte for byte
//!    (see [`Record`]). An SDK that rewrites a record it did not fully author
//!    must not silently delete what a newer build put there.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub mod record;
pub mod session;

/// The versions this build can serve.
///
/// A list, not a number: "the current version" is what a protocol says right
/// before it drops an old client. Serving several is the normal state.
pub const KNOWN: &[u16] = &[1, 2, 3, 4, 5];

/// The version this build SPEAKS when it starts a conversation.
///
/// v2 adds [`Reply::CallBytes`]. It is a new VARIANT, not a new field on an
/// existing message, so a v1 reader meeting it fails to decode and counts
/// `Dropped::Unparseable` rather than reading it as something else — which is
/// what appending a field would have done.
///
/// # What a reader does with a version it does not know
///
/// A REQUEST carries its version and is answered [`Reply::Unsupported`] with
/// the list this build serves, so a newer client can fall back rather than
/// wait.
///
/// A REPLY carries no version — there is no envelope on the reply side — so
/// the only protection a reply has is its variant tag. That is why a new
/// message is added as a variant and never as a field, and why the delegate
/// sends v2-only messages ONLY to a client that said it speaks v2. A v1
/// client is never sent one at all, so its decoder never has to refuse one.
///
/// v3 adds [`WriteState::TooLarge`] — again a new variant, sent only to a
/// client that spoke v3; an older one is told [`WriteState::Failed`], which
/// means the same thing to it (not in the tree; sending it again is refused
/// again) without a reason it could not read.
///
/// v4 puts the SESSION on every request and adds [`Reply::SessionWriteState`]
/// (craftworks-sdk#146). Every tab and page load was `ClientId(1)` and its
/// first write `WriteId 1`, so the engine's `(client, write)` keys collided by
/// default. The delegate receives a bare frame with no connection behind it —
/// every tab shares one context — so a session said once, on `Identity`, would
/// be overwritten by the next tab's; it rides on each frame instead. A v4
/// client is told its writes' states with the session named, so a tab can
/// drop a state for somebody else's write.
///
/// v5 is the WRITE PATH's wire (WRITE-PATH.md revision 3, craftworks-sdk#183).
/// Its envelope carries the client's `now` (wall-clock ms) and a per-session
/// FRAME sequence on every request; a write carries its session's `floor`
/// ([`Request::WriteFrom`]); every reply to a v5 client is wrapped with the
/// session's [`Ack`] ([`Reply::Acked`]); and two verdicts are new —
/// [`WriteState::Duplicate`] and [`WriteState::OutOfOrder`]. v5 is KNOWN
/// (decodable) and not yet CURRENT: it becomes current when a client and an
/// engine both speak it. Types only — no behaviour lives here.
pub const CURRENT: u16 = 4;

/// The first version whose envelope carries `now` and a frame sequence, whose
/// write carries a floor, and whose replies carry an [`Ack`].
pub const FLOOR_SINCE: u16 = 5;

/// The session a message from before v4 is taken to be: the one every client
/// was, before there were sessions. An old page still works; it is simply
/// the one session that cannot be told apart from another old page.
pub const LEGACY_SESSION: u64 = 1;

/// The first version whose envelope carries a session.
pub const SESSION_SINCE: u16 = 4;

/// How many bits of a session are significant: 48. The delegate keys a
/// write by `{session, version}` packed into the engine's one 64-bit client
/// id, so the version rides with the write wherever the write goes — a
/// parked write, a commit in flight — with no second table beside it.
pub const SESSION_BITS: u32 = 48;

/// A session from 64 random bits: the low 48, never zero or the legacy one.
pub fn mint_session(random: u64) -> u64 {
    let s = random & ((1u64 << SESSION_BITS) - 1);
    if s == LEGACY_SESSION || s == 0 { LEGACY_SESSION + 1 } else { s }
}

/// Whether a v4 frame's session is one a client can hold.
pub fn session_is_valid(session: u64) -> bool {
    session != 0 && session != LEGACY_SESSION && session < (1u64 << SESSION_BITS)
}

/// A client's message, with its version on the front.
///
/// On the wire the version is always first; from v4 the session follows it
/// (see [`encode_session_request`]). Decoded, an older message carries
/// [`LEGACY_SESSION`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// Which protocol version the body is in. First field, always.
    pub version: u16,
    /// Which page load sent it: minted at random by the client, carrying
    /// nothing of the person or of any key.
    pub session: u64,
    /// The client's wall clock, in ms, when it sent this (v5). `None` means
    /// THIS VERSION DOES NOT CARRY IT — never "time zero": nothing may read it
    /// as 0 (`unwrap_or(0)`), or every pre-v5 frame would say it is 1970.
    pub now_ms: Option<u64>,
    /// This session's frame sequence: 1, 2, 3… on every request it sends
    /// (v5), so "my frame was seen" is a fact read off the next [`Ack`].
    /// `None` means this version does not carry it — never 0, which a v5
    /// frame may not use.
    pub frame: Option<u64>,
    pub body: Request,
}

/// The v4 envelope, as it is on the wire: the version, the session, the body.
#[derive(Serialize, Deserialize)]
struct EnvelopeV4 {
    version: u16,
    session: u64,
    body: Request,
}

/// The v5 envelope, as it is on the wire.
#[derive(Serialize, Deserialize)]
struct EnvelopeV5 {
    version: u16,
    session: u64,
    now_ms: u64,
    frame: u64,
    body: Request,
}

/// The envelope before v4: the version, then the body.
#[derive(Serialize, Deserialize)]
struct LegacyEnvelope {
    version: u16,
    body: Request,
}

/// What a client asks the engine to do.
///
/// APPEND ONLY. A variant's position is its wire tag, so inserting one
/// renumbers every variant after it and silently turns old messages into
/// different new ones. New requests go at the END, and a version that needs
/// to change an existing one is a new VERSION.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Who am I talking to?
    ///
    /// The first thing a client should send, and the only one whose answer
    /// is about the engine rather than the data: which build, where its
    /// authority came from, and where its head is.
    Identity,
    Get {
        req_id: u64,
        key: Vec<u8>,
    },
    /// A page of a range, as a KEY continuation.
    ///
    /// The cursor is a key and not an offset: an offset into a tree that is
    /// being written to means a page that skips or repeats rows, and the
    /// caller has no way to know which.
    Range {
        req_id: u64,
        lo: Bound,
        hi: Bound,
        reverse: bool,
        /// Continue AFTER this key. `None` starts at the end `reverse` picks.
        after: Option<Vec<u8>>,
        /// How many entries one page may hold. See [`MAX_PAGE_ENTRIES`].
        ///
        /// Clamped by the engine, which reports what it used.
        max_entries: u32,
    },
    Write {
        /// CLIENT-chosen, so a re-submission is recognisable as the same
        /// write rather than a second one.
        write_id: u64,
        ops: Vec<Op>,
    },
    /// What happened to write N?
    AskWrite {
        write_id: u64,
    },
    /// Advisory: these roots are about to be read. Budgeted, never promised.
    Preload {
        roots: Vec<[u8; 32]>,
    },
    /// Tell me when these writes change state.
    Subscribe {
        write_ids: Vec<u64>,
    },
    /// The client is going away. Ship what is waiting.
    Flush,
    /// Time, from a connected client. It may decide WHEN, never WHAT.
    ///
    /// `now` is **whole seconds since the Unix epoch** — the wall clock
    /// quantised by [`TICK_MS`], not a count of how many times a timer
    /// fired. See [`TICK_MS`] for why the distinction is the whole design.
    Tick {
        now: u64,
    },
    /// Give the engine what it cannot produce for itself.
    ///
    /// A delegate cannot fabricate a contract — it has no way to produce wasm
    /// — and it cannot mint authority. These arrive once and are kept in the
    /// node's secret store, which survives a restart where the context does
    /// not.
    ///
    /// APPENDED, at the end of v1's vocabulary. A variant's position is its
    /// wire tag, so this is the only place a new request can go without
    /// renumbering the ones before it and silently turning old messages into
    /// different new ones. The recorded v1 session is what proves it did not.
    Install {
        block_code: Vec<u8>,
        register_code: Vec<u8>,
        register_params: Vec<u8>,
        /// TEST ONLY today: generated per run by a driver, never read from
        /// disk. A named type rather than bare bytes, because the danger is
        /// not that a test key leaks — it is that a REAL one arrives here
        /// and nothing notices.
        signing_key: TestKey,
    },
    /// Tell me when anything in this KEY RANGE changes.
    ///
    /// **For live data only.** A subscription costs context, a diff on every
    /// root move and a push the client must handle, continuously, and it is
    /// worth that only where the data changes while someone is looking and
    /// the change is small against what it changed. Ordinary data is read
    /// when it is wanted and is correct then; subscribing to it spends a slot
    /// the live data needed. Nothing above this layer takes a subscription
    /// out on a client's behalf.
    ///
    /// A separate request from [`Request::Subscribe`] rather than a field
    /// added to it. A variant's position is its wire tag and its FIELDS are
    /// its body, so adding `lo`/`hi` to `Subscribe` would leave the tag alone
    /// and change what follows it — a v1 client's `Subscribe` would decode as
    /// a range subscription over whatever the next bytes happened to be. The
    /// recorded v1 session is what proves this did not happen.
    SubscribeRange {
        /// CLIENT-chosen, so `Unsubscribe` and every `Changed` name the same
        /// subscription without the client having to learn an id the engine
        /// invented.
        sub_id: u64,
        lo: Bound,
        hi: Bound,
    },
    /// Stop telling me. Unknown ids are not an error: a client dropping a
    /// subscription it already lost should not have to find out first.
    Unsubscribe {
        sub_id: u64,
    },
    /// Emit a call tree as operations happen, or stop.
    ///
    /// **Asked for in advance, never afterwards.** A delegate gets a fresh
    /// linear memory on every call (F32), so there is nothing to ask about
    /// once an operation is over — a trace held until it finished is a trace
    /// that does not survive the thing it describes. So a client turns
    /// emission ON and the steps arrive as they happen; assembling them into
    /// a tree, and timing them, is the client's job.
    Trace {
        on: bool,
    },
    /// What changed in this range since the root I last saw?
    ///
    /// **No subscription is involved.** A delta is a capability of the tree,
    /// available to any reader holding a root: an ordinary reload uses it, and
    /// a binding that is not live uses it too. Being TOLD is the separate,
    /// declared thing, and all it does is trigger this sooner.
    ///
    /// **Per TREE.** A view assembled over several device trees is NOT the
    /// union of their per-tree deltas — a tombstone in one tree can REVEAL an
    /// older value in another, which no per-tree diff mentions. Phase 3 has
    /// one tree per identity; this is stated so nothing later assumes the
    /// union is valid.
    ChangesSince {
        req_id: u64,
        /// The root the CLIENT last saw. The client owns this, not the
        /// engine — which is what makes a missed notification recoverable:
        /// the gap closes in one call however many were dropped.
        from: [u8; 32],
        lo: Bound,
        hi: Bound,
        max_entries: u32,
    },
    /// A write, with the lowest write id this session still holds un-ended
    /// (v5 — WRITE-PATH.md "The rule"). The ONLY write shape in a v5 frame:
    /// a plain [`Request::Write`] there is refused at decode
    /// ([`Dropped::WrongWriteShape`]), as this one is below v5.
    WriteFrom {
        write_id: u64,
        floor: u64,
        ops: Vec<Op>,
    },
    /// A write that says what it READ (M2, craftworks-sdk#148): the engine
    /// checks every read against the tree the ops land on, in the same apply,
    /// and a read that no longer holds applies NOTHING and ends
    /// [`WriteState::Conflict`] with [`Reply::Conflicted`] naming the key.
    ///
    /// `Db` declares a read for every key it writes (the record it patched or
    /// found absent, and the domain's schema), so `writes ⊆ keys(reads)` holds
    /// for everything it sends; the engine enforcing it is sdk#235.
    ///
    /// Sent to [`page::Server`](../page/server/index.html) at v4: that server
    /// answers only the `Db` compiled into the SAME wasm bundle, so client and
    /// server can never be from different builds on this path — the "a new
    /// message only to a client that spoke vN" rule guards against build
    /// skew, and here there is none (main's ruling on sdk#225). The Shell
    /// takes it too; it runs the same engine.
    Commit {
        write_id: u64,
        reads: Vec<(Vec<u8>, Expect)>,
        ops: Vec<Op>,
    },
    /// A [`Request::Commit`] that commits only IN COMPANY (sdk#350): the
    /// engine holds it until a non-deferred write is queued with it, and it
    /// rides that write's cut -- so opening an app, whose schema defines are
    /// deferred, commits nothing. A VARIANT, not a field on `Commit`: the
    /// wire is bincode-exact, and a field would make every frame an older
    /// build sends undecodable (this module's rule: messages are appended as
    /// variants). The engine sees ONE write event either way, with a flag.
    DeferredCommit {
        write_id: u64,
        reads: Vec<(Vec<u8>, Expect)>,
        ops: Vec<Op>,
    },
}

impl Request {
    /// A FORCED write: a [`Request::Commit`] reading every op key as
    /// [`Expect::Any`] — "I write this key whatever it holds" (sdk#235, W8).
    /// The ONE way a write that does not depend on what was there is sent, so
    /// the engine can count it; a reads-less [`Request::Write`] is refused as
    /// [`WriteState::Unread`].
    pub fn forced_write(write_id: u64, ops: Vec<Op>) -> Request {
        let reads = ops
            .iter()
            .map(|o| match o {
                Op::Put(k, _) | Op::Delete(k) => (k.clone(), Expect::Any),
            })
            .collect();
        Request::Commit { write_id, reads, ops }
    }
}

/// What a [`Request::Commit`] READ at a key, as the engine checks it
/// (`engine::Expect`, the same four cases).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expect {
    /// The key is not in the tree (a create).
    Absent,
    /// The key is in the tree, whatever it holds (a child's parent).
    Present,
    /// The key holds exactly this value: the engine's `leaf_hash` of the
    /// bytes read — the LEAF form, inline or by reference, never a hash the
    /// client made up of its own.
    Value([u8; 32]),
    /// "I write this key whatever it holds" — the FORCED form, named so it is
    /// countable (sdk#235, W8). TRANSITIONAL: only a store-level batch that
    /// cannot read first sends it; `Db` refuses to build one; sdk#281 removes
    /// it once the count is zero.
    Any,
}

/// Which id space a trace question is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceOf {
    Write(u64),
    Read(u64),
}

/// WHICH HEAD AN ANSWER WAS READ AT.
///
/// Carried by every reply that hands over data or names where the tree
/// stands, so a client can tell whether two answers describe the same tree.
///
/// **`seq` is what orders them; `root` is what identifies one.** Roots are
/// content hashes and have no order at all — "is this newer" cannot be asked
/// of two roots, only of two seqs — so a client compares seq to decide
/// staleness and root to decide sameness. A design that compared roots would
/// look like it worked until two of them arrived out of order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct At {
    pub seq: u64,
    pub root: [u8; 32],
}

/// The most entries one page of a range may hold.
///
/// **A caller that wants a full page asks for THIS**, never `0` and never a
/// literal. A literal in a client goes stale the moment the engine's own
/// limit moves, and the two then disagree about what a full page is.
///
/// # `0` is not "no limit"
///
/// It reads like one, and it is not. The delegate shell clamps a request with
/// `clamp(1, MAX_PAGE_ENTRIES)`, so `0` becomes **one entry** — a range of N
/// rows then loads in N round trips of a single row each, which at roughly
/// 15 ms per delegate call (F32) is fifteen seconds for a thousand rows.
/// Correct, and crawling. The engine reached through other paths treats it as
/// unlimited instead, so the two readers disagree, and a caller cannot be
/// right about a value two readers interpret differently.
///
/// So `0` means "the engine decides", it is not what a loader should send,
/// and this constant is what it should send instead.
///
/// # It also bounds a PROOF (sdk#15)
///
/// `max_entries` comes off the wire, and `verify_range`'s cost-to-refuse bound
/// is `2 * height + max_entries` blocks. So the number a caller sends decides
/// how much a CHECKER must hash to discover it was lied to — the hostile-cost
/// shape, where the asker pays nothing and the checker pays everything. An
/// unclamped 60,000-entry request licenses a ~60,000-block proof.
///
/// At 256 entries that is at most ~266 blocks, so under 4.5 MiB hashed against
/// the 16 KiB node limit — an amount a phone can afford to refuse. The bound
/// is therefore not only about round trips: **raising this constant raises
/// what a hostile peer can make a client compute**, and that has to be part of
/// the decision, not discovered afterwards by whoever adds the first
/// proof-backed reader.
pub const MAX_PAGE_ENTRIES: u32 = 256;

/// A signing key that is not a real one, and says so in its own type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestKey(pub Vec<u8>);

/// One end of a range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bound {
    Unbounded,
    Included(Vec<u8>),
    Excluded(Vec<u8>),
}

/// What a client asked to change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    Put(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
}

/// What the engine says back. APPEND ONLY, for the same reason as `Request`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    /// This build cannot serve that version, and here is what it can.
    ///
    /// The whole point of versioning from message one: a client that hears
    /// this knows to fall back, where silence would leave it waiting and a
    /// guess would leave it wrong.
    Unsupported {
        got: u16,
        known: Vec<u16>,
    },
    Identity {
        /// What this engine build calls itself.
        engine: String,
        /// Where its authority came from — never the key.
        key_source: String,
        head_seq: u64,
        head_root: [u8; 32],
        /// Whether this delegate can sign and publish a head — i.e. whether
        /// it has been provisioned.
        ///
        /// The head's CONTRACT INSTANCE ID, or all zeroes if there is none.
        ///
        /// A public address, not a secret: it is what any node routes on, and
        /// it says nothing about the key that signs the head.
        ///
        /// Without it a page cannot subscribe to its own head, and a tab that
        /// made no write can only poll — an engine-originated push returns to
        /// whoever invoked the delegate (F40), so the client API is the only
        /// way to be TOLD. The delegate has always derived this to tell a
        /// head read-back from a block; it simply never crossed.
        ///
        /// Zero exactly when `head_writable` is false: both come from having
        /// the Register's code and parameters.
        head_id: [u8; 32],
        /// PRESENCE ONLY: it says that a key is there, never which, and
        /// nothing derived from it. A page reads this to decide whether to
        /// install; `false` with a non-zero `head_seq` cannot happen and
        /// would mean the store lost a secret the engine had already used.
        head_writable: bool,
    },
    /// `Install` arrived at a delegate that is already provisioned, and
    /// NOTHING was changed.
    ///
    /// The ordinary outcome of a race, not an error. Two tabs opened together
    /// on a fresh node both find it unprovisioned and both install; the first
    /// one to arrive wins and the second is told this. Were the second to
    /// overwrite, it would mint a second signing key, move the head's
    /// contract id, and orphan everything the first had already written.
    ///
    /// Replacing a key or the contract code is the hand-over design in
    /// sdk#14, and is never a blind overwrite.
    AlreadyInstalled,
    Value {
        req_id: u64,
        value: Option<Vec<u8>>,
    },
    Page {
        req_id: u64,
        entries: Vec<(Vec<u8>, Vec<u8>)>,
        /// Pass back as `after`. `None` means the range is exhausted.
        cursor: Option<Vec<u8>>,
        /// What the engine actually used, which may be less than asked.
        max_entries: u32,
        /// The head this page was read at.
        ///
        /// Without it a page is rows and nothing else, and a range assembled
        /// from several pages can be STITCHED FROM TWO TREES without anyone
        /// being able to tell. One delegate context serves every tab (F47),
        /// so another tab writing between page 1 and page 2 is ordinary, not
        /// rare.
        at: At,
    },
    /// How far along a write is.
    WriteState {
        write_id: u64,
        state: WriteState,
    },
    /// A read could not be answered. A reply, not a silence.
    Unavailable {
        req_id: u64,
        blocked_on: [u8; 32],
    },
    /// Something arrived that this build could not use. Counted, never
    /// silent: a client talking in a format the engine cannot read would
    /// otherwise wait for ever for an answer to a message never understood.
    Dropped {
        reason: Dropped,
    },
    /// What one engine call DID. Appended to v1.
    ///
    /// Part of the protocol rather than a side channel, because an engine
    /// hosted in a delegate has no log anyone can read and the node prints
    /// nothing about it — so without this, five different breaks in the
    /// write path all present as the same thing: a write that stops at
    /// `Accepted`. Every field is a count the engine already had, and it
    /// found all five.
    Call {
        saw: Saw,
        /// Effects the core returned.
        effects: u32,
        /// Node operations issued.
        ops: u32,
        /// Blocks put and not yet read back.
        awaiting: u32,
        /// Puts confirmed by reading them back.
        read_back: u32,
        /// Effects still queued at the end of the call, and therefore LOST.
        /// Must be zero.
        stranded: u32,
        /// Inbound messages this build could not use.
        dropped: u32,
        /// What the head bump cost, by route: a PUT carries the contract's
        /// CODE, an UPDATE names it by id.
        head_put: u32,
        head_update: u32,
        /// What the node said, when it said anything.
        note: String,
    },
    /// Something in a subscribed range changed. Appended to v1.
    ///
    /// PUSHED, not answered: no client asked for this message, which is the
    /// whole point — a list that refreshes itself is a list nobody polls for.
    Changed {
        sub_id: u64,
        /// Where the tree is NOW. That is all.
        ///
        /// No key list and no previous root. The CLIENT owns "the last root I
        /// saw" and passes it to [`Request::ChangesSince`], which is what
        /// makes a dropped notification self-healing: the next one, or the
        /// client's own backstop, diffs across the whole gap in one call.
        ///
        /// It matters because delivery is lossy by construction — the node's
        /// notification channel drops when full, and a subscription can be
        /// evicted without anyone being told — so a client that missed some is
        /// the ORDINARY case. A payload here would also raise the drop rate
        /// for every other subscriber, to save a round trip that is local.
        new_root: [u8; 32],
        seq: u64,
        /// How the engine knows. See [`Why`].
        why: Why,
    },
    /// What changed since the root the client named. Appended to v1.
    Delta {
        req_id: u64,
        /// The head these changes bring the reader to.
        at: At,
        /// `None` as a value is a REMOVAL, not an empty value. A client told
        /// an empty value keeps a key the writer deleted.
        changes: Vec<(Vec<u8>, Option<Vec<u8>>)>,
        cursor: Option<Vec<u8>>,
        /// Where the client now stands, if it applies these.
        new_root: [u8; 32],
    },
    /// The delta could not be computed; read the range in full instead.
    ///
    /// **A defined answer, not an error.** The old root's blocks may be gone
    /// — superseded, holding no demand, evicted — and for a client that was
    /// away a while that is ordinary rather than a failure. Naming the
    /// fallback here means the degraded path is one thing that gets tested,
    /// instead of something every caller re-invents.
    FullReloadRequired {
        req_id: u64,
        new_root: [u8; 32],
        /// The head the reader should expect to reach by loading again.
        at: At,
    },
    /// A subscribe was taken, or refused and why. Appended to v1.
    Subscribed {
        sub_id: u64,
        accepted: Accepted,
    },
    /// One step of an operation's call tree. Appended to v1.
    ///
    /// Emitted INCREMENTALLY, as it happens — never accumulated and sent at
    /// the end. A delegate gets a fresh linear memory every call (F32), so a
    /// tree held in memory until an operation finishes is a tree that does
    /// not survive the operation it describes.
    ///
    /// There is no duration here. The engine has no clock — it is sans-IO by
    /// construction — and a number it invented would be worse than none.
    /// The CLIENT stamps each step as it arrives.
    Step {
        /// Which operation this belongs to.
        of: TraceOf,
        /// Depth in the tree: 0 is the operation itself.
        depth: u8,
        /// What this step is. A short fixed vocabulary, not free text: a
        /// trace a client has to parse English out of is a log.
        what: Step,
        /// A count whose meaning depends on `what` — blocks, bytes, rows.
        n: u64,
    },
    /// What one call handed to the NODE, in bytes and ops.
    ///
    /// A new VARIANT rather than new fields on [`Reply::Call`], and the
    /// difference is the whole safety argument. Appending a field changes an
    /// existing variant: a reader that does not know about it does not error,
    /// it reads a DIFFERENT MESSAGE — the same class of silent
    /// re-interpretation as a data file whose meaning changed while every
    /// field stayed where it was. Appending a variant gives it a new wire tag,
    /// and a reader that does not know that tag FAILS to decode, which is
    /// counted as `Dropped::Unparseable` rather than believed.
    ///
    /// So this needs no version bump, and per this file's own rule it must
    /// not: a version is for changing an existing message, not for adding one.
    ///
    /// The counts are the delegate's own, from the ops actually leaving the
    /// call. `put_bytes` is what this delegate OFFERED the node — never what
    /// the node then sends peer-to-peer, which only the node can count (six
    /// external instruments failed to infer it, freenet-contracts#39).
    CallBytes {
        /// Bytes handed to the node in `Op::Put` this call.
        put_bytes: u64,
        /// Puts issued.
        puts: u32,
        /// Gets issued, so a reader can tell a call that wrote from one that
        /// only looked.
        gets: u32,
    },
    /// How far along a write is — and WHOSE write (v4, craftworks-sdk#146).
    ///
    /// Every tab shares one delegate context, and a state is delivered to
    /// whichever connection's call produced it — so a tab ticking can be told
    /// the state of another tab's write, with the same write id as its own.
    /// Named, it drops it. Sent only to a writer that spoke v4; an older one
    /// keeps [`Reply::WriteState`].
    SessionWriteState {
        session: u64,
        write_id: u64,
        state: WriteState,
    },
    /// A reply to a v5 client, with that session's [`Ack`] (v5). Wraps EVERY
    /// reply to a v5 client — verdicts, pages, tick answers, call reports —
    /// and is never sent to an older one. ONE wrapper: a body that is itself
    /// `Acked` is refused at decode ([`Dropped::NestedAck`]).
    Acked {
        ack: Ack,
        body: Box<Reply>,
    },
    /// A [`Request::Commit`] read that no longer held: nothing of the write
    /// was applied (M2). `key` is the read that failed and `current` what the
    /// tree holds there now (its leaf hash, or `None`: absent), so the app can
    /// decide against the truth and not against a copy that may lag it. The
    /// write's state is [`WriteState::Conflict`], sent beside this. From
    /// `page::Server` to v4, for the reason on [`Request::Commit`].
    Conflicted {
        session: u64,
        write_id: u64,
        key: Vec<u8>,
        current: Option<[u8; 32]>,
    },
    /// A write refused AT THE DOOR because it changes `key` without having
    /// read it (sdk#235, W8): nothing applied. The write's state is
    /// [`WriteState::Unread`], sent beside this. From `page::Server` to v4
    /// (the same bundle, as [`Reply::Conflicted`]).
    Unread {
        session: u64,
        write_id: u64,
        key: Vec<u8>,
    },    /// A PUBLISHED write whose value at some of its keys another device of
    /// the same identity replaced (sdk#225b): the register's tie-break kept
    /// the other device's head at this write's seq, and the merge could not
    /// keep these `keys` (both sides changed them, or a read of this write's
    /// changed there). The winner's values stand at them; the write stays
    /// Published for the rest. `seq`/`root` name the head that won. From
    /// `page::Server` to v4 (the same bundle, as [`Reply::Conflicted`]).
    Superseded {
        session: u64,
        write_id: u64,
        seq: u64,
        root: [u8; 32],
        keys: Vec<Vec<u8>>,
    },
}

/// What the engine knows about one session's writes, on every reply to it
/// (v5 — WRITE-PATH.md "The ledger", "The rule"). Cumulative, so a dropped or
/// misrouted verdict heals on the next reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ack {
    /// WHOSE — a reply can reach another session (F49), and a client drops
    /// an Ack that is not its own, as it drops a foreign `SessionWriteState`.
    pub session: u64,
    /// The highest write id of this session whose commit is published. 0 is
    /// SAFE BY DIRECTION: it claims nothing published, so an engine that does
    /// not know yet can say 0 and settle nothing falsely.
    pub published_through: u64,
    /// The highest write id the engine has TAKEN (it can go backwards: the
    /// engine forgot). 0 is safe by direction, as above: nothing claimed taken.
    pub taken_through: u64,
    /// The TREE's parity, as a stat rather than a per-write verdict the
    /// platform can misroute: `None` when the engine does not KNOW (so it can
    /// never read as "0 owed — durable"); `Some` with the groups still owed at
    /// a head seq.
    pub parity: Option<Parity>,
    /// The highest frame sequence of this session the engine has seen — a
    /// MAXIMUM, not a receipt: it does not say that every lower frame
    /// arrived, only that none higher has.
    pub frames_seen: u64,
    /// The frame that started the call this reply is made in, if a frame of
    /// THIS session did (a reply produced in another session's call — a tick
    /// deciding a verdict — answers none of this session's frames).
    pub answering: Option<u64>,
}

/// The tree's parity at a head: how many groups are still owed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parity {
    pub owed_groups: u64,
    pub at_seq: u64,
}

/// Why a subscriber was told something changed.
///
/// Distinct facts, never merged. Re-reading is correct on any of them, but a
/// client deciding whether to back off has to know which it got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Why {
    /// The trees were compared over this range and they differ. A finding.
    Diffed,
    /// A block the comparison needed was not held. DURABLE — the old root's
    /// blocks may be gone for good — so repetition means the range goes
    /// `Stale` rather than notifying for ever.
    BlockMissing,
    /// The comparison was not attempted within this call's budget.
    /// TRANSIENT: retry promptly, do not back off.
    Budgeted,
    /// The engine has STOPPED comparing this range and is saying so once.
    /// Reload the range to retry; a declared degradation beats a silent one.
    Stale,
}

/// What came of a subscribe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accepted {
    Yes,
    /// The engine is at its own cap. The client still holds the intention and
    /// may try again once it drops one.
    Full,
    /// A bound longer than the engine will hold. Refused rather than trimmed:
    /// a silently narrowed range watches something the client did not ask for.
    TooWide,
}

/// One kind of step in an operation's call tree.
///
/// APPEND ONLY, like everything else on this wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    /// The operation began. `n` = how many edits, or 0 for a read.
    Began,
    /// The core returned effects. `n` = how many.
    Effects,
    /// Blocks were put. `n` = how many.
    Put,
    /// A put was read back and found. `n` = how many.
    ReadBack,
    /// The head was written. `n` = the seq.
    Head,
    /// The head was read back at the seq it was written at.
    HeadConfirmed,
    /// The operation reached a state a client can see. `n` = the
    /// [`WriteState`] as its wire tag, or rows for a read.
    Reached,
    /// Blocks were fetched. `n` = how many.
    Fetch,

    // ---- the SDK's own boundaries ----
    //
    // ONE event vocabulary, so a test's dump, the harness's tables,
    // `db.trace` and the builder's viewer all read the same words. These are
    // recorded client-side rather than sent by the engine, and they are here
    // rather than in a second enum for exactly that reason: a trace that
    // stops at the wire is a trace that cannot show why a binding did not
    // update.
    /// A client subscription to a tree's HEAD contract was asked for.
    /// `n` = 1 if it was accepted.
    HeadWatch,
    /// The node pushed "this head moved". `n` = how many bindings it woke.
    HeadMoved,
    /// A binding reloaded. `n` = rows it now holds.
    Reloaded,
    /// A binding looked and nothing had moved. `n` = 0.
    Quiet,
    /// A live binding lost its notifier and is on the backstop alone.
    Downgraded,
}

/// What kind of message woke an engine call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Saw {
    Client,
    GetResponse,
    PutResponse,
    UpdateResponse,
    Other,
}

/// The states a write moves through, as a client sees them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WriteState {
    Accepted,
    Stalled,
    Published,
    ParityComplete,
    /// TERMINAL, nothing applied. The client still holds the write and may
    /// re-submit it — which is what the OUTBOX is for.
    Busy,
    /// TERMINAL. The edit is not in the tree and never will be.
    Failed,
    /// TERMINAL. Its commit will not publish and the client is the only
    /// thing that still has it.
    Lost,
    /// TERMINAL, nothing applied, and **never** acceptable as it is: the
    /// write is over `limit` of `bound` by what `got` says (craftworks-sdk#136).
    /// Unlike `Busy` it must not be re-sent — the same write is refused the
    /// same way every time. Splitting it is the caller's decision.
    ///
    /// `limit` and `got` SATURATE at `u32::MAX` (see [`saturating_u32`]):
    /// a count too large for the field says "at least this much", never a
    /// wrapped small number that would read as under the limit. v3 only.
    TooLarge {
        bound: WriteBound,
        limit: u32,
        got: u32,
    },
    /// This (session, write id) is below the engine's next — already taken
    /// (v5, rule 1). Nothing applied; the reply's [`Ack`] says whether it
    /// published.
    Duplicate,
    /// Not the write the engine expects next from this session (v5, rule 4).
    /// Nothing applied; the client re-sends `expected`, at once, whole.
    OutOfOrder {
        expected: u64,
    },
    /// TERMINAL, nothing applied: a key a [`Request::Commit`] READ no longer
    /// holds what it read ([`Reply::Conflicted`] names it). NEVER re-sent as
    /// it is — the same write conflicts the same way; what to do next is the
    /// app's. From `page::Server` at v4 (see [`Request::Commit`]).
    Conflict,
    /// TERMINAL, nothing applied: the write changes a key it did not read
    /// (sdk#235, W8) — judged at the door, before any other answer.
    /// [`Reply::Unread`] names the key. NEVER re-sent: the same write is
    /// refused the same way every time.
    Unread,
    /// Nothing applied, not admitted NOW: the page's write queue holds
    /// `bytes` of its `limit` and this session already has a write in it
    /// (R-b; COMMIT-LIFE K1). BACKPRESSURE: the client waits for room and
    /// makes the write again. Never `Busy`. Counts saturate at `u32::MAX` as
    /// `TooLarge`'s do. From `page::Server` at v4.
    QueueFull { bytes: u32, limit: u32 },
    /// TERMINAL: whether the write LANDED cannot be known (its commit died by
    /// the engine's lights and the new head's ledger no longer lists this
    /// page; COMMIT-LIFE ⁵). Never re-sent, never rolled back: the app is
    /// told to CHECK. From `page::Server` at v4; an older client is told
    /// `Stalled` (nothing claimed).
    Unknown,
    /// TERMINAL, as [`WriteState::Failed`], with its cause named: the ONE
    /// list, [`FailWhy`] (sdk#500). From `page::Server` at v4; an older client
    /// is told `Failed`, which is true to it without a cause it could not read.
    FailedWhy { why: FailWhy },
}

pub use core_types::fail::FailWhy;

/// Which of the engine's bounds a [`WriteState::TooLarge`] write is over.
/// APPEND ONLY: a variant's position is its wire tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WriteBound {
    /// Distinct blocks one commit would write (`max_commit_blocks`).
    CommitBlocks,
    /// Bytes in one write (`max_write_bytes`).
    WriteBytes,
}

/// A count into a `u32` wire field, saturating rather than wrapping.
///
/// `n as u32` of 2^32 + 5 is 5 — a count that says "far over" arriving as
/// "just over", or under. A saturated count is still over any limit it was
/// compared with.
pub fn saturating_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl WriteState {
    /// Is there nothing more coming for this write?
    pub fn terminal(self) -> bool {
        matches!(
            self,
            WriteState::Busy
                | WriteState::Failed
                | WriteState::Lost
                | WriteState::TooLarge { .. }
                | WriteState::OutOfOrder { .. }
                | WriteState::Conflict
                | WriteState::Unread
                | WriteState::QueueFull { .. }
                | WriteState::Unknown
                | WriteState::FailedWhy { .. }
        )
    }

    /// Should the client send it again?
    ///
    /// Only `Busy`: nothing was applied, so a re-submission applies it once.
    /// `Failed` is also "not in the tree", but it means the write itself was
    /// refused and sending it again refuses it again. `Lost` needs the
    /// client's judgement, since another writer may have moved the tree
    /// underneath it.
    pub fn should_resubmit(self) -> bool {
        matches!(self, WriteState::Busy | WriteState::OutOfOrder { .. })
    }

    /// The protocol version that introduced this state. A client that spoke
    /// an older one cannot decode it and must be told [`Self::for_client`].
    ///
    /// ONE MATCH, so a state added later (sdk#143's `Conflict`) is one arm
    /// here and one in `for_client`, and the version tests pick it up by
    /// iterating [`Self::NEWER_THAN_V2`].
    pub fn since(self) -> u16 {
        match self {
            WriteState::Accepted
            | WriteState::Stalled
            | WriteState::Published
            | WriteState::ParityComplete
            | WriteState::Busy
            | WriteState::Failed
            | WriteState::Lost => 1,
            WriteState::TooLarge { .. } => 3,
            WriteState::Conflict
            | WriteState::Unread
            | WriteState::QueueFull { .. }
            | WriteState::Unknown
            | WriteState::FailedWhy { .. } => SESSION_SINCE,
            WriteState::Duplicate | WriteState::OutOfOrder { .. } => FLOOR_SINCE,
        }
    }

    /// What a client that speaks `version` is told for this state: the state
    /// itself if it can read it, else the older state that means the same to
    /// it. The delegate calls this at the one place write states leave it.
    pub fn for_client(self, version: u16) -> WriteState {
        // Reaching a pre-v5 client with an order-rule verdict is a BUG, and a
        // debug build says so here, at the call. The VALUE it is told is
        // `mapped_down`, which is pure and tested in every profile: a
        // delegate is a release build, and there the value is what ships.
        debug_assert!(
            version >= FLOOR_SINCE || !matches!(self, WriteState::Duplicate | WriteState::OutOfOrder { .. }),
            "the order rule is v5-only: a pre-v5 client must never be told this"
        );
        self.mapped_down(version)
    }

    /// What a client of `version` is told for this state — the pure mapping,
    /// with no assertion, so it is tested in debug and release alike.
    pub fn mapped_down(self, version: u16) -> WriteState {
        if version >= self.since() {
            return self;
        }
        match self {
            // Not in the tree, and sending it again is refused again —
            // exactly what `Failed` already tells an older client.
            WriteState::TooLarge { .. } => WriteState::Failed,
            // Nothing applied and sending it again conflicts again: what
            // `Failed` tells an older client.
            WriteState::Conflict => WriteState::Failed,
            // Refused at the door, and refused the same way again: what
            // `Failed` tells an older client — never an unknown tag it drops
            // and later reads as "may have been saved" (architect, sdk#235).
            WriteState::Unread => WriteState::Failed,
            // Nothing applied; the queue is the page's. `Failed`, as `Unread`:
            // never `Busy`, which an older outbox re-sent for ever.
            WriteState::QueueFull { .. } => WriteState::Failed,
            // Claims nothing: `Stalled` (never `Lost`, which would roll back
            // a write that may have landed).
            WriteState::Unknown => WriteState::Stalled,
            // The same end without its cause: what `Failed` always told.
            WriteState::FailedWhy { .. } => WriteState::Failed,
            // The order rule's verdicts exist only on v5, whose writes carry a
            // floor; a pre-v5 client never meets them. If one ever did, it is
            // told the TRUE v4 equivalent — never `Failed`, which would roll
            // back a write the engine may have applied (a false rollback).
            // Nothing applied, may be sent again: exactly `Busy`.
            WriteState::OutOfOrder { .. } => WriteState::Busy,
            // Non-terminal, claims nothing false: `Stalled`.
            WriteState::Duplicate => WriteState::Stalled,
            other => other,
        }
    }

    /// `TooLarge`, from counts as the engine holds them. The ONLY way the
    /// delegate builds one, so the narrowing to the wire's `u32` happens in
    /// one place and saturates -- a wrapped count could read as UNDER the
    /// limit it is over.
    pub fn too_large(bound: WriteBound, limit: usize, got: usize) -> WriteState {
        WriteState::TooLarge {
            bound,
            limit: saturating_u32(limit),
            got: saturating_u32(got),
        }
    }

    /// Every state newer than v2, one example each — what the version tests
    /// iterate, so a new state is covered by adding it here.
    pub const NEWER_THAN_V2: &'static [WriteState] = &[
        WriteState::TooLarge {
            bound: WriteBound::CommitBlocks,
            limit: 128,
            got: 129,
        },
        WriteState::Duplicate,
        WriteState::OutOfOrder { expected: 7 },
        WriteState::Conflict,
        WriteState::Unread,
        WriteState::QueueFull { bytes: 4096, limit: 4096 },
        WriteState::Unknown,
        WriteState::FailedWhy { why: FailWhy::SignerRefused },
    ];

    /// The order rule's verdicts (v5): never sent below v5, so their
    /// downgrade is a debug assertion, not a path.
    pub const ORDER_RULE: &'static [WriteState] = &[WriteState::Duplicate, WriteState::OutOfOrder { expected: 7 }];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dropped {
    /// Not a message this build can read at all.
    Unparseable,
    /// Trailing bytes: the prefix parsed, and a prefix is not a message.
    TrailingBytes,
    /// Bigger than this build will decode.
    TooLarge,
    /// A response about something nobody asked for.
    Unexpected,
    /// A message kind this side has no use for. Appended, like everything
    /// else: a variant's position is its wire tag.
    NotForUs,
    /// A v4 frame whose session is not one a client can hold: zero, the
    /// legacy session, or wider than [`SESSION_BITS`] (craftworks-sdk#146).
    /// Refused by name rather than truncated: a truncated session is told
    /// its verdicts under a number it does not hold, and drops them all.
    BadSession,
    /// A v5 frame whose frame sequence is 0: sequences start at 1, as
    /// sessions are never 0.
    BadFrame,
    /// A write in the wrong shape for its version: a plain `Write` in a v5
    /// frame (which must say its floor), or a `WriteFrom` below v5. One write
    /// shape per version.
    WrongWriteShape,
    /// An `Acked` reply whose body is itself `Acked`: one wrapper, never two.
    NestedAck,
    /// A v5 frame asked of an encoder that cannot write `now` and a frame
    /// sequence (`encode_session_request`); use `encode_v5_request`.
    NeedsV5Envelope,
    /// An `Acked` reply whose `ack.session` is not the session its body names:
    /// one reply is about one session.
    AckSessionMismatch,
    /// A session-less encoder asked for a version whose frames carry a
    /// session (v4 and later): use `encode_session_request`, or say you mean
    /// the legacy path with `LAST_SESSIONLESS` (craftworks-sdk#194).
    NeedsSession,
}

/// The largest message this build will decode.
///
/// A bound BEFORE the decoder sees anything, so a length field cannot ask for
/// an allocation the sender chooses.
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;

/// Encoding options for everything on this wire.
///
/// `with_fixint_encoding` to match `bincode::serialize`; NO
/// `allow_trailing_bytes`, so a message with anything after it is refused
/// rather than half-read; and a limit, so a damaged length cannot ask for
/// more than the cap.
fn opts() -> impl bincode::Options {
    use bincode::Options;
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_MESSAGE as u64)
}

/// Encode a client's message — or say why it cannot be sent.
///
/// A `Result`, never an empty frame. This was `.unwrap_or_default()`, so a
/// message over [`MAX_MESSAGE`] became `[]`: the client sent nothing, the
/// engine answered `Unparseable` with no write id, the write sat awaiting a
/// verdict until the copy's timeout rolled it back, and the only words anyone
/// saw were the wrong ones (craftworks-sdk#136).
///
/// A request with NO session is a pre-v4 request: from v4 a frame must name
/// a real session, and the legacy one is refused at decode
/// ([`Dropped::BadSession`]). So this encodes at most v3
/// ([`LAST_SESSIONLESS`]), and asking it for v4 or later is REFUSED by name
/// ([`Dropped::NeedsSession`]) — it used to CLAMP, so every caller passing
/// `CURRENT` (every probe) silently spoke v3 on the legacy session, the one
/// path no app uses (craftworks-sdk#194). A client that has a session sends it
/// with [`encode_session_request`]; one that means the legacy path says so
/// with `LAST_SESSIONLESS`.
pub fn encode_request(version: u16, body: &Request) -> Result<Vec<u8>, Dropped> {
    if version >= SESSION_SINCE {
        return Err(Dropped::NeedsSession);
    }
    encode_session_request(version, LEGACY_SESSION, body)
}

/// The newest version with NO session on the wire: what a caller of
/// [`encode_request`] passes when it means the legacy path.
pub const LAST_SESSIONLESS: u16 = SESSION_SINCE - 1;

/// Encode a client's message from `session`. Before v4 the session is not
/// on the wire and a reader takes it to be [`LEGACY_SESSION`].
///
/// At most v4: a v5 frame also carries `now` and a frame sequence, so asking
/// for v5 here is REFUSED by name ([`Dropped::NeedsV5Envelope`]) — never
/// clamped: a clamp would, the day `CURRENT` becomes 5, keep every caller
/// sending v4 without a word.
pub fn encode_session_request(version: u16, session: u64, body: &Request) -> Result<Vec<u8>, Dropped> {
    use bincode::Options;
    if version >= FLOOR_SINCE {
        return Err(Dropped::NeedsV5Envelope);
    }
    // An invalid session on a v4 frame ENCODES (so a test can send one) and
    // is refused at DECODE, which is where it has to be enforced anyway.
    if version >= SESSION_SINCE {
        opts().serialize(&EnvelopeV4 { version, session, body: body.clone() })
    } else {
        opts().serialize(&LegacyEnvelope { version, body: body.clone() })
    }
    .map_err(|e| classify(&e))
}

/// Encode a v5 frame: the session, the client's wall clock in ms, and this
/// session's next frame sequence. Like the session, a bad frame (0) ENCODES
/// and is refused at decode.
pub fn encode_v5_request(session: u64, now_ms: u64, frame: u64, body: &Request) -> Result<Vec<u8>, Dropped> {
    use bincode::Options;
    opts()
        .serialize(&EnvelopeV5 { version: FLOOR_SINCE, session, now_ms, frame, body: body.clone() })
        .map_err(|e| classify(&e))
}

/// Encode a reply — or say why it cannot be sent. See [`encode_request`].
pub fn encode_reply(r: &Reply) -> Result<Vec<u8>, Dropped> {
    use bincode::Options;
    opts().serialize(r).map_err(|e| classify(&e))
}

/// How many bytes a client message would encode to, with no limit applied —
/// so a sender can refuse a write that will not fit BEFORE anything is sent
/// or held.
pub fn request_len(version: u16, body: &Request) -> u64 {
    use bincode::Options;
    let o = bincode::DefaultOptions::new().with_fixint_encoding();
    if version >= FLOOR_SINCE {
        o.serialized_size(&EnvelopeV5 { version, session: LEGACY_SESSION, now_ms: 0, frame: 0, body: body.clone() })
    } else if version >= SESSION_SINCE {
        o.serialized_size(&EnvelopeV4 { version, session: LEGACY_SESSION, body: body.clone() })
    } else {
        o.serialized_size(&LegacyEnvelope { version, body: body.clone() })
    }
    .unwrap_or(u64::MAX)
}

/// What came of trying to read a client's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    Ok(Envelope),
    /// A version this build does not serve. The VERSION was readable, which
    /// is the whole reason it is on the front.
    Unsupported(u16),
    Dropped(Dropped),
}

/// Read a client's message.
///
/// The version is checked BEFORE the body is trusted: a v2 body decoded by a
/// v1 decoder is not an error, it is a different message, and the version is
/// the only thing that can say so.
pub fn decode_request(bytes: &[u8]) -> Incoming {
    use bincode::Options;
    if bytes.len() > MAX_MESSAGE {
        return Incoming::Dropped(Dropped::TooLarge);
    }
    // The version alone, first. A body this build cannot read must still
    // produce `Unsupported` rather than `Unparseable`, or a newer client
    // learns nothing about why it was refused.
    let version = match bytes.get(..2) {
        Some(v) => u16::from_le_bytes([v[0], v[1]]),
        None => return Incoming::Dropped(Dropped::Unparseable),
    };
    if !KNOWN.contains(&version) {
        return Incoming::Unsupported(version);
    }
    let decoded = if version >= FLOOR_SINCE {
        match opts().deserialize::<EnvelopeV5>(bytes) {
            Ok(e) if !session_is_valid(e.session) => return Incoming::Dropped(Dropped::BadSession),
            Ok(e) if e.frame == 0 => return Incoming::Dropped(Dropped::BadFrame),
            Ok(e) => Ok(Envelope { version: e.version, session: e.session, now_ms: Some(e.now_ms), frame: Some(e.frame), body: e.body }),
            Err(e) => Err(e),
        }
    } else if version >= SESSION_SINCE {
        match opts().deserialize::<EnvelopeV4>(bytes) {
            Ok(e) if !session_is_valid(e.session) => return Incoming::Dropped(Dropped::BadSession),
            Ok(e) => Ok(Envelope { version: e.version, session: e.session, now_ms: None, frame: None, body: e.body }),
            Err(e) => Err(e),
        }
    } else {
        opts()
            .deserialize::<LegacyEnvelope>(bytes)
            .map(|e| Envelope { version: e.version, session: LEGACY_SESSION, now_ms: None, frame: None, body: e.body })
    };
    match decoded {
        // ONE write shape per version: a v5 write says its floor, and a
        // floor means nothing below v5.
        Ok(e) if matches!(e.body, Request::Write { .. }) && e.version >= FLOOR_SINCE => Incoming::Dropped(Dropped::WrongWriteShape),
        Ok(e) if matches!(e.body, Request::WriteFrom { .. }) && e.version < FLOOR_SINCE => Incoming::Dropped(Dropped::WrongWriteShape),
        Ok(e) => Incoming::Ok(e),
        Err(e) => Incoming::Dropped(classify(&e)),
    }
}

pub fn decode_reply(bytes: &[u8]) -> Result<Reply, Dropped> {
    use bincode::Options;
    if bytes.len() > MAX_MESSAGE {
        return Err(Dropped::TooLarge);
    }
    match opts().deserialize::<Reply>(bytes) {
        // One wrapper, never two.
        Ok(Reply::Acked { body, .. }) if matches!(*body, Reply::Acked { .. }) => Err(Dropped::NestedAck),
        // One reply is about one session: an Ack for A around B's verdict is
        // refused rather than half-applied.
        Ok(Reply::Acked { ack, body }) if matches!(*body, Reply::SessionWriteState { session, .. } if session != ack.session) => {
            Err(Dropped::AckSessionMismatch)
        }
        Ok(r) => Ok(r),
        Err(e) => Err(classify(&e)),
    }
}

fn classify(e: &bincode::Error) -> Dropped {
    match **e {
        bincode::ErrorKind::SizeLimit => Dropped::TooLarge,
        // bincode reports leftover input this way when trailing bytes are
        // not allowed.
        _ if e.to_string().contains("Slice had bytes remaining") => Dropped::TrailingBytes,
        _ => Dropped::Unparseable,
    }
}

/// How often a connected page sends [`Request::Tick`], in milliseconds.
///
/// # What a tick IS
///
/// The engine has no clock (F32: a delegate gets a fresh linear memory every
/// call, and nothing in it advances time). Every deadline it has — when owed
/// parity goes out, when a stuck commit is called `Stalled` — is measured
/// against the number a client last sent, and against nothing else.
///
/// So the number's MEANING is a decision, and there were two candidates:
///
/// * **How many times my timer fired.** Rejected. A browser throttles a
///   background tab's timers to as little as one a minute, so N ticks would
///   mean anywhere from N seconds to N minutes — the deadline would stretch
///   silently with the thing least worth trusting. Worse, a delegate's
///   context is shared by EVERY connection (F47): a second tab opening would
///   send 1, 2, 3 while the first was at 4000, and `now` would jump
///   backwards, making everything look young again.
///
/// * **The wall clock, quantised.** What this is. `now` is
///   `Date.now() / TICK_MS` — whole seconds since the epoch. Two tabs on one
///   node agree exactly, because they are reading the same clock rather than
///   counting their own events, and a throttled tab sends the right number
///   late instead of a wrong number on time.
///
/// # And the quantum is what the engine's bounds are in
///
/// `Params::parity_age` and `Params::max_accept_age` are counts of this
/// unit. **At `TICK_MS = 1000` they read as seconds, and only then.** They
/// are plain small numbers because of this constant, so moving it rescales
/// every deadline in the engine at once: 32 and 64 become 32 and 64 of
/// whatever the new quantum is, silently, with nothing failing. Anyone
/// changing it converts those params in the same commit, or states in
/// `Params` what the new unit is.
///
/// # A CLIENT'S clock, and what that client can do with it
///
/// The engine has no clock of its own, so this is not a client HELPING with
/// the time — it is the only time there is. A client therefore decides WHEN
/// the engine acts, and the honest way to read the guarantee is: a delegate
/// trusts the wall clock of whoever is connected to it.
///
/// That is fine for the case this is built for — a person's own node, driven
/// by their own tabs. It is worth writing down what it means when it is not.
///
/// A client that lies about the time can:
///
/// * **make deadlines fire early**, by sending a number far in the future.
///   Owed parity goes out before it has coalesced, and a commit in flight is
///   reported `Stalled` sooner than it deserves. Both cost work, and neither
///   is wrong about the DATA;
/// * **make deadlines not fire**, by sending a number that never advances.
///   Owed parity waits and a stuck commit is never called stalled. This is
///   exactly the state before anything sent time at all, so it is a denial
///   of the deadlines, not a corruption.
///
/// It cannot:
///
/// * **change what is written.** Time decides WHEN, never WHAT: every effect
///   a tick produces is one the engine already owed, computed from the tree
///   and not from the number. `Tick` carries no keys, no values and no root;
/// * **move a deadline backwards for somebody else in a way that loses
///   data.** `now` going down makes things look young again — work is
///   delayed, not undone — and the age comparisons saturate rather than
///   wrap;
/// * **be told apart from an honest client by the engine**, which is the
///   actual limit: a delegate's context is shared by every connection (F47),
///   so the LAST tick wins and there is nowhere to record whose it was.
///
/// The bound that matters, then, is on who may connect to a node's delegate,
/// not on what this field says. A shared delegate whose clients are not all
/// trusted needs the time to come from somewhere they do not control, and
/// nothing here provides that.
pub const TICK_MS: u64 = 1_000;

/// The wall clock in the unit [`Request::Tick`] carries.
///
/// One function so a page cannot quantise it one way and a test another.
pub const fn tick_of(now_ms: u64) -> u64 {
    now_ms / TICK_MS
}
