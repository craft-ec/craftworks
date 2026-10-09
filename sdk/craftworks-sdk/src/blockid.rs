//! Two 32-byte hashes that are not the same thing.
//!
//! [`ContentHash`] is `BLAKE3(bytes)` — the hash of some bytes, and nothing
//! more. [`BlockId`] is `BLAKE3(kind ‖ body)`, which is what addresses a block
//! on the network. For the bytes `value` those are `06444a49…` and `2585d5bd…`:
//! same input, different 32 bytes, and only the second fetches anything.
//!
//! (Issue #7 gives `6437b3ac…` for the first. That is the frozen vector for
//! `abc`, picked up by mistake; `tests/ids.rs` pins both numbers.)
//!
//! Before this they were both `[u8; 32]` behind the name "cid", so putting one
//! where the other belonged compiled, ran, and failed later as a block that is
//! not there — a fetch returning nothing, a page that stays empty, and no way
//! to tell that from a block that has not arrived yet.
//!
//! Two defences, because each catches what the other cannot:
//!
//! 1. **Distinct types.** Inside Rust the compiler refuses the substitution.
//!    There is no `From<ContentHash> for BlockId` and no accessor that yields
//!    a bare `[u8; 32]` you could pass along as either; you get the bytes by
//!    asking a named question ([`BlockId::hash`], [`ContentHash::bytes`]).
//! 2. **A tag in the text form.** Once an id is a string in a link, a message,
//!    a URL or a log line, the type is gone. So the tag travels with it and
//!    parsing is what checks it: an untagged 64-hex string is refused, and so
//!    is a tag naming the other kind.
//!
//! # The text form
//!
//! ```text
//! raw:2585d5bd…     a block of opaque bytes
//! node:8e3a91c4…    a tree-node block
//! hash:6437b3ac…    a content hash — addresses nothing
//! ```
//!
//! `<tag>:<64 lower-case hex>`. The tag is the block's KIND, because the kind
//! is hashed into the id: without it you cannot recompute the id from the body,
//! so an id that did not carry its kind would not be checkable. `hash` is in
//! the same namespace so that a content hash pasted where a block id belongs
//! gets a parse error that says which it is, instead of a lookup that misses.
//!
//! Rejected alternatives: a bare hex string plus a separate kind field (the two
//! come apart exactly when copied by hand, which is when it matters);
//! `blk:raw:…` (a second separator buys nothing while every tag is already
//! distinct); a multibase/multihash prefix (self-describing for algorithms
//! nobody here uses, at the cost of a vocabulary this codebase does not own).

use crate::hex;
use core_types::kind::BlockKind;
use freenet_prolly::Cid;
use std::fmt;
use std::str::FromStr;

/// What addresses a block: `BLAKE3(kind ‖ body)`, with the kind it was made
/// from kept beside it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId {
    kind: u8,
    hash: Cid,
}

/// `BLAKE3(bytes)`. The hash of some bytes — not an address, and nothing will
/// fetch by it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentHash(Cid);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdError {
    /// 64 hex characters and no tag. Named separately from an unknown tag
    /// because this is what a bare hash from before this change looks like,
    /// and the message can say so.
    Untagged,
    /// A tag, but not one this build knows.
    UnknownTag(String),
    /// A well-formed id of the other sort. The common real mistake.
    WrongSort {
        want: &'static str,
        got: &'static str,
    },
    /// Not `<tag>:<64 hex>` at all.
    Malformed,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdError::Untagged => write!(
                f,
                "64 hex characters with no tag: write raw:… or node:… for a block id, hash:… for a content hash"
            ),
            IdError::UnknownTag(t) => write!(f, "unknown id tag {t:?}"),
            IdError::WrongSort { want, got } => {
                write!(f, "this is a {got}, and a {want} is needed here")
            }
            IdError::Malformed => write!(f, "not an id: expected <tag>:<64 hex characters>"),
        }
    }
}

impl std::error::Error for IdError {}

/// The tag for a block kind, or `None` if this build does not know the kind.
///
/// Kinds come from `freenet_prolly::kind`, so the vocabulary is the substrate's
/// and not a second list that can drift from it.
pub fn tag_of(kind: u8) -> Option<&'static str> {
    tag(BlockKind::of_byte(kind)?)
}

/// The tag a kind is NAMED by in a public id, or `None` for a kind no public id names. EXHAUSTIVE over the one list
/// of kinds (core_types::kind): a new kind does not compile until it is decided here whether ids name it.
fn tag(kind: BlockKind) -> Option<&'static str> {
    match kind {
        BlockKind::Raw => Some("raw"),
        BlockKind::TreeNode => Some("node"),
        BlockKind::Parity | BlockKind::Pack => None,
    }
}

fn kind_of(tag_text: &str) -> Option<u8> {
    BlockKind::ALL.into_iter().find(|k| tag(*k) == Some(tag_text)).map(BlockKind::byte)
}

/// `<tag>:<64 hex>` split into its parts, with the shape checked once for both
/// types so they cannot disagree about what a well-formed id looks like.
fn split(s: &str) -> Result<(&str, Cid), IdError> {
    let Some((tag, rest)) = s.split_once(':') else {
        return Err(if is_hash(s) {
            IdError::Untagged
        } else {
            IdError::Malformed
        });
    };
    if !is_hash(rest) {
        return Err(IdError::Malformed);
    }
    let out = core_types::hex::decode_array::<32>(rest).ok_or(IdError::Malformed)?;
    Ok((tag, out))
}

/// Lower-case hex only: an id has ONE spelling, so two copies of the same id
/// are the same string and comparing them as strings is safe.
fn is_hash(s: &str) -> bool {
    core_types::hex::is_lower(s, 32)
}

impl BlockId {
    /// The id of a block of `kind` holding `body`.
    pub fn of(kind: u8, body: &[u8]) -> Self {
        BlockId {
            kind,
            // Computed by the library that defines what a block id IS, so this
            // cannot drift from what the network keys on.
            hash: freenet_prolly::block_id(kind, body),
        }
    }

    /// The id of a block of opaque bytes — what an app's own bytes become.
    pub fn raw(body: &[u8]) -> Self {
        Self::of(BlockKind::Raw.byte(), body)
    }

    /// The id of a tree-node block.
    pub fn node(body: &[u8]) -> Self {
        Self::of(BlockKind::TreeNode.byte(), body)
    }

    /// An id whose bytes are already known — a root the tree handed back, an id
    /// read out of a node. The caller states the kind because the bytes alone
    /// do not carry it.
    pub fn from_parts(kind: u8, hash: Cid) -> Self {
        BlockId { kind, hash }
    }

    pub fn kind(&self) -> u8 {
        self.kind
    }

    /// The 32 bytes the network keys on.
    pub fn hash(&self) -> Cid {
        self.hash
    }

    pub fn tag(&self) -> Option<&'static str> {
        tag_of(self.kind)
    }
}

impl ContentHash {
    pub fn of(bytes: &[u8]) -> Self {
        ContentHash(*blake3::hash(bytes).as_bytes())
    }

    pub fn bytes(&self) -> Cid {
        self.0
    }
}

impl fmt::Display for BlockId {
    /// An unknown kind prints as its number (`k7:…`) rather than being hidden:
    /// a reader can still see what it is, and parsing it back fails here rather
    /// than somewhere that would treat it as a known kind.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.tag() {
            Some(t) => write!(f, "{t}:{}", hex(&self.hash)),
            None => write!(f, "k{}:{}", self.kind, hex(&self.hash)),
        }
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "hash:{}", hex(&self.0))
    }
}

// Debug is the same text: an id in a log or a test failure should be the string
// you can paste back, not a list of 32 numbers.
impl fmt::Debug for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for BlockId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, IdError> {
        let (tag, hash) = split(s)?;
        if tag == "hash" {
            return Err(IdError::WrongSort {
                want: "block id",
                got: "content hash",
            });
        }
        match kind_of(tag) {
            Some(kind) => Ok(BlockId { kind, hash }),
            None => Err(IdError::UnknownTag(tag.to_string())),
        }
    }
}

impl FromStr for ContentHash {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, IdError> {
        let (tag, hash) = split(s)?;
        if tag != "hash" {
            return Err(IdError::WrongSort {
                want: "content hash",
                got: if kind_of(tag).is_some() {
                    "block id"
                } else {
                    "an id of some other sort"
                },
            });
        }
        Ok(ContentHash(hash))
    }
}
