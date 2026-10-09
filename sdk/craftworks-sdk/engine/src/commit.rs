//! A COMMIT'S LIFE (COMMIT-LIFE rev 5, sdk#481; the owner's "Structure before code" and "One writer, by type"): the
//! ONE commit in flight as ONE stage -- `Idle`, `Racing` (its PUTs out, the head waiting for race-ready, §P),
//! `Heading` (its `UpdateHead` sent) -- with its data in its variant, so it dies with the stage (C1); and the published
//! commits still putting toward BACKED_UP (`Backing`, with the writes carried onto the next own commit's).
//!
//! The fields are PRIVATE to this module, so the only code that can change a stage or a Backing is the transitions
//! below: `start`, `send_head`, `end` (forward only, C2: nothing here moves Heading back to Racing), and `back`,
//! `acked`, `supersede`, `join_open`, `take_carry`. The commit's CONTENT (its acks, held members, held-back parity) is
//! counted through [`CommitLife::commit_mut`], which cannot change the stage. The engine's side of an end -- the queue,
//! the warm root, the seq -- is ONE function, `Engine::end_commit` (C3).
//!
//! A machine's module: no catch-all over an enum here (CLAUDE.md), so a new stage fails to compile -- under clippy --
//! until every match handles it, a `_` over several variants and over one alike.
#![deny(clippy::wildcard_enum_match_arm, clippy::match_wildcard_for_single_variants)]

use crate::{Backing, ClientId, Commit, CommitStage, Witness, WriteId};
use freenet_prolly::Cid;
use std::collections::BTreeSet;

/// How a commit ends (C3): every end goes through `Engine::end_commit`, with this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum End {
    /// Its head showed (E6).
    Published,
    /// A foreign head (E8), adopted; the WITNESS decides a Heading commit's group (⁵). A Racing commit's head never
    /// left: the witness is not read and no try is spent (A8).
    Dead { witness: Option<Witness>, head: (u64, Cid) },
    /// A final refusal: the head refused by the signer (E9, `SignerRefused`), or a block the commit needs rejected
    /// (E4, `BlockRejected`). Its writes are told `Failed` with this cause (sdk#500).
    Failed(core_types::fail::FailWhy),
}

#[derive(Debug, Default)]
enum Stage {
    #[default]
    Idle,
    Racing {
        commit: Commit,
        /// The engine time it started: the stall clock (E11).
        started: u64,
    },
    Heading {
        commit: Commit,
        started: u64,
        /// When the head was last asked about (E12), and how many times: its pace doubles.
        asked_at: u64,
        rounds: u32,
    },
}

#[derive(Debug, Default)]
pub(crate) struct CommitLife {
    stage: Stage,
    /// Published commits still putting toward BACKED_UP (§P), in publish order.
    backing: Vec<Backing>,
    /// Writes whose Backing a later commit superseded: they join the next own commit's Backing.
    carry: BTreeSet<(ClientId, WriteId)>,
    /// Supersedes made with a commit IN FLIGHT: an impossible cell today (every caller runs after `end`), counted into
    /// `Engine::impossible_transitions` -- which the COMMIT-LIFE model asserts is 0 at every step -- rather than a
    /// debug_assert, so a test can drive the real in-flight case and see the exclusion hold (the second reviewer).
    in_flight_supersedes: u64,
}

impl CommitLife {
    // ---- the stage ----

    pub(crate) fn stage(&self) -> CommitStage {
        match &self.stage {
            Stage::Idle => CommitStage::Idle,
            Stage::Racing { .. } => CommitStage::Racing,
            Stage::Heading { .. } => CommitStage::Heading,
        }
    }

    /// The commit in flight.
    pub(crate) fn commit(&self) -> Option<&Commit> {
        match &self.stage {
            Stage::Idle => None,
            Stage::Racing { commit, .. } | Stage::Heading { commit, .. } => Some(commit),
        }
    }

    /// The commit in flight, to COUNT into (acks, held members, the held-back parity). Its stage is not reachable
    /// from here.
    pub(crate) fn commit_mut(&mut self) -> Option<&mut Commit> {
        match &mut self.stage {
            Stage::Idle => None,
            Stage::Racing { commit, .. } | Stage::Heading { commit, .. } => Some(commit),
        }
    }

    /// When the commit in flight started (the stall clock), if one is.
    pub(crate) fn started(&self) -> Option<u64> {
        match &self.stage {
            Stage::Idle => None,
            Stage::Racing { started, .. } | Stage::Heading { started, .. } => Some(*started),
        }
    }

    /// Re-anchor the stall clock: to `now` when the clock was reset, or when it started before any clock (`0`).
    pub(crate) fn anchor(&mut self, now: u64, reset: bool) {
        match &mut self.stage {
            Stage::Idle => {}
            Stage::Racing { started, .. } => {
                if reset || *started == 0 {
                    *started = now;
                }
            }
            Stage::Heading { started, asked_at, .. } => {
                if reset || *started == 0 {
                    *started = now;
                }
                if reset || *asked_at == 0 {
                    *asked_at = now;
                }
            }
        }
    }

    /// Idle → Racing (E1). `false` if a commit is already in flight: an impossible cell, counted by the caller.
    pub(crate) fn start(&mut self, commit: Commit, now: u64) -> bool {
        match self.stage {
            Stage::Idle => {
                self.stage = Stage::Racing { commit, started: now };
                true
            }
            Stage::Racing { .. } | Stage::Heading { .. } => false,
        }
    }

    /// Racing → Heading: the head is sent at `now`. `false` in any other stage (a Heading commit re-issues without
    /// moving).
    pub(crate) fn send_head(&mut self, now: u64) -> bool {
        match std::mem::take(&mut self.stage) {
            Stage::Racing { commit, started } => {
                self.stage = Stage::Heading { commit, started, asked_at: now, rounds: 0 };
                true
            }
            other @ (Stage::Idle | Stage::Heading { .. }) => {
                self.stage = other;
                false
            }
        }
    }

    /// Heading x E12: is the head question due at `now` (the pace `reask`, doubling per round)? When it is, the round
    /// is counted. Never an end (C5); in any other stage, never due.
    pub(crate) fn head_question_due(&mut self, now: u64, reask: u64) -> bool {
        match &mut self.stage {
            Stage::Heading { asked_at, rounds, .. } => {
                let wait = reask.saturating_mul(1 << (*rounds).min(crate::asks::MAX_DOUBLINGS));
                if now.saturating_sub(*asked_at) < wait {
                    return false;
                }
                *asked_at = now;
                *rounds += 1;
                true
            }
            Stage::Idle | Stage::Racing { .. } => false,
        }
    }

    /// → Idle: the commit in flight, taken for `Engine::end_commit`, and whether its head had been sent.
    pub(crate) fn end(&mut self) -> Option<(Commit, bool)> {
        match std::mem::take(&mut self.stage) {
            Stage::Idle => None,
            Stage::Racing { commit, .. } => Some((commit, false)),
            Stage::Heading { commit, .. } => Some((commit, true)),
        }
    }

    // ---- Backing (§P) ----

    /// A published commit's blocks still out: its Backing.
    pub(crate) fn back(&mut self, writes: Vec<(ClientId, WriteId)>, remaining: BTreeSet<Cid>) {
        self.backing.push(Backing { writes, remaining });
    }

    /// `id` acked: the writes of every Backing it drained.
    pub(crate) fn acked(&mut self, id: &Cid) -> Vec<(ClientId, WriteId)> {
        let mut drained = Vec::new();
        self.backing.retain_mut(|b| {
            if b.remaining.remove(id) && b.remaining.is_empty() {
                drained.extend(b.writes.iter().copied());
                return false;
            }
            true
        });
        drained
    }

    /// Stragglers a root move made garbage (`gone`): withdrawn from every Backing, whose writes are carried onto the
    /// next own commit's. The ids withdrawn, and the writes of every Backing that drained.
    ///
    /// Correct whatever the caller does (the architect on C1): a block the commit IN FLIGHT still owes is never
    /// withdrawn, even when a root move made it garbage for a Backing -- its PUT is that commit's too. Every call site
    /// runs with no commit in flight today (publish and adopt, after `end`); the assert makes a future one loud.
    pub(crate) fn supersede(&mut self, gone: &BTreeSet<Cid>) -> (BTreeSet<Cid>, Vec<(ClientId, WriteId)>) {
        if self.commit().is_some() {
            self.in_flight_supersedes += 1;
        }
        let owed: BTreeSet<Cid> = self.commit().map(|c| c.owed().copied().collect()).unwrap_or_default();
        self.supersede_excluding(gone, &owed)
    }

    /// [`CommitLife::supersede`]'s body: every Backing loses the ids of `gone` -- except those `owed` by a commit in
    /// flight.
    fn supersede_excluding(&mut self, gone: &BTreeSet<Cid>, owed: &BTreeSet<Cid>) -> (BTreeSet<Cid>, Vec<(ClientId, WriteId)>) {
        let mut withdrawn = BTreeSet::new();
        for b in self.backing.iter_mut() {
            let hit: Vec<Cid> = b.remaining.intersection(gone).filter(|id| !owed.contains(*id)).copied().collect();
            if hit.is_empty() {
                continue;
            }
            for id in hit {
                b.remaining.remove(&id);
                withdrawn.insert(id);
            }
            self.carry.extend(b.writes.iter().copied());
        }
        let mut drained = Vec::new();
        self.backing.retain(|b| {
            if b.remaining.is_empty() {
                drained.extend(b.writes.iter().copied());
                false
            } else {
                true
            }
        });
        (withdrawn, drained)
    }

    /// A no-op write (#164) joins every open Backing: BACKED_UP when they drain. `false` when none is open (BACKED_UP
    /// now).
    pub(crate) fn join_open(&mut self, w: (ClientId, WriteId)) -> bool {
        if self.backing.is_empty() {
            return false;
        }
        for b in self.backing.iter_mut() {
            if !b.writes.contains(&w) {
                b.writes.push(w);
            }
        }
        true
    }

    /// The carried writes, for the commit publishing now.
    pub(crate) fn take_carry(&mut self) -> BTreeSet<(ClientId, WriteId)> {
        std::mem::take(&mut self.carry)
    }

    // ---- read accessors: nothing below changes a stage or a Backing ----

    /// Is `w` still waiting on a Backing, or carried to the next one?
    pub(crate) fn waits_for(&self, w: &(ClientId, WriteId)) -> bool {
        self.carry.contains(w) || self.backing.iter().any(|b| b.writes.contains(w))
    }

    /// Does a published commit's Backing still owe `id`?
    pub(crate) fn backing_owes(&self, id: &Cid) -> bool {
        self.backing.iter().any(|b| b.remaining.contains(id))
    }

    /// Every block the Backings still owe.
    pub(crate) fn unacked(&self) -> BTreeSet<Cid> {
        self.backing.iter().flat_map(|b| b.remaining.iter().copied()).collect()
    }

    /// Supersedes made with a commit in flight (an impossible cell, counted).
    pub(crate) fn in_flight_supersedes(&self) -> u64 {
        self.in_flight_supersedes
    }

    pub(crate) fn backing_len(&self) -> usize {
        self.backing.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: &[u8]) -> BTreeSet<Cid> {
        n.iter().map(|b| [*b; 32]).collect()
    }

    /// A block the commit IN FLIGHT owes is never withdrawn by a supersede, even when the root move made it garbage for
    /// a Backing (the architect's C1 residual). Mutant: drop the `owed` exclusion -> red.
    #[test]
    fn a_block_the_commit_in_flight_owes_is_never_withdrawn() {
        let mut life = CommitLife::default();
        life.back(vec![(ClientId(1), WriteId(1))], ids(&[1, 2, 3]));
        // 2 and 3 are garbage for the Backing; 2 is also owed by the commit in flight.
        let (withdrawn, _) = life.supersede_excluding(&ids(&[2, 3]), &ids(&[2]));
        assert_eq!(withdrawn, ids(&[3]), "a block the commit in flight owes was withdrawn");
        assert!(life.backing_owes(&[2; 32]), "the in-flight commit's block left the Backing");
        assert!(!life.backing_owes(&[3; 32]));
    }

    /// THE PRODUCTION PATH, with a REAL commit in flight (the second reviewer on #538: a hand-made set on an idle life
    /// left the owed-set derivation untested): the in-flight commit's owed block is not withdrawn from a Backing that a
    /// root move made it garbage for, and the impossible cell is counted. Mutant: an empty owed set in `supersede` ->
    /// red.
    #[test]
    fn a_real_in_flight_commits_owed_block_survives_a_supersede() {
        let mut life = CommitLife::default();
        life.back(vec![(ClientId(1), WriteId(1))], ids(&[1, 2, 3]));
        assert!(life.start(Commit::for_test(ids(&[2, 9])), 0), "THE SETUP: the commit did not start");
        let (withdrawn, _) = life.supersede(&ids(&[2, 3]));
        assert_eq!(withdrawn, ids(&[3]), "the in-flight commit's owed block was withdrawn");
        assert!(life.backing_owes(&[2; 32]));
        assert_eq!(life.in_flight_supersedes(), 1, "the impossible cell was not counted");
    }

    /// With nothing in flight, every garbage block is withdrawn from every Backing (a block two published commits share
    /// leaves both).
    #[test]
    fn with_nothing_in_flight_every_backing_loses_the_garbage() {
        let mut life = CommitLife::default();
        life.back(vec![(ClientId(1), WriteId(1))], ids(&[1, 2]));
        life.back(vec![(ClientId(1), WriteId(2))], ids(&[2, 4]));
        let (withdrawn, _) = life.supersede(&ids(&[2]));
        assert_eq!(withdrawn, ids(&[2]));
        assert!(!life.backing_owes(&[2; 32]), "a Backing still owes a garbage block");
    }
}
