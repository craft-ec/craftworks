//! A READ REPAIRS THROUGH PARITY (Phase 4, DURABILITY-STATE gap 1): a block
//! that no attempt could fetch is rebuilt from its sibling group -- any `k`
//! of the group's `k + PARITY` blocks -- and kept only if it hashes to the id that
//! was asked for. Nothing here reaches the network: the engine asks for the
//! group's blocks with the same `FetchBlock` every read uses (the page's one
//! sender), and hands what it rebuilt to the page as a block to keep.
//!
//! The group is found in a node the engine already HOLDS: the read that
//! missed a block descended through its parent, and the parent lists both the
//! group's members and its `PARITY` parity ids (freenet-prolly `parity.rs`, the
//! rule; a leaf's groups are over its referenced values, a branch's over its
//! children). The ROOT is a group of one (sdk#335): its parity ids ride in its
//! head, so [`root_group`] builds its group from those, not from a node.

use freenet_prolly::node::Node;
use freenet_prolly::parity;
use freenet_prolly::store::Blocks;
use freenet_prolly::{block_id, kind, Cid};
use crate::PARITY;

/// How many held nodes the group search may visit. It walks only what the
/// engine already holds, from the root the read stands on.
pub const MAX_SEARCH_NODES: usize = 4096;

/// EVERY slot position holding `id` (the architect on sdk#542: THE one lookup of an id among a group's slots). Identical
/// values are one block, so one id can fill several slots, and an answer for it -- bytes or NotFound -- answers EVERY
/// one. A first-match lookup (`position`) marked one and was the defect twice (sdk#527's mark, sdk#542's repair); the
/// `slots_of` control (engine/tests/slots_of.rs) fails the build on one anywhere else.
/// Generic over what a slot holds (a block's `Cid`, a piece's node key): page-io's publish marks its piece sets through
/// it too.
pub fn slots_of<'a, T: PartialEq<Q>, Q: ?Sized>(slots: &'a [T], id: &'a Q) -> impl Iterator<Item = usize> + 'a {
    slots.iter().enumerate().filter(move |(_, s)| **s == *id).map(|(i, _)| i)
}

/// One sibling group, as a repair needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// The block being rebuilt, and its place among the members.
    pub missing: Cid,
    pub missing_ix: usize,
    /// The members' kind (`RAW` for a leaf's values, `TREE_NODE` for a
    /// branch's children): a member's SYMBOL is `kind ‖ body`.
    pub kind: u8,
    /// `k` data members, then the group's [`PARITY`] parity blocks, in the code's column order.
    pub slots: Vec<Cid>,
    pub k: usize,
    /// The longest a rebuilt member of this group may be (a stranger's
    /// length is bounded before it costs anything).
    pub max_len: usize,
}

impl Group {
    /// Every slot holding `id` ([`slots_of`]).
    pub fn slots_of<'a>(&'a self, id: &'a Cid) -> impl Iterator<Item = usize> + 'a {
        slots_of(&self.slots, id)
    }

    /// Is slot `i` a parity block?
    pub fn is_parity(&self, i: usize) -> bool {
        i >= self.k
    }

    /// Does `bytes` belong in slot `i`? Checked by hash like every arrival: [`crate::read::matches_id`], the one check
    /// (the architect: not a second hash check of its own; engineer2's sweep).
    pub fn fits(&self, i: usize, bytes: &[u8]) -> bool {
        crate::read::matches_id(&self.slots[i], bytes)
    }

    /// Slot `i`'s block as the code takes it: a member as its length-prefixed
    /// symbol (`kind ‖ body`), parity as stored.
    pub fn stored(&self, i: usize, bytes: &[u8]) -> Vec<u8> {
        if self.is_parity(i) {
            bytes.to_vec()
        } else {
            let mut st = Vec::with_capacity(1 + bytes.len());
            st.push(self.kind);
            st.extend_from_slice(bytes);
            parity::symbol(&st)
        }
    }
}

/// A GROUP'S MARGIN (sdk#524; one owner with the assets audit, engineer2's #478 port): of its `k` members and its
/// parity, `present` are there -- margin = present - k. The CALLER says what "present" means, in its own doc: the audit
/// counts what it VERIFIED (Held, Fetched); a reader counts what is NOT answered NotFound (a silent block may be there).
pub fn margin(present: usize, k: usize) -> i64 {
    present as i64 - k as i64
}

core_types::vocabulary! {
    /// A group's health from its margin (KEEPER §4): WHOLE (all `k + m` there), DEGRADED (at least `k`: recoverable),
    /// DAMAGED (below `k`: no decode can rebuild a member). THE one derivation and its words: the audit's DAMAGED and a
    /// reader's damaged group are this (sdk#524).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum GroupHealth {
        Whole => "WHOLE",
        Degraded => "DEGRADED",
        Damaged => "DAMAGED",
    }
}

impl GroupHealth {
    /// `present` of a group of `k` members + `m` parity.
    pub fn of(present: usize, k: usize, m: usize) -> GroupHealth {
        let margin = margin(present, k);
        if margin >= m as i64 {
            GroupHealth::Whole
        } else if margin >= 0 {
            GroupHealth::Degraded
        } else {
            GroupHealth::Damaged
        }
    }
}

/// The root's group of ONE (sdk#335): `k = 1`, the root then its parity, coded
/// like a branch's child (a node, `TREE_NODE`). [`root_parity`] makes the
/// blocks; this is what a read rebuilds the root from.
pub fn root_group(root: Cid, parity_ids: &[Cid]) -> Group {
    let mut slots = vec![root];
    slots.extend_from_slice(parity_ids);
    Group { missing: root, missing_ix: 0, kind: kind::TREE_NODE, slots, k: 1, max_len: parity::MAX_MEMBER_NODE }
}

/// The root's parity blocks (sdk#335): its group of one, coded by the tree's
/// own rule for a group of nodes (`kind ‖ bytes`, then `encode_group`), so a
/// root is repairable by anyone the way every other node is. `None` only if
/// the code refuses (it does not for k = 1).
pub fn root_parity(root_bytes: &[u8]) -> Option<Vec<(Cid, Vec<u8>)>> {
    let mut st = Vec::with_capacity(1 + root_bytes.len());
    st.push(kind::TREE_NODE);
    st.extend_from_slice(root_bytes);
    let blocks = parity::encode_group(&[st]).ok()?;
    Some(blocks.into_iter().map(|p| (block_id(kind::PARITY, &p), p)).collect())
}

/// The group `missing` belongs to, found in a node held under `root`.
/// `None`: no held node lists it in a group (the root, a tree written without
/// parity, or a parent not held).
pub fn find_group(blocks: &dyn Blocks, root: Cid, missing: Cid) -> Option<Group> {
    let mut stack = vec![root];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(cid) = stack.pop() {
        if !seen.insert(cid) || seen.len() > MAX_SEARCH_NODES {
            continue;
        }
        let Some(bytes) = blocks.get(&cid) else { continue };
        let Ok(node) = Node::parse(bytes) else { continue };
        let ids: Vec<Cid> = node.parity().collect();
        for (g, (_, members)) in parity::group_members(&node).into_iter().enumerate() {
            let Some(missing_ix) = slots_of(&members, &missing).next() else { continue };
            let par = ids.get(PARITY * g..PARITY * (g + 1))?;
            let leaf = node.is_leaf();
            let k = members.len();
            let mut slots = members;
            slots.extend_from_slice(par);
            return Some(Group {
                missing,
                missing_ix,
                kind: if leaf { kind::RAW } else { kind::TREE_NODE },
                slots,
                k,
                max_len: if leaf { parity::MAX_MEMBER_VALUE } else { parity::MAX_MEMBER_NODE },
            });
        }
        if !node.is_leaf() {
            for i in 0..node.len() {
                stack.push(node.child(i).0);
            }
        }
    }
    None
}

/// Rebuild the missing member from what is held (`have[i]`: slot `i` as
/// [`Group::stored`] gives it), and VERIFY it: the body is returned only if
/// it hashes to the id that was asked for.
pub fn rebuild(group: &Group, have: &[Option<Vec<u8>>]) -> Result<Vec<u8>, String> {
    let states = parity::repair_group(group.k, have, group.max_len).map_err(|e| format!("the group could not be solved: {e:?}"))?;
    let state = states.get(group.missing_ix).ok_or("the solve returned too few members")?;
    let (&k, body) = state.split_first().ok_or("a rebuilt member is empty")?;
    if k != group.kind || block_id(group.kind, body) != group.missing {
        return Err("the rebuilt block does not hash to the id that was asked for".into());
    }
    Ok(body.to_vec())
}
