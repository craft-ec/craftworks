use craftworks_sdk::id::from_hex;
use craftworks_sdk::*;
use testkit::MemStore;
use serde_json::{json, Map, Value};

/// Deterministic time and randomness.
struct FakeEnv {
    now: u64,
    n: u32,
}
impl Env for FakeEnv {
    fn now_ms(&mut self) -> u64 {
        self.now
    }
    fn rand32(&mut self) -> u32 {
        self.n = self.n.wrapping_mul(1664525).wrapping_add(1013904223);
        self.n
    }
}

/// A second, deliberately different store: an unsorted Vec. The whole suite runs
/// on both, so the SDK cannot be relying on anything but the four operations.
#[derive(Default)]
struct VecStore(Vec<(Vec<u8>, Vec<u8>)>);
impl Store for VecStore {
    fn apply_batch(&mut self, edits: &[(Vec<u8>, craftworks_sdk::Edit)]) -> Result<(), craftworks_sdk::Refused> {
        for (k, e) in edits {
            self.0.retain(|(a, _)| a != k);
            if let craftworks_sdk::Edit::Put(v) = e {
                self.0.push((k.clone(), v.clone()));
            }
        }
        Ok(())
    }
}

impl craftworks_sdk::Reads for VecStore {
    fn get(&mut self, k: &[u8]) -> craftworks_sdk::Read<Option<Vec<u8>>> {
        Ok(self.0.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone()))
    }
    fn scan(
        &mut self,
        lo: &[u8],
        hi: &[u8],
        reverse: bool,
        limit: usize,
    ) -> craftworks_sdk::Read<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut r: Vec<_> = self
            .0
            .iter()
            .filter(|(k, _)| k.as_slice() >= lo && k.as_slice() < hi)
            .cloned()
            .collect();
        r.sort();
        if reverse {
            r.reverse();
        }
        r.truncate(limit);
        Ok(r)
    }
    fn root(&mut self) -> craftworks_sdk::Read<[u8; 32]> {
        let mut v = self.0.clone();
        v.sort();
        let mut h = blake3::Hasher::new();
        for (k, val) in &v {
            h.update(&(k.len() as u64).to_le_bytes());
            h.update(k);
            h.update(&(val.len() as u64).to_le_bytes());
            h.update(val);
        }
        Ok(*h.finalize().as_bytes())
    }
    fn changes_since(
        &mut self,
        _from: [u8; 32],
        _lo: &[u8],
        _hi: &[u8],
        _max: u32,
    ) -> craftworks_sdk::Read<craftworks_sdk::Delta> {
        Ok(craftworks_sdk::Delta::FullReloadRequired {
            new_root: self.root()?,
        })
    }
}

fn task_schema() -> Schema {
    serde_json::from_value(json!({"type": "Task", "fields": [
        {"name": "title", "kind": "text", "required": true},
        {"name": "done", "kind": "bool"},
        {"name": "priority", "kind": "int"},
    ]}))
    .unwrap()
}
fn obj(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}
fn db<S: Store + craftworks_sdk::Reads + Default>() -> Db<S, FakeEnv> {
    let mut d = Db::new(S::default(), FakeEnv { now: 1_000, n: 1 }, *b"dev1");
    d.define("tasks", &task_schema()).unwrap();
    d
}

fn suite<S: Store + craftworks_sdk::Reads + Default>() {
    // create / read
    let mut d = db::<S>();
    let a = d
        .put("tasks", &obj(json!({"title": "a", "done": false})))
        .unwrap();
    let b = d
        .put("tasks", &obj(json!({"title": "b", "priority": 3})))
        .unwrap();
    let c = d.put("tasks", &obj(json!({"title": "c"}))).unwrap();
    assert_eq!(a.fields, obj(json!({"title": "a", "done": false})));
    let (ia, ib, ic) = (
        from_hex(&a.id).unwrap(),
        from_hex(&b.id).unwrap(),
        from_hex(&c.id).unwrap(),
    );
    // ids strictly increase even though the clock never moved
    assert!(ia < ib && ib < ic, "ids must sort in creation order");
    assert_eq!(d.get("tasks", ib).unwrap().unwrap(), b);
    assert_eq!(d.count("tasks").unwrap(), 3);

    // scan: order, reverse, limit, paging
    let titles = |rs: Vec<Record>| {
        rs.iter()
            .map(|r| r.fields["title"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        titles(d.scan("tasks", Scan::default()).unwrap()),
        ["a", "b", "c"]
    );
    assert_eq!(
        titles(
            d.scan(
                "tasks",
                Scan {
                    reverse: true,
                    ..Default::default()
                }
            )
            .unwrap()
        ),
        ["c", "b", "a"]
    );
    assert_eq!(
        titles(
            d.scan(
                "tasks",
                Scan {
                    limit: 2,
                    ..Default::default()
                }
            )
            .unwrap()
        ),
        ["a", "b"]
    );
    assert_eq!(
        titles(
            d.scan(
                "tasks",
                Scan {
                    after: Some(ia),
                    ..Default::default()
                }
            )
            .unwrap()
        ),
        ["b", "c"]
    );
    assert_eq!(
        titles(
            d.scan(
                "tasks",
                Scan {
                    after: Some(ic),
                    reverse: true,
                    limit: 1
                }
            )
            .unwrap()
        ),
        ["b"]
    );

    // update merges, null removes, created is kept, updated moves
    d_set_now(&mut d, 5_000);
    let u = d
        .update("tasks", ib, &obj(json!({"done": true, "priority": null})))
        .unwrap();
    assert_eq!(u.fields, obj(json!({"title": "b", "done": true})));
    assert_eq!((u.created, u.updated), (b.created, 5_000));
    // an update may not break the schema either
    assert!(d
        .update("tasks", ib, &obj(json!({"title": null})))
        .unwrap_err()
        .to_string()
        .contains("required"));

    // delete
    assert!(d.delete("tasks", ia).unwrap());
    assert!(!d.delete("tasks", ia).unwrap());
    assert_eq!(d.get("tasks", ia).unwrap(), None);
    assert_eq!(
        titles(d.scan("tasks", Scan::default()).unwrap()),
        ["b", "c"]
    );

    // domains are separate ranges: "task" and "tasks2" must not bleed into "tasks"
    d.define("task", &task_schema()).unwrap();
    d.define("tasks2", &task_schema()).unwrap();
    d.put("task", &obj(json!({"title": "x"}))).unwrap();
    d.put("tasks2", &obj(json!({"title": "y"}))).unwrap();
    assert_eq!(d.count("tasks").unwrap(), 2);
    assert_eq!(d.domains().unwrap(), ["task", "tasks", "tasks2"]);
}

fn d_set_now<S: Store + craftworks_sdk::Reads>(d: &mut Db<S, FakeEnv>, now: u64) {
    // FakeEnv is reachable only through Db; rebuild around the same store is
    // overkill, so tests advance time through this helper.
    d.env_mut().now = now;
}

#[test]
fn crud_scan_update_delete_on_the_sorted_store() {
    suite::<MemStore>();
}
#[test]
fn the_same_suite_passes_on_an_unrelated_store() {
    suite::<VecStore>();
}
#[test]
fn the_same_suite_passes_on_the_real_tree() {
    suite::<TreeStore>();
}

#[test]
fn writes_that_violate_the_schema_are_refused() {
    let mut d = db::<MemStore>();
    let err =
        |d: &mut Db<MemStore, FakeEnv>, v: Value| d.put("tasks", &obj(v)).unwrap_err().to_string();
    assert!(err(&mut d, json!({"done": true})).contains("`title` is required"));
    assert!(err(&mut d, json!({"title": 7})).contains("must be text"));
    assert!(err(&mut d, json!({"title": "t", "priority": "high"})).contains("must be int"));
    assert!(err(&mut d, json!({"title": "t", "colour": "red"})).contains("not a field"));
    assert!(d
        .put("nope", &obj(json!({"title": "t"})))
        .unwrap_err()
        .to_string()
        .contains("no schema"));
    assert!(d
        .put("Bad Domain", &obj(json!({})))
        .unwrap_err()
        .to_string()
        .contains("domain"));
    assert_eq!(
        d.count("tasks").unwrap(),
        0,
        "a refused write must store nothing"
    );
}

#[test]
fn schemas_only_grow_so_old_records_stay_valid() {
    let mut d = db::<MemStore>();
    let old = d
        .put("tasks", &obj(json!({"title": "written before"})))
        .unwrap();
    let id = from_hex(&old.id).unwrap();

    // extend: append an optional field
    let mut v2 = task_schema();
    v2.fields.push(Field {
        name: "due".into(),
        kind: Kind::Time,
        required: false,
    });
    d.define("tasks", &v2).unwrap();
    // the old record reads under the new schema with no migration…
    assert_eq!(
        d.get("tasks", id).unwrap().unwrap().fields,
        obj(json!({"title": "written before"}))
    );
    // …and can take the new field
    let u = d.update("tasks", id, &obj(json!({"due": 99}))).unwrap();
    assert_eq!(u.fields["due"], json!(99));

    // anything but appending optional fields is refused
    let mut required = v2.clone();
    required.fields.push(Field {
        name: "owner".into(),
        kind: Kind::Text,
        required: true,
    });
    assert!(d
        .define("tasks", &required)
        .unwrap_err()
        .to_string()
        .contains("cannot be required"));
    let mut retyped = v2.clone();
    retyped.fields[1].kind = Kind::Int;
    assert!(d
        .define("tasks", &retyped)
        .unwrap_err()
        .to_string()
        .contains("only appended"));
    let mut dropped = v2.clone();
    dropped.fields.remove(0);
    assert!(d
        .define("tasks", &dropped)
        .unwrap_err()
        .to_string()
        .contains("only appended"));
    let mut renamed = v2.clone();
    renamed.type_name = "Todo".into();
    assert!(d
        .define("tasks", &renamed)
        .unwrap_err()
        .to_string()
        .contains("cannot become"));
    assert_eq!(
        d.schema("tasks").unwrap().unwrap(),
        v2,
        "a refused schema must not be stored"
    );
}

#[test]
fn an_old_reader_ignores_fields_a_newer_writer_added() {
    let v1 = task_schema();
    let mut v2 = v1.clone();
    v2.fields.push(Field {
        name: "due".into(),
        kind: Kind::Time,
        required: false,
    });
    let bytes = record::encode(&v2, &obj(json!({"title": "t", "due": 5})), 1, 1, &[0; 32]).unwrap();
    assert_eq!(
        record::decode(&v1, &bytes).unwrap().fields,
        obj(json!({"title": "t"}))
    );
}

#[test]
fn every_kind_round_trips_and_garbage_never_panics() {
    let s: Schema = serde_json::from_value(json!({"type": "All", "fields": [
        {"name": "t", "kind": "text"}, {"name": "i", "kind": "int"}, {"name": "f", "kind": "float"},
        {"name": "b", "kind": "bool"}, {"name": "w", "kind": "time"}, {"name": "x", "kind": "bytes"},
        {"name": "r", "kind": "ref"},
    ]}))
    .unwrap();
    let v = obj(
        json!({"t": "héllo", "i": -42, "f": 1.5, "b": true, "w": 1_700_000_000_000u64, "x": "00ff80", "r": "craftec://public/abc/posts/1"}),
    );
    let bytes = record::encode(&s, &v, 1, 2, &[9; 32]).unwrap();
    let d = record::decode(&s, &bytes).unwrap();
    assert_eq!(
        (d.fields, d.created, d.updated, d.author),
        (v, 1, 2, [9; 32])
    );
    // truncations and bit flips: an error or a value, never a panic
    let mut n = 1u32;
    for i in 0..20_000 {
        let mut m = bytes.clone();
        n = n.wrapping_mul(1664525).wrapping_add(1013904223);
        if i % 2 == 0 {
            m.truncate(n as usize % (bytes.len() + 1));
        } else {
            let at = n as usize % m.len();
            m[at] ^= 1 << (n >> 29);
        }
        let _ = record::decode(&s, &m);
    }
}

#[test]
fn keys_follow_the_architecture_keyspace() {
    let mut d = db::<MemStore>();
    let r = d.put("tasks", &obj(json!({"title": "k"}))).unwrap();
    let id = from_hex(&r.id).unwrap();
    let key = db::record_key("tasks", id);
    assert_eq!(&key[..7], b"\x01tasks\x00");
    assert_eq!(&key[7..], &id);
    // id = 8 B ms timestamp ‖ 4 B device ‖ 4 B tail
    assert_eq!(&id[..8], &1_000u64.to_be_bytes());
    assert_eq!(&id[8..12], b"dev1");
    assert!(Reads::get(d.store_mut(), &key).unwrap().is_some());
    assert!(
        Reads::get(d.store_mut(), b"\x00schema\x00tasks")
            .unwrap()
            .is_some(),
        "the schema lives in the store, in the system range"
    );
}
