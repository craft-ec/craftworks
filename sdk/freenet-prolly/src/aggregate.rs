//! Counting a range without reading it.
//!
//! A branch records, for each child, what that child's subtree contains. So a
//! count over a million records can touch two root-to-leaf paths instead of a
//! million entries: every subtree that lies wholly inside the range contributes
//! its recorded aggregate and is never loaded.
//!
//! # Whose word the number rests on
//!
//! That is the whole point and the whole catch. A subtree inside the range is
//! never opened, so nothing checks that what its parent says about it is true.
//! The aggregate sits inside the parent, so it is authenticated — as what the
//! WRITER COMMITTED TO, which is a different claim from being correct.
//!
//! The two answers are different TYPES, and there is no conversion from one to
//! the other:
//!
//! - [`Claimed`] — fast, and believed. Fine for DISPLAY. A writer can inflate
//!   its own counts with junk entries anyway, and the claim is cheap to refute
//!   ([`fraud`] does it from two blocks).
//! - [`Verified`] — every node in range is read and checked against its parent.
//!   Anything that GATES must use this: quotas, metering, tier enforcement,
//!   totals rolled into a signed head. There the incentive runs the other way —
//!   to UNDER-report — and a believed aggregate is the attack.
//!
//! A proof of inclusion does not close the gap. A child's aggregate is already
//! inside the hashed parent; a Merkle proof proves exactly that, and no more. A
//! count independent of the writer's word needs every node in range read, by
//! construction. There is no third product.
//!
//! Stated once more, because it is the rule this whole library follows:
//! aggregates are believed for BUDGETING and for counts marked claimed, never
//! for content.

use crate::node::{Agg, Node, Value};
use crate::range::{children, in_range, Range, Span, MAX_NEED};
use crate::store::{Blocks, Held, ReadError};
use crate::{block_id, kind, Cid};

/// A count taken from what branches RECORD about subtrees nobody opened.
///
/// `Agg.bytes` is LOGICAL bytes: a key's full length plus the value's full
/// length, counting a referenced value at its real size rather than the 32
/// bytes of the reference that stands for it in the leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claimed(Agg);

/// A count where every node in the range was read and checked against its
/// parent. Same fields, same meaning of `bytes` — a different amount of trust,
/// which is why it is a different type.
/// ```
/// # use freenet_prolly::{aggregate::aggregate_verified, build::init, store::MemBlocks};
/// # let mut b = MemBlocks::default(); let root = init(&mut b);
/// let v = aggregate_verified(&b, &root, &Default::default()).unwrap();
/// assert_eq!(v.agg().count, 0);
/// ```
///
/// The field is private, so this is the ONLY way to get one. A `Verified` that
/// could be written by hand would be a `Claimed` with a better name:
///
/// ```compile_fail
/// # use freenet_prolly::{aggregate::Verified, node::Agg};
/// // no: only `aggregate_verified` may say a number was verified
/// let lie = Verified(Agg::default());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verified(Agg);

impl Claimed {
    /// The numbers, with whatever trust the type carries. Named rather than
    /// public so the type has to be said out loud at every use.
    pub fn agg(&self) -> Agg {
        self.0
    }
}

impl Verified {
    pub fn agg(&self) -> Agg {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AggError {
    Read(ReadError),
    /// The range carried a paging instruction. An aggregate of "a page" is a
    /// different question with a different answer, so it is refused rather than
    /// quietly answered as if the instruction were not there.
    NotWholeRange(&'static str),
}

impl From<ReadError> for AggError {
    fn from(e: ReadError) -> Self {
        AggError::Read(e)
    }
}

/// How much of the answer is taken on trust.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Trust {
    /// Believe a subtree that lies wholly inside the range.
    Claimed,
    /// Open everything and check it against its parent.
    Verified,
}

/// `{count, bytes}` of the entries in `r`, from the aggregates branches carry.
///
/// Reads two root-to-leaf paths, not the range. See the module doc for what
/// that costs in trust — and use [`aggregate_verified`] for anything that
/// gates.
pub fn aggregate<B: Blocks>(blocks: &B, root: &Cid, r: &Range) -> Result<Claimed, AggError> {
    Ok(Claimed(run(blocks, root, r, Trust::Claimed)?))
}

/// The same numbers, with every node in the range read and checked against its
/// parent. The oracle for [`aggregate`], and what a reader uses when the answer
/// matters.
pub fn aggregate_verified<B: Blocks>(
    blocks: &B,
    root: &Cid,
    r: &Range,
) -> Result<Verified, AggError> {
    Ok(Verified(run(blocks, root, r, Trust::Verified)?))
}

fn run<B: Blocks>(blocks: &B, root: &Cid, r: &Range, trust: Trust) -> Result<Agg, AggError> {
    whole_range(r)?;
    let node = Held::root(blocks, root)?;
    let mut out = Agg::default();
    let mut need = Vec::new();
    count(blocks, &node, r, trust, &mut out, &mut need)?;
    if !need.is_empty() {
        return Err(ReadError::Need(need).into());
    }
    Ok(out)
}

/// An aggregate answers about a whole range. Paging is a different question.
fn whole_range(r: &Range) -> Result<(), AggError> {
    let d = Range::default();
    if r.reverse {
        return Err(AggError::NotWholeRange("reverse"));
    }
    if r.after.is_some() {
        return Err(AggError::NotWholeRange("after"));
    }
    // The defaults mean "no opinion" — `Range::default()` and `Range::prefix`
    // carry them, and both must work here. Anything else is a paging request.
    //
    // Which makes this check DEFINITIONAL rather than absolute: "no opinion" is
    // defined as equal to the default rather than as the absence of a limit, so
    // if `Range`'s defaults ever change meaning, this moves with them.
    if r.max_entries != d.max_entries || r.max_bytes != d.max_bytes {
        return Err(AggError::NotWholeRange("a limit"));
    }
    Ok(())
}

fn count<'a, B: Blocks>(
    blocks: &'a B,
    node: &Held<'a>,
    r: &Range,
    trust: Trust,
    out: &mut Agg,
    need: &mut Vec<Cid>,
) -> Result<(), ReadError> {
    if node.is_leaf() {
        // The only place entries are counted one by one. A leaf is either an
        // edge of the range or, under `Verified`, everything.
        for i in 0..node.len() {
            let key = node.key(i);
            if !in_range(&key, r) {
                continue;
            }
            let vlen = match node.value(i) {
                Value::Inline(b) => b.len() as u64,
                Value::Ref { len, .. } => len as u64,
            };
            *out = out
                .checked_add(Agg {
                    count: 1,
                    bytes: key.len() as u64 + vlen,
                })
                .ok_or(ReadError::Mismatch(crate::block_id(kind::TREE_NODE, &[])))?;
        }
        return Ok(());
    }
    for c in children(node, r) {
        // A subtree wholly inside the range is the whole point: take what the
        // parent says and do not open it.
        if c.span == Span::Inside && trust == Trust::Claimed {
            *out = out.checked_add(c.agg).ok_or(ReadError::Mismatch(c.id))?;
            continue;
        }
        if blocks.get(&c.id).is_none() {
            // Asked for only once the child is actually wanted: an inside
            // subtree under `Claimed` returns above, so a count never so much
            // as asks the store whether those blocks are there.
            //
            // At most two per level are edges, so this stays small; the cap is
            // the same one the frontier uses.
            if need.len() < MAX_NEED && !need.contains(&c.id) {
                need.push(c.id);
            }
            continue;
        }
        // `Held::open` is what makes `Verified` mean anything: it refuses a
        // child whose level, first key or aggregate disagrees with its parent,
        // or whose keys run past the bound it inherits (freenet-prolly#52 —
        // before that, this counted `z` under `a` when `m` followed).
        let child = node.open(blocks, c.idx)?;
        count(blocks, &child, r, trust, out, need)?;
    }
    Ok(())
}

/// Do these two blocks refute each other?
///
/// A parent records three things about each child: its id, its first key, and
/// what its subtree adds up to. The child's own header states the last two. If
/// they disagree, the pair is a proof: both blocks are hash-keyed, so whoever
/// is shown them can check they are the blocks their ids say they are, and
/// nothing else is needed — no third block, no trust in whoever produced them.
/// `true` means *these two cannot both be right*, which is all a proof from two
/// blocks can ever establish; it does not say which of them is the liar.
///
/// It is `false` — never a panic — for anything that is not such a pair:
/// garbage, a leaf where a branch was expected, a block this parent does not
/// name, an empty node. **This function exists so a stranger can hand a keeper
/// or a client two blocks**, so every accessor below is reached only after the
/// shape that makes it valid has been checked. It is conservative by
/// construction: it returns `true` only when it can point at two concrete
/// values that disagree.
///
/// A child at the wrong level counts, and is the reason the level is compared
/// rather than assumed: a branch may only name children one level below it, so
/// a parent naming a block at any other level has recorded something that block
/// contradicts, exactly like a wrong aggregate.
pub fn fraud(parent: &[u8], child: &[u8]) -> bool {
    let (Ok(p), Ok(c)) = (Node::parse(parent), Node::parse(child)) else {
        return false;
    };
    // A leaf names no children, so it can hold no claim about this block.
    if p.is_leaf() {
        return false;
    }
    let id = block_id(kind::TREE_NODE, child);
    (0..p.len()).filter(|i| p.child(*i).0 == id).any(|i| {
        p.child(i).1 != c.agg()
            || !crate::node::one_level_below(p.level(), c.level())
            || (!c.is_empty() && p.key(i) != c.key(0))
    })
}
