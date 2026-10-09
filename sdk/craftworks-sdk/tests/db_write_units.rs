//! How many keys each `Db` operation writes, counted at the store boundary
//! every backend shares (moved from `outbox_multi_key.rs`, whose outbox went
//! with R-b; the count still says what a write's size is).

use craftworks_sdk::store::{Delta, Edit, Read, Reads, Store};
use craftworks_sdk::{id::loc_from_hex, Db, SystemEnv};
use testkit::MemStore;
use serde_json::{json, Map, Value};


/// THE BLAST RADIUS, measured: how many keys does each `Db` operation write?
///
/// If one record `put` wrote several keys (the record and its index entries),
/// every contended `put` would have jammed the outbox, not just explicit
/// batches. Today each is ONE key — so the defect needed `apply_batch`. The
/// SDK's own comment says index entries will join the record's write
/// (`Db::write`); when they do, this count moves, and every write grows.
/// Counts the keys in every write a `Db` hands its store — at the store
/// boundary, which is the one every backend shares.
struct Counting<S> {
    inner: S,
    writes: Vec<usize>,
}
impl<S: Store> Store for Counting<S> {
    fn apply_batch(&mut self, edits: &[(Vec<u8>, Edit)]) -> Result<(), craftworks_sdk::Refused> {
        self.writes.push(edits.len());
        self.inner.apply_batch(edits)
    }
}
impl<S: Reads> Reads for Counting<S> {
    fn get(&mut self, key: &[u8]) -> Read<Option<Vec<u8>>> {
        self.inner.get(key)
    }
    fn scan(
        &mut self,
        lo: &[u8],
        hi: &[u8],
        reverse: bool,
        limit: usize,
    ) -> Read<Vec<(Vec<u8>, Vec<u8>)>> {
        self.inner.scan(lo, hi, reverse, limit)
    }
    fn root(&mut self) -> Read<[u8; 32]> {
        self.inner.root()
    }
    fn changes_since(
        &mut self,
        from: [u8; 32],
        lo: &[u8],
        hi: &[u8],
        max_entries: u32,
    ) -> Read<Delta> {
        self.inner.changes_since(from, lo, hi, max_entries)
    }
}

#[test]
fn how_many_keys_each_db_operation_writes() {
    let mut db = Db::new(
        Counting {
            inner: MemStore::default(),
            writes: vec![],
        },
        SystemEnv,
        [0u8; 4],
    );
    let schema = |parent: Option<&str>| {
        serde_json::from_value::<craftworks_sdk::Schema>(json!({
            "type": "Note",
            "fields": [{"name": "title", "kind": "text"}, {"name": "owner", "kind": "text", "required": parent.is_some()}],
            "parent": parent,
        }))
        .unwrap()
    };
    let mut table: Vec<(String, Vec<usize>)> = Vec::new();
    let take = |db: &mut Db<Counting<MemStore>, SystemEnv>,
                op: String,
                table: &mut Vec<(String, Vec<usize>)>| {
        table.push((op, std::mem::take(&mut db.store_mut().writes)));
    };
    for (label, domain, parent) in [
        ("plain", "notes", None),
        ("parent-keyed", "kids", Some("owner")),
    ] {
        db.define(domain, &schema(parent)).unwrap();
        take(&mut db, format!("{label} define"), &mut table);
        let mut f = Map::new();
        f.insert("title".into(), Value::from("t"));
        f.insert(
            "owner".into(),
            Value::from("00112233445566778899aabbccddeeff"),
        );
        let rec = db.put(domain, &f).unwrap();
        take(&mut db, format!("{label} put"), &mut table);
        let loc = loc_from_hex(&rec.id).unwrap();
        let mut patch = Map::new();
        patch.insert("title".into(), Value::from("u"));
        db.update(domain, loc, &patch).unwrap();
        take(&mut db, format!("{label} update"), &mut table);
        db.delete(domain, loc).unwrap();
        take(&mut db, format!("{label} delete"), &mut table);
    }
    for (op, writes) in &table {
        println!("  {op:<22} keys per write: {writes:?}");
    }
    assert_eq!(table.len(), 8, "every operation was counted");
    for (op, writes) in &table {
        assert_eq!(
            writes,
            &vec![1],
            "{op}: expected ONE write of ONE key, got {writes:?} — see this test's doc"
        );
    }
}
