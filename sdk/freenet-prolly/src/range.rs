//! Ordered scan over a key range, forward or reverse.
//!
//! Shaped by the engine that calls it (ARCHITECTURE §12): it runs in rounds of
//! a few network operations and cannot wait. So a scan serves what it can from
//! the blocks in hand, says where to resume **by key**, and names the blocks it
//! is missing so the caller can fetch a whole round's worth at once.
//!
//! Resuming by key, never by a position in a node, is what makes a page survive
//! the tree changing underneath it: the caller may hand the next page a newer
//! root and get that tree's entries from the same key on.
//!
//! Aggregates are believed for BUDGETING and never for CONTENT: what a page
//! serves comes from leaves the reader has loaded and checked against their
//! parents, while what it NAMES is decided from numbers a branch records about
//! children nobody has seen yet. Naming a block one does not hold is a decision
//! made on hearsay, and the bounds here are what keep a lie about it cheap.
//!
//! # For the range aggregate (#7), which reuses [`walk`]
//!
//! Two things want changing first, neither of which matters while the walk only
//! builds a frontier:
//!
//! - it builds `node.key(i)` for every child — the same per-entry allocation
//!   #18 took out of `check_node`. Prefer `node.search` on the two bounds and
//!   treat everything strictly between them as [`Span::Inside`], which needs no
//!   key at all;
//! - it is a procedure with a sink rather than a visitor. #7 wants
//!   `(child, agg, Span)` yielded, and it will need `Edge` children DESCENDED
//!   rather than merely classified, since an edge's exact contribution is only
//!   known further down.

use crate::boundary::MAX_LOGICAL;
use crate::cursor::Cursor;
use crate::node::{Agg, Node, Value};
use crate::store::{Blocks, Held, ReadError};
use crate::Cid;
use core::cmp::Ordering;
use std::ops::Bound;

/// Most blocks one page will name. The caller fetches a round at a time, so
/// naming more than it can use only delays the entries it asked for.
pub use crate::apply::MAX_NEED;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeError {
    Read(ReadError),
    /// `max_entries` was 0. A page that can never hold an entry is a caller
    /// mistake, not an empty result: returning `Ok` would loop forever.
    NoLimit,
}

impl From<ReadError> for RangeError {
    fn from(e: ReadError) -> Self {
        RangeError::Read(e)
    }
}

/// Knobs that exist so the tests can run a control. The defaults are the
/// format's behaviour; nothing else should change them.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Stop naming blocks once the named aggregates cover the caller's limit.
    /// With this off, a page for twenty entries names blocks for sixty-four.
    pub agg_bound: bool,
    /// Budget a leaf's bytes by what a PAGE can take from it, not by the
    /// logical bytes its aggregate records. With this off, one leaf of large
    /// referenced values "covers" a 256 KiB budget by itself.
    pub cap_leaf_bytes: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            agg_bound: true,
            cap_leaf_bytes: true,
        }
    }
}

/// What to scan. `lo`/`hi` are the range; `after` is where to resume.
#[derive(Clone, Debug)]
pub struct Range {
    pub lo: Bound<Vec<u8>>,
    pub hi: Bound<Vec<u8>>,
    pub reverse: bool,
    /// Resume strictly after this key in scan order: forward, keys greater than
    /// it; reverse, keys less than it. Intersected with `lo`/`hi`, and a key
    /// outside them is not an error — it may have been deleted under a newer
    /// root.
    pub after: Option<Vec<u8>>,
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for Range {
    fn default() -> Self {
        Range {
            lo: Bound::Unbounded,
            hi: Bound::Unbounded,
            reverse: false,
            after: None,
            max_entries: 1024,
            max_bytes: 256 * 1024,
        }
    }
}

impl Range {
    /// Every key starting with `p`, i.e. `[p, successor(p))`.
    ///
    /// The successor increments the last byte below `0xff` and drops the rest;
    /// a prefix that is empty, all `0xff`, or longer than a key can be has no
    /// successor, so the range runs to the end.
    pub fn prefix(p: &[u8]) -> Range {
        Range {
            lo: Bound::Included(p.to_vec()),
            hi: match successor(p) {
                Some(s) => Bound::Excluded(s),
                None => Bound::Unbounded,
            },
            ..Range::default()
        }
    }
}

/// Smallest key that is greater than every key starting with `p`.
fn successor(p: &[u8]) -> Option<Vec<u8>> {
    // A key longer than MAX_KEY cannot exist, so nothing starts with such a
    // prefix — but the range must then be empty, not unbounded, which the
    // caller gets from `lo` being above every key.
    let cut = p.iter().rposition(|b| *b != 0xff)?;
    let mut s = p[..=cut].to_vec();
    s[cut] += 1;
    Some(s)
}

/// One page of a scan. `entries` borrow from `blocks`, so consume a page before
/// inserting newly fetched blocks into the source.
#[derive(Debug)]
pub struct Page<'a> {
    pub entries: Vec<(Vec<u8>, Value<'a>)>,
    /// Resume by passing the ORIGINAL `lo`/`hi` again with `after = next`; the
    /// bounds are the range, this is only the cursor into it.
    ///
    /// It does NOT promise more entries exist: when the limit lands exactly on
    /// the range's last entry, the scan does not read on to find out, and one
    /// extra empty page is the honest cost.
    pub next: Option<Vec<u8>>,
    /// Blocks to fetch, in scan order: the leaf the scan stopped at, plus
    /// further blocks of the range that the held branches can name.
    pub need: Vec<Cid>,
    /// WHY the page ended. Only [`PageEnd::Blocked`] leaves work undone, and a
    /// reader that treats an empty `need` as "complete" cannot tell the
    /// difference — so the scan says it outright.
    pub end: PageEnd,
}

/// How a page ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageEnd {
    /// The entry or byte limit was reached. More may follow.
    Limit,
    /// The scan walked off the end of the range.
    EndOfRange,
    /// The scan walked off the end of the tree.
    EndOfTree,
    /// A block was missing. **Nothing here says the range is exhausted** —
    /// this is the one ending a completeness claim may not be built on.
    Blocked,
}

impl Page<'_> {
    /// The scan is over: it ran out of range or out of tree, and it got there
    /// by SCANNING rather than by failing to fetch something.
    ///
    /// Not "nothing to fetch": a scan that could not open its first leaf also
    /// has nothing to fetch in the range, and it has served nothing.
    pub fn finished(&self) -> bool {
        matches!(self.end, PageEnd::EndOfRange | PageEnd::EndOfTree)
    }
}

/// Does `key` satisfy the low end of the range?
fn above_lo(key: &[u8], lo: &Bound<Vec<u8>>) -> bool {
    match lo {
        Bound::Unbounded => true,
        Bound::Included(k) => key >= k.as_slice(),
        Bound::Excluded(k) => key > k.as_slice(),
    }
}

/// Does `key` satisfy the high end of the range?
pub(crate) fn below_hi(key: &[u8], hi: &Bound<Vec<u8>>) -> bool {
    match hi {
        Bound::Unbounded => true,
        Bound::Included(k) => key <= k.as_slice(),
        Bound::Excluded(k) => key < k.as_slice(),
    }
}

pub(crate) fn in_range(key: &[u8], r: &Range) -> bool {
    above_lo(key, &r.lo) && below_hi(key, &r.hi)
}

/// Where a scan starts, with `after` folded into the bound it tightens.
fn start_bound(r: &Range) -> Bound<Vec<u8>> {
    let bound = if r.reverse { &r.hi } else { &r.lo };
    let Some(a) = &r.after else {
        return bound.clone();
    };
    let inside = if r.reverse {
        below_hi(a, bound)
    } else {
        above_lo(a, bound)
    };
    // An `after` outside the range leaves the bound alone rather than failing:
    // the caller may be resuming into a newer tree where that key is gone.
    if inside {
        Bound::Excluded(a.clone())
    } else {
        bound.clone()
    }
}

/// How many bytes an entry costs the page.
/// What a page CARRIES for one entry: the key, and the value as the page holds
/// it — a referenced value is 32 bytes of id here, whatever its real length.
///
/// Shared with [`diff`](crate::diff) so the two cannot drift: a byte limit that
/// charged a reference at its referenced length would refuse to put two large
/// values in one page while sending 64 bytes.
pub(crate) fn entry_bytes(key: &[u8], v: &Value<'_>) -> usize {
    key.len()
        + match v {
            Value::Inline(b) => b.len(),
            Value::Ref { .. } => 32,
        }
}

/// Do the bounds already say the range is empty?
///
/// Decided from the bounds alone, before anything is read. It matters most on a
/// RESUME: paging reverse past the last entry of the range leaves a start bound
/// of `Excluded(lo)` against a lower bound of `Included(lo)`, which nothing can
/// satisfy — and without this the scan would descend toward a neighbouring leaf
/// it has no use for, and either fetch it or (worse, before #37) conclude from
/// an empty frontier that it had finished.
pub(crate) fn bounds_are_empty(r: &Range) -> bool {
    let start = start_bound(r);
    let far = if r.reverse { &r.lo } else { &r.hi };
    match (&start, far) {
        (Bound::Unbounded, _) | (_, Bound::Unbounded) => false,
        (Bound::Included(s), Bound::Included(f)) => {
            if r.reverse {
                s < f
            } else {
                s > f
            }
        }
        // One end open: a key must lie strictly between them.
        (Bound::Included(s), Bound::Excluded(f))
        | (Bound::Excluded(s), Bound::Included(f))
        | (Bound::Excluded(s), Bound::Excluded(f)) => {
            if r.reverse {
                s <= f
            } else {
                s >= f
            }
        }
    }
}

/// Place a cursor at the first entry of the scan. `None` means the scan is over
/// before it starts — which is not the same as an empty tree: it is reached when
/// the only step left would cross into a leaf that lies wholly outside the
/// range, and that step is exactly what must not be paid for.
fn open<'a, B: Blocks>(
    blocks: &'a B,
    root: &Cid,
    r: &Range,
) -> Result<Option<Cursor<'a, B>>, ReadError> {
    let start = start_bound(r);
    if r.reverse {
        // Both bounded cases go through `seek`, so the step backwards is one the
        // range can be consulted about first. `seek_before` would take it inside
        // the cursor, where the range is not known.
        let (key, included) = match &start {
            Bound::Unbounded => return Cursor::seek_last(blocks, root).map(Some),
            Bound::Included(k) => (k, true),
            Bound::Excluded(k) => (k, false),
        };
        // The MIRRORED descent. Seeking forward and stepping back would land
        // inside the leaf whose first key is `key` — a leaf a reverse scan
        // excluding `key` has no business in, and one the scan then cannot
        // proceed without. `seek_back` picks the last child whose first key
        // satisfies the bound, from the parent's keys, so no such leaf is ever
        // loaded.
        let c = Cursor::seek_back(blocks, root, key, included)?;
        return Ok(c.peek().is_some().then_some(c));
    }
    match &start {
        Bound::Unbounded => Cursor::seek(blocks, root, &[]).map(Some),
        Bound::Included(k) => Cursor::seek(blocks, root, k).map(Some),
        Bound::Excluded(k) => {
            let mut c = Cursor::seek(blocks, root, k)?;
            if c.peek().map(|(got, _)| got == *k) == Some(true) {
                if at_edge_of_range(&c, r) {
                    return Ok(None);
                }
                c.next()?;
            }
            Ok(Some(c))
        }
    }
}

/// Serve as much of `r` from `blocks` as the limits and the held blocks allow.
///
/// Never skips a hole: the page stops at the first missing leaf, and `need`
/// names it. Values are borrowed from `blocks`.
pub fn range<'a, B: Blocks>(blocks: &'a B, root: &Cid, r: &Range) -> Result<Page<'a>, RangeError> {
    range_with(Options::default(), blocks, root, r)
}

/// [`range`], with the test controls.
pub fn range_with<'a, B: Blocks>(
    opts: Options,
    blocks: &'a B,
    root: &Cid,
    r: &Range,
) -> Result<Page<'a>, RangeError> {
    if r.max_entries == 0 {
        return Err(RangeError::NoLimit);
    }
    let mut page = Page {
        entries: Vec::new(),
        next: None,
        need: Vec::new(),
        end: PageEnd::Blocked,
    };
    // Bounds that cannot hold a key: the scan is over before it starts, and it
    // is the SCAN saying so — no block is read to find out.
    if bounds_are_empty(r) {
        page.end = PageEnd::EndOfRange;
        return Ok(page);
    }
    // The root itself may be missing; then there is nothing to serve and one
    // block to ask for.
    if blocks.get(root).is_none() {
        page.need.push(*root);
        return Ok(page);
    }

    let mut cur = match open(blocks, root, r) {
        // The only step left would have left the range: the SCAN decided this,
        // so it is a real ending.
        Ok(None) => {
            page.end = PageEnd::EndOfRange;
            return Ok(page);
        }
        Ok(Some(c)) => c,
        Err(ReadError::Need(ids)) => {
            // Nothing served. Name what the held branches can see of the range
            // — and if that is nothing, name the block the descent stopped on
            // anyway. An empty frontier used to be read as "nothing in range is
            // missing, so the page is finished": it is not. Nothing to FETCH is
            // not nothing to SERVE, and a scan that has served nothing has not
            // finished anything. A wasted fetch costs a block; a false
            // "finished" is a listing that lies.
            page.need = frontier(opts, blocks, root, &rest_from_start(r))?;
            if page.need.is_empty() {
                page.need = ids;
            }
            return Ok(page);
        }
        Err(e) => return Err(e.into()),
    };

    let mut bytes = 0usize;
    // The key to resume after is always the last one actually SERVED. Resuming
    // after an entry the page refused would skip it.
    let mut last_served: Option<Vec<u8>> = None;
    loop {
        let Some((key, value)) = cur.peek() else {
            // Ran off the end of the tree: the scan is complete.
            page.end = PageEnd::EndOfTree;
            return Ok(page);
        };
        if !in_range(&key, r) {
            // Past the far end of the range: complete.
            page.end = PageEnd::EndOfRange;
            return Ok(page);
        }
        let cost = entry_bytes(&key, &value);
        // Progress: a page always returns one entry if one is available, even
        // when that entry alone is over the byte limit. Otherwise a single large
        // entry would stall the scan forever.
        if !page.entries.is_empty() && bytes + cost > r.max_bytes {
            page.end = PageEnd::Limit;
            break;
        }
        bytes += cost;
        page.entries.push((key.clone(), value));
        last_served = Some(key.clone());
        if page.entries.len() >= r.max_entries {
            page.end = PageEnd::Limit;
            break;
        }
        // Stepping to the neighbouring leaf costs a block, and the ancestors
        // already say which keys it holds. A scan that has reached the end of
        // its range stops here instead of paying for a leaf it cannot use.
        if at_edge_of_range(&cur, r) {
            page.end = PageEnd::EndOfRange;
            return Ok(page);
        }
        // Step. A missing next leaf ends the page with a frontier.
        let moved = if r.reverse { cur.prev() } else { cur.next() };
        match moved {
            Ok(true) => {}
            Ok(false) => {
                page.end = PageEnd::EndOfTree;
                return Ok(page);
            }
            Err(ReadError::Need(ids)) => {
                page.next = last_served.clone();
                page.need = frontier(opts, blocks, root, &rest_after(r, &key))?;
                if page.need.is_empty() {
                    page.need = ids;
                }
                return Ok(page);
            }
            Err(e) => return Err(e.into()),
        }
    }
    // The limit stopped us with entries in hand. Resuming from the last key
    // served is correct whether or not anything follows it.
    page.next = last_served;
    Ok(page)
}

/// Is the neighbouring leaf wholly outside the range? Decided from the
/// ancestors, so it never loads it.
fn at_edge_of_range<B: Blocks>(cur: &Cursor<'_, B>, r: &Range) -> bool {
    // Only the last entry of a leaf costs a block to step past. Anywhere else
    // the neighbouring leaf is irrelevant — asking about it here would end the
    // page in the middle of a leaf that still has entries in range.
    let crossing = if r.reverse {
        cur.at_leaf_start()
    } else {
        cur.at_leaf_end()
    };
    if !crossing {
        return false;
    }
    if r.reverse {
        // Everything before this leaf is below its smallest key.
        match (cur.leaf_min_key(), &r.lo) {
            (Some(min), Bound::Included(k) | Bound::Excluded(k)) => min <= *k,
            _ => false,
        }
    } else {
        match (cur.next_min_key(), &r.hi) {
            (Some(min), _) => !below_hi(&min, &r.hi),
            (None, _) => false,
        }
    }
}

/// The part of `r` still to come after `key` in scan order.
fn rest_after(r: &Range, key: &[u8]) -> Range {
    let mut rest = r.clone();
    if r.reverse {
        rest.hi = Bound::Excluded(key.to_vec());
    } else {
        rest.lo = Bound::Excluded(key.to_vec());
    }
    rest.after = None;
    rest
}

/// The part of `r` the scan has not started yet, with `after` folded in.
fn rest_from_start(r: &Range) -> Range {
    let mut rest = r.clone();
    let start = start_bound(r);
    if r.reverse {
        rest.hi = start;
    } else {
        rest.lo = start;
    }
    rest.after = None;
    rest
}

/// Which part of the range a child covers.
///
/// The frontier treats both the same — see [`Walk::name`] — so this exists for
/// the range aggregate of #7, which reuses this walk and needs to know whether a
/// child's recorded aggregate is the exact answer or an upper bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Span {
    /// Wholly inside the range: its aggregate counts exactly.
    Inside,
    /// Straddles an end of the range: its aggregate is an upper bound on what
    /// the scan will take from it.
    Edge,
}

/// The blocks a scan of `rest` needs, in scan order, named from the held
/// branches alone.
///
/// Public because the per-device overlay k-way merges several [`Cursor`]s and
/// has to name the frontier of a tree that is blocked while the others keep
/// serving — it needs this without going through [`range`].
pub fn frontier_of<B: Blocks>(blocks: &B, root: &Cid, rest: &Range) -> Result<Vec<Cid>, ReadError> {
    frontier(Options::default(), blocks, root, rest)
}

/// The blocks a scan needs to continue past `from`, in scan order.
///
/// Walks the branches that are HELD, names the blocks that are not, and stops
/// once the blocks it has named can already cover the rest of the caller's
/// limit — otherwise "the latest 20 posts" would fetch 64 leaves to serve 20
/// entries. Aggregates make that decidable without reading anything: a child
/// wholly inside the range contributes exactly its `agg`, an edge child at most
/// its `agg`.
///
/// A missing branch is named and not descended: its children are unknowable
/// until it arrives.
fn frontier<B: Blocks>(
    opts: Options,
    blocks: &B,
    root: &Cid,
    rest: &Range,
) -> Result<Vec<Cid>, ReadError> {
    let mut f = Walk {
        out: Vec::new(),
        entries: 0,
        bytes: 0,
        want_entries: rest.max_entries as u64,
        want_bytes: rest.max_bytes as u64,
        covered: false,
        agg_bound: opts.agg_bound,
        cap_leaf_bytes: opts.cap_leaf_bytes,
    };
    let node = Held::root(blocks, root)?;
    walk(blocks, &node, rest, &mut |c: &Child| {
        if f.covered {
            return Visit::Stop;
        }
        if blocks.get(&c.id).is_none() {
            f.name(c.id, c.agg, c.span, c.is_leaf);
            return Visit::Next;
        }
        if c.is_leaf {
            // Held: nothing to fetch, but what it holds counts against the
            // caller's limit.
            f.hold(c.agg);
            return Visit::Next;
        }
        Visit::Descend
    })?;
    Ok(f.out)
}

struct Walk {
    out: Vec<Cid>,
    entries: u64,
    bytes: u64,
    want_entries: u64,
    want_bytes: u64,
    covered: bool,
    agg_bound: bool,
    cap_leaf_bytes: bool,
}

impl Walk {
    /// Name a block, and record what it can contribute.
    ///
    /// An `Inside` child contributes exactly its aggregate and an `Edge` child
    /// at most its aggregate, so both are added the same way: the question here
    /// is only whether enough has been named to cover the caller's limit, and
    /// for that an upper bound is the right side to err on — it can end the
    /// frontier early, costing one more round, never a wrong answer. (The span
    /// is still yielded: the range aggregate of #7 needs the exact/bound
    /// distinction, and this walk is the one it will reuse.)
    fn name(&mut self, id: Cid, agg: Agg, _span: Span, leaf: bool) {
        if self.out.contains(&id) {
            return;
        }
        self.out.push(id);
        // The recount comes after the push, so the block that stopped the scan
        // is always named however tight the limit is.
        self.add(agg, leaf);
    }

    /// A leaf that is already HELD needs no fetch, but it still fills the
    /// caller's limit — so it counts against the budget too, or blocks beyond it
    /// get named for entries the caller will never ask for.
    fn hold(&mut self, agg: Agg) {
        // Never before something has been named: the block that stopped the scan
        // has to be in the frontier, or the caller cannot make progress at all.
        if self.out.is_empty() {
            return;
        }
        self.add(agg, true);
    }

    fn add(&mut self, agg: Agg, leaf: bool) {
        // `agg.bytes` is LOGICAL bytes: a referenced value counts its full
        // length, while a page charges it 32 bytes for the reference. So a
        // leaf's aggregate can claim to cover a whole byte budget on its own,
        // and the frontier would stop after naming it. What a page can actually
        // take from a leaf is at most the leaf's own measure — still an upper
        // bound, and one that does not depend on where the values live.
        //
        // A missing BRANCH keeps its recorded bytes: it is named and not
        // descended, so how its subtree is shaped is unknowable here.
        let bytes = if leaf && self.cap_leaf_bytes {
            agg.bytes.min(MAX_LOGICAL as u64)
        } else {
            agg.bytes
        };
        self.entries = self.entries.saturating_add(agg.count);
        self.bytes = self.bytes.saturating_add(bytes);
        // EITHER limit ends a page, so either one being covered means what has
        // been named can already fill it. Requiring both would name a byte
        // budget's worth of leaves to serve twenty entries — the exact waste
        // this bound exists to prevent.
        let enough =
            self.agg_bound && (self.entries >= self.want_entries || self.bytes >= self.want_bytes);
        if self.out.len() >= MAX_NEED || enough {
            self.covered = true;
        }
    }
}

/// One child of a branch, as the walk sees it — before anything is loaded.
///
/// Everything here is read from the PARENT, so it is known even for a child
/// nobody holds. That is the point: a frontier names blocks it does not have.
#[derive(Clone)]
pub(crate) struct Child {
    /// Which child of the parent this is — what `load_child` needs to check it.
    pub idx: usize,
    pub id: Cid,
    pub agg: Agg,
    pub span: Span,
    /// A child of a level-1 branch is a leaf.
    pub is_leaf: bool,
}

/// What the visitor wants done with a child it has just been shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Visit {
    /// Look inside it. Only a held branch can be descended; anything else is
    /// treated as `Next`.
    Descend,
    /// Nothing more here; carry on with the next child.
    Next,
    /// End the walk.
    Stop,
}

/// Show `visit` every child of `node` that intersects `r`, in scan order.
///
/// A visitor rather than a procedure with a sink: the frontier below is one
/// consumer, and the range aggregate (#7) is another that needs the same
/// classification for a different purpose. Neither belongs inside the walk.
///
/// The node is [`Held`], so it carries the smallest key of whatever follows it
/// — the bound its last child's coverage ends at, which the node itself cannot
/// know. That is what makes [`Span::Inside`] decidable for a last child.
pub(crate) fn walk<'a, B: Blocks>(
    blocks: &'a B,
    node: &Held<'a>,
    r: &Range,
    visit: &mut impl FnMut(&Child) -> Visit,
) -> Result<(), ReadError> {
    // Depth-first, in scan order: a child that is descended into is finished
    // before its next sibling is shown. The frontier's `need` is a list the
    // caller fetches in order, so this is not a detail of the traversal.
    //
    // Classified one index at a time rather than through a list, so a walk
    // allocates nothing per node either.
    for i in scan_order(node, r) {
        let Some(c) = classify(node, i, node.upper(), r) else {
            continue;
        };
        match visit(&c) {
            Visit::Stop => return Ok(()),
            Visit::Next => continue,
            Visit::Descend => {}
        }
        if c.is_leaf || blocks.get(&c.id).is_none() {
            continue;
        }
        // Opened through `Held`, like every other child. This walk used to take
        // a child on a level check alone ("aggregates are believed for
        // budgeting"), but a SPAN is not a budget: an unchecked one made the
        // frontier unsorted, and reverse paging over it returned the same page
        // for ever (freenet-prolly#52).
        let loaded = node.open(blocks, c.idx)?;
        walk(blocks, &loaded, r, visit)?;
    }
    Ok(())
}

/// The children of THIS node that intersect `r`, in scan order.
///
/// One level only, and it loads nothing: everything a [`Child`] carries is read
/// from this node. A caller that wants to go deeper decides how, which is what
/// separates the frontier from the range aggregate.
pub(crate) fn children(node: &Held<'_>, r: &Range) -> Vec<Child> {
    scan_order(node, r)
        .filter_map(|i| classify(node, i, node.upper(), r))
        .collect()
}

/// The indices of `node`'s children, in the order a scan visits them.
fn scan_order(node: &Node<'_>, r: &Range) -> Box<dyn Iterator<Item = usize>> {
    let n = if node.is_leaf() { 0 } else { node.len() };
    if r.reverse {
        Box::new((0..n).rev())
    } else {
        Box::new(0..n)
    }
}

/// Child `i` of `node`, if it intersects `r`.
///
/// Takes no store: classifying a child reads the PARENT only, so deciding that
/// a subtree lies wholly inside a range costs nothing and touches nothing. That
/// is what makes an aggregate cheap, so it is a property of the signature
/// rather than of how carefully each caller uses it — including "is the block
/// held", which is a question about a specific child and belongs to whoever
/// wants to open one.
fn classify(node: &Node<'_>, i: usize, upper: Option<&[u8]>, r: &Range) -> Option<Child> {
    let n = node.len();
    let prefix = node.prefix();
    {
        // Keys are read as the node's shared prefix plus this entry's suffix and
        // are NOT joined: a key is only built for a child the walk descends
        // into, which is a handful per walk rather than one per child.
        let min = Key(prefix, node.suffix(i));
        let next = (i + 1 < n).then(|| Key(prefix, node.suffix(i + 1)));

        // Every key this child holds is ≥ `min` and < `next`. Both tests below
        // follow from that and nothing else, so neither reads the child.
        //
        // Wholly above the range: its smallest key is already past the top.
        let above = !min.below_hi(&r.hi);
        // Wholly below it: everything it holds is under `next`, so if `next` is
        // at or below the bottom bound, nothing in it can qualify.
        let below = match (&next, &r.lo) {
            (Some(nx), Bound::Included(k) | Bound::Excluded(k)) => {
                nx.cmp_key(k) != Ordering::Greater
            }
            _ => false,
        };
        if above || below {
            return None;
        }
        // Wholly within: its smallest key clears the bottom, and everything
        // under `next` clears the top.
        let inside = min.above_lo(&r.lo)
            && match (&next, &r.hi) {
                (_, Bound::Unbounded) => true,
                (Some(nx), Bound::Included(k) | Bound::Excluded(k)) => {
                    nx.cmp_key(k) != Ordering::Greater
                }
                (None, _) => match upper {
                    // The node's own bound stands in for a missing next key.
                    Some(u) => {
                        below_hi(u, &r.hi)
                            || matches!(&r.hi, Bound::Included(h) | Bound::Excluded(h) if u <= h.as_slice())
                    }
                    None => false,
                },
            };
        let (id, agg) = node.child(i);
        let child = Child {
            idx: i,
            id,
            agg,
            span: if inside { Span::Inside } else { Span::Edge },
            is_leaf: node.level() == 1,
        };
        Some(child)
    }
}

/// A key held as the node keeps it: a shared prefix and one entry's suffix,
/// compared without being joined.
struct Key<'a>(&'a [u8], &'a [u8]);

impl Key<'_> {
    fn cmp_key(&self, other: &[u8]) -> Ordering {
        let Key(prefix, suffix) = self;
        let n = prefix.len().min(other.len());
        match prefix[..n].cmp(&other[..n]) {
            Ordering::Equal => {}
            ord => return ord,
        }
        if other.len() < prefix.len() {
            return Ordering::Greater;
        }
        (*suffix).cmp(&other[prefix.len()..])
    }

    fn above_lo(&self, lo: &Bound<Vec<u8>>) -> bool {
        match lo {
            Bound::Unbounded => true,
            Bound::Included(k) => self.cmp_key(k) != Ordering::Less,
            Bound::Excluded(k) => self.cmp_key(k) == Ordering::Greater,
        }
    }

    fn below_hi(&self, hi: &Bound<Vec<u8>>) -> bool {
        match hi {
            Bound::Unbounded => true,
            Bound::Included(k) => self.cmp_key(k) != Ordering::Greater,
            Bound::Excluded(k) => self.cmp_key(k) == Ordering::Less,
        }
    }
}

/// The bytes of a value that lives in its own block.
///
/// The id binds the block's kind (`BLAKE3(RAW ‖ body)`), so a tree node can
/// never be served as a value: its id would be a different one. The recorded
/// length is checked, because a leaf's `vlen` and the block are two separate
/// statements and a reader that trusted the leaf could be handed either.
pub fn value_bytes<'a>(
    blocks: &'a impl Blocks,
    cid: &Cid,
    len: u32,
) -> Result<&'a [u8], ReadError> {
    let body = blocks.get(cid).ok_or_else(|| ReadError::Need(vec![*cid]))?;
    if body.len() != len as usize {
        return Err(ReadError::Mismatch(*cid));
    }
    Ok(body)
}

/// The bytes of a value, wherever the format put it.
///
/// A leaf holds a short value inline and a long one by reference, so every
/// reader has to handle both. Doing it here means a caller does not write the
/// match — and does not forget the length check on the reference arm.
pub fn read_value<'a>(blocks: &'a impl Blocks, v: Value<'a>) -> Result<&'a [u8], ReadError> {
    match v {
        Value::Inline(b) => Ok(b),
        Value::Ref { cid, len } => value_bytes(blocks, &cid, len),
    }
}
