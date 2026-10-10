//! THE EPOCH STATEMENT (TOKEN §7d): one number set every witness computes alike — each account's REPUTATION as of the
//! epoch's start, its KEEPER share of the epoch's issuance and its WREN ALLOWANCE — published as a PROLLY TREE (built the
//! one way a whole tree is built, `engine::repair::build_tree`) whose ROOT witnesses sign; a claim cites its line with a
//! prolly proof. Anyone computes and publishes it (TOKEN §7d, the two rules).
use crate::Did;
use freenet_prolly::Cid;
use std::collections::BTreeMap;

/// THE SPLIT (the one pot-split rule: keeper issuance, the witness slice, the WREN allowance): `pot` divided by `weights`,
/// each share rounded DOWN and capped at `cap`; what rounding and the cap leave is NEVER PAID (never minted or sent) —
/// so moving weight around cannot gain anything the cap took. A zero weight gets nothing.
pub fn shares(pot: u128, cap: u128, weights: &BTreeMap<Did, u128>) -> BTreeMap<Did, u128> {
    let total: u128 = weights.values().sum();
    if total == 0 {
        return BTreeMap::new();
    }
    weights
        .iter()
        .filter(|(_, w)| **w > 0)
        .map(|(d, w)| (*d, (pot.saturating_mul(*w) / total).min(cap)))
        .filter(|(_, a)| *a > 0)
        .collect()
}

/// One account's LINE in an epoch's statement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Line {
    /// Reputation as of the epoch's start (fixed point, [`crate::ONE`]).
    pub rep: u128,
    /// Its keeper share of the epoch's issuance (0: did not prove keeping, or below the gate).
    pub keeper: u128,
    /// Its WREN allowance for the period this epoch closes (0: none).
    pub allowance: u128,
}

impl Line {
    pub fn encode(&self) -> Vec<u8> {
        [self.rep.to_le_bytes(), self.keeper.to_le_bytes(), self.allowance.to_le_bytes()].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Line> {
        (b.len() == 48).then(|| Line {
            rep: u128::from_le_bytes(b[..16].try_into().unwrap()),
            keeper: u128::from_le_bytes(b[16..32].try_into().unwrap()),
            allowance: u128::from_le_bytes(b[32..].try_into().unwrap()),
        })
    }
}

/// An epoch's statement: every account with a line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Statement {
    pub epoch: u64,
    pub lines: BTreeMap<Did, Line>,
}

/// What the statement is computed from (all public, all the same everywhere).
pub struct Inputs<'a> {
    pub epoch: u64,
    /// Reputation as of the epoch's start (`reputation_at`).
    pub rep: &'a BTreeMap<Did, u128>,
    /// The accounts that PROVED keeping this epoch (their standing records).
    pub kept: &'a [Did],
    /// The epoch's keeper pot and the cap per account.
    pub keeper_pot: u128,
    pub keeper_cap: u128,
    /// The GATE: the least reputation that earns (a trust share, fixed point).
    pub gate: u128,
    /// The WREN allowance's pot and cap for the period this epoch closes (0: this epoch closes none).
    pub allowance_pot: u128,
    pub allowance_cap: u128,
    /// Accounts that never earn (the roots).
    pub never: &'a [Did],
}

/// THE STATEMENT of an epoch: keepers split the keeper pot by reputation among those past the gate who kept; everyone
/// past the gate splits the allowance pot by reputation; every reputation is listed.
pub fn compute(x: &Inputs) -> Statement {
    let eligible = |d: &Did| x.rep.get(d).copied().unwrap_or(0) >= x.gate && !x.never.contains(d);
    let keepers: BTreeMap<Did, u128> = x.kept.iter().filter(|d| eligible(d)).map(|d| (*d, x.rep[d])).collect();
    let everyone: BTreeMap<Did, u128> = x.rep.iter().filter(|(d, _)| eligible(d)).map(|(d, r)| (*d, *r)).collect();
    let k = shares(x.keeper_pot, x.keeper_cap, &keepers);
    let a = shares(x.allowance_pot, x.allowance_cap, &everyone);
    let lines = x
        .rep
        .iter()
        .map(|(d, r)| (*d, Line { rep: *r, keeper: k.get(d).copied().unwrap_or(0), allowance: a.get(d).copied().unwrap_or(0) }))
        .collect();
    Statement { epoch: x.epoch, lines }
}

impl Statement {
    /// Its TREE (DID → line) — blocks added to `blocks`: its root (what witnesses sign) and the root's parity ids.
    pub fn tree(&self, blocks: &mut BTreeMap<Cid, Vec<u8>>) -> Option<(Cid, Vec<Cid>)> {
        engine::repair::build_tree(self.lines.iter().map(|(d, l)| (d.to_vec(), l.encode())), blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ONE;

    fn d(n: u8) -> Did {
        [n; 32]
    }

    #[test]
    fn the_split_is_by_weight_capped_rounded_down_and_never_pays_the_rest() {
        let w: BTreeMap<Did, u128> = [(d(1), 913), (d(2), 79), (d(3), 8), (d(4), 0)].into_iter().collect();
        let s = shares(10_000, 200, &w);
        // 1 would take 9,130 (capped 200), 2 790 (capped 200), 3 80; 4 nothing.
        assert_eq!(s, [(d(1), 200), (d(2), 200), (d(3), 80)].into_iter().collect());
        assert_eq!(10_000 - s.values().sum::<u128>(), 9_520, "the rest is not paid");
        // Uncapped: the parts never exceed the pot (rounded down).
        let u = shares(1_000, u128::MAX, &[(d(1), 1), (d(2), 1), (d(3), 1)].into_iter().collect());
        assert_eq!(u.values().sum::<u128>(), 999);
        assert!(shares(1_000, 10, &BTreeMap::new()).is_empty());
        // A weight whose share rounds down to nothing is not listed (nothing to pay).
        let tiny = shares(10, 10, &[(d(1), 1_000), (d(2), 1)].into_iter().collect());
        assert_eq!(tiny, [(d(1), 9)].into_iter().collect());
    }

    #[test]
    fn a_tree_of_one_split_gains_nothing_over_one_account() {
        // One account with weight 100, or the same 100 split over 1,000 fakes: the same total paid (to rounding).
        let one: BTreeMap<Did, u128> = [(d(1), 100), (d(2), 900)].into_iter().collect();
        let mut many: BTreeMap<Did, u128> = [(d(2), 900)].into_iter().collect();
        for i in 0..100u8 {
            let mut x = [7u8; 32];
            x[0] = i;
            many.insert(x, 1);
        }
        let a = shares(10_000, 10_000, &one)[&d(1)];
        let b: u128 = shares(10_000, 10_000, &many).iter().filter(|(k, _)| k[1] == 7).map(|(_, v)| v).sum();
        println!("one account: {a}; its weight split over 100: {b}");
        assert!(b <= a, "splitting gained {b} over {a}");
    }

    #[test]
    fn the_statement_pays_keepers_past_the_gate_never_the_root_and_its_tree_is_canonical() {
        let rep: BTreeMap<Did, u128> = [(d(0), ONE), (d(1), ONE / 10), (d(2), ONE / 100), (d(3), ONE / 1_000_000)].into_iter().collect();
        let x = Inputs { epoch: 5, rep: &rep, kept: &[d(0), d(1), d(3)], keeper_pot: 1_000, keeper_cap: 1_000, gate: ONE / 100_000, allowance_pot: 100, allowance_cap: 100, never: &[d(0)] };
        let s = compute(&x);
        // The root kept but never earns; 3 kept but is below the gate; 2 did not keep: only 1 earns the keeper pot.
        assert_eq!((s.lines[&d(0)].keeper, s.lines[&d(1)].keeper, s.lines[&d(2)].keeper, s.lines[&d(3)].keeper), (0, 1_000, 0, 0));
        // The allowance: everyone past the gate but the root, by reputation (1 and 2: 100 × 10/11, 100 × 1/11).
        assert_eq!((s.lines[&d(1)].allowance, s.lines[&d(2)].allowance, s.lines[&d(3)].allowance), (90, 9, 0));
        assert_eq!(Line::decode(&s.lines[&d(1)].encode()), Some(s.lines[&d(1)]));
        // The tree: the same statement, the same root, however it is built; one changed line, another root.
        let (mut b1, mut b2, mut b3) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        let r1 = s.tree(&mut b1).unwrap().0;
        assert_eq!(compute(&x).tree(&mut b2).unwrap().0, r1);
        let mut other = s.clone();
        other.lines.get_mut(&d(2)).unwrap().allowance += 1;
        assert_ne!(other.tree(&mut b3).unwrap().0, r1);
        // A line is provable from the root (a prolly proof): a claim cites it.
        let proof = freenet_prolly::proof::prove(&b1, &r1, &d(1)).unwrap();
        assert!(freenet_prolly::proof::verify(&r1, &d(1), &proof).is_ok());
    }
}
