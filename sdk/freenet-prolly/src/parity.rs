//! Which blocks form a parity group, and what a group's symbols are.
//!
//! A node's children are coded in groups; each group gets [`PARITY`] parity blocks
//! ([`crate::rs`]), and the node lists their ids. What this module decides is
//! the **membership** — and it has to decide it the same way for every reader,
//! because `pcount` is verified exactly and a disagreement about grouping is a
//! disagreement about whether a node is valid.
//!
//! **Groups are content-defined on the child's KEY, never on its cid.** A
//! child's cid changes on every write beneath it, so a cid-keyed boundary would
//! appear or disappear on roughly one write in eight per level and regroup two
//! neighbouring groups each time — less stable than counting, in exchange for
//! being more stable on the rare case. The tree's own chunking hashes the key
//! for the same reason.
//!
//! ```text
//! h = first 4 bytes (LE) of BLAKE3("PT01-pgroup" ‖ level ‖ key)
//! close after this member  iff  the group holds ≥ MIN_GROUP  and  h < 2³²/3
//! forced close             iff  the group holds MAX_GROUP
//! ```
//!
//! A final group shorter than `MIN_GROUP` folds into the previous one when the
//! two together are at most `MAX_GROUP`, and otherwise stands alone. **Only the
//! last group can ever be short**, because every earlier one closed on the rule
//! above, so there is at most one fold and it never cascades — which is what
//! makes the reserve arithmetic exact rather than approximate.
//!
//! On a LEAF the members are the referenced values, grouped **by size class
//! first** (§11's padding classes) and content-defined by key within a class.
//! Referenced values run from 1 KiB to 256 KiB with no size-tightness, so
//! grouping them by key alone would pad a mixed group to its largest and write
//! [`PARITY`] parity blocks of that size: for private data, where every value in a
//! class is already padded to the class, the waste is instead zero.
//!
//! Inline values are not members of anything: they live in the leaf, and the
//! leaf is a member of its parent's group. The reserve arithmetic depends on
//! that.

use crate::node::{Node, Value, MAX_VALUE};
use crate::rs;

/// Domain for the grouping hash. Distinct from `"PT01-split"` so a key cannot
/// mean one thing to the boundary rule and another to the grouping rule.
const DOMAIN: &[u8] = b"PT01-pgroup";

/// THE TREE'S PARITY PER GROUP (sdk#321, a format epoch): eight, so a group
/// survives any eight losses. A NODE-FORMAT number, owned HERE -- the codec
/// (`rs`) takes `m` per call and only bounds it (`rs::MAX_M`) -- and every count
/// of a node's parity ids derives from it.
pub const PARITY: usize = 8;
const _: () = assert!(PARITY <= rs::MAX_M);
/// A group closes on the hash only once it holds this many members. Twenty-one,
/// and the number is FORCED by the reserve at [`PARITY`] = 8 rather than chosen:
/// a full leaf of referenced values needs `(266 / MIN_GROUP + 4) × 8` parity
/// ids, which at 21 is exactly the reserve's 128 (4,096 B) and at 20 is 136
/// (`boundary::reserve` asserts both).
pub const MIN_GROUP: usize = 21;
/// A group closes here whatever the hash says. A NODE-FORMAT number of its own
/// (the architect's MIN 21 / MAX 36): the reserve depends on [`MIN_GROUP`] only,
/// MAX sets the repair width. The TREE's literal, never derived from the codec:
/// a codec that grows `rs::MAX_K` must not silently regroup every tree (a hidden
/// epoch); the codec only has to be able to code it (asserted below).
pub const MAX_GROUP: usize = 36;
const _: () = assert!(MAX_GROUP <= rs::MAX_K);
/// Mean group size ≈ 23: after the 21st member, one member in three closes it.
const CLOSE_THRESHOLD: u32 = (u32::MAX / 3) + 1;
/// The same value, exposed so a frozen vector can carry it: a constant only
/// the source knows is a constant nobody reviewing a diff can see move.
pub const CLOSE_THRESHOLD_FOR_VECTORS: u32 = CLOSE_THRESHOLD;

/// The size classes for referenced values, in bytes — §11's padding classes.
/// Every Ref value is 1 KiB + 1 .. [`MAX_VALUE`], so these four cover all of
/// them and there is no fifth class and no excluded tail.
pub const CLASSES: [usize; 4] = [4 * 1024, 16 * 1024, 64 * 1024, MAX_VALUE];

/// Which class a referenced value of `len` bytes belongs to.
pub fn class_of(len: usize) -> usize {
    CLASSES
        .iter()
        .position(|&c| len <= c)
        .unwrap_or(CLASSES.len() - 1)
}

/// Grouping hashes taken. `check_node` pays one per member, so what a REFUSED
/// node costs is a property in its own right — a node that is mis-cut or
/// oversized must be refused without buying the grouping. Thread-local: the
/// harness runs tests on many threads and a shared counter reports all of them.
#[cfg(any(test, feature = "testing"))]
pub mod work {
    use core::cell::Cell;
    thread_local! { static N: Cell<usize> = const { Cell::new(0) }; }
    pub fn hashes() -> usize {
        N.with(|n| n.get())
    }
    pub fn reset() {
        N.with(|n| n.set(0));
    }
    pub(super) fn tick() {
        N.with(|n| n.set(n.get() + 1));
    }
}

/// The grouping hash of one key, held as `prefix ‖ suffix` so a reader checking
/// a whole node never allocates a key.
pub fn group_hash_parts(level: u8, prefix: &[u8], suffix: &[u8]) -> u32 {
    #[cfg(any(test, feature = "testing"))]
    work::tick();
    let mut h = blake3::Hasher::new();
    h.update(DOMAIN);
    h.update(&[level]);
    h.update(prefix);
    h.update(suffix);
    let b = h.finalize();
    let b = b.as_bytes();
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Split a run of members, in key order, into groups.
///
/// `hashes` is the grouping hash of each member's key, in order. What comes
/// back is the group sizes, summing to `hashes.len()`.
pub fn group_sizes(hashes: &[u32]) -> Vec<usize> {
    if hashes.is_empty() {
        return Vec::new();
    }
    let mut sizes: Vec<usize> = Vec::new();
    let mut open = 0usize;
    for &h in hashes {
        open += 1;
        if open == MAX_GROUP || (open >= MIN_GROUP && h < CLOSE_THRESHOLD) {
            sizes.push(open);
            open = 0;
        }
    }
    if open > 0 {
        // Only the last group can be short, so this fold happens at most once
        // and cannot cascade.
        match sizes.last_mut() {
            Some(prev) if open < MIN_GROUP && *prev + open <= MAX_GROUP => *prev += open,
            _ => sizes.push(open),
        }
    }
    sizes
}

/// The groups of a node, as counts, in the order their parity ids appear.
///
/// A branch groups its children in key order. A leaf groups its REFERENCED
/// values by class ascending, then by key within the class. A leaf with no
/// referenced values has no groups, and so no parity.
pub fn group_sizes_of(node: &Node<'_>) -> Vec<usize> {
    // Derived from `group_members`, not computed a second way. Two enumerations
    // of the same grouping is the one way a writer and a checker can come to
    // disagree about whether a node is valid.
    group_members(node)
        .into_iter()
        .map(|(_, m)| m.len())
        .collect()
}

/// How many parity ids a node must carry: [`PARITY`] per group, and a pure function
/// of the entries. This is what `check_node` compares `pcount` against.
pub fn pcount_of(node: &Node<'_>) -> usize {
    PARITY * group_sizes_of(node).len()
}

/// One member's coded symbol: `len:u32 LE ‖ state`, and zeros for ever after.
///
/// `state` is the member's FULL block — `kind ‖ body` — so a rebuilt symbol IS
/// the block and verifies against its id by one hash, with nothing inferred
/// from context. The length prefix is what ends it: the parent stores a cid and
/// an aggregate, neither of which carries a length, so without it a repairer
/// could not know where the block stops.
///
/// There is no padding here. A symbol is these bytes followed by infinitely
/// many zeros, and parity is defined per byte index — which is what lets a
/// parity block be stored trimmed, and what stops one member's length from
/// being part of what the others contribute.
pub fn symbol(state: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + state.len());
    out.extend_from_slice(&(state.len() as u32).to_le_bytes());
    out.extend_from_slice(state);
    out
}

/// The [`PARITY`] parity symbols for one group of member states, trimmed.
pub fn encode_group(states: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, rs::RsError> {
    let data: Vec<Vec<u8>> = states.iter().map(|s| symbol(s)).collect();
    rs::encode(&data, PARITY)
}

/// Recover every member's state from any `k` of the `k + PARITY` blocks.
///
/// `have` holds the blocks as they are STORED: data symbols as `symbol`
/// produces them, parity trimmed. Each answer is the member's state, its length
/// taken from its own prefix.
/// `max_len` is the largest a member of THIS group may be — the caller knows
/// which kind it holds, and a rebuilt length is a stranger's number until it is
/// bounded. [`MAX_MEMBER_VALUE`] and [`MAX_MEMBER_NODE`] are the two.
pub fn repair_group(
    k: usize,
    have: &[Option<Vec<u8>>],
    max_len: usize,
) -> Result<Vec<Vec<u8>>, rs::RsError> {
    Ok(rs::repair(k, PARITY, have, max_len)?
        .into_iter()
        .map(|s| s[4..].to_vec())
        .collect())
}

/// The longest state a leaf's parity member can have: a `RAW` block holding a
/// value at the tree's cap, plus its kind byte.
pub const MAX_MEMBER_VALUE: usize = 1 + MAX_VALUE;
/// The longest state a branch's parity member can have: a node at `MAX_NODE`,
/// plus its kind byte.
pub const MAX_MEMBER_NODE: usize = 1 + crate::node::MAX_NODE;

/// A member changed: the new parity, from the OLD parity and the two symbols.
///
/// `parity' = trim(parity ⊕ C[·][c] · (symbol' ⊕ symbol))`, per byte index. The
/// code is linear, so a writer that holds the [`PARITY`] old parity blocks and both
/// versions of the one member it touched needs nothing else — no reads of the
/// other members, which is what keeps a write off the read path.
///
/// This is for a member CHANGING IN PLACE. A membership change — an insert, a
/// removal, a value crossing a size class — moves the columns, and columns are
/// positional, so the group is recoded from its members instead.
pub fn update_group(
    old_parity: &[Vec<u8>],
    column: usize,
    old_state: &[u8],
    new_state: &[u8],
) -> Result<Vec<Vec<u8>>, rs::RsError> {
    if old_parity.len() != PARITY {
        return Err(rs::RsError::Ragged);
    }
    if column >= MAX_GROUP {
        return Err(rs::RsError::GroupSize(column + 1));
    }
    let (old, new) = (symbol(old_state), symbol(new_state));
    let width = old_parity
        .iter()
        .map(|p| p.len())
        .chain([old.len(), new.len()])
        .max()
        .unwrap_or(0);
    let mut out = Vec::with_capacity(PARITY);
    for (r, p) in old_parity.iter().enumerate() {
        let f = rs::coeff(r, column);
        let mut v = vec![0u8; width];
        for (i, o) in v.iter_mut().enumerate() {
            let d = rs::byte_at(&new, i) ^ rs::byte_at(&old, i);
            *o = rs::byte_at(p, i) ^ rs::mul_pub(f, d);
        }
        rs::trim(&mut v);
        out.push(v);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hashes(n: usize, closing: &[usize]) -> Vec<u32> {
        (0..n)
            .map(|i| if closing.contains(&i) { 0 } else { u32::MAX })
            .collect()
    }

    /// The close rule: never before MIN_GROUP, always by MAX_GROUP.
    #[test]
    fn a_group_never_closes_early_and_never_runs_past_the_maximum() {
        // A run of three MAX_GROUPs: long enough that the tail's fold cannot
        // hide the first group, at any MIN and MAX.
        let n = 3 * MAX_GROUP;
        // A hash that always closes: every group is exactly MIN_GROUP until
        // the tail.
        let all_close: Vec<u32> = vec![0; n];
        let sizes = group_sizes(&all_close);
        assert!(sizes.iter().all(|&s| (MIN_GROUP..=MAX_GROUP).contains(&s)));
        assert_eq!(sizes.iter().sum::<usize>(), n);
        assert_eq!(sizes[0], MIN_GROUP, "a group closed before the minimum");

        // A hash that never closes: every group is exactly MAX_GROUP.
        let none: Vec<u32> = vec![u32::MAX; n];
        let sizes = group_sizes(&none);
        assert_eq!(sizes.iter().sum::<usize>(), n);
        assert!(sizes[..sizes.len() - 1].iter().all(|&s| s == MAX_GROUP));
    }

    /// Only the last group can be short, so the fold happens at most once and
    /// never cascades. That is what makes the reserve worst case exactly
    /// `⌊n / MIN_GROUP⌋ + 1` per run.
    #[test]
    fn at_most_one_group_is_short_and_the_fold_never_cascades() {
        for n in 1..=3 * MAX_GROUP {
            for closing in [
                vec![],
                (0..n).step_by(7).collect::<Vec<_>>(),
                (0..n).collect(),
            ] {
                let sizes = group_sizes(&hashes(n, &closing));
                assert_eq!(sizes.iter().sum::<usize>(), n, "n = {n}: members lost");
                assert!(
                    sizes.iter().all(|&s| s <= MAX_GROUP),
                    "n = {n}: a group over the maximum"
                );
                let short = sizes.iter().filter(|&&s| s < MIN_GROUP).count();
                assert!(short <= 1, "n = {n}: {short} short groups");
                if short == 1 {
                    assert!(
                        *sizes.last().expect("non-empty") < MIN_GROUP,
                        "n = {n}: a short group that is not the last"
                    );
                }
            }
        }
    }

    /// The fold: a short tail joins the previous group when it fits, and
    /// stands alone when it does not.
    #[test]
    fn a_short_tail_folds_only_when_it_fits() {
        // MIN_GROUP then 2: folds to one group of MIN_GROUP + 2.
        let mut h = vec![u32::MAX; MIN_GROUP + 2];
        h[MIN_GROUP - 1] = 0;
        assert_eq!(group_sizes(&h), vec![MIN_GROUP + 2]);
        // MAX_GROUP then 2: over MAX_GROUP together, so the tail stands alone.
        let mut h = vec![u32::MAX; MAX_GROUP + 2];
        h[MAX_GROUP - 1] = 0;
        assert_eq!(group_sizes(&h), vec![MAX_GROUP, 2]);
        // A run shorter than MIN_GROUP is one group, with nothing to fold into.
        assert_eq!(group_sizes(&[u32::MAX; 3]), vec![3]);
        assert_eq!(group_sizes(&[]), Vec::<usize>::new());
    }

    /// Every referenced value falls in exactly one class, and the boundaries
    /// are inclusive upper bounds.
    #[test]
    fn the_size_classes_cover_every_referenced_value() {
        assert_eq!(class_of(1025), 0);
        assert_eq!(class_of(4 * 1024), 0, "the boundary is inclusive");
        assert_eq!(class_of(4 * 1024 + 1), 1);
        assert_eq!(class_of(16 * 1024), 1);
        assert_eq!(class_of(64 * 1024), 2);
        assert_eq!(class_of(64 * 1024 + 1), 3);
        assert_eq!(class_of(MAX_VALUE), 3, "the largest value a Ref can hold");
    }

    /// A symbol carries its own length, so trimming is lossless: a group round
    /// trips through its parity at wildly unequal lengths.
    #[test]
    fn a_group_round_trips_through_its_parity_at_unequal_lengths() {
        let states: Vec<Vec<u8>> = vec![
            vec![0u8; 1],
            (0..100u8).collect(),
            vec![7u8; 4096],
            vec![9u8; 33],
        ];
        let parity = encode_group(&states).expect("a codeable group");
        let k = states.len();
        let all: Vec<Vec<u8>> = states
            .iter()
            .map(|s| symbol(s))
            .chain(parity.iter().cloned())
            .collect();
        // Lose the three largest data symbols — including the LONGEST, which
        // is the case that makes trimming non-trivial: nothing left says how
        // far the group reaches except the rebuilt length prefixes.
        let have: Vec<Option<Vec<u8>>> = (0..k + PARITY)
            .map(|j| (!(1..=3).contains(&j)).then(|| all[j].clone()))
            .collect();
        assert_eq!(
            repair_group(k, &have, MAX_MEMBER_VALUE).expect("repairable"),
            states
        );
    }

    /// The delta update equals coding the group from scratch — in place, and
    /// across the length boundary in both directions, which is where a rule
    /// that read a parity block's length as the width would break.
    #[test]
    fn an_incremental_update_equals_coding_the_group_again() {
        let base: Vec<Vec<u8>> = (0..5).map(|i| vec![i as u8 + 1; 40 + i * 7]).collect();
        let old_parity = encode_group(&base).expect("codeable");
        for (what, column, new_state) in [
            ("in place, same length", 2, vec![0xaa; 54]),
            ("grows past the longest", 0, vec![0xbb; 5_000]),
            ("shrinks below every other", 4, vec![0xcc; 1]),
            ("becomes empty", 3, Vec::new()),
        ] {
            let mut want_states = base.clone();
            want_states[column] = new_state.clone();
            let want = encode_group(&want_states).expect("codeable");
            let got =
                update_group(&old_parity, column, &base[column], &new_state).expect("updatable");
            assert_eq!(got, want, "{what}: delta differs from a fresh coding");
        }
    }

    /// Members that are all empty code to empty parity, and it is legal.
    #[test]
    fn all_empty_members_give_empty_parity() {
        // An empty STATE still has a non-zero length prefix of zero... which is
        // four zero bytes, so the symbol is entirely zero and so is the parity.
        let states: Vec<Vec<u8>> = vec![Vec::new(); 4];
        let parity = encode_group(&states).expect("codeable");
        assert!(parity.iter().all(|p| p.is_empty()), "empty parity expected");
        // And it repairs: every index reads zero, the prefixes say zero, and
        // the members come back empty.
        let all: Vec<Option<Vec<u8>>> = (0..4 + PARITY).map(|_| Some(Vec::new())).collect();
        assert_eq!(
            repair_group(4, &all, MAX_MEMBER_VALUE).expect("repairable"),
            states
        );
    }
}

#[cfg(test)]
mod repair_cases {
    use super::*;

    fn blocks(states: &[Vec<u8>]) -> (usize, Vec<Vec<u8>>) {
        let parity = encode_group(states).expect("codeable");
        let all: Vec<Vec<u8>> = states
            .iter()
            .map(|s| symbol(s))
            .chain(parity.iter().cloned())
            .collect();
        (states.len(), all)
    }

    fn lose(k: usize, all: &[Vec<u8>], gone: &[usize]) -> Vec<Option<Vec<u8>>> {
        (0..k + PARITY)
            .map(|j| (!gone.contains(&j)).then(|| all[j].clone()))
            .collect()
    }

    /// The cases trimming makes non-trivial, each named because each is a
    /// different reason the width could be unrecoverable.
    #[test]
    fn the_cases_that_trimming_makes_hard_all_repair() {
        // 1. The LONGEST member is the one lost: nothing left says how far the
        //    group reaches except the rebuilt length prefix.
        let states: Vec<Vec<u8>> = vec![vec![1u8; 10], vec![2u8; 20], vec![3u8; 500]];
        let (k, all) = blocks(&states);
        assert_eq!(
            repair_group(k, &lose(k, &all, &[2]), MAX_MEMBER_VALUE).unwrap(),
            states
        );

        // 2. BOTH of two equal-longest members lost, which is the case where
        //    the parity tail can cancel while the data is not zero.
        let states: Vec<Vec<u8>> = vec![vec![1u8; 8], vec![5u8; 300], vec![9u8; 300]];
        let (k, all) = blocks(&states);
        assert_eq!(
            repair_group(k, &lose(k, &all, &[1, 2]), MAX_MEMBER_VALUE).unwrap(),
            states
        );

        // 3. A member whose parity tail CANCELLED: two equal-longest members
        //    chosen so the trimmed parity stops short of the data. Constructed,
        //    not hoped for — the assertion below is what says it happened.
        let a = vec![0xa5u8; 64];
        let scale = rs::mul_pub(rs::coeff(0, 1), rs::div_pub(1, rs::coeff(0, 2)));
        let b: Vec<u8> = a.iter().map(|&x| rs::mul_pub(x, scale)).collect();
        let states = vec![vec![7u8; 4], a.clone(), b];
        let (k, all) = blocks(&states);
        let parity = &all[k];
        assert!(
            parity.len() < 4 + states[1].len(),
            "the fixture must really cancel: parity is {} and the data reaches {}",
            parity.len(),
            4 + states[1].len()
        );
        assert_eq!(
            repair_group(k, &lose(k, &all, &[1]), MAX_MEMBER_VALUE).unwrap(),
            states
        );

        // 4. A PARITY block lost along with two data blocks.
        let states: Vec<Vec<u8>> = (0..6).map(|i| vec![i as u8 + 1; 30 + i * 11]).collect();
        let (k, all) = blocks(&states);
        assert_eq!(
            repair_group(k, &lose(k, &all, &[0, 3, k + 1]), MAX_MEMBER_VALUE).unwrap(),
            states
        );

        // 5. All-empty members: every block is empty, and it still repairs.
        let states: Vec<Vec<u8>> = vec![Vec::new(); 5];
        let (k, all) = blocks(&states);
        assert!(
            all.iter().skip(k).all(|p| p.is_empty()),
            "parity must be empty"
        );
        assert_eq!(
            repair_group(k, &lose(k, &all, &[0, 1, 2]), MAX_MEMBER_VALUE).unwrap(),
            states
        );
    }
}

#[cfg(test)]
mod hostile_repair {
    use super::*;

    /// **What a hostile group costs a repairer.** Parity ids are not checkable
    /// by a host, so a writer can list ids of blocks whose bytes it chose; a
    /// keeper repairing that tree solves whatever the rebuilt prefixes claim.
    /// Unbounded, a prefix of 4 GiB is four billion byte columns before the
    /// caller ever gets a block to hash — a stall, from a tree that validated.
    ///
    /// Counted, because the verdict is "refused" either way.
    #[test]
    fn a_rebuilt_length_past_its_kinds_ceiling_is_refused_after_the_prefixes() {
        // A group whose symbols say their members are 4 GiB long. Nothing
        // here is a real block; that is the point — the ids were a stranger's.
        let huge = u32::MAX as usize;
        let states: Vec<Vec<u8>> = (0..4)
            .map(|i| {
                let mut v = (huge as u32).to_le_bytes().to_vec();
                v.push(i as u8);
                v
            })
            .collect();
        let parity = rs::encode(&states, PARITY).expect("codeable");
        let all: Vec<Vec<u8>> = states.iter().cloned().chain(parity).collect();
        let have: Vec<Option<Vec<u8>>> = (0..4 + PARITY)
            .map(|j| (j >= PARITY).then(|| all[j].clone()))
            .collect();

        rs::work::reset();
        assert_eq!(
            rs::repair(4, PARITY, &have, MAX_MEMBER_VALUE),
            Err(rs::RsError::MemberTooLong(huge))
        );
        assert_eq!(
            rs::work::columns(),
            4,
            "a hostile length was solved past its prefix"
        );

        // The control: an honest member at the ceiling still repairs, so the
        // bound is a bound and not a refusal of everything large.
        let big = vec![0xabu8; MAX_MEMBER_VALUE];
        let small = vec![1u8, 2, 3];
        let states = vec![big.clone(), small.clone()];
        let parity = encode_group(&states).expect("codeable");
        let all: Vec<Vec<u8>> = states
            .iter()
            .map(|s| symbol(s))
            .chain(parity.iter().cloned())
            .collect();
        let have: Vec<Option<Vec<u8>>> = (0..2 + PARITY)
            .map(|j| (j != 0).then(|| all[j].clone()))
            .collect();
        rs::work::reset();
        assert_eq!(
            repair_group(2, &have, MAX_MEMBER_VALUE).expect("an honest member at the cap"),
            states
        );
        assert!(
            rs::work::columns() > MAX_MEMBER_VALUE,
            "the control must really solve the whole member"
        );
    }
}

#[cfg(test)]
mod frozen_constants {
    use super::*;

    /// The close comparison is `<`, at exactly `2³²/3 + 1`.
    ///
    /// `group_sizes` is otherwise exercised with synthetic hashes of `0` and
    /// `u32::MAX`, which cannot tell `<` from `<=` at the threshold. Searching
    /// for a real key whose grouping hash lands exactly on it is a 2³² search,
    /// so the comparison is pinned here with injected values instead — and the
    /// threshold itself is on the vectors' const line, so a change to either
    /// shows up in two places.
    #[test]
    fn the_close_threshold_comparison_is_pinned_at_its_exact_value() {
        // A run long enough that the FOLD cannot hide the difference: with a
        // short tail, a group of MIN and a tail of 1 merge back and both
        // answers look alike. MIN + MAX members give MIN+MAX against MAX+MIN.
        let run = |at_min: u32| {
            let mut h = vec![u32::MAX; MIN_GROUP - 1];
            h.push(at_min);
            h.extend(std::iter::repeat_n(u32::MAX, MAX_GROUP));
            group_sizes(&h)
        };
        assert_eq!(
            run(CLOSE_THRESHOLD - 1),
            vec![MIN_GROUP, MAX_GROUP],
            "a hash one below the threshold must close the group"
        );
        assert_eq!(
            run(CLOSE_THRESHOLD),
            vec![MAX_GROUP, MIN_GROUP],
            "a hash AT the threshold must not: the comparison is `<`, not `<=`"
        );
    }

    /// The class boundaries are inclusive upper bounds, pinned at each edge.
    #[test]
    fn the_class_edges_are_where_the_rule_says() {
        for (i, &c) in CLASSES.iter().enumerate() {
            assert_eq!(class_of(c), i, "{c} must be the top of class {i}");
            if i + 1 < CLASSES.len() {
                assert_eq!(
                    class_of(c + 1),
                    i + 1,
                    "{} must open class {}",
                    c + 1,
                    i + 1
                );
            }
        }
        assert_eq!(class_of(4096), 0);
        assert_eq!(class_of(4097), 1);
        assert_eq!(class_of(MAX_VALUE), CLASSES.len() - 1);
    }
}

/// The parity a node already has, indexed by the members it covers.
///
/// A rewrite replaces a node, but most of its groups are untouched: the same
/// members, in the same order, in the same class. Their parity is a pure
/// function of those members, so it is already correct and already stored —
/// recomputing it costs reads of every member for a result that cannot differ.
///
/// This is what lets a writer say "I have seen this exact group before" and
/// copy the three ids instead. It is built from a node the rebuild is
/// REPLACING, so nothing here is a claim: the ids came out of a node that was
/// validated when it was stored.
#[derive(Default)]
pub struct GroupIndex {
    /// `(class, member cids) → the group's [`PARITY`] parity ids`, in the order the
    /// node listed them.
    groups: Vec<(usize, Vec<crate::Cid>, [crate::Cid; PARITY])>,
}

impl GroupIndex {
    /// Read a node's groups. A node whose parity count disagrees with its
    /// entries contributes nothing — it should never have been stored, and a
    /// writer must not build on it.
    pub fn of(node: &Node<'_>) -> GroupIndex {
        let mut out = GroupIndex::default();
        out.add(node);
        out
    }

    /// Add another node's groups.
    ///
    /// A rewrite usually spans several old nodes and produces several new ones,
    /// with the boundaries moving between them — so a group that survives may
    /// well end up in a different node than it started in. Parity is a pure
    /// function of the members and the class, and position is not part of it,
    /// so a group found in ANY node this rewrite is replacing is the same group.
    pub fn add(&mut self, node: &Node<'_>) {
        let out = self;
        let ids: Vec<crate::Cid> = node.parity().collect();
        if ids.len() != pcount_of(node) {
            return;
        }
        let mut at = 0usize;
        for (class, members) in group_members(node) {
            let group: [crate::Cid; PARITY] = std::array::from_fn(|i| ids[at + i]);
            at += PARITY;
            out.groups.push((class, members, group));
        }
    }

    /// The [`PARITY`] ids for exactly this group, if the node already had it.
    pub fn exact(&self, class: usize, members: &[crate::Cid]) -> Option<[crate::Cid; PARITY]> {
        self.groups
            .iter()
            .find(|(c, m, _)| *c == class && m == members)
            .map(|(_, _, ids)| *ids)
    }

    /// A group of the same class and the same length differing in exactly ONE
    /// position: the position, the cid that used to be there, and the old
    /// parity ids.
    ///
    /// This is the shape [`update_group`] can correct, and the reason it is
    /// worth finding: the correction reads two member blocks, where a recode
    /// reads all of them. A group differing in two positions is not reported —
    /// the saving is gone by then, and the caller would have to hold two old
    /// states to use it.
    ///
    /// The first match wins. Any group answering the description gives the same
    /// answer, because the ids are a function of the members.
    pub fn one_off(
        &self,
        class: usize,
        members: &[crate::Cid],
    ) -> Option<(usize, crate::Cid, [crate::Cid; PARITY])> {
        self.groups.iter().find_map(|(c, m, ids)| {
            if *c != class || m.len() != members.len() {
                return None;
            }
            let mut differ = m
                .iter()
                .zip(members)
                .enumerate()
                .filter(|(_, (a, b))| a != b);
            let (pos, (was, _)) = differ.next()?;
            // Exactly one: a second difference and this is not a delta.
            if differ.next().is_some() {
                return None;
            }
            Some((pos, *was, *ids))
        })
    }
}

/// A node's groups as `(class, member cids)`, in the order their parity ids
/// appear. The member cids are what the group is OVER: children for a branch,
/// referenced values for a leaf.
pub fn group_members(node: &Node<'_>) -> Vec<(usize, Vec<crate::Cid>)> {
    let level = node.level();
    let prefix = node.prefix();
    let mut runs: Vec<(usize, Vec<u32>, Vec<crate::Cid>)> = Vec::new();
    if level > 0 {
        let mut h = Vec::with_capacity(node.len());
        let mut c = Vec::with_capacity(node.len());
        for i in 0..node.len() {
            h.push(group_hash_parts(level, prefix, node.suffix(i)));
            c.push(node.child(i).0);
        }
        runs.push((0, h, c));
    } else {
        for class in 0..CLASSES.len() {
            let mut h = Vec::new();
            let mut c = Vec::new();
            for i in 0..node.len() {
                if let Value::Ref { cid, len } = node.value(i) {
                    if class_of(len as usize) == class {
                        h.push(group_hash_parts(level, prefix, node.suffix(i)));
                        c.push(cid);
                    }
                }
            }
            if !h.is_empty() {
                runs.push((class, h, c));
            }
        }
    }
    let mut out = Vec::new();
    for (class, hashes, cids) in runs {
        let mut at = 0usize;
        for size in group_sizes(&hashes) {
            out.push((class, cids[at..at + size].to_vec()));
            at += size;
        }
    }
    out
}

/// A node's parity BLOCKS, computed from its members.
///
/// The node lists ids; this is what produces the bytes behind them. A writer
/// puts these after the commit (§9: parity ids may be listed before the parity
/// blocks exist), a keeper computes them to repair, and a delta update needs
/// the old ones — so it is the same function for all three rather than three
/// spellings of it.
///
/// Returns `None` if any member is not readable: parity over a member whose
/// bytes nobody has is parity over nothing.
pub fn blocks_of(
    node: &Node<'_>,
    blocks: &dyn crate::store::Blocks,
) -> Option<Vec<(crate::Cid, Vec<u8>)>> {
    let leaf = node.level() == 0;
    let kind = if leaf {
        crate::kind::RAW
    } else {
        crate::kind::TREE_NODE
    };
    let mut out = Vec::new();
    for (_, members) in group_members(node) {
        let mut states = Vec::with_capacity(members.len());
        for c in &members {
            let b = blocks.get(c)?;
            let mut st = Vec::with_capacity(1 + b.len());
            st.push(kind);
            st.extend_from_slice(b);
            states.push(st);
        }
        for p in encode_group(&states).ok()? {
            out.push((crate::block_id(crate::kind::PARITY, &p), p));
        }
    }
    Some(out)
}
