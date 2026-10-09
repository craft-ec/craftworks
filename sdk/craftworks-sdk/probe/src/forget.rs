//! A NODE THAT LOST BLOCKS (the Assets demo, main 2026-09-27: publish, THEN the node loses data): which blocks of a
//! tree a STOPPED private node forgets, decided from what the node itself stores. `node-forget` removes their states;
//! this module only chooses, so it is tested without a node.
//!
//! The groups are the tree's own: every stored Block-contract state that is a TREE NODE lists its groups
//! (`freenet_prolly::parity::group_members`, members then the node's `PARITY` parity ids per group) -- the same reading
//! `probe::lose` makes of a GET answer. Chosen: the first `groups` groups with `k >= 2` (by `--domain`: only a leaf's
//! group of that domain's records, by the SDK's ONE record-key encoder), in the order the node's states are read.
//! Each loses `n` of its blocks, DATA members first: `n = m` is the margin (repairable), `n = m + 1` the control.
use crate::lose::{parse_lose, Target};
use freenet_prolly::node::{Node, Value};
use freenet_prolly::parity::{group_members, PARITY};
use freenet_prolly::{kind, Cid};
use std::collections::BTreeSet;

/// One group of the tree, and the slots chosen to be forgotten.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chosen {
    /// `k` members, then the group's parity.
    pub slots: Vec<Cid>,
    pub k: usize,
    pub lost: BTreeSet<Cid>,
}

/// `--lose m|m+1` -> the number each chosen group loses.
pub fn lose_count(s: &str) -> anyhow::Result<usize> {
    let _ = Target::Data;
    parse_lose(s)
}

/// Every group (members, parity) of a TREE-NODE state's node, `None` for any other block. `domain`: only a leaf's
/// groups whose members are ALL values under that record prefix.
pub fn groups_of(state: &[u8], domain: Option<&[u8]>) -> Vec<(Vec<Cid>, Vec<Cid>)> {
    let Some((&kind::TREE_NODE, body)) = state.split_first() else { return Vec::new() };
    let Ok(node) = Node::parse(body) else { return Vec::new() };
    let parity: Vec<Cid> = node.parity().collect();
    let groups = group_members(&node);
    let mut out = Vec::new();
    for (g, (_, members)) in groups.iter().enumerate() {
        if members.len() < 2 {
            continue;
        }
        if let Some(prefix) = domain {
            if !node.is_leaf() {
                continue;
            }
            // Every member must be a value referenced by an entry whose key has the prefix.
            let under = members.iter().all(|m| {
                (0..node.len()).any(|i| matches!(node.value(i), Value::Ref { cid, .. } if cid == *m) && node.key(i).starts_with(prefix))
            });
            if !under {
                continue;
            }
        }
        if let Some(par) = parity.get(PARITY * g..PARITY * (g + 1)) {
            out.push((members.clone(), par.to_vec()));
        }
    }
    out
}

/// The TREE-NODE states reachable from `root` (the head's current tree), root first: a store also holds every older
/// version's nodes, and a group of an old node is not the tree's -- forgetting it loses nothing the head reads.
/// `state_of` answers a stored block's state; a child the store does not hold is skipped.
pub fn reachable<'s>(root: Cid, state_of: impl Fn(&Cid) -> Option<&'s [u8]>) -> Vec<&'s [u8]> {
    let (mut out, mut seen, mut todo) = (Vec::new(), BTreeSet::new(), vec![root]);
    while let Some(cid) = todo.pop() {
        if !seen.insert(cid) {
            continue;
        }
        let Some(state) = state_of(&cid) else { continue };
        let Some((&kind::TREE_NODE, body)) = state.split_first() else { continue };
        let Ok(node) = Node::parse(body) else { continue };
        if !node.is_leaf() {
            todo.extend((0..node.len()).map(|i| node.child(i).0));
        }
        out.push(state);
    }
    out
}

/// From `groups` in order, the first `take` distinct ones, each losing `n` slots, data members first. A slot shared by
/// two chosen groups is counted once per group it stands in (so no group loses more than `n` of its own).
pub fn choose(groups: &[(Vec<Cid>, Vec<Cid>)], take: usize, n: usize) -> Vec<Chosen> {
    let mut out: Vec<Chosen> = Vec::new();
    let mut seen: BTreeSet<Vec<Cid>> = BTreeSet::new();
    let mut lost_anywhere: BTreeSet<Cid> = BTreeSet::new();
    for (members, parity) in groups {
        if out.len() >= take {
            break;
        }
        let slots: Vec<Cid> = members.iter().chain(parity.iter()).copied().collect();
        if !seen.insert(slots.clone()) {
            continue;
        }
        // A group that shares a slot with one already chosen would lose more than `n`: skip it.
        if slots.iter().any(|s| lost_anywhere.contains(s)) || out.iter().any(|c| c.slots.iter().any(|s| slots.contains(s))) {
            continue;
        }
        let lost: BTreeSet<Cid> = slots.iter().copied().take(n).collect();
        lost_anywhere.extend(lost.iter().copied());
        out.push(Chosen { k: members.len(), slots, lost });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(b: u8) -> Cid {
        Cid::from([b; 32])
    }

    #[test]
    fn each_chosen_group_loses_exactly_n_data_members_first_and_no_group_is_hit_twice() {
        let g = |base: u8| ((0..4).map(|i| cid(base + i)).collect::<Vec<_>>(), (0..PARITY as u8).map(|i| cid(base + 100 + i)).collect::<Vec<_>>());
        let groups = vec![g(10), g(10), g(20), g(30)];
        let c = choose(&groups, 2, PARITY);
        assert_eq!(c.len(), 2, "a repeated group was chosen twice, or too few were chosen");
        for ch in &c {
            assert_eq!(ch.lost.len(), PARITY);
            // Data members first: the lost are the first PARITY slots, members before parity.
            assert!(ch.lost.iter().all(|l| ch.slots[..PARITY].contains(l)));
            assert_eq!(ch.k, 4);
        }
        // The control: m + 1 in one group.
        let ctl = choose(&groups, 1, PARITY + 1);
        assert_eq!(ctl[0].lost.len(), PARITY + 1);
    }

    #[test]
    fn reachable_walks_only_the_roots_tree() {
        use freenet_prolly::node::NodeBuilder as Builder;
        let leaf = |v: u8| {
            let mut b = Builder::leaf();
            b.push(&[v], Value::Inline(&[v])).unwrap();
            let mut s = vec![kind::TREE_NODE];
            s.extend(b.finish().unwrap());
            s
        };
        let (a, old) = (leaf(1), leaf(2));
        let mut br = Builder::branch(1);
        br.push_child(&[1], cid(10), Default::default()).unwrap();
        let mut root = vec![kind::TREE_NODE];
        root.extend(br.finish().unwrap());
        let store = [(cid(9), root.clone()), (cid(10), a.clone()), (cid(11), old)];
        let got = reachable(cid(9), |c| store.iter().find(|(k, _)| k == c).map(|(_, v)| v.as_slice()));
        assert_eq!(got, vec![root.as_slice(), a.as_slice()], "the old version's leaf (11) is not the root's tree");
    }

    #[test]
    fn a_group_that_shares_a_slot_with_a_chosen_one_is_skipped() {
        let a = (vec![cid(1), cid(2)], vec![cid(50)]);
        let b = (vec![cid(2), cid(3)], vec![cid(51)]);
        let c = (vec![cid(4), cid(5)], vec![cid(52)]);
        let chosen = choose(&[a, b, c], 2, 1);
        assert_eq!(chosen.iter().map(|c| c.slots[0]).collect::<Vec<_>>(), vec![cid(1), cid(4)]);
    }
}
