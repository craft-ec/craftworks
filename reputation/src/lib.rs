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

/// The STARTING ROOT (owner 10-10: "wren hold the key and root reputation. i'll be the first layer member as anyone
/// else"): WREN — the account Claude keeps, its recovery words also held offline by the owner (to rotate it back). The
/// root vouches by its published policy (docs/REPUTATION.md §6), never earns or witnesses; every other account starts
/// the same, relationships build the rest.
pub const ROOTS: &[&str] = &["did:craftec:BZ6QnLTT3pnaWtUvfWr9bjtc8nCeRnC2Q7q44XMNUt5h"];

/// One vouch: `from` for `to`, at beacon height `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vouch {
    pub from: Did,
    pub to: Did,
    pub at: u64,
}

pub mod item;
pub mod statement;

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
    /// `a`'s line up to its root — stopping at a PROMOTED root (a family's head: kinship ends there).
    fn line(&self, a: &Did, promoted: &BTreeSet<Did>) -> Vec<Did> {
        let mut out = vec![*a];
        let mut x = *a;
        while !promoted.contains(&x) {
            let Some(Some(p)) = self.parent.get(&x) else { break };
            out.push(*p);
            x = *p;
        }
        out
    }
    /// `a`'s LINEAGE: the starting root's direct vouchee on its line (itself for one; `None` for a root).
    pub fn lineage(&self, a: &Did) -> Option<Did> {
        let mut x = *a;
        loop {
            match self.parent.get(&x) {
                Some(Some(p)) if self.parent.get(p) == Some(&None) => return Some(x),
                Some(Some(p)) => x = *p,
                _ => return None,
            }
        }
    }
    /// Closeness of `a` and `b` as (num, den): 0 in different trees or families.
    pub fn closeness(&self, a: &Did, b: &Did) -> (u128, u128) {
        self.closeness_in(a, b, &BTreeSet::new())
    }
    fn closeness_in(&self, a: &Did, b: &Did, promoted: &BTreeSet<Did>) -> (u128, u128) {
        let (la, lb) = (self.line(a, promoted), self.line(b, promoted));
        let sb: BTreeSet<&Did> = lb.iter().collect();
        let Some(lca) = la.iter().find(|x| sb.contains(x)) else { return (0, 1) };
        // Depths within the family: from its head.
        let head = |l: &Vec<Did>| self.depth[l.last().unwrap()];
        let h = head(&la);
        let (da, db, dl) = (self.depth[a] - h, self.depth[b] - h, self.depth[lca] - h);
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

/// Beacon heights per epoch (the standing epoch): what the ramp counts in.
pub const EPOCH: u64 = 1024;
/// Epochs over which a vouch grows into full weight: a quarter in the epoch it is made, full after [`RAMP`].
pub const RAMP: u64 = 4;

/// REPUTATION AS OF beacon height `now` (TOKEN §7d: the epoch-start SNAPSHOT every witness computes alike): only vouches
/// made by then, each RAMPED in — `(epochs it has stood + 1) / RAMP` of its weight, full after RAMP epochs — so nobody
/// is vouched in, paid an epoch and dropped, and a voucher has time to notice before a vouch carries all of its weight.
pub fn reputation_at(roots: &[Did], vouches: &[Vouch], now: u64) -> BTreeMap<Did, u128> {
    let then: Vec<Vouch> = vouches.iter().copied().filter(|v| v.at <= now).collect();
    reputation_ramped(roots, &then, &|v: &Vouch| {
        let stood = (now / EPOCH).saturating_sub(v.at / EPOCH) + 1;
        ONE * stood.min(RAMP) as u128 / RAMP as u128
    })
}

/// Vouchers' lineages an account needs to be PROMOTED — besides holding at least the MEDIAN reputation of the admitted
/// accounts (relative: reputation thins out as the network grows, so a fixed bar would not scale).
pub const PROMOTE_LINEAGES: usize = 3;

/// REPUTATION of every admitted account (fixed point, [`ONE`] = 1.0), from `roots` and all `vouches` — with
/// PROMOTION: an account holding at least the median reputation and vouched for from [`PROMOTE_LINEAGES`] different lineages heads a
/// FAMILY of its own (kinship stops at it), never a source of trust. Computed once without families, the promoted
/// found from that, then again with them.
pub fn reputation(roots: &[Did], vouches: &[Vouch]) -> BTreeMap<Did, u128> {
    reputation_ramped(roots, vouches, &|_| ONE)
}

/// [`reputation`] with each vouch's weight scaled by `ramp` (fixed point).
fn reputation_ramped(roots: &[Did], vouches: &[Vouch], ramp: &dyn Fn(&Vouch) -> u128) -> BTreeMap<Did, u128> {
    let vs = counted(vouches);
    let t = tree(roots, &vs);
    let first = flow(roots, &vs, &t, &BTreeSet::new(), ramp);
    let promoted = promoted(roots, &vs, &t, &first);
    if promoted.is_empty() {
        return first;
    }
    flow(roots, &vs, &t, &promoted, ramp)
}

/// The PROMOTED: not a starting root, holding at least the median reputation of the admitted non-root accounts,
/// vouched for from [`PROMOTE_LINEAGES`] lineages.
pub fn promoted(roots: &[Did], vouches: &[Vouch], t: &Tree, rep: &BTreeMap<Did, u128>) -> BTreeSet<Did> {
    let mut lineages: BTreeMap<Did, BTreeSet<Did>> = BTreeMap::new();
    for x in vouches {
        if let (true, Some(l)) = (t.parent.contains_key(&x.to), t.lineage(&x.from)) {
            lineages.entry(x.to).or_default().insert(l);
        }
    }
    let mut all: Vec<u128> = rep.iter().filter(|(a, _)| !roots.contains(a)).map(|(_, r)| *r).collect();
    all.sort();
    let median = all.get(all.len() / 2).copied().unwrap_or(u128::MAX);
    lineages
        .into_iter()
        .filter(|(a, ls)| !roots.contains(a) && ls.len() >= PROMOTE_LINEAGES && rep.get(a).copied().unwrap_or(0) >= median)
        .map(|(a, _)| a)
        .collect()
}

fn flow(roots: &[Did], vs: &[Vouch], t: &Tree, families: &BTreeSet<Did>, ramp: &dyn Fn(&Vouch) -> u128) -> BTreeMap<Did, u128> {
    let vs = vs.to_vec();
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
                let (n, den) = t.closeness_in(f, g, families);
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
            *next.get_mut(&x.to).unwrap() += share * diversity[&(x.from, x.to)] / ONE * ramp(x) / ONE;
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

    #[test]
    fn a_ring_off_one_fooled_account_is_one_lineage_and_promotes_nobody() {
        let mut vs = community();
        vs.extend(ring(6, 300));
        let t = tree(&[d(0)], &counted(&vs));
        // Every fake's lineage is the fooled account's: 1 (6 hangs under 1).
        assert!((1000..1300).all(|i| t.lineage(&d(i)) == Some(d(1))));
        let r = reputation(&[d(0)], &vs);
        assert!(promoted(&[d(0)], &counted(&vs), &t, &r).iter().all(|a| !(1000..1300).contains(&u32::from_be_bytes(a[..4].try_into().unwrap()))));
    }

    #[test]
    fn an_account_trusted_across_three_lineages_heads_a_family_and_adds_no_trust() {
        // 600 is vouched for by 7 (lineage 1), 12 (lineage 2) and 17 (lineage 3); then vouches for 601 and 602.
        let mut vs = community();
        vs.extend([v(7, 600, 3), v(12, 600, 3), v(17, 600, 3), v(600, 601, 4), v(600, 602, 4), v(601, 603, 5), v(602, 603, 5)]);
        let c = counted(&vs);
        let t = tree(&[d(0)], &c);
        let first = flow(&[d(0)], &c, &t, &BTreeSet::new(), &|_| ONE);
        let p = promoted(&[d(0)], &c, &t, &first);
        println!("600's reputation {}; promoted {}", first[&d(600)], p.len());
        assert!(p.contains(&d(600)));
        // Kinship stops at the family's head: 601 and 602 are siblings in 600's family (depth 1 under it) — as before
        // promotion — but against an account outside it they are unrelated.
        assert_eq!(t.closeness_in(&d(601), &d(602), &p), (1, 2));
        assert_eq!(t.closeness_in(&d(601), &d(6), &p).0, 0);
        // No new trust: the whole network's reputation is the same with and without the family (to rounding) or less.
        let after = reputation(&[d(0)], &vs);
        let sum = |r: &BTreeMap<Did, u128>| r.values().sum::<u128>();
        println!("total before {} after {}", sum(&first), sum(&after));
        assert!(sum(&after) <= sum(&first) + sum(&first) / 50, "promotion created trust");
    }

    #[test]
    fn a_ring_off_three_lineages_stays_within_what_it_was_given() {
        // The attacker fooled one account in each of three lineages (6, 11, 16); its 300 fakes vouch each other.
        let mut vs = community();
        vs.extend([v(11, 1000, 5), v(16, 1000, 5)]);
        vs.extend(ring(6, 300));
        let r = reputation(&[d(0)], &vs);
        let flat = flat_reputation(&[d(0)], &vs);
        let (got, bound) = (total(&r, 1000, 1300), total(&flat, 1000, 1300));
        println!("ring off three lineages: {got}; with no discount at all: {bound}");
        // Promotions may give it families, but never more than the flow with no discount — what it was given.
        assert!(got <= bound, "the ring took more than its inputs allow");
    }

    #[test]
    fn a_vouch_ramps_in_over_epochs_and_a_later_one_is_not_seen() {
        // 0 vouches 1 at height 10 (epoch 0); 1 vouches 2 at height 3 × EPOCH (epoch 3).
        let vs = vec![v(0, 1, 10), v(1, 2, 3 * EPOCH)];
        // Read at each epoch's LAST height (the vouch at height 10 exists from then on in epoch 0).
        let at = |e: u64| reputation_at(&[d(0)], &vs, e * EPOCH + EPOCH - 1);
        let one = |e: u64| at(e).get(&d(1)).copied().unwrap_or(0);
        println!("1's reputation by epoch: {} {} {} {} {}", one(0), one(1), one(2), one(3), one(4));
        // A quarter in its first epoch, rising each epoch, full (the unramped value) from the fourth on.
        assert!(one(0) < one(1) && one(1) < one(2) && one(2) < one(3));
        assert_eq!(one(3), one(9));
        assert_eq!(one(3), reputation(&[d(0)], &vs[..1])[&d(1)]);
        assert!((one(0) as i128 - (one(3) / 4) as i128).abs() <= 1, "a quarter in the first epoch");
        // 2's vouch (epoch 3) is not seen before it is made: no reputation for 2 at epoch 2.
        assert_eq!(at(2).get(&d(2)), None);
        assert!(at(3).get(&d(2)).is_some());
        // And at height 9, before 0's own vouch: nobody but the root.
        assert_eq!(reputation_at(&[d(0)], &vs, 9).get(&d(1)), None);
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
