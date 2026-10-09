//! What the engine has ASKED the node for and not been answered, and when
//! (sdk#150).
//!
//! An ask is not a fact. An effect the engine emits can be held by the
//! scheduler and dropped at the end of the call, cut by a per-return limit,
//! or answered on a connection that is gone. So "sent" never means "done",
//! and it never means "in progress" either: the only truth is the node's
//! answer. Recording the emission as the fact -- `Owed::sent = true`, cleared
//! by nothing -- made every lost effect a permanent loss: the first commit's
//! parity, stranded behind the empty tree's root, was never written at all.
//!
//! So what is OWED is derived from fact every call (a group with parity
//! blocks not confirmed), and this table only PACES it: an ask may go out
//! when it has never gone out, or when `reask_after` ticks have passed
//! without an answer. Every ask is idempotent -- a content-addressed put --
//! so the only cost of an ask whose answer was merely slow is one more put.
//!
//! Carried in the context, because an answer arrives in a later call. It is
//! bounded by what it paces: an entry leaves when its answer arrives or when
//! what it asked for stops being owed.

use freenet_prolly::Cid;
use std::collections::BTreeMap;

/// The most one ask costs in the context: an enum tag, an id, `at` and
/// `attempts`, fixed-width. Pinned by `engine/tests/parity_asks.rs`, which
/// measures a table at its cap.
pub const ASK_BYTES: usize = 4 + 32 + 8 + 4;

/// The longest wait between re-asks is `reask_after << MAX_DOUBLINGS`.
pub const MAX_DOUBLINGS: u32 = 6;

/// The share of `max_context_bytes` the table may take at its cap: 1/16.
pub const CONTEXT_SHARE: usize = 16;

/// One kind of thing the engine asks the node for.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Ask {
    /// A parity block put.
    Parity(Cid),
}

/// When an ask last went out, and how many times it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Pace {
    /// The engine's `now` when it last went out. 0 is "before any clock".
    pub at: u64,
    pub attempts: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asks {
    by: BTreeMap<Ask, Pace>,
}

impl Asks {
    /// Whether `ask` may go out at `now`: never asked, or unanswered for
    /// `after` ticks -- doubled for each attempt past the first, up to 64x.
    ///
    /// DECAY, not an end (sdk#150 review C). Redundancy is worth asking for
    /// as long as it is owed, but at a fixed pace a node that never answers
    /// took 798 parity puts in 600 ticks for ONE 64-record write -- about
    /// 0.46 MiB/s upstream for ever. Doubling bounds that to a handful per
    /// context lifetime and still re-asks.
    pub fn due(&self, ask: &Ask, now: u64, after: u64) -> bool {
        self.by.get(ask).is_none_or(|p| {
            let doublings = p.attempts.saturating_sub(1).min(MAX_DOUBLINGS);
            now.saturating_sub(p.at) >= after.saturating_mul(1 << doublings)
        })
    }

    /// `ask` went out at `now`.
    pub fn asked(&mut self, ask: Ask, now: u64) {
        let p = self.by.entry(ask).or_insert(Pace {
            at: now,
            attempts: 0,
        });
        p.at = now;
        p.attempts = p.attempts.saturating_add(1);
    }

    /// The node answered `ask` with a FAILURE: it is re-dated to `now`, as if
    /// just asked, and not counted again -- the failure is the answer to the
    /// attempt already counted, so a failing node is paced exactly like a
    /// silent one.
    ///
    /// Only an ask already MADE can fail, so this re-dates an existing entry
    /// and never inserts one: it is the one writer that does not check
    /// `max_asks`, and the table's bound must not rest on a path being
    /// unreachable.
    pub fn failed(&mut self, ask: Ask, now: u64) {
        if let Some(p) = self.by.get_mut(&ask) {
            p.at = now;
        }
    }

    /// The node answered `ask`, or it stopped being owed.
    pub fn settled(&mut self, ask: &Ask) {
        self.by.remove(ask);
    }

    /// Keep only the asks `owed` still says are wanted.
    pub fn retain(&mut self, mut owed: impl FnMut(&Ask) -> bool) {
        self.by.retain(|a, _| owed(a));
    }

    /// Asks made before there was a clock are dated from the first one, as
    /// the commit timer is: measured from 0, every one would be due at once.
    pub fn anchor(&mut self, now: u64) {
        for p in self.by.values_mut() {
            if p.at == 0 {
                p.at = now;
            }
        }
    }

    /// The clock was reset: every ask is dated from the new one.
    pub fn reanchor(&mut self, now: u64) {
        for p in self.by.values_mut() {
            p.at = now;
        }
    }

    pub fn get(&self, ask: &Ask) -> Option<Pace> {
        self.by.get(ask).copied()
    }

    pub fn len(&self) -> usize {
        self.by.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by.is_empty()
    }

    pub fn to_vec(&self) -> Vec<(Ask, Pace)> {
        self.by.iter().map(|(a, p)| (*a, *p)).collect()
    }

    pub fn from_vec(v: Vec<(Ask, Pace)>) -> Self {
        Asks {
            by: v.into_iter().collect(),
        }
    }
}
