//! THE PAGE'S STORE (READ-STATE, design B): reads WALK the tree from the one
//! owner's root over the blocks the page holds. No rows are copied, no range is
//! recorded, no root is kept here.
//!
//! # Why a walk and not a copy
//!
//! The row copy this replaces kept a second form of three facts the page
//! already owns: the head (the engine's), the tree (its blocks) and which
//! parts of it had been read (`loaded`/`stale`). Every defect of 2026-09-23
//! was two of those disagreeing (READ-STATE, § The measured case). Blocks are
//! content-addressed, so a cached block is never stale and nothing here is
//! ever invalidated.
//!
//! # A read that needs a block the page does not hold
//!
//! The walk stops, and the engine is asked to read the same range AT THE SAME
//! ROOT ([`Server::fetch`]): its parked read fetches through page-io on the
//! RTO estimator, and it always ENDS — answered, or given up within its rounds
//! and GETs. That read is the call's TICKET. The caller (`Db` through the
//! session) returns `NOT_LOADED` with it, the page waits for it to end
//! ([`PageStore::take_ended`]), and then RESUMES it ([`PageStore::resume`]):
//! the next call's walks are pinned to the ticket's root. So a chain of hops
//! (a schema, then its rows) converges on ONE immutable tree and never chases
//! a head that keeps moving (READ-STATE inv. 2; the architect's chase).
//!
//! # Writes (R-b)
//!
//! Framed by [`Writes`] and handed to the engine AT ONCE: the door is
//! synchronous in the page, so a refusal (`QueueFull`, `TooLarge`, `Unread`)
//! is the caller's return value. The ENGINE owns the queue; a write it has
//! taken is in the warm root reads walk, or -- still `Applying` -- a read of
//! its keys WAITS for it (never an overlay). What became of each write is
//! PULLED from the Server ([`PageStore::sync`]) and handed to the notices.
//!
//! [`Server::fetch`]: page::server::Server::fetch

use crate::store::{Delta, Edit, IdWidth, Read, Reads, Refused, RowState, Store, StoreError};
use crate::writes::Writes;
use engine::read::{DeltaSpec, ReadResult, ScanSpec, Walk, Walked};
use freenet_prolly::Cid;
use page::server::{Fetched, Host};
use std::collections::BTreeMap;
use std::ops::Bound;

/// Rows in key order, as a walk answers them.
type Rows = Vec<(Vec<u8>, Vec<u8>)>;

/// How a ticket ended: the code a parked read is woken with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The engine's read at the ticket's root finished: walk again, pinned.
    Loaded,
    /// A block could not be had: not "absent", and never answered as empty.
    Unavailable,
    /// A newer head was adopted while this read, pinned to an older root,
    /// had made no progress (#330 ruling): nothing arrived for its chain,
    /// and it waits on a block unanswered or answered NotFound. The WHOLE
    /// chain runs again, unpinned, at the head — one tree per chain (inv. 2).
    Superseded,
}

impl Ended {
    /// The stable code that crosses to JavaScript.
    pub fn code(self) -> &'static str {
        match self {
            Ended::Loaded => "LOADED",
            Ended::Unavailable => "UNAVAILABLE",
            Ended::Superseded => "SUPERSEDED",
        }
    }
}

/// What a `Db` call came back as, once a refusal for want of blocks has been
/// given its ticket ([`PageStore::decide`]).
#[derive(Debug)]
pub enum Outcome<T> {
    /// It worked.
    Done(T),
    /// A walk stopped on blocks the page does not hold; the engine is reading
    /// them at the walk's root, and this ticket ends when it has.
    Wait(crate::db::DbError, u64),
    /// Nothing more is coming.
    Told(crate::db::DbError),
}

/// Entries a walk asks for per page. A walk loops pages until its limit or the
/// range's end; this bounds only one step's working set.
const WALK_PAGE: usize = 4096;

/// A read that stopped on missing blocks, waiting for the engine's fetch.
#[derive(Debug, Clone)]
struct Ticket {
    /// The root its walk stopped at; `None` before the head was recovered
    /// (the engine reads at the head once it is).
    root: Option<Cid>,
    /// When it was made, or when it ended.
    at_ms: u64,
    ended: Option<Ended>,
    /// A read that waits on this page's own write still APPLYING in its
    /// range (R-b), not on a fetch: it ends when no such write is left, and
    /// resumes UN-PINNED at the head that write has joined (the pin trap).
    queue_wait: Option<(Vec<u8>, Vec<u8>)>,
    /// Its CHAIN has made progress: it was made by a walk resumed from a
    /// ticket that ended `Loaded` (blocks arrived for an earlier hop). A
    /// chain that progressed finishes on its tree; only one that has not may
    /// be superseded (#330 ruling).
    progressed: bool,
}

/// A fetch asked for before there was a page to ask (the store exists from
/// the session's first line; the page from provisioning on).
type Deferred = (u64, Option<Cid>, freenet_prolly::range::Range);

/// How many ENDED tickets are kept for a `resume` that may still come
/// ([`PageStore::forget_old_ended`]).
pub const KEEP_ENDED: usize = 64;

/// The page's store: `Db` reads and writes through it, over a [`Host`].
pub struct PageStore<H: Host> {
    host: Option<H>,
    /// The write half: framing and what the app is told.
    pub writes: Writes,
    now_ms: Box<dyn Fn() -> u64>,
    tickets: BTreeMap<u64, Ticket>,
    next_ticket: u64,
    /// Tickets that ended since the page last asked, in order.
    ended: Vec<(u64, Ended)>,
    /// The ticket the last failed read made, for the caller to hand out.
    last_ticket: Option<u64>,
    /// The root this call's walks are pinned to ([`PageStore::resume`]).
    pinned: Option<Cid>,
    /// The pin came from a ticket that ended `Loaded`: a ticket this call
    /// makes continues a chain that has made progress.
    pinned_progressed: bool,
    /// The Server's adoption count last seen: a newer one is a head moved
    /// past older roots, when stalled tickets are superseded.
    seen_adoptions: u64,
    deferred: Vec<Deferred>,
    /// Walks made — what a test and the cost measurement read.
    pub walks: u64,
    /// The root this store's last ANSWERED read walked: what a LIVE binding
    /// records as its `RenderedAt` when its read completes.
    answered_at: Option<Cid>,
    /// A VIEW of somebody's published head (`open_named`): it writes nothing,
    /// so its warm root IS its published root — asserted where `head()`
    /// answers, so a future "preview as another user" of one's OWN tree cannot
    /// inherit the warm root silently (READ-STATE inv. 6).
    view: bool,
    /// Why a ticket ended UNAVAILABLE, in the engine's words, until it is
    /// taken with its end ([`PageStore::why`]).
    why: BTreeMap<u64, String>,
    /// Whether a read covering this page's own write still APPLYING waits
    /// for it (R-b). Always on; off only as the model test's control, which
    /// must then see a read that misses its own write.
    pub wait_on_applying: bool,
    /// Tries the next write made had already spent: a `Db` re-run's (ONE
    /// budget, sdk#265). Handed to the engine with that write, then 0.
    carry: u32,
}

impl<H: Host> PageStore<H> {
    pub fn new(now_ms: Box<dyn Fn() -> u64>, writes_clock: Box<dyn Fn() -> u64>) -> Self {
        PageStore {
            host: None,
            writes: Writes::new(writes_clock),
            now_ms,
            tickets: BTreeMap::new(),
            next_ticket: 1,
            ended: Vec::new(),
            last_ticket: None,
            pinned: None,
            pinned_progressed: false,
            seen_adoptions: 0,
            deferred: Vec::new(),
            walks: 0,
            answered_at: None,
            carry: 0,
            view: false,
            why: BTreeMap::new(),
            wait_on_applying: true,
        }
    }

    /// The page exists: reads walk its tree from now on, and any fetch asked
    /// for before it goes now.
    pub fn set_host(&mut self, host: H) {
        self.host = Some(host);
        for (ticket, root, range) in std::mem::take(&mut self.deferred) {
            self.host.as_mut().expect("just set").with_server(|s| s.fetch(ticket, root, range));
        }
        self.sync();
    }

    pub fn host(&self) -> Option<&H> {
        self.host.as_ref()
    }

    pub fn host_mut(&mut self) -> Option<&mut H> {
        self.host.as_mut()
    }

    /// Carry what the client produced to the page and what the page answered
    /// back: every write frame is handed to the engine NOW (the door is
    /// synchronous), every reply reaches the write half, and every fetch that
    /// ended ends its ticket.
    pub fn sync(&mut self) {
        let Some(h) = self.host.as_mut() else { return };
        for _ in 0..64 {
            let frames = self.writes.take_outbound();
            for f in &frames {
                h.client(f);
            }
            let replies = h.take_replies();
            for r in &replies {
                self.writes.on_inbound(r);
            }
            if frames.is_empty() && replies.is_empty() {
                break;
            }
        }
        // WHAT BECAME OF THIS CLIENT'S WRITES, pulled (R-b): conflicts first
        // (their chains read the fates), then every terminal fate, read.
        if let Some(session) = self.writes.client.session() {
            let (chains, fates, backed) = h.with_server(|s| (s.take_conflicted(session), s.take_fates(session), s.take_backed_up(session)));
            self.writes.on_conflicted(chains);
            for (write_id, fate) in fates {
                self.writes.on_fate(write_id, fate);
            }
            self.writes.on_backed_up(backed);
        }
        let fetched = h.with_server(|s| s.take_fetched());
        let now = (self.now_ms)();
        for (id, how) in fetched {
            let how = match how {
                Fetched::Loaded => Ended::Loaded,
                Fetched::Unavailable(why) => {
                    self.why.insert(id, why);
                    Ended::Unavailable
                }
            };
            self.end(id, how, now);
        }
        // A NEWER HEAD (#330 ruling): every open ticket pinned to an older
        // root whose chain has made no progress, and whose engine read has
        // had no block since it started while it waits on one, ends
        // SUPERSEDED — its read withdrawn — and the chain runs again at the
        // head. Answer-driven (the head moved), never a clock (rule 8); a
        // chain receiving blocks finishes on its tree (inv. 2).
        let (adoptions, head) = {
            let h = self.host.as_ref().expect("checked");
            (h.peek(|s| s.adoptions()), h.peek(|s| s.read_root()))
        };
        if adoptions > self.seen_adoptions {
            self.seen_adoptions = adoptions;
            let candidates: Vec<u64> = self
                .tickets
                .iter()
                .filter(|(_, t)| t.ended.is_none() && t.queue_wait.is_none() && !t.progressed)
                .filter(|(_, t)| t.root.is_some() && t.root != head)
                .map(|(id, _)| *id)
                .collect();
            for id in candidates {
                let h = self.host.as_mut().expect("checked");
                if h.with_server(|s| s.supersede_fetch(id)) {
                    self.end(id, Ended::Superseded, now);
                }
            }
        }
        // Reads waiting on this page's own write still applying: ended once
        // no such write is left in their range.
        let waiting: Vec<(u64, Vec<u8>, Vec<u8>)> = self
            .tickets
            .iter()
            .filter(|(_, t)| t.ended.is_none())
            .filter_map(|(id, t)| t.queue_wait.clone().map(|(lo, hi)| (*id, lo, hi)))
            .collect();
        for (id, lo, hi) in waiting {
            let h = self.host.as_ref().expect("checked");
            if !h.peek(|s| s.applying_touches(&lo, &hi)) {
                self.end(id, Ended::Loaded, now);
            }
        }
    }

    /// The oldest of this client's writes still `Applying`, asked after once
    /// it has waited (sdk#174): its path may need more blocks than one chain
    /// fetches, and only its client asking starts the next.
    pub fn ask_after_applying(&mut self) -> Option<u64> {
        let session = self.writes.client.session()?;
        let first = self.host.as_ref()?.peek(|s| s.first_applying_of(session));
        let asked = self.writes.ask_after(first);
        self.sync();
        asked
    }

    /// This client's writes the page has not finished: every write not yet
    /// `Published` or ended (craftworks-sdk#163) -- what a closing tab loses.
    /// A held DEFERRED write (an app's define on open, sdk#350) is not one:
    /// the engine's rule, counted for this session, not a second tally.
    pub fn unsaved_writes(&self) -> usize {
        match (self.host.as_ref(), self.writes.client.session()) {
            (Some(h), Some(session)) => h.peek(|s| s.unsaved_of(session)),
            _ => 0,
        }
    }

    /// The page's queue: writes and bytes, every session's.
    pub fn queue_load(&self) -> (usize, usize) {
        self.host.as_ref().map_or((0, 0), |h| h.peek(|s| s.queue_load()))
    }

    /// Hand a made write to the engine and collect the door's verdict in
    /// the same call: a refusal is the caller's return value, never a notice
    /// found later.
    fn hand_over(&mut self, made: Result<u64, Refused>) -> Result<(), Refused> {
        let carry = std::mem::take(&mut self.carry);
        let write_id = made?;
        let Some(h) = self.host.as_mut() else { return Ok(()) };
        if carry > 0 {
            h.with_server(|s| s.carry_tries(carry));
        }
        for f in self.writes.take_outbound() {
            h.client(&f);
        }
        let session = self.writes.client.session().expect("a made write has a session");
        let door = h.with_server(|s| s.fate(session, write_id));
        let refused = match door {
            Some(page::fates::Fate::QueueFull { bytes, limit }) => Some(Refused::QueueFull { bytes, limit }),
            Some(page::fates::Fate::TooLarge { bound, limit, got }) => Some(Refused::TooLarge {
                bound: match bound {
                    engine::WriteBound::CommitBlocks => protocol::WriteBound::CommitBlocks,
                    engine::WriteBound::WriteBytes => protocol::WriteBound::WriteBytes,
                },
                limit: protocol::saturating_u32(limit),
                got: protocol::saturating_u32(got),
            }),
            Some(page::fates::Fate::Unread { .. }) => Some(Refused::Unread),
            Some(f) if f.terminal() => {
                self.writes.on_fate(write_id, f);
                None
            }
            _ => None,
        };
        if let Some(why) = refused {
            self.writes.refused_at_door(write_id, why);
        }
        self.sync();
        refused.map_or(Ok(()), Err)
    }

    fn end(&mut self, id: u64, how: Ended, now: u64) {
        if let Some(t) = self.tickets.get_mut(&id) {
            if t.ended.is_none() {
                t.ended = Some(how);
                t.at_ms = now;
                self.ended.push((id, how));
            }
        }
        self.forget_old_ended();
    }

    /// AN ENDED TICKET THE APP NEVER RESUMES is forgotten once
    /// [`KEEP_ENDED`] newer ones have ended — a COUNT over finished reads,
    /// never a clock on a waiting one (rule 8). It was the old lifetime's
    /// second job (engineer2, sdk#302 review): without it, every abandoned
    /// read kept its entry for the life of the page. Forgetting pins
    /// nothing wrong: `resume` of a forgotten ticket reads unpinned, at the
    /// head. An OPEN ticket is never forgotten.
    fn forget_old_ended(&mut self) {
        let mut ended: Vec<(u64, u64)> = self.tickets.iter().filter(|(_, t)| t.ended.is_some()).map(|(id, t)| (t.at_ms, *id)).collect();
        if ended.len() <= KEEP_ENDED {
            return;
        }
        ended.sort();
        for (_, id) in ended.drain(..ended.len() - KEEP_ENDED) {
            self.tickets.remove(&id);
            self.why.remove(&id);
        }
    }

    /// Tickets that ended since the last call, in order. Drains.
    pub fn take_ended(&mut self) -> Vec<(u64, Ended)> {
        self.sync();
        std::mem::take(&mut self.ended)
    }

    /// Why this ended ticket did not deliver, if the engine said. Drains.
    pub fn why(&mut self, ticket: u64) -> Option<String> {
        self.why.remove(&ticket)
    }

    /// The ticket the last refused read made. Drains.
    pub fn take_ticket(&mut self) -> Option<u64> {
        self.last_ticket.take()
    }

    /// A parked read woke on `ticket`: the NEXT call's walks read at that
    /// ticket's root, so the chain ends on one tree. A ticket unknown, not
    /// ended, or made before the head was recovered pins nothing (the walk
    /// reads the head).
    pub fn resume(&mut self, ticket: u64) {
        self.pinned = match self.tickets.get(&ticket) {
            Some(Ticket { ended: Some(Ended::Loaded), root, .. }) => *root,
            _ => None,
        };
        self.pinned_progressed = self.pinned.is_some();
        self.tickets.remove(&ticket);
    }

    /// This store is a VIEW (READ-STATE inv. 6: another user's walk takes the
    /// published root).
    pub fn set_view(&mut self) {
        self.view = true;
    }

    /// The root the last answered read walked (see the field).
    pub fn answered_at(&self) -> Option<Cid> {
        self.answered_at
    }

    /// Pin this call's walks to `root` — what `resume` does for a ticket's
    /// root, for a caller that holds a root of its own (a control's copy).
    pub fn pin(&mut self, root: Cid) {
        self.pinned = Some(root);
    }

    /// THE DECISION, for every `Db` call a page makes: a `NotLoaded` is a walk
    /// that stopped on blocks this page does not hold, and this store has
    /// already asked the engine to read them at the walk's root — that read
    /// is the TICKET. A `NotLoaded` with no ticket, and anything else, is
    /// TOLD. The call's pin ends here, answered or not: the next call walks
    /// the head unless it is resumed.
    ///
    /// Here, not in the web `Session`, so something native can run it: the
    /// session is `#[wasm_bindgen]` in a `cdylib`, and a decision inline there
    /// is one only a fake session in JavaScript could reach (sdk#89).
    ///
    /// A write that had to READ before it could apply comes through here too:
    /// every read in `Db` happens before the single write that mutates, so a
    /// `NotLoaded` leaves the tree untouched and asking again repeats the
    /// attempt, not the effect.
    pub fn decide<T>(&mut self, r: Result<T, crate::db::DbError>) -> Outcome<T> {
        let ticket = self.take_ticket();
        self.unpin();
        match r {
            Ok(v) => Outcome::Done(v),
            Err(e) if e.code() == "NOT_LOADED" => match ticket {
                Some(t) => Outcome::Wait(e, t),
                None => Outcome::Told(e),
            },
            Err(e) => Outcome::Told(e),
        }
    }

    /// The call ended: its pin goes.
    pub fn unpin(&mut self) {
        self.pinned = None;
        self.pinned_progressed = false;
    }

    // NO TICKET LIFETIME (rules 7, 8): a ticket ENDS only on its answer —
    // the engine's read finished (`Loaded`) or could not have a block
    // (`Unavailable`) — and is removed when the read resumes. While it waits,
    // the page's sender re-sends the node request under it, and the page's
    // `not_answering` says for how long.

    /// Tickets still open (not ended). What the model asserts is zero at rest.
    pub fn open_tickets(&self) -> usize {
        self.tickets.values().filter(|t| t.ended.is_none()).count()
    }

    /// The root this page's walks read now: the pinned one, or the owner's
    /// (`Server::read_root`, the warm root). `None` before the head is
    /// recovered, or before there is a page.
    pub fn head(&mut self) -> Option<Cid> {
        if let Some(p) = self.pinned {
            return Some(p);
        }
        let view = self.view;
        self.host.as_mut()?.with_server(|s| {
            let warm = s.read_root()?;
            if view {
                // A view writes nothing: its warm root is its published one,
                // and it is the published root another user's walk takes.
                let published = s.page.published().1;
                assert_eq!(warm, published, "a VIEW's warm root differs from its published root: something wrote into a view");
                return Some(published);
            }
            Some(warm)
        })
    }


    /// A walk stopped: the engine reads `range` at `root`, and the read is
    /// this call's ticket.
    fn park(&mut self, root: Option<Cid>, range: freenet_prolly::range::Range) -> StoreError {
        let id = self.next_ticket;
        self.next_ticket += 1;
        let now = (self.now_ms)();
        self.tickets.insert(id, Ticket { root, at_ms: now, ended: None, queue_wait: None, progressed: self.pinned_progressed });
        self.last_ticket = Some(id);
        match self.host.as_mut() {
            Some(h) => h.with_server(|s| s.fetch(id, root, range)),
            None => self.deferred.push((id, root, range)),
        }
        self.sync();
        StoreError::NotLoaded
    }

    fn walk(&mut self, root: &Cid, w: &Walk) -> Walked {
        self.walks += 1;
        match self.host.as_mut() {
            Some(h) => h.with_server(|s| s.walk(root, w)),
            None => Walked::Need(Vec::new()),
        }
    }

    /// Every entry of `[lo, hi)` at `root`, in order, at most `limit`: pages
    /// looped. `Err` is the scan to fetch from where the walk stopped.
    fn walk_scan(&mut self, root: Cid, lo: &[u8], hi: &[u8], reverse: bool, limit: usize) -> Result<Rows, WalkStop> {
        let mut out: Rows = Vec::new();
        let mut after: Option<Vec<u8>> = None;
        while out.len() < limit {
            let spec = ScanSpec {
                lo: Bound::Included(lo.to_vec()),
                hi: Bound::Excluded(hi.to_vec()),
                reverse,
                after: after.clone(),
                max_entries: (limit - out.len()).min(WALK_PAGE),
                max_bytes: usize::MAX,
            };
            match self.walk(&root, &Walk::Scan(Box::new(spec.clone()))) {
                Walked::Done(ReadResult::Page { entries, cursor, .. }) => {
                    out.extend(entries);
                    match cursor {
                        Some(c) => after = Some(c),
                        None => break,
                    }
                }
                Walked::Need(_) => return Err(WalkStop::Need((&spec).into())),
                Walked::Done(_) | Walked::Broken(_) => return Err(WalkStop::Unavailable),
            }
        }
        Ok(out)
    }

    /// The root to read, or the ticket to wait on when there is none yet --
    /// or while this page's own write in `[lo, hi)` is still APPLYING (R-b):
    /// it is in no root, and a read that did not wait for it would miss it.
    /// That ticket resumes UN-PINNED, at the head the write has joined.
    fn root_or_park(&mut self, lo: &[u8], hi: &[u8]) -> Result<Cid, StoreError> {
        let applying = self.wait_on_applying && self.host.as_ref().is_some_and(|h| h.peek(|s| s.applying_touches(lo, hi)));
        if applying {
            let id = self.next_ticket;
            self.next_ticket += 1;
            let now = (self.now_ms)();
            self.tickets.insert(id, Ticket { root: None, at_ms: now, ended: None, queue_wait: Some((lo.to_vec(), hi.to_vec())), progressed: false });
            self.last_ticket = Some(id);
            return Err(StoreError::NotLoaded);
        }
        match self.head() {
            Some(r) => Ok(r),
            None => Err(self.park(None, scan_range(lo, hi, 1))),
        }
    }

    /// What this key's OWN write is doing: rolled back (this client was told
    /// its write ended unpublished), else the Server's `key_state` (R-b).
    pub fn row_state_of(&self, key: &[u8]) -> RowState {
        if self.writes.rolled_back(key) {
            return RowState::RolledBack;
        }
        let Some(h) = self.host.as_ref() else { return RowState::Clean };
        match h.peek(|s| s.key_stage(key)) {
            // Not gone yet: behind the commit in flight, or its path fetching.
            Some(engine::Stage::Applying | engine::Stage::Queued) => RowState::Queued,
            // Gone, and nothing has confirmed it: closing the tab loses it.
            Some(engine::Stage::Committing) => RowState::Pending,
            None => match h.peek(|s| s.key_state(key)) {
                Some(page::server::KeyState::Saving) => RowState::Pending,
                Some(page::server::KeyState::SavedAndBackedUp) => RowState::BackedUp,
                _ => RowState::Clean,
            },
        }
    }
}

/// Why a walk did not answer.
enum WalkStop {
    Need(freenet_prolly::range::Range),
    Unavailable,
}

fn scan_range(lo: &[u8], hi: &[u8], max_entries: usize) -> freenet_prolly::range::Range {
    freenet_prolly::range::Range {
        lo: Bound::Included(lo.to_vec()),
        hi: Bound::Excluded(hi.to_vec()),
        reverse: false,
        after: None,
        max_entries,
        max_bytes: usize::MAX,
    }
}

/// The key just after `key`: `[key, next(key))` holds exactly `key`.
fn next_key(key: &[u8]) -> Vec<u8> {
    let mut k = key.to_vec();
    k.push(0);
    k
}

impl<H: Host> Reads for PageStore<H> {
    fn get(&mut self, key: &[u8]) -> Read<Option<Vec<u8>>> {
        self.sync();
        let root = self.root_or_park(key, &next_key(key))?;
        match self.walk(&root, &Walk::Get(key.to_vec())) {
            Walked::Done(ReadResult::Value(v)) => {
                self.answered_at = Some(root);
                Ok(v)
            }
            Walked::Need(_) => Err(self.park(Some(root), scan_range(key, &next_key(key), 1))),
            Walked::Done(_) | Walked::Broken(_) => Err(StoreError::Unavailable),
        }
    }

    fn row_state(&self, key: &[u8]) -> RowState {
        self.row_state_of(key)
    }

    fn wrong_width(&self, given: IdWidth, wanted: IdWidth) {
        self.writes.client.record_wrong_width(given, wanted);
    }

    fn scan(&mut self, lo: &[u8], hi: &[u8], reverse: bool, limit: usize) -> Read<Vec<(Vec<u8>, Vec<u8>)>> {
        self.sync();
        if lo >= hi {
            return Ok(Vec::new());
        }
        let root = self.root_or_park(lo, hi)?;
        match self.walk_scan(root, lo, hi, reverse, limit) {
            Ok(rows) => {
                self.answered_at = Some(root);
                Ok(rows)
            }
            Err(WalkStop::Need(range)) => Err(self.park(Some(root), range)),
            Err(WalkStop::Unavailable) => Err(StoreError::Unavailable),
        }
    }

    fn root(&mut self) -> Read<[u8; 32]> {
        self.head().ok_or(StoreError::NotLoaded)
    }

    /// The tree's own diff, walked here: what changed in `[lo, hi)` from
    /// `from` to this store's root. Blocks missing (from `from`'s side or the
    /// root's) is a full read, the defined answer — never a shorter list.
    fn changes_since(&mut self, from: [u8; 32], lo: &[u8], hi: &[u8], max_entries: u32) -> Read<Delta> {
        let root = self.head().ok_or(StoreError::NotLoaded)?;
        let spec = DeltaSpec { from, lo: Bound::Included(lo.to_vec()), hi: Bound::Excluded(hi.to_vec()), max_entries: max_entries.max(1) as usize };
        match self.walk(&root, &Walk::Delta(Box::new(spec))) {
            Walked::Done(ReadResult::Delta { changes, cursor, new_root }) => Ok(Delta::Changes { changes, cursor, new_root }),
            _ => Ok(Delta::FullReloadRequired { new_root: root }),
        }
    }
}

impl<H: Host> Store for PageStore<H> {
    /// A store-level batch that cannot read first: it says so, key by key,
    /// as `Expect::Any` (sdk#235, W8) -- counted by the engine and shown.
    /// Only store-level callers build `Any`; `Db` refuses to.
    fn apply_batch(&mut self, edits: &[(Vec<u8>, Edit)]) -> Result<(), Refused> {
        let reads: Vec<(Vec<u8>, protocol::Expect)> = edits.iter().map(|(k, _)| (k.clone(), protocol::Expect::Any)).collect();
        let made = self.writes.make(&reads, edits);
        self.hand_over(made)
    }

    fn last_write_id(&self) -> Option<u64> {
        self.writes.last_write_id()
    }

    /// Not yet ended: in the engine's queue, or ended and not yet pulled.
    fn is_pending_write(&self, write_id: u64) -> bool {
        self.writes.is_open(write_id)
    }

    fn take_conflict_chains(&mut self) -> Vec<crate::store::ConflictChain> {
        self.sync();
        self.writes.take_conflict_chains()
    }

    /// The next write made goes to the engine with these tries spent.
    fn carry_tries(&mut self, tries: u32) {
        self.carry = tries;
    }

    /// The engine's: the one budget every write draws on.
    fn max_write_tries(&self) -> u32 {
        self.host.as_ref().map_or(engine::Params::default().max_write_tries, |h| h.peek(|s| s.max_write_tries()))
    }

    /// Made, then handed to the engine in the same call: the door's verdict
    /// (taken, or refused by name) is known when this returns.
    fn apply_commit(&mut self, reads: &[(Vec<u8>, protocol::Expect)], edits: &[(Vec<u8>, Edit)]) -> Result<(), Refused> {
        let made = self.writes.make(reads, edits);
        self.hand_over(made)
    }

    fn apply_deferred_commit(&mut self, reads: &[(Vec<u8>, protocol::Expect)], edits: &[(Vec<u8>, Edit)]) -> Result<(), Refused> {
        let made = self.writes.make_as(reads, edits, true);
        self.hand_over(made)
    }
}

#[cfg(test)]
mod ended_tickets {
    use super::*;

    /// Never called: the store under test has no host.
    struct NoHost;
    impl Host for NoHost {
        fn with_server<R>(&mut self, _: impl FnOnce(&mut page::server::Server) -> R) -> R {
            unreachable!("no host")
        }
        fn peek<R>(&self, _: impl FnOnce(&page::server::Server) -> R) -> R {
            unreachable!("no host")
        }
        fn client(&mut self, _: &[u8]) {}
        fn take_replies(&mut self) -> Vec<Vec<u8>> {
            Vec::new()
        }
    }

    fn store() -> PageStore<NoHost> {
        PageStore::new(Box::new(|| 0), Box::new(|| 0))
    }

    fn open(s: &mut PageStore<NoHost>, id: u64, now: u64) {
        s.tickets.insert(id, Ticket { root: None, at_ms: now, ended: None, queue_wait: None, progressed: false });
    }

    /// An abandoned read's ticket is not kept for the life of the page: of
    /// 200 ended and never resumed, the newest KEEP_ENDED stay, and an OPEN
    /// ticket among them is never forgotten.
    #[test]
    fn ended_tickets_the_app_never_resumes_are_forgotten_beyond_the_newest_few() {
        let mut s = store();
        open(&mut s, 10_000, 0);
        for id in 1..=200u64 {
            open(&mut s, id, id);
            s.why.insert(id, "gone".into());
            s.end(id, Ended::Unavailable, 1_000 + id);
        }
        let ended: Vec<u64> = s.tickets.iter().filter(|(_, t)| t.ended.is_some()).map(|(id, _)| *id).collect();
        assert_eq!(ended.len(), KEEP_ENDED, "abandoned ended tickets are kept for ever: {}", ended.len());
        assert_eq!(ended, (200 - KEEP_ENDED as u64 + 1..=200).collect::<Vec<_>>(), "not the NEWEST ended tickets were kept");
        assert!(s.tickets.get(&10_000).is_some_and(|t| t.ended.is_none()), "an OPEN ticket was forgotten");
        assert_eq!(s.why.len(), KEEP_ENDED, "the reasons of forgotten tickets were kept");
        // A forgotten ticket resumes UNPINNED (reads at the head), a kept one pinned to its root.
        s.resume(1);
        assert_eq!(s.pinned, None);
    }
}
