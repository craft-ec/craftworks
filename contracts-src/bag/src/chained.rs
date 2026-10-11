//! CHAINED BAGS: an epoch's items in a CHAIN of bags (a bag holds at most the bag contract's `ITEMS_MAX`; one bag an
//! epoch would cap what an epoch can carry). A writer drops into the first bag of the epoch with room (fewer than
//! [`ROOM`] items) and reads back that it was kept (over a cap a bag keeps its most-worked items: not kept → the next);
//! a reader reads k = 0, 1, … and stops at the first with room (a writer moves on only from a full one; reading a bag
//! nobody made makes it, empty). One rule for every such list: the payments bags and the witness queue (the wallet),
//! the files registry, the keepers registry.

/// A bag with fewer items than this takes an item (the bag's own cap, less room for the writers racing it).
pub const ROOM: usize = crate::ITEMS_MAX - 64;

/// Is a bag of these PAYLOADS (lengths) FULL — no more an epoch's next item goes in: `ROOM` items, or its bytes within
/// four of the largest items of the bag's byte cap (audit 10-11: a bag of a few big items fills by BYTES long before it
/// fills by count; a reader stopping only at a count would never reach the bags after it). Writers move on from a full
/// bag and readers read on past it — the one rule both run.
pub fn full(payload_lens: impl IntoIterator<Item = usize>) -> bool {
    let (mut n, mut bytes) = (0usize, crate::MAGIC.len() + 2);
    for len in payload_lens {
        n += 1;
        bytes += 4 + 8 + len;
    }
    n >= ROOM || bytes + 4 * (4 + crate::ITEM_MAX) > crate::STATE_MAX
}

/// Bag `k` of `epoch` in the chain named `domain` (a bag's address).
pub fn address(domain: &str, epoch: u64, k: u32) -> [u8; 32] {
    blake3::derive_key(domain, &[&epoch.to_le_bytes()[..], &k.to_le_bytes()].concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bag_is_full_by_count_or_by_bytes() {
        assert!(!full([10usize; 100]));
        assert!(full(vec![10usize; ROOM]), "ROOM small items");
        // A few big items: far under ROOM by count, full by bytes (a reader stopping only at a count would stop here).
        assert!(full([crate::ITEM_MAX - 8; 12]), "12 items of 16 KiB");
        assert!(!full([crate::ITEM_MAX - 8; 8]), "room for four more of the largest");
    }
}
