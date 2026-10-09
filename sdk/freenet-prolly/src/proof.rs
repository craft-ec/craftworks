//! Checking one key against a root you trust, without holding the tree.
//!
//! A reader who has a signed head — a root and nothing else — can be handed a
//! proof and decide for itself whether a key is present, what its value is, or
//! that it is absent. No trust in whoever produced the proof: every block in it
//! is checked against the pointer that names it, and the chain of pointers ends
//! at the root the reader already trusted.
//!
//! # A proof is the path, and nothing cleverer
//!
//! A node's id hashes its WHOLE bytes, so nothing smaller than a node can be
//! checked against its parent's pointer. The proof is therefore the node bodies
//! on the root-to-leaf path — five nodes for a million entries.
//!
//! # `verify` is `get`, run over a store that holds only the proof
//!
//! This is the whole design. Build a block source from the proof, keyed the way
//! every block is keyed, and run the library's own reader against it:
//!
//! - a block that is not what its parent says it is cannot be FOUND, because
//!   the store is keyed by `block_id(TREE_NODE, bytes)` and tampering changes
//!   the id;
//! - a child whose level, first key or aggregate disagrees with its parent is
//!   refused by [`load_child`](crate::store::load_child), which the reader
//!   already calls;
//! - a proof missing a node on the path produces `Need`, which is an incomplete
//!   proof, never an answer.
//!
//! So there is no second verifier to drift from the reader. A change to how the
//! tree is read is a change to how proofs are checked, in the same edit.
//!
//! Canonicity is the part `get` cannot supply, and it is enforced here: the
//! blocks given are exactly the blocks read, with no repeats, in the format's
//! own order — **level descending, then first key ascending**, which for a path
//! is top down. Stated as a rule rather than as "whatever the reader visited",
//! so that reordering a traversal is not a silent format break and a verifier
//! written from this page in another language agrees with this one.
//!
//! That makes it **one proof per (root, PATH)**, not per (root, key): the key
//! is not in the proof and is not believed if it were. The verifier computes
//! the answer for the key IT asks about, from blocks authenticated against the
//! root IT trusts. So the same bytes are the canonical proof for every key
//! whose read takes that path — a feature, since one proof answers a key and
//! its neighbours.
//!
//! # A proof is bytes from a stranger
//!
//! So the SHAPE is checked before the bytes are: one parse and one hash decide
//! whether the rest is worth touching. The count is a `u16` and nothing in the
//! proof ties it to the tree, so a 65,000-node proof would otherwise be parsed
//! and hashed in full before being refused — a second of work on a phone, from
//! a message anyone can send. The root settles it: its level bounds how many
//! blocks a path against it can need.
//!
//! # What a proof CLAIMS
//!
//! Exactly this: **what the library's own `get` returns against this root**.
//! Not "the key is not in any tree" — a statement about one root, and only
//! because that root's bytes commit to everything the answer rests on.
//!
//! A key below the tree's minimum is the clearest case. `get` answers at the
//! root without descending, because the root's recorded first key is already
//! above the key being asked about — so the proof is ONE node, and it is
//! complete. That first key is the writer's commitment like every other field
//! in the node, hashed into the id the reader already trusted, so a one-node
//! proof of absence is as strong as a five-node one. A reader that expected
//! `height` blocks and saw one would be wrong to call it truncated.
//!
//! And a writer cannot profit by lying there. A parent key that misstates its
//! child's first key builds a tree `load_child` refuses for every key in that
//! child: the lie buys a consistent "absent" and breaks the writer's own tree
//! to get it.
//!
//! # What is out of scope, and must be named
//!
//! **The chain above the root.** A proof is checked against a root, and is only
//! as fresh as the root it is checked against. Where that root came from —
//! device head Register ← identity entry ← directory ← signed global head — is
//! somebody else's problem and a real one; nothing here says a root is current.
//!
//! **Sealed domains cannot be proven to outsiders at all.** Node bodies are
//! ciphertext there, so a reader who cannot decrypt them cannot check a path,
//! and no proof in this module changes that.
//!
//! # What a proof does NOT tell you
//!
//! A proof answers one question against one root. Everything below is outside
//! it, and a reader that forgets so is trusting something it has not checked:
//!
//! - **Whether the root is current.** A proof is exactly as fresh as the head
//!   it is checked against; nothing here says that head is the latest one.
//!   Where the root came from — device head Register ← identity entry ←
//!   directory ← signed global head — is somebody else's problem and a real
//!   one.
//! - **What a person's other devices hold.** One root is one tree. An identity
//!   with several device heads has several, and a proof about one says nothing
//!   about the rest.
//! - **The bytes behind a referenced value**, unless the proof carries them. A
//!   proof of `Value::Ref` authenticates an id and a length; the bytes are a
//!   separate fetch, and the returned type says which you were given.
//! - **Any COUNT.** [`verify_aggregate`] returns
//!   [`Claimed`](crate::aggregate::Claimed): the writer's number,
//!   authenticated as the writer's. A proof cannot make a count true.
//! - **Anything in a Bag.** A bag asserts only that names met a price; its
//!   `count` and `full` are claims a stranger can buy, and no proof changes
//!   that.
//! - **A sealed domain, at all.** Node bodies are ciphertext there, so a reader
//!   who cannot decrypt them cannot check a path. There is no proof to offer an
//!   outsider.
//!
//! # What a proof cannot do
//!
//! It authenticates what the writer COMMITTED TO. It cannot upgrade that into
//! agreement with reality: [`verify_aggregate`] returns
//! [`Claimed`](crate::aggregate::Claimed) and there is no way to get a
//! `Verified` out of a proof, because a count over subtrees nobody opened is
//! exactly what a proof cannot check (see [`crate::aggregate`]).

use crate::aggregate::{aggregate, AggError, Claimed};
use crate::node::{Node, Value, MAX_NODE, MAX_VALUE};
use crate::range::{bounds_are_empty, range, PageEnd, Range};
use crate::read::get;
use crate::store::{Blocks, ReadError};
use crate::{block_id, kind, Cid};
use std::cell::RefCell;

/// The proof format's magic. Bumped if the encoding changes.
pub const MAGIC: &[u8; 4] = b"PP01";

/// The node bodies on the path from a root to one key, root first.
///
/// Optionally the raw bytes of a referenced value, so a proof about a key whose
/// value lives outside the leaf can carry the value too.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Proof {
    /// Root first, exactly the path a read takes.
    pub nodes: Vec<Vec<u8>>,
    /// The bytes behind a `Value::Ref`, if the prover chose to include them.
    ///
    /// Carrying it or not gives two different CLAIMS, not two spellings of
    /// one: without it the proof says "the value with this id and length",
    /// with it "these bytes". Both are canonical for the claim they make, and
    /// the returned [`Proven`] says which was proved.
    pub value: Option<Vec<u8>>,
}

/// What a verified proof says about the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Proven<'a> {
    /// The key is in the tree with this value. A `Ref` means the proof
    /// authenticated the value's id and length but did not carry its bytes —
    /// the type says which, so a caller cannot mistake one for the other.
    Present(Value<'a>),
    /// The key is not in the tree. A statement about THIS root and no other.
    Absent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProofError {
    /// Not a proof: wrong magic, truncated, a length that runs off the end.
    Malformed(&'static str),
    /// A block in the proof is not a node.
    NotANode,
    /// The proof does not contain the whole path — the read ran out of blocks.
    /// This is what an absence proof "made" by dropping the last node becomes.
    Incomplete,
    /// A block in the proof was never read: it is not on the path, so the proof
    /// is not the canonical one for this key.
    Extra,
    /// The proof's blocks are not in the order a read visits them, or one is
    /// repeated.
    OutOfOrder,
    /// The first block is not the root being proved against.
    WrongRoot,
    /// A block is not what its parent says it is.
    Mismatch(Cid),
    /// The carried value is not the one the leaf names — or is carried at all
    /// where there is nothing for it to prove, because the leaf holds its value
    /// inline or the key is absent. An unchecked trailer would mean two
    /// different byte strings verifying to the same answer, which is the
    /// canonicity this format is supposed to have.
    BadValue,
    /// The proof is for a different range than the one being verified.
    WrongRange,
    /// The proof claims more blocks than a proof against this root could
    /// possibly need, or a block larger than a block can be. Refused before
    /// the work of parsing and hashing them is done — see [`verify`].
    TooLarge(&'static str),
    /// A request a proof cannot be made for.
    Unsupported(&'static str),
    /// The page is short because a block was missing, so the listing is not
    /// complete for the span it claims. The lie a gateway would tell.
    NotComplete,
}

impl std::fmt::Display for ProofError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProofError::Malformed(w) => write!(f, "not a proof: {w}"),
            ProofError::NotANode => write!(f, "a block in the proof is not a node"),
            ProofError::Incomplete => write!(f, "the proof does not contain the whole path"),
            ProofError::Extra => write!(f, "the proof carries a block that is not on the path"),
            ProofError::OutOfOrder => write!(f, "the proof's blocks are not in read order"),
            ProofError::WrongRoot => write!(f, "the proof does not start at this root"),
            ProofError::Mismatch(_) => write!(f, "a block is not what its parent says it is"),
            ProofError::BadValue => write!(f, "the carried value is not the one the leaf names"),
            ProofError::WrongRange => write!(f, "the proof is for a different range"),
            ProofError::TooLarge(w) => write!(f, "the proof is impossibly large: {w}"),
            ProofError::Unsupported(w) => write!(f, "a proof cannot be made for {w}"),
            ProofError::NotComplete => {
                write!(f, "the listing is missing entries it should contain")
            }
        }
    }
}

impl std::error::Error for ProofError {}

// ---------------------------------------------------------------------------
// encoding
// ---------------------------------------------------------------------------

impl Proof {
    /// `"PP01" ‖ count u16 ‖ (len u32 ‖ bytes)* ‖ value_len u32 ‖ value`,
    /// little-endian like every other length in this crate.
    ///
    /// `value_len` is `u32::MAX` when no value is carried, so "no value" and
    /// "an empty value" are different encodings rather than the same one.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::from(&MAGIC[..]);
        out.extend_from_slice(&(self.nodes.len() as u16).to_le_bytes());
        for n in &self.nodes {
            out.extend_from_slice(&(n.len() as u32).to_le_bytes());
            out.extend_from_slice(n);
        }
        match &self.value {
            Some(v) => {
                out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                out.extend_from_slice(v);
            }
            None => out.extend_from_slice(&u32::MAX.to_le_bytes()),
        }
        out
    }

    /// Read a proof from bytes a stranger sent. Every length is checked against
    /// what is left before it is used.
    pub fn decode(bytes: &[u8]) -> Result<Proof, ProofError> {
        let mut r = Reader { b: bytes, at: 0 };
        if r.take(4)? != MAGIC {
            return Err(ProofError::Malformed("magic"));
        }
        let count = u16::from_le_bytes(r.take(2)?.try_into().expect("2 bytes"));
        let mut nodes = Vec::new();
        for _ in 0..count {
            let len = u32::from_le_bytes(r.take(4)?.try_into().expect("4 bytes"));
            // A block cannot be larger than a block. Checked before the bytes
            // are copied, so a lying length costs nothing.
            if len as usize > MAX_NODE {
                return Err(ProofError::TooLarge("a node"));
            }
            #[cfg(test)]
            work::tick_copied();
            nodes.push(r.take(len as usize)?.to_vec());
        }
        let vlen = u32::from_le_bytes(r.take(4)?.try_into().expect("4 bytes"));
        let value = if vlen == u32::MAX {
            None
        } else {
            if vlen as usize > MAX_VALUE {
                return Err(ProofError::TooLarge("a value"));
            }
            Some(r.take(vlen as usize)?.to_vec())
        };
        if r.at != bytes.len() {
            return Err(ProofError::Malformed("trailing bytes"));
        }
        Ok(Proof { nodes, value })
    }

    /// What this proof costs to send.
    pub fn bytes(&self) -> usize {
        self.encode().len()
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ProofError> {
        let end = self
            .at
            .checked_add(n)
            .ok_or(ProofError::Malformed("length"))?;
        if end > self.b.len() {
            return Err(ProofError::Malformed("truncated"));
        }
        let out = &self.b[self.at..end];
        self.at = end;
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// proving
// ---------------------------------------------------------------------------

/// The blocks a read touched, in the order it touched them.
struct Recording<'a, B: Blocks> {
    inner: &'a B,
    read: RefCell<Vec<(Cid, Vec<u8>)>>,
}

impl<B: Blocks> Blocks for Recording<'_, B> {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        let got = self.inner.get(cid);
        if let Some(b) = got {
            let mut r = self.read.borrow_mut();
            if !r.iter().any(|(c, _)| c == cid) {
                r.push((*cid, b.to_vec()));
            }
        }
        got
    }
}

/// A proof that `key` is present with its value, or absent, under `root`.
///
/// The path is whatever the reader actually reads — recorded, not
/// reconstructed, so a proof cannot disagree with the read it is meant to
/// stand for.
pub fn prove<B: Blocks>(blocks: &B, root: &Cid, key: &[u8]) -> Result<Proof, ReadError> {
    let rec = Recording {
        inner: blocks,
        read: RefCell::default(),
    };
    get(&rec, root, key)?;
    let nodes = rec.read.into_inner().into_iter().map(|(_, b)| b).collect();
    Ok(Proof { nodes, value: None })
}

/// The same, carrying the bytes of a referenced value when there is one.
pub fn prove_with_value<B: Blocks>(blocks: &B, root: &Cid, key: &[u8]) -> Result<Proof, ReadError> {
    let mut p = prove(blocks, root, key)?;
    if let Some(Value::Ref { cid, .. }) = get(blocks, root, key)? {
        let bytes = blocks.get(&cid).ok_or(ReadError::Need(vec![cid]))?;
        p.value = Some(bytes.to_vec());
    }
    Ok(p)
}

/// A proof of what `aggregate` answers for `range` under `root`.
///
/// Canonical per (root, range): the blocks the count reads depend on the range,
/// so a proof is about both.
pub fn prove_aggregate<B: Blocks>(
    blocks: &B,
    root: &Cid,
    range: &Range,
) -> Result<Proof, AggError> {
    let rec = Recording {
        inner: blocks,
        read: RefCell::default(),
    };
    aggregate(&rec, root, range)?;
    let mut nodes: Vec<Vec<u8>> = rec.read.into_inner().into_iter().map(|(_, b)| b).collect();
    // Recorded, then SORTED into the format's order — so the bytes of a proof
    // do not depend on the order `aggregate` happens to walk in, and a change
    // to that walk leaves every stored proof byte-identical.
    sort_canonical(&mut nodes);
    Ok(Proof { nodes, value: None })
}

/// The format's order: level descending, then first key ascending.
fn sort_canonical(nodes: &mut [Vec<u8>]) {
    nodes.sort_by_key(|b| {
        let n = Node::parse(b).expect("proving from parsed nodes");
        (std::cmp::Reverse(n.level()), n.key(0))
    });
}

// ---------------------------------------------------------------------------
// verifying
// ---------------------------------------------------------------------------

/// Check the proof's SHAPE before any of its bytes are parsed or hashed.
///
/// A proof is bytes from a stranger, and the count is a `u16` nothing ties to
/// the tree — so a 65,000-node proof would otherwise be parsed and hashed in
/// full before being refused, which on the device that matters (a phone, in
/// wasm) is a denial of service dressed as a proof. The root settles it: parse
/// the first node ONLY, check it is the root the reader trusts, read its level,
/// and refuse anything claiming more blocks than a path against that root could
/// possibly need.
fn shape<'a>(proof: &'a Proof, root: &Cid, per_level: usize) -> Result<Node<'a>, ProofError> {
    let first = proof.nodes.first().ok_or(ProofError::Incomplete)?;
    if block_id(kind::TREE_NODE, first) != *root {
        return Err(ProofError::WrongRoot);
    }
    let node = Node::parse(first).map_err(|_| ProofError::NotANode)?;
    let height = node.level() as usize + 1;
    if proof.nodes.len() > per_level * height + 1 {
        return Err(ProofError::TooLarge("more blocks than the tree is tall"));
    }
    Ok(node)
}

/// What verifying a proof COST, counted so a test can assert on work rather
/// than on a clock.
///
/// Per-thread, and that is the whole point. The harness runs a binary's tests
/// on many threads, so a process-global counter reports every thread's work to
/// every reader: `assert_eq!(hashed(), 0)` after a refused proof then passes
/// when the other tests happen to be elsewhere and fails when they are not,
/// and nothing in a green run says which it was. These counters lied exactly
/// that way once — a mutant in `apply.rs` was recorded KILLED by a cost test
/// here that has nothing to do with it, and because cargo stops at the first
/// failing test binary, the suite that should have judged the mutant never ran.
/// A false kill is worse than a survivor: it ends the investigation.
#[cfg(test)]
pub(crate) mod work {
    use core::cell::Cell;
    use std::sync::atomic::AtomicUsize;

    thread_local! {
        static HASHED: Cell<usize> = const { Cell::new(0) };
        static COPIED: Cell<usize> = const { Cell::new(0) };
    }

    /// Blocks parsed and hashed by `ProofStore::new` — the cost a hostile
    /// proof can impose.
    pub(crate) fn hashed() -> usize {
        HASHED.with(|n| n.get())
    }

    /// Node bodies COPIED out of the wire form by `Proof::decode` — the other
    /// half of the cost, and the one a shape check on the bytes avoids.
    pub(crate) fn copied() -> usize {
        COPIED.with(|n| n.get())
    }

    /// Both, because a test that measures one and forgets the other reads a
    /// number left over from whatever it did before.
    pub(crate) fn reset() {
        HASHED.with(|n| n.set(0));
        COPIED.with(|n| n.set(0));
        SHARED_HASHED.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn tick_hashed() {
        HASHED.with(|n| n.set(n.get() + 1));
        SHARED_HASHED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    pub(crate) fn tick_copied() {
        COPIED.with(|n| n.set(n.get() + 1));
    }

    /// The same count kept the OLD way, ticked beside the per-thread one and
    /// read by nothing but the control test. It exists so the control can show
    /// what the process-global form reports under the same load, rather than
    /// describing it.
    pub(crate) static SHARED_HASHED: AtomicUsize = AtomicUsize::new(0);
}

/// A store holding exactly the proof's blocks, keyed the way every block is
/// keyed, recording which ones were asked for.
struct ProofStore<'a> {
    blocks: Vec<(Cid, &'a [u8])>,
    read: RefCell<Vec<Cid>>,
}

impl<'a> ProofStore<'a> {
    fn new(p: &'a Proof) -> Result<Self, ProofError> {
        let mut blocks = Vec::with_capacity(p.nodes.len());
        for n in &p.nodes {
            #[cfg(test)]
            work::tick_hashed();
            // Parsed here so a block that is not a node is refused as such,
            // rather than reaching the reader as a missing block.
            Node::parse(n).map_err(|_| ProofError::NotANode)?;
            let id = block_id(kind::TREE_NODE, n);
            if blocks.iter().any(|(c, _)| *c == id) {
                return Err(ProofError::OutOfOrder);
            }
            blocks.push((id, n.as_slice()));
        }
        Ok(ProofStore {
            blocks,
            read: RefCell::default(),
        })
    }

    /// Canonicity, in two parts: the SET is exactly what was read, and the
    /// ORDER is the format's, not the traversal's.
    ///
    /// The order rule is stated normatively — **level descending, then first
    /// key ascending** — rather than "whatever the reader happened to visit".
    /// Tying it to a traversal would mean that reordering that walk turns every
    /// stored proof into `OutOfOrder`, with no test calling it a format break,
    /// and that a verifier written from this doc in another language would
    /// disagree with this one. For a key proof the two coincide: a path visits
    /// one node per level, top down.
    fn check_canonical(&self) -> Result<(), ProofError> {
        // Every block given was used: the set rule. `read` only ever records
        // ids that were found here, so this is also "nothing was read twice
        // and nothing is left over" — one check, not two saying the same thing
        // where only one of them could ever fail.
        let read = self.read.borrow();
        if !self.blocks.iter().all(|(id, _)| read.contains(id)) {
            return Err(ProofError::Extra);
        }
        // And the list is in the format's order: the order rule.
        // THE EMPTY TREE. Its root is the one node with no first key; it orders
        // as `(0, [])`. No "only when alone" condition: an empty block that is not
        // the root is either never read (`Extra`) or read as a child and refused
        // by `load_child` (`Mismatch`), both BEFORE this check — so such a
        // condition would be a guard no test can distinguish (prolly#53).
        let key_of = |b: &[u8]| -> Option<(u8, Vec<u8>)> {
            let n = Node::parse(b).ok()?;
            if n.is_empty() {
                return n.is_leaf().then(|| (0, Vec::new()));
            }
            Some((n.level(), n.key(0)))
        };
        let mut last: Option<(u8, Vec<u8>)> = None;
        for (_, b) in &self.blocks {
            let k = key_of(b).ok_or(ProofError::NotANode)?;
            if let Some(prev) = &last {
                // Level descending, then first key ascending.
                let ordered = prev.0 > k.0 || (prev.0 == k.0 && prev.1 < k.1);
                if !ordered {
                    return Err(ProofError::OutOfOrder);
                }
            }
            last = Some(k);
        }
        Ok(())
    }
}

impl<'a> Blocks for ProofStore<'a> {
    fn get(&self, cid: &Cid) -> Option<&[u8]> {
        let found = self.blocks.iter().find(|(c, _)| c == cid).map(|(_, b)| *b);
        if found.is_some() {
            let mut r = self.read.borrow_mut();
            if !r.contains(cid) {
                r.push(*cid);
            }
        }
        found
    }
}

fn read_err(e: ReadError) -> ProofError {
    match e {
        ReadError::Need(_) => ProofError::Incomplete,
        ReadError::Mismatch(c) => ProofError::Mismatch(c),
        ReadError::Corrupt(..) => ProofError::NotANode,
    }
}

/// Decide, from `proof` alone, whether `key` is under `root`.
///
/// Total: every input a stranger can send is an `Err` or an answer, never a
/// panic. An incomplete proof is [`ProofError::Incomplete`] and never
/// [`Proven::Absent`] — dropping the last node of a path does not turn a
/// present key into an absent one.
pub fn verify<'a>(root: &Cid, key: &[u8], proof: &'a Proof) -> Result<Proven<'a>, ProofError> {
    // Shape first: one parse and one hash decide whether the rest is worth
    // touching. A key proof is one node per level.
    shape(proof, root, 1)?;
    let store = ProofStore::new(proof)?;
    let found = get(&store, root, key).map_err(read_err)?;
    store.check_canonical()?;

    // The ANSWER is the reader's; the returned value has to borrow the proof
    // rather than the store that was just dropped, so it is read again from the
    // last block of the path — and the two are compared, so a difference
    // between them is an error rather than a silent divergence.
    let last = proof.nodes.last().ok_or(ProofError::Incomplete)?;
    let node = Node::parse(last).map_err(|_| ProofError::NotANode)?;
    let again = node
        .is_leaf()
        .then(|| node.search(key).ok().map(|i| node.value(i)))
        .flatten();
    if again != found {
        return Err(ProofError::OutOfOrder);
    }

    // A trailer is only meaningful for a value that lives in its own block.
    // Anywhere else it is unchecked bytes riding along, so it is refused.
    if proof.value.is_some() && !matches!(again, Some(Value::Ref { .. })) {
        return Err(ProofError::BadValue);
    }
    Ok(match again {
        None => Proven::Absent,
        Some(Value::Inline(b)) => Proven::Present(Value::Inline(b)),
        Some(Value::Ref { cid, len }) => match &proof.value {
            None => Proven::Present(Value::Ref { cid, len }),
            Some(v) => {
                // The carried bytes must be the value the leaf names — both the
                // id and the length, since the leaf records both.
                if v.len() != len as usize || block_id(kind::RAW, v) != cid {
                    return Err(ProofError::BadValue);
                }
                Proven::Present(Value::Inline(v))
            }
        },
    })
}

/// Decide, from `proof` alone, what `root` claims about `range`.
///
/// Returns [`Claimed`], never `Verified`: **a proof authenticates what the
/// writer committed to, it cannot upgrade a claim.** The count rests on
/// subtrees nobody opened; a proof shows the writer said so, and no proof can
/// show it is true.
pub fn verify_aggregate(root: &Cid, range: &Range, proof: &Proof) -> Result<Claimed, ProofError> {
    no_trailer(proof)?;
    // An aggregate reads at most the two edge paths, so two per level.
    shape(proof, root, 2)?;
    let store = ProofStore::new(proof)?;
    let got = aggregate(&store, root, range).map_err(|e| match e {
        AggError::Read(r) => read_err(r),
        AggError::NotWholeRange(_) => ProofError::WrongRange,
    })?;
    store.check_canonical()?;
    Ok(got)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::{apply_into, Edit};
    use crate::build::init;
    use crate::node::NodeBuilder;
    use crate::store::MemBlocks;

    /// Refusing a hostile proof must cost about what an honest one costs.
    ///
    /// Asserted on BLOCKS HASHED, not on a clock: the number is the work an
    /// attacker can impose, and it is the same number on a phone as here.
    /// Before the shape gate, a 65,003-node proof was parsed and hashed in full
    /// — 3.6 MiB and a second of work — before being refused as `Extra`.
    #[test]
    fn a_hostile_proof_is_refused_for_the_price_of_an_honest_one() {
        let mut blocks = MemBlocks::default();
        let root = init(&mut blocks);
        let edits: Vec<(Vec<u8>, Edit)> = (0..20_000u32)
            .map(|i| {
                (
                    format!("k/{i:08}").into_bytes(),
                    Edit::Put(vec![(i % 251) as u8; 120]),
                )
            })
            .collect();
        let root = apply_into(&mut blocks, &root, &edits).unwrap().root;
        let key = b"k/00010000".to_vec();
        let honest = prove(&blocks, &root, &key).unwrap();
        assert!(matches!(
            verify(&root, &key, &honest).unwrap(),
            Proven::Present(_)
        ));

        work::reset();
        let _ = verify(&root, &key, &honest);
        let honest_work = work::hashed();
        assert_eq!(honest_work, honest.nodes.len());

        // The honest path, then thousands of distinct, individually VALID
        // leaves — every one of which must be parsed and hashed before the old
        // code could notice they were not on the path.
        for n in [1_000usize, 10_000, 65_000] {
            let mut nodes = honest.nodes.clone();
            for i in 0..n {
                let mut b = NodeBuilder::leaf();
                b.push(format!("pad/{i:012}").as_bytes(), Value::Inline(&[0u8; 64]))
                    .unwrap();
                nodes.push(b.finish().unwrap());
            }
            let hostile = Proof { nodes, value: None };
            work::reset();
            let got = verify(&root, &key, &hostile);
            let work = work::hashed();
            assert!(got.is_err(), "{n} pad nodes must be refused");
            assert_eq!(
                work, 0,
                "{n} pad nodes cost {work} blocks hashed; the shape gate should \
                 have refused it after the root"
            );
        }
        println!("honest proof: {honest_work} blocks hashed; 65,000 padded nodes: 0");
    }

    /// The same, through the wire form: a lying length must not be believed.
    #[test]
    fn decode_refuses_impossible_lengths_before_copying() {
        let mut b = Vec::from(&MAGIC[..]);
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&(u32::MAX - 1).to_le_bytes()); // a node "larger than the world"
        assert_eq!(Proof::decode(&b), Err(ProofError::TooLarge("a node")));

        let mut b = Vec::from(&MAGIC[..]);
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&(u32::MAX - 1).to_le_bytes()); // a value the same
        assert_eq!(Proof::decode(&b), Err(ProofError::TooLarge("a value")));
    }
}

// ---------------------------------------------------------------------------
// range proofs: a LISTING is complete
// ---------------------------------------------------------------------------

/// A page must be bounded for a proof to exist: the shape gate needs a count
/// bound, and an unlimited page can be the whole tree.
///
/// Both limits at zero means "no limit" to [`range`], so it is refused here.
/// The defaults are 1024 entries and 256 KiB, so an ordinary caller never meets
/// this.
fn bounded(r: &Range) -> Result<usize, ProofError> {
    // `range` refuses `max_entries == 0` itself, so this looks redundant — it
    // is not. The bound has to exist BEFORE the proof is parsed: without it
    // there is nothing to gate the shape on, and a padded proof would be
    // parsed and hashed in full before `range` ever ran to refuse the request.
    //
    // A FINITE entry limit specifically. A page limited only by bytes has no
    // leaf count derivable from the root — an entry's minimum size is not a
    // number this format wants to depend on — so there would be nothing to
    // gate the cost of refusing on.
    if r.max_entries == 0 {
        return Err(ProofError::Unsupported("a page with no entry limit"));
    }
    Ok(r.max_entries)
}

/// A proof that a page of `range` is COMPLETE: the entries it contains are
/// every entry under `root` in the span it covers.
///
/// This is what a gateway can lie about. One key is checkable with [`prove`];
/// a LISTING — "here are the posts under `d/post/`" — is only checkable if
/// omission is detectable, and it is: drop a leaf and the scan stops early and
/// names what is missing; drop an entry from a leaf and the leaf's id changes,
/// so the block is not found at all.
pub fn prove_range<B: Blocks>(blocks: &B, root: &Cid, r: &Range) -> Result<Proof, ProofError> {
    bounded(r)?;
    let rec = Recording {
        inner: blocks,
        read: RefCell::default(),
    };
    let page = range(&rec, root, r).map_err(|e| match e {
        crate::range::RangeError::Read(e) => read_err(e),
        _ => ProofError::Unsupported("this range"),
    })?;
    if page.end == PageEnd::Blocked {
        // The prover itself could not complete the page.
        return Err(ProofError::Incomplete);
    }
    let mut nodes: Vec<Vec<u8>> = rec.read.into_inner().into_iter().map(|(_, b)| b).collect();
    sort_canonical(&mut nodes);
    Ok(Proof { nodes, value: None })
}

/// `Proof::value` carries the bytes behind ONE referenced value, which only a
/// point proof names. On a range or aggregate proof nothing reads it, so it is
/// unauthenticated bytes riding along: refused, FIRST, so the decoded door and
/// the wire door cannot disagree about it (the wire door already answers
/// `Extra` for an empty query carrying one).
fn no_trailer(proof: &Proof) -> Result<(), ProofError> {
    if proof.value.is_some() {
        return Err(ProofError::Extra);
    }
    Ok(())
}

/// Check a page against a root, from the proof alone.
///
/// Completeness rests on HOW the page ended — `Limit`, `EndOfRange` or
/// `EndOfTree`, never `Blocked` — and not on `need` being empty. See #37: a
/// scan that could not fetch a block could report an empty `need` and no
/// entries, and a verifier reading that as "complete" would accept a forged
/// reverse continuation as an empty listing.
///
/// Returns the page the proof establishes. **The claim is exactly: under this
/// root, the entries of `r` from its start up to `page.next` are these and no
/// others.** A continuation page proves its own span and says nothing about the
/// pages before it — each page carries its own proof.
///
/// Canonical **for** the (root, `Range`-with-limits) it was produced for: one
/// byte string per question. That is not the same as "no other question can be
/// answered from it" — exactly as a key proof is one proof per (root, PATH) and
/// answers for every key on that path, a page proof's leaves can answer for a
/// slightly larger limit or a slightly later start, and those answers are true
/// for their own spans because `next` moves with them. What never happens is a
/// question getting a different answer from the tree's: anything needing blocks
/// the proof does not carry is refused rather than guessed at.
pub fn verify_range(root: &Cid, r: &Range, proof: &Proof) -> Result<ProvenPage, ProofError> {
    no_trailer(proof)?;
    let per_page = bounded(r)?;
    // A question whose BOUNDS cannot hold a key is answered by the verifier
    // itself, from its own range — nothing in the proof is consulted, because
    // nothing in a proof could change the answer. This is the last page of a
    // paged listing: `after` has reached the far bound, `range` answers
    // EndOfRange before reading anything, so the honest proof has ZERO blocks.
    // Refusing it (as this did) makes a light client paging a feed to its end
    // see its final page rejected and conclude the gateway lied.
    //
    // The canonical proof for such a question is the EMPTY one, so blocks
    // attached to it are `Extra`.
    if bounds_are_empty(r) {
        if !proof.nodes.is_empty() {
            return Err(ProofError::Extra);
        }
        return Ok(ProvenPage {
            entries: Vec::new(),
            next: None,
        });
    }
    // Shape first, as ever: the root's level bounds the two edge paths, the
    // entry limit bounds the leaves, and both are known from the first block.
    //
    // The bound is deliberately LOOSE — a 1,024-entry page is about 78 blocks
    // against a bound near 1,030 — because it exists to cap the cost of
    // REFUSING, not to describe a proof anyone would send.
    let height = node_height(proof, root)?;
    shape_bounded(proof, root, 2 * height + per_page)?;
    let store = ProofStore::new(proof)?;
    let page = range(&store, root, r).map_err(|e| match e {
        crate::range::RangeError::Read(e) => read_err(e),
        _ => ProofError::Unsupported("this range"),
    })?;
    // The one check that makes this a COMPLETENESS proof, and it asks the SCAN
    // rather than inspecting `need`: a page is complete only if it ended by
    // running out of range, out of tree, or at its limit. `Blocked` means a
    // block was missing — whatever `need` looks like. #37 was exactly this: a
    // scan that could not fetch a block could end with `need` empty and no
    // entries, and an empty `need` read as "complete".
    if page.end == PageEnd::Blocked {
        return Err(ProofError::NotComplete);
    }
    debug_assert!(
        page.need.is_empty(),
        "a page that scanned to its end needs nothing"
    );
    store.check_canonical()?;
    Ok(ProvenPage {
        entries: page
            .entries
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    match v {
                        Value::Inline(b) => ProvenValue::Inline(b.to_vec()),
                        Value::Ref { cid, len } => ProvenValue::Ref {
                            cid: *cid,
                            len: *len,
                        },
                    },
                )
            })
            .collect(),
        next: page.next.clone(),
    })
}

/// A page a proof establishes, owned.
///
/// Owned rather than borrowed because the store that checked the blocks is
/// dropped when verification finishes, and a page is at most `max_entries`
/// entries the caller asked for anyway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenPage {
    pub entries: Vec<(Vec<u8>, ProvenValue)>,
    /// Where the proved span ends. The claim is about `[start of r, next)`.
    pub next: Option<Vec<u8>>,
}

/// A value as a proven page carries it: the bytes, or the reference the leaf
/// holds. The same two claims as [`Proven`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvenValue {
    Inline(Vec<u8>),
    Ref { cid: Cid, len: u32 },
}

/// The root's height, from the first block only.
fn node_height(proof: &Proof, root: &Cid) -> Result<usize, ProofError> {
    let first = proof.nodes.first().ok_or(ProofError::Incomplete)?;
    if block_id(kind::TREE_NODE, first) != *root {
        return Err(ProofError::WrongRoot);
    }
    let n = Node::parse(first).map_err(|_| ProofError::NotANode)?;
    Ok(n.level() as usize + 1)
}

/// The shape gate with an explicit bound, for proofs whose size is not one node
/// per level.
fn shape_bounded<'a>(
    proof: &'a Proof,
    root: &Cid,
    max_nodes: usize,
) -> Result<Node<'a>, ProofError> {
    let first = proof.nodes.first().ok_or(ProofError::Incomplete)?;
    if block_id(kind::TREE_NODE, first) != *root {
        return Err(ProofError::WrongRoot);
    }
    if proof.nodes.len() > max_nodes {
        return Err(ProofError::TooLarge("more blocks than the page can need"));
    }
    Node::parse(first).map_err(|_| ProofError::NotANode)
}

// ---------------------------------------------------------------------------
// refusing without allocating
// ---------------------------------------------------------------------------

/// Read the proof's shape out of its wire bytes without copying any of it.
///
/// `decode` copies every node before anything can look at the root, so a 3.4
/// MiB proof costs 3.4 MiB of copying to refuse — the shape gate stops the
/// hashing but not the allocation. These read the first node in place, so the
/// price of refusing stops depending on what a stranger attached.
fn peek_first(bytes: &[u8]) -> Result<(usize, &[u8]), ProofError> {
    let mut r = Reader { b: bytes, at: 0 };
    if r.take(4)? != MAGIC {
        return Err(ProofError::Malformed("magic"));
    }
    let count = u16::from_le_bytes(r.take(2)?.try_into().expect("2 bytes")) as usize;
    let len = u32::from_le_bytes(r.take(4)?.try_into().expect("4 bytes")) as usize;
    if len > MAX_NODE {
        return Err(ProofError::TooLarge("a node"));
    }
    Ok((count, r.take(len)?))
}

/// The count a proof against `root` may claim, decided from the first node
/// alone — without decoding the rest.
fn shape_of_bytes(
    bytes: &[u8],
    root: &Cid,
    per_level: usize,
    extra: usize,
) -> Result<(), ProofError> {
    let (count, first) = peek_first(bytes)?;
    if block_id(kind::TREE_NODE, first) != *root {
        return Err(ProofError::WrongRoot);
    }
    let node = Node::parse(first).map_err(|_| ProofError::NotANode)?;
    let height = node.level() as usize + 1;
    if count > per_level * height + extra + 1 {
        return Err(ProofError::TooLarge("more blocks than the tree is tall"));
    }
    Ok(())
}

/// [`verify`] from the wire form, refusing an impossible proof before it is
/// copied.
///
/// The answer is owned, for the same reason [`ProvenPage`] is: the checking
/// borrows the decoded proof, which this call owns. There is still exactly one
/// verifier — this decodes, then calls [`verify`].
pub fn verify_bytes(root: &Cid, key: &[u8], bytes: &[u8]) -> Result<ProvenOwned, ProofError> {
    shape_of_bytes(bytes, root, 1, 0)?;
    let proof = Proof::decode(bytes)?;
    Ok(match verify(root, key, &proof)? {
        Proven::Absent => ProvenOwned::Absent,
        Proven::Present(Value::Inline(b)) => ProvenOwned::Present(ProvenValue::Inline(b.to_vec())),
        Proven::Present(Value::Ref { cid, len }) => {
            ProvenOwned::Present(ProvenValue::Ref { cid, len })
        }
    })
}

/// [`Proven`], owned — what [`verify_bytes`] answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvenOwned {
    Present(ProvenValue),
    Absent,
}

/// [`verify_range`] from the wire form, refusing before copying.
pub fn verify_range_bytes(root: &Cid, r: &Range, bytes: &[u8]) -> Result<ProvenPage, ProofError> {
    let per_page = bounded(r)?;
    // BEFORE the shape gate, because that gate reads the root's level out of a
    // first block and an empty proof has none. This is the WIRE door — what a
    // light client and a browser call — so an empty final page refused here is
    // refused where it matters, whatever `verify_range` does with an
    // already-decoded proof. Fixing one entry point and not the other left the
    // bug exactly where the users are.
    if bounds_are_empty(r) {
        // Compared as BYTES against the one canonical encoding: nothing is
        // decoded, nothing is hashed, and anything else is Extra.
        if bytes != empty_proof_bytes() {
            return Err(ProofError::Extra);
        }
        return Ok(ProvenPage {
            entries: Vec::new(),
            next: None,
        });
    }
    shape_of_bytes(bytes, root, 2, per_page)?;
    verify_range(root, r, &Proof::decode(bytes)?)
}

/// The one encoding of the empty proof — the canonical answer to a question
/// whose bounds cannot hold a key.
fn empty_proof_bytes() -> Vec<u8> {
    Proof::default().encode()
}

#[cfg(test)]
mod range_tests {
    use super::*;
    use crate::apply::{apply_into, Edit};
    use crate::build::init;
    use crate::node::NodeBuilder;
    use crate::store::MemBlocks;

    fn tree() -> (MemBlocks, Cid, Vec<Vec<u8>>) {
        let mut blocks = MemBlocks::default();
        let root = init(&mut blocks);
        let edits: Vec<(Vec<u8>, Edit)> = (0..20_000u32)
            .map(|i| {
                (
                    format!("k/{i:08}").into_bytes(),
                    Edit::Put(vec![(i % 251) as u8; 120]),
                )
            })
            .collect();
        let root = apply_into(&mut blocks, &root, &edits).unwrap().root;
        let keys = edits.into_iter().map(|(k, _)| k).collect();
        (blocks, root, keys)
    }

    fn padded(honest: &Proof, n: usize) -> Proof {
        let mut nodes = honest.nodes.clone();
        for i in 0..n {
            let mut b = NodeBuilder::leaf();
            b.push(format!("pad/{i:012}").as_bytes(), Value::Inline(&[0u8; 64]))
                .unwrap();
            nodes.push(b.finish().unwrap());
        }
        Proof { nodes, value: None }
    }

    /// The cost of refusing a padded LISTING proof, in blocks hashed and node
    /// bodies copied — including for a range with no limit, where the bound
    /// has to come from somewhere before anything is parsed.
    #[test]
    fn a_padded_range_proof_costs_nothing_to_refuse() {
        let (blocks, root, keys) = tree();
        let r = Range {
            lo: std::ops::Bound::Included(keys[500].clone()),
            max_entries: 50,
            ..Range::default()
        };
        let honest = prove_range(&blocks, &root, &r).unwrap();
        work::reset();
        verify_range(&root, &r, &honest).unwrap();
        let honest_work = work::hashed();
        assert_eq!(honest_work, honest.nodes.len());

        let big = padded(&honest, 20_000);
        work::reset();
        assert!(matches!(
            verify_range(&root, &r, &big),
            Err(ProofError::TooLarge(_))
        ));
        assert_eq!(work::hashed(), 0, "padding must not be hashed");

        // With NO entry limit there is no bound to derive, so it must be
        // refused before the proof is touched — not after `range` declines.
        let unlimited = Range {
            max_entries: 0,
            ..r.clone()
        };
        work::reset();
        assert!(matches!(
            verify_range(&root, &unlimited, &big),
            Err(ProofError::Unsupported(_))
        ));
        assert_eq!(
            work::hashed(),
            0,
            "an unlimited range must be refused before the proof is parsed"
        );

        // And from the wire: refused before the bytes are copied.
        let wire = big.encode();
        work::reset();
        assert!(matches!(
            verify_range_bytes(&root, &r, &wire),
            Err(ProofError::TooLarge(_))
        ));
        assert_eq!(
            work::copied(),
            0,
            "{} B of padding was copied before being refused",
            wire.len()
        );
        println!(
            "listing proof: honest {honest_work} blocks; 20,000 pad nodes ({} B) cost 0 hashed, 0 copied",
            wire.len()
        );
    }

    /// An empty-bounds question is answered, or refused, without decoding or
    /// hashing anything — including at the wire door, which is the one a light
    /// client calls.
    #[test]
    fn an_empty_question_costs_nothing_at_either_door() {
        let (blocks, root, keys) = tree();
        // Reverse, resumed exactly at its lower bound: the bounds are empty.
        let q = Range {
            lo: std::ops::Bound::Included(keys[100].clone()),
            hi: std::ops::Bound::Included(keys[300].clone()),
            reverse: true,
            after: Some(keys[100].clone()),
            max_entries: 20,
            ..Range::default()
        };
        let honest = prove_range(&blocks, &root, &q).unwrap();
        assert!(honest.nodes.is_empty());
        let wire = honest.encode();

        work::reset();
        assert!(verify_range_bytes(&root, &q, &wire)
            .unwrap()
            .entries
            .is_empty());
        assert_eq!(work::hashed(), 0);
        assert_eq!(work::copied(), 0);

        // A padded proof aimed at an empty question: refused without decoding.
        let padded = {
            let real = prove_range(
                &blocks,
                &root,
                &Range {
                    after: None,
                    ..q.clone()
                },
            )
            .unwrap();
            padded_proof(&real, 5_000).encode()
        };
        assert!(padded.len() > 500_000);
        work::reset();
        assert_eq!(
            verify_range_bytes(&root, &q, &padded),
            Err(ProofError::Extra)
        );
        assert_eq!(work::hashed(), 0, "padding was hashed");
        assert_eq!(
            work::copied(),
            0,
            "{} B was copied to refuse an empty question",
            padded.len()
        );
        println!("empty question: answered and refused at 0 hashed, 0 copied");
    }

    fn padded_proof(honest: &Proof, n: usize) -> Proof {
        padded(honest, n)
    }

    /// A prover cannot pass off a page it could not complete itself.
    #[test]
    fn a_prover_that_cannot_complete_the_page_refuses() {
        let (blocks, root, keys) = tree();
        let r = Range {
            lo: std::ops::Bound::Included(keys[500].clone()),
            max_entries: 50,
            ..Range::default()
        };
        assert!(prove_range(&blocks, &root, &r).is_ok());

        // A store with only the root: the scan cannot finish the page.
        let mut partial = MemBlocks::default();
        partial.insert(root, blocks.get(&root).unwrap());
        assert_eq!(
            prove_range(&partial, &root, &r),
            Err(ProofError::Incomplete),
            "a prover missing blocks must refuse, not ship a short page"
        );
    }
}

/// The cost counters are per-thread, and this is what that buys.
///
/// A process-global counter reports the work of every thread to every reader,
/// so a cost assertion over it passes when the other tests happen to be
/// elsewhere and fails when they are not — and a green run never says which.
/// Here that is not left to luck: two barriers make the failure DETERMINISTIC.
/// Every thread waits until all of them are ready, does its own fixed amount of
/// counted work, and waits again until all of them have finished. Only then
/// does it read. A per-thread counter reads exactly its own `PER_THREAD`; a
/// shared one reads `THREADS * PER_THREAD`, every time, because all the work is
/// provably done before any read happens.
///
/// The shared reading is not described, it is TAKEN: `work::SHARED_HASHED` is
/// ticked beside the per-thread counter and read only here. So this test states
/// what the old form would have reported under the same load, rather than
/// asking the reader to believe it.
#[cfg(test)]
mod counter_scope {
    use super::*;
    use crate::apply::{apply_into, Edit};
    use crate::build::init;
    use crate::store::MemBlocks;
    use std::sync::atomic::Ordering;
    use std::sync::Barrier;

    const THREADS: usize = 4;
    /// Entries per thread's fixture. Big enough that the tree has a branch
    /// level, so the proof is more than one node and `mine == n` is a real
    /// equality rather than 1 == 1.
    const ENTRIES: usize = 2_000;

    #[test]
    fn a_cost_counter_reports_this_threads_work_and_no_other_threads() {
        let ready = Barrier::new(THREADS);
        let measured = Barrier::new(THREADS);
        std::thread::scope(|s| {
            for _ in 0..THREADS {
                s.spawn(|| {
                    // A proof whose verification hashes a known number of
                    // blocks. Built before the barrier, so the counted region
                    // holds nothing but the work being counted.
                    let mut blocks = MemBlocks::default();
                    let root = init(&mut blocks);
                    let edits: Vec<(Vec<u8>, Edit)> = (0..ENTRIES as u32)
                        .map(|i| {
                            (
                                format!("k/{i:08}").into_bytes(),
                                Edit::Put(vec![(i % 251) as u8; 120]),
                            )
                        })
                        .collect();
                    let root = apply_into(&mut blocks, &root, &edits).unwrap().root;
                    let key = b"k/00001000".to_vec();
                    let honest = prove(&blocks, &root, &key).unwrap();
                    let n = honest.nodes.len();
                    assert!(n > 0, "the fixture must do real work");

                    work::reset();
                    ready.wait();
                    verify(&root, &key, &honest).expect("an honest proof verifies");
                    let mine = work::hashed();
                    // Every thread's work is finished before any thread reads.
                    measured.wait();
                    let shared = work::SHARED_HASHED.load(Ordering::Relaxed);

                    assert_eq!(
                        mine, n,
                        "the counter reported other threads' work: it is not per-thread"
                    );
                    // The control: the same count kept the old way is N times
                    // as large, which is exactly the number the cost tests used
                    // to assert on.
                    assert!(
                        shared >= mine * THREADS,
                        "the shared counter read {shared}, not {} — the control \
                         is not loaded and proves nothing",
                        mine * THREADS
                    );
                });
            }
        });
    }
}
