//! THE WRITE HALF of the page's store (R-b; READ-STATE § The pull API).
//!
//! The ENGINE owns the write queue: order, the rebuild, `Lost` and go-back-N,
//! the bound (`QueueFull`), every verdict. What is left here is what only
//! this client can know:
//! * the FRAMING: a write made here is minted an id and framed onto the wire
//!   ([`Client`]), and handed to the engine at once (the door is synchronous
//!   in the page);
//! * which keys each of ITS OWN writes touched, until the write ends -- so a
//!   row whose write ended can be named (re-rendered, or shown rolled back);
//! * what the APP has been told: fates are PULLED from the page's `Server`
//!   ([`Writes::on_fate`]) and turned into the notices the app drains.
//!
//! Nothing here times a write out, re-sends one, or holds one back: a queued
//! write's only exits are the named fates the engine gives it (sdk#291: the
//! outbox's 60 s timeout rolled back writes that were only waiting their
//! turn -- 34 of 3,000 paced puts, gone, with "saving" reading 0).

use crate::engine_client::Client;
use crate::store::{Edit, Refused};
use page::fates::Fate;
use protocol::Request;
use std::collections::{BTreeMap, BTreeSet};

/// How long a write may stay `Applying` before this client asks after it
/// (sdk#174): a write whose path needs more blocks than one chain may fetch
/// asks for nothing more until its client asks (`AskWrite`), which starts a
/// new chain. At most one ask outstanding, a second apart.
pub const ASK_AFTER_MS: u64 = 1_000;

/// A Published write whose `keys` another device of the same identity
/// replaced (sdk#225b): those rows show the winner's values now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Superseded {
    pub write_id: u64,
    pub keys: Vec<Vec<u8>>,
}

/// A write refused because the SDK wrote a key it did not read (sdk#235,
/// W8), and the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unread {
    pub write_ids: Vec<u64>,
    pub key: Option<Vec<u8>>,
}

/// A write that did not apply because a key it READ had moved (M2): which
/// write, which key, what the tree holds there now (`None`: absent), and the
/// earlier write whose own end it cascades from (`after`, footnote 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflicted {
    pub write_id: u64,
    pub key: Vec<u8>,
    pub current: Option<[u8; 32]>,
    pub after: Option<u64>,
}

/// A write that ended without being published, and why: the fates the app
/// is told by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// Nothing of it is in the tree, for the cause named (sdk#500); `signer`:
    /// the signer's own why beside `SignerRefused`.
    Failed { why: engine::FailWhy, signer: Option<String> },
    Lost,
    ForcedLost,
    TooLarge { bound: engine::WriteBound, limit: usize, got: usize },
    /// It may have landed: the app is told to check (COMMIT-LIFE ⁵).
    Unknown,
}

/// The write half: framing, this client's own writes' keys, and the notices.
pub struct Writes {
    pub client: Client,
    next_write_id: u64,
    now_ms: Box<dyn Fn() -> u64>,
    /// Keys each of this client's writes touched, until the write ends.
    keys_of: BTreeMap<u64, Vec<Vec<u8>>>,
    /// Writes made with an `Expect::Any` read (sdk#235): a `Lost` one is a
    /// forced write that fell, named as such.
    forced: BTreeSet<u64>,
    /// Keys whose last write of this client's ended unpublished. Kept because
    /// the fact outlives the write: a row asking afterwards would otherwise be
    /// told "saved" about a write that never landed. Cleared when the key is
    /// written again.
    rolled_back: BTreeSet<Vec<u8>>,
    /// `QueueFull` fates that reached `on_fate` for a still-OPEN write (sdk#450): the door's verdict arriving after
    /// the door, which `hand_over` rules out. Loud in debug; in release ignored and COUNTED here, never a panic (a
    /// panic is a dead wasm page). TEMPORARY: outside the recording spine until the recorder reaches Writes
    /// (sdk#457), when this becomes a recorded diagnostic and the field is deleted.
    pub late_door_verdicts: u64,
    conflicts: Vec<Conflicted>,
    unread: Vec<Unread>,
    ended: Vec<(u64, Ended)>,
    /// Writes that ended Published, in the order the page said so. Drained.
    published: Vec<u64>,
    superseded: Vec<Superseded>,
    state_changed: Vec<Vec<u8>>,
    chains: Vec<crate::store::ConflictChain>,
    /// When this client last asked after an `Applying` write (sdk#174).
    asked_at: Option<u64>,
}

impl Writes {
    pub fn new(now_ms: Box<dyn Fn() -> u64>) -> Writes {
        Writes {
            client: Client::new(),
            next_write_id: 1,
            now_ms,
            keys_of: BTreeMap::new(),
            forced: BTreeSet::new(),
            rolled_back: BTreeSet::new(),
            late_door_verdicts: 0,
            conflicts: Vec::new(),
            unread: Vec::new(),
            ended: Vec::new(),
            published: Vec::new(),
            superseded: Vec::new(),
            state_changed: Vec::new(),
            chains: Vec::new(),
            asked_at: None,
        }
    }

    /// The write id the next write will carry.
    pub fn next_write_id(&self) -> u64 {
        self.next_write_id
    }

    pub fn last_write_id(&self) -> Option<u64> {
        self.next_write_id.checked_sub(1).filter(|id| *id > 0)
    }

    /// Make a write: an id, framed onto the wire -- or REFUSED before it is
    /// framed, and the refusal returned. It says what it READ (M2). The
    /// engine's own door verdict is the caller's to collect
    /// ([`crate::page_store::PageStore`] does, in the same call).
    pub fn make(&mut self, reads: &[(Vec<u8>, protocol::Expect)], edits: &[(Vec<u8>, Edit)]) -> Result<u64, Refused> {
        self.make_as(reads, edits, false)
    }

    /// [`Writes::make`], DEFERRED or not (sdk#350): a deferred write goes as
    /// `DeferredCommit`, the wire's form of the one flag -- same id, same
    /// refusals, same fate bookkeeping.
    pub fn make_as(&mut self, reads: &[(Vec<u8>, protocol::Expect)], edits: &[(Vec<u8>, Edit)], deferred: bool) -> Result<u64, Refused> {
        // The id this write WILL carry, taken only once it is sent: a write
        // refused here never reaches the wire, so it uses up no id and the
        // ids a node sees have no gap (sdk#251).
        let write_id = self.next_write_id;
        // NO SESSION, NO WRITE (sdk#146): refused by name, rather than sent
        // under a number another tab shares.
        if self.client.session().is_none() {
            return Err(Refused::NoSession);
        }
        let ops: Vec<protocol::Op> = edits
            .iter()
            .map(|(k, e)| match e {
                Edit::Put(v) => protocol::Op::Put(k.clone(), v.clone()),
                Edit::Delete => protocol::Op::Delete(k.clone()),
            })
            .collect();
        let request = if deferred {
            Request::DeferredCommit { write_id, reads: reads.to_vec(), ops }
        } else if reads.is_empty() {
            Request::Write { write_id, ops }
        } else {
            Request::Commit { write_id, reads: reads.to_vec(), ops }
        };
        // WILL IT FIT ON THE WIRE? (craftworks-sdk#136)
        let bytes = protocol::request_len(protocol::CURRENT, &request);
        if bytes > protocol::MAX_MESSAGE as u64 {
            return Err(Refused::TooLargeToSend { bytes, limit: protocol::MAX_MESSAGE });
        }
        let keys: Vec<Vec<u8>> = edits.iter().map(|(k, _)| k.clone()).collect();
        for k in &keys {
            // Something is in flight for it now: a truer answer than an old
            // failure.
            self.rolled_back.remove(k);
        }
        self.next_write_id += 1;
        self.keys_of.insert(write_id, keys);
        if reads.iter().any(|(_, e)| *e == protocol::Expect::Any) {
            self.forced.insert(write_id);
        }
        self.client.send(&request);
        Ok(write_id)
    }

    /// The door refused `write_id` in the call that made it: the caller gets
    /// the refusal, so the write leaves no notice behind.
    pub fn refused_at_door(&mut self, write_id: u64, why: Refused) {
        self.keys_of.remove(&write_id);
        self.forced.remove(&write_id);
        let _ = why;
    }

    /// Is this one of this client's writes that has not ended?
    pub fn is_open(&self, write_id: u64) -> bool {
        self.keys_of.contains_key(&write_id)
    }

    /// Writes of this client's not yet ended.
    pub fn open_writes(&self) -> usize {
        self.keys_of.len()
    }

    /// A TERMINAL fate the Server has for one of this client's writes.
    pub fn on_fate(&mut self, write_id: u64, fate: Fate) {
        // Only a write of this client's that is still open: a fate is told
        // once, and a write refused at its door was told by its return.
        if !fate.terminal() || !self.keys_of.contains_key(&write_id) {
            return;
        }
        // THE DOOR'S VERDICT ONLY (sdk#450): the engine says `QueueFull` at admission, in the step that took the
        // write's frame, and `hand_over` takes it in that same call (`refused_at_door` closes the write), so an open
        // write never meets it here. Not an end: the SDK waits for room and makes the write again. If one ever does
        // arrive: loud in debug, and in release nothing is ended or named -- it is counted.
        if matches!(fate, Fate::QueueFull { .. }) {
            debug_assert!(false, "QueueFull reached on_fate for open write {write_id}: the door's verdict after the door");
            self.late_door_verdicts += 1;
            return;
        }
        let keys = self.keys_of.remove(&write_id).unwrap_or_default();
        let forced = self.forced.remove(&write_id);
        self.state_changed.extend(keys.iter().cloned());
        let ended = match fate {
            Fate::Published { .. } => {
                self.published.push(write_id);
                return;
            }
            Fate::Conflict { key, current, after } => {
                self.rolled_back.extend(keys);
                self.conflicts.push(Conflicted { write_id, key, current, after: after.map(|a| a.1) });
                return;
            }
            Fate::Unread { key } => {
                self.rolled_back.extend(keys);
                self.unread.push(Unread { write_ids: vec![write_id], key: Some(key) });
                return;
            }
            Fate::TooLarge { bound, limit, got } => Ended::TooLarge { bound, limit, got },
            Fate::Failed { why, signer } => Ended::Failed { why, signer },
            Fate::Lost if forced => Ended::ForcedLost,
            Fate::Lost => Ended::Lost,
            // May have landed: NOT rolled back (its row still shows it).
            Fate::Unknown => {
                self.ended.push((write_id, Ended::Unknown));
                return;
            }
            // Returned above, before anything was taken (not terminal; the door's QueueFull): never a panic.
            Fate::Applying | Fate::Queued | Fate::Committing | Fate::QueueFull { .. } => return,
        };
        self.rolled_back.extend(keys);
        self.ended.push((write_id, ended));
    }

    /// Keys whose last write of this client's became BACKED_UP (sdk#415): its fate ended at `Published`, so this
    /// is its row's state changing -- its bindings re-read, as for every own-write state change (builder#107).
    pub fn on_backed_up(&mut self, keys: Vec<Vec<u8>>) {
        self.state_changed.extend(keys);
    }

    /// Conflict chains the Server named for this client (`Db`'s re-run).
    pub fn on_conflicted(&mut self, chains: Vec<(Vec<u64>, Vec<Vec<u8>>, u32)>) {
        self.chains.extend(chains.into_iter().map(|(write_ids, keys, tries)| crate::store::ConflictChain { write_ids, keys, tries }));
    }

    /// A reply arrived: the client's own bookkeeping (framing, the tick
    /// gate, traces), and the one verdict that is still PUSHED -- another
    /// device replaced rows of a published write (sdk#225b).
    pub fn on_inbound(&mut self, bytes: &[u8]) {
        if let Ok(r) = protocol::decode_reply(bytes) {
            // A pushed write state decides nothing here (its fate is pulled);
            // one naming another session, or none, is still COUNTED as
            // foreign (sdk#146, the architect's A2 on sdk#165).
            let _ = self.client.own_write_state(&r);
            if let protocol::Reply::Superseded { session, write_id, keys, .. } = r {
                if self.client.session() == Some(session) {
                    self.state_changed.extend(keys.iter().cloned());
                    self.superseded.push(Superseded { write_id, keys });
                }
            }
        }
        self.client.on_inbound(bytes);
    }

    /// Ask after the oldest `Applying` write of this client's once it has
    /// waited [`ASK_AFTER_MS`] (sdk#174): at most one ask a second.
    pub fn ask_after(&mut self, applying: Option<u64>) -> Option<u64> {
        let now = (self.now_ms)();
        let write_id = applying?;
        if self.asked_at.is_some_and(|at| now.saturating_sub(at) < ASK_AFTER_MS) {
            return None;
        }
        if !self.client.send_ask(now, write_id) {
            return None;
        }
        self.asked_at = Some(now);
        Some(write_id)
    }

    /// GIVE THE ENGINE THE TIME (see [`Client::send_tick`]).
    pub fn send_tick(&mut self, now_ms: u64) -> bool {
        self.client.send_tick(now_ms)
    }

    /// The page is going away: ship what is waiting.
    pub fn send_flush(&mut self) {
        self.client.send(&Request::Flush);
    }

    pub fn take_outbound(&mut self) -> Vec<Vec<u8>> {
        self.client.take_outbound()
    }

    /// Whether a key's last write of this client's ended unpublished.
    pub fn rolled_back(&self, key: &[u8]) -> bool {
        self.rolled_back.contains(key)
    }

    pub fn take_conflicts(&mut self) -> Vec<Conflicted> {
        std::mem::take(&mut self.conflicts)
    }

    pub fn take_unread(&mut self) -> Vec<Unread> {
        std::mem::take(&mut self.unread)
    }

    /// Writes that ended unpublished, each with its named fate. Drains.
    pub fn take_ended(&mut self) -> Vec<(u64, Ended)> {
        std::mem::take(&mut self.ended)
    }

    /// Writes that ended Published since the last call. Drains.
    pub fn take_published(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.published)
    }

    pub fn take_superseded(&mut self) -> Vec<Superseded> {
        std::mem::take(&mut self.superseded)
    }

    /// Keys whose own write changed state since the last call (builder#107).
    pub fn take_state_changed(&mut self) -> Vec<Vec<u8>> {
        let mut keys = std::mem::take(&mut self.state_changed);
        keys.sort();
        keys.dedup();
        keys
    }

    pub fn take_conflict_chains(&mut self) -> Vec<crate::store::ConflictChain> {
        std::mem::take(&mut self.chains)
    }
}
