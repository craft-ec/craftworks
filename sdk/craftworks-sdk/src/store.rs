//! The storage interface the SDK is written against: a sorted byte map.
//!
//! Today an in-memory map sits behind it. The prolly tree, and later the engine
//! delegate, plug in behind the same four operations; nothing above changes.

use std::collections::BTreeMap;

/// One change to one key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    Put(Vec<u8>),
    Delete,
}

/// Why a read could not be answered.
///
/// Reads have an error channel and writes do not, and the asymmetry is
/// deliberate: a write is screened by [`Db`](crate::Db) before it reaches a
/// store, but whether a read can be answered at all is a property of the
/// store, and only the store knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The engine could not answer: a block it needed was not to be had.
    ///
    /// Not "the key does not exist" — that is `Ok(None)`, and the two are
    /// different facts an app acts on differently.
    Unavailable,
    /// The connection to the engine produced nothing usable.
    NoAnswer,
    /// This range has never been loaded, so nothing here knows what is in it.
    ///
    /// **Not "empty".** The two are one byte apart in a reply and a world
    /// apart in what an app does with them: told "empty" it draws an empty
    /// list and stops. Told this, it reloads. A cache that answered the first
    /// for the second would make every unbound range look like missing data,
    /// with nothing on screen to say so.
    NotLoaded,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Unavailable => f.write_str(
                "the engine could not reach a block this read needed; \
                 the key may well exist",
            ),
            StoreError::NoAnswer => f.write_str("the engine did not answer"),
            StoreError::NotLoaded => f.write_str(
                "this range has not been loaded yet — reload it; it is not known to be empty",
            ),
        }
    }
}

impl std::error::Error for StoreError {}

pub type Read<T> = std::result::Result<T, StoreError>;

/// What a delta answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delta {
    /// The changes, and where they bring the reader to. `None` as a value is
    /// a REMOVAL, not an empty value.
    Changes {
        changes: Vec<(Vec<u8>, Option<Vec<u8>>)>,
        cursor: Option<Vec<u8>>,
        new_root: [u8; 32],
    },
    /// The delta could not be computed; read the range in full.
    ///
    /// A defined outcome rather than an error: the root the reader was
    /// standing on may simply be gone, which is ordinary for a reader that
    /// was away a while.
    FullReloadRequired { new_root: [u8; 32] },
}
core_types::vocabulary! {
    /// Reading, which for some stores is a ROUND TRIP.
    ///
    /// Separate from [`Store`], and taking `&mut self`, and both are the same
    /// decision. An engine-backed store's data is on the other side of a
    /// connection: a read is an operation with a duration, not an accessor. A
    /// `&self` read would have to hide interior mutability — making a network
    /// round trip look free — or answer `None`, which says the key does not
    /// exist, or panic, which in a browser with `panic = abort` is a dead wasm
    /// instance rather than something an app can catch.
    ///
    /// `&mut self` says so in the type, and it says so for every backend, which
    /// is what lets one surface sit over both: the in-memory store implements
    /// this trivially and an app does not change when it moves onto a node.
    /// What a record's OWN write is doing.
    ///
    /// A fixed set of codes and never prose: a row branches on this to decide
    /// whether it may say "saved", and a caller that matched on a message would
    /// change behaviour the first time anybody reworded it, silently.
    ///
    /// The distinction a person actually needs is the middle one. "Saving",
    /// "saved here but not yet on the network" and "on the network" are three
    /// different facts, and somebody deciding whether it is safe to close the tab
    /// needs the second told apart from the third.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub enum RowState {
        /// No pending write: this is what the engine last said.
        #[default]
        Clean => "CLEAN",
        /// Written here and not sent yet — the engine was busy with another.
        Queued => "QUEUED",
        /// Sent and accepted, not yet published. **Closing the tab now loses it.**
        Pending => "PENDING",
        /// A write that was rolled back: it failed, or nothing ever answered it.
        RolledBack => "ROLLED_BACK",
        /// Saved, AND its redundancy is on the network: the commit of the key's
        /// last write is BACKED_UP -- every group it changed whole (sdk#415, rule
        /// 10; READ-STATE `key_state`'s `SavedAndBackedUp`, the one owner). What
        /// the builder's "saved + backed up" chip shows.
        BackedUp => "BACKED_UP",
}
}

impl RowState {
    /// Whether a row showing this value may claim to be saved.
    ///
    /// An invariant, not a convenience: a row showing an unacknowledged value
    /// must SAY so.
    pub fn is_settled(self) -> bool {
        matches!(self, RowState::Clean | RowState::BackedUp)
    }

    /// Saved, and its redundancy on the network too.
    pub fn is_backed_up(self) -> bool {
        self == RowState::BackedUp
    }

    /// Lost: the write was rolled back -- it failed, or nothing ever answered it -- and its value is not in the tree
    /// (sdk#518).
    pub fn is_lost(self) -> bool {
        self == RowState::RolledBack
    }
}

/// The two widths a record id comes in: a bare record's rkey (32 hex) or a
/// parented one's parent ‖ rkey (64 hex). A value from a fixed set, so the
/// recorder can put nothing else on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdWidth {
    Bare,
    Parented,
}

impl IdWidth {
    pub fn of(parented: bool) -> Self {
        if parented { IdWidth::Parented } else { IdWidth::Bare }
    }

    /// Hex characters in an id of this width.
    pub fn hex(self) -> u64 {
        match self {
            IdWidth::Bare => 32,
            IdWidth::Parented => 64,
        }
    }
}

pub trait Reads {
    fn get(&mut self, key: &[u8]) -> Read<Option<Vec<u8>>>;

    /// What this key's own write is doing.
    ///
    /// The default is `Clean`, which is the truth for a store that IS the
    /// tree: an in-memory write is applied the moment it is made, so there is
    /// never a write in flight to report. A store that talks to a node
    /// overrides this, because for it the question has a real answer.
    fn row_state(&self, _key: &[u8]) -> RowState {
        RowState::Clean
    }

    /// A read was handed a well-formed record id of the wrong WIDTH for its
    /// domain — 32 hex where the domain keys under a parent (64), or the
    /// reverse — and answered "not found".
    ///
    /// That answer is right (craftworks-sdk#118: the ids reaching a read come
    /// from outside), and it makes one programmer error silent: a caller that
    /// truncated a 64-hex id. So it is RECORDED, where a store has a recorder,
    /// instead of thrown. Only the two widths: never the id, and never the
    /// domain, whose name is the app's.
    ///
    /// No-op by default: a store with no recorder has nowhere to put it.
    fn wrong_width(&self, _given: IdWidth, _wanted: IdWidth) {}

    /// Entries with `lo <= key < hi`, ascending, or descending if `reverse`;
    /// at most `limit`.
    fn scan(
        &mut self,
        lo: &[u8],
        hi: &[u8],
        reverse: bool,
        limit: usize,
    ) -> Read<Vec<(Vec<u8>, Vec<u8>)>>;

    /// How many entries lie in `[lo, hi)` (craftworks-sdk#123).
    ///
    /// By default it enumerates — right for a store with no tree behind it
    /// (a map holds rows, not nodes). A store that HAS
    /// the tree answers from the aggregate every node already carries, in
    /// O(height) node reads rather than one per entry.
    ///
    /// **Per TREE, never per view.** Per-tree counts must never be summed into
    /// an overlay's count: a key on two device trees counts twice, and a
    /// delete in one can reveal an older value in another.
    ///
    /// A count that could not be answered is an ERROR (`NotLoaded`), never 0:
    /// "I have not looked" is not "there is nothing there".
    fn count(&mut self, lo: &[u8], hi: &[u8]) -> Read<u64> {
        Ok(self.scan(lo, hi, false, usize::MAX)?.len() as u64)
    }

    /// The root this store is currently standing on.
    ///
    /// What a reader passes back to [`Reads::changes_since`]. An in-memory
    /// store has one too — its tree has a root like any other — so a binding
    /// written against this works unchanged on both backends.
    fn root(&mut self) -> Read<[u8; 32]>;

    /// What changed in `[lo, hi)` since `from`.
    ///
    /// **Per TREE.** A view assembled over several device trees is NOT the
    /// union of their per-tree deltas: a tombstone in one tree can REVEAL an
    /// older value in another, which no per-tree diff mentions. Phase 3 has
    /// one tree per identity; this is stated so nothing later assumes the
    /// union is valid.
    ///
    /// No subscription is involved. A delta is a capability of the tree,
    /// available to any reader holding a root — a plain reload uses it.
    fn changes_since(
        &mut self,
        from: [u8; 32],
        lo: &[u8],
        hi: &[u8],
        max_entries: u32,
    ) -> Read<Delta>;
}

/// A sorted byte map: the WRITE half.
///
/// Reading lives in [`Reads`], because for some stores a read is a round trip
/// and that has to be visible in the type rather than hidden behind `&self`.
///
/// **An implementation may treat a write it cannot represent as a bug and
/// stop.** A key or value past what the underlying structure allows is not a
/// refusal the store can name — [`Refused`] is for a
/// store holding writes for a node — and failing quietly would be worse than
/// failing loudly. Screening is
/// [`Db`](crate::Db)'s job: it checks every key and value against the tree's
/// limits before a store is touched, and returns an error the app can handle.
/// That division is deliberate, and it is why [`TreeStore`](crate::TreeStore)
/// may panic where a limit is breached.
pub trait Store {
    /// One key, as a batch of one: refused or not the same way as
    /// [`Store::apply_batch`], and the refusal RETURNED (sdk#186). These
    /// returned nothing once, so a refusal through them reached only a list —
    /// the door #180 closed for `apply_batch`, left open beside it. They are
    /// not separate paths any more: no store implements them.
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), Refused> {
        self.apply_batch(&[(key.to_vec(), Edit::Put(value.to_vec()))])
    }
    /// See [`Store::put`].
    fn delete(&mut self, key: &[u8]) -> Result<(), Refused> {
        self.apply_batch(&[(key.to_vec(), Edit::Delete)])
    }

    /// Apply several edits as ONE change.
    ///
    /// A record and the index entries that point at it are one fact, and the
    /// store behind this trait may be a structure where every write costs a new
    /// root — writing them one at a time would make several versions of a state
    /// that was never half-written. The default loops, which is right for a
    /// store where a write is just a write.
    ///
    /// **The caller sorts.** Edits arrive sorted by key with no duplicates; the
    /// SDK does that in [`sorted_edits`] before calling, because it is the
    /// caller that knows which of two writes to one key is the later one.
    ///
    /// **A refusal is the RETURN VALUE** (craftworks-sdk#180). A store that
    /// holds writes for a node — the page's store — may
    /// refuse one before anything is applied: no room, no session, too large
    /// to send. It used to push the refusal onto a list and return `()`, and
    /// nothing read the list: `Db` answered `Created` for 45 of 300 writes the
    /// copy had refused. Returned, it cannot be missed — there is no second
    /// place to look.
    fn apply_batch(&mut self, edits: &[(Vec<u8>, Edit)]) -> Result<(), Refused>;
    /// The id of the write this store made last, if it numbers them (M2's
    /// re-run: `Db` remembers what each of its writes MEANT by this id).
    fn last_write_id(&self) -> Option<u64> {
        None
    }

    /// Is this write still waiting on the node (not yet ended)?
    fn is_pending_write(&self, write_id: u64) -> bool {
        let _ = write_id;
        false
    }

    /// Conflicts since the last call, each a CHAIN: the write that conflicted
    /// and the later writes that fell with it (on its keys, W1), in the order
    /// they were made, with the keys whose reads no longer held.
    fn take_conflict_chains(&mut self) -> Vec<ConflictChain> {
        Vec::new()
    }

    /// ONE BUDGET (sdk#265): the NEXT write made is a `Db` re-run of one
    /// that had spent `tries`; the engine counts on from there. `0`: none.
    fn carry_tries(&mut self, tries: u32) {
        let _ = tries;
    }

    /// The one budget of tries a write has -- dead-commit re-sends and `Db`
    /// re-runs together. The ENGINE owns the number; a store without one
    /// never conflicts, and answers the engine's default.
    fn max_write_tries(&self) -> u32 {
        engine::Params::default().max_write_tries
    }

    /// Apply several edits as ONE change that says what it READ (M2,
    /// sdk#148): a store that holds writes for a NODE sends the reads with
    /// them, and the engine checks them where the edits land. The default is
    /// right for a store that IS the truth — `TreeStore`, or a test's in-memory map — where
    /// `Db` read and writes inside one `&mut` borrow, so nothing can have
    /// moved between the read and the write.
    fn apply_commit(
        &mut self,
        reads: &[(Vec<u8>, protocol::Expect)],
        edits: &[(Vec<u8>, Edit)],
    ) -> Result<(), Refused> {
        let _ = reads;
        self.apply_batch(edits)
    }

    /// [`Store::apply_commit`], DEFERRED (sdk#350): a write that commits only
    /// in the company of a non-deferred one (an app's schema on open). Only a
    /// store that holds writes for a page's ENGINE can defer; for every other
    /// store -- the truth itself -- a deferred write is simply a write.
    fn apply_deferred_commit(
        &mut self,
        reads: &[(Vec<u8>, protocol::Expect)],
        edits: &[(Vec<u8>, Edit)],
    ) -> Result<(), Refused> {
        self.apply_commit(reads, edits)
    }
}

/// A write that CONFLICTED (a key it read had moved) and the later writes that
/// fell with it, in the order they were made (M2's re-run, sdk#143/#144).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConflictChain {
    pub write_ids: Vec<u64>,
    /// The keys whose reads no longer held (often the schema, a key no write
    /// in the chain wrote).
    pub keys: Vec<Vec<u8>>,
    /// The most tries any write of the chain had spent in the engine
    /// (dead-commit re-sends and earlier re-runs): its re-run draws on from
    /// here -- ONE budget, sdk#265, never a count of `Db`'s own.
    pub tries: u32,
}

/// Why a write was refused before it was applied anywhere.
///
/// Refused, not queued and not dropped: it reaches the app as an answer it
/// can act on rather than as a write that quietly never happens. Each reason
/// asks something different of the caller -- wait, reload, split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// The page's write queue holds `bytes` of its `limit` (R-b; COMMIT-LIFE
    /// K1). Nothing was applied. BACKPRESSURE: the queue drains as its writes
    /// publish, and the same write made again then is taken -- the SDK's
    /// `write()` waits for that, and says this only at the app's deadline.
    QueueFull { bytes: usize, limit: usize },
    /// The write, encoded, would be `bytes` against a wire limit of `limit`:
    /// it cannot be SENT (craftworks-sdk#136).
    TooLargeToSend { bytes: u64, limit: usize },
    /// The ENGINE refused it as never acceptable: over `limit` of `bound` by
    /// `got`. Sending it again is refused again, so splitting it is the
    /// caller's call (craftworks-sdk#136).
    TooLarge { bound: protocol::WriteBound, limit: u32, got: u32 },
    /// The write changes a key it did not read (sdk#235, W8): refused at the
    /// engine's door. The SDK's bug, never the person's.
    Unread,
    /// This page has NO SESSION: the browser gave it no randomness to mint one
    /// (craftworks-sdk#146).
    NoSession,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::QueueFull { bytes, limit } => {
                write!(f, "{bytes} of {limit} bytes of writes are already waiting to be saved; wait for them, then try again")
            }
            Refused::NoSession => write!(
                f,
                "this page could not start a session (the browser gave it no randomness), so it cannot save; reload the page"
            ),
            Refused::TooLargeToSend { bytes, limit } => write!(
                f,
                "this write is {bytes} bytes as sent and the limit is {limit}; split it into smaller writes"
            ),
            Refused::TooLarge { bound, limit, got } => match bound {
                protocol::WriteBound::CommitBlocks => write!(
                    f,
                    "this write would change {got} blocks at once and the limit is {limit}; split it into smaller writes"
                ),
                protocol::WriteBound::WriteBytes => write!(
                    f,
                    "this write is {got} bytes and the limit is {limit}; split it into smaller writes"
                ),
            },
            Refused::Unread => write!(f, "the app's SDK wrote a key without reading it first; this is a bug in the SDK, not something you did"),
        }
    }
}

/// Sort by key and drop all but the LAST edit for each key.
///
/// Last-write-wins is the SDK's rule, not the tree's: a batch is a sequence the
/// caller wrote in order, and the tree takes a set. Doing it here means every
/// store sees the same batch, so the differential tests compare like with like.
pub fn sorted_edits(edits: Vec<(Vec<u8>, Edit)>) -> Vec<(Vec<u8>, Edit)> {
    let mut out: BTreeMap<Vec<u8>, Edit> = BTreeMap::new();
    for (k, e) in edits {
        out.insert(k, e);
    }
    out.into_iter().collect()
}

