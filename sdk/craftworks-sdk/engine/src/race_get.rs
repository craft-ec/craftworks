//! RACE GET (the owner's rule 11, sdk#303): a read asks ALL `k + m` blocks of a sibling group and finishes on the
//! first `k`, repairing from parity.
//!
//! #300's repair already does the second half: given a group, it asks every other block of it at once, counts
//! what arrives (each checked by hash against its slot, parity included), rebuilds the missing member at `k`,
//! verifies it, and hands it to the page as a block to keep, where it lands like any arrival. What it did not do
//! was START until the member itself had been answered NotFound -- and a member that is SILENT is never answered,
//! so its read waited on the slowest block. Racing starts that gather the FIRST time a read wants the member:
//!
//! - the member is asked, as before, and so is the rest of its group, at once (point reads too: rule 11 as
//!   written; a "race only after an RTO" hedge would amend it, and is not built);
//! - whichever comes first ends it: the member itself (`on_arrived` ends its repair), or any `k` of the group
//!   (rebuilt and verified);
//! - each block is asked ONCE however many members race it (`start_repair` does not re-ask a slot in flight);
//! - what a finished race no longer wants is WITHDRAWN (`Effect::Unwanted`, net at the call's end): the page drops its GET instead
//!   of re-asking it for ever, and does not keep it if it arrives late;
//! - the group is found in the parent held under the read's OWN pinned root (another root's parity would not
//!   match); the root is a group of ONE (sdk#335) when its head listed its parity -- the first of its `1 + m` to
//!   arrive answers -- and is fetched singly when it did not.
//!
//! Stragglers need nothing here: the page re-sends a GET on its RTO with no count (rule 7), and a block that
//! arrives after its group resolved is either used (someone still waits) or dropped (withdrawn).

use crate::{Effect, Engine};
use freenet_prolly::store::Blocks;
use freenet_prolly::{block_id, kind, Cid};
use std::collections::BTreeSet;

impl<B: Blocks> Engine<B> {
    /// A read has just asked for `id` under `root`: race the rest of its group. Nothing when racing is off, when
    /// `id` is already being raced or repaired, or when it is in no group (a root whose head listed no parity).
    pub(crate) fn race(&mut self, id: Cid, root: Cid) -> Vec<Effect> {
        if !self.params.race_get || !self.params.repair_reads || self.repairs.contains_key(&id) {
            return Vec::new();
        }
        match self.group_for(root, id) {
            Some(group) => {
                let mut out = self.start_repair(group.clone(), false);
                // REPAIR mode (sdk#479): each parity slot's GET stays out until the NODE answers it (an audit reader,
                // served only by the node), so a parity block it lacks is learned even when it answers with silence.
                if self.repair_pass {
                    for i in group.k..group.slots.len() {
                        let slot = group.slots[i];
                        if self.blocks.get(&slot).is_some() || self.parity_watch.contains_key(&slot) || self.parity_owed.contains_key(&slot) {
                            continue;
                        }
                        self.parity_watch.insert(slot, group.clone());
                        out.extend(self.on_audit_want(slot, crate::REPAIR_PASS));
                    }
                }
                out
            }
            None => Vec::new(),
        }
    }

    /// A block rebuilt from its group. Its bytes must BE its id, checked here and not only inside the rebuild
    /// (the architect's (a)): otherwise nothing -- the page never holds bytes under a wrong id. Verified, the
    /// page KEEPS it, and PUTs it back (sdk#303) when a reader WAITED on it (a rebuild nobody asked for is not
    /// written: the architect's (c)).
    pub(crate) fn rebuilt(&self, id: Cid, body: &[u8]) -> Vec<Effect> {
        if !crate::read::matches_id(&id, body) {
            return Vec::new();
        }
        let mut out = vec![Effect::Keep { id, bytes: body.to_vec() }];
        // PUT back for whoever needed it: a read, or a parked write (sdk#405) -- the network is whole again.
        let wants = self.readers_of(&id);
        // A CHECK pass (sdk#479) puts nothing: it only counts.
        if self.put_back && (wants.reads || wants.parked_write) {
            out.push(Effect::PutRepaired { id, bytes: body.to_vec() });
        }
        out
    }

    /// The block a race was racing is answered NotFound: the race becomes a REPAIR, and the slots it dropped
    /// (NotFound or wrong beside a healthy read) are asked again, now until answered.
    pub(crate) fn race_missed(&mut self, id: Cid) -> Vec<Effect> {
        let Some(r) = self.repairs.get_mut(&id) else { return Vec::new() };
        if r.missed {
            return Vec::new();
        }
        r.missed = true;
        let own: Vec<usize> = r.group.slots_of(&id).collect();
        r.absent.extend(own);
        let slots: Vec<(usize, Cid)> = std::mem::take(&mut r.dropped).into_iter().map(|i| (i, r.group.slots[i])).collect();
        for (i, _) in &slots {
            r.asked.insert(*i, 0);
        }
        let mut out = Vec::new();
        for (_, slot) in slots {
            let in_flight = self.want(slot, crate::Reader::Repair(id)).any();
            if !in_flight {
                self.reads.fetches += 1;
                self.step_asks.entry(slot).or_insert(false);
                out.push(Effect::FetchBlock { id: slot, via: crate::read::Via::Direct, attempt: 0 });
            }
        }
        out
    }

    /// PARITY PUT BACK (sdk#479, the owner via core dev): a parity block the node answered NotFound is re-encoded from
    /// its group's `k` members, with the code the save uses (`parity::encode_group` over each member's `kind ‖ body`),
    /// once the page holds them all -- and PUT only if it hashes to the id its parent lists (never a different block
    /// under a listed id). A member not held yet leaves it owed: the scan that reads the group brings it.
    pub(crate) fn put_owed_parity(&mut self) -> Vec<Effect> {
        let mut out = Vec::new();
        // A CHECK pass (sdk#479) puts nothing: an owed parity stays owed, counted lost.
        if !self.put_back {
            return out;
        }
        let owed: Vec<(Cid, crate::repair::Group)> = self.parity_owed.iter().map(|(c, g)| (*c, g.clone())).collect();
        for (id, g) in owed {
            let states: Option<Vec<Vec<u8>>> = g.slots[..g.k]
                .iter()
                .map(|m| {
                    self.blocks.get(m).map(|b| {
                        let mut st = Vec::with_capacity(1 + b.len());
                        st.push(g.kind);
                        st.extend_from_slice(b);
                        st
                    })
                })
                .collect();
            let Some(states) = states else { continue };
            self.parity_owed.remove(&id);
            // Its PARITY slot, through the one lookup (`repair::slots_of`: the build fails on a first match).
            let Some(ix) = g.slots_of(&id).find(|&i| g.is_parity(i)) else { continue };
            let encoded = freenet_prolly::parity::encode_group(&states).ok().and_then(|p| p.into_iter().nth(ix - g.k));
            match encoded {
                Some(bytes) if block_id(kind::PARITY, &bytes) == id => {
                    self.parity_counts.1 += 1;
                    out.push(Effect::PutRepaired { id, bytes });
                }
                _ => self.parity_counts.2 += 1,
            }
        }
        out
    }

    /// Land every block rebuilt this step, like an arrival, in a LOOP: a landing that finishes more races queues
    /// their rebuilds behind it rather than recursing into them ([`Engine`]'s `landing`).
    ///
    /// A block lands ONCE per step. `on_arrived` clears `arrived` when it ends, and the page keeps a rebuilt block
    /// only when it applies `Keep`, after the step -- so a read re-descending through it later in the same step
    /// wants it again, races, and rebuilds it again; landed again, that loop never ends. Once is enough: the read
    /// asks the block, and the next step reads it from the page.
    pub(crate) fn land_rebuilt(&mut self) -> Vec<Effect> {
        let mut out = Vec::new();
        let mut landed: BTreeSet<Cid> = BTreeSet::new();
        while !self.landing.is_empty() {
            let (id, body) = self.landing.remove(0);
            if landed.insert(id) {
                out.extend(self.on_arrived(id, body, false));
            }
        }
        out
    }
}

#[cfg(test)]
mod put_back {
    use crate::read::ReqId;
    use crate::{Effect, Engine, Params};
    use freenet_prolly::store::MemBlocks;
    use freenet_prolly::{block_id, kind};

    fn put(fx: &[Effect]) -> usize {
        fx.iter().filter(|f| matches!(f, Effect::PutRepaired { .. })).count()
    }

    /// The put-back decision, each condition on its own: wanted AND verified is put; verified but unwanted is
    /// only kept; unverified is neither. (Through the engine, a rebuild never yields bytes that are not its id -- `repair::rebuild`
    /// refuses them first -- so (a) is reachable only here.)
    #[test]
    fn a_rebuilt_block_is_put_back_only_when_wanted_and_verified() {
        let mut e = Engine::new(Params::default(), MemBlocks::default());
        let body = b"a rebuilt block".to_vec();
        let id = block_id(kind::RAW, &body);
        assert_eq!(put(&e.rebuilt(id, &body)), 0, "(c) a rebuild nobody waited on was put");
        e.want(id, crate::Reader::Read(ReqId(1)));
        assert_eq!(put(&e.rebuilt(id, &body)), 1, "a wanted, verified rebuild was not put");
        assert_eq!(put(&e.rebuilt(id, b"not its bytes")), 0, "(a) an unverified rebuild was put");
        assert!(e.rebuilt(id, b"not its bytes").is_empty(), "(a) an unverified rebuild was KEPT: the page would hold bytes under a wrong id");
    }

    /// OWED PARITY IS PUT ONLY UNDER ITS OWN ID (engineer3 for sdk#479): a group whose parent lists one TRUE parity id
    /// and one that is not what its members encode to (a wrong or forged listing). With every member held, the true
    /// one is re-encoded and PUT, byte for byte; the other is never PUT under that id, and is counted mismatched.
    /// Mutant "skip the id check" -> red.
    #[test]
    fn owed_parity_is_put_only_under_the_id_its_bytes_hash_to() {
        let mut e = Engine::new(Params::default(), MemBlocks::default());
        let members: Vec<Vec<u8>> = (0..3u8).map(|i| vec![b'm' + i; 700]).collect();
        let ids: Vec<freenet_prolly::Cid> = members.iter().map(|m| block_id(kind::RAW, m)).collect();
        for (id, m) in ids.iter().zip(&members) {
            e.blocks.insert(*id, m);
        }
        let states: Vec<Vec<u8>> = members.iter().map(|m| [vec![kind::RAW], m.clone()].concat()).collect();
        let parity = freenet_prolly::parity::encode_group(&states).expect("the group encodes");
        let truth = block_id(kind::PARITY, &parity[0]);
        let forged: freenet_prolly::Cid = [0xee; 32];
        let mut slots = ids.clone();
        slots.extend([truth, forged]);
        let g = crate::repair::Group { missing: ids[0], missing_ix: 0, kind: kind::RAW, slots, k: 3, max_len: freenet_prolly::parity::MAX_MEMBER_VALUE };
        e.parity_owed.insert(truth, g.clone());
        e.parity_owed.insert(forged, g);
        let out = e.put_owed_parity();
        let put: Vec<(freenet_prolly::Cid, Vec<u8>)> = out.iter().filter_map(|f| if let Effect::PutRepaired { id, bytes } = f { Some((*id, bytes.clone())) } else { None }).collect();
        assert_eq!(put, vec![(truth, parity[0].clone())], "owed parity PUT under an id its bytes do not hash to, or not PUT byte for byte");
        assert_eq!((e.parity_counts.1, e.parity_counts.2), (1, 1), "the true parity not counted put, or the forged one not counted mismatched");
        assert!(e.parity_owed.is_empty(), "owed parity left owed with every member held");
    }
}
