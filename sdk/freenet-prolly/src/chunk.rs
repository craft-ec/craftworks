//! Chunk ONE level: entries in, closed nodes out. The from-scratch build and
//! incremental edits both go through this, so they cannot disagree about where
//! a node ends.

use crate::boundary;
use crate::node::{Agg, BuildError, Node, NodeBuilder, Value};
use crate::{block_id, kind, Cid};

/// Decides whether a node closes after an entry: `(level, key, s_before, s_after)`.
pub type SplitRule = fn(u8, &[u8], usize, usize) -> bool;

/// What an entry holds, owned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Inline(Vec<u8>),
    Ref { cid: Cid, len: u32 },
    Child { cid: Cid, agg: Agg },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: Vec<u8>,
    pub body: Body,
}

impl Entry {
    /// Entry `i` of `node`.
    pub fn of(node: &Node<'_>, i: usize) -> Entry {
        let body = if node.is_leaf() {
            match node.value(i) {
                Value::Inline(b) => Body::Inline(b.to_vec()),
                Value::Ref { cid, len } => Body::Ref { cid, len },
            }
        } else {
            let (cid, agg) = node.child(i);
            Body::Child { cid, agg }
        };
        Entry {
            key: node.key(i),
            body,
        }
    }
}

/// The parity a node owes, accumulated as its members arrive.
///
/// **Members are recorded, not read.** A group's parity ids can come from three
/// places, and only the last needs the members' bytes at all:
///
/// - the node being replaced already had this exact group — same members, same
///   order, same class — so its ids are already right and are copied;
/// - it had a group differing in ONE position, so the new parity is the old
///   parity plus a correction that needs only the two versions of that one
///   member (`parity::update_group`);
/// - neither, so the group is coded from its members.
///
/// Parity is a pure function of the members, so all three give the same bytes.
/// What differs is what has to be READ — and on an ordinary edit most groups
/// of a rewritten node are untouched, so most of them read nothing.
///
/// What is held is bounded the same way as before: a group can still gain
/// members while it is the last of its run, so the CIDS of at most the last
/// closed group and the open one matter, and bytes are fetched per group at
/// the moment it is coded and dropped immediately after.
#[derive(Default)]
struct Run {
    class: usize,
    /// Four bytes each, so keeping all of them costs nothing and removes any
    /// need to re-derive the grouping.
    hashes: Vec<u32>,
    /// One per member, in key order. Cheap: the bytes are not here.
    cids: Vec<Cid>,
    /// The member states the caller supplied for entries it is writing, keyed
    /// by cid — a value being written is not in the store yet.
    encoded: usize,
    ids: Vec<Cid>,
}

/// Where a group's parity came from. Counted so a test can assert that an
/// ordinary edit REUSES rather than recodes — the saving is invisible to any
/// assertion about the bytes, which are identical either way.
#[cfg(any(test, feature = "testing"))]
pub mod source {
    use core::cell::Cell;
    thread_local! {
        static REUSED: Cell<usize> = const { Cell::new(0) };
        static DELTA: Cell<usize> = const { Cell::new(0) };
        static RECODED: Cell<usize> = const { Cell::new(0) };
        /// Parity blocks a rewrite coded and then did NOT report, because no
        /// node it kept lists them. See `filtered`.
        static FILTERED: Cell<usize> = const { Cell::new(0) };
    }
    pub fn reused() -> usize {
        REUSED.with(|n| n.get())
    }
    pub fn delta() -> usize {
        DELTA.with(|n| n.get())
    }
    pub fn recoded() -> usize {
        RECODED.with(|n| n.get())
    }
    /// Parity blocks coded and then withheld: a node was closed, its groups
    /// coded, and the node later dropped from the tree (a rewrite whose root
    /// collapses to a single-child branch). Reporting them would be PUTs for
    /// redundancy over a node nobody has.
    ///
    /// This is a TRIPWIRE, not a statistic. It is zero across the whole suite
    /// -- 143 tests, including the randomised edit sequences and the case that
    /// collapses the root -- so the filter that produces it has never been
    /// observed to remove anything. The test asserts the zero, so the day a
    /// case does reach it, the suite says so instead of the filter quietly
    /// mattering for the first time in production.
    pub fn filtered() -> usize {
        FILTERED.with(|n| n.get())
    }
    pub fn reset() {
        REUSED.with(|n| n.set(0));
        DELTA.with(|n| n.set(0));
        RECODED.with(|n| n.set(0));
        FILTERED.with(|n| n.set(0));
    }
    pub(super) fn tick_reused() {
        REUSED.with(|n| n.set(n.get() + 1));
    }
    pub(super) fn tick_delta() {
        DELTA.with(|n| n.set(n.get() + 1));
    }
    pub(super) fn tick_recoded() {
        RECODED.with(|n| n.set(n.get() + 1));
    }
    pub fn tick_filtered(n_blocks: usize) {
        FILTERED.with(|n| n.set(n.get() + n_blocks));
    }
}

impl Run {
    fn push(&mut self, h: u32, cid: Cid) {
        self.hashes.push(h);
        self.cids.push(cid);
    }

    /// The state of one member: `kind ‖ body`, from the store.
    fn state(
        blocks: &dyn crate::store::Blocks,
        class_is_leaf: bool,
        cid: &Cid,
    ) -> Result<Vec<u8>, BuildError> {
        let bytes = blocks.get(cid).ok_or(BuildError::MissingMember(*cid))?;
        let k = if class_is_leaf {
            kind::RAW
        } else {
            kind::TREE_NODE
        };
        let mut st = Vec::with_capacity(1 + bytes.len());
        st.push(k);
        st.extend_from_slice(bytes);
        Ok(st)
    }

    fn finish(
        &mut self,
        leaf: bool,
        blocks: &dyn crate::store::Blocks,
        old: Option<&crate::parity::GroupIndex>,
        coded: &mut Vec<(Cid, Vec<u8>)>,
    ) -> Result<Vec<Cid>, BuildError> {
        let sizes = crate::parity::group_sizes(&self.hashes);
        let mut at = 0usize;
        for size in sizes {
            let members = &self.cids[at..at + size];
            at += size;
            let ids = self.group_ids(leaf, blocks, old, members, coded)?;
            self.ids.extend_from_slice(&ids);
        }
        self.encoded = at;
        Ok(std::mem::take(&mut self.ids))
    }

    /// Hand a group's freshly coded parity blocks (`parity::PARITY` of them) to
    /// the caller and return their ids. Only a group that was actually CODED reports: a reused group's
    /// parity is already out on the network, and re-reporting it would have
    /// the engine pay PUTs for blocks it has already put.
    fn record(coded: &mut Vec<(Cid, Vec<u8>)>, parity: Vec<Vec<u8>>) -> [Cid; crate::parity::PARITY] {
        let ids: [Cid; crate::parity::PARITY] = std::array::from_fn(|i| block_id(kind::PARITY, &parity[i]));
        for (id, bytes) in ids.iter().zip(parity) {
            coded.push((*id, bytes));
        }
        ids
    }

    fn group_ids(
        &self,
        leaf: bool,
        blocks: &dyn crate::store::Blocks,
        old: Option<&crate::parity::GroupIndex>,
        members: &[Cid],
        coded: &mut Vec<(Cid, Vec<u8>)>,
    ) -> Result<[Cid; crate::parity::PARITY], BuildError> {
        if let Some(idx) = old {
            // Already had this exact group: its parity is a pure function of
            // these members, so it is already right.
            if let Some(ids) = idx.exact(self.class, members) {
                #[cfg(any(test, feature = "testing"))]
                source::tick_reused();
                return Ok(ids);
            }
            // One member changed in place: the correction needs only that
            // member's two versions and the PARITY old parity BLOCKS. If any of
            // them is not to hand, fall through and recode — never produce
            // parity that does not cover what it claims.
            if let Some((pos, was, old_ids)) = idx.one_off(self.class, members) {
                if let Some(parity) = self.delta(leaf, blocks, members, pos, was, old_ids)? {
                    #[cfg(any(test, feature = "testing"))]
                    source::tick_delta();
                    return Ok(Self::record(coded, parity));
                }
            }
        }
        #[cfg(any(test, feature = "testing"))]
        source::tick_recoded();
        let mut states = Vec::with_capacity(members.len());
        for c in members {
            states.push(Run::state(blocks, leaf, c)?);
        }
        let parity = crate::parity::encode_group(&states).map_err(|_| BuildError::NodeTooLarge)?;
        Ok(Self::record(coded, parity))
    }

    /// Correct a group's parity for a single member changing in place.
    ///
    /// `None` when the old parity blocks are not available: the caller recodes.
    fn delta(
        &self,
        leaf: bool,
        blocks: &dyn crate::store::Blocks,
        members: &[Cid],
        pos: usize,
        was: Cid,
        old_ids: [Cid; crate::parity::PARITY],
    ) -> Result<Option<Vec<Vec<u8>>>, BuildError> {
        let mut old_parity = Vec::with_capacity(crate::parity::PARITY);
        for id in &old_ids {
            match blocks.get(id) {
                Some(b) => old_parity.push(b.to_vec()),
                // A parity block that was never put, or has been dropped. The
                // fallback is a recode, never "no parity".
                None => return Ok(None),
            }
        }
        let Some(old_state) = blocks.get(&was).map(|b| {
            let mut st = Vec::with_capacity(1 + b.len());
            st.push(if leaf { kind::RAW } else { kind::TREE_NODE });
            st.extend_from_slice(b);
            st
        }) else {
            return Ok(None);
        };
        let new_state = Run::state(blocks, leaf, &members[pos])?;
        let parity = crate::parity::update_group(&old_parity, pos, &old_state, &new_state)
            .map_err(|_| BuildError::NodeTooLarge)?;
        Ok(Some(parity))
    }
}

/// A node that has been closed.
#[derive(Clone, Debug)]
pub struct Closed {
    pub min_key: Vec<u8>,
    pub cid: Cid,
    pub agg: Agg,
    pub bytes: Vec<u8>,
}

impl Closed {
    /// This node as an entry of the level above.
    pub fn as_child(&self) -> Entry {
        Entry {
            key: self.min_key.clone(),
            body: Body::Child {
                cid: self.cid,
                agg: self.agg,
            },
        }
    }
}

pub struct LevelChunker {
    level: u8,
    rule: SplitRule,
    open: NodeBuilder,
    /// The open node's parity. A branch has one run over its children; a leaf
    /// one per size class, since its members are grouped by class first.
    runs: Vec<Run>,
    /// The groups of every node this rewrite is REPLACING. Empty for a build
    /// from scratch, where there is nothing to reuse.
    old: crate::parity::GroupIndex,
    /// Parity blocks this chunker has CODED, with their bytes.
    ///
    /// A node lists its parity ids; nothing until now produced the bytes
    /// behind them, so the caller had no way to put what the node promised.
    /// These are kept apart from the closed nodes rather than pushed into the
    /// same `out`, because the order is the caller's to choose: data nodes,
    /// then the head, then parity (ARCHITECTURE §7). The library must not
    /// interleave them.
    coded: Vec<(Cid, Vec<u8>)>,
}

fn new_node(level: u8) -> NodeBuilder {
    if level == 0 {
        NodeBuilder::leaf()
    } else {
        NodeBuilder::branch(level)
    }
}

impl LevelChunker {
    pub fn new(level: u8, rule: SplitRule) -> Self {
        LevelChunker {
            level,
            rule,
            open: new_node(level),
            runs: Vec::new(),
            old: crate::parity::GroupIndex::default(),
            coded: Vec::new(),
        }
    }

    /// True when no entry is waiting in an open node: the chunker is in the
    /// state it has at the start of any node.
    pub fn is_clean(&self) -> bool {
        self.open.is_empty()
    }

    /// Add the next entry (keys strictly increasing; the caller has validated
    /// it). Nodes this closes are appended to `out`.
    /// Add an entry.
    ///
    /// `blocks` is where a parity member's bytes come from: every child of a
    /// branch, every referenced value of a leaf. An inline value is not a
    /// member of anything. A member whose bytes cannot be found is REFUSED —
    /// coding it as absent would produce a node whose parity does not protect
    /// what it claims to.
    pub fn push(
        &mut self,
        key: &[u8],
        body: &Body,
        blocks: &dyn crate::store::Blocks,
        out: &mut Vec<Closed>,
    ) -> Result<(), BuildError> {
        let cost = match body {
            Body::Inline(b) => NodeBuilder::leaf_cost(key, &Value::Inline(b)),
            Body::Ref { cid, len } => NodeBuilder::leaf_cost(
                key,
                &Value::Ref {
                    cid: *cid,
                    len: *len,
                },
            ),
            Body::Child { .. } => NodeBuilder::child_cost(key),
        };
        // The hard limit closes the node BEFORE the entry that would overflow it.
        if self.open.logical_len() + cost > boundary::MAX_LOGICAL {
            self.close(blocks, out)?;
        }
        // The member is RECORDED here, not read. Which of its bytes are needed
        // depends on whether its group turns out to be reusable, and that is
        // not known until the group closes.
        let member = match body {
            Body::Inline(_) => None,
            Body::Ref { cid, len } => Some((crate::parity::class_of(*len as usize), *cid)),
            Body::Child { cid, .. } => Some((0, *cid)),
        };
        let before = self.open.logical_len();
        match body {
            Body::Inline(b) => self.open.push(key, Value::Inline(b))?,
            Body::Ref { cid, len } => self.open.push(
                key,
                Value::Ref {
                    cid: *cid,
                    len: *len,
                },
            )?,
            Body::Child { cid, agg } => self.open.push_child(key, *cid, *agg)?,
        }
        if let Some((class, cid)) = member {
            let h = crate::parity::group_hash_parts(self.level, key, &[]);
            self.run(class).push(h, cid);
        }
        if (self.rule)(self.level, key, before, self.open.logical_len()) {
            self.close(blocks, out)?;
        }
        Ok(())
    }

    /// Close the open node, if it holds anything (end of the level).
    pub fn finish(
        &mut self,
        blocks: &dyn crate::store::Blocks,
        out: &mut Vec<Closed>,
    ) -> Result<(), BuildError> {
        if !self.open.is_empty() {
            self.close(blocks, out)?;
        }
        Ok(())
    }

    fn run(&mut self, class: usize) -> &mut Run {
        while self.runs.len() <= class {
            let c = self.runs.len();
            self.runs.push(Run {
                class: c,
                ..Run::default()
            });
        }
        &mut self.runs[class]
    }

    /// An old node this rewrite is replacing. Its groups are where untouched
    /// parity is copied from — and it is NOT cleared when a node closes,
    /// because boundaries move: a group that survives can land in a different
    /// node than it started in.
    pub fn replacing(&mut self, node: &Node<'_>) {
        self.old.add(node);
    }

    /// The parity blocks coded since the last call, and their ids.
    ///
    /// Only groups this chunker actually coded — one that was reused already
    /// has its parity out, and one that was corrected reports the corrected
    /// bytes. Taking them empties the list, so a caller draining as it goes
    /// never reports a block twice.
    pub fn take_coded(&mut self) -> Vec<(Cid, Vec<u8>)> {
        std::mem::take(&mut self.coded)
    }

    fn close(
        &mut self,
        blocks: &dyn crate::store::Blocks,
        out: &mut Vec<Closed>,
    ) -> Result<(), BuildError> {
        let mut done = std::mem::replace(&mut self.open, new_node(self.level));
        // Ids in the order the format requires: class ascending, group order
        // within a class.
        let leaf = self.level == 0;
        // Taken out so the runs can borrow it while `self` is borrowed
        // mutably; put back before returning, since later nodes of this same
        // rewrite reuse from it too.
        let old = std::mem::take(&mut self.old);
        let mut runs = std::mem::take(&mut self.runs);
        // Out for the same reason as `old`: the runs borrow it while `self` is
        // borrowed mutably.
        let mut coded = std::mem::take(&mut self.coded);
        let mut result = Ok(());
        for run in runs.iter_mut() {
            match run.finish(leaf, blocks, Some(&old), &mut coded) {
                Ok(ids) => {
                    for id in ids {
                        if let Err(e) = done.push_parity(id) {
                            result = Err(e);
                            break;
                        }
                    }
                }
                Err(e) => result = Err(e),
            }
            if result.is_err() {
                break;
            }
        }
        self.coded = coded;
        self.old = old;
        result?;
        let min_key = done.min_key().unwrap_or_default().to_vec();
        let agg = done.agg();
        let bytes = done.finish()?;
        out.push(Closed {
            min_key,
            cid: block_id(kind::TREE_NODE, &bytes),
            agg,
            bytes,
        });
        Ok(())
    }
}

/// The empty tree: one empty leaf.
pub fn empty_leaf() -> Closed {
    let bytes = NodeBuilder::leaf().finish().expect("an empty leaf encodes");
    Closed {
        min_key: Vec::new(),
        cid: block_id(kind::TREE_NODE, &bytes),
        agg: Agg::default(),
        bytes,
    }
}

// Every entry fits an empty node with room to spare, so the forced close in
// `push` always makes progress.
const _: () = assert!(
    crate::node::HEADER + 6 + 7 + crate::node::MAX_KEY + crate::node::MAX_INLINE
        <= boundary::MAX_LOGICAL
);
