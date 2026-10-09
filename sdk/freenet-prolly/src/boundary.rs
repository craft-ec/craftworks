//! Where one node ends and the next begins.
//!
//! The decision is made per entry, *after* the entry is added to the open node:
//!
//! ```text
//! h32 = first 4 bytes (LE) of BLAKE3("PT01-split" ‖ level ‖ key)
//! split after this entry  iff  s_after ≥ MIN_SPLIT
//!                          and  h32 · λ⁴  <  (s_after⁴ − s_before⁴) · 2³²
//! ```
//! where `s` is the open node's [`NodeBuilder::logical_len`](crate::node::NodeBuilder::logical_len)
//! before and after the entry. The hash depends on the key alone, so a value
//! edit never changes *whether an entry can be* a boundary; the size term makes
//! a split ever more likely as the node grows (a Weibull hazard of shape 4),
//! which keeps node sizes tight around the mean instead of geometrically spread.
//!
//! Everything is integer arithmetic in `u128` (both sides stay below 2⁸⁹):
//! boundaries decide hashes, and float rounding is not portable across targets.
//!
//! An entry that would push the open node past `MAX_LOGICAL` closes the
//! node *before* it instead. That forced split is the only one not decided by
//! content; the hazard makes it vanishingly rare.
//!
//! `LAMBDA`, the limits and the hash domain are format constants: changing any
//! of them changes every root hash. They are pinned by `tests/vectors.txt`.
//! "Frozen" means frozen for PT01 code and vectors. Node size stays a measured
//! question until the phase-3 gate (put latency and the delegate's ops-per-round
//! cap may favour larger nodes); no stored data exists before then, so reopening
//! it costs only regenerated vectors.

use crate::node::{Node, NodeBuilder, HEADER, MAX_KEY, MAX_NODE};

/// Scale of the split hazard. Mean `logical_len` ≈ 0.906·λ plus about half an entry.
pub const LAMBDA: u64 = 4400;
/// No content-defined split closes a node smaller than this.
pub const MIN_SPLIT: usize = 1024;
/// Hard limit on any node's `logical_len`. The remaining 4 KiB of the 16 KiB
/// node is reserved for parity cids, which are not part of the boundary measure
/// (so how parity is grouped can change without moving a boundary).
pub const MAX_LOGICAL: usize = 12 * 1024;

// A content-defined split never closes a branch holding a single child: even the
// largest child entry leaves the node below MIN_SPLIT. Only the last node of a
// level can have one child. Raising MAX_KEY must not silently break this.
const _: () = assert!(HEADER + 6 + 50 + MAX_KEY < MIN_SPLIT);
const _: () = assert!(MAX_LOGICAL + 4096 <= MAX_NODE);

const DOMAIN: &[u8] = b"PT01-split";
const LAMBDA4: u128 = (LAMBDA as u128).pow(4);

/// What the node's own entries require of its parity region, and what it
/// claims. Three ids per sibling group, and the grouping is a pure function of
/// the keys (and, on a leaf, of the referenced values' size classes), so this
/// is decidable from the node alone.
///
/// It says nothing about the ids themselves. Verifying parity CONTENTS needs
/// the children, which is 8+ related fetches per validation; a writer can still
/// list garbage, but only a fixed number of ids, only harming recovery of its
/// own tree, and a reader finds out the first time a rebuild fails to hash.
pub fn check_parity(node: &Node<'_>) -> Result<(), BoundaryError> {
    let want = crate::parity::pcount_of(node);
    let got = node.parity_count();
    if want != got {
        return Err(BoundaryError::WrongParityCount(want, got));
    }
    Ok(())
}

/// The per-entry split hash.
pub fn split_hash(level: u8, key: &[u8]) -> u32 {
    split_hash_parts(level, key, &[])
}

/// [`split_hash`] of `prefix ‖ suffix`, without joining them.
///
/// A node stores each key as the node's shared prefix plus that entry's suffix,
/// and BLAKE3 is a streaming hash: feeding the two pieces in order is the same
/// digest as feeding the concatenation. So a reader checking a whole node never
/// has to allocate a key — which was almost the entire cost of [`check_node`].
pub fn split_hash_parts(level: u8, prefix: &[u8], suffix: &[u8]) -> u32 {
    let mut h = blake3::Hasher::new();
    h.update(DOMAIN);
    h.update(&[level]);
    h.update(prefix);
    h.update(suffix);
    let d = h.finalize();
    let b = d.as_bytes();
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Whether a node at `level` closes after the entry with `key`, which grew the
/// node's `logical_len` from `s_before` to `s_after`.
pub fn splits_after(level: u8, key: &[u8], s_before: usize, s_after: usize) -> bool {
    splits_after_parts(level, key, &[], s_before, s_after)
}

/// [`splits_after`] for a key held as `prefix ‖ suffix`.
pub fn splits_after_parts(
    level: u8,
    prefix: &[u8],
    suffix: &[u8],
    s_before: usize,
    s_after: usize,
) -> bool {
    debug_assert!(s_before < s_after && s_after <= MAX_LOGICAL);
    if s_after < MIN_SPLIT {
        return false;
    }
    let p4 = |s: usize| (s as u128).pow(4);
    let lhs = split_hash_parts(level, prefix, suffix) as u128 * LAMBDA4;
    let rhs = (p4(s_after) - p4(s_before)) << 32;
    lhs < rhs
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryError {
    /// The node should have ended after entry `.0`, and goes on.
    InteriorSplit(usize),
    /// The entries measure more than `MAX_LOGICAL`.
    TooLarge,
    /// `pcount` is not what the node's own entries require. The count is a pure
    /// function of them — `parity::PARITY` ids per sibling group — so a host can check it
    /// exactly, and the parity region stops being the one part of a valid node
    /// no rule constrains. `.0` is what the entries require, `.1` what the node
    /// claims.
    WrongParityCount(usize, usize),
}

/// The part of the split rule that one node can be held to: no entry but the
/// last satisfies the rule at its position, and the node is within the hard
/// limit. Whether the LAST entry should have closed the node cannot be told from
/// the node alone (the last node of a level, and a node closed by the hard
/// limit, end without it), so that is not checked.
///
/// For hosts: a parsed node that fails this was not produced by the format's
/// chunker, and keeping it would poison dedup and diff for its readers.
///
/// # What this cannot prove
///
/// The split hash is salted with the node's own `level` field, and nothing here
/// constrains it — so a node cut anywhere could in principle be relabelled to a
/// level under which its cut passes, a search over at most 255 values done
/// offline. Two things make that worthless rather than dangerous, and both
/// belong here rather than in a reviewer's memory:
///
/// - A **leaf cannot be relabelled at all.** Level 0 has a different entry shape,
///   so a leaf claiming to be a branch fails [`Node::parse`] outright — and
///   leaves are about 98 % of a tree's nodes.
/// - A **relabelled branch is unreachable.** [`load_child`](crate::store::load_child)
///   requires a child's level to be its parent's minus one, anchored at the
///   leaves, so no reader following a root will ever arrive at it. It is orphan
///   garbage under a different id: it costs its writer a PUT and gains nothing.
///
/// So this check guards against accidents and lazy writers. The reader is the
/// enforcer.
pub fn check_node(node: &Node<'_>) -> Result<(), BoundaryError> {
    // Keys are read as the node's shared prefix plus each entry's suffix and are
    // never joined: this runs on every tree-node block on every hosting node, and
    // an allocation per entry was almost the whole of its cost.
    let prefix = node.prefix();
    let level = node.level();
    let mut s = HEADER;
    for i in 0..node.len() {
        let suffix = node.suffix(i);
        let klen = prefix.len() + suffix.len();
        let after = s + if node.is_leaf() {
            NodeBuilder::leaf_cost_len(klen, &node.value(i))
        } else {
            NodeBuilder::child_cost_len(klen)
        };
        if after > MAX_LOGICAL {
            return Err(BoundaryError::TooLarge);
        }
        if i + 1 < node.len() && splits_after_parts(level, prefix, suffix, s, after) {
            return Err(BoundaryError::InteriorSplit(i));
        }
        s = after;
    }
    // Last, because it walks the keys again and the entry checks above are the
    // cheaper refusals: a node that is mis-cut or oversized is refused without
    // paying for the grouping hashes.
    check_parity(node)
}

/// The reserve arithmetic, derived here rather than asserted from memory.
///
/// Both worst cases must fit the 4 KiB the node format reserves, and both
/// depend on the SMALLEST entry of their kind and on [`MIN_GROUP`]: a smaller
/// minimum group means more groups means more ids. The `+ 1` is the one short
/// tail a run can end with, and the leaf's `+ CLASSES` is one per size class,
/// since each class's run ends independently.
///
/// If any of MAX_LOGICAL, the minimum entry costs, the class count or the
/// minimum group size moves, this fails at compile time instead of a node
/// becoming unencodable at run time.
mod reserve {
    use super::MAX_LOGICAL;
    use crate::node::{HEADER, MAX_NODE, MAX_PCOUNT};
    use crate::parity::{CLASSES, MIN_GROUP};
    use crate::parity::PARITY;

    /// A branch entry: klen(2) + cid(32) + agg(16) + a one-byte suffix, plus
    /// the 6-byte offset/key4 table slot.
    const MIN_BRANCH_ENTRY: usize = 6 + 50 + 1;
    /// A leaf Ref entry: klen(2) + vkind(1) + vlen(4) + a one-byte suffix +
    /// cid(32), plus the table slot.
    const MIN_LEAF_REF_ENTRY: usize = 6 + 7 + 1 + 32;

    const fn fits(n: usize) -> usize {
        (MAX_LOGICAL - HEADER) / n
    }
    const BRANCH_CHILDREN: usize = fits(MIN_BRANCH_ENTRY);
    const LEAF_REFS: usize = fits(MIN_LEAF_REF_ENTRY);
    /// One run, so one short tail.
    const BRANCH_GROUPS: usize = BRANCH_CHILDREN / MIN_GROUP + 1;
    /// One run per class, so one short tail per class.
    const LEAF_GROUPS: usize = LEAF_REFS / MIN_GROUP + CLASSES.len();

    /// A full leaf's parity ids at a minimum group and a parity per group: the
    /// ONE formula the asserts and the named test below share.
    const fn leaf_ids(min_group: usize, parity: usize) -> usize {
        (LEAF_REFS / min_group + CLASSES.len()) * parity
    }

    const _: () = assert!(MAX_LOGICAL + 4096 <= MAX_NODE);
    const _: () = assert!(BRANCH_GROUPS * PARITY <= MAX_PCOUNT);
    const _: () = assert!(LEAF_GROUPS * PARITY <= MAX_PCOUNT);
    const _: () = assert!(leaf_ids(MIN_GROUP, PARITY) == LEAF_GROUPS * PARITY);
    // The leaf is the binding case: a fifth class, or a smaller minimum group,
    // does not fit. Asserted so that is a fact rather than a comment.
    const _: () = assert!((LEAF_REFS / MIN_GROUP + CLASSES.len() + 1) * PARITY > MAX_PCOUNT);
    const _: () = assert!(leaf_ids(MIN_GROUP - 1, PARITY) > MAX_PCOUNT);

    /// sdk#321's epoch, by name: at [`PARITY`] = 8 a full leaf needs EXACTLY the
    /// reserve at [`MIN_GROUP`] = 21 (128 ids), and one fewer member per group
    /// (MIN 20) needs 136 -- the mutant the compile-time asserts above refuse.
    #[test]
    fn the_leaf_worst_case_fills_the_reserve_exactly_at_m8_min21() {
        println!(
            "  leaf refs {LEAF_REFS}, classes {}, MIN {MIN_GROUP}, m {PARITY}: {} ids of {MAX_PCOUNT}; at MIN {}: {}",
            CLASSES.len(),
            leaf_ids(MIN_GROUP, PARITY),
            MIN_GROUP - 1,
            leaf_ids(MIN_GROUP - 1, PARITY)
        );
        assert_eq!((PARITY, MIN_GROUP), (8, 21), "the epoch's numbers moved");
        assert_eq!(
            (PARITY, MIN_GROUP, crate::parity::MAX_GROUP),
            (8, 21, 36),
            "the node format's grouping moved: a format epoch, never a side effect"
        );
        assert_eq!(leaf_ids(MIN_GROUP, PARITY), MAX_PCOUNT, "the leaf worst case is not the reserve exactly");
        assert_eq!(leaf_ids(20, PARITY), 136, "MIN 20 would need 136 ids");
        assert!(leaf_ids(20, PARITY) > MAX_PCOUNT);
    }
}
