//! MISDEEDS REACH REPUTATION (TOKEN §7d decision 3, docs/REPUTATION.md §3 "staked on the vouchee"): an account whose
//! misdeed is PROVEN in the misdeed registry (the proofs and their check: `craftworks_rules::misdeed`) loses all its
//! reputation, passes none on, and its DIRECT vouchers are charged by their vouch's weight — the one rule every
//! statement build runs (`statement::build`).
//!
//! - **PROVEN at epoch `p`**: the earliest registry epoch holding a proof that holds. It counts from the statement of
//!   epoch `p + 1` on (a registry epoch is still filling while its own statement is built) — for good.
//! - **THE CHEAT**: its reputation 0 and its vouches dropped from the flow (it passes nothing on); it never earns
//!   (`statement::build` adds it to the accounts that never earn).
//! - **THE CHARGE**: each DIRECT voucher `V` of the cheat — whose vouch for it was COUNTED (among `V`'s earliest
//!   [`crate::BUDGET`]) and standing at the START of epoch `p` (withdrawing after that does not escape it) — loses, in
//!   the statements of epochs `p + 1 ..= p + CHARGE_EPOCHS`, the share of its own reputation that vouch carried:
//!   `f = DAMP × ramp / n` — `DAMP` = [`crate::DAMP_NUM`] / [`crate::DAMP_DEN`] (the share an account passes on), `n`
//!   = `V`'s counted vouches then (its share is split over them), `ramp` = `min(epochs the vouch had stood at p + 1,
//!   RAMP) / RAMP` (a fresh vouch carried less, so costs less). Several cheats add up: `rep(V) × (1 − Σf)`, rounded down
//!   (Σf < DAMP < 1: at most `n` vouches of at most DAMP / n each). Applied to `V`'s own number after the flow — what
//!   `V` passes on to its other vouchees is not charged (they did not vouch for the cheat).
use crate::{counted, Did, Vouch, DAMP_DEN, DAMP_NUM, EPOCH, ONE, RAMP};
use std::collections::BTreeMap;

/// Statements a voucher is charged for, from the first that reads its vouchee's proof (tuned by measurement).
pub const CHARGE_EPOCHS: u64 = 30;

/// The cheats of statement `epoch`: those PROVEN (`proven`: account → earliest registry epoch with a holding proof)
/// before it.
pub fn cheats(epoch: u64, proven: &BTreeMap<Did, u64>) -> BTreeMap<Did, u64> {
    proven.iter().filter(|(_, p)| **p < epoch).map(|(d, p)| (*d, *p)).collect()
}

/// Each charged voucher's lost share (fixed point, [`ONE`]) in statement `epoch`: `standing_at(h)` — the vouches
/// standing at beacon height `h` (`item::standing`).
pub fn charges(epoch: u64, cheats: &BTreeMap<Did, u64>, standing_at: &dyn Fn(u64) -> Vec<Vouch>) -> BTreeMap<Did, u128> {
    let mut lost: BTreeMap<Did, u128> = BTreeMap::new();
    let mut snapshots: BTreeMap<u64, Vec<Vouch>> = BTreeMap::new();
    for (c, p) in cheats {
        if epoch > p + CHARGE_EPOCHS {
            continue;
        }
        let then = snapshots.entry(*p).or_insert_with(|| counted(&standing_at(p * EPOCH)));
        for x in then.iter().filter(|x| x.to == *c && !cheats.contains_key(&x.from)) {
            let n = then.iter().filter(|y| y.from == x.from).count() as u128;
            let stood = (p.saturating_sub(x.at / EPOCH) + 1).min(RAMP) as u128;
            *lost.entry(x.from).or_insert(0) += ONE * DAMP_NUM * stood / (DAMP_DEN * RAMP as u128 * n);
        }
    }
    lost
}

/// REPUTATION after misdeeds: every cheat 0, every charged voucher less by its [`charges`].
pub fn apply(rep: &mut BTreeMap<Did, u128>, cheats: &BTreeMap<Did, u64>, charges: &BTreeMap<Did, u128>) {
    for (v, f) in charges {
        if let Some(r) = rep.get_mut(v) {
            *r -= *r * (*f).min(ONE) / ONE;
        }
    }
    for c in cheats.keys() {
        if let Some(r) = rep.get_mut(c) {
            *r = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(n: u8) -> Did {
        [n; 32]
    }
    fn v(from: u8, to: u8, at: u64) -> Vouch {
        Vouch { from: d(from), to: d(to), at }
    }

    #[test]
    fn a_direct_voucher_is_charged_its_vouchs_share_for_a_period_only() {
        // 1 vouches 2 (the cheat) and 3, both standing a full RAMP before; 4 vouches 2 only, one epoch before.
        let vs = vec![v(1, 2, 0), v(1, 3, 0), v(4, 2, 9 * EPOCH)];
        let cheats: BTreeMap<Did, u64> = [(d(2), 10)].into_iter().collect();
        let c = charges(11, &cheats, &|_| vs.clone());
        // 1: DAMP / 2 (two vouches), fully ramped; 4: DAMP / 1 at a ramp of 2/4.
        assert_eq!(c[&d(1)], ONE * 85 / 200);
        assert_eq!(c[&d(4)], ONE * 85 * 2 / 400);
        assert!(!c.contains_key(&d(3)), "only the cheat's vouchers");
        // The period: still at 10 + CHARGE_EPOCHS, gone after.
        assert!(!charges(10 + CHARGE_EPOCHS, &cheats, &|_| vs.clone()).is_empty());
        assert!(charges(11 + CHARGE_EPOCHS, &cheats, &|_| vs.clone()).is_empty());
        let mut rep: BTreeMap<Did, u128> = [(d(1), ONE), (d(2), ONE), (d(3), ONE), (d(4), ONE)].into_iter().collect();
        apply(&mut rep, &cheats, &c);
        assert_eq!((rep[&d(1)], rep[&d(2)], rep[&d(3)], rep[&d(4)]), (ONE - ONE * 85 / 200, 0, ONE, ONE - ONE * 85 * 2 / 400));
        // Proven at 10: not a cheat of statement 10 (its registry still filling), one of 11.
        assert!(super::cheats(10, &cheats).is_empty());
        assert_eq!(super::cheats(11, &cheats).len(), 1);
    }
}
