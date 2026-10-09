//! The real prolly tree behind [`Store`], in memory.
//!
//! An app written against the SDK today runs on the data structure it will run
//! on later: the same nodes, the same root hash, the same history-independent
//! shape. What is missing is only the network — every block is held here, so
//! nothing ever has to wait.
//!
//! Because every block is held, [`ReadError::Need`] cannot happen. It is not
//! handled gracefully: it is a bug in this file, and it says so and stops.
//! Papering over it would turn "this store forgot to keep a block it emitted"
//! into a silently wrong answer.

use crate::store::{sorted_edits, Delta, Edit, Read, Reads, Store};
use freenet_prolly::apply::{apply_into, Edit as TreeEdit};
use freenet_prolly::build::init;
use freenet_prolly::node::{Node, Value};
use freenet_prolly::range::{range, read_value, Range};
use freenet_prolly::read::{get, height};
use freenet_prolly::store::{Blocks, BlocksMut, MemBlocks, ReadError};
use freenet_prolly::Cid;
use std::ops::Bound;

/// What a store holds, for the builder's tree panel and for tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub blocks: usize,
    pub bytes: usize,
    /// Levels in the tree; 1 is a single leaf.
    pub height: usize,
}

/// Knobs that exist so a test can run a control. The defaults are the real
/// behaviour.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Keep the blocks of values too large to inline. With this off a value
    /// over 1 KiB can be written and never read back — which is what the
    /// negative control proves the suite would catch.
    pub keep_value_blocks: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            keep_value_blocks: true,
        }
    }
}

/// Where a store's blocks go.
///
/// The library writes emitted blocks through [`BlocksMut`], so the negative
/// control lives here rather than in a step the store can skip: a sink that
/// drops the blocks of values too large to inline. That is the one mistake a
/// hand-written absorb loop makes, and it has to stay expressible or nothing
/// proves the suite would catch it.
#[derive(Default)]
struct Kept {
    inner: MemBlocks,
    drop_value_blocks: bool,
    /// Block reads, for a COST assertion (`TreeStore::block_reads`): how many
    /// nodes a count or a scan actually touched. A counter, never an input.
    reads: std::cell::Cell<u64>,
    /// The distinct blocks among those reads: what a COLD store would have to
    /// fetch, which is the cost that matters (a re-read is a cache hit).
    read_ids: std::cell::RefCell<std::collections::BTreeSet<Cid>>,
}

impl Blocks for Kept {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        self.reads.set(self.reads.get() + 1);
        self.read_ids.borrow_mut().insert(*cid);
        self.inner.get(cid)
    }
}

impl BlocksMut for Kept {
    fn insert_block(&mut self, cid: Cid, bytes: &[u8]) {
        if self.drop_value_blocks && Node::parse(bytes).is_err() {
            return;
        }
        self.inner.insert(cid, bytes);
    }
}

pub struct TreeStore {
    blocks: Kept,
    root: Cid,
}

impl Default for TreeStore {
    fn default() -> Self {
        Self::with_options(Options::default())
    }
}

impl TreeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(opts: Options) -> Self {
        let mut blocks = Kept {
            drop_value_blocks: !opts.keep_value_blocks,
            ..Kept::default()
        };
        let root = init(&mut blocks);
        TreeStore {
            blocks,
            root,
        }
    }

    /// The tree's root hash. This is the whole contents in 32 bytes: two stores
    /// holding the same records report the same root whatever order they were
    /// written in.
    pub fn root(&self) -> Cid {
        self.root
    }

    /// Blocks read since this store was made — what a cost test asserts on
    /// (a count must be O(height), not one read per entry: sdk#123). Read-only
    /// and never an input to anything the store decides.
    pub fn block_reads(&self) -> u64 {
        self.blocks.reads.get()
    }

    /// The DISTINCT blocks read since the last call, and forget them — what a
    /// cold store would have fetched for the work in between.
    pub fn take_distinct_block_reads(&self) -> usize {
        std::mem::take(&mut *self.blocks.read_ids.borrow_mut()).len()
    }

    pub fn stats(&self) -> Stats {
        Stats {
            blocks: self.blocks.inner.0.len(),
            bytes: self.blocks.inner.0.values().map(Vec::len).sum(),
            height: height(&self.blocks, &self.root).unwrap_or_else(|e| Self::impossible(e)),
        }
    }

    /// Every block this store holds, as (id, bytes).
    ///
    /// A store nobody can walk is a store nobody can check. The parity rule
    /// (freenet-prolly#19) is a property of each NODE, and the only way to know
    /// that the nodes this SDK writes satisfy it is to look at all of them —
    /// including the superseded ones, which stay, and which a reader can still
    /// be handed.
    pub fn blocks(&self) -> impl Iterator<Item = (&Cid, &[u8])> {
        self.blocks.inner.0.iter().map(|(c, b)| (c, b.as_slice()))
    }

    /// The bytes of a value, wherever the format put it.
    fn materialise(&self, v: Value<'_>) -> Vec<u8> {
        match read_value(&self.blocks, v) {
            Ok(b) => b.to_vec(),
            Err(e) => Self::impossible(e),
        }
    }

    /// Every block is in memory, so a read that asks for one is this file being
    /// wrong — most likely a block that was emitted and not kept.
    fn impossible(e: ReadError) -> ! {
        match e {
            ReadError::Need(ids) => unreachable!(
                "TreeStore is in memory, so no block can be missing — but it asked for {}. \
                 A block this store emitted was not kept.",
                ids.iter()
                    .map(|c| crate::hex(c))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            other => unreachable!("the tree refused a block this store wrote: {other:?}"),
        }
    }
}

impl TreeStore {
    /// The read, which for this store is local and cannot need a round trip.
    ///
    /// The trait's `get` wraps this. `delete` calls it directly, so that a
    /// `Result` no caller of this store can ever see `Err` on does not have to
    /// be unwrapped on a write path.
    fn lookup(&self, key: &[u8]) -> Option<Vec<u8>> {
        match get(&self.blocks, &self.root, key) {
            Ok(v) => v.map(|v| self.materialise(v)),
            Err(e) => Self::impossible(e),
        }
    }
}

impl Store for TreeStore {
    /// One `apply`, so a record and everything written with it become one new
    /// root. Writing them one at a time would mint a root for a state the app
    /// never had.
    fn apply_batch(&mut self, edits: &[(Vec<u8>, Edit)]) -> Result<(), crate::store::Refused> {
        if edits.is_empty() {
            return Ok(());
        }
        let batch: Vec<(Vec<u8>, TreeEdit)> = sorted_edits(edits.to_vec())
            .into_iter()
            .map(|(k, e)| {
                (
                    k,
                    match e {
                        Edit::Put(v) => TreeEdit::Put(v),
                        Edit::Delete => TreeEdit::Delete,
                    },
                )
            })
            .collect();
        // `apply_into` writes every emitted block back; nothing is ever removed,
        // so superseded nodes stay. `replaced` is a hint for a store that wants
        // to collect them, not permission to drop history.
        let applied = match apply_into(&mut self.blocks, &self.root, &batch) {
            Ok(a) => a,
            Err(freenet_prolly::apply::ApplyError::Read(e)) => Self::impossible(e),
            // A refused batch emits nothing and changes nothing, so leaving the
            // store exactly as it was is not cleanup — it is what already
            // happened.
            //
            // `Store` has no error channel, so this cannot be reported; see the
            // trait's doc. `Db::write` screens every key and value against these
            // same limits before a store is touched and returns an error the app
            // can handle, so reaching here means something bypassed it.
            Err(e) => unreachable!("the SDK built a batch the tree refuses: {e:?}"),
        };
        self.root = applied.root;
        Ok(())
    }
}

impl Reads for TreeStore {
    fn get(&mut self, key: &[u8]) -> Read<Option<Vec<u8>>> {
        Ok(self.lookup(key))
    }

    /// From the tree's own aggregate: O(height) node reads, however many
    /// entries the range holds (craftworks-sdk#123). An edge leaf is counted
    /// entry by entry and only wholly-inside subtrees contribute their
    /// recorded count, so the answer is exactly the entries in range — what
    /// the enumerating default returns.
    ///
    /// `Claimed`, not `Verified`: this is the reader's OWN tree, so the writer
    /// whose recorded counts it trusts is itself. Only the count is used;
    /// `bytes` is logical and is not a storage size.
    fn count(&mut self, lo: &[u8], hi: &[u8]) -> Read<u64> {
        if lo >= hi {
            return Ok(0);
        }
        // A FRESH range: one carrying a limit, `after` or `reverse` is refused
        // by `aggregate`, rather than quietly answered as a page's count.
        let r = Range {
            lo: Bound::Included(lo.to_vec()),
            hi: Bound::Excluded(hi.to_vec()),
            ..Range::default()
        };
        match freenet_prolly::aggregate::aggregate(&self.blocks, &self.root, &r) {
            Ok(c) => Ok(c.agg().count),
            Err(freenet_prolly::aggregate::AggError::Read(e)) => Self::impossible(e),
            Err(e) => unreachable!("the SDK asked for a count of a range that is not whole: {e:?}"),
        }
    }

    fn scan(
        &mut self,
        lo: &[u8],
        hi: &[u8],
        reverse: bool,
        limit: usize,
    ) -> Read<Vec<(Vec<u8>, Vec<u8>)>> {
        if lo >= hi || limit == 0 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut req = Range {
            lo: Bound::Included(lo.to_vec()),
            hi: Bound::Excluded(hi.to_vec()),
            reverse,
            after: None,
            // One page unless the caller wants more than a page holds.
            max_entries: limit.min(4096),
            max_bytes: usize::MAX,
        };
        loop {
            let page = match range(&self.blocks, &self.root, &req) {
                Ok(p) => p,
                Err(freenet_prolly::range::RangeError::Read(e)) => Self::impossible(e),
                Err(e) => unreachable!("the SDK asked for an impossible range: {e:?}"),
            };
            for (k, v) in &page.entries {
                if out.len() == limit {
                    break;
                }
                out.push((k.clone(), self.materialise(*v)));
            }
            let (next, finished) = (page.next.clone(), page.finished());
            debug_assert!(page.need.is_empty(), "in memory nothing can be missing");
            drop(page);
            if finished || out.len() == limit {
                return Ok(out);
            }
            req.after = next;
        }
    }

    fn root(&mut self) -> Read<[u8; 32]> {
        Ok(TreeStore::root(self))
    }

    /// The in-memory tree keeps every block it has ever written, so it CAN
    /// diff two of its own roots — and does, through the same library call
    /// the engine uses. A binding's delta path is therefore exercised on this
    /// backend too, rather than only against a node.
    fn changes_since(
        &mut self,
        from: [u8; 32],
        lo: &[u8],
        hi: &[u8],
        max_entries: u32,
    ) -> Read<Delta> {
        let now = TreeStore::root(self);
        let r = Range {
            lo: Bound::Included(lo.to_vec()),
            hi: Bound::Excluded(hi.to_vec()),
            reverse: false,
            after: None,
            max_entries: max_entries.max(1) as usize,
            max_bytes: usize::MAX,
        };
        match freenet_prolly::diff::diff(&self.blocks, &from, &now, &r, None) {
            Ok(page) if page.need.is_empty() => {
                let cursor = page.next.as_ref().map(|n| n.after.clone());
                let changes = page
                    .changes
                    .into_iter()
                    .map(|c| match c {
                        freenet_prolly::diff::Change::Added { key, new }
                        | freenet_prolly::diff::Change::Changed { key, new, .. } => {
                            (key, Some(self.materialise(new)))
                        }
                        freenet_prolly::diff::Change::Removed { key, .. } => (key, None),
                    })
                    .collect();
                Ok(Delta::Changes {
                    changes,
                    cursor,
                    new_root: now,
                })
            }
            // A root this store never held, or one whose blocks it cannot
            // reach. The defined answer, not an error.
            _ => Ok(Delta::FullReloadRequired { new_root: now }),
        }
    }
}
