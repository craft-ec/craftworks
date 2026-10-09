//! WHO WANTS A BLOCK (WANTED-LIFE, sdk#480 part 3; the owner's "Structure before code"): the reader indexes -- the
//! parked reads waiting on a block, the repairs that need it as a slot, the parked write's needs, and the assets
//! audit's passes (sdk#530: W1, EVERY holder of the (block, GET) key) -- held in ONE type whose fields are PRIVATE to
//! this module. The only code that can change them is the transitions below (`want`, `drop_reader`, `served`,
//! `missed_audits`, `release_write`); everything else reads them through [`Wanted::readers_of`] and the read
//! accessors. The compiler is the one-writer gate: a source scan of spellings could be walked around (the
//! architect on #504).
//!
//! A machine's module: no catch-all over an enum here (CLAUDE.md), so a new `Reader` fails to compile -- under clippy
//! -- until every match handles it, a `_` over several variants and over one alike.
#![deny(clippy::wildcard_enum_match_arm, clippy::match_wildcard_for_single_variants)]

use crate::read::ReqId;
use crate::{PassId, Readers};
use freenet_prolly::Cid;
use std::collections::{BTreeMap, BTreeSet};

/// ONE reader of a block, as [`Wanted::want`] and [`Wanted::drop_reader`] take it: a parked read, a repair (by the block
/// it rebuilds), the one parked write, or an assets-audit pass (sdk#530).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reader {
    Read(ReqId),
    Repair(Cid),
    Write,
    Audit(PassId),
}

/// What an arrival took ([`Wanted::served`]): the reads it wakes, and the audit passes it answers.
#[derive(Debug, Default)]
pub(crate) struct Served {
    pub(crate) reads: Option<BTreeSet<ReqId>>,
    pub(crate) audits: BTreeSet<PassId>,
}

#[derive(Debug, Default)]
pub(crate) struct Wanted {
    /// Who is waiting on each outstanding block. Two requests needing one block share its fetch: the second does not
    /// pay for the first's round trip, and a popular branch is fetched once however many readers want it.
    waiting: BTreeMap<Cid, BTreeSet<ReqId>>,
    /// Which repairs a group block is being fetched for.
    slots: BTreeMap<Cid, BTreeSet<Cid>>,
    /// The blocks the parked write is waiting on, so an arrival for something else does not re-run its apply.
    needs: BTreeSet<Cid>,
    /// The assets audit's passes waiting on each block's GET (WANTED-LIFE W1, W7): answered ONLY by the node.
    audits: BTreeMap<Cid, BTreeSet<PassId>>,
}

impl Wanted {
    /// WHO WANTS `id`, derived, never stored.
    pub(crate) fn readers_of(&self, id: &Cid) -> Readers {
        Readers {
            reads: self.waiting.contains_key(id),
            repairs: self.slots.contains_key(id),
            parked_write: self.needs.contains(id),
            audits: self.audits.contains_key(id),
        }
    }

    /// THE WANT: `reader` wants `id`. Who wanted it BEFORE -- each caller's own ask rule reads it (one ask per block).
    pub(crate) fn want(&mut self, id: Cid, reader: Reader) -> Readers {
        let before = self.readers_of(&id);
        match reader {
            Reader::Read(req) => {
                self.waiting.entry(id).or_default().insert(req);
            }
            Reader::Repair(missing) => {
                self.slots.entry(id).or_default().insert(missing);
            }
            Reader::Write => {
                self.needs.insert(id);
            }
            Reader::Audit(pass) => {
                self.audits.entry(id).or_default().insert(pass);
            }
        }
        before
    }

    /// THE DROP: `reader` no longer wants `id`. `true` when nobody does now.
    pub(crate) fn drop_reader(&mut self, id: Cid, reader: Reader) -> bool {
        match reader {
            Reader::Read(req) => {
                if let Some(reqs) = self.waiting.get_mut(&id) {
                    reqs.remove(&req);
                    if reqs.is_empty() {
                        self.waiting.remove(&id);
                    }
                }
            }
            Reader::Repair(missing) => {
                if let Some(set) = self.slots.get_mut(&id) {
                    set.remove(&missing);
                    if set.is_empty() {
                        self.slots.remove(&id);
                    }
                }
            }
            Reader::Write => {
                self.needs.remove(&id);
            }
            Reader::Audit(pass) => {
                if let Some(set) = self.audits.get_mut(&id) {
                    set.remove(&pass);
                    if set.is_empty() {
                        self.audits.remove(&id);
                    }
                }
            }
        }
        !self.readers_of(&id).any()
    }

    /// `id` ARRIVED: the readers it serves are TAKEN, both sets at once (WANTED-LIFE A2), so neither can be answered
    /// twice or forgotten. `by_node`: the NODE's answer to the GET (E2n) -- it serves the reads AND the audits; a LOCAL
    /// landing (E2r: a rebuild from parity) serves the reads only, and the audits keep waiting for the node (W7). Not a
    /// drop: the block is here.
    pub(crate) fn served(&mut self, id: Cid, by_node: bool) -> Served {
        Served { reads: self.waiting.remove(&id), audits: if by_node { self.audits.remove(&id).unwrap_or_default() } else { BTreeSet::new() } }
    }

    /// `id` was answered NotFound (E3): the AUDITS waiting on it are answered (absent, their verdict) and TAKEN; every
    /// other reader keeps waiting (rule 8).
    pub(crate) fn missed_audits(&mut self, id: Cid) -> BTreeSet<PassId> {
        self.audits.remove(&id).unwrap_or_default()
    }

    /// The parked write is released: every need loses it. The needs nobody else wants now.
    pub(crate) fn release_write(&mut self) -> Vec<Cid> {
        let needs = std::mem::take(&mut self.needs);
        needs.into_iter().filter(|id| !self.readers_of(id).any()).collect()
    }

    // ---- read accessors: nothing below changes a reader ----

    /// Is an audit pass waiting on `id`?
    pub(crate) fn audit_waits_on(&self, id: &Cid) -> bool {
        self.audits.contains_key(id)
    }

    /// Is a parked read waiting on `id`?
    pub(crate) fn read_waits_on(&self, id: &Cid) -> bool {
        self.waiting.contains_key(id)
    }

    /// The parked reads waiting on `id`.
    pub(crate) fn reads_on(&self, id: &Cid) -> Option<&BTreeSet<ReqId>> {
        self.waiting.get(id)
    }

    /// The blocks read `req` waits on.
    pub(crate) fn waited_by(&self, req: ReqId) -> Vec<Cid> {
        self.waiting.iter().filter(|(_, reqs)| reqs.contains(&req)).map(|(id, _)| *id).collect()
    }

    /// Every block some parked read waits on.
    pub(crate) fn read_blocks(&self) -> impl Iterator<Item = &Cid> {
        self.waiting.keys()
    }

    /// The whole read-wait index, for the state walk only (its size, never its meaning).
    pub(crate) fn waiting(&self) -> &BTreeMap<Cid, BTreeSet<ReqId>> {
        &self.waiting
    }

    /// Is `id` a slot some repair is fetching?
    pub(crate) fn is_slot(&self, id: &Cid) -> bool {
        self.slots.contains_key(id)
    }

    /// The repairs (by the block they rebuild) that need slot `id`.
    pub(crate) fn repairs_on(&self, id: &Cid) -> Vec<Cid> {
        self.slots.get(id).map_or(Vec::new(), |s| s.iter().copied().collect())
    }

    /// The slots repair `missing` is fetching.
    pub(crate) fn slots_of(&self, missing: &Cid) -> Vec<Cid> {
        self.slots.iter().filter(|(_, set)| set.contains(missing)).map(|(slot, _)| *slot).collect()
    }

    /// The parked write's needs.
    pub(crate) fn write_needs(&self) -> &BTreeSet<Cid> {
        &self.needs
    }
}
