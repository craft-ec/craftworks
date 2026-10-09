//! Does a tree this SDK writes satisfy the rule a HOST enforces?
//!
//! The Block contract runs `freenet_prolly::boundary::check_node` on every
//! `TREE_NODE` it is offered (freenet-contracts, `well_formed`). So "the tests
//! pass" says nothing about whether a node this SDK emits would be KEPT — the
//! suite never asked the question the network asks. It bit us once already:
//! the SDK pinned the tree library from before the parity rule and would have
//! shipped trees every host refuses, and nothing in this repo noticed. The
//! panel's `prollyRev` did.
//!
//! This file asks it directly, walking every block a store holds — including
//! the superseded nodes, which stay and which a reader can still be handed.

use craftworks_sdk::store::{Edit, Reads, Store};
use craftworks_sdk::{Db, TreeStore};
use freenet_prolly::boundary::check_node;
use freenet_prolly::node::{Node, MAX_INLINE};
use serde_json::{json, Map, Value};

/// Every node in the store, checked the way a host checks it.
fn every_node_passes(store: &TreeStore) -> (usize, usize) {
    let (mut nodes, mut others) = (0, 0);
    for (cid, bytes) in store.blocks() {
        match Node::parse(bytes) {
            Ok(node) => {
                nodes += 1;
                check_node(&node).unwrap_or_else(|e| {
                    panic!(
                        "a node this SDK wrote would be REFUSED by a host: {e:?} (block {})",
                        craftworks_sdk::hex(cid)
                    )
                });
            }
            // Value blocks: bytes too large to inline. Not nodes, not this
            // rule's business — but counted, so "every node passed" cannot be
            // true because there were no nodes.
            Err(_) => others += 1,
        }
    }
    (nodes, others)
}

fn rng(seed: u64) -> impl FnMut() -> u64 {
    let mut s = seed | 1;
    move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    }
}

/// Enough entries to force several levels, so BRANCHES are checked and not
/// only a single leaf — the split rule is about interiors. Chosen as the
/// smallest that clears the node floor below with room to spare: the suite ran
/// for minutes in a debug build, and a fixture that is bigger than the property
/// it demonstrates is paid for on every run by everyone.
const ENTRIES: u64 = 2000;

#[test]
fn every_node_a_tall_tree_holds_would_be_accepted_by_a_host() {
    let mut store = TreeStore::new();
    let mut r = rng(7);
    // Written as ONE batch, not entry by entry. Entry by entry the store also
    // accumulates a superseded node per write, which multiplies the work by the
    // number of entries without adding a single new SHAPE — and it took this
    // suite to minutes in a debug build. The history case has its own test
    // below, where the superseded nodes are the point.
    let batch: Vec<(Vec<u8>, Edit)> = (0..ENTRIES)
        .map(|i| {
            let v = vec![(r() % 251) as u8; 24 + (r() % 400) as usize];
            (format!("k/{i:08}").into_bytes(), Edit::Put(v))
        })
        .collect();
    store.apply_batch(&batch).expect("the store took the write");

    // The property the fixture exists for is DEPTH: the split rule is about
    // interiors, so a run that only ever saw leaves would pass while saying
    // nothing. Asserted on the height rather than on the entry count, because
    // the entry count is a proxy that the tree library is free to change.
    let h = store.stats().height;
    assert!(
        h >= 3,
        "the fixture must have branches above branches, got height {h}"
    );
    let (nodes, others) = every_node_passes(&store);
    assert!(
        nodes > 20,
        "a tall tree must hold many nodes, found {nodes}"
    );
    println!("height {h}: checked {nodes} nodes ({others} value blocks) — all accepted");
}

#[test]
fn nodes_holding_referenced_values_are_accepted_too() {
    // Values over MAX_INLINE are stored as references. Closing a leaf that
    // holds one needs the value's BYTES, so this is the case where a store that
    // did not keep them would either fail or, worse, build a node without the
    // parity the host demands.
    let mut store = TreeStore::new();
    let mut r = rng(11);
    for i in 0..60u64 {
        let big = vec![(r() % 251) as u8; MAX_INLINE + 1 + (r() % 2048) as usize];
        store.put(format!("big/{i:06}").as_bytes(), &big).expect("the store took the write");
    }
    let (nodes, others) = every_node_passes(&store);
    assert!(nodes >= 1);
    assert!(
        others > 0,
        "this fixture is pointless if nothing was referenced"
    );
    // And the values really read back, which is the other half: a node that
    // passes the host's check while its value block is gone is a tree that
    // validates and cannot be read.
    let mut r = rng(11);
    for i in 0..60u64 {
        let want = vec![(r() % 251) as u8; MAX_INLINE + 1 + (r() % 2048) as usize];
        assert_eq!(
            Reads::get(&mut store, format!("big/{i:06}").as_bytes()),
            Ok(Some(want))
        );
    }
    println!("checked {nodes} nodes over {others} referenced value blocks");
}

#[test]
fn deletes_and_rewrites_leave_no_refusable_node_behind() {
    // Superseded nodes stay in the store, and a reader can be handed one. If an
    // edit path could emit a node that fails the rule, checking only the
    // current root would never see it.
    let mut store = TreeStore::new();
    let mut r = rng(13);
    for i in 0..500u64 {
        store.put(format!("k/{i:08}").as_bytes(), &[(r() % 251) as u8; 40]).expect("the store took the write");
    }
    let mut batch: Vec<(Vec<u8>, Edit)> = Vec::new();
    for i in (0..500u64).step_by(3) {
        batch.push((format!("k/{i:08}").into_bytes(), Edit::Delete));
    }
    for i in (1..500u64).step_by(7) {
        batch.push((format!("k/{i:08}").into_bytes(), Edit::Put(vec![9u8; 900])));
    }
    store.apply_batch(&batch).expect("the store took the write");
    let (nodes, _) = every_node_passes(&store);
    println!("checked {nodes} nodes across the whole history of edits");
}

#[test]
fn a_database_built_through_the_public_api_writes_acceptable_nodes() {
    // The route an app actually takes: a schema, records, the Db surface. The
    // store tests above reach the tree directly, which is not what ships.
    struct FixedEnv(u64, u32);
    impl craftworks_sdk::Env for FixedEnv {
        fn now_ms(&mut self) -> u64 {
            self.0 += 1;
            self.0
        }
        fn rand32(&mut self) -> u32 {
            self.1 = self.1.wrapping_mul(1664525).wrapping_add(1013904223);
            self.1
        }
    }
    let mut db = Db::new(
        TreeStore::new(),
        FixedEnv(1_700_000_000_000, 7),
        [1, 2, 3, 4],
    );
    db.define(
        "tasks",
        &serde_json::from_value(json!({
            "type": "Task",
            "fields": [
                { "name": "title", "kind": "text", "required": true },
                { "name": "body", "kind": "text" }
            ]
        }))
        .unwrap(),
    )
    .expect("the schema is valid");
    for i in 0..400usize {
        let fields: Map<String, Value> = serde_json::from_value(json!({
            "title": format!("t{i}"),
            "body": "x".repeat(200 + i % 900),
        }))
        .unwrap();
        db.put("tasks", &fields).expect("a valid record");
    }
    let (nodes, others) = every_node_passes(db.store());
    assert!(nodes > 5, "found only {nodes} nodes");
    println!("Db route: checked {nodes} nodes ({others} value blocks)");
}
