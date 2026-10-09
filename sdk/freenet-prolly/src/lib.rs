//! Prolly tree: a sorted `key -> value` map whose shape and root hash are a pure
//! function of its contents. See ARCHITECTURE.md §5. Built in phase 1.

pub mod aggregate;
pub mod apply;
pub mod boundary;
pub mod build;
pub mod chunk;
pub mod cursor;
pub mod diff;
pub mod node;
pub mod pack;
pub mod parity;
pub mod proof;
pub mod range;
pub mod read;
pub mod rs;
pub mod store;

/// Id of a block. It is the Block contract's key material, so a child pointer
/// read from a parent is exactly what fetches the child from the network.
pub type Cid = [u8; 32];

/// What a block holds. The values are the Block contract's kind bytes and are
/// part of this format: they are hashed into every id.
pub mod kind {
    /// Opaque bytes: a value stored outside its leaf.
    pub const RAW: u8 = 0;
    /// A tree node (`PT01`).
    pub const TREE_NODE: u8 = 1;
    /// One coded symbol of a sibling group ([`crate::parity`]). Defined here
    /// because a parity block's id is `block_id(PARITY, symbol)` and the tree
    /// is what computes it; the Block contract holds the same byte.
    pub const PARITY: u8 = 4;
}

/// The id of a block of `kind` holding `body`: `BLAKE3(kind ‖ body)`, which is
/// the hash of the Block contract's state for it.
pub fn block_id(kind: u8, body: &[u8]) -> Cid {
    let mut h = blake3::Hasher::new();
    h.update(&[kind]);
    h.update(body);
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_the_hash_of_the_contract_state() {
        // Computed the way the contract does: hash of the encoded state.
        let body = b"PT01 some node bytes";
        let mut state = vec![kind::TREE_NODE];
        state.extend_from_slice(body);
        assert_eq!(
            block_id(kind::TREE_NODE, body),
            *blake3::hash(&state).as_bytes()
        );
        assert_ne!(block_id(kind::TREE_NODE, body), block_id(kind::RAW, body));
        assert_ne!(block_id(kind::RAW, body), *blake3::hash(body).as_bytes());
    }
}
