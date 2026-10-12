//! THE LOT (TOKEN §7a): who witnesses an entry. A STAKE CLAIM per staker per epoch; per entry a TICKET per claimer —
//! an exponential race, `−ln u / stake`, in fixed-point integers (the same on every machine) — and the COMMITTEE, the
//! k lowest. Stake-proportional exactly: the lowest of exponentials with rates w₁…wₙ is claimer i with chance wᵢ/Σw,
//! so splitting stake across identities gains nothing.

/// A SEAT in claims epoch `epoch`'s lots: staker `owner` (a DID) with `amount` of stake, witness key `key`. Never made from
/// what anyone says: only `wallet::valid_stakers` makes these — from a `Body::Claim` in the staker's own pool chain,
/// accepted before the epoch's freeze.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StakeClaim {
    pub owner: [u8; 32],
    pub amount: u128,
    pub epoch: u64,
    pub key: [u8; 32],
}

/// log₂(x) in fixed point (32 fractional bits), x ≥ 1 — integer steps only.
pub fn log2_fx(x: u64) -> u64 {
    let n = 63 - x.leading_zeros() as u64;
    // y = x / 2ⁿ in [1, 2), held as Q62.
    let mut y: u128 = ((x as u128) << 62) >> n;
    let mut frac = 0u64;
    for i in (0..32).rev() {
        y = (y * y) >> 62;
        if y >= 2u128 << 62 {
            y >>= 1;
            frac |= 1 << i;
        }
    }
    (n << 32) | frac
}

/// The TICKET of `owner` (staking `amount`) for the entry at (`chain`, `seq`), under beacon output `out`: lower wins.
pub fn ticket(out: &[u8; 32], chain: &[u8; 32], seq: u64, owner: &[u8; 32], amount: u128) -> u128 {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-09 token lot");
    h.update(out);
    h.update(chain);
    h.update(&seq.to_le_bytes());
    h.update(owner);
    let x = u64::from_le_bytes(h.finalize().as_bytes()[..8].try_into().unwrap()) | 1;
    // −log₂(u) for u = x / 2⁶⁴, in Q32 — then divided by the stake, scaled up first to keep its precision.
    let neg = (64u128 << 32) - log2_fx(x) as u128;
    (neg << 64) / amount.max(1)
}

/// The seats of `epoch` (of `V(epoch)`, for the pool `_pool`), one per owner (its first in the list) and of some stake:
/// (owner, amount, index) — taken once, then drawn from for every entry.
pub fn valid(_pool: &[u8; 32], epoch: u64, claims: &[StakeClaim]) -> Vec<([u8; 32], u128, usize)> {
    let mut out: Vec<([u8; 32], u128, usize)> = Vec::new();
    for (i, c) in claims.iter().enumerate() {
        if c.epoch == epoch && c.amount > 0 && !out.iter().any(|x| x.0 == c.owner) {
            out.push((c.owner, c.amount, i));
        }
    }
    out
}

/// The COMMITTEE for (`chain`, `seq`) among `valid` claims: the `k` lowest tickets, as indexes into the claims, lowest
/// first.
pub fn committee_of(out: &[u8; 32], chain: &[u8; 32], seq: u64, valid: &[([u8; 32], u128, usize)], k: usize) -> Vec<usize> {
    let mut t: Vec<(u128, [u8; 32], usize)> = valid.iter().map(|(o, a, i)| (ticket(out, chain, seq, o, *a), *o, *i)).collect();
    t.sort();
    t.into_iter().take(k).map(|x| x.2).collect()
}

/// [`committee_of`] over the claims that hold.
pub fn committee(definition: &[u8; 32], out: &[u8; 32], chain: &[u8; 32], seq: u64, epoch: u64, claims: &[StakeClaim], k: usize) -> Vec<usize> {
    committee_of(out, chain, seq, &valid(definition, epoch, claims), k)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEF: [u8; 32] = [5; 32];
    fn claim(n: u8, amount: u128) -> StakeClaim {
        claim_of(n, [n; 32], amount, 1)
    }
    /// A seat (key n, for `owner`, `amount`, `epoch`).
    fn claim_of(n: u8, owner: [u8; 32], amount: u128, epoch: u64) -> StakeClaim {
        StakeClaim { owner, amount, epoch, key: [n; 32] }
    }

    #[test]
    fn the_fixed_point_log_is_exact_on_powers_and_close_between() {
        for n in [1u64, 2, 1 << 10, 1 << 40, 1 << 63] {
            assert_eq!(log2_fx(n), (63 - n.leading_zeros() as u64) << 32, "{n}");
        }
        for x in [3u64, 1000, 123_456_789, u64::MAX] {
            let got = log2_fx(x) as f64 / (1u64 << 32) as f64;
            assert!((got - (x as f64).log2()).abs() < 1e-8, "{x}: {got}");
        }
    }

    #[test]
    fn the_lot_is_stake_proportional_and_splitting_gains_nothing() {
        let claims = vec![claim(1, 100), claim(2, 200), claim(3, 700)];
        let mut wins = [0u32; 3];
        let n = 20_000u64;
        let v = valid(&DEF, 1, &claims);
        for seq in 0..n {
            wins[committee_of(&[9; 32], &[8; 32], seq, &v, 1)[0]] += 1;
        }
        let share: Vec<f64> = wins.iter().map(|w| *w as f64 / n as f64).collect();
        println!("shares for stakes 100/200/700 over {n} draws: {share:?}");
        for (s, want) in share.iter().zip([0.1, 0.2, 0.7]) {
            assert!((s - want).abs() < 0.015, "{share:?}");
        }
        // 700 split across seven identities of 100: the same share for the splitter as a whole.
        let mut split = vec![claim(1, 100), claim(2, 200)];
        split.extend((10..17).map(|n| claim(n, 100)));
        let mut theirs = 0u32;
        let v = valid(&DEF, 1, &split);
        for seq in 0..n {
            if committee_of(&[9; 32], &[8; 32], seq, &v, 1)[0] >= 2 {
                theirs += 1;
            }
        }
        let s = theirs as f64 / n as f64;
        println!("seven identities of 100: {s}");
        assert!((s - 0.7).abs() < 0.015, "{s}");
    }

    #[test]
    fn the_committee_is_the_lowest_holding_claims_one_per_owner_of_the_epoch() {
        let mut claims = vec![claim(1, 100), claim(2, 100), claim(3, 100)];
        let c = committee(&DEF, &[9; 32], &[8; 32], 5, 1, &claims, 2);
        assert_eq!(c.len(), 2);
        assert_eq!(c, committee(&DEF, &[9; 32], &[8; 32], 5, 1, &claims, 2), "the same for everyone");
        assert_ne!(committee(&DEF, &[9; 32], &[8; 32], 5, 1, &claims, 3), committee(&DEF, &[10; 32], &[8; 32], 5, 1, &claims, 3), "another output, another order (here)");
        // Another epoch's, a second of one owner, a stake of nothing: out.
        claims.push(claim_of(5, [5; 32], 1_000_000, 2));
        claims.push(claim_of(6, [1; 32], 1_000_000, 1));
        claims.push(claim_of(7, [7; 32], 0, 1));
        assert_eq!(valid(&DEF, 1, &claims).len(), 3, "another epoch's, an owner's second, a stake of nothing: out");
        for i in committee(&DEF, &[9; 32], &[8; 32], 5, 1, &claims, 10) {
            assert!(i < 3, "only the three seats: {i}");
        }
    }
}
