//! Change a tree: `apply(root, edits)` gives the tree a from-scratch build of
//! the edited contents would give, touching only the nodes the edits reach.
//!
//! Per level, left to right: restart the chunker at the first entry of the node
//! that holds the next edit (a chunker at the start of any node is in the same
//! state whatever lies to its left), stream old entries merged with the edits,
//! and stop as soon as the chunker closes a node exactly where an old node
//! ended with no edit left for the node that follows — from there on the old
//! nodes are what a rebuild would produce, and are kept by id. The run of nodes
//! that was replaced becomes the edit batch for the level above.
//!
//! Nothing is handed to the sink, and nothing is read that the edits do not
//! reach, until the whole change has succeeded. A refused batch, or one that
//! stops for a missing block, leaves no trace.

use crate::boundary::{self, MAX_LOGICAL};
use crate::chunk::{empty_leaf, Body, Closed, LevelChunker, SplitRule};
use crate::cursor::LevelCursor;
use crate::node::{Agg, BuildError, Node, Value, HEADER, MAX_INLINE, MAX_KEY};
use crate::store::{Blocks, BlocksMut, Held, ReadError};
use crate::Cid;
use std::collections::{BTreeMap, HashMap, HashSet};

pub use crate::node::MAX_VALUE;
/// At most this many missing blocks are named at once.
pub const MAX_NEED: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Store these bytes under the key. The library decides whether they live
    /// in the leaf or in a block of their own.
    Put(Vec<u8>),
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyError {
    Read(ReadError),
    /// Keys in a batch must be strictly increasing (so: no key twice).
    NotSorted,
    KeyTooLong,
    ValueTooLong,
    Node(BuildError),
}

impl ApplyError {
    /// A missing parity member is a READ, not a malformed tree: the caller
    /// fetches it and calls again, exactly as for a missing node. Only a build
    /// with no store to fetch from turns it into a refusal.
    fn from_build(e: BuildError) -> Self {
        match e {
            BuildError::MissingMember(cid) => ApplyError::Read(ReadError::Need(vec![cid])),
            other => ApplyError::Node(other),
        }
    }
}

impl From<ReadError> for ApplyError {
    fn from(e: ReadError) -> Self {
        ApplyError::Read(e)
    }
}
impl From<BuildError> for ApplyError {
    fn from(e: BuildError) -> Self {
        ApplyError::Node(e)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub root: Cid,
    /// Tree nodes of the old tree that the new tree no longer uses. A hint for
    /// caches and retention, never a delete list: another tree may still use
    /// them. (Value blocks are not listed: another key may hold the same value.)
    pub replaced: Vec<Cid>,
    /// The parity blocks this rewrite CODED, with their bytes: the redundancy
    /// the new nodes promise and nobody had until now.
    ///
    /// A node lists parity ids; the bytes behind them are separate blocks, and
    /// ARCHITECTURE §7 has the commit go data nodes → head → parity. So they
    /// are handed over apart from the sink rather than interleaved with it,
    /// and the ORDER stays the caller's decision.
    ///
    /// **What the engine owes.** A group's parity ids may be listed before its
    /// parity blocks exist, but the engine owes those blocks, and it should
    /// track the debt the way it tracks an unpublished commit. Until they are
    /// put, that group has NO redundancy — and a later edit to it falls back
    /// to a full recode, because a correction needs the old parity to correct.
    ///
    /// Groups that were REUSED are not listed: their parity is already out.
    /// Corrected and recoded groups are, `parity::PARITY` blocks each.
    pub parity: crate::build::ParityBlocks,
}

/// How to apply. The format is [`Options::default`]; anything else exists so
/// tests can show what each part is for.
#[derive(Clone, Copy)]
pub struct Options {
    pub rule: SplitRule,
    /// Re-chunk the node before an edited first entry when it may have been
    /// closed by the hard limit. Off = the bug this guards against.
    pub recheck_neighbour: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            rule: boundary::splits_after,
            recheck_neighbour: true,
        }
    }
}

type LevelEdits = Vec<(Vec<u8>, Option<Body>)>;

struct LevelResult {
    new: Vec<Closed>,
    /// Old nodes that were streamed: id and smallest key.
    old: Vec<(Cid, Vec<u8>)>,
    /// Every old node of the level was streamed: `new` IS the level.
    whole: bool,
    /// Per edit: did it change anything?
    effective: Vec<bool>,
    /// Parity blocks coded while rewriting this level. Not yet filtered: a
    /// node closed here can still be dropped later, and its parity with it.
    coded: Vec<(Cid, Vec<u8>)>,
}

/// From a leaf's recorded aggregate alone: can it have been closed by the hard
/// limit? A forced close needs `logical_len + cost(next entry) > MAX_LOGICAL`,
/// the largest entry costs 13 + MAX_KEY + MAX_INLINE, and a leaf's logical size
/// is at most HEADER + 13·count + bytes (what is stored for a value is never
/// longer than the value).
fn leaf_was_not_forced(agg: Agg) -> bool {
    let bound = (HEADER as u64)
        .saturating_add(agg.count.saturating_mul(13))
        .saturating_add(agg.bytes);
    bound <= (MAX_LOGICAL - (13 + MAX_KEY + MAX_INLINE)) as u64
}

fn rewrite_level<B: Blocks>(
    blocks: &B,
    root: &Cid,
    floor: u8,
    edits: &LevelEdits,
    overlay: &mut crate::store::Overlay<'_, B>,
    opts: Options,
) -> Result<LevelResult, ApplyError> {
    let rule = opts.rule;
    let mut res = LevelResult {
        new: Vec::new(),
        old: Vec::new(),
        whole: true,
        effective: vec![false; edits.len()],
        coded: Vec::new(),
    };
    // How much of `res.new` the overlay has already been told about. A branch's
    // parity members are the children this same rebuild just closed, so they
    // have to be readable the moment the level above asks for them.
    let mut synced = 0usize;
    let mut i = 0;
    let mut reached_end = false;
    // Smallest key of the node after the last one streamed.
    let mut resume_at: Option<Vec<u8>> = None;
    while i < edits.len() {
        let Some(mut cur) = LevelCursor::seek(blocks, root, floor, &edits[i].0)? else {
            // A level above the old root: nothing old to merge with.
            let mut chunker = LevelChunker::new(floor, rule);
            for (n, (key, body)) in edits.iter().enumerate() {
                if let Some(body) = body {
                    chunker
                        .push(key, body, overlay, &mut res.new)
                        .map_err(ApplyError::from_build)?;
                    sync(overlay, &res.new, &mut synced);
                    res.effective[n] = true;
                }
            }
            chunker
                .finish(overlay, &mut res.new)
                .map_err(ApplyError::from_build)?;
            res.coded.extend(chunker.take_coded());
            sync(overlay, &res.new, &mut synced);
            return Ok(res);
        };
        // A node closed by the hard limit ended because of the entry AFTER it.
        // If that entry is what this edit changes, the node before must be
        // re-chunked too — unless it is known not to have been force-closed.
        let last_streamed = res.old.last().map(|(id, _)| *id);
        if opts.recheck_neighbour && !cur.node().is_empty() && cur.node().key(0) == edits[i].0 {
            if let Some((prev, agg)) = cur.prev_info()? {
                // A run stops only when the chunker is clean, and it is clean only
                // after a CONTENT close (a forced close happens before the next
                // push, so a to-be-forced node is still open at the end of an old
                // node's entries). So if the node before is where the last run
                // stopped, it was not closed by the limit.
                let safe = Some(prev) == last_streamed || (floor == 0 && leaf_was_not_forced(agg));
                if !safe {
                    cur.retreat()?;
                }
            }
        }
        // Does this run start where the last one stopped (or at the start of
        // the level)? Told from keys already in hand, not by loading anything.
        let first_key = (!cur.node().is_empty()).then(|| cur.node().key(0));
        res.whole &= match &resume_at {
            None => cur.is_first(),
            Some(k) => Some(k) == first_key.as_ref(),
        };

        let mut chunker = LevelChunker::new(floor, rule);
        loop {
            let node = *cur.node();
            // This node is being replaced, so its groups are available to the
            // nodes that replace it: parity is a pure function of a group's
            // members, so a group that survives the rewrite needs no recoding
            // and its ids are already stored.
            chunker.replacing(&node);
            let upper = cur.next_min_key();
            res.old.push((
                cur.id(),
                if node.is_empty() {
                    Vec::new()
                } else {
                    node.key(0)
                },
            ));
            let mut j = 0;
            loop {
                let edit = edits
                    .get(i)
                    .filter(|(k, _)| upper.as_ref().is_none_or(|u| k < u));
                let have = (j < node.len()).then(|| crate::chunk::Entry::of(&node, j));
                match (have, edit) {
                    (None, None) => break,
                    (Some(e), Some((k, _))) if e.key < *k => {
                        chunker
                            .push(&e.key, &e.body, overlay, &mut res.new)
                            .map_err(ApplyError::from_build)?;
                        sync(overlay, &res.new, &mut synced);
                        j += 1;
                    }
                    (Some(e), None) => {
                        chunker
                            .push(&e.key, &e.body, overlay, &mut res.new)
                            .map_err(ApplyError::from_build)?;
                        sync(overlay, &res.new, &mut synced);
                        j += 1;
                    }
                    (have, Some((k, body))) => {
                        let same_key = have.as_ref().is_some_and(|e| e.key == *k);
                        let old_body = have.filter(|_| same_key).map(|e| e.body);
                        res.effective[i] = old_body.as_ref() != body.as_ref();
                        if let Some(body) = body {
                            chunker
                                .push(k, body, overlay, &mut res.new)
                                .map_err(ApplyError::from_build)?;
                            sync(overlay, &res.new, &mut synced);
                        }
                        if same_key {
                            j += 1;
                        }
                        i += 1;
                    }
                }
            }
            if upper.is_none() {
                chunker
                    .finish(overlay, &mut res.new)
                    .map_err(ApplyError::from_build)?;
                res.coded.extend(chunker.take_coded());
                sync(overlay, &res.new, &mut synced);
                reached_end = true;
                break;
            }
            // Clean here means: the chunker closed a node exactly where this old
            // node ended. What follows is what a rebuild would produce, unless
            // an edit says otherwise — and then the outer loop starts again
            // there. The next node is not loaded just to find that out.
            if chunker.is_clean() {
                // This chunker is abandoned here and the run resumes with a
                // fresh one, so what it coded has to come out now. The nodes
                // it closed are already in `res.new`; their parity would
                // otherwise be dropped on the floor, and the engine would
                // never learn it owed them.
                res.coded.extend(chunker.take_coded());
                resume_at = upper;
                break;
            }
            cur.advance()?;
        }
    }
    res.whole &= reached_end;
    Ok(res)
}

/// Ids of every node of the tree at `root` above `floor`.
/// Tell the overlay about every node closed since the last call.
fn sync<B: Blocks>(overlay: &mut crate::store::Overlay<'_, B>, new: &[Closed], synced: &mut usize) {
    for c in &new[*synced..] {
        overlay.put(c.cid, &c.bytes);
    }
    *synced = new.len();
}

fn nodes_above<B: Blocks>(blocks: &B, root: &Cid, floor: u8) -> Result<Vec<Cid>, ReadError> {
    let mut out = Vec::new();
    let mut todo = vec![(*root, Held::root(blocks, root)?)];
    while let Some((id, node)) = todo.pop() {
        if node.level() <= floor {
            continue;
        }
        out.push(id);
        // Saturating: `floor` is a caller's number, and "above floor + 1" at
        // 255 is simply nothing (freenet-prolly#55).
        if node.level() > floor.saturating_add(1) {
            for i in 0..node.len() {
                todo.push((node.child(i).0, node.open(blocks, i)?));
            }
        }
    }
    Ok(out)
}

/// Apply a key-sorted batch of edits to the tree at `root`.
///
/// New blocks go to `sink` only if the whole batch succeeds. An edit that
/// changes nothing (the same value again, a delete of an absent key) costs
/// nothing: same root, nothing emitted.
///
/// On [`ReadError::Need`] the caller fetches and calls again with the same
/// arguments, so `(root, edits)` must fit whatever the caller can keep between
/// calls; the library sets no byte limit of its own.
/// [`apply`], writing every block it emits back into `blocks`.
///
/// The loop this replaces is the same in every consumer, and getting it wrong —
/// dropping a value block, say — is silent until a read. A refused batch emits
/// nothing, so nothing is written and the store is left exactly as it was.
pub fn apply_into<B: BlocksMut>(
    blocks: &mut B,
    root: &Cid,
    edits: &[(Vec<u8>, Edit)],
) -> Result<Applied, ApplyError> {
    let mut emitted: Vec<(Cid, Vec<u8>)> = Vec::new();
    let applied = apply(&*blocks, root, edits, |c, b| emitted.push((c, b.to_vec())))?;
    for (c, b) in emitted {
        blocks.insert_block(c, &b);
    }
    Ok(applied)
}

pub fn apply<B: Blocks>(
    blocks: &B,
    root: &Cid,
    edits: &[(Vec<u8>, Edit)],
    sink: impl FnMut(Cid, &[u8]),
) -> Result<Applied, ApplyError> {
    apply_with(Options::default(), blocks, root, edits, sink)
}

/// [`apply`] with non-default [`Options`]; for comparison in tests.
pub fn apply_with<B: Blocks>(
    opts: Options,
    blocks: &B,
    root: &Cid,
    edits: &[(Vec<u8>, Edit)],
    mut sink: impl FnMut(Cid, &[u8]),
) -> Result<Applied, ApplyError> {
    // 1. Refuse a bad batch before anything is read.
    for (n, (key, edit)) in edits.iter().enumerate() {
        if n > 0 && edits[n - 1].0 >= *key {
            return Err(ApplyError::NotSorted);
        }
        if key.len() > MAX_KEY {
            return Err(ApplyError::KeyTooLong);
        }
        if matches!(edit, Edit::Put(v) if v.len() > MAX_VALUE) {
            return Err(ApplyError::ValueTooLong);
        }
    }
    // 2. The library owns the inline-or-reference decision, in one place:
    //    `Value::for_bytes`. A caller building the same tree by hand calls the
    //    same function, so an oracle cannot drift from the rule.
    // Fresh blocks first, the caller's store second: a branch's parity members
    // are the children this rebuild just closed, and those are not in the
    // caller's store until it inserts what the sink gave it.
    let mut overlay = crate::store::Overlay::new(Some(blocks));
    let mut values: Vec<Option<(Cid, &[u8])>> = Vec::with_capacity(edits.len());
    let mut level: LevelEdits = Vec::with_capacity(edits.len());
    for (key, edit) in edits {
        let (body, block) = match edit {
            Edit::Delete => (None, None),
            Edit::Put(v) => match Value::for_bytes(v) {
                (Value::Inline(b), _) => (Some(Body::Inline(b.to_vec())), None),
                (Value::Ref { cid, len }, block) => (Some(Body::Ref { cid, len }), block),
            },
        };
        values.push(block);
        level.push((key.clone(), body));
    }
    // The values this call is writing must be readable as parity members
    // before the leaf that references them is coded — they are not in the
    // caller's store yet, and will not be until it inserts what the sink gave
    // it. An OLD value that a leaf still references comes from the store.
    for (cid, bytes) in values.iter().flatten() {
        overlay.put(*cid, bytes);
    }
    // 3. Name every missing block the edits already point at, in one go.
    let mut need: Vec<Cid> = Vec::new();
    for (key, _) in edits {
        match LevelCursor::seek(blocks, root, 0, key) {
            Err(ReadError::Need(ids)) => {
                for id in ids {
                    if !need.contains(&id) && need.len() < MAX_NEED {
                        need.push(id);
                    }
                }
            }
            Err(e) => return Err(e.into()),
            Ok(_) => {}
        }
    }
    if !need.is_empty() {
        return Err(ReadError::Need(need).into());
    }

    // 4. Rewrite level by level.
    //
    // The level the old root sits on is the last one that EXISTS. Above it
    // there are no parents holding anything, which changes what has to be
    // passed up (see below).
    let old_root_level = Node::parse(
        blocks
            .get(root)
            .ok_or_else(|| ReadError::Need(vec![*root]))?,
    )
    .map_err(|e| ReadError::Corrupt(*root, e))?
    .level();
    let mut new_nodes: Vec<Closed> = Vec::new();
    // Every parity block coded anywhere in this rewrite. Not what gets
    // reported: a node closed at one level can still be dropped at the next,
    // and reporting its parity would have the engine pay PUTs for redundancy
    // over a node that is not in the tree. The kept nodes decide, below.
    let mut coded: Vec<(Cid, Vec<u8>)> = Vec::new();
    let mut old_ids: Vec<Cid> = Vec::new();
    let mut value_blocks: Vec<(Cid, &[u8])> = Vec::new();
    let mut new_root = *root;
    let mut floor: u8 = 0;
    loop {
        let r = rewrite_level(blocks, root, floor, &level, &mut overlay, opts)?;
        if floor == 0 {
            value_blocks = values
                .iter()
                .zip(&r.effective)
                .filter_map(|(v, used)| v.filter(|_| *used))
                .collect();
        }
        old_ids.extend(r.old.iter().map(|(id, _)| *id));
        coded.extend(r.coded);
        let mut done = false;
        if r.whole && r.new.len() <= 1 {
            // The lowest level with exactly one node is the root.
            let top = r.new.first().cloned().unwrap_or_else(empty_leaf);
            new_root = top.cid;
            old_ids.extend(nodes_above(blocks, root, floor)?);
            if r.new.is_empty() {
                new_nodes.push(top);
            }
            done = true;
        }
        let was: HashSet<Cid> = r.old.iter().map(|(id, _)| *id).collect();
        let now: HashSet<Cid> = r.new.iter().map(|n| n.cid).collect();
        let mut up: BTreeMap<Vec<u8>, Option<Body>> = BTreeMap::new();
        for (id, min_key) in &r.old {
            if !now.contains(id) {
                up.insert(min_key.clone(), None);
            }
        }
        // Normally only the DIFFERENCE goes up: a node whose bytes did not
        // change is already recorded by its parent, so re-stating it would just
        // rewrite the parent to say the same thing.
        //
        // That reasoning needs a parent. When this level is the old root's, the
        // level above does not exist yet, and if the rewrite ends with more than
        // one node the tree is about to grow one. Nothing holds ANY of these
        // nodes, so every one of them has to go up — including a node that
        // survived byte-identically, which is exactly what an append produces:
        // it fills a new node at the end and leaves everything before it alone.
        // Sending only the new sibling up built a parent with one child, and the
        // single-child-root rule then threw the rest of the tree away.
        let growing = floor == old_root_level && r.new.len() > 1;
        for n in &r.new {
            if growing || !was.contains(&n.cid) {
                let e = n.as_child();
                up.insert(e.key, Some(e.body));
            }
        }
        new_nodes.extend(r.new);
        if done || up.is_empty() {
            break;
        }
        level = up.into_iter().collect();
        floor = floor.checked_add(1).ok_or(BuildError::Overflow)?;
    }

    // 5. The root is the LOWEST level with exactly one node. A level can end up
    // with one node that no edit touched (everything around it was deleted);
    // the levels above then shrink to branches with a single child, which a
    // rebuild would never make. A real root branch has at least two children,
    // so: while the root is a single-child branch, the child is the root.
    // (Only the root: the last node of a lower level may hold one child.)
    loop {
        let bytes = match new_nodes.iter().find(|n| n.cid == new_root) {
            Some(n) => n.bytes.as_slice(),
            None => blocks
                .get(&new_root)
                .ok_or_else(|| ReadError::Need(vec![new_root]))?,
        };
        let node = Node::parse(bytes).map_err(|e| ReadError::Corrupt(new_root, e))?;
        if node.is_leaf() || node.len() != 1 {
            break;
        }
        let dropped = new_root;
        new_root = node.child(0).0;
        let before = new_nodes.len();
        new_nodes.retain(|n| n.cid != dropped);
        if new_nodes.len() == before {
            old_ids.push(dropped); // an old node, no longer part of the tree
        }
    }

    // 6. Success: hand over what is new, children before parents.
    let old: HashSet<Cid> = old_ids.iter().copied().collect();
    let mut sent: HashSet<Cid> = HashSet::new();
    for (cid, bytes) in value_blocks {
        if sent.insert(cid) {
            sink(cid, bytes);
        }
    }
    let mut kept: HashSet<Cid> = HashSet::new();
    for n in &new_nodes {
        kept.insert(n.cid);
        if !old.contains(&n.cid) && sent.insert(n.cid) {
            sink(n.cid, &n.bytes);
        }
    }
    let mut replaced: Vec<Cid> = Vec::new();
    for id in old_ids {
        if !kept.contains(&id) && !replaced.contains(&id) {
            replaced.push(id);
        }
    }
    // The parity the tree actually promises. Driven by the KEPT nodes' own
    // parity lists rather than by what the chunkers happened to code, which is
    // what makes both halves true at once: nothing reported that no node lists
    // (a wasted PUT over a node that was dropped), and nothing listed left
    // unreported except a reused group, whose parity is already out.
    //
    // An id can be listed by two nodes — two groups with the same members and
    // the same class have the same parity, because parity is a function of
    // exactly those two things — so it is reported once.
    let mut have: HashMap<Cid, Vec<u8>> = HashMap::new();
    for (id, bytes) in coded {
        have.entry(id).or_insert(bytes);
    }
    let mut parity: Vec<(Cid, Vec<u8>)> = Vec::new();
    let mut reported: HashSet<Cid> = HashSet::new();
    for n in &new_nodes {
        let node = Node::parse(&n.bytes).map_err(|e| ReadError::Corrupt(n.cid, e))?;
        for id in node.parity() {
            if let Some(bytes) = have.get(&id) {
                if reported.insert(id) {
                    parity.push((id, bytes.clone()));
                }
            }
        }
    }

    // The tripwire, not a safety net: `parity` is already correct either way.
    // If this is ever non-zero, a node this rewrite closed was dropped with
    // its parity, and the filter above stopped being a formality.
    #[cfg(any(test, feature = "testing"))]
    crate::chunk::source::tick_filtered(have.len() - parity.len());

    Ok(Applied {
        root: new_root,
        replaced,
        parity,
    })
}
