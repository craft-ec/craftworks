//! A `BTreeMap` behind the SDK's `Store` — test code only (sdk#305).
//!
//! It was `craftworks_sdk::MemStore`, a backend no production path used. What
//! it is FOR is being obviously right: the differential tests check the prolly
//! tree against it, and a binding test runs the full-reload path on it (it has
//! no history, so it cannot diff two roots).

use craftworks_sdk::store::{Delta, Edit, Read, Reads, Refused, Store};
use std::collections::BTreeMap;

/// In-memory store: a sorted map, the REFERENCE the tree is checked against.
#[derive(Default)]
pub struct MemStore(BTreeMap<Vec<u8>, Vec<u8>>);

impl Store for MemStore {
    fn apply_batch(&mut self, edits: &[(Vec<u8>, Edit)]) -> Result<(), Refused> {
        for (k, e) in edits {
            match e {
                Edit::Put(v) => {
                    self.0.insert(k.clone(), v.clone());
                }
                Edit::Delete => {
                    self.0.remove(k);
                }
            }
        }
        Ok(())
    }
}

impl Reads for MemStore {
    fn get(&mut self, key: &[u8]) -> Read<Option<Vec<u8>>> {
        Ok(self.0.get(key).cloned())
    }
    fn scan(
        &mut self,
        lo: &[u8],
        hi: &[u8],
        reverse: bool,
        limit: usize,
    ) -> Read<Vec<(Vec<u8>, Vec<u8>)>> {
        if lo >= hi {
            return Ok(Vec::new());
        }
        let r = self
            .0
            .range(lo.to_vec()..hi.to_vec())
            .map(|(k, v)| (k.clone(), v.clone()));
        Ok(if reverse {
            r.rev().take(limit).collect()
        } else {
            r.take(limit).collect()
        })
    }
    /// A hash of the whole map.
    ///
    /// Not a prolly root, and it does not have to be: what a root is FOR here
    /// is answering "is this the same content I last saw", and a content hash
    /// answers that exactly. It means a binding over the in-memory store
    /// behaves like one over a tree — including reloading when it should —
    /// rather than working only on the backend it was tested against.
    fn root(&mut self) -> Read<[u8; 32]> {
        let mut all = Vec::new();
        for (k, v) in &self.0 {
            all.extend_from_slice(&(k.len() as u64).to_le_bytes());
            all.extend_from_slice(k);
            all.extend_from_slice(&(v.len() as u64).to_le_bytes());
            all.extend_from_slice(v);
        }
        Ok(freenet_prolly::block_id(freenet_prolly::kind::RAW, &all))
    }
    /// The in-memory store keeps no history, so it cannot diff two roots.
    ///
    /// It says so with the DEFINED answer rather than an error, which is the
    /// point of that answer existing: a binding does the same thing here as
    /// it does against an engine whose old root has been evicted, so the
    /// reload path is exercised on both backends instead of only the one
    /// that is harder to test.
    fn changes_since(
        &mut self,
        _from: [u8; 32],
        _lo: &[u8],
        _hi: &[u8],
        _max_entries: u32,
    ) -> Read<Delta> {
        Ok(Delta::FullReloadRequired {
            new_root: self.root()?,
        })
    }
}
