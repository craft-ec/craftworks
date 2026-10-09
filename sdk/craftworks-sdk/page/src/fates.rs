//! A WRITE'S FATE, kept where it is decided (R-b; READ-STATE § The pull API;
//! COMMIT-LIFE § A write's stage in the page's queue).
//!
//! The engine PUSHES verdicts (`Effect::Notify`); the page's `Server` is the
//! one place every verdict passes, so it keeps them here for the app to PULL.
//! A write still in the queue has no entry: its stage is the engine's
//! (`Engine::queue_stages`), asked when the app asks. A terminal fate is
//! kept until the app reads it. Unread terminals are BOUNDED: past
//! [`MAX_UNREAD`] the oldest is dropped and COUNTED -- never kept for ever,
//! never dropped silently.

use engine::{FailWhy, State, WriteBound};
use std::collections::{BTreeMap, VecDeque};

/// Unread terminal fates kept, across every session of the page.
pub const MAX_UNREAD: usize = 4096;

/// (session, write id): write ids are per session.
pub type FateKey = (u64, u64);

/// What became of a write, as the app reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fate {
    /// In the queue, its path's blocks being fetched (or behind one that is).
    Applying,
    /// In the warm root, not in a commit yet.
    Queued,
    /// The commit in flight carries it.
    Committing,
    /// TERMINAL. In the published tree at `seq`. `backed_up`: its commit's
    /// parity settled (`ParityComplete`) before the app read this.
    Published { seq: u64, backed_up: bool },
    /// TERMINAL, nothing applied: a read no longer held. `after`: the earlier
    /// write (session, write id) whose own end this one cascades from.
    Conflict { key: Vec<u8>, current: Option<[u8; 32]>, after: Option<FateKey> },
    /// TERMINAL, nothing applied: a key written and not read (sdk#235).
    Unread { key: Vec<u8> },
    /// TERMINAL, nothing applied: over one of the engine's bounds.
    TooLarge { bound: WriteBound, limit: usize, got: usize },
    /// TERMINAL, never admitted: the queue held `bytes` of its `limit` and
    /// this session already had a write in it. Backpressure (COMMIT-LIFE K1):
    /// the SDK waits for room and makes it again.
    QueueFull { bytes: usize, limit: usize },
    /// TERMINAL, nothing applied, for the cause named (sdk#500). `signer`:
    /// the signer's own why, beside a `SignerRefused` (`Page`'s refusal).
    Failed { why: FailWhy, signer: Option<String> },
    /// TERMINAL: its commit died and it fell -- forced, or its tries spent.
    Lost,
    /// TERMINAL: whether it landed cannot be known (the head's ledger no
    /// longer lists this page, COMMIT-LIFE ⁵). The app is told to CHECK.
    Unknown,
}

impl Fate {
    pub fn terminal(&self) -> bool {
        !matches!(self, Fate::Applying | Fate::Queued | Fate::Committing)
    }

    /// The stable code that crosses to JavaScript.
    pub fn code(&self) -> &'static str {
        match self {
            Fate::Applying => "APPLYING",
            Fate::Queued => "QUEUED",
            Fate::Committing => "COMMITTING",
            Fate::Published { backed_up: false, .. } => "PUBLISHED",
            Fate::Published { backed_up: true, .. } => "BACKED_UP",
            Fate::Conflict { .. } => "CONFLICT",
            Fate::Unread { .. } => "UNREAD",
            Fate::TooLarge { .. } => "TOO_LARGE",
            Fate::QueueFull { .. } => "QUEUE_FULL",
            Fate::Failed { .. } => "FAILED",
            Fate::Lost => "LOST",
            Fate::Unknown => "UNKNOWN",
        }
    }
}

/// Terminal fates not yet read, in the order they ended.
#[derive(Debug, Default)]
pub struct Fates {
    kept: BTreeMap<FateKey, Fate>,
    order: VecDeque<FateKey>,
    /// Unread terminals dropped past [`MAX_UNREAD`], oldest first. Counted.
    pub dropped: u64,
    /// Conflicts not yet taken by `Db`'s re-run, in order.
    conflicts: Vec<FateKey>,
    /// Tries each conflicted write had spent (ONE budget, sdk#265).
    tries: BTreeMap<FateKey, u32>,
}

impl Fates {
    /// A verdict the engine emitted. Non-terminal ones (`Accepted`,
    /// `Stalled`) are the queue's, read from the engine; `ParityComplete`
    /// only marks an unread `Published`.
    pub fn told(&mut self, key: FateKey, state: &State, seq: u64) {
        let fate = match *state {
            State::Accepted | State::Stalled | State::Busy => return,
            State::ParityComplete => {
                if let Some(Fate::Published { backed_up, .. }) = self.kept.get_mut(&key) {
                    *backed_up = true;
                }
                return;
            }
            State::Published => Fate::Published { seq, backed_up: false },
            // Its key, current value and cause arrive beside it (`Conflicted`).
            State::Conflict => Fate::Conflict { key: Vec::new(), current: None, after: None },
            State::Unread => Fate::Unread { key: Vec::new() },
            State::TooLarge { bound, limit, got } => Fate::TooLarge { bound, limit, got },
            State::QueueFull { bytes, limit } => Fate::QueueFull { bytes, limit },
            State::Failed { why } => Fate::Failed { why, signer: None },
            State::Lost => Fate::Lost,
            State::Unknown => Fate::Unknown,
        };
        if matches!(fate, Fate::Conflict { .. }) {
            self.conflicts.push(key);
        }
        if self.kept.insert(key, fate).is_none() {
            self.order.push_back(key);
        }
        while self.kept.len() > MAX_UNREAD {
            let Some(old) = self.order.pop_front() else { break };
            if self.kept.remove(&old).is_some() {
                self.dropped += 1;
            }
        }
    }

    /// The key, value and cause beside a `Conflict` (`Effect::Conflicted`).
    pub fn conflicted(&mut self, key: FateKey, k: Vec<u8>, current: Option<[u8; 32]>, after: Option<FateKey>, tries: u32) {
        if self.conflicts.contains(&key) {
            self.tries.insert(key, tries);
        }
        if let Some(Fate::Conflict { key: kk, current: c, after: a }) = self.kept.get_mut(&key) {
            *kk = k;
            *c = current;
            *a = after;
        }
    }

    /// The signer's why beside a `Failed { SignerRefused }` (the page's refusal, sdk#500).
    pub fn signer_refused(&mut self, key: FateKey, why: String) {
        if let Some(Fate::Failed { why: FailWhy::SignerRefused, signer }) = self.kept.get_mut(&key) {
            *signer = Some(why);
        }
    }

    /// The key beside an `Unread` (`Effect::Unread`).
    pub fn unread(&mut self, key: FateKey, k: Vec<u8>) {
        if let Some(Fate::Unread { key: kk }) = self.kept.get_mut(&key) {
            *kk = k;
        }
    }

    /// The terminal fate of `key`, READ: gone once read.
    pub fn take(&mut self, key: FateKey) -> Option<Fate> {
        let f = self.kept.remove(&key)?;
        self.order.retain(|k| *k != key);
        Some(f)
    }

    /// Every unread terminal fate of `session`, in the order they ended, READ.
    pub fn take_session(&mut self, session: u64) -> Vec<(u64, Fate)> {
        let mine: Vec<FateKey> = self.order.iter().copied().filter(|k| k.0 == session).collect();
        self.order.retain(|k| k.0 != session);
        mine.into_iter().filter_map(|k| self.kept.remove(&k).map(|f| (k.1, f))).collect()
    }

    /// Peek without reading.
    pub fn get(&self, key: FateKey) -> Option<&Fate> {
        self.kept.get(&key)
    }

    /// Conflicts of `session` not yet taken for a re-run, each with the
    /// writes that cascade from it (`after` naming it, transitively), in
    /// order. Drained; the fates themselves stay until read.
    /// Each chain: its writes, the keys whose reads no longer held, and the
    /// most tries any write of it had spent (the chain's re-run goes on
    /// from there: ONE budget, sdk#265).
    pub fn take_conflicted(&mut self, session: u64) -> Vec<(Vec<u64>, Vec<Vec<u8>>, u32)> {
        let (mine, rest): (Vec<FateKey>, Vec<FateKey>) = self.conflicts.drain(..).partition(|k| k.0 == session);
        self.conflicts = rest;
        let mut chains: Vec<(Vec<u64>, Vec<Vec<u8>>, u32)> = Vec::new();
        let mut chain_of: BTreeMap<FateKey, usize> = BTreeMap::new();
        for k in mine {
            let (key, after) = match self.kept.get(&k) {
                Some(Fate::Conflict { key, after, .. }) => (key.clone(), *after),
                _ => (Vec::new(), None),
            };
            match after.and_then(|a| chain_of.get(&a).copied()) {
                Some(i) => {
                    chains[i].0.push(k.1);
                    chains[i].2 = chains[i].2.max(self.tries.remove(&k).unwrap_or(0));
                    chain_of.insert(k, i);
                }
                None => {
                    chain_of.insert(k, chains.len());
                    chains.push((vec![k.1], if key.is_empty() { Vec::new() } else { vec![key] }, self.tries.remove(&k).unwrap_or(0)));
                }
            }
        }
        chains
    }

    pub fn unread_count(&self) -> usize {
        self.kept.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Past the bound the OLDEST unread terminal is dropped, and each drop is
    /// counted: kept for ever is a leak, dropped silently is a lost verdict.
    #[test]
    fn unread_fates_are_bounded_oldest_first_and_every_drop_counted() {
        let mut f = Fates::default();
        for w in 0..(MAX_UNREAD as u64 + 5) {
            f.told((1, w), &State::Failed { why: FailWhy::TreeDamaged }, 0);
        }
        assert_eq!(f.unread_count(), MAX_UNREAD);
        assert_eq!(f.dropped, 5, "drops were not counted one for one");
        assert_eq!(f.take((1, 0)), None, "the oldest was not the one dropped");
        assert_eq!(f.take((1, 5)), Some(Fate::Failed { why: FailWhy::TreeDamaged, signer: None }), "a fate inside the bound was lost");
        assert_eq!(f.take((1, 5)), None, "a fate outlived being read");
    }

    /// `ParityComplete` marks an unread `Published` and makes nothing else.
    #[test]
    fn parity_complete_marks_published_and_invents_nothing() {
        let mut f = Fates::default();
        f.told((1, 1), &State::Published, 7);
        f.told((1, 1), &State::ParityComplete, 7);
        f.told((1, 2), &State::ParityComplete, 7);
        assert_eq!(f.take((1, 1)), Some(Fate::Published { seq: 7, backed_up: true }));
        assert_eq!(f.take((1, 2)), None, "ParityComplete made a fate for a write never published here");
        f.told((1, 3), &State::Accepted, 7);
        f.told((1, 3), &State::Stalled, 7);
        assert_eq!(f.take((1, 3)), None, "a non-terminal verdict was kept as a fate");
    }

    /// A cascade is ONE chain in app order: a conflict naming an earlier
    /// conflict of the session as `after` joins its chain; a conflict
    /// caused by nothing (or by a write that is not a conflict) starts one.
    #[test]
    fn conflicts_are_chained_by_their_cause() {
        let mut f = Fates::default();
        for (w, after) in [(1u64, None), (2, Some((1u64, 1u64))), (3, None), (4, Some((1, 2))), (5, Some((9, 9)))] {
            f.told((1, w), &State::Conflict, 0);
            f.conflicted((1, w), format!("k{w}").into_bytes(), None, after, w as u32);
        }
        f.told((2, 1), &State::Conflict, 0);
        let chains = f.take_conflicted(1);
        assert_eq!(
            chains,
            vec![(vec![1, 2, 4], vec![b"k1".to_vec()], 4), (vec![3], vec![b"k3".to_vec()], 3), (vec![5], vec![b"k5".to_vec()], 5)],
            "a chain carries the MOST tries any write of it spent (ONE budget)"
        );
        assert!(f.take_conflicted(1).is_empty(), "conflicts were not drained");
        assert_eq!(f.take_conflicted(2).len(), 1, "another session's conflict was taken with this one's");
        assert!(matches!(f.take((1, 1)), Some(Fate::Conflict { .. })), "taking the chain read the fate");
    }
}
