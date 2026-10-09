//! An id a read cannot address answers ABSENT, not a refusal.
//!
//! The ids reaching `get` come from outside the program: a `lastOpened` value
//! in browser storage written by an older build, a pasted link, a record
//! deleted on another device. For every one of those the honest answer is
//! *absent*, which `get` already has a representation for — it returns `None`
//! for an id that parses but is not there.
//!
//! Making "malformed" a different CONTROL-FLOW outcome from "missing" forces
//! every caller holding an outside id into a `try`, and the one that forgets
//! is the one that crashes. That is the library deciding the caller's control
//! flow (craftworks-sdk#118).
//!
//! **One shape in the issue is now stale and is corrected here.** It lists a
//! 64-character id among the malformed inputs. Since #122 a 64-hex id is a
//! VALID id — a record keyed under a parent — so the table below tests it as
//! valid-but-absent rather than as nonsense, and covers 48 and 65 instead.
//! The stale-id case is also no longer hypothetical: a component id stored
//! before #122 is 32 hex where the domain now wants 64.
use craftworks_sdk::id::Loc;
use craftworks_sdk::*;
use testkit::MemStore;
use serde_json::{json, Map, Value};

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

/// The same deliberately different second store the rest of the suite uses, so
/// this cannot be relying on anything but the four operations.
#[derive(Default)]
struct VecStore(Vec<(Vec<u8>, Vec<u8>)>);
impl Store for VecStore {
    fn apply_batch(&mut self, edits: &[(Vec<u8>, store::Edit)]) -> Result<(), craftworks_sdk::Refused> {
        for (k, e) in edits {
            self.0.retain(|(a, _)| a != k);
            if let store::Edit::Put(v) = e {
                self.0.push((k.clone(), v.clone()));
            }
        }
        Ok(())
    }
}
impl Reads for VecStore {
    fn get(&mut self, key: &[u8]) -> store::Read<Option<Vec<u8>>> {
        Ok(self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()))
    }
    fn scan(
        &mut self,
        lo: &[u8],
        hi: &[u8],
        reverse: bool,
        limit: usize,
    ) -> store::Read<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut out: Vec<_> = self
            .0
            .iter()
            .filter(|(k, _)| k.as_slice() >= lo && k.as_slice() < hi)
            .cloned()
            .collect();
        out.sort();
        if reverse {
            out.reverse();
        }
        out.truncate(limit);
        Ok(out)
    }
    fn root(&mut self) -> store::Read<[u8; 32]> {
        Ok([0u8; 32])
    }
    fn changes_since(
        &mut self,
        _from: [u8; 32],
        _lo: &[u8],
        _hi: &[u8],
        _max: u32,
    ) -> store::Read<Delta> {
        unreachable!("this suite never asks for a delta")
    }
}

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

fn plain() -> Schema {
    Schema {
        type_name: "Task".into(),
        fields: vec![Field {
            name: "title".into(),
            kind: Kind::Text,
            required: true,
        }],
        parent: None,
    }
}

fn parented() -> Schema {
    Schema {
        type_name: "Component".into(),
        fields: vec![Field {
            name: "pid".into(),
            kind: Kind::Text,
            required: true,
        }],
        parent: Some("pid".into()),
    }
}

/// Everything that arrives from outside and is not a usable id here.
const NONSENSE: &[(&str, &str)] = &[
    ("the empty string", ""),
    ("a truncated id", "0123456789abcdef0123456789abcde"),
    ("one character too many", "0123456789abcdef0123456789abcdef0"),
    ("not hex at all", "nope"),
    ("right length, wrong alphabet", "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
    ("half a parented id", "0123456789abcdef0123456789abcdef0123456789abcdef"),
    ("a parented id plus one", "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0"),
    ("a whole pasted sentence", "have you seen this project"),
];

fn table<S: Store + Reads + Default>(label: &str) {
    let mut d: Db<S, FakeEnv> = Db::new(S::default(), FakeEnv { now: 1_000, n: 1 }, *b"dev1");
    d.define("tasks", &plain()).unwrap();
    d.define("component", &parented()).unwrap();
    let real = d.put("tasks", &obj(json!({"title": "here"}))).unwrap();

    for (what, id) in NONSENSE {
        let got = craftworks_sdk::id::loc_from_hex(id);
        assert!(got.is_none(), "[{label}] {what} should not parse as an id");
    }

    // A VALID id that was never written is absent. This is the control: it is
    // what "absent" already looked like, and the whole point is that nonsense
    // now looks the same.
    let never = craftworks_sdk::id::loc_from_hex("ffffffffffffffffffffffffffffffff").unwrap();
    assert!(
        d.get("tasks", never).unwrap().is_none(),
        "[{label}] a well-formed id that was never written must be absent",
    );
    // And the record that IS there still comes back, or this test would pass
    // on a `get` that returned `None` for everything.
    assert!(
        d.get("tasks", craftworks_sdk::id::loc_from_hex(&real.id).unwrap())
            .unwrap()
            .is_some(),
        "[{label}] THE CONTROL: a real id still reads",
    );

    // A 64-hex id IS valid since #122 — it addresses a record under a parent.
    // In a domain with no parent it cannot address anything, and that is a
    // READ that cannot be satisfied, so it is absent rather than a refusal.
    let parented_id = craftworks_sdk::id::loc_from_hex(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .unwrap();
    assert!(
        d.get("tasks", parented_id).unwrap().is_none(),
        "[{label}] a parented id in a domain with no parent is absent, not a refusal",
    );

    // The mirror, and the case #122 actually created: an id stored before that
    // change is 32 hex where the domain now wants 64.
    let stale = Loc::bare(craftworks_sdk::id::from_hex("0123456789abcdef0123456789abcdef").unwrap());
    assert!(
        d.get("component", stale).unwrap().is_none(),
        "[{label}] a pre-#122 bare id must read as absent, not crash the app on load",
    );

    // A WRITE still refuses, and the asymmetry is the point: answering
    // "nothing to do" to a delete aimed at an id nobody can resolve would hide
    // the caller's mistake rather than report it.
    assert!(
        d.delete("component", stale).is_err(),
        "[{label}] a write against an unaddressable id must still refuse",
    );
    assert!(
        d.update("component", stale, &Map::new()).is_err(),
        "[{label}] an update against an unaddressable id must still refuse",
    );

    // And the domain is still checked. "No such domain" is a programming
    // error, not an outside id, and must not be hidden behind `None`.
    assert!(
        d.get("nope", never).is_err(),
        "[{label}] a read of a domain that does not exist is still an error",
    );
}

#[test]
fn an_unaddressable_id_reads_as_absent_over_a_sorted_store() {
    table::<MemStore>("MemStore");
}

#[test]
fn an_unaddressable_id_reads_as_absent_over_a_different_store() {
    table::<VecStore>("VecStore");
}
