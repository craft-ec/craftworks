//! $CRAFT (TOKEN §7d, owner 10-10: "i trust your projection"): the storage token, minted ONLY by keeper issuance — no
//! premine, no sale, no authority. Its DEFINITION is fixed here, the same bytes on every page and tool for a network's
//! beacon, so anyone may put it and nobody owns it: max 1,000,000,000 (9 decimals), a POT per standing epoch of
//! ~61,035, halving every 8,192 epochs (the geometric sum is the cap) — 90% to the epoch's KEEPERS by its statement
//! (each at most 2% of the pot), 10% to its WITNESSES.

use craftworks_token_contract::{self as t, Definition, Keepers};

/// The most $CRAFT there will ever be, in units (10⁹ × 10⁹).
pub const MAX: u128 = 1_000_000_000 * t::UNIT;
/// Standing epochs between halvings.
pub const HALF_LIFE: u64 = 8_192;
/// The first epoch's POT: half the cap over the first half-life (the halvings sum to the cap).
pub const POT: u128 = MAX / 2 / HALF_LIFE as u128;
/// The issuance rule.
pub const KEEPERS: Keepers = Keepers { rate: POT, half_life: HALF_LIFE, start: 0 };

/// $CRAFT's definition on the network of beacon `beacon` (the token code `code_hash`, the key log code `idlog`).
pub fn definition(code_hash: [u8; 32], beacon: [u8; 32], idlog: [u8; 32]) -> Definition {
    Definition {
        code_hash,
        decimals: t::DECIMALS,
        min_send: 1,
        genesis_owner: [0; 32],
        genesis_amount: 0,
        name: b"CRAFT".to_vec(),
        nonce: *b"craftworks craft",
        beacon,
        margin: 2,
        idlog,
        mint: None,
        freeze: None,
        ids: t::Ids::None,
        royalty: None,
        cap: 0,
        item: [0; 32],
        version: [0; 32],
        soulbound: false,
        transfer_fee: None,
        sale: None,
        keepers: Some(KEEPERS),
    }
}

/// The KEEPER POT of standing epoch `epoch` and its cap per account: 90% of the epoch's pot, at most 2% of it each.
pub fn keeper_pot(epoch: u64) -> (u128, u128) {
    let pot = KEEPERS.amount(epoch);
    (pot / 10 * 9, pot / 50)
}

/// The WITNESS POT of standing epoch `epoch` and its cap per account: 10% of the epoch's pot (SPENDABLE: fees never
/// become stake), at most 2% of it each.
pub fn witness_pot(epoch: u64) -> (u128, u128) {
    let pot = KEEPERS.amount(epoch);
    (pot / 10, pot / 50)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn craft_is_one_definition_whose_pots_sum_to_its_cap() {
        let d = definition([1; 32], [2; 32], [3; 32]);
        assert_eq!(Definition::decode(&d.encode()), Some(d.clone()), "a valid definition");
        assert_eq!(t::id(&[1; 32], &d.encode()), t::id(&[1; 32], &definition([1; 32], [2; 32], [3; 32]).encode()), "the same bytes everywhere");
        assert_eq!(POT / t::UNIT, 61_035, "~61,035 a standing epoch");
        // Every epoch's pot over all halvings: at most the cap (integer halving leaves dust under it).
        let total: u128 = (0..128u64).map(|h| KEEPERS.amount(h * HALF_LIFE) * HALF_LIFE as u128).sum();
        assert!(total <= MAX && MAX - total < MAX / 1_000_000_000, "{total} vs {MAX}");
        let (k, cap) = keeper_pot(0);
        assert_eq!((k, cap), (POT / 10 * 9, POT / 50));
        assert_eq!(keeper_pot(HALF_LIFE).0, (POT / 2) / 10 * 9, "halved");
    }
}
