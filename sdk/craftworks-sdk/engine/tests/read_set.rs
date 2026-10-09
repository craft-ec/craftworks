//! THE READ SET (sdk#148): a write states what it READ, and the engine checks
//! it against the tree the ops land on, inside that same apply (R0). A read
//! that no longer holds applies NOTHING and ends `Conflict`, beside an
//! `Effect::Conflicted` naming the key and what it holds now.
//!
//! R1 a conflicting write never changes the tree;
//! R2 a write whose reads hold applies exactly as the same write with none;
//! R3 two updates from one stale view: one applies, the other conflicts;
//! R4 `create_at` twice on one key: one creates, the other conflicts;
//! and the architect's attack: a value stored by REFERENCE is compared by its
//! reference, never fetched to be compared; a read key whose path is not held
//! PARKS (never a Conflict) and is checked again on the root it resumes on.

use engine::{leaf_hash, ClientId, Effect, Event, Expect, LeafForm, Op, Params, State, WriteId};
use freenet_prolly::Cid;
use std::collections::BTreeMap;

mod common;
use common::{Harness, Store};

/// A write with these reads. Every op key the test does NOT read is FORCED
/// (`Expect::Any`, sdk#235 W8): these tests are about the reads they declare —
/// often of a key the write does not touch — and a write must name every key
/// it changes, so the rest are named as "whatever it holds". The declared
/// reads are checked exactly as before.
fn write(id: u64, ops: &[(&[u8], Option<&[u8]>)], reads: Vec<(Vec<u8>, Expect)>) -> Event {
    let ops: Vec<(Vec<u8>, Op)> = ops
        .iter()
        .map(|(k, v)| (k.to_vec(), v.map_or(Op::Delete, |v| Op::Put(v.to_vec()))))
        .collect();
    let mut reads = reads;
    for (k, _) in &ops {
        if !reads.iter().any(|(r, _)| r == k) {
            reads.push((k.clone(), Expect::Any));
        }
    }
    Event::Write {
        client: ClientId(1),
        write_id: WriteId(id),
        ops,
        reads,
        deferred: false,
    }
}

fn states(fx: &[Effect], id: u64) -> Vec<State> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::Notify { write_id, state, .. } if write_id.0 == id => Some(*state),
            _ => None,
        })
        .collect()
}

fn conflicted(fx: &[Effect], id: u64) -> Option<(Vec<u8>, Option<LeafForm>)> {
    fx.iter().find_map(|f| match f {
        Effect::Conflicted { write_id, key, current, .. } if write_id.0 == id => Some((key.clone(), current.clone())),
        _ => None,
    })
}

/// Confirm every put and the head, until nothing is left.
fn settle(h: &mut Harness, mut fx: Vec<Effect>) -> Vec<Effect> {
    let mut all = fx.clone();
    for _ in 0..100 {
        let mut next = Vec::new();
        for f in &fx {
            match f {
                Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } => {
                    next.extend(h.step(Event::PutConfirmed(*id)))
                }
                Effect::UpdateHead { seq, .. } => next.extend(h.step(Event::HeadConfirmed(*seq))),
                // Answered inside `Harness::step` (`common::answer_held`) for every member the node holds, as the page does.
                Effect::ConfirmHeld { .. } => {}
                other => common::no_answer_owed(other),
            }
        }
        if next.is_empty() {
            return all;
        }
        all.extend(next.iter().cloned());
        fx = next;
    }
    panic!("never settled");
}

fn fresh() -> Harness {
    Harness::new(Params::default(), Store::fresh())
}

/// What key `k` holds in the tree at `root`, as bytes (a reference resolved).
fn value_at(store: &Store, root: &Cid, k: &[u8]) -> Option<Vec<u8>> {
    let v = freenet_prolly::read::get(store, root, k).expect("the tree is held")?;
    Some(freenet_prolly::range::read_value(store, v).expect("the value is held").to_vec())
}

#[test]
fn a_write_whose_reads_hold_applies_exactly_as_a_blind_one() {
    let (mut a, mut b) = (fresh(), fresh());
    for h in [&mut a, &mut b] {
        let fx = h.step(write(1, &[(b"k", Some(b"v1"))], vec![]));
        settle(h, fx);
    }
    let fx = a.step(write(2, &[(b"k", Some(b"v2")), (b"j", Some(b"x"))], vec![]));
    settle(&mut a, fx);
    let reads = vec![(b"k".to_vec(), Expect::Value(leaf_hash(b"v1"))), (b"j".to_vec(), Expect::Absent)];
    let fx = b.step(write(2, &[(b"k", Some(b"v2")), (b"j", Some(b"x"))], reads));
    let all = settle(&mut b, fx);
    assert!(states(&all, 2).contains(&State::Published), "a write whose reads held did not publish");
    assert_eq!(a.published_root(), b.published_root(), "reads that hold changed what the write produced");
}

#[test]
fn a_write_whose_read_changed_applies_nothing_and_says_what_is_there() {
    let mut h = fresh();
    let fx = h.step(write(1, &[(b"k", Some(b"v1"))], vec![]));
    settle(&mut h, fx);
    let before = h.published_root();
    // It read v0 — a copy from before v1 landed.
    let fx = h.step(write(2, &[(b"k", Some(b"v2")), (b"j", Some(b"x"))], vec![(b"k".to_vec(), Expect::Value(leaf_hash(b"v0")))]));
    assert_eq!(states(&fx, 2), vec![State::Conflict]);
    assert_eq!(conflicted(&fx, 2), Some((b"k".to_vec(), Some(LeafForm::Inline(b"v1".to_vec())))));
    assert!(!fx.iter().any(|f| matches!(f, Effect::PutBlock { .. } | Effect::UpdateHead { .. })), "a conflict wrote something");
    assert_eq!(h.root(), before, "R1 — a conflicting write changed the tree");
    // And the engine is not left busy: the next write goes through.
    let fx = h.step(write(3, &[(b"k", Some(b"v3"))], vec![(b"k".to_vec(), Expect::Value(leaf_hash(b"v1")))]));
    assert!(settle(&mut h, fx).iter().any(|f| matches!(f, Effect::Notify { write_id, state: State::Published, .. } if write_id.0 == 3)));
}

#[test]
fn create_at_twice_on_one_key_creates_once_and_conflicts_once() {
    let mut h = fresh();
    let fx = h.step(write(1, &[(b"id/7", Some(b"first"))], vec![(b"id/7".to_vec(), Expect::Absent)]));
    assert!(states(&settle(&mut h, fx), 1).contains(&State::Published));
    let fx = h.step(write(2, &[(b"id/7", Some(b"second"))], vec![(b"id/7".to_vec(), Expect::Absent)]));
    assert_eq!(states(&fx, 2), vec![State::Conflict], "R4: a second create over an existing key applied");
    assert_eq!(value_at(&h.store, &h.published_root(), b"id/7").as_deref(), Some(&b"first"[..]));
}

#[test]
fn two_updates_from_one_stale_view_one_applies_and_the_other_conflicts() {
    let mut h = fresh();
    let fx = h.step(write(1, &[(b"n", Some(b"1"))], vec![]));
    settle(&mut h, fx);
    // Two tabs both read "1" and both write "2".
    let read = vec![(b"n".to_vec(), Expect::Value(leaf_hash(b"1")))];
    let fx = h.step(write(2, &[(b"n", Some(b"2a"))], read.clone()));
    assert!(states(&settle(&mut h, fx), 2).contains(&State::Published));
    let fx = h.step(write(3, &[(b"n", Some(b"2b"))], read));
    assert_eq!(states(&fx, 3), vec![State::Conflict], "R3: both updates from one view applied (#143)");
    assert_eq!(value_at(&h.store, &h.published_root(), b"n").as_deref(), Some(&b"2a"[..]));
}

#[test]
fn present_holds_for_any_value_and_fails_on_an_absent_key() {
    let mut h = fresh();
    let fx = h.step(write(1, &[(b"parent", Some(b"p"))], vec![]));
    settle(&mut h, fx);
    let fx = h.step(write(2, &[(b"parent", Some(b"edited"))], vec![]));
    settle(&mut h, fx);
    // A child needs its parent to EXIST, not to be unchanged.
    let fx = h.step(write(3, &[(b"parent/child", Some(b"c"))], vec![(b"parent".to_vec(), Expect::Present)]));
    assert!(states(&settle(&mut h, fx), 3).contains(&State::Published), "Present failed on an edited parent");
    let fx = h.step(write(4, &[(b"gone/child", Some(b"c"))], vec![(b"gone".to_vec(), Expect::Present)]));
    assert_eq!(states(&fx, 4), vec![State::Conflict]);
    assert_eq!(conflicted(&fx, 4), Some((b"gone".to_vec(), None)));
}

/// A value over the inline limit lives in its own block. Its expectation is
/// the hash of its REFERENCE, so the engine compares without that block —
/// here, with the block gone from the node entirely.
#[test]
fn a_large_value_is_compared_by_its_reference_and_never_fetched() {
    let mut h = fresh();
    let big = vec![7u8; 5_000];
    let fx = h.step(write(1, &[(b"doc", Some(&big))], vec![]));
    settle(&mut h, fx);
    let (form, stored) = freenet_prolly::node::Value::for_bytes(&big);
    let freenet_prolly::node::Value::Ref { cid, .. } = form else { panic!("5000 B was inline") };
    assert!(stored.is_some());
    h.store.forget(cid);
    let fx = h.step(write(2, &[(b"other", Some(b"x"))], vec![(b"doc".to_vec(), Expect::Value(leaf_hash(&big)))]));
    assert!(!fx.iter().any(|f| matches!(f, Effect::FetchBlock { id, .. } if *id == cid)), "the value was fetched to be compared");
    assert!(states(&settle(&mut h, fx), 2).contains(&State::Published));
    let mut other = big.clone();
    other[0] = 8;
    let fx = h.step(write(3, &[(b"other", Some(b"y"))], vec![(b"doc".to_vec(), Expect::Value(leaf_hash(&other)))]));
    assert_eq!(states(&fx, 3), vec![State::Conflict]);
    assert!(matches!(conflicted(&fx, 3), Some((_, Some(LeafForm::Ref { cid: c, len: 5_000 }))) if c == cid));
}

/// The leaf hash tells an INLINE value from a REFERENCE: an inline value
/// whose bytes are exactly a reference's `cid ‖ len` is a different value and
/// must not satisfy an expectation of the reference (and the reverse).
#[test]
fn an_inline_value_spelling_a_reference_is_not_that_reference() {
    let mut h = fresh();
    let big = vec![9u8; 5_000];
    let fx = h.step(write(1, &[(b"doc", Some(&big))], vec![]));
    settle(&mut h, fx);
    let freenet_prolly::node::Value::Ref { cid, len } = freenet_prolly::node::Value::for_bytes(&big).0 else { panic!("inline") };
    let mut spelled = cid.to_vec();
    spelled.extend_from_slice(&len.to_le_bytes());
    assert!(matches!(freenet_prolly::node::Value::for_bytes(&spelled).0, freenet_prolly::node::Value::Inline(_)));
    let fx = h.step(write(2, &[(b"other", Some(b"x"))], vec![(b"doc".to_vec(), Expect::Value(leaf_hash(&spelled)))]));
    assert_eq!(states(&fx, 2), vec![State::Conflict], "an inline value spelling a reference satisfied it");
}

/// A cold read that is WRONG from the start: it parks (nothing proved yet),
/// and on resume — the reads carried in the parked write — it conflicts.
#[test]
fn a_cold_read_that_is_wrong_parks_then_conflicts_on_resume() {
    // LIVE ONLY (R-b): a parked write waits in the page's queue, which is
    // page memory and never in a context.
    let mut h = fresh();
    let ops: Vec<(Vec<u8>, Vec<u8>)> = (0..400).map(|i| (format!("k/{i:04}").into_bytes(), vec![b'a'; 200])).collect();
    let op_refs: Vec<(&[u8], Option<&[u8]>)> = ops.iter().map(|(k, v)| (k.as_slice(), Some(v.as_slice()))).collect();
    let fx = h.step(write(1, &op_refs, vec![]));
    settle(&mut h, fx);
    let root = h.published_root();
    for c in held_blocks(&h.store, &root).iter().filter(|c| **c != root) {
        h.store.forget(*c);
    }
    let fx = h.step(write(2, &[(b"z", Some(b"1"))], vec![(b"k/0123".to_vec(), Expect::Value(leaf_hash(b"not what is there")))]));
    assert!(states(&fx, 2).is_empty(), "a cold read conflicted before its path was read");
    let mut out = fx;
    let mut all = Vec::new();
    for _ in 0..20 {
        let asks: Vec<Cid> = out.iter().filter_map(|f| if let Effect::FetchBlock { id, .. } = f { Some(*id) } else { None }).collect();
        all.extend(out.iter().cloned());
        if asks.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for id in asks {
            h.store.remember(&id);
            let bytes = freenet_prolly::store::Blocks::get(&h.store, &id).expect("held").to_vec();
            next.extend(h.step(Event::BlockArrived { id, bytes }));
        }
        out = next;
    }
    assert_eq!(states(&all, 2), vec![State::Conflict], "the parked write's reads were not checked on resume");
    assert_eq!(h.published_root(), root);
}

/// A read key whose path is not held PARKS the write (never a Conflict:
/// nothing proved it differs), and it is checked on the root it resumes on.
/// LIVE ONLY (R-b): the write and its reads wait in the page's queue, which
/// is page memory and never in a context.
#[test]
fn a_read_whose_path_is_not_held_parks_and_is_checked_when_it_resumes() {
    let mut h = fresh();
    let ops: Vec<(Vec<u8>, Vec<u8>)> = (0..400).map(|i| (format!("k/{i:04}").into_bytes(), vec![b'a'; 200])).collect();
    let op_refs: Vec<(&[u8], Option<&[u8]>)> = ops.iter().map(|(k, v)| (k.as_slice(), Some(v.as_slice()))).collect();
    let fx = h.step(write(1, &op_refs, vec![]));
    settle(&mut h, fx);
    // Forget every block but the root: the read's path is cold.
    let root = h.published_root();
    let held: Vec<Cid> = held_blocks(&h.store, &root);
    for c in held.iter().filter(|c| **c != root) {
        h.store.forget(*c);
    }
    let fx = h.step(write(2, &[(b"z", Some(b"1"))], vec![(b"k/0123".to_vec(), Expect::Value(leaf_hash(&[b'a'; 200])))]));
    assert!(states(&fx, 2).is_empty(), "a cold read was answered {:?} instead of parking", states(&fx, 2));
    let fetches: Vec<Cid> = fx.iter().filter_map(|f| if let Effect::FetchBlock { id, .. } = f { Some(*id) } else { None }).collect();
    assert!(!fetches.is_empty(), "parked without asking for the read's path");
    // Deliver until it resolves.
    let mut out = fx;
    for _ in 0..20 {
        let asks: Vec<Cid> = out.iter().filter_map(|f| if let Effect::FetchBlock { id, .. } = f { Some(*id) } else { None }).collect();
        if asks.is_empty() {
            break;
        }
        let mut next = Vec::new();
        for id in asks {
            h.store.remember(&id);
            let bytes = freenet_prolly::store::Blocks::get(&h.store, &id).expect("held").to_vec();
            next.extend(h.step(Event::BlockArrived { id, bytes }));
        }
        out = next;
    }
    assert!(settle(&mut h, out).iter().any(|f| matches!(f, Effect::Notify { write_id, state: State::Published, .. } if write_id.0 == 2)), "the resumed write did not publish");
}

/// Every block reachable from `root`.
fn held_blocks(store: &Store, root: &Cid) -> Vec<Cid> {
    let mut out = vec![*root];
    let mut i = 0;
    while i < out.len() {
        let id = out[i];
        if let Some(bytes) = freenet_prolly::store::Blocks::get(store, &id) {
            if let Ok(node) = freenet_prolly::node::Node::parse(bytes) {
                if !node.is_leaf() {
                    for c in 0..node.len() {
                        out.push(node.child(c).0);
                    }
                }
            }
        }
        i += 1;
    }
    out
}

/// THE MODEL: two clients update a few keys from VIEWS that go stale — each
/// reads its own last-seen value, the other writes in between. Checked on
/// every write against the real tree: a Published write's read held on the
/// tree it applied to (R0), a Conflict left the tree as it was (R1) and named
/// what is there now, and at the end the tree is exactly the published writes.
#[test]
fn model_stale_views_never_overwrite_and_conflicts_name_the_truth() {
    let (mut published, mut conflicts) = (0usize, 0usize);
    for seed in 1..=40u64 {
        let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let mut h = fresh();
        // A fresh device's root is the EMPTY LEAF, a constant of the format
        // that no store holds until the first commit publishes.
        let empty = h.published_root();
        let at = |h: &Harness, root: &Cid, k: &[u8]| if *root == empty { None } else { value_at(&h.store, root, k) };
        let keys: Vec<Vec<u8>> = (0..3).map(|i| format!("r/{i}").into_bytes()).collect();
        // What each client last saw of each key (None: absent).
        let mut views: [BTreeMap<Vec<u8>, Option<Vec<u8>>>; 2] = Default::default();
        for v in &mut views {
            for k in &keys {
                v.insert(k.clone(), None);
            }
        }
        for n in 0..60u64 {
            let c = (next() % 2) as usize;
            let k = keys[(next() % 3) as usize].clone();
            let seen = views[c][&k].clone();
            let want = match &seen {
                None => Expect::Absent,
                Some(v) => Expect::Value(leaf_hash(v)),
            };
            let new = format!("{seed}-{n}-{c}").into_bytes();
            let before = h.published_root();
            let truth = at(&h, &before, &k);
            let id = 1_000 * seed + n;
            let fx = h.step(write(id, &[(&k, Some(&new))], vec![(k.clone(), want)]));
            let all = settle(&mut h, fx);
            let st = states(&all, id);
            if st.contains(&State::Published) {
                // R0: its read held on the tree it applied to.
                assert_eq!(truth, seen, "seed {seed}: a write published over a value it had not read");
                assert_eq!(at(&h, &h.published_root(), &k), Some(new.clone()));
                published += 1;
                views[c].insert(k, Some(new));
            } else {
                assert_eq!(st, vec![State::Conflict], "seed {seed}: neither published nor conflicted: {st:?}");
                assert_ne!(truth, seen, "seed {seed}: a read that held was called a conflict");
                assert_eq!(h.published_root(), before, "seed {seed}: R1 — a conflict changed the tree");
                let (ck, current) = conflicted(&all, id).expect("a Conflict without its Conflicted");
                assert_eq!(ck, k);
                assert_eq!(current, truth.as_deref().map(|t| LeafForm::Inline(t.to_vec())), "seed {seed}: the conflict misreported what is there");
                conflicts += 1;
                // The app learns the truth from the conflict itself.
                views[c].insert(k, truth);
            }
        }
    }
    println!("read-set model: {published} published, {conflicts} conflicts");
    assert!(published > 500 && conflicts > 200, "the model did not reach both outcomes: {published} / {conflicts}");
}

/// main's ruling on sdk#148's SDK slice: a write whose value ALREADY is what
/// the tree holds at every key whose read no longer holds is a success, not a
/// Conflict. Two tabs define one schema from one Absent base: both succeed.
/// Control: the same race with DIFFERENT bytes still conflicts.
#[test]
fn a_write_that_is_already_there_at_its_conflicting_key_succeeds() {
    let mut h = fresh();
    let fx = h.step(write(1, &[(b"schema/x", Some(b"S"))], vec![(b"schema/x".to_vec(), Expect::Absent)]));
    assert!(states(&settle(&mut h, fx), 1).contains(&State::Published));
    // The second tab read Absent too, and writes EXACTLY what is there.
    let fx = h.step(write(2, &[(b"schema/x", Some(b"S"))], vec![(b"schema/x".to_vec(), Expect::Absent)]));
    let all = settle(&mut h, fx);
    assert!(conflicted(&all, 2).is_none(), "an equal write was told Conflict");
    assert!(states(&all, 2).contains(&State::Published), "an equal write was not Published: {:?}", states(&all, 2));
    // The control: different bytes from the same stale read conflict.
    let fx = h.step(write(3, &[(b"schema/x", Some(b"T"))], vec![(b"schema/x".to_vec(), Expect::Absent)]));
    assert_eq!(states(&fx, 3), vec![State::Conflict]);
    // A delete of what is ALREADY gone, read Present: the same rule.
    let fx = h.step(write(4, &[(b"gone", None)], vec![(b"gone".to_vec(), Expect::Present)]));
    assert!(conflicted(&settle(&mut h, fx), 4).is_none(), "deleting what is already absent was told Conflict");
}

/// The no-op path still runs the compare (the architect's #4): a delete of a
/// record read ABSENT that someone has since CREATED is a Conflict — never a
/// silent "nothing to do".
#[test]
fn a_delete_of_a_record_read_absent_that_now_exists_conflicts() {
    let mut h = fresh();
    let fx = h.step(write(1, &[(b"r", Some(b"made elsewhere"))], vec![]));
    settle(&mut h, fx);
    let fx = h.step(write(2, &[(b"r", None)], vec![(b"r".to_vec(), Expect::Absent)]));
    assert_eq!(states(&fx, 2), vec![State::Conflict], "a delete over a record created since its read went through");
    assert_eq!(value_at(&h.store, &h.published_root(), b"r").as_deref(), Some(&b"made elsewhere"[..]));
}

/// sdk#145, THE LOST ANSWER: w3 (title t1, read the record at v0) and w4
/// (body b1, read it at w3's value) both publish. w3's answer was lost, and
/// w3 is re-sent byte for byte, WITH ITS READS. Its read no longer holds and
/// its value is not what is there, so it is a Conflict: it applies NOTHING
/// (root unchanged, no block, no head), and w4's body stands. The engine
/// remembers no published id; the read set is what refuses the re-send.
/// (Without reads a re-send is still applied as new: the named residual, a
/// store-level blind write only. Every `Db` write carries its reads.)
/// Control: a NEW write with a read that holds applies — refusal is by what
/// the write read, not by its bytes.
#[test]
fn a_re_sent_write_whose_answer_was_lost_applies_nothing_and_undoes_nothing() {
    let mut h = fresh();
    let fx = h.step(write(1, &[(b"rec", Some(b"t0 b0"))], vec![]));
    settle(&mut h, fx);
    let w3 = |id| write(id, &[(b"rec", Some(b"t1 b0"))], vec![(b"rec".to_vec(), Expect::Value(leaf_hash(b"t0 b0")))]);
    let fx = h.step(w3(3));
    assert!(states(&settle(&mut h, fx), 3).contains(&State::Published));
    let fx = h.step(write(4, &[(b"rec", Some(b"t1 b1"))], vec![(b"rec".to_vec(), Expect::Value(leaf_hash(b"t1 b0")))]));
    assert!(states(&settle(&mut h, fx), 4).contains(&State::Published));
    let before = h.published_root();
    let fx = h.step(w3(3));
    assert_eq!(states(&fx, 3), vec![State::Conflict], "the re-send was not refused");
    assert!(!fx.iter().any(|f| matches!(f, Effect::PutBlock { .. } | Effect::UpdateHead { .. })), "the re-send wrote something");
    let all = settle(&mut h, fx);
    assert!(!states(&all, 3).contains(&State::Published));
    assert_eq!(h.published_root(), before, "the re-send moved the tree");
    assert_eq!(value_at(&h.store, &h.published_root(), b"rec").as_deref(), Some(&b"t1 b1"[..]), "w4 was undone");
    // The control: a NEW write, its read holding, applies.
    let fx = h.step(write(5, &[(b"rec", Some(b"t1 b0"))], vec![(b"rec".to_vec(), Expect::Value(leaf_hash(b"t1 b1")))]));
    assert!(states(&settle(&mut h, fx), 5).contains(&State::Published), "the control did not apply");
}
