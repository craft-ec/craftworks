//! Build a whole tree from entries in key order, one pass, bottom-up.
//!
//! Every level is chunked by the same rule ([`crate::boundary`]); a node that
//! closes becomes one child entry of the level above. The root is the single
//! node of the lowest level that produces exactly one node. An empty tree is one
//! empty leaf.

use crate::boundary;
use crate::chunk::{empty_leaf, Body, Closed, LevelChunker};
use crate::node::{BuildError, NodeBuilder, Value};
use crate::Cid;

pub use crate::chunk::SplitRule;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeError {
    Node(BuildError),
    /// More levels than a `u8` can number.
    TooDeep,
    /// An earlier call failed after nodes had been closed; the builder's output
    /// can no longer be the canonical tree, so it refuses to go on.
    Poisoned,
}

impl From<BuildError> for TreeError {
    fn from(e: BuildError) -> Self {
        TreeError::Node(e)
    }
}

struct Level {
    chunker: LevelChunker,
    /// The first closed node, held back until a second one proves this level
    /// is not the root.
    first: Option<Closed>,
    closed: usize,
}

/// Parity blocks with their ids: what a writer must PUT so the redundancy its
/// nodes promise actually exists.
pub type ParityBlocks = Vec<(Cid, Vec<u8>)>;

/// Streaming tree builder. Closed nodes are handed to `sink` as `(cid, bytes)`.
pub struct TreeBuilder<F: FnMut(Cid, &[u8])> {
    /// Every block this build has produced or been handed, so a parity member
    /// can be read back the moment its parent needs it. A `Ref` whose bytes the
    /// builder never saw is a miss, and a miss is refused.
    seen: crate::store::Overlay<'static, crate::store::MemBlocks>,
    levels: Vec<Level>,
    /// Last key pushed: order must hold across node boundaries too.
    last: Option<Vec<u8>>,
    poisoned: bool,
    rule: SplitRule,
    sink: F,
}

impl<F: FnMut(Cid, &[u8])> TreeBuilder<F> {
    pub fn new(sink: F) -> Self {
        Self::with_rule(boundary::splits_after, sink)
    }

    /// Hand the builder a value block it did not write, so a `Ref` naming it
    /// can be coded. Without this a caller that stored its own values would be
    /// refused — correctly, since parity over a member whose bytes nobody has
    /// is parity over nothing.
    pub fn see(&mut self, cid: Cid, bytes: &[u8]) {
        self.seen.put(cid, bytes);
    }

    /// A builder with a different split rule. The format uses
    /// [`boundary::splits_after`]; anything else exists for comparison in tests.
    pub fn with_rule(rule: SplitRule, sink: F) -> Self {
        TreeBuilder {
            seen: crate::store::Overlay::new(None),
            levels: Vec::new(),
            last: None,
            poisoned: false,
            rule,
            sink,
        }
    }

    /// Add the next entry. Keys must be strictly increasing.
    ///
    /// A refused entry (bad order, key too long, wrong value encoding) leaves the
    /// builder exactly as it was — everything that can refuse an entry is checked
    /// before anything changes, so what a caller tried first never shows in the
    /// root. A failure past that point poisons the builder.
    pub fn push(&mut self, key: &[u8], value: Value<'_>) -> Result<(), TreeError> {
        if self.poisoned {
            return Err(TreeError::Poisoned);
        }
        if self.last.as_deref().is_some_and(|p| p >= key) {
            return Err(BuildError::NotSorted.into());
        }
        NodeBuilder::check_leaf(key, &value)?;
        let body = match value {
            Value::Inline(b) => Body::Inline(b.to_vec()),
            Value::Ref { cid, len } => Body::Ref { cid, len },
        };
        // Defensive: nothing below can fail for a validated entry (TooDeep and
        // aggregate overflow are out of reach), but if it ever did, nodes may
        // already have been emitted.
        if let Err(e) = self.add(0, key, &body) {
            self.poisoned = true;
            return Err(e);
        }
        self.last = Some(key.to_vec());
        Ok(())
    }

    fn add(&mut self, l: usize, key: &[u8], body: &Body) -> Result<(), TreeError> {
        if l == self.levels.len() {
            let level = u8::try_from(l).map_err(|_| TreeError::TooDeep)?;
            self.levels.push(Level {
                chunker: LevelChunker::new(level, self.rule),
                first: None,
                closed: 0,
            });
        }
        let mut done = Vec::new();
        self.levels[l]
            .chunker
            .push(key, body, &self.seen, &mut done)?;
        self.closed(l, done)
    }

    /// Pass the nodes level `l` just closed to the sink and to the level above.
    fn closed(&mut self, l: usize, done: Vec<Closed>) -> Result<(), TreeError> {
        for node in done {
            // Readable before the level above asks for it as a member.
            self.seen.put(node.cid, &node.bytes);
            (self.sink)(node.cid, &node.bytes);
            let lv = &mut self.levels[l];
            lv.closed += 1;
            if lv.closed == 1 {
                lv.first = Some(node);
                continue;
            }
            if let Some(first) = lv.first.take() {
                let e = first.as_child();
                self.add(l + 1, &e.key, &e.body)?;
            }
            let e = node.as_child();
            self.add(l + 1, &e.key, &e.body)?;
        }
        Ok(())
    }

    /// Close every level and return the root's cid.
    ///
    /// The parity blocks this build coded are DROPPED. That is right for a
    /// caller building a tree it is not going to store — a fixture, an oracle,
    /// a comparison — and wrong for one that is: the nodes it just took name
    /// parity ids whose bytes now exist nowhere, so every group of the new
    /// tree has no redundancy until someone codes it again. A caller that
    /// stores the tree wants [`TreeBuilder::finish_with_parity`].
    pub fn finish(self) -> Result<Cid, TreeError> {
        self.finish_with_parity().map(|(root, _)| root)
    }

    /// Close every level, returning the root's cid and the parity blocks the
    /// build coded.
    ///
    /// Separate from the sink, and returned rather than streamed, because the
    /// order is the caller's decision: data nodes, then the head, then parity
    /// (ARCHITECTURE §7). Every group of a fresh tree is coded — there is no
    /// older tree to reuse from — so this is `parity::PARITY` blocks per group.
    pub fn finish_with_parity(mut self) -> Result<(Cid, ParityBlocks), TreeError> {
        if self.poisoned {
            return Err(TreeError::Poisoned);
        }
        if self.levels.is_empty() {
            let leaf = empty_leaf();
            (self.sink)(leaf.cid, &leaf.bytes);
            // The empty leaf has no members, so no group and no parity.
            return Ok((leaf.cid, Vec::new()));
        }
        // A chunker holds what it coded until it is drained, so each level is
        // drained once, after its own finish — and the levels above the one
        // that produced the root are drained at the end, since the loop never
        // reaches them.
        let mut parity: ParityBlocks = Vec::new();
        let mut l = 0;
        loop {
            let mut done = Vec::new();
            self.levels[l].chunker.finish(&self.seen, &mut done)?;
            parity.extend(self.levels[l].chunker.take_coded());
            self.closed(l, done)?;
            let lv = &mut self.levels[l];
            if lv.closed == 1 {
                let root = lv.first.take().expect("held back").cid;
                // The levels above this one were never reached, but a level
                // that closed nodes earlier in the loop has already been
                // drained; only the ones after `l` can still hold anything.
                for lv in self.levels.iter_mut().skip(l + 1) {
                    parity.extend(lv.chunker.take_coded());
                }
                return Ok((root, parity));
            }
            l += 1;
        }
    }
}

impl<F: FnMut(Cid, &[u8])> TreeBuilder<F> {
    /// Add the next entry from raw bytes, letting the format choose how the
    /// value is stored and handing any value block to the same sink as the
    /// nodes.
    ///
    /// This is what a caller should reach for. [`TreeBuilder::push`] takes a
    /// [`Value`] already decided, which means restating
    /// [`Value::for_bytes`](crate::node::Value::for_bytes) — and a from-scratch
    /// build that restates the rule is exactly how an oracle stops agreeing
    /// with the thing it is checking.
    pub fn push_bytes(&mut self, key: &[u8], bytes: &[u8]) -> Result<(), TreeError> {
        let (value, block) = Value::for_bytes(bytes);
        // Registered BEFORE the push: the leaf's parity reads the member back
        // the moment the entry goes in, so emitting the block afterwards would
        // be a miss.
        if let Some((cid, b)) = block {
            self.seen.put(cid, b);
        }
        self.push(key, value)?;
        if let Some((cid, b)) = block {
            (self.sink)(cid, b);
        }
        Ok(())
    }
}

/// The root of the empty tree: one empty leaf. Every tree starts here, and two
/// empty trees have the same root.
pub fn empty_root() -> Cid {
    empty_leaf().cid
}

/// Put the empty tree into `blocks` and return its root — what a new store
/// needs before it can take a write.
pub fn init<B: crate::store::BlocksMut>(blocks: &mut B) -> Cid {
    let leaf = empty_leaf();
    blocks.insert_block(leaf.cid, &leaf.bytes);
    leaf.cid
}

/// Build a tree from entries in key order; returns the root cid.
pub fn build<'a>(
    entries: impl IntoIterator<Item = (&'a [u8], Value<'a>)>,
    sink: impl FnMut(Cid, &[u8]),
) -> Result<Cid, TreeError> {
    let mut t = TreeBuilder::new(sink);
    for (k, v) in entries {
        t.push(k, v)?;
    }
    t.finish()
}
