//! Who is watching which range, and what a root move means for them.
//!
//! A subscription is a standing question — "has anything between these two
//! keys changed?" — and the whole difficulty is answering it without reading
//! the tree. The prolly tree makes that cheap in exactly the way this needs:
//! a node's id is the hash of its bytes, so **equal id ⇒ equal subtree**, and
//! `diff` skips whole subtrees on that alone. Two roots that are equal cost
//! ZERO reads, which is the common case on every call where nothing
//! published.
//!
//! So the answer to "did this range change" is a diff restricted to the range,
//! asked for ONE change. Not the changes — whether there are any. A commit
//! that rewrites ten thousand keys outside a subscriber's range costs that
//! subscriber the descent and nothing else.
//!
//! # What a subscription is FOR, and what it is not for
//!
//! **Only data that is actually live.** A subscription is a standing cost —
//! context bytes, a diff on every root move, a push the client must handle —
//! paid continuously, in exchange for hearing about a change without asking.
//! That trade is only worth making where the data CHANGES while someone is
//! looking at it and where the change is worth acting on immediately: a feed,
//! a presence list, a document two people have open, a counter that ticks.
//!
//! For ordinary data it is a loss. A record read once and shown, a schema, a
//! settings blob, a list a person opens and closes — these are read when
//! wanted and are correct at the moment they are read. Subscribing to them
//! buys a notification nobody is waiting for, and it spends a slot that the
//! live data needed.
//!
//! The second half of the test is the DELTA. The reason to be told "something
//! in this range changed" rather than to re-read is that the change is small
//! against the thing it changed — one new row in a long list, one field in a
//! document. Where hearing about a change means re-reading the whole range
//! anyway, a subscription has bought a push instead of a poll and little
//! else. That is why `Changed` carries the root it moved FROM as well as the
//! one it moved to: the pair is what a delta is computed between, and an
//! engine that told a client only where the tree landed would have left it
//! with no way to ask what moved.
//!
//! **Nothing subscribes on a client's behalf.** There is no implicit
//! subscription anywhere above this: no read takes one out, no collection
//! opens one because it was listed. An app names the range and says so,
//! because only the app knows which of its data is live. `max_subscriptions`
//! is deliberately small for the same reason — a cap that comfortably fits
//! "everything this app touches" is a cap that invites it.
//!
//! # What is bounded, and why each bound exists
//!
//! Everything here is held in a delegate's context, which is 400 KiB and is
//! the budget every other thing also comes out of. Two separate bounds:
//!
//! - **How many.** `max_subscriptions`. Over it, the subscribe is REFUSED and
//!   the client is told — never silently dropped, because a client that
//!   believes it is subscribed and is not will wait for ever for a
//!   notification that is not coming.
//! - **How big each one is.** `max_sub_key`. The bounds are client-chosen
//!   byte strings, so without this a single subscription is an allocation the
//!   sender picks the size of — the thing `MAX_MESSAGE` exists to prevent, one
//!   layer in.

use crate::ClientId;
use freenet_prolly::range::Range;
use freenet_prolly::store::Blocks;
use freenet_prolly::Cid;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::Bound;

/// A range someone is watching.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubRange {
    pub lo: Bound<Vec<u8>>,
    pub hi: Bound<Vec<u8>>,
}

impl SubRange {
    /// The longest key in either bound, which is what a size cap is about.
    fn widest(&self) -> usize {
        let one = |b: &Bound<Vec<u8>>| match b {
            Bound::Unbounded => 0,
            Bound::Included(k) | Bound::Excluded(k) => k.len(),
        };
        one(&self.lo).max(one(&self.hi))
    }
}

/// Why a subscriber is being told.
///
/// Three different facts, never merged into one. A client that re-reads on any
/// of them is correct, but one deciding whether to back off, or counting
/// commits, has to know which it got — and an engine that reported a guess as
/// a finding would make that count silently wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The trees were compared over this range and they differ. A FINDING.
    Diffed,
    /// The comparison needed a block this engine does not hold.
    ///
    /// DURABLE, and that is what separates it from `Budgeted`: the old root's
    /// blocks may be superseded, unpinned and evicted, in which case no amount
    /// of waiting brings them back. Repeated, it is what makes a range go
    /// `Stale` rather than notify for ever.
    BlockMissing,
    /// The comparison could not be attempted within this call's budget.
    ///
    /// TRANSIENT. Nothing is wrong; the engine ran out of room. Retrying
    /// promptly is right, and backing off would be wrong.
    Budgeted,
    /// This engine has STOPPED comparing this range, and says so once.
    ///
    /// After `stale_after` consecutive `BlockMissing` the range is not worth
    /// asking about again on its own: the client is told, exactly once, and
    /// the range stays quiet until the client reloads it. A declared
    /// degradation beats a silent one — and beats a notification every commit
    /// that the client can do nothing with.
    Stale,
}

/// One subscription, and what the engine has learnt about it.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Watch {
    range: SubRange,
    /// The root this subscription was last told about.
    ///
    /// The whole of the "at most one notification per (subscription, new
    /// root)" invariant. Two paths can observe one root move in a single call
    /// — a commit landing and a head read — and without this the client gets
    /// the same fact twice and has no way to tell it from two commits.
    last_told: Option<Cid>,
    /// Consecutive `BlockMissing` answers. Reset by any answer that is not
    /// one, so a range that recovers is not held against its own history.
    missing_streak: u32,
    /// Stopped, and the client has been told so.
    stale: bool,
    /// When this subscription was taken, in subscribe order: what makes one
    /// SESSION older than another (sdk#146). A client id is a random
    /// session, so the id itself says nothing about age.
    born: u64,
}

/// Every standing subscription, by client and client-chosen id.
#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Subs {
    ranges: BTreeMap<(u64, u64), Watch>,
    /// The next `born`.
    next: u64,
}

/// The bounds a subscribe is checked against.
#[derive(Clone, Copy, Debug)]
pub struct SubLimits {
    pub max_subs: usize,
    pub max_per_client: usize,
    pub max_key: usize,
}

/// What came of a subscribe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accepted {
    Yes,
    /// At the cap. The client still holds the intention and may try again
    /// once something is dropped.
    ///
    /// The engine REFUSES here where the node EVICTS, and the difference is
    /// deliberate: the node cannot tell a delegate it evicted something, so it
    /// must evict rather than starve. This engine is answering a client that
    /// is listening, so a refusal it can see beats a silent drop it cannot.
    Full,
    /// A bound longer than this engine will hold.
    TooWide,
}

impl Subs {
    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Take a subscription, or say why not.
    ///
    /// Re-subscribing an id RESETS it — range, last-told root and any `Stale`
    /// state — and does not count against the cap again. Two reasons, and the
    /// second is the load-bearing one. A client narrowing a range it already
    /// holds is not asking for more. And the SDK re-asserts its subscriptions
    /// on every reconnect, so this is the ordinary path, not the exceptional
    /// one: it must be idempotent, and it must be how a `Stale` range is
    /// brought back to life, because there is nothing else a client can do
    /// about one.
    ///
    /// # Whose room it is (sdk#146)
    ///
    /// Every page load is its own client now, and nothing tells the engine a
    /// page went away — so a reload's old subscriptions stayed, under one
    /// engine-wide cap, and a few reloads refused every new page's bindings.
    /// Two bounds instead:
    ///
    /// - **Per client**, `max_per_client`: one page cannot take the room of
    ///   every other. Over it, `Full`, and the page is told.
    /// - **Engine-wide**, `max_subs`: at the cap, the OLDEST OTHER client —
    ///   the one whose first standing subscription is oldest — loses all of
    ///   its subscriptions, which are RETURNED (the caller decides whether it
    ///   can tell anyone; today it cannot address them, see the engine). A
    ///   page that reloaded away is exactly the room this frees. Only when
    ///   the asking client is the only one holding anything is the subscribe
    ///   refused.
    pub fn add(
        &mut self,
        client: ClientId,
        sub_id: u64,
        range: SubRange,
        limits: SubLimits,
    ) -> (Accepted, Vec<(ClientId, u64)>) {
        if range.widest() > limits.max_key {
            return (Accepted::TooWide, Vec::new());
        }
        let key = (client.0, sub_id);
        let mut evicted = Vec::new();
        if let Some(w) = self.ranges.get_mut(&key) {
            // A re-subscribe resets it and keeps its place.
            *w = Watch { range, last_told: None, missing_streak: 0, stale: false, born: w.born };
            return (Accepted::Yes, evicted);
        }
        if self.ranges.keys().filter(|(c, _)| *c == client.0).count() >= limits.max_per_client {
            return (Accepted::Full, evicted);
        }
        if self.ranges.len() >= limits.max_subs {
            // The oldest OTHER client: the smallest `born` among its entries.
            let mut oldest: Option<(u64, u64)> = None; // (born, client)
            for ((c, _), w) in &self.ranges {
                if *c != client.0 && oldest.is_none_or(|(b, _)| w.born < b) {
                    oldest = Some((w.born, *c));
                }
            }
            let Some((_, victim)) = oldest else {
                return (Accepted::Full, evicted);
            };
            evicted = self
                .ranges
                .keys()
                .filter(|(c, _)| *c == victim)
                .map(|(c, s)| (ClientId(*c), *s))
                .collect();
            self.ranges.retain(|(c, _), _| *c != victim);
        }
        let born = self.next;
        self.next += 1;
        self.ranges.insert(key, Watch { range, last_told: None, missing_streak: 0, stale: false, born });
        (Accepted::Yes, evicted)
    }

    /// Drop one. An id that is not there is not an error — a client dropping
    /// a subscription it has already lost should not have to find out first.
    ///
    /// This drops the ENGINE's copy. It does not and cannot release the
    /// node's: the platform has no unsubscribe, so a delegate goes on being
    /// woken for contracts its engine has forgotten. That is ordinary, and it
    /// is why being woken for something unknown is not an error anywhere here.
    pub fn remove(&mut self, client: ClientId, sub_id: u64) {
        self.ranges.remove(&(client.0, sub_id));
    }

    /// Drop everything a client holds. Called when it disconnects: a
    /// subscription whose client is gone is a cost with nobody to pay it.
    pub fn drop_client(&mut self, client: ClientId) {
        self.ranges.retain(|(c, _), _| *c != client.0);
    }

    pub fn iter(&self) -> impl Iterator<Item = (ClientId, u64, &SubRange)> {
        self.ranges
            .iter()
            .map(|((c, s), w)| (ClientId(*c), *s, &w.range))
    }

    /// Every subscription that a move from `a` to `b` touched.
    ///
    /// Takes `&mut self` because deciding is also REMEMBERING: the invariant
    /// is at most one notification per (subscription, new root), and a
    /// subscription that has already been told about `b` must not be told
    /// again when a second path observes the same move.
    ///
    /// `a == b` is answered without reading anything, which is what makes this
    /// safe to call on every root move including the ones that changed
    /// nothing.
    pub fn touched<B: Blocks>(
        &mut self,
        blocks: &B,
        a: &Cid,
        b: &Cid,
        stale_after: u32,
    ) -> Vec<(ClientId, u64, Why)> {
        let mut out = Vec::new();
        if a == b {
            return out;
        }
        for ((c, s), watch) in self.ranges.iter_mut() {
            // The invariant, enforced where it is decided rather than asserted
            // in a test: one notification per (subscription, new root).
            if watch.last_told == Some(*b) {
                continue;
            }
            // A range that has gone stale stays quiet. It was told once; the
            // way back is a reload, which re-subscribes and resets it.
            if watch.stale {
                watch.last_told = Some(*b);
                continue;
            }
            let Some(why) = changed_in(blocks, a, b, &watch.range) else {
                watch.missing_streak = 0;
                watch.last_told = Some(*b);
                continue;
            };
            let why = match why {
                Why::BlockMissing => {
                    watch.missing_streak += 1;
                    if watch.missing_streak >= stale_after {
                        watch.stale = true;
                        Why::Stale
                    } else {
                        Why::BlockMissing
                    }
                }
                other => {
                    watch.missing_streak = 0;
                    other
                }
            };
            watch.last_told = Some(*b);
            out.push((ClientId(*c), *s, why));
        }
        out
    }

    /// Notify everyone without comparing anything. The CONTROL for the range
    /// filter, and the shape a downgrade-to-polling engine would take.
    pub fn touched_without_diff(&mut self, a: &Cid, b: &Cid) -> Vec<(ClientId, u64, Why)> {
        let mut out = Vec::new();
        if a == b {
            return out;
        }
        for ((c, s), watch) in self.ranges.iter_mut() {
            if watch.last_told == Some(*b) {
                continue;
            }
            watch.last_told = Some(*b);
            // Nothing was compared, so nothing may be claimed. `Budgeted` is
            // the honest word: the engine did not look.
            out.push((ClientId(*c), *s, Why::Budgeted));
        }
        out
    }
}

/// Did anything in `r` change between the two roots?
///
/// `None` = no. `Some(Diffed)` = yes, demonstrably. `Some(Unknown)` = the
/// comparison could not be completed.
///
/// **One change is the whole question.** `max_entries: 1` stops the diff at
/// the first difference inside the range, so a commit that rewrote the rest
/// of the tree costs a subscriber the descent to its own range and no more.
/// Asking for the changes and then testing whether the list is empty would be
/// the same answer at the price of the commit's size.
fn changed_in<B: Blocks>(blocks: &B, a: &Cid, b: &Cid, r: &SubRange) -> Option<Why> {
    let range = Range {
        lo: r.lo.clone(),
        hi: r.hi.clone(),
        reverse: false,
        after: None,
        max_entries: 1,
        max_bytes: usize::MAX,
    };
    match freenet_prolly::diff::diff(blocks, a, b, &range, None) {
        Ok(page) => {
            if !page.changes.is_empty() {
                Some(Why::Diffed)
            } else if !page.need.is_empty() {
                // The diff stopped on a block this engine does not hold, so
                // "no changes found" is "none found SO FAR" — which is not
                // the same sentence and must not be reported as it.
                //
                // DURABLE, not transient: the block it wants is usually on the
                // OLD root, which no longer has a writer keeping it and may
                // have been evicted. Waiting does not fix that, which is why
                // repeating this is what makes a range `Stale`.
                Some(Why::BlockMissing)
            } else {
                None
            }
        }
        // A diff that could not RUN — a resume token from another pair of
        // roots, a range instruction a diff has no answer for. Not evidence of
        // no change, and not a missing block either: nothing was attempted, so
        // the transient word is the honest one and a prompt retry is right.
        Err(_) => Some(Why::Budgeted),
    }
}
