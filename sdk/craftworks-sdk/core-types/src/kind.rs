//! THE BLOCK KINDS, ONE LIST (the owner's "structure before code", 2026-09-26): every kind a block can be stored,
//! fetched or packed as. `engine::read::matches_id` once listed three of the four by hand and missed PARITY, so a
//! fetched parity block was judged "not its id" and never kept (craftworks-sdk, main's report). Every check, table
//! and parser now reads this enum, and matches it EXHAUSTIVELY -- no `_ =>` over it -- so a new kind fails to compile
//! until each of them has handled it.
//!
//! Its BYTE is the first byte of the Block contract's state, hashed into the block's id (`BLAKE3(kind ‖ body)`).
//! RAW, TREE_NODE and PARITY are freenet-prolly's `kind` constants; PACK is the Block contract's own. This crate has
//! no dependencies, so the bytes are stated here and pinned equal to freenet-prolly's by `engine/tests/kinds.rs`.

/// A kind of block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlockKind {
    /// Opaque bytes: a value stored outside its leaf.
    Raw,
    /// A tree node.
    TreeNode,
    /// One coded symbol of a sibling group.
    Parity,
    /// A pack: a transport of several blocks in one.
    Pack,
}

impl BlockKind {
    /// Every kind, in [`index`](Self::index) order. Complete by test: each kind's exhaustive `index` is its place here.
    pub const ALL: [BlockKind; 4] = [BlockKind::Raw, BlockKind::TreeNode, BlockKind::Parity, BlockKind::Pack];

    /// Its place in [`ALL`](Self::ALL). Exhaustive: a new kind fails to compile here, and the test fails until
    /// `ALL` holds it at this place.
    pub const fn index(self) -> usize {
        match self {
            BlockKind::Raw => 0,
            BlockKind::TreeNode => 1,
            BlockKind::Parity => 2,
            BlockKind::Pack => 3,
        }
    }

    /// Its byte: the Block contract state's first byte, hashed into the block's id.
    pub const fn byte(self) -> u8 {
        match self {
            BlockKind::Raw => 0,
            BlockKind::TreeNode => 1,
            BlockKind::Parity => 4,
            BlockKind::Pack => 6,
        }
    }

    /// The kind whose byte this is, or `None` for a byte no kind has. Found in [`ALL`](Self::ALL), never matched
    /// by a second list of bytes.
    pub const fn of_byte(b: u8) -> Option<BlockKind> {
        let mut i = 0;
        while i < BlockKind::ALL.len() {
            if BlockKind::ALL[i].byte() == b {
                return Some(BlockKind::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::BlockKind;

    /// `ALL` is every kind, each at its own `index`, and no two share a byte. A kind added to the enum and not to
    /// `ALL` fails here (its exhaustive `index` is past the list).
    #[test]
    fn all_is_every_kind_once_and_bytes_are_distinct() {
        for (i, k) in BlockKind::ALL.iter().enumerate() {
            assert_eq!(k.index(), i, "{k:?} is not at its own index in ALL");
            assert_eq!(BlockKind::of_byte(k.byte()), Some(*k), "{k:?}'s byte does not name it");
        }
        let mut bytes: Vec<u8> = BlockKind::ALL.iter().map(|k| k.byte()).collect();
        bytes.sort_unstable();
        bytes.dedup();
        assert_eq!(bytes.len(), BlockKind::ALL.len(), "two kinds share a byte");
        assert_eq!(BlockKind::of_byte(2), None, "a byte no kind has named one");
    }
}
