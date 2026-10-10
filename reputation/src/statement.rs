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
    /// Its WITNESS FEES of the epoch's issuance (TOKEN §7d: the pot's 10%, by the signatures it gave — SPENDABLE).
    pub witness: u128,
}

impl Line {
    pub fn encode(&self) -> Vec<u8> {
        [self.rep.to_le_bytes(), self.keeper.to_le_bytes(), self.allowance.to_le_bytes(), self.witness.to_le_bytes()].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Line> {
        (b.len() == 64).then(|| Line {
            rep: u128::from_le_bytes(b[..16].try_into().unwrap()),
            keeper: u128::from_le_bytes(b[16..32].try_into().unwrap()),
            allowance: u128::from_le_bytes(b[32..48].try_into().unwrap()),
            witness: u128::from_le_bytes(b[48..].try_into().unwrap()),
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
    /// The WITNESS signatures each account gave (seated, on the epoch's queued entries: `wallet::fees::count`), the
    /// witness pot and its cap per account.
    pub signed: &'a BTreeMap<Did, u64>,
    pub witness_pot: u128,
    pub witness_cap: u128,
    /// Accounts that never earn (the roots).
    pub never: &'a [Did],
}

/// THE STATEMENT of an epoch: keepers split the keeper pot by reputation among those past the gate who kept (while few
/// kept — bootstrap — every keeper, by reputation plus a floor); everyone past the gate splits the allowance pot by
/// reputation; every reputation is listed, and every keeper (a line for it even with no reputation).
pub fn compute(x: &Inputs) -> Statement {
    let eligible = |d: &Did| x.rep.get(d).copied().unwrap_or(0) >= x.gate && !x.never.contains(d);
    // BOOTSTRAP (`KEEPERS_GATED_FROM`): too few kept for the gate — every keeper earns, reputation plus a floor.
    let bootstrap = x.kept.len() < crate::KEEPERS_GATED_FROM;
    let keepers: BTreeMap<Did, u128> = if bootstrap {
        x.kept.iter().filter(|d| !x.never.contains(d)).map(|d| (*d, x.rep.get(d).copied().unwrap_or(0) + x.gate)).collect()
    } else {
        x.kept.iter().filter(|d| eligible(d)).map(|d| (*d, x.rep[d])).collect()
    };
    let everyone: BTreeMap<Did, u128> = x.rep.iter().filter(|(d, _)| eligible(d)).map(|(d, r)| (*d, *r)).collect();
    let k = shares(x.keeper_pot, x.keeper_cap, &keepers);
    let a = shares(x.allowance_pot, x.allowance_cap, &everyone);
    // WITNESSES: by the signatures they gave (stake made them witnesses: no gate), the roots never.
    let signers: BTreeMap<Did, u128> = x.signed.iter().filter(|(d, n)| **n > 0 && !x.never.contains(d)).map(|(d, n)| (*d, *n as u128)).collect();
    let w = shares(x.witness_pot, x.witness_cap, &signers);
    let lines = x
        .rep
        .keys()
        .chain(k.keys())
        .chain(w.keys())
        .map(|d| (*d, Line { rep: x.rep.get(d).copied().unwrap_or(0), keeper: k.get(d).copied().unwrap_or(0), allowance: a.get(d).copied().unwrap_or(0), witness: w.get(d).copied().unwrap_or(0) }))
        .collect();
    Statement { epoch: x.epoch, lines }
}

/// `did`'s LINE under statement root `root`, from its proof (a prolly proof): `None` — no line, or a proof that does not
/// hold. The one check every payout from a statement makes (the allowance's, the keeper mint's).
pub fn line_proven(root: &Cid, did: &Did, proof: &[u8]) -> Option<Line> {
    match freenet_prolly::proof::verify_bytes(root, did, proof).ok()? {
        freenet_prolly::proof::ProvenOwned::Present(freenet_prolly::proof::ProvenValue::Inline(b)) => Line::decode(&b),
        _ => None,
    }
}

/// The STATEMENT BAG of epoch `epoch` (a bag's address): where anyone posts the statement's root ([`Posted`]).
pub fn bag_address(epoch: u64) -> [u8; 32] {
    blake3::derive_key("craftworks 2026-10-10 epoch statement", &epoch.to_le_bytes())
}

/// A statement's ROOT as posted in its epoch's bag: what witnesses check and sign (TOKEN §7d).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Posted {
    pub epoch: u64,
    pub root: Cid,
    pub parity: Vec<Cid>,
}

impl Posted {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = b"CWST".to_vec();
        v.extend_from_slice(&self.epoch.to_le_bytes());
        v.extend_from_slice(&self.root);
        v.push(self.parity.len() as u8);
        for c in &self.parity {
            v.extend_from_slice(c);
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<Posted> {
        let b = b.strip_prefix(b"CWST")?;
        let epoch = u64::from_le_bytes(b.get(..8)?.try_into().ok()?);
        let root: Cid = b.get(8..40)?.try_into().ok()?;
        let n = *b.get(40)? as usize;
        let rest = b.get(41..)?;
        (rest.len() == n * 32).then(|| Posted { epoch, root, parity: rest.chunks(32).map(|c| c.try_into().unwrap()).collect() })
    }
}

/// The pots an epoch's statement splits: (keeper issuance pot, cap), (allowance pot, cap) — each token's own rule
/// (the allowance's: `craftworks_allowance_contract::{POT, CAP}`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pots {
    pub keeper: (u128, u128),
    pub allowance: (u128, u128),
    pub witness: (u128, u128),
}

/// THE STATEMENT of standing epoch `epoch` — the ONE computation (pages publish it, witnesses recompute it): reputation
/// as of the epoch's START from the signed vouch items standing then (each verified by its voucher's key log,
/// `log_of`), from the [`crate::ROOTS`], split by `pots` (`kept`: the accounts that proved keeping; `signed`: the
/// witness signatures each gave).
pub fn build(epoch: u64, items: &[crate::item::Item], log_of: &dyn Fn(&Did) -> Option<craftworks_idlog_contract::Log>, kept: &[Did], signed: &BTreeMap<Did, u64>, pots: Pots) -> Statement {
    let start = epoch * crate::EPOCH;
    let vs = crate::item::standing(items, start, log_of);
    let rep = crate::reputation_at(crate::ROOTS, &vs, start);
    compute(&Inputs { epoch, rep: &rep, kept, keeper_pot: pots.keeper.0, keeper_cap: pots.keeper.1, gate: crate::GATE, allowance_pot: pots.allowance.0, allowance_cap: pots.allowance.1, signed, witness_pot: pots.witness.0, witness_cap: pots.witness.1, never: crate::ROOTS })
}

/// Where an epoch's statement is WITNESSED (TOKEN §7a, as an entry is): its witness bag is the token's
/// `witness_bag(place, 0)`, a signature names its ROOT as the entry; its lot is drawn at [`lot_height`].
pub fn witness_place(epoch: u64) -> [u8; 32] {
    blake3::derive_key("craftworks 2026-10-10 statement witness place", &epoch.to_le_bytes())
}

/// Heights into an epoch before its statement is computed and posted (page and witness alike): the epoch before's
/// last keeper answers SETTLE first (measured 10-10: computed 2 heights in, a page and a witness read the same records
/// at different moments and disagreed on `kept`).
pub const SETTLE: u64 = 32;

/// The beacon height an epoch's statement's committee is drawn at: just past when it is due ([`SETTLE`] heights into
/// the epoch) — its output exists only after the statement is fixed.
pub fn lot_height(epoch: u64) -> u64 {
    epoch * crate::EPOCH + SETTLE + 2
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
        // GATED (as many kept as `KEEPERS_GATED_FROM`; the rest have no reputation).
        let fill: Vec<Did> = (0..crate::KEEPERS_GATED_FROM as u32).map(|i| *blake3::hash(&i.to_le_bytes()).as_bytes()).collect();
        let kept: Vec<Did> = [d(0), d(1), d(3)].into_iter().chain(fill.iter().copied()).take(crate::KEEPERS_GATED_FROM).collect();
        let none = BTreeMap::new();
        let x = Inputs { epoch: 5, rep: &rep, kept: &kept, keeper_pot: 1_000, keeper_cap: 1_000, gate: ONE / 100_000, allowance_pot: 100, allowance_cap: 100, signed: &none, witness_pot: 0, witness_cap: 0, never: &[d(0)] };
        let s = compute(&x);
        // The root kept but never earns; 3 kept but is below the gate; 2 did not keep; no reputation, no line: only 1
        // earns the keeper pot.
        assert_eq!((s.lines[&d(0)].keeper, s.lines[&d(1)].keeper, s.lines[&d(2)].keeper, s.lines[&d(3)].keeper), (0, 1_000, 0, 0));
        assert!(!s.lines.contains_key(&fill[5]));
        // BOOTSTRAP (fewer kept): every keeper but the root earns — reputation plus the gate's floor, so reputation
        // still weighs more — and one with no reputation has a line to claim from.
        let few = Inputs { kept: &[d(0), d(1), d(3), d(4)], keeper_pot: 1_000_000, keeper_cap: 1_000_000, ..x };
        let b = compute(&few);
        let kp = |n: u8| b.lines.get(&d(n)).map_or(0, |l| l.keeper);
        assert_eq!(kp(0), 0, "the root never earns");
        assert_eq!(kp(2), 0, "did not keep");
        assert!(kp(1) > kp(3) && kp(3) > kp(4) && kp(4) > 0, "{} {} {}", kp(1), kp(3), kp(4));
        assert_eq!(b.lines[&d(4)].rep, 0);
        // The allowance keeps its gate in bootstrap too.
        assert_eq!((b.lines[&d(3)].allowance, b.lines[&d(4)].allowance), (0, 0));
        // WITNESS FEES: by the signatures given — the root never earns; 1 (3 signatures) and 6 (no reputation, 1
        // signature) split the pot 3 : 1; the cap holds.
        let signed: BTreeMap<Did, u64> = [(d(0), 10), (d(1), 3), (d(6), 1)].into_iter().collect();
        let fees = compute(&Inputs { signed: &signed, witness_pot: 400, witness_cap: 400, ..few });
        let wf = |n: u8| fees.lines.get(&d(n)).map_or(0, |l| l.witness);
        assert_eq!((wf(0), wf(1), wf(6)), (0, 300, 100));
        assert_eq!(fees.lines[&d(6)].rep, 0, "a line even without reputation");
        let capped = compute(&Inputs { signed: &signed, witness_pot: 400, witness_cap: 250, ..few });
        assert_eq!(capped.lines[&d(1)].witness, 250, "capped; the rest never minted");
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
        // Its root as posted round-trips.
        let posted = Posted { epoch: 5, root: r1, parity: s.tree(&mut BTreeMap::new()).unwrap().1 };
        assert_eq!(Posted::decode(&posted.encode()), Some(posted.clone()));
        assert_ne!(bag_address(5), bag_address(6));
    }
}
