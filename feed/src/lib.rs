//! FEEDS (ARCHITECTURE: every writer its own feed; versions are causal). A table is read from several writers' feeds —
//! each a tail under its writer's own key — and MERGED here.
//!
//! Every row a feed writes is a VERSION: its id (the writer, and the feed's sequence it was written at), the id of the
//! version it replaces (`after`: the one its writer saw as current), and its value, or none (a delete is a version
//! too, so it wins over what it replaced in every other feed). Versions of one row form chains through `after`; the
//! current one is a HEAD — a version nothing replaces — and of several heads (two writers, at the same time) every
//! reader picks the same: the longer chain, then the higher id. No clock is involved.

use std::collections::{BTreeMap, HashMap};

/// A version's id: its writer's key and the feed's sequence it was written at.
pub type Id = ([u8; 32], u64);

const MAGIC: [u8; 2] = [0xCF, 0x01];

/// A row's value as its feed stores it (before sealing).
pub fn envelope(id: Id, after: Option<Id>, value: Option<&[u8]>) -> Vec<u8> {
    let mut b = MAGIC.to_vec();
    b.extend_from_slice(&id.0);
    b.extend_from_slice(&id.1.to_be_bytes());
    b.push(u8::from(after.is_some()) | (u8::from(value.is_none()) << 1));
    if let Some((w, s)) = after {
        b.extend_from_slice(&w);
        b.extend_from_slice(&s.to_be_bytes());
    }
    b.extend_from_slice(value.unwrap_or_default());
    b
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub id: Id,
    pub after: Option<Id>,
    /// `None`: deleted.
    pub value: Option<Vec<u8>>,
}

/// A stored value read back: `None` if it is not a version's envelope.
pub fn open(b: &[u8]) -> Option<Version> {
    let rest = b.strip_prefix(&MAGIC)?;
    let (w, rest) = rest.split_first_chunk::<32>()?;
    let (s, rest) = rest.split_first_chunk::<8>()?;
    let (&flags, mut rest) = rest.split_first()?;
    if flags & !3 != 0 {
        return None;
    }
    let after = if flags & 1 == 1 {
        let (aw, r) = rest.split_first_chunk::<32>()?;
        let (asq, r) = r.split_first_chunk::<8>()?;
        rest = r;
        Some((*aw, u64::from_be_bytes(*asq)))
    } else {
        None
    };
    let value = (flags & 2 == 0).then(|| rest.to_vec());
    if value.is_none() && !rest.is_empty() {
        return None;
    }
    Some(Version { id: (*w, u64::from_be_bytes(*s)), after, value })
}

/// One row of the merged table: its value and the id of the version that holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub value: Vec<u8>,
    pub id: Id,
}

/// MERGE the rows of several feeds, each `(its writer's key, its rows: key -> stored value)`. A version whose id names
/// another writer than the feed it is in is not taken (a writer speaks only for itself), nor is a value that is not a
/// version. Rows whose current version is a delete are absent.
pub fn merge(feeds: &[([u8; 32], &BTreeMap<Vec<u8>, Vec<u8>>)]) -> BTreeMap<Vec<u8>, Row> {
    let mut versions: BTreeMap<&[u8], Vec<Version>> = BTreeMap::new();
    for (writer, rows) in feeds {
        for (k, v) in rows.iter() {
            if let Some(ver) = open(v).filter(|ver| ver.id.0 == *writer) {
                versions.entry(k).or_default().push(ver);
            }
        }
    }
    let mut out = BTreeMap::new();
    for (k, vs) in versions {
        if let Some(head) = current(&vs) {
            if let Some(value) = &head.value {
                out.insert(k.to_vec(), Row { value: value.clone(), id: head.id });
            }
        }
    }
    out
}

/// The current version of one row: of the heads (versions nothing replaces), the longest chain, then the highest id.
fn current(vs: &[Version]) -> Option<&Version> {
    let by_id: HashMap<Id, &Version> = vs.iter().map(|v| (v.id, v)).collect();
    let replaced: std::collections::HashSet<Id> = vs.iter().filter_map(|v| v.after).collect();
    let mut depth: HashMap<Id, u64> = HashMap::new();
    let depth_of = |id: Id, depth: &mut HashMap<Id, u64>| -> u64 {
        // Walk back to what is known, then count forward; a cycle (a writer lying about `after`) ends the walk.
        let mut chain = Vec::new();
        let mut at = Some(id);
        let mut seen = std::collections::HashSet::new();
        while let Some(i) = at {
            if let Some(d) = depth.get(&i) {
                let mut d = *d;
                for c in chain.iter().rev() {
                    d += 1;
                    depth.insert(*c, d);
                }
                return depth[&id];
            }
            if !seen.insert(i) {
                break;
            }
            chain.push(i);
            at = by_id.get(&i).and_then(|v| v.after).filter(|a| by_id.contains_key(a));
        }
        for (n, c) in chain.iter().rev().enumerate() {
            depth.insert(*c, n as u64 + 1);
        }
        depth[&id]
    };
    vs.iter()
        .filter(|v| !replaced.contains(&v.id))
        .map(|v| (depth_of(v.id, &mut depth), v))
        .max_by(|(da, a), (db, b)| da.cmp(db).then(a.id.cmp(&b.id)))
        .map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: [u8; 32] = [0xA; 32];
    const B: [u8; 32] = [0xB; 32];

    fn feed(rows: &[(&str, Vec<u8>)]) -> BTreeMap<Vec<u8>, Vec<u8>> {
        rows.iter().map(|(k, v)| (k.as_bytes().to_vec(), v.clone())).collect()
    }

    fn values(m: &BTreeMap<Vec<u8>, Row>) -> Vec<String> {
        m.iter().map(|(k, r)| format!("{}={}", String::from_utf8_lossy(k), String::from_utf8_lossy(&r.value))).collect()
    }

    #[test]
    fn an_envelope_reads_back_and_anything_else_is_not_one() {
        let v = envelope((A, 7), Some((B, 3)), Some(b"x"));
        assert_eq!(open(&v), Some(Version { id: (A, 7), after: Some((B, 3)), value: Some(b"x".to_vec()) }));
        let d = envelope((A, 8), None, None);
        assert_eq!(open(&d), Some(Version { id: (A, 8), after: None, value: None }));
        assert_eq!(open(br#"{"title":"a plain row"}"#), None);
        assert_eq!(open(&[0xCF, 0x01, 1]), None, "cut short");
        assert_eq!(open(&[&d[..], b"x"].concat()), None, "a delete carries no value");
    }

    #[test]
    fn a_version_replaces_what_it_names_across_feeds_and_a_delete_is_a_version() {
        let a = feed(&[("n", envelope((A, 1), None, Some(b"one")))]);
        let b = feed(&[("n", envelope((B, 1), Some((A, 1)), Some(b"two")))]);
        let m = merge(&[(A, &a), (B, &b)]);
        assert_eq!(values(&m), ["n=two"]);
        assert_eq!(m[&b"n".to_vec()].id, (B, 1));
        // A deletes after seeing two: gone everywhere.
        let a2 = feed(&[("n", envelope((A, 2), Some((B, 1)), None))]);
        assert!(merge(&[(A, &a2), (B, &b)]).is_empty());
        // Control: A deleting what IT saw (one) while B wrote two: concurrent, equal chains, the higher id (B) wins.
        let a3 = feed(&[("n", envelope((A, 2), Some((A, 1)), None))]);
        assert_eq!(values(&merge(&[(A, &a3), (B, &b)])), ["n=two"]);
    }

    #[test]
    fn concurrent_versions_are_settled_the_same_way_whatever_the_order() {
        let a = feed(&[("n", envelope((A, 2), Some((A, 1)), Some(b"from a")))]);
        let b = feed(&[("n", envelope((B, 5), Some((A, 1)), Some(b"from b")))]);
        let x = merge(&[(A, &a), (B, &b)]);
        assert_eq!(x, merge(&[(B, &b), (A, &a)]));
        // Equal chains: the higher id (B's key) wins.
        assert_eq!(x[&b"n".to_vec()].value, b"from b");
        // A longer chain wins over a higher id: 1-2-3 beats 1-5.
        let a3 = feed(&[("n", envelope((A, 3), Some((A, 2)), Some(b"a again")))]);
        let z = merge(&[(A, &a3), (A, &a), (B, &b)]);
        assert_eq!(z[&b"n".to_vec()].value, b"a again");
    }

    #[test]
    fn a_writer_speaks_only_for_itself_and_a_value_that_is_no_version_is_not_taken() {
        let a = feed(&[("n", envelope((A, 1), None, Some(b"new"))), ("p", b"plain".to_vec())]);
        assert_eq!(values(&merge(&[(A, &a)])), ["n=new"]);
        // A version claiming B's id, in A's feed: not taken (the control: B's own is).
        let forged = feed(&[("k", envelope((B, 9), None, Some(b"forged")))]);
        assert!(merge(&[(A, &forged)]).is_empty());
        assert_eq!(values(&merge(&[(B, &forged)])), ["k=forged"]);
    }

    #[test]
    fn a_writer_lying_about_after_neither_hangs_nor_wins() {
        // Two versions naming each other: a cycle, so neither is a head.
        let x = feed(&[("n", envelope((A, 1), Some((A, 2)), Some(b"x")))]);
        let y = feed(&[("n", envelope((A, 2), Some((A, 1)), Some(b"y")))]);
        assert!(merge(&[(A, &x), (A, &y)]).is_empty());
        // Control: an honest version beside the cycle is the row.
        let z = feed(&[("n", envelope((B, 1), None, Some(b"z")))]);
        assert_eq!(values(&merge(&[(A, &x), (A, &y), (B, &z)])), ["n=z"]);
    }
}

/// The page's side of this package (the `storage` capability composes it with the core, which stores and seals).
#[cfg(target_arch = "wasm32")]
mod js {
    use super::*;
    use wasm_bindgen::prelude::*;

    fn id_hex((w, s): Id) -> String {
        let mut h: String = w.iter().map(|b| format!("{b:02x}")).collect();
        h.push_str(&format!("{s:016x}"));
        h
    }

    fn id_of(h: &str) -> Result<Id, JsValue> {
        let b = (h.len() == 80).then(|| (0..40).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok()).collect::<Option<Vec<u8>>>()).flatten();
        let b = b.ok_or_else(|| JsValue::from_str(&format!("not a version id: {h}")))?;
        Ok((b[..32].try_into().expect("32"), u64::from_be_bytes(b[32..].try_into().expect("8"))))
    }

    fn key32(b: &[u8]) -> Result<[u8; 32], JsValue> {
        b.try_into().map_err(|_| JsValue::from_str("a writer is a 32-byte key"))
    }

    /// A row's value as a version: writer `writer` at its feed's sequence `seq`, replacing `after` (an id; empty:
    /// none); `value` undefined: a delete.
    #[wasm_bindgen]
    pub fn version(writer: &[u8], seq: f64, after: &str, value: Option<Vec<u8>>) -> Result<Vec<u8>, JsValue> {
        let after = if after.is_empty() { None } else { Some(id_of(after)?) };
        Ok(envelope((key32(writer)?, seq as u64), after, value.as_deref()))
    }

    /// MERGE: `feeds` = `[[writer, [[key, value], …]], …]` (bytes, as each feed stores them). Returns the table's rows
    /// `[{ key, value, id }]` (bytes).
    #[wasm_bindgen]
    pub fn merge_feeds(feeds: js_sys::Array) -> Result<js_sys::Array, JsValue> {
        let mut owned = Vec::new();
        for f in feeds.iter() {
            let f = js_sys::Array::from(&f);
            let writer = key32(&js_sys::Uint8Array::new(&f.get(0)).to_vec())?;
            let rows: BTreeMap<Vec<u8>, Vec<u8>> = js_sys::Array::from(&f.get(1))
                .iter()
                .map(|r| {
                    let r = js_sys::Array::from(&r);
                    (js_sys::Uint8Array::new(&r.get(0)).to_vec(), js_sys::Uint8Array::new(&r.get(1)).to_vec())
                })
                .collect();
            owned.push((writer, rows));
        }
        let refs: Vec<_> = owned.iter().map(|(w, r)| (*w, r)).collect();
        let out = js_sys::Array::new();
        for (k, row) in merge(&refs) {
            let o = js_sys::Object::new();
            js_sys::Reflect::set(&o, &"key".into(), &js_sys::Uint8Array::from(&k[..]).into())?;
            js_sys::Reflect::set(&o, &"value".into(), &js_sys::Uint8Array::from(&row.value[..]).into())?;
            js_sys::Reflect::set(&o, &"id".into(), &JsValue::from(id_hex(row.id)))?;
            out.push(&o);
        }
        Ok(out)
    }
}
