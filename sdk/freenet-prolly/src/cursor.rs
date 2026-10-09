//! A position in one LEVEL of a tree: the path from the root down to a node at
//! the floor level, with moves to the neighbouring nodes of that level.
//!
//! A move either completes or fails leaving the cursor where it was, so after a
//! [`ReadError::Need`] the caller supplies the block and simply repeats it.

use crate::node::{Agg, Node, Value};
use crate::store::{Blocks, Held, ReadError};
use crate::Cid;

struct Step<'a> {
    id: Cid,
    /// Held, not bare: the bound this node's keys must stay under travels with
    /// it, so every child opened below it is checked against the whole path
    /// (freenet-prolly#52).
    node: Held<'a>,
    /// Which child the path takes below this node (unused at the floor).
    taken: usize,
}

pub struct LevelCursor<'a, B: Blocks> {
    blocks: &'a B,
    floor: u8,
    path: Vec<Step<'a>>,
}

/// Which child of a branch can hold `key`: the last whose key is ≤ `key`.
pub(crate) fn child_for(node: &Node<'_>, key: &[u8]) -> usize {
    match node.search(key) {
        Ok(i) => i,
        Err(0) => 0,
        Err(i) => i - 1,
    }
}

impl<'a, B: Blocks> LevelCursor<'a, B> {
    /// A cursor on the LAST node of the floor level.
    pub fn seek_last(blocks: &'a B, root: &Cid, floor: u8) -> Result<Option<Self>, ReadError> {
        let node = Held::root(blocks, root)?;
        if node.level() < floor {
            return Ok(None);
        }
        let mut c = LevelCursor {
            blocks,
            floor,
            path: vec![Step {
                id: *root,
                node,
                taken: 0,
            }],
        };
        c.descend(|n| n.len() - 1)?;
        Ok(Some(c))
    }

    /// A cursor on the floor-level node chosen by `pick` at every branch.
    ///
    /// The descent rule is the caller's, because a REVERSE scan needs the
    /// mirrored one: "the last child whose first key is below the bound",
    /// which is decided from the parent's keys and therefore never loads a
    /// child outside the bound. `None` if the rule finds no child at all.
    pub fn seek_pick(
        blocks: &'a B,
        root: &Cid,
        floor: u8,
        pick: impl Fn(&Node<'a>) -> Option<usize>,
    ) -> Result<Option<Self>, ReadError> {
        let node = Held::root(blocks, root)?;
        if node.level() < floor {
            return Ok(None);
        }
        let mut c = LevelCursor {
            blocks,
            floor,
            path: vec![Step {
                id: *root,
                node,
                taken: 0,
            }],
        };
        let missing = std::cell::Cell::new(false);
        c.descend(|n| match pick(n) {
            Some(i) => i,
            None => {
                missing.set(true);
                0
            }
        })?;
        Ok((!missing.get()).then_some(c))
    }

    /// A cursor on the floor-level node that can hold `key`. `None` if the tree
    /// is not that tall.
    pub fn seek(
        blocks: &'a B,
        root: &Cid,
        floor: u8,
        key: &[u8],
    ) -> Result<Option<Self>, ReadError> {
        let node = Held::root(blocks, root)?;
        if node.level() < floor {
            return Ok(None);
        }
        let mut c = LevelCursor {
            blocks,
            floor,
            path: vec![Step {
                id: *root,
                node,
                taken: 0,
            }],
        };
        c.descend(|n| child_for(n, key))?;
        Ok(Some(c))
    }

    /// Extend the path down to the floor, choosing a child at each branch.
    fn descend(&mut self, pick: impl Fn(&Node<'a>) -> usize) -> Result<(), ReadError> {
        let keep = self.path.len();
        let r = (|| {
            while self.path.last().expect("non-empty").node.level() > self.floor {
                let top = self.path.last_mut().expect("non-empty");
                top.taken = pick(&top.node);
                let (id, _) = top.node.child(top.taken);
                let node = top.node.open(self.blocks, top.taken)?;
                self.path.push(Step { id, node, taken: 0 });
            }
            Ok(())
        })();
        if r.is_err() {
            self.path.truncate(keep);
        }
        r
    }

    pub fn node(&self) -> &Node<'a> {
        &self.path.last().expect("non-empty").node
    }
    pub fn id(&self) -> Cid {
        self.path.last().expect("non-empty").id
    }

    /// The deepest ancestor whose path can move one child to the right (`+1`)
    /// or left (`-1`).
    fn turn(&self, right: bool) -> Option<usize> {
        let above = &self.path[..self.path.len() - 1];
        above.iter().rposition(|s| {
            if right {
                s.taken + 1 < s.node.len()
            } else {
                s.taken > 0
            }
        })
    }

    /// Smallest key of the next node on this level, if there is one. Read from
    /// an ancestor: a branch key is its subtree's smallest key.
    pub fn next_min_key(&self) -> Option<Vec<u8>> {
        self.turn(true).map(|d| {
            let s = &self.path[d];
            s.node.key(s.taken + 1)
        })
    }

    pub fn is_first(&self) -> bool {
        self.turn(false).is_none()
    }

    fn step(&mut self, right: bool) -> Result<bool, ReadError> {
        let Some(d) = self.turn(right) else {
            return Ok(false);
        };
        let saved: Vec<usize> = self.path.iter().map(|s| s.taken).collect();
        let tail: Vec<Step<'a>> = self.path.drain(d + 1..).collect();
        let s = &mut self.path[d];
        s.taken = if right { s.taken + 1 } else { s.taken - 1 };
        // `descend` sets `taken` on the node it starts from, so start one below.
        let r = (|| {
            let top = &self.path[d];
            let (id, _) = top.node.child(top.taken);
            let node = top.node.open(self.blocks, top.taken)?;
            self.path.push(Step { id, node, taken: 0 });
            self.descend(|n| if right { 0 } else { n.len() - 1 })
        })();
        if let Err(e) = r {
            self.path.truncate(d + 1);
            self.path.extend(tail);
            for (s, t) in self.path.iter_mut().zip(saved) {
                s.taken = t;
            }
            return Err(e);
        }
        Ok(true)
    }

    /// Move to the next node on this level. `false` at the end of the level.
    pub fn advance(&mut self) -> Result<bool, ReadError> {
        self.step(true)
    }

    /// Move to the previous node on this level. `false` at the start.
    pub fn retreat(&mut self) -> Result<bool, ReadError> {
        self.step(false)
    }

    /// Id and recorded aggregate of the previous node on this level, without
    /// loading it (only the branches above it).
    pub fn prev_info(&self) -> Result<Option<(Cid, Agg)>, ReadError> {
        let Some(d) = self.turn(false) else {
            return Ok(None);
        };
        let s = &self.path[d];
        let mut i = s.taken - 1;
        let mut held;
        let mut node = &s.node;
        // Saturating, as in `apply::nodes_above`: `floor` comes from the
        // caller, and nothing is above 256 (freenet-prolly#55).
        while node.level() > self.floor.saturating_add(1) {
            held = node.open(self.blocks, i)?;
            node = &held;
            i = node.len() - 1;
        }
        Ok(Some(node.child(i)))
    }

    /// Ids of every node on the path above the floor node, root first.
    pub fn ancestors(&self) -> impl Iterator<Item = Cid> + '_ {
        self.path[..self.path.len() - 1].iter().map(|s| s.id)
    }
}

/// A position at one entry of the tree, and the moves a scan needs.
///
/// This is a [`LevelCursor`] at the leaves plus an index. Kept public and
/// separate from [`range`](crate::range): the per-device overlay (ARCHITECTURE
/// §5) k-way merges several of these, and it owns its own limit.
///
/// Every move is atomic. On [`ReadError::Need`] the cursor is exactly where it
/// was, so the caller fetches the block and repeats the same call.
///
/// Whether a further leaf exists is read from the ancestors, never by loading
/// it: a scan that has reached the end of its range does not pay for one more
/// block to discover that.
pub struct Cursor<'a, B: Blocks> {
    level: LevelCursor<'a, B>,
    pos: Pos,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pos {
    /// On entry `i` of the floor node.
    At(usize),
    /// Past the last entry of the tree. The level cursor is on the last leaf.
    AfterEnd,
    /// Before the first entry of the tree. The level cursor is on the first leaf.
    BeforeStart,
}

impl<'a, B: Blocks> Cursor<'a, B> {
    /// The first entry with a key ≥ `key`, or past the end if there is none.
    pub fn seek(blocks: &'a B, root: &Cid, key: &[u8]) -> Result<Self, ReadError> {
        let level = LevelCursor::seek(blocks, root, 0, key)?.expect("floor 0 always exists");
        let mut c = Cursor {
            level,
            pos: Pos::AfterEnd,
        };
        let node = c.level.node();
        let i = match node.search(key) {
            Ok(i) => i,
            Err(i) => i,
        };
        if i < node.len() {
            c.pos = Pos::At(i);
            return Ok(c);
        }
        // Every entry of this leaf is below `key`. The next leaf's first entry
        // is the answer — if there is a next leaf, which the ancestors know.
        if c.level.next_min_key().is_some() {
            c.level.advance()?;
            c.pos = if c.level.node().is_empty() {
                Pos::AfterEnd
            } else {
                Pos::At(0)
            };
        }
        Ok(c)
    }

    /// The last entry at or below `key` (`included`) or strictly below it,
    /// positioned WITHOUT loading a leaf outside that bound.
    ///
    /// This is what a reverse scan opens with. Seeking FORWARD and stepping
    /// back looks equivalent and is not: `seek` lands inside the leaf whose
    /// first key is `key`, a leaf that a reverse scan excluding `key` has no
    /// business in — and if that leaf is not held, the seek fails for a block
    /// the scan does not need. The answer is in the PREVIOUS child, and the
    /// parent's keys say so without anything being loaded.
    pub fn seek_back(
        blocks: &'a B,
        root: &Cid,
        key: &[u8],
        included: bool,
    ) -> Result<Self, ReadError> {
        let pick = |n: &Node<'a>| -> Option<usize> {
            // The last child whose first key satisfies the bound. Every key in
            // child i is ≥ its first key and < child i+1's, so a child whose
            // first key satisfies the bound holds at least one entry that does.
            let i = child_for(n, key);
            if !included && n.key(i).as_slice() == key {
                // Everything in this child is ≥ `key`, which is excluded.
                i.checked_sub(1)
            } else if included || n.key(i).as_slice() < key {
                Some(i)
            } else {
                // `key` is below this node's first child.
                None
            }
        };
        let Some(level) = LevelCursor::seek_pick(blocks, root, 0, pick)? else {
            // Nothing in the tree satisfies the bound.
            let level = LevelCursor::seek(blocks, root, 0, key)?.expect("floor 0 always exists");
            return Ok(Cursor {
                level,
                pos: Pos::BeforeStart,
            });
        };
        let node = level.node();
        // Within the leaf, the last entry satisfying the bound.
        let first_ge = match node.search(key) {
            Ok(i) => {
                if included {
                    i + 1
                } else {
                    i
                }
            }
            Err(i) => i,
        };
        let pos = match first_ge.checked_sub(1) {
            Some(i) if i < node.len() => Pos::At(i),
            _ => Pos::BeforeStart,
        };
        Ok(Cursor { level, pos })
    }

    /// The last entry with a key < `key`, or before the start if there is none.
    pub fn seek_before(blocks: &'a B, root: &Cid, key: &[u8]) -> Result<Self, ReadError> {
        let level = LevelCursor::seek(blocks, root, 0, key)?.expect("floor 0 always exists");
        let mut c = Cursor {
            level,
            pos: Pos::BeforeStart,
        };
        let node = c.level.node();
        let first_ge = match node.search(key) {
            Ok(i) => i,
            Err(i) => i,
        };
        if first_ge > 0 {
            c.pos = Pos::At(first_ge - 1);
            return Ok(c);
        }
        if !c.level.is_first() {
            c.level.retreat()?;
            let len = c.level.node().len();
            c.pos = if len == 0 {
                Pos::BeforeStart
            } else {
                Pos::At(len - 1)
            };
        }
        Ok(c)
    }

    /// The last entry of the tree, or before the start if it is empty.
    pub fn seek_last(blocks: &'a B, root: &Cid) -> Result<Self, ReadError> {
        let level = LevelCursor::seek_last(blocks, root, 0)?.expect("floor 0 always exists");
        let len = level.node().len();
        Ok(Cursor {
            level,
            pos: if len == 0 {
                Pos::BeforeStart
            } else {
                Pos::At(len - 1)
            },
        })
    }

    /// The entry the cursor is on, if it is on one.
    pub fn peek(&self) -> Option<(Vec<u8>, Value<'a>)> {
        match self.pos {
            Pos::At(i) if i < self.level.node().len() => {
                Some((self.level.node().key(i), self.level.node().value(i)))
            }
            _ => None,
        }
    }

    /// Id of the leaf the cursor is on.
    pub fn leaf(&self) -> Cid {
        self.level.id()
    }

    /// Smallest key of the leaf after this one, if there is one. Read from the
    /// ancestors, so it costs nothing.
    pub fn next_min_key(&self) -> Option<Vec<u8>> {
        self.level.next_min_key()
    }

    /// Smallest key of the leaf the cursor is on. Everything in the leaf BEFORE
    /// this one is below it, which is how a reverse scan decides it has reached
    /// the bottom of its range without loading that leaf.
    pub fn leaf_min_key(&self) -> Option<Vec<u8>> {
        let n = self.level.node();
        (!n.is_empty()).then(|| n.key(0))
    }

    /// Is this the first leaf of the tree?
    pub fn is_first_leaf(&self) -> bool {
        self.level.is_first()
    }

    /// Is the cursor on the last entry of its leaf, so that [`Self::next`] would
    /// have to load another block?
    pub fn at_leaf_end(&self) -> bool {
        matches!(self.pos, Pos::At(i) if i + 1 >= self.level.node().len())
    }

    /// Is the cursor on the first entry of its leaf, so that [`Self::prev`]
    /// would have to load another block?
    pub fn at_leaf_start(&self) -> bool {
        matches!(self.pos, Pos::At(0))
    }

    /// Move to the next entry. `false` if there is none, leaving the cursor past
    /// the end.
    // Not `Iterator`: a step can fail with the block it needs, and the items
    // borrow from the source rather than from the cursor.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<bool, ReadError> {
        match self.pos {
            Pos::AfterEnd => Ok(false),
            Pos::BeforeStart => {
                self.pos = if self.level.node().is_empty() {
                    Pos::AfterEnd
                } else {
                    Pos::At(0)
                };
                Ok(!matches!(self.pos, Pos::AfterEnd))
            }
            Pos::At(i) if i + 1 < self.level.node().len() => {
                self.pos = Pos::At(i + 1);
                Ok(true)
            }
            Pos::At(_) => {
                if self.level.next_min_key().is_none() {
                    self.pos = Pos::AfterEnd;
                    return Ok(false);
                }
                self.level.advance()?;
                self.pos = if self.level.node().is_empty() {
                    Pos::AfterEnd
                } else {
                    Pos::At(0)
                };
                Ok(!matches!(self.pos, Pos::AfterEnd))
            }
        }
    }

    /// Move to the previous entry. `false` if there is none, leaving the cursor
    /// before the start.
    pub fn prev(&mut self) -> Result<bool, ReadError> {
        match self.pos {
            Pos::BeforeStart => Ok(false),
            Pos::AfterEnd => {
                let len = self.level.node().len();
                self.pos = if len == 0 {
                    Pos::BeforeStart
                } else {
                    Pos::At(len - 1)
                };
                Ok(!matches!(self.pos, Pos::BeforeStart))
            }
            Pos::At(i) if i > 0 => {
                self.pos = Pos::At(i - 1);
                Ok(true)
            }
            Pos::At(_) => {
                if self.level.is_first() {
                    self.pos = Pos::BeforeStart;
                    return Ok(false);
                }
                self.level.retreat()?;
                let len = self.level.node().len();
                self.pos = if len == 0 {
                    Pos::BeforeStart
                } else {
                    Pos::At(len - 1)
                };
                Ok(!matches!(self.pos, Pos::BeforeStart))
            }
        }
    }
}

/// A position that stands on a child SLOT — an id and a first key read out of
/// the parent — without the child being loaded.
///
/// This is what makes a structural diff cheap. A [`LevelCursor`] descends to
/// its floor before it can tell you anything, so "both sides are at the start of
/// a node" would already have cost a root-to-leaf path in each tree; on a cold
/// tree those are network fetches of blocks the diff then declares irrelevant.
/// A slot carries everything the comparison needs — the child's id, and where
/// its keys begin — and the child is loaded only when the ids disagree.
///
/// The floor therefore moves DOWN lazily and back UP on its own: finishing a
/// descended subtree pops the path to the shallowest node that still has a
/// sibling, so the next comparison happens as high in the tree as it can.
pub struct SlotCursor<'a, B: Blocks> {
    blocks: &'a B,
    /// Loaded branches, root first. `taken` is the slot the cursor is on; the
    /// last element's `taken` is the CURRENT slot and its child is not loaded.
    path: Vec<Step<'a>>,
    /// Past the last slot of the tree.
    done: bool,
    /// The cursor stands on the ROOT itself, as a slot: the root is loaded (its
    /// level and first key are needed to compare it) but the cursor has not
    /// entered it.
    ///
    /// Without this the first comparison would already be one level down, and
    /// the commonest shape in this codebase would pay for it: when `b` is `a`
    /// with a new level on top, `a`'s whole root is a CHILD of `b`'s root, and
    /// standing on it lets one comparison skip the entire old tree. Entering
    /// `a`'s root first would instead compare its children one by one.
    at_root: bool,
}

/// What a slot is: where its keys start, how deep it goes, and what it holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot<'a> {
    /// A child subtree of `level`, not loaded.
    Sub { level: u8, id: Cid },
    /// An entry of a leaf — there is nothing below this.
    Entry(Value<'a>),
}

impl<'a, B: Blocks> SlotCursor<'a, B> {
    /// Open at the first slot of the tree. `None` if the tree is empty.
    ///
    /// Loads the root and nothing else — so two trees are compared for equality
    /// at a cost of one block each, and at NO cost when their roots are equal,
    /// because the caller compares the ids it already has before opening.
    pub fn open(blocks: &'a B, root: &Cid) -> Result<Option<Self>, ReadError> {
        let node = Held::root(blocks, root)?;
        if node.is_empty() {
            return Ok(None);
        }
        Ok(Some(SlotCursor {
            blocks,
            path: vec![Step {
                id: *root,
                node,
                taken: 0,
            }],
            done: false,
            at_root: true,
        }))
    }

    fn top(&self) -> &Step<'a> {
        self.path.last().expect("non-empty")
    }

    pub fn finished(&self) -> bool {
        self.done
    }

    /// The first key of the current slot. For an entry, the entry's key; for
    /// the root, the smallest key in the tree.
    pub fn key(&self) -> Vec<u8> {
        let s = self.top();
        if self.at_root {
            return s.node.key(0);
        }
        s.node.key(s.taken)
    }

    pub fn slot(&self) -> Slot<'a> {
        let s = self.top();
        if self.at_root {
            return Slot::Sub {
                level: s.node.level(),
                id: s.id,
            };
        }
        if s.node.is_leaf() {
            Slot::Entry(s.node.value(s.taken))
        } else {
            let (id, _) = s.node.child(s.taken);
            Slot::Sub {
                level: s.node.level() - 1,
                id,
            }
        }
    }

    /// How deep this slot reaches: the child's level, or `None` for an entry.
    /// Equal ids imply equal levels, so a comparison is only meaningful between
    /// slots of the same depth.
    pub fn level(&self) -> Option<u8> {
        match self.slot() {
            Slot::Sub { level, .. } => Some(level),
            Slot::Entry(_) => None,
        }
    }

    /// The first key AFTER this slot — never by loading anything. `None` at the
    /// end of the tree.
    ///
    /// The slot's next sibling, else the bound its node was OPENED under: the
    /// same fact `Held::open` checked, read rather than re-derived from the
    /// ancestors (it used to be computed here and enforced nowhere, while
    /// `diff` dismissed whole subtrees on it — freenet-prolly#52).
    ///
    /// This is what lets a whole subtree be dismissed as lying below the other
    /// side's position without opening it.
    pub fn end_key(&self) -> Option<Vec<u8>> {
        if self.at_root {
            // Nothing follows the whole tree.
            return None;
        }
        let s = self.top();
        s.node.child_upper(s.taken)
    }

    /// Load the current slot's child and stand on its first slot.
    ///
    /// The one call that costs a block, so every read this cursor makes is a
    /// comparison that failed or a position that had to be reached.
    pub fn descend(&mut self) -> Result<Cid, ReadError> {
        if self.at_root {
            // The root is already loaded; entering it costs nothing.
            self.at_root = false;
            return Ok(self.top().id);
        }
        let top = self.path.last().expect("non-empty");
        debug_assert!(!top.node.is_leaf(), "an entry has nothing below it");
        let (id, _) = top.node.child(top.taken);
        let node = top.node.open(self.blocks, top.taken)?;
        self.path.push(Step { id, node, taken: 0 });
        Ok(id)
    }

    /// Move past the current slot, rising to the shallowest node that still has
    /// a slot left — so the next comparison is made as high as it can be.
    pub fn advance(&mut self) {
        if self.at_root {
            self.done = true;
            return;
        }
        while let Some(s) = self.path.last_mut() {
            if s.taken + 1 < s.node.len() {
                s.taken += 1;
                return;
            }
            self.path.pop();
        }
        self.done = true;
    }

    /// The largest key under the current slot, when it can be known without
    /// loading anything.
    ///
    /// Only at the ROOT, and only when the root is a leaf: its entries are in
    /// hand, so the last one is the tree's largest key. Everywhere else a slot
    /// is bounded by the key that FOLLOWS it ([`end_key`](Self::end_key)),
    /// which the root has not got — nothing follows a whole tree.
    pub fn last_key_if_known(&self) -> Option<Vec<u8>> {
        let s = self.top();
        (self.at_root && s.node.is_leaf()).then(|| s.node.key(s.node.len() - 1))
    }

    /// The current node and the index within it — what `need` is computed from.
    pub fn parent(&self) -> (&Node<'a>, usize) {
        let s = self.top();
        (&s.node, s.taken)
    }

    /// The loaded node of exactly `level` on this cursor's path, and the slot
    /// it is currently taking.
    ///
    /// Two cursors can sit at different DEPTHS — the normal sync shape, where
    /// one side is held and the other is being fetched — and child-id sets from
    /// two different levels are disjoint whatever the trees hold, so comparing
    /// them names everything and means nothing. This is how a caller finds the
    /// level the two sides can actually be compared at.
    pub fn node_at_level(&self, level: u8) -> Option<(&Node<'a>, usize)> {
        self.path
            .iter()
            .find(|s| s.node.level() == level)
            .map(|s| (s.node.node(), s.taken))
    }
}
