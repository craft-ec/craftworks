//! Where blocks come from.
//!
//! Tree operations are pure functions of a root and a [`Blocks`] source. They
//! never wait: an operation that reaches a block the source does not hold stops
//! with [`TreeError::Need`] naming it, the caller fetches it, and runs the same
//! operation again. That fits a caller that works in bounded rounds and cannot
//! block — and re-running is cheap, because the upper levels are the ones
//! already held.

use crate::node::{Node, NodeError};
use crate::Cid;
use std::collections::HashMap;

/// A source of blocks by id. An implementation MUST only return a body whose
/// [`block_id`](crate::block_id) for its kind is `cid` (hash-keyed storage gives
/// this for free). `get` lends bytes that live as long as the source, so a
/// source is a map of blocks already in memory — not a lazy fetcher: what is
/// missing is reported as [`ReadError::Need`], fetched by the caller, added to
/// the map, and the operation is run again.
pub trait Blocks {
    fn get(&self, cid: &Cid) -> Option<&[u8]>;
}

/// So a `&B` is a block source wherever `B` is.
///
/// Without it, anything that WRAPS a block source — the probed source in the
/// tests, a caching layer, a counting one — has to own it, which means cloning
/// a store to hand it to two wrappers. The trait takes `&self` already, so
/// this costs nothing and removes a copy.
impl<B: Blocks + ?Sized> Blocks for &B {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        (**self).get(cid)
    }
}

/// A block source that can also be written to.
///
/// [`apply_into`](crate::apply::apply_into) uses it so that no caller has to
/// write the loop that feeds emitted blocks back into its own store — getting
/// that loop wrong is silent until a read, which makes it exactly the kind of
/// thing a library should do once.
pub trait BlocksMut: Blocks {
    fn insert_block(&mut self, cid: Cid, bytes: &[u8]);
}

/// Fresh blocks first, a base store second.
///
/// A rebuild needs its own output back: a branch's parity members are its
/// CHILDREN, and when a branch closes those children were created moments ago
/// and are not in the caller's store yet — `apply` emits them to a sink and the
/// caller inserts them afterwards. Reading only the store would miss exactly
/// the members the rebuild just made.
///
/// So writes look here first and fall through. `build` with no base store uses
/// an overlay over nothing, where a `Ref` whose bytes were never handed over is
/// a miss — and a miss is refused, never quietly coded as absent.
pub struct Overlay<'a, B: Blocks> {
    fresh: std::collections::BTreeMap<Cid, Vec<u8>>,
    base: Option<&'a B>,
}

impl<'a, B: Blocks> Overlay<'a, B> {
    pub fn new(base: Option<&'a B>) -> Self {
        Overlay {
            fresh: std::collections::BTreeMap::new(),
            base,
        }
    }

    /// Remember a block this call produced or was handed.
    pub fn put(&mut self, cid: Cid, bytes: &[u8]) {
        self.fresh.insert(cid, bytes.to_vec());
    }

    pub fn holds(&self, cid: &Cid) -> bool {
        self.fresh.contains_key(cid)
    }
}

impl<B: Blocks> Blocks for Overlay<'_, B> {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        self.fresh
            .get(cid)
            .map(|v| v.as_slice())
            .or_else(|| self.base.and_then(|b| b.get(cid)))
    }
}

/// Blocks held in memory.
#[derive(Default, Clone)]
pub struct MemBlocks(pub HashMap<Cid, Vec<u8>>);

impl MemBlocks {
    pub fn insert(&mut self, cid: Cid, bytes: &[u8]) {
        self.0.insert(cid, bytes.to_vec());
    }
}

impl BlocksMut for MemBlocks {
    fn insert_block(&mut self, cid: Cid, bytes: &[u8]) {
        self.insert(cid, bytes);
    }
}

impl Blocks for MemBlocks {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        self.0.get(cid).map(Vec::as_slice)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// These blocks are required and not held. Fetch them and run again.
    Need(Vec<Cid>),
    /// A block does not parse as a node.
    Corrupt(Cid, NodeError),
    /// A child is not what its parent says: wrong level, first key or aggregate.
    Mismatch(Cid),
}

/// Load and parse the node `cid`, UNCHECKED — for a root, or a block looked at
/// on its own. A child reached from a parent is opened with [`Held::open`],
/// which is what checks it; a `Node` from here is not bound to any parent.
pub fn load<'a>(blocks: &'a impl Blocks, cid: &Cid) -> Result<Node<'a>, ReadError> {
    let bytes = blocks.get(cid).ok_or_else(|| ReadError::Need(vec![*cid]))?;
    Node::parse(bytes).map_err(|e| ReadError::Corrupt(*cid, e))
}

/// A node reached from a trusted root, carrying the bound its keys must stay
/// under (freenet-prolly#52).
///
/// THE ONLY WAY TO HOLD A CHILD. A parent records each child's FIRST key and
/// nothing about its last, so a writer can put `z` in the leaf under `a` and `m`
/// in the next one: one root, and a range read says `z` is present while a
/// point read — which descends to the child under `m` — says it is absent.
/// Equivocation without a fork. The fix is that a child's keys END before
/// whatever follows it, and that bound is INHERITED: a leaf that is the last
/// child of its parent is bounded by the parent's own successor, which only the
/// path from the root knows. (A next-sibling-only check was tried: it refused
/// the adjacent overlap, kept the suite green, and still accepted the same
/// overlap one level up. `tests/overlapping_spans.rs` keeps it as the control.)
///
/// So the bound travels WITH the node rather than as a parameter: a parameter
/// was computed in two places and enforced in none, and the next call site
/// would forget it again. The fields are private, [`Held::root`] is the only
/// way in and [`Held::open`] the only way down, so a child that skipped the
/// checks cannot exist:
///
/// ```compile_fail,E0451
/// # use freenet_prolly::{node::Node, store::Held};
/// fn forge<'a>(node: Node<'a>) -> Held<'a> { Held { node, upper: None } }
/// ```
/// and the unchecked route it replaced is gone:
/// ```compile_fail,E0432
/// use freenet_prolly::store::load_child;
/// ```
/// The CONTROL for both — the sanctioned route compiles, so the two above fail
/// for the reason they name and not a typo:
/// ```
/// # use freenet_prolly::{store::{Held, MemBlocks, ReadError}, Cid};
/// fn way_in(b: &MemBlocks, root: &Cid) -> Result<Option<Vec<u8>>, ReadError> {
///     let h = Held::root(b, root)?;
///     Ok(if h.is_leaf() { None } else { h.open(b, 0)?.upper().map(<[u8]>::to_vec) })
/// }
/// ```
pub struct Held<'a> {
    node: Node<'a>,
    /// The smallest key after this node's whole subtree. `None` at the right
    /// edge of the tree — nothing follows a root.
    upper: Option<Vec<u8>>,
}

impl<'a> Held<'a> {
    /// The only way in. Nothing follows a whole tree, so a root is unbounded.
    pub fn root(blocks: &'a impl Blocks, cid: &Cid) -> Result<Held<'a>, ReadError> {
        Ok(Held {
            node: load(blocks, cid)?,
            upper: None,
        })
    }

    /// The only way down: load child `i` and check it against what this node
    /// records — its level, its first key, its aggregate — and that its keys
    /// END before the bound it inherits.
    ///
    /// One comparison suffices for the span: `Node::parse` refuses unsorted
    /// keys, so a last key under the bound puts every key under it, and a
    /// branch's own children are held to the same bound when THEY are opened.
    pub fn open(&self, blocks: &'a impl Blocks, i: usize) -> Result<Held<'a>, ReadError> {
        let (cid, agg) = self.node.child(i);
        let child = load(blocks, &cid)?;
        let upper = self.child_upper(i);
        let ok = crate::node::one_level_below(self.node.level(), child.level())
            && !child.is_empty()
            && child.agg() == agg
            && child.key(0) == self.node.key(i)
            && match &upper {
                Some(end) => child.key(child.len() - 1) < *end,
                None => true,
            };
        if ok {
            Ok(Held { node: child, upper })
        } else {
            Err(ReadError::Mismatch(cid))
        }
    }

    /// The smallest key after this node's subtree; `None` at the right edge.
    pub fn upper(&self) -> Option<&[u8]> {
        self.upper.as_deref()
    }

    /// The bound child `i` will be held to, without loading it: its next
    /// sibling's key, else this node's own bound.
    pub fn child_upper(&self, i: usize) -> Option<Vec<u8>> {
        if i + 1 < self.node.len() {
            Some(self.node.key(i + 1))
        } else {
            self.upper.clone()
        }
    }

    pub fn node(&self) -> &Node<'a> {
        &self.node
    }
}

/// Read access to the node. Reading is free; only CONSTRUCTING a `Held` is
/// guarded, and that is what `Deref` cannot do.
impl<'a> core::ops::Deref for Held<'a> {
    type Target = Node<'a>;
    fn deref(&self) -> &Node<'a> {
        &self.node
    }
}
