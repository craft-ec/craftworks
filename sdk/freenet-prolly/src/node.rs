//! Tree node format `PT01`: one node is one Block, read in place.
//!
//! ```text
//! header (28 B)
//!   magic   "PT01"      4
//!   level   u8          0 = leaf
//!   flags   u8          reserved, must be 0
//!   count   u16         entries
//!   agg     count:u64 ‖ bytes:u64     this subtree
//!   pcount  u16         parity cids: 3 per sibling group (branch: children;
//!                       leaf: referenced values)
//!   plen    u16         length of the shared key prefix
//! prefix [u8; plen]     the longest common prefix of every key in the node
//! offs   [u16; count]   offset of each entry from the start of the node
//! keys4  [u32; count]   first 4 bytes of each key SUFFIX, big-endian, zero-padded
//! entries, contiguous, in key order — each holds only the key's suffix
//!   leaf    klen:u16 ‖ vkind:u8 ‖ vlen:u32 ‖ suffix ‖ val
//!             vkind 0 = inline bytes (vlen = their length), vlen ≤ MAX_INLINE
//!             vkind 1 = reference: val is cid(32); vlen = referenced length,
//!                       vlen > MAX_INLINE — so a value has exactly one encoding
//!   branch  klen:u16 ‖ child:cid(32) ‖ child_agg(16) ‖ suffix    key = child's min key
//! parity [cid; pcount]
//! ```
//! Keys in one node share their leading bytes (type byte, domain, …), so the
//! prefix is stored once and the 4-byte search index is taken over the suffix,
//! where keys actually differ.
//!
//! All integers little-endian except `keys4`, which is big-endian so that integer
//! order equals byte order. Nothing is aligned; every read is `from_le_bytes`.
//!
//! [`Node::parse`] accepts exactly the bytes [`NodeBuilder`] can produce: a node
//! that parses is sorted, tiled with no gaps or trailing bytes, carries an
//! aggregate equal to the fold of its entries, and stores exactly the longest
//! common prefix — so every logical node has one encoding.

use crate::Cid;

pub const MAGIC: &[u8; 4] = b"PT01";
/// Hard cap on an encoded node. The chunker clamps well below this.
pub const MAX_NODE: usize = 16 * 1024;
/// Encoded size of an empty node.
pub const HEADER: usize = 28;
/// Longest key (prefix + suffix). Key *shapes* keep real keys far below this.
pub const MAX_KEY: usize = 512;
/// Longest value stored in the leaf. Anything longer MUST be a reference and
/// anything this short MUST be inline: the choice is part of the format, or
/// the same contents could hash two ways.
pub const MAX_INLINE: usize = 1024;
/// Longest value at all. A referenced value is one Block, and this is the Block
/// contract's body limit. **This crate does not chunk: a longer value is
/// refused** (`ApplyError::ValueTooLong`, `BuildError::ValueTooLong`). Storing
/// one as a blob (manifest + chunks) is a caller's job — see ARCHITECTURE §7
/// `BlobRef` — and the caller stores the manifest's ref as the value.
pub const MAX_VALUE: usize = 256 * 1024;
const LEAF_FIXED: usize = 2 + 1 + 4;
const BRANCH_FIXED: usize = 2 + 32 + 16;
const REF_LEN: usize = 32;

/// The most parity cids the reserve can hold: 4 KiB of 32-byte ids.
///
/// A cheap bound at PARSE time, before the entries are read. The EXACT count is
/// a pure function of the entries and is checked in
/// [`check_node`](crate::boundary::check_node) — but a node claiming more
/// parity than could ever fit must be refused before its entries are walked,
/// or the region is a place to hang bytes that cost a reader work to reach.
pub const MAX_PCOUNT: usize = (MAX_NODE - MAX_LOGICAL_RESERVE) / REF_LEN;
/// The entries' hard limit, restated here so `MAX_PCOUNT` does not depend on
/// `boundary`, which depends on this module. Asserted equal to
/// [`crate::boundary::MAX_LOGICAL`] there.
const MAX_LOGICAL_RESERVE: usize = 12 * 1024;

/// Rolled-up statistics of a subtree. Per-type counts need no extra fields:
/// keys are type-prefixed, so a range aggregate yields them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Agg {
    /// Leaf entries in the subtree.
    pub count: u64,
    /// Logical bytes: key length + value length (referenced length for refs).
    pub bytes: u64,
}

impl Agg {
    /// Overflow-checked sum; `None` on overflow.
    pub fn checked_add(self, o: Agg) -> Option<Agg> {
        Some(Agg {
            count: self.count.checked_add(o.count)?,
            bytes: self.bytes.checked_add(o.bytes)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value<'a> {
    Inline(&'a [u8]),
    /// A value stored in its own block. (A caller's blob manifest is such a
    /// value too: this crate stores the bytes it is given and never chunks.)
    Ref {
        cid: Cid,
        len: u32,
    },
}

impl<'a> Value<'a> {
    /// How the format requires `bytes` to be stored, and the block that has to
    /// be kept if it does not fit in the leaf.
    ///
    /// The choice is part of the format, not a policy: a value at or below
    /// [`MAX_INLINE`] MUST be inline and anything longer MUST be a reference,
    /// or the same contents would hash two ways. This is the ONE place that
    /// decides it — `apply` and [`TreeBuilder::push_bytes`](crate::build::TreeBuilder::push_bytes)
    /// both call it, so a caller building the same tree by hand cannot drift
    /// from the rule by restating it.
    pub fn for_bytes(bytes: &'a [u8]) -> (Value<'a>, Option<(Cid, &'a [u8])>) {
        if bytes.len() <= MAX_INLINE {
            (Value::Inline(bytes), None)
        } else {
            let cid = crate::block_id(crate::kind::RAW, bytes);
            (
                Value::Ref {
                    cid,
                    len: bytes.len() as u32,
                },
                Some((cid, bytes)),
            )
        }
    }

    fn logical_len(&self) -> u64 {
        match self {
            Value::Inline(b) => b.len() as u64,
            Value::Ref { len, .. } => *len as u64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeError {
    TooShort,
    TooLarge,
    BadMagic,
    BadFlags,
    /// A field is inconsistent with the level, or the parity region is larger
    /// than the reserve can hold.
    BadShape,
    OffsetOutOfRange,
    /// Entries do not tile the entry region exactly.
    BadTiling,
    BadEntry,
    KeysNotSorted,
    BadKeyPrefix,
    AggMismatch,
    Overflow,
    TrailingBytes,
    KeyTooLong,
    /// The stored prefix is not exactly the longest common prefix of the keys.
    NonCanonicalPrefix,
    /// An inline value longer than `MAX_INLINE`, a reference to one that short,
    /// or a reference to a value longer than `MAX_VALUE`.
    NonCanonicalValue,
}

fn u16_at(b: &[u8], i: usize) -> usize {
    u16::from_le_bytes([b[i], b[i + 1]]) as usize
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}
fn cid_at(b: &[u8], i: usize) -> Cid {
    b[i..i + 32].try_into().unwrap()
}

/// Length of the longest common prefix of `a` and `b`.
pub fn lcp(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// First four bytes as a big-endian integer, zero-padded.
pub fn key4(key: &[u8]) -> u32 {
    let mut p = [0u8; 4];
    let n = key.len().min(4);
    p[..n].copy_from_slice(&key[..n]);
    u32::from_be_bytes(p)
}

/// A validated, borrowed view of an encoded node.
#[derive(Clone, Copy, Debug)]
pub struct Node<'a> {
    bytes: &'a [u8],
    level: u8,
    count: usize,
    pcount: usize,
    plen: usize,
    agg: Agg,
}

impl<'a> Node<'a> {
    /// Validate `bytes` completely. Never panics on any input.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, NodeError> {
        if bytes.len() < HEADER {
            return Err(NodeError::TooShort);
        }
        if bytes.len() > MAX_NODE {
            return Err(NodeError::TooLarge);
        }
        if &bytes[..4] != MAGIC {
            return Err(NodeError::BadMagic);
        }
        let level = bytes[4];
        if bytes[5] != 0 {
            return Err(NodeError::BadFlags);
        }
        let count = u16_at(bytes, 6);
        let agg = Agg {
            count: u64_at(bytes, 8),
            bytes: u64_at(bytes, 16),
        };
        let pcount = u16_at(bytes, 24);
        let plen = u16_at(bytes, 26);
        // A LEAF may carry parity now: it protects the values stored by
        // reference (#19). What `pcount` must EQUAL is a boundary-rule question
        // — it is a pure function of the entries, so it needs them parsed —
        // and lives in `check_node`. Here only the region's size is decided.
        if pcount > MAX_PCOUNT {
            return Err(NodeError::BadShape);
        }
        // A branch with no children is meaningless; an empty leaf is the empty tree.
        if level > 0 && count == 0 {
            return Err(NodeError::BadShape);
        }
        // The empty tree has no keys, hence no prefix.
        if (count == 0 && plen != 0) || plen > MAX_KEY {
            return Err(if plen > MAX_KEY {
                NodeError::KeyTooLong
            } else {
                NodeError::BadShape
            });
        }
        let table = HEADER + plen;
        let entries_start = table + count * 6;
        let parity_len = pcount * REF_LEN;
        if entries_start + parity_len > bytes.len() {
            return Err(NodeError::TooShort);
        }
        let entries_end = bytes.len() - parity_len;

        let node = Node {
            bytes,
            level,
            count,
            pcount,
            plen,
            agg,
        };
        let mut cursor = entries_start;
        let mut fold = Agg::default();
        let mut prev_key: Option<&[u8]> = None;
        for i in 0..count {
            let off = u16_at(bytes, table + i * 2);
            // Entries must tile: each starts exactly where the previous ended.
            if off != cursor {
                return Err(if off < entries_start || off >= entries_end {
                    NodeError::OffsetOutOfRange
                } else {
                    NodeError::BadTiling
                });
            }
            let fixed = if level == 0 { LEAF_FIXED } else { BRANCH_FIXED };
            if off + fixed > entries_end {
                return Err(NodeError::BadEntry);
            }
            let klen = u16_at(bytes, off);
            let (key_at, end, entry_agg) = if level == 0 {
                let vkind = bytes[off + 2];
                let vlen = u32_at(bytes, off + 3);
                let stored = match vkind {
                    0 if vlen as usize <= MAX_INLINE => vlen as usize,
                    1 if vlen as usize > MAX_INLINE && vlen as usize <= MAX_VALUE => REF_LEN,
                    0 | 1 => return Err(NodeError::NonCanonicalValue),
                    _ => return Err(NodeError::BadEntry),
                };
                let key_at = off + LEAF_FIXED;
                let end = key_at
                    .checked_add(klen)
                    .and_then(|e| e.checked_add(stored))
                    .ok_or(NodeError::Overflow)?;
                (
                    key_at,
                    end,
                    Agg {
                        count: 1,
                        bytes: (plen + klen) as u64 + vlen as u64,
                    },
                )
            } else {
                let key_at = off + BRANCH_FIXED;
                let child_agg = Agg {
                    count: u64_at(bytes, off + 34),
                    bytes: u64_at(bytes, off + 42),
                };
                (key_at, key_at + klen, child_agg)
            };
            if end > entries_end {
                return Err(NodeError::BadEntry);
            }
            if plen + klen > MAX_KEY {
                return Err(NodeError::KeyTooLong);
            }
            let key = &bytes[key_at..key_at + klen];
            if prev_key.is_some_and(|p| p >= key) {
                return Err(NodeError::KeysNotSorted);
            }
            if u32_at_be(bytes, table + count * 2 + i * 4) != key4(key) {
                return Err(NodeError::BadKeyPrefix);
            }
            fold = fold.checked_add(entry_agg).ok_or(NodeError::Overflow)?;
            prev_key = Some(key);
            cursor = end;
        }
        if cursor != entries_end {
            return Err(NodeError::TrailingBytes);
        }
        if fold != agg {
            return Err(NodeError::AggMismatch);
        }
        // One encoding per node: the prefix must be maximal. Keys are sorted, so the
        // common prefix of all equals that of the first and last; with the prefix
        // stripped those two suffixes must share nothing. A lone key is all prefix.
        let maximal = match count {
            0 => true,
            1 => node.suffix(0).is_empty(),
            _ => lcp(node.suffix(0), node.suffix(count - 1)) == 0,
        };
        if !maximal {
            return Err(NodeError::NonCanonicalPrefix);
        }
        Ok(node)
    }

    pub fn level(&self) -> u8 {
        self.level
    }
    pub fn is_leaf(&self) -> bool {
        self.level == 0
    }
    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn agg(&self) -> Agg {
        self.agg
    }
    pub fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    fn off(&self, i: usize) -> usize {
        u16_at(self.bytes, HEADER + self.plen + i * 2)
    }

    /// The bytes every key in this node starts with.
    pub fn prefix(&self) -> &'a [u8] {
        &self.bytes[HEADER..HEADER + self.plen]
    }

    /// Full key of entry `i` (prefix + suffix). Allocates; hot paths use
    /// [`Node::prefix`] and [`Node::suffix`].
    pub fn key(&self, i: usize) -> Vec<u8> {
        [self.prefix(), self.suffix(i)].concat()
    }

    /// Key of entry `i` with the node's prefix removed.
    pub fn suffix(&self, i: usize) -> &'a [u8] {
        let off = self.off(i);
        let klen = u16_at(self.bytes, off);
        let at = off
            + if self.level == 0 {
                LEAF_FIXED
            } else {
                BRANCH_FIXED
            };
        &self.bytes[at..at + klen]
    }

    /// Value of leaf entry `i`. Panics if this is a branch or `i` is out of range.
    pub fn value(&self, i: usize) -> Value<'a> {
        assert!(self.is_leaf() && i < self.count);
        let off = self.off(i);
        let klen = u16_at(self.bytes, off);
        let vlen = u32_at(self.bytes, off + 3);
        let at = off + LEAF_FIXED + klen;
        match self.bytes[off + 2] {
            0 => Value::Inline(&self.bytes[at..at + vlen as usize]),
            _ => Value::Ref {
                cid: cid_at(self.bytes, at),
                len: vlen,
            },
        }
    }

    /// Child of branch entry `i`. Panics if this is a leaf or `i` is out of range.
    pub fn child(&self, i: usize) -> (Cid, Agg) {
        assert!(!self.is_leaf() && i < self.count);
        let off = self.off(i);
        (
            cid_at(self.bytes, off + 2),
            Agg {
                count: u64_at(self.bytes, off + 34),
                bytes: u64_at(self.bytes, off + 42),
            },
        )
    }

    /// How many parity cids this node carries.
    pub fn parity_count(&self) -> usize {
        self.pcount
    }

    pub fn parity(&self) -> impl Iterator<Item = Cid> + 'a {
        let start = self.bytes.len() - self.pcount * REF_LEN;
        let b = self.bytes;
        (0..self.pcount).map(move |i| cid_at(b, start + i * REF_LEN))
    }

    /// Binary search for a full key: `Ok(i)` if entry `i` has it, else `Err(i)` =
    /// insertion point. The shared prefix is compared once; after that only the
    /// inline 4-byte suffix prefixes are compared, touching suffix bytes on a tie.
    pub fn search(&self, key: &[u8]) -> Result<usize, usize> {
        let prefix = self.prefix();
        let Some(rest) = key.strip_prefix(prefix) else {
            // Every key here starts with `prefix`; a key that does not sorts
            // wholly before or wholly after all of them.
            return Err(if key < prefix { 0 } else { self.count });
        };
        let want = key4(rest);
        let table = HEADER + self.plen + self.count * 2;
        let (mut lo, mut hi) = (0usize, self.count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let have = u32_at_be(self.bytes, table + mid * 4);
            match have.cmp(&want).then_with(|| self.suffix(mid).cmp(rest)) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Ok(mid),
            }
        }
        Err(lo)
    }
}

fn u32_at_be(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// Keys must be pushed in strictly increasing order.
    NotSorted,
    KeyTooLong,
    /// An inline value longer than `MAX_INLINE` (store it by reference), or any
    /// value longer than `MAX_VALUE` — refused, not chunked: a blob (manifest +
    /// chunks) is the caller's to build, see [`MAX_VALUE`].
    ValueTooLong,
    /// A reference to a value of at most `MAX_INLINE` bytes: store it inline.
    ValueTooShort,
    TooManyEntries,
    NodeTooLarge,
    /// A parity member's bytes could not be found, so its group cannot be
    /// coded. The cid is carried so a caller with a remote store can FETCH it
    /// and try again: refusing outright is right only when the bytes can never
    /// arrive, which is a build with no base store.
    MissingMember(Cid),
    /// A value was pushed to a branch, or a child to a leaf.
    WrongKind,
    Overflow,
}

enum Body {
    Leaf {
        vkind: u8,
        vlen: u32,
        stored: Vec<u8>,
    },
    Branch {
        child: Cid,
        agg: Agg,
    },
}

/// Builds the canonical encoding of a node. Entries must arrive in key order.
/// Keys are buffered: the shared prefix is only known once the last key is in.
pub struct NodeBuilder {
    level: u8,
    entries: Vec<(Vec<u8>, Body)>,
    parity: Vec<Cid>,
    agg: Agg,
    logical: usize,
}

impl NodeBuilder {
    pub fn leaf() -> Self {
        Self::new(0)
    }
    /// `level` must be ≥ 1.
    pub fn branch(level: u8) -> Self {
        assert!(level >= 1);
        Self::new(level)
    }
    fn new(level: u8) -> Self {
        NodeBuilder {
            level,
            entries: Vec::new(),
            parity: Vec::new(),
            agg: Agg::default(),
            logical: HEADER,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Size of this node with every key stored in full and no parity: header +
    /// per entry (6 B of tables + fixed fields + key + stored value). It depends
    /// only on the entries themselves — never on what prefix they happen to
    /// share, nor on how parity is grouped — which makes it the right measure
    /// for deciding node boundaries.
    pub fn logical_len(&self) -> usize {
        self.logical
    }

    /// Upper bound on the encoded size: [`Self::logical_len`] plus parity.
    pub fn encoded_bound(&self) -> usize {
        self.logical + self.parity.len() * REF_LEN
    }

    /// Aggregate of the entries pushed so far.
    pub fn agg(&self) -> Agg {
        self.agg
    }

    /// The first (smallest) key pushed, if any.
    pub fn min_key(&self) -> Option<&[u8]> {
        self.entries.first().map(|(k, _)| k.as_slice())
    }

    /// Whether `key` and `value` can ever be a leaf entry: key length and the
    /// one-encoding rule. Independent of the builder's state, so a caller can
    /// check before doing anything that has side effects.
    pub fn check_leaf(key: &[u8], value: &Value<'_>) -> Result<(), BuildError> {
        if key.len() > MAX_KEY {
            return Err(BuildError::KeyTooLong);
        }
        match value {
            Value::Inline(b) if b.len() > MAX_INLINE => Err(BuildError::ValueTooLong),
            Value::Ref { len, .. } if *len as usize <= MAX_INLINE => Err(BuildError::ValueTooShort),
            Value::Ref { len, .. } if *len as usize > MAX_VALUE => Err(BuildError::ValueTooLong),
            _ => Ok(()),
        }
    }

    /// What one entry adds to [`Self::logical_len`].
    pub fn leaf_cost(key: &[u8], value: &Value<'_>) -> usize {
        Self::leaf_cost_len(key.len(), value)
    }

    /// [`Self::leaf_cost`] from the key's LENGTH, for a reader that holds the
    /// key in two pieces and has no reason to join them.
    pub fn leaf_cost_len(klen: usize, value: &Value<'_>) -> usize {
        let stored = match value {
            Value::Inline(b) => b.len(),
            Value::Ref { .. } => REF_LEN,
        };
        6 + LEAF_FIXED + klen + stored
    }

    /// What one child adds to [`Self::logical_len`].
    pub fn child_cost(min_key: &[u8]) -> usize {
        Self::child_cost_len(min_key.len())
    }

    /// [`Self::child_cost`] from the key's LENGTH.
    pub fn child_cost_len(klen: usize) -> usize {
        6 + BRANCH_FIXED + klen
    }

    fn admit(
        &mut self,
        key: &[u8],
        fixed: usize,
        stored: usize,
        entry_agg: Agg,
    ) -> Result<(), BuildError> {
        if key.len() > MAX_KEY {
            return Err(BuildError::KeyTooLong);
        }
        if self
            .entries
            .last()
            .is_some_and(|(p, _)| p.as_slice() >= key)
        {
            return Err(BuildError::NotSorted);
        }
        if self.entries.len() >= u16::MAX as usize {
            return Err(BuildError::TooManyEntries);
        }
        let grown = self.logical + 6 + fixed + key.len() + stored;
        if grown + self.parity.len() * REF_LEN > MAX_NODE {
            return Err(BuildError::NodeTooLarge);
        }
        self.agg = self
            .agg
            .checked_add(entry_agg)
            .ok_or(BuildError::Overflow)?;
        self.logical = grown;
        Ok(())
    }

    pub fn push(&mut self, key: &[u8], value: Value<'_>) -> Result<(), BuildError> {
        if self.level != 0 {
            return Err(BuildError::WrongKind);
        }
        Self::check_leaf(key, &value)?;
        let (vkind, vlen, stored): (u8, u32, Vec<u8>) = match value {
            Value::Inline(b) => (0, b.len() as u32, b.to_vec()),
            Value::Ref { cid, len } => (1, len, cid.to_vec()),
        };
        let entry_agg = Agg {
            count: 1,
            bytes: key.len() as u64 + value.logical_len(),
        };
        self.admit(key, LEAF_FIXED, stored.len(), entry_agg)?;
        self.entries.push((
            key.to_vec(),
            Body::Leaf {
                vkind,
                vlen,
                stored,
            },
        ));
        Ok(())
    }

    pub fn push_child(
        &mut self,
        min_key: &[u8],
        child: Cid,
        child_agg: Agg,
    ) -> Result<(), BuildError> {
        if self.level == 0 {
            return Err(BuildError::WrongKind);
        }
        self.admit(min_key, BRANCH_FIXED, 0, child_agg)?;
        self.entries.push((
            min_key.to_vec(),
            Body::Branch {
                child,
                agg: child_agg,
            },
        ));
        Ok(())
    }

    /// A leaf carries parity too now — over the values it stores by reference —
    /// so the level no longer decides whether an id may be added.
    pub fn push_parity(&mut self, cid: Cid) -> Result<(), BuildError> {
        if self.parity.len() >= u16::MAX as usize || self.encoded_bound() + REF_LEN > MAX_NODE {
            return Err(BuildError::NodeTooLarge);
        }
        self.parity.push(cid);
        Ok(())
    }

    pub fn finish(self) -> Result<Vec<u8>, BuildError> {
        if self.level > 0 && self.entries.is_empty() {
            return Err(BuildError::WrongKind);
        }
        let n = self.entries.len();
        let plen = match n {
            0 => 0,
            1 => self.entries[0].0.len(),
            _ => lcp(&self.entries[0].0, &self.entries[n - 1].0),
        };
        let mut body = Vec::new();
        let mut offs = Vec::with_capacity(n);
        let base = HEADER + plen + n * 6;
        for (key, b) in &self.entries {
            let suffix = &key[plen..];
            offs.push(u16::try_from(base + body.len()).map_err(|_| BuildError::NodeTooLarge)?);
            body.extend_from_slice(&(suffix.len() as u16).to_le_bytes());
            match b {
                Body::Leaf {
                    vkind,
                    vlen,
                    stored,
                } => {
                    body.push(*vkind);
                    body.extend_from_slice(&vlen.to_le_bytes());
                    body.extend_from_slice(suffix);
                    body.extend_from_slice(stored);
                }
                Body::Branch { child, agg } => {
                    body.extend_from_slice(child);
                    body.extend_from_slice(&agg.count.to_le_bytes());
                    body.extend_from_slice(&agg.bytes.to_le_bytes());
                    body.extend_from_slice(suffix);
                }
            }
        }
        let mut out = Vec::with_capacity(base + body.len() + self.parity.len() * REF_LEN);
        out.extend_from_slice(MAGIC);
        out.push(self.level);
        out.push(0);
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(&self.agg.count.to_le_bytes());
        out.extend_from_slice(&self.agg.bytes.to_le_bytes());
        out.extend_from_slice(&(self.parity.len() as u16).to_le_bytes());
        out.extend_from_slice(&(plen as u16).to_le_bytes());
        out.extend_from_slice(self.entries.first().map(|(k, _)| &k[..plen]).unwrap_or(&[]));
        for o in &offs {
            out.extend_from_slice(&o.to_le_bytes());
        }
        for (key, _) in &self.entries {
            out.extend_from_slice(&key4(&key[plen..]).to_be_bytes());
        }
        out.extend_from_slice(&body);
        for c in &self.parity {
            out.extend_from_slice(c);
        }
        debug_assert!(
            out.len() <= self.logical + self.parity.len() * REF_LEN && out.len() <= MAX_NODE
        );
        Ok(out)
    }
}

/// Is `child` exactly one level below `parent`? The ONE place this is decided.
///
/// Both levels come from bytes a stranger may have written, and a level is a
/// `u8`: `child + 1 == parent` overflows at 255 (freenet-prolly#55). With
/// overflow checks on that is a PANIC inside the verifiers that exist to take
/// strangers' bytes; with them off it wraps to 0, and the answer was right only
/// because no branch has level 0. Checked arithmetic makes the answer the same
/// in every profile, on purpose rather than by accident.
pub(crate) fn one_level_below(parent: u8, child: u8) -> bool {
    child.checked_add(1) == Some(parent)
}
