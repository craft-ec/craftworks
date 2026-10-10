//! CHAINED BAGS: an epoch's items in a CHAIN of bags (a bag holds at most the bag contract's `ITEMS_MAX`; one bag an
//! epoch would cap what an epoch can carry). A writer drops into the first bag of the epoch with room (fewer than
//! [`ROOM`] items) and reads back that it was kept (over a cap a bag keeps its most-worked items: not kept → the next);
//! a reader reads k = 0, 1, … and stops at the first with room (a writer moves on only from a full one; reading a bag
//! nobody made makes it, empty). One rule for every such list: the payments bags (`stealth`), the witness queue
//! (`queue`).

/// A bag with fewer items than this takes an item (the bag's own cap, less room for the writers racing it).
pub const ROOM: usize = craftworks_bag_contract::ITEMS_MAX - 64;

/// Bag `k` of `epoch` in the chain named `domain` (a bag's address).
pub fn address(domain: &str, epoch: u64, k: u32) -> [u8; 32] {
    blake3::derive_key(domain, &[&epoch.to_le_bytes()[..], &k.to_le_bytes()].concat())
}
