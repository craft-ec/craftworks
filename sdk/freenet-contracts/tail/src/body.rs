//! The tail's BODY: the tree's root, how far the tree covers, and the rows not yet in it.
//!
//! One canonical encoding, because the writer signs its hash: a host that accepted two spellings of one body would
//! let one decision hash two ways. Every integer is little-endian.
//!
//! ```text
//! body  = root_flag(1) ‖ root(32)? ‖ through(8) ‖ count(4) ‖ entry*count
//! entry = key_len(2) ‖ key ‖ seq(8) ‖ kind(1) ‖ (value_len(4) ‖ value)?     kind 0 = delete, 1 = set
//! ops   = count(2) ‖ op*count
//! op    = 1 ‖ key_len(2) ‖ key ‖ value_len(4) ‖ value        set
//!       | 2 ‖ key_len(2) ‖ key                               delete
//!       | 3 ‖ root(32) ‖ through(8)                          flush
//! ```
//!
//! Entries are strictly sorted by key (so one key appears once), and every entry's `seq` is above `through`: a row
//! at or below `through` is in the tree, and a body naming it again would be a second spelling.
//!
//! [`Body::apply`] is the ONE way a body changes. The host runs it on a delta, the writer runs it before signing, so
//! the two cannot disagree about what a delta means.

use std::collections::BTreeMap;

pub const HASH_LEN: usize = 32;
/// Largest encoded body. The tail is a buffer: past this the writer must flush before writing more.
pub const MAX_BODY: usize = 256 * 1024;
/// The tree's own key limit (freenet-prolly `node::MAX_KEY`): a row the tree cannot hold could never be flushed.
pub const MAX_KEY: usize = 512;
/// Largest value one entry may carry. Bigger values go in their own Block, and the entry carries the reference.
pub const MAX_VALUE: usize = 64 * 1024;
/// Most operations one delta may carry.
pub const MAX_OPS: usize = 4096;

/// Domain separator for the body hash the writer signs. Keeps a tail body from ever hashing like a Register value:
/// both contracts share the Register's signed-message format and params.
pub const BODY_DOMAIN: &[u8; 9] = b"TL01-body";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The delta that last wrote this key.
    pub seq: u64,
    /// `None` = deleted (a tombstone, so the delete reaches the tree at the next flush).
    pub value: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Body {
    /// The tree this tail sits in front of; `None` until the first flush.
    pub root: Option<[u8; HASH_LEN]>,
    /// Every delta with `seq <= through` is in the tree at `root`.
    pub through: u64,
    /// Rows written since `through`, newest per key.
    pub entries: BTreeMap<Vec<u8>, Entry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Set {
        key: Vec<u8>,
        value: Vec<u8>,
    },
    Delete {
        key: Vec<u8>,
    },
    /// The tree at `root` holds every delta up to `through`: those rows leave the tail.
    Flush {
        root: [u8; HASH_LEN],
        through: u64,
    },
}

/// A little-endian reader that refuses to read past the end.
struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (h, t) = self.0.split_at_checked(n)?;
        self.0 = t;
        Some(h)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn hash(&mut self) -> Option<[u8; HASH_LEN]> {
        self.take(HASH_LEN)?.try_into().ok()
    }
    fn key(&mut self) -> Option<Vec<u8>> {
        let n = self.u16()? as usize;
        (1..=MAX_KEY).contains(&n).then_some(())?;
        Some(self.take(n)?.to_vec())
    }
    fn value(&mut self) -> Option<Vec<u8>> {
        let n = self.u32()? as usize;
        (n <= MAX_VALUE).then_some(())?;
        Some(self.take(n)?.to_vec())
    }
    fn done(&self) -> bool {
        self.0.is_empty()
    }
}

fn put_key(out: &mut Vec<u8>, k: &[u8]) {
    out.extend_from_slice(&(k.len() as u16).to_le_bytes());
    out.extend_from_slice(k);
}

fn put_value(out: &mut Vec<u8>, v: &[u8]) {
    out.extend_from_slice(&(v.len() as u32).to_le_bytes());
    out.extend_from_slice(v);
}

fn key_ok(k: &[u8]) -> bool {
    (1..=MAX_KEY).contains(&k.len())
}

impl Body {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match &self.root {
            None => out.push(0),
            Some(r) => {
                out.push(1);
                out.extend_from_slice(r);
            }
        }
        out.extend_from_slice(&self.through.to_le_bytes());
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (k, e) in &self.entries {
            put_key(&mut out, k);
            out.extend_from_slice(&e.seq.to_le_bytes());
            match &e.value {
                None => out.push(0),
                Some(v) => {
                    out.push(1);
                    put_value(&mut out, v);
                }
            }
        }
        out
    }

    /// Parse exactly one canonical encoding, whose rows are all written at or below `seq` (the record's).
    pub fn parse(b: &[u8], seq: u64) -> Option<Body> {
        if b.len() > MAX_BODY {
            return None;
        }
        let mut c = Cursor(b);
        let root = match c.u8()? {
            0 => None,
            1 => Some(c.hash()?),
            _ => return None,
        };
        let through = c.u64()?;
        // Nothing can be in a tree that does not exist, and the tree cannot hold what was not yet written.
        if (root.is_none() && through != 0) || through > seq {
            return None;
        }
        let count = c.u32()? as usize;
        let mut entries = BTreeMap::new();
        let mut last: Option<Vec<u8>> = None;
        for _ in 0..count {
            let key = c.key()?;
            if last.as_ref().is_some_and(|l| key <= *l) {
                return None;
            }
            let eseq = c.u64()?;
            if eseq <= through || eseq > seq {
                return None;
            }
            let value = match c.u8()? {
                0 => None,
                1 => Some(c.value()?),
                _ => return None,
            };
            last = Some(key.clone());
            entries.insert(key, Entry { seq: eseq, value });
        }
        c.done().then_some(Body {
            root,
            through,
            entries,
        })
    }

    /// The hash the writer signs.
    pub fn hash(&self) -> [u8; HASH_LEN] {
        hash_encoded(&self.encode())
    }

    /// Apply one delta's operations, in order, as the delta numbered `seq`. `None` if the delta is not a legal next
    /// step: an empty op list, a bad key or value, a flush that moves `through` backwards or past this delta, or a
    /// result over [`MAX_BODY`].
    pub fn apply(&self, ops: &[Op], seq: u64) -> Option<Body> {
        if ops.is_empty() || ops.len() > MAX_OPS || seq == 0 {
            return None;
        }
        let mut b = self.clone();
        for op in ops {
            match op {
                Op::Set { key, value } => {
                    if !key_ok(key) || value.len() > MAX_VALUE {
                        return None;
                    }
                    b.entries.insert(
                        key.clone(),
                        Entry {
                            seq,
                            value: Some(value.clone()),
                        },
                    );
                }
                Op::Delete { key } => {
                    if !key_ok(key) {
                        return None;
                    }
                    b.entries.insert(key.clone(), Entry { seq, value: None });
                }
                Op::Flush { root, through } => {
                    // A flush covers deltas already made, never this one: this delta's own rows stay.
                    if *through < b.through || *through >= seq {
                        return None;
                    }
                    b.root = Some(*root);
                    b.through = *through;
                    b.entries.retain(|_, e| e.seq > *through);
                }
            }
        }
        (b.encode().len() <= MAX_BODY).then_some(b)
    }

    /// The pending row for `key`: `Some(Some(v))` set, `Some(None)` deleted, `None` not in the tail (read the tree).
    pub fn get(&self, key: &[u8]) -> Option<Option<&[u8]>> {
        self.entries.get(key).map(|e| e.value.as_deref())
    }
}

pub fn hash_encoded(body: &[u8]) -> [u8; HASH_LEN] {
    let mut h = blake3::Hasher::new();
    h.update(BODY_DOMAIN);
    h.update(body);
    *h.finalize().as_bytes()
}

pub fn encode_ops(ops: &[Op]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(ops.len() as u16).to_le_bytes());
    for op in ops {
        match op {
            Op::Set { key, value } => {
                out.push(1);
                put_key(&mut out, key);
                put_value(&mut out, value);
            }
            Op::Delete { key } => {
                out.push(2);
                put_key(&mut out, key);
            }
            Op::Flush { root, through } => {
                out.push(3);
                out.extend_from_slice(root);
                out.extend_from_slice(&through.to_le_bytes());
            }
        }
    }
    out
}

pub fn parse_ops(b: &[u8]) -> Option<Vec<Op>> {
    if b.len() > MAX_BODY {
        return None;
    }
    let mut c = Cursor(b);
    let n = c.u16()? as usize;
    if n == 0 || n > MAX_OPS {
        return None;
    }
    let mut ops = Vec::with_capacity(n);
    for _ in 0..n {
        ops.push(match c.u8()? {
            1 => Op::Set {
                key: c.key()?,
                value: c.value()?,
            },
            2 => Op::Delete { key: c.key()? },
            3 => Op::Flush {
                root: c.hash()?,
                through: c.u64()?,
            },
            _ => return None,
        });
    }
    c.done().then_some(ops)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(k: &str, v: &str) -> Op {
        Op::Set {
            key: k.as_bytes().to_vec(),
            value: v.as_bytes().to_vec(),
        }
    }

    #[test]
    fn a_body_round_trips_and_its_encoding_is_the_only_one() {
        let b = Body::default()
            .apply(&[set("b", "2"), set("a", "1")], 1)
            .unwrap()
            .apply(&[Op::Delete { key: b"c".to_vec() }], 2)
            .unwrap();
        let bytes = b.encode();
        assert_eq!(Body::parse(&bytes, 2), Some(b.clone()));
        // Trailing bytes, and a row newer than the record, are refused.
        assert_eq!(Body::parse(&[bytes.clone(), vec![0]].concat(), 2), None);
        assert_eq!(Body::parse(&bytes, 1), None);
    }

    #[test]
    fn unsorted_or_repeated_keys_are_refused() {
        let b = Body::default()
            .apply(&[set("a", "1"), set("b", "2")], 1)
            .unwrap();
        let mut bytes = b.encode();
        // Swap the two single-byte keys: entry layout is key_len(2) key(1) seq(8) kind(1) len(4) value(1).
        let first = 1 + 8 + 4 + 2;
        let second = first + 1 + 8 + 1 + 4 + 1 + 2;
        bytes.swap(first, second);
        assert_eq!(Body::parse(&bytes, 1), None);
    }

    #[test]
    fn a_flush_removes_exactly_the_rows_it_covers() {
        let b = Body::default()
            .apply(&[set("a", "1")], 1)
            .unwrap()
            .apply(&[set("b", "2")], 2)
            .unwrap();
        let root = [7u8; 32];
        let f = b
            .apply(&[Op::Flush { root, through: 1 }, set("c", "3")], 3)
            .unwrap();
        assert_eq!(f.root, Some(root));
        assert_eq!(f.through, 1);
        assert_eq!(f.get(b"a"), None, "a is in the tree now");
        assert_eq!(f.get(b"b"), Some(Some(&b"2"[..])));
        assert_eq!(f.get(b"c"), Some(Some(&b"3"[..])));
        assert_eq!(Body::parse(&f.encode(), 3), Some(f));
    }

    #[test]
    fn a_row_rewritten_after_the_flush_point_stays() {
        let b = Body::default()
            .apply(&[set("a", "old")], 1)
            .unwrap()
            .apply(&[set("a", "new")], 2)
            .unwrap();
        let f = b
            .apply(
                &[Op::Flush {
                    root: [1; 32],
                    through: 1,
                }],
                3,
            )
            .unwrap();
        assert_eq!(f.get(b"a"), Some(Some(&b"new"[..])));
    }

    #[test]
    fn a_flush_cannot_go_backwards_or_cover_its_own_delta() {
        let b = Body::default().apply(&[set("a", "1")], 1).unwrap();
        let f = b
            .apply(
                &[Op::Flush {
                    root: [1; 32],
                    through: 1,
                }],
                2,
            )
            .unwrap();
        assert!(f
            .apply(
                &[Op::Flush {
                    root: [2; 32],
                    through: 0
                }],
                3
            )
            .is_none());
        assert!(f
            .apply(
                &[Op::Flush {
                    root: [2; 32],
                    through: 3
                }],
                3
            )
            .is_none());
    }

    #[test]
    fn empty_bad_or_oversized_deltas_are_refused() {
        let b = Body::default();
        assert!(b.apply(&[], 1).is_none());
        assert!(b.apply(&[set("", "x")], 1).is_none());
        let big = Op::Set {
            key: b"k".to_vec(),
            value: vec![0; MAX_VALUE + 1],
        };
        assert!(b.apply(&[big], 1).is_none());
        // Many max-size values push the body over its cap.
        let ops: Vec<Op> = (0..5u16)
            .map(|i| Op::Set {
                key: i.to_le_bytes().to_vec(),
                value: vec![0; MAX_VALUE],
            })
            .collect();
        assert!(b.apply(&ops, 1).is_none());
    }

    #[test]
    fn ops_round_trip_and_trailing_bytes_are_refused() {
        let ops = vec![
            set("a", "1"),
            Op::Delete { key: b"b".to_vec() },
            Op::Flush {
                root: [9; 32],
                through: 4,
            },
        ];
        let bytes = encode_ops(&ops);
        assert_eq!(parse_ops(&bytes), Some(ops));
        assert_eq!(parse_ops(&[bytes, vec![0]].concat()), None);
        assert_eq!(parse_ops(&encode_ops(&[])), None);
    }
}
