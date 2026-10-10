//! REPUTATION (docs/REPUTATION.md): a pure function of the public VOUCHES and the ROOTS — recomputed by anyone, the same
//! numbers on every machine (integers only).
//!
//! - **Vouches.** "`from` vouches for `to`", written at beacon height `at`. Each account's earliest [`BUDGET`] count.
//! - **The trust TREE.** Trust starts at the roots. An account is ADMITTED when a vouch reaches it from an admitted
//!   account, at `max(the vouch's height, its voucher's admission)`; its PARENT is the voucher that admits it earliest
//!   (ties: the lowest DID). Each root's descendants are its tree. (Not "first voucher regardless of trust": then a
//!   sybil makes every fake vouch before being vouched for — each its own root, all unrelated.)
//! - **Kinship.** Two accounts' CLOSENESS is how far back their lines meet: `(depth of their lowest common ancestor + 1)
//!   / (the shallower one's depth + 1)` — siblings close to 1, distant branches small, different trees 0.
//! - **Diversity.** An account's vouchers, in admission order: each counts by the product of `(1 − closeness)` to every
//!   voucher before it — the first in full, close family next to nothing.
//! - **Trust flow.** Personalised PageRank from the roots: each round an account passes [`DAMP`] of its reputation,
//!   split over its counted vouches, each scaled by that vouch's diversity at the vouchee; roots get `1 − DAMP` each
//!   round. A ring of fakes hung off one fooled account is ONE branch: what reaches it is what that account passed on.
use std::collections::{BTreeMap, BTreeSet};

pub type Did = [u8; 32];

/// One vouch: `from` for `to`, at beacon height `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vouch {
    pub from: Did,
    pub to: Did,
    pub at: u64,
}

/// The most vouches of one account that count (its earliest).
pub const BUDGET: usize = 10;
/// The share an account passes on each round: DAMP_NUM / DAMP_DEN.
pub const DAMP_NUM: u128 = 85;
pub const DAMP_DEN: u128 = 100;
/// Rounds of the flow.
pub const ROUNDS: usize = 40;
/// Fixed point: 1.0.
pub const ONE: u128 = 1_000_000_000_000;

/// The TREE: each admitted account's parent (`None`: a root) and depth, in admission order.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    pub parent: BTreeMap<Did, Option<Did>>,
    pub depth: BTreeMap<Did, u32>,
    pub admitted: BTreeMap<Did, u64>,
}

impl Tree {
    fn line(&self, a: &Did) -> Vec<Did> {
        let mut out = vec![*a];
        let mut x = *a;
        while let Some(Some(p)) = self.parent.get(&x) {
            out.push(*p);
            x = *p;
        }
        out
    }
    /// Closeness of `a` and `b` as (num, den): 0 in different trees.
    pub fn closeness(&self, a: &Did, b: &Did) -> (u128, u128) {
        let (la, lb) = (self.line(a), self.line(b));
        let sb: BTreeSet<&Did> = lb.iter().collect();
        let Some(lca) = la.iter().find(|x| sb.contains(x)) else { return (0, 1) };
        let (da, db, dl) = (self.depth[a], self.depth[b], self.depth[lca]);
        (dl as u128 + 1, da.min(db) as u128 + 1)
    }
}

/// The vouches that count: each account's earliest BUDGET (by height, then target), none to itself, one per pair.
pub fn counted(vouches: &[Vouch]) -> Vec<Vouch> {
    let mut v: Vec<Vouch> = vouches.iter().copied().filter(|x| x.from != x.to).collect();
    v.sort_by_key(|x| (x.from, x.at, x.to));
    let mut out = Vec::new();
    let (mut cur, mut n, mut seen) = (None, 0usize, BTreeSet::new());
    for x in v {
        if cur != Some(x.from) {
            cur = Some(x.from);
            n = 0;
        }
        if n < BUDGET && seen.insert((x.from, x.to)) {
            out.push(x);
            n += 1;
        }
    }
    out
}

/// The trust TREE from `roots` over the counted vouches.
pub fn tree(roots: &[Did], vouches: &[Vouch]) -> Tree {
    let mut t = Tree::default();
    // Earliest admission first (a simple Dijkstra over "admitted at": max(vouch height, voucher's admission)).
    let mut frontier: BTreeSet<(u64, Did, Did)> = BTreeSet::new(); // (time, to, from)
    let mut by_from: BTreeMap<Did, Vec<&Vouch>> = BTreeMap::new();
    for x in vouches {
        by_from.entry(x.from).or_default().push(x);
    }
    for r in roots {
        if t.parent.contains_key(r) {
            continue;
        }
        t.parent.insert(*r, None);
        t.depth.insert(*r, 0);
        t.admitted.insert(*r, 0);
    }
    for r in roots {
        for x in by_from.get(r).into_iter().flatten() {
            frontier.insert((x.at, x.to, *r));
        }
    }
    while let Some((time, to, from)) = frontier.pop_first() {
        if t.parent.contains_key(&to) {
            continue;
        }
        // Earliest time; ties: the lowest voucher DID (the set orders by (time, to, from)).
        t.parent.insert(to, Some(from));
        t.depth.insert(to, t.depth[&from] + 1);
        t.admitted.insert(to, time);
        for x in by_from.get(&to).into_iter().flatten() {
            if !t.parent.contains_key(&x.to) {
                frontier.insert((x.at.max(time), x.to, to));
            }
        }
    }
    t
}

/// REPUTATION of every admitted account (fixed point, [`ONE`] = 1.0), from `roots` and all `vouches`.
pub fn reputation(roots: &[Did], vouches: &[Vouch]) -> BTreeMap<Did, u128> {
    let vs = counted(vouches);
    let t = tree(roots, &vs);
    let vs: Vec<Vouch> = vs.into_iter().filter(|x| t.parent.contains_key(&x.from) && t.parent.contains_key(&x.to)).collect();
    // Each vouch's DIVERSITY at its vouchee: its vouchers in admission order, each by Π(1 − closeness) to those before.
    let mut into: BTreeMap<Did, Vec<Did>> = BTreeMap::new();
    for x in &vs {
        into.entry(x.to).or_default().push(x.from);
    }
    let mut diversity: BTreeMap<(Did, Did), u128> = BTreeMap::new();
    for (to, mut froms) in into {
        froms.sort_by_key(|f| (t.admitted[f], *f));
        for (i, f) in froms.iter().enumerate() {
            let mut d = ONE;
            for g in &froms[..i] {
                let (n, den) = t.closeness(f, g);
                d = d * (den - n) / den;
            }
            diversity.insert((*f, to), d);
        }
    }
    let outdeg: BTreeMap<Did, u128> = vs.iter().fold(BTreeMap::new(), |mut m, x| {
        *m.entry(x.from).or_insert(0) += 1;
        m
    });
    let root_set: BTreeSet<Did> = roots.iter().copied().collect();
    let seed = |d: &Did| if root_set.contains(d) { ONE * (DAMP_DEN - DAMP_NUM) / DAMP_DEN } else { 0 };
    let mut r: BTreeMap<Did, u128> = t.parent.keys().map(|d| (*d, if root_set.contains(d) { ONE } else { 0 })).collect();
    for _ in 0..ROUNDS {
        let mut next: BTreeMap<Did, u128> = t.parent.keys().map(|d| (*d, seed(d))).collect();
        for x in &vs {
            let share = r[&x.from] * DAMP_NUM / DAMP_DEN / outdeg[&x.from];
            *next.get_mut(&x.to).unwrap() += share * diversity[&(x.from, x.to)] / ONE;
        }
        r = next;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(n: u32) -> Did {
        let mut x = [0u8; 32];
        x[..4].copy_from_slice(&n.to_be_bytes());
        x
    }
    fn v(from: u32, to: u32, at: u64) -> Vouch {
        Vouch { from: d(from), to: d(to), at }
    }
    /// An honest community: root 0 vouches 1..=5; each of them vouches five others (6..30).
    fn community() -> Vec<Vouch> {
        let mut out: Vec<Vouch> = (1..=5).map(|i| v(0, i, 1)).collect();
        for i in 1..=5 {
            for j in 0..5 {
                out.push(v(i, 5 + (i - 1) * 5 + j + 1, 2));
            }
        }
        out
    }
    /// A sybil ring of `n` fakes hung off the fooled account `fooled` (ids from 1000), each vouching the next ones.
    fn ring(fooled: u32, n: u32) -> Vec<Vouch> {
        let mut out = vec![v(fooled, 1000, 5)];
        for i in 0..n {
            for k in 1..=9 {
                out.push(v(1000 + i, 1000 + (i + k) % n, 6));
            }
        }
        out
    }
    fn total(r: &BTreeMap<Did, u128>, from: u32, to: u32) -> u128 {
        (from..to).map(|i| r.get(&d(i)).copied().unwrap_or(0)).sum()
    }

    #[test]
    fn the_tree_admits_through_trust_and_names_the_earliest_voucher() {
        let vs = vec![v(0, 1, 5), v(2, 1, 3), v(0, 2, 1), v(3, 4, 0)];
        let t = tree(&[d(0)], &counted(&vs));
        // 1: by 2 at max(3, 2's admission 1) = 3, before 0's vouch at 5 — so its parent is 2.
        assert_eq!(t.parent[&d(1)], Some(d(2)));
        assert_eq!((t.depth[&d(1)], t.admitted[&d(1)]), (2, 3));
        // 3 and 4: no trust reaches them — not in the tree, whatever they vouch.
        assert!(!t.parent.contains_key(&d(3)) && !t.parent.contains_key(&d(4)));
        // A vouch dated before its voucher was admitted waits for that admission.
        let late = tree(&[d(0)], &counted(&[v(0, 1, 9), v(1, 2, 2)]));
        assert_eq!(late.admitted[&d(2)], 9);
    }

    #[test]
    fn closeness_is_how_far_back_two_lines_meet() {
        let t = tree(&[d(0), d(50)], &counted(&community()));
        // Siblings 6 and 7 (both under 1, depth 2): meet at 1 (depth 1) → 2/3.
        assert_eq!(t.closeness(&d(6), &d(7)), (2, 3));
        // Cousins 6 (under 1) and 11 (under 2): meet at the root → 1/3.
        assert_eq!(t.closeness(&d(6), &d(11)), (1, 3));
        // Another tree (root 50, alone): 0.
        assert_eq!(t.closeness(&d(6), &d(50)).0, 0);
    }

    #[test]
    fn a_sybil_ring_gets_one_branchs_worth_however_many_fakes() {
        let base = community();
        let with = |n: u32| {
            let mut vs = base.clone();
            vs.extend(ring(6, n));
            reputation(&[d(0)], &vs)
        };
        let (small, big) = (with(10), with(1000));
        let (s, b) = (total(&small, 1000, 1010), total(&big, 1000, 2000));
        println!("ring of 10: {s}; ring of 1000: {b}; the fooled account: {}", small[&d(6)]);
        // The ring's whole reputation does not grow with its size (within 10%: rounding)…
        assert!(b <= s + s / 10, "a bigger ring took more: {s} → {b}");
        // …and stays within what the fooled account could pass on (DAMP / (1 − DAMP) of it: the flow's own bound).
        assert!(s <= small[&d(6)] * DAMP_NUM / (DAMP_DEN - DAMP_NUM) + ONE / 1000, "the ring got more than its branch");
        // CONTROL: without the diversity discount (every vouch in full), the same ring takes far more.
        let mut vs = base.clone();
        vs.extend(ring(6, 1000));
        let flat = flat_reputation(&[d(0)], &vs);
        let fb = total(&flat, 1000, 2000);
        println!("the same ring of 1000, no discount: {fb}");
        assert!(fb > b * 2, "the discount must matter: {b} vs {fb}");
    }

    #[test]
    fn vouches_from_other_branches_count_more_than_from_close_family() {
        // X is vouched for by two SIBLINGS (6, 7 under 1); Y by two COUSINS (8 under 1, 13 under 2) — same voucher depth.
        let mut vs = community();
        vs.extend([v(6, 500, 3), v(7, 500, 3), v(8, 501, 3), v(13, 501, 3)]);
        let r = reputation(&[d(0)], &vs);
        println!("by siblings {}; by cousins {}", r[&d(500)], r[&d(501)]);
        assert!(r[&d(501)] > r[&d(500)], "cousins' vouches must count more than siblings'");
    }

    #[test]
    fn only_the_earliest_budget_counts_and_no_self_vouch() {
        let vs: Vec<Vouch> = (1..=15).map(|i| v(0, i, i as u64)).chain([v(0, 0, 0)]).collect();
        let c = counted(&vs);
        assert_eq!(c.len(), BUDGET);
        assert!(c.iter().all(|x| x.to != d(0) && x.at <= BUDGET as u64));
    }

    #[test]
    fn the_same_inputs_give_the_same_numbers_in_any_order() {
        let mut vs = community();
        vs.extend(ring(6, 30));
        let a = reputation(&[d(0)], &vs);
        vs.reverse();
        assert_eq!(a, reputation(&[d(0)], &vs));
    }

    /// The control: the same flow with every vouch counted in full (no diversity).
    fn flat_reputation(roots: &[Did], vouches: &[Vouch]) -> BTreeMap<Did, u128> {
        let vs = counted(vouches);
        let t = tree(roots, &vs);
        let vs: Vec<Vouch> = vs.into_iter().filter(|x| t.parent.contains_key(&x.from) && t.parent.contains_key(&x.to)).collect();
        let outdeg: BTreeMap<Did, u128> = vs.iter().fold(BTreeMap::new(), |mut m, x| {
            *m.entry(x.from).or_insert(0) += 1;
            m
        });
        let rs: BTreeSet<Did> = roots.iter().copied().collect();
        let mut r: BTreeMap<Did, u128> = t.parent.keys().map(|d| (*d, if rs.contains(d) { ONE } else { 0 })).collect();
        for _ in 0..ROUNDS {
            let mut next: BTreeMap<Did, u128> = t.parent.keys().map(|d| (*d, if rs.contains(d) { ONE * 15 / 100 } else { 0 })).collect();
            for x in &vs {
                *next.get_mut(&x.to).unwrap() += r[&x.from] * DAMP_NUM / DAMP_DEN / outdeg[&x.from];
            }
            r = next;
        }
        r
    }
}
