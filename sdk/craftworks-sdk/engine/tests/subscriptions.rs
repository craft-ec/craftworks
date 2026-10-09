//! Acceptance for range subscriptions (craftworks-sdk#44 part 2).
//!
//! The claim under test is not "a notification arrives". It is **exactly one
//! per commit, and none at all for a range the commit did not touch** — and
//! the second half is the one that can pass for the wrong reason, because a
//! test that subscribes to an untouched range and sees nothing looks identical
//! whether the range filter works or the notification machinery is simply not
//! wired up.
//!
//! So every "none" assertion here runs beside a control that makes the
//! notification actually happen: the same commit, the same engine, with
//! `notify_without_diff` on. If the control is silent the test fails on the
//! control, naming the real problem, instead of passing on the assertion.

use engine::subs::{SubRange, Why};
use engine::{ClientId, Effect, Engine, Event, Op, Params, State, WriteId};
use freenet_prolly::Cid;
use std::ops::Bound;

mod common;
use common::{Harness, Store};

fn c(n: u64) -> ClientId {
    ClientId(n)
}
fn w(n: u64) -> WriteId {
    WriteId(n)
}

fn put(k: &str, v: &[u8]) -> (Vec<u8>, Op) {
    (k.as_bytes().to_vec(), Op::Put(v.to_vec()))
}

/// `[lo, hi)` over string keys.
fn range(lo: &str, hi: &str) -> SubRange {
    SubRange {
        lo: Bound::Included(lo.as_bytes().to_vec()),
        hi: Bound::Excluded(hi.as_bytes().to_vec()),
    }
}

fn subscribe(client: u64, sub_id: u64, r: SubRange) -> Event {
    Event::SubscribeRange {
        client: c(client),
        sub_id,
        range: r,
    }
}

/// Every `Changed` in a batch of effects.
fn changes(effects: &[Effect]) -> Vec<(u64, u64, Why, Cid)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Changed {
                client,
                sub_id,
                new_root,
                why,
                ..
            } => Some((client.0, *sub_id, *why, *new_root)),
            _ => None,
        })
        .collect()
}

fn accepted(effects: &[Effect]) -> Vec<engine::subs::Accepted> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Subscribed { accepted, .. } => Some(*accepted),
            _ => None,
        })
        .collect()
}

/// Drive one write all the way to its head being confirmed, collecting every
/// effect produced along the way.
fn commit(h: &mut Harness, write_id: u64, ops: Vec<(Vec<u8>, Op)>) -> Vec<Effect> {
    let mut all = h.step(Event::forced_write(c(1), w(write_id), ops));
    let mut i = 0;
    let mut guard = 0;
    while i < all.len() {
        guard += 1;
        assert!(guard < 10_000, "the write did not settle");
        let next = match &all[i] {
            Effect::PutPack { id, .. } | Effect::PutBlock { id, .. } => {
                Some(Event::PutConfirmed(*id))
            }
            // The node holds every block: a changed group's other member it is asked about is held.
            Effect::ConfirmHeld { id } => Some(Event::PutConfirmed(*id)),
            Effect::UpdateHead { seq, .. } => Some(Event::HeadConfirmed(*seq)),
            _ => None,
        };
        i += 1;
        if let Some(ev) = next {
            let more = h.step(ev);
            all.extend(more);
        }
    }
    all
}

fn params(notify_without_diff: bool) -> Params {
    Params {
        notify_without_diff,
        ..Params::default()
    }
}

/// A commit inside a subscribed range notifies exactly once; one outside it
/// notifies not at all — and the control proves that silence is the FILTER.
#[test]
fn one_changed_per_commit_in_range_and_none_outside_it() {
    // Seed a tree so the ranges have something on either side of them.
    let mut h = Harness::new(params(false), Store::fresh());
    let seeded = commit(
        &mut h,
        1,
        (0..8)
            .map(|i| put(&format!("a/{i:02}"), b"seed"))
            .chain((0..8).map(|i| put(&format!("z/{i:02}"), b"seed")))
            .collect(),
    );
    assert!(
        changes(&seeded).is_empty(),
        "a commit before anyone subscribed notified somebody"
    );

    // Watch only the `a/` half.
    let out = h.step(subscribe(1, 7, range("a/", "b/")));
    assert_eq!(accepted(&out), [engine::subs::Accepted::Yes]);

    // A write INSIDE the range: exactly one notification.
    let inside = commit(&mut h, 2, vec![put("a/03", b"changed")]);
    let cs = changes(&inside);
    assert_eq!(
        cs.len(),
        1,
        "expected exactly one Changed for one commit, got {cs:?}"
    );
    let (client, sub_id, why, new_root) = cs[0];
    assert_eq!((client, sub_id), (1, 7));
    assert_eq!(
        why,
        Why::Diffed,
        "the tree was held, so the comparison could be \
         COMPLETED — anything else means the engine guessed, and the \
         range filter is then not being exercised at all"
    );
    assert_eq!(
        new_root,
        h.published_root(),
        "a Changed named a root the engine is not standing on"
    );

    // A write OUTSIDE it: nothing.
    let outside = commit(&mut h, 3, vec![put("z/03", b"changed")]);
    assert!(
        changes(&outside).is_empty(),
        "a commit outside the range notified: {:?}",
        changes(&outside)
    );
}

/// THE CONTROL for the test above.
///
/// Same engine, same subscription, same out-of-range commit — with the range
/// filter turned off. If this is silent, then "none outside the range" above
/// was measuring a notification path that does not work at all, and the
/// filter it names was never exercised.
#[test]
fn with_the_range_filter_off_the_same_commit_notifies() {
    let mut h = Harness::new(params(true), Store::fresh());
    commit(
        &mut h,
        1,
        (0..8).map(|i| put(&format!("z/{i:02}"), b"seed")).collect(),
    );
    let out = h.step(subscribe(1, 7, range("a/", "b/")));
    assert_eq!(accepted(&out), [engine::subs::Accepted::Yes]);

    let outside = commit(&mut h, 2, vec![put("z/03", b"changed")]);
    let cs = changes(&outside);
    assert_eq!(
        cs.len(),
        1,
        "the control did not fire: with the filter OFF an out-of-range commit \
         must still notify, or the previous test's silence proves nothing \
         about the filter"
    );
    assert_eq!(
        cs[0].2,
        Why::Budgeted,
        "nothing was compared, so the engine must not claim it diffed"
    );
}

/// A second client learns about a root it did not write.
///
/// This is the path a second tab is actually on: its engine never committed
/// anything, it re-read the head, and the root moved underneath it. An
/// implementation that notified only from its own commit path would pass every
/// single-engine test and do nothing at all in the case the feature exists for.
#[test]
fn a_reader_that_only_re_read_the_head_is_notified() {
    let store = Store::fresh();
    // Writer: builds a tree, publishes, then changes one key in `a/`.
    let mut writer = Harness::new(params(false), store.clone());
    commit(
        &mut writer,
        1,
        (0..8).map(|i| put(&format!("a/{i:02}"), b"seed")).collect(),
    );
    let first = writer.published_root();
    commit(&mut writer, 2, vec![put("a/03", b"changed")]);
    let second = writer.published_root();
    assert_ne!(first, second, "the writer's two commits produced one root");

    // Reader: a separate engine over the SAME node blocks. It writes nothing.
    let mut reader: Engine<Store> = Engine::new(params(false), store.clone());
    let out = reader.step(Event::Start {
        key: engine::KeySource::Provisioned(engine::Provisioned::Test),
        epochs: vec![engine::Epoch(1)],
    });
    assert!(matches!(out.as_slice(), [Effect::ReadHead { .. }]));
    let out = reader.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 1,
        root: first,
    });
    assert!(
        changes(&out).is_empty(),
        "nobody had subscribed yet, so the first head read must notify nobody"
    );

    let out = reader.step(Event::SubscribeRange {
        client: c(9),
        sub_id: 1,
        range: range("a/", "b/"),
    });
    assert_eq!(accepted(&out), [engine::subs::Accepted::Yes]);

    // The head moves to the writer's newer root. THIS is the notification
    // that makes a second tab work.
    let out = reader.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 2,
        root: second,
    });
    let cs = changes(&out);
    assert_eq!(
        cs.len(),
        1,
        "a reader that re-read the head onto a newer root was not told: {cs:?}"
    );
    assert_eq!((cs[0].0, cs[0].1), (9, 1));
    assert_eq!(cs[0].2, Why::Diffed);
    assert_eq!(
        cs[0].3, second,
        "the notification must name where the tree IS"
    );
}

/// Re-reading the SAME head notifies nobody, and reads nothing to decide it.
///
/// The counter needs a control, and finding that out cost a mutation: deleting
/// the `a == b` short circuit in `Subs::touched` did NOT move this number,
/// because `freenet_prolly::diff` short-circuits on equal roots too and reads
/// nothing — "not even the roots", as its own doc puts it. So a zero here is
/// guaranteed by the layer below whatever this layer does, and an assertion
/// that zero blocks were read is one that no plausible break in THIS file can
/// make fail.
///
/// What makes it mean something is the second half: the same counter, over a
/// root that DID move, must be non-zero. That is what shows the instrument can
/// see a read at all — without it, a store that had silently stopped counting
/// would produce exactly this test's passing output.
#[test]
fn an_unchanged_root_costs_nothing_and_says_nothing() {
    let store = Store::fresh();
    let mut writer = Harness::new(params(false), store.clone());
    commit(
        &mut writer,
        1,
        (0..64)
            .map(|i| put(&format!("a/{i:03}"), b"seed"))
            .collect(),
    );
    let root = writer.published_root();

    let mut reader: Engine<Store> = Engine::new(params(false), store.clone());
    reader.step(Event::Start {
        key: engine::KeySource::Provisioned(engine::Provisioned::Test),
        epochs: vec![engine::Epoch(1)],
    });
    reader.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 1,
        root,
    });
    reader.step(Event::SubscribeRange {
        client: c(1),
        sub_id: 1,
        range: range("a/", "b/"),
    });

    let before = store.reads();
    let out = reader.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 1,
        root,
    });
    assert!(changes(&out).is_empty(), "an unchanged root notified");
    assert_eq!(
        store.reads() - before,
        0,
        "equal roots must be decided WITHOUT reading the tree — that is the \
         property that makes this safe to run on every root move"
    );

    // THE CONTROL. A root that really moved must cost reads through the same
    // counter, or the zero above is an instrument that cannot see anything.
    let mut writer2 = Harness::new(params(false), store.clone());
    writer2.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 1,
        root,
    });
    commit(&mut writer2, 9, vec![put("a/001", b"moved")]);
    let moved = writer2.published_root();
    assert_ne!(moved, root);

    let before = store.reads();
    let out = reader.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 2,
        root: moved,
    });
    assert_eq!(changes(&out).len(), 1, "the moved root notified nobody");
    assert!(
        store.reads() - before > 0,
        "comparing two DIFFERENT roots read no blocks, so the counter is not \
         measuring anything and the zero above proves nothing"
    );
    println!(
        "  unchanged root: 0 reads; changed root: {} reads",
        store.reads() - before
    );
}

/// The cap refuses, by name, and the refusal is answered.
#[test]
fn the_subscription_cap_refuses_and_says_so() {
    let p = Params {
        max_subscriptions: 3,
        ..params(false)
    };
    let mut h = Harness::new(p, Store::fresh());
    for i in 0..3u64 {
        let out = h.step(subscribe(1, i, range("a/", "b/")));
        assert_eq!(
            accepted(&out),
            [engine::subs::Accepted::Yes],
            "subscription {i} was refused below the cap"
        );
    }
    let out = h.step(subscribe(1, 99, range("a/", "b/")));
    assert_eq!(
        accepted(&out),
        [engine::subs::Accepted::Full],
        "the cap did not refuse, so the bound is not a bound"
    );

    // Re-subscribing an id ALREADY held replaces it and does not pay again.
    let out = h.step(subscribe(1, 1, range("c/", "d/")));
    assert_eq!(
        accepted(&out),
        [engine::subs::Accepted::Yes],
        "narrowing a subscription already held was charged as a new one"
    );

    // Dropping one makes room.
    h.step(Event::Unsubscribe {
        client: c(1),
        sub_id: 0,
    });
    let out = h.step(subscribe(1, 99, range("a/", "b/")));
    assert_eq!(accepted(&out), [engine::subs::Accepted::Yes]);
}

/// A bound longer than the engine will hold is refused, not truncated.
#[test]
fn an_oversized_bound_is_refused_rather_than_trimmed() {
    let p = Params {
        max_sub_key: 16,
        ..params(false)
    };
    let mut h = Harness::new(p, Store::fresh());
    let huge = SubRange {
        lo: Bound::Included(vec![b'k'; 17]),
        hi: Bound::Unbounded,
    };
    let out = h.step(Event::SubscribeRange {
        client: c(1),
        sub_id: 1,
        range: huge,
    });
    assert_eq!(
        accepted(&out),
        [engine::subs::Accepted::TooWide],
        "a bound past the cap was accepted; the context is then a size the \
         CLIENT chooses"
    );
    // And exactly at the cap is fine — a bound nothing can satisfy is not a
    // bound either.
    let out = h.step(Event::SubscribeRange {
        client: c(1),
        sub_id: 2,
        range: SubRange {
            lo: Bound::Included(vec![b'k'; 16]),
            hi: Bound::Unbounded,
        },
    });
    assert_eq!(accepted(&out), [engine::subs::Accepted::Yes]);
}

/// A client that went away stops costing anything.
#[test]
fn a_departed_clients_subscriptions_go_with_it() {
    let mut h = Harness::new(params(false), Store::fresh());
    commit(
        &mut h,
        1,
        (0..8).map(|i| put(&format!("a/{i:02}"), b"seed")).collect(),
    );
    h.step(subscribe(1, 1, range("a/", "b/")));
    h.step(subscribe(2, 1, range("a/", "b/")));

    let both = commit(&mut h, 2, vec![put("a/01", b"x")]);
    assert_eq!(changes(&both).len(), 2, "two clients, two notifications");

    h.step(Event::ClientGone { client: c(1) });
    let one = commit(&mut h, 3, vec![put("a/02", b"y")]);
    let cs = changes(&one);
    assert_eq!(
        cs.len(),
        1,
        "the departed client was still notified: {cs:?}"
    );
    assert_eq!(cs[0].0, 2, "the WRONG client was dropped");
}

/// Writes still behave. A subscription must not change what a write does.
#[test]
fn subscribing_does_not_change_what_a_write_reports() {
    let mut with = Harness::new(params(false), Store::fresh());
    with.step(subscribe(1, 1, range("a/", "b/")));
    let a = commit(&mut with, 1, vec![put("a/01", b"v")]);

    let mut without = Harness::new(params(false), Store::fresh());
    let b = commit(&mut without, 1, vec![put("a/01", b"v")]);

    let states = |fs: &[Effect]| -> Vec<State> {
        fs.iter()
            .filter_map(|f| match f {
                Effect::Notify { state, .. } => Some(*state),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        states(&a),
        states(&b),
        "a subscription changed the states a write went through"
    );
}

// ---------------------------------------------------------------------------
// Delta updates, which need NO subscription at all.
// ---------------------------------------------------------------------------

fn deltas(effects: &[Effect]) -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
    effects
        .iter()
        .find_map(|e| match e {
            Effect::Reply {
                result: engine::read::ReadResult::Delta { changes, .. },
                ..
            } => Some(changes.clone()),
            Effect::Reply {
                result: engine::read::ReadResult::FullReloadRequired { .. },
                ..
            } => panic!("the delta asked for a full reload where the blocks were all held"),
            _ => None,
        })
        .unwrap_or_default()
}

/// A reload fetches the DIFFERENCE, and the full re-read is the control.
///
/// This is the claim that makes the delta layer worth having, and it is a
/// COST claim, so it is measured rather than asserted about. Both arms run on
/// one cold reader over one store, so the only thing that differs is which
/// question was asked.
///
/// No subscription is taken out anywhere in this test. That is deliberate:
/// delta update is a property of the tree, available to any reader holding a
/// root, and a test that reached for a subscription to get one would have
/// tangled the two layers the design keeps apart.
///
/// # The small-tree number, which is NOT what the name claims
///
/// Written first over 256 rows, this measured **6 block reads for a delta of
/// three changes against 4 for the whole range** — the delta was the more
/// expensive of the two. That is not a defect and it is not noise: 256 small
/// rows fit in about four blocks, a diff opens a path down BOTH trees, and a
/// range already four blocks wide has nothing to save. The companion test
/// keeps a small-tree measurement for exactly that reason: a cost claim with
/// no stated regime gets cited where it is false, and the first thing a
/// reader wants to know about "the delta is cheaper" is when it is not.
#[test]
fn a_reload_reads_the_delta_not_the_range_and_the_full_read_is_the_control() {
    let store = Store::fresh();
    let mut writer = Harness::new(params(false), store.clone());
    commit(
        &mut writer,
        1,
        (0..8_000)
            .map(|i| put(&format!("a/{i:05}"), b"seed"))
            .collect(),
    );
    let saw = writer.published_root();

    // A foreign write moves three keys.
    commit(
        &mut writer,
        2,
        vec![
            put("a/00007", b"moved"),
            put("a/03100", b"moved"),
            put("a/07200", b"moved"),
        ],
    );
    let now = writer.published_root();
    assert_ne!(saw, now);

    // THE DELTA. A reader standing on `saw` asks only what moved.
    let mut reader: Engine<Store> = Engine::new(params(false), store.clone());
    reader.adopt_root_for_test(now);
    let before = store.reads();
    let out = reader.step(Event::ChangesSince {
        client: c(1),
        req_id: engine::read::ReqId(1),
        from: saw,
        range: range("a/", "b/"),
        max_entries: 1000,
    });
    let delta_reads = store.reads() - before;
    let changed = deltas(&out);
    assert_eq!(
        changed.len(),
        3,
        "the delta did not name exactly the three keys that moved: {:?}",
        changed
            .iter()
            .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
            .collect::<Vec<_>>()
    );
    for (k, v) in &changed {
        assert_eq!(
            v.as_deref(),
            Some(&b"moved"[..]),
            "{:?}",
            String::from_utf8_lossy(k)
        );
    }

    // THE CONTROL: the same reader, the same range, read in full. If this is
    // not materially more expensive then the delta bought nothing and the
    // claim in this test's name is false.
    let mut full: Engine<Store> = Engine::new(params(false), store.clone());
    full.adopt_root_for_test(now);
    let before = store.reads();
    let full_out = full.step(Event::Scan {
        client: c(1),
        req_id: engine::read::ReqId(2),
        range: Box::new(freenet_prolly::range::Range {
            lo: Bound::Included(b"a/".to_vec()),
            hi: Bound::Excluded(b"b/".to_vec()),
            reverse: false,
            after: None,
            max_entries: 100_000,
            max_bytes: usize::MAX,
        }),
    });
    let full_reads = store.reads() - before;
    let rows = full_out
        .iter()
        .find_map(|e| match e {
            Effect::Reply {
                result: engine::read::ReadResult::Page { entries, .. },
                ..
            } => Some(entries.len()),
            _ => None,
        })
        .unwrap_or(0);

    println!("  8,000 rows — delta: {delta_reads} reads for 3 changes; full re-read: {full_reads} reads for {rows} rows");
    assert_eq!(
        rows, 8_000,
        "the control did not read the whole range, so it \
        is not the comparison this test claims to make"
    );
    assert!(
        delta_reads * 4 < full_reads,
        "the delta read {delta_reads} blocks and the full re-read {full_reads}: \
         the delta is not materially cheaper, so there is nothing here worth \
         the machinery"
    );
}

/// The same comparison on a SMALL tree, where the delta does not pay.
///
/// Kept, and kept passing, because the useful half of a cost claim is where it
/// stops being true. A range that already fits in a handful of blocks has
/// nothing for a delta to avoid reading, and a diff pays for a path down two
/// trees rather than one — so it costs at best the same and, as the number of
/// changes grows, more. Measured here: **one change, 4 reads either way**;
/// the same tree with three changes read 6 against the full range's 4. An SDK
/// deciding whether to reload by delta or by re-reading has its answer here
/// rather than in a benchmark nobody ran.
#[test]
fn a_small_tree_has_no_delta_to_save() {
    let store = Store::fresh();
    let mut writer = Harness::new(params(false), store.clone());
    commit(
        &mut writer,
        1,
        (0..256)
            .map(|i| put(&format!("a/{i:04}"), b"seed"))
            .collect(),
    );
    let saw = writer.published_root();
    commit(&mut writer, 2, vec![put("a/0007", b"moved")]);
    let now = writer.published_root();

    let mut reader: Engine<Store> = Engine::new(params(false), store.clone());
    reader.adopt_root_for_test(now);
    let before = store.reads();
    let out = reader.step(Event::ChangesSince {
        client: c(1),
        req_id: engine::read::ReqId(1),
        from: saw,
        range: range("a/", "b/"),
        max_entries: 1000,
    });
    let delta_reads = store.reads() - before;
    assert_eq!(deltas(&out).len(), 1);

    let mut full: Engine<Store> = Engine::new(params(false), store.clone());
    full.adopt_root_for_test(now);
    let before = store.reads();
    full.step(Event::Scan {
        client: c(1),
        req_id: engine::read::ReqId(2),
        range: Box::new(freenet_prolly::range::Range {
            lo: Bound::Included(b"a/".to_vec()),
            hi: Bound::Excluded(b"b/".to_vec()),
            reverse: false,
            after: None,
            max_entries: 100_000,
            max_bytes: usize::MAX,
        }),
    });
    let full_reads = store.reads() - before;

    println!("  256 rows — delta: {delta_reads} reads; full re-read: {full_reads} reads");
    assert!(
        delta_reads >= full_reads,
        "the delta is now cheaper at 256 rows ({delta_reads} vs {full_reads}). \
         That is an improvement, not a failure — but the regime this test \
         records has moved, so update the number and the guidance that cites it"
    );
}

/// A removal arrives as a removal, not as an empty value.
///
/// The distinction is the whole difference between a reader that drops the key
/// and one that keeps it holding nothing — and an empty value is exactly what
/// a careless materialisation produces, so it is pinned.
#[test]
fn a_deleted_key_comes_back_as_a_removal() {
    let store = Store::fresh();
    let mut writer = Harness::new(params(false), store.clone());
    commit(
        &mut writer,
        1,
        (0..8).map(|i| put(&format!("a/{i:02}"), b"seed")).collect(),
    );
    let saw = writer.published_root();
    commit(&mut writer, 2, vec![(b"a/03".to_vec(), Op::Delete)]);
    let now = writer.published_root();

    let mut reader: Engine<Store> = Engine::new(params(false), store.clone());
    reader.adopt_root_for_test(now);
    let out = reader.step(Event::ChangesSince {
        client: c(1),
        req_id: engine::read::ReqId(1),
        from: saw,
        range: range("a/", "b/"),
        max_entries: 100,
    });
    let changed = deltas(&out);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].0, b"a/03");
    assert_eq!(
        changed[0].1, None,
        "a deleted key came back with a value; a reader told that keeps the \
         key it was supposed to drop"
    );
}

/// Asking for a delta from the root you are already on is empty, and free.
#[test]
fn a_delta_from_the_current_root_is_empty_and_reads_nothing() {
    let store = Store::fresh();
    let mut writer = Harness::new(params(false), store.clone());
    commit(
        &mut writer,
        1,
        (0..64)
            .map(|i| put(&format!("a/{i:03}"), b"seed"))
            .collect(),
    );
    let now = writer.published_root();

    let mut reader: Engine<Store> = Engine::new(params(false), store.clone());
    reader.adopt_root_for_test(now);
    let before = store.reads();
    let out = reader.step(Event::ChangesSince {
        client: c(1),
        req_id: engine::read::ReqId(1),
        from: now,
        range: range("a/", "b/"),
        max_entries: 100,
    });
    assert!(deltas(&out).is_empty());
    assert_eq!(
        store.reads() - before,
        0,
        "a delta against the same root read blocks"
    );
}

// ---------------------------------------------------------------------------
// The storm invariants: at most one notification per (subscription, new root),
// and a range that cannot be compared goes quiet instead of shouting.
// ---------------------------------------------------------------------------

/// Two paths observing ONE root move produce ONE notification.
///
/// A commit landing and a head read are separate paths and both call
/// `notify_subs`. Without the invariant a client is told the same fact twice
/// and has no way to tell that from two commits — which matters precisely
/// because the notification carries no keys, so "how many times was I told"
/// is all it has.
#[test]
fn one_root_move_seen_twice_notifies_once() {
    let mut h = Harness::new(params(false), Store::fresh());
    commit(
        &mut h,
        1,
        (0..8).map(|i| put(&format!("a/{i:02}"), b"seed")).collect(),
    );
    h.step(subscribe(1, 1, range("a/", "b/")));

    let first = commit(&mut h, 2, vec![put("a/01", b"x")]);
    assert_eq!(changes(&first).len(), 1, "the commit did not notify once");
    let landed = h.published_root();

    // The same root, arriving again by the OTHER path.
    let again = h.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 99,
        root: landed,
    });
    assert!(
        changes(&again).is_empty(),
        "the same root notified twice: {:?}",
        changes(&again)
    );

    // And a genuinely NEW root still does notify — otherwise the invariant
    // above would be satisfied by an engine that had simply stopped.
    let third = commit(&mut h, 3, vec![put("a/02", b"y")]);
    assert_eq!(
        changes(&third).len(),
        1,
        "after de-duplicating one root the engine stopped notifying at all"
    );
}

/// A range whose comparison keeps failing goes Stale, once, and then quiet.
///
/// The control is the count. An engine that simply stopped notifying would
/// satisfy "no storm" perfectly, so the test pins BOTH halves: the client is
/// told the range went stale, exactly once, and every later commit is silent.
#[test]
fn a_range_that_cannot_be_compared_goes_stale_and_stops() {
    let p = Params {
        stale_after_missing: 2,
        ..params(false)
    };
    let store = Store::fresh();
    let mut writer = Harness::new(p, store.clone());
    commit(
        &mut writer,
        1,
        (0..2_000)
            .map(|i| put(&format!("a/{i:04}"), b"seed"))
            .collect(),
    );

    // A reader that has adopted the root but holds NONE of the tree: every
    // comparison it is asked for stops on a block it does not have.
    let blind = Store::fresh();
    let mut reader: Engine<Store> = Engine::new(p, blind);
    reader.step(Event::Start {
        key: engine::KeySource::Provisioned(engine::Provisioned::Test),
        epochs: vec![engine::Epoch(1)],
    });
    reader.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 1,
        root: writer.published_root(),
    });
    reader.step(Event::SubscribeRange {
        client: c(1),
        sub_id: 1,
        range: range("a/", "b/"),
    });

    let mut seen: Vec<Why> = Vec::new();
    for i in 0..6u64 {
        commit(
            &mut writer,
            10 + i,
            vec![put(&format!("a/{i:04}"), b"moved")],
        );
        let out = reader.step(Event::HeadRead {
            epoch: engine::Epoch(1),
            seq: 2 + i,
            root: writer.published_root(),
        });
        seen.extend(changes(&out).into_iter().map(|c| c.2));
    }

    println!("  blind reader over 6 commits: {seen:?}");
    assert_eq!(
        seen,
        vec![Why::BlockMissing, Why::Stale],
        "expected: told twice it could not tell, then told ONCE that it has \
         stopped, then silence. Anything longer is the storm this bound \
         exists to prevent; anything shorter and the client was never told"
    );
}

/// THE CONTROL for the test above: a reader that CAN compare is not silenced.
///
/// Without this, "two notifications then quiet" would also be the output of an
/// engine whose notification path had broken after the second call, and the
/// stale bound would be taking credit for it.
#[test]
fn a_reader_that_holds_the_tree_keeps_being_notified() {
    let p = Params {
        stale_after_missing: 2,
        ..params(false)
    };
    let store = Store::fresh();
    let mut writer = Harness::new(p, store.clone());
    commit(
        &mut writer,
        1,
        (0..2_000)
            .map(|i| put(&format!("a/{i:04}"), b"seed"))
            .collect(),
    );
    // The SAME store, so every comparison can be completed.
    let mut reader: Engine<Store> = Engine::new(p, store.clone());
    reader.step(Event::Start {
        key: engine::KeySource::Provisioned(engine::Provisioned::Test),
        epochs: vec![engine::Epoch(1)],
    });
    reader.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 1,
        root: writer.published_root(),
    });
    reader.step(Event::SubscribeRange {
        client: c(1),
        sub_id: 1,
        range: range("a/", "b/"),
    });

    let mut seen: Vec<Why> = Vec::new();
    for i in 0..6u64 {
        commit(
            &mut writer,
            10 + i,
            vec![put(&format!("a/{i:04}"), b"moved")],
        );
        let out = reader.step(Event::HeadRead {
            epoch: engine::Epoch(1),
            seq: 2 + i,
            root: writer.published_root(),
        });
        seen.extend(changes(&out).into_iter().map(|c| c.2));
    }
    assert_eq!(
        seen,
        vec![Why::Diffed; 6],
        "a reader holding the whole tree was silenced; the stale bound is \
         firing on something other than a missing block"
    );
}

/// Re-subscribing brings a Stale range back. It is the only thing that can.
#[test]
fn a_reload_revives_a_stale_range() {
    let p = Params {
        stale_after_missing: 1,
        ..params(false)
    };
    let store = Store::fresh();
    let mut writer = Harness::new(p, store.clone());
    commit(
        &mut writer,
        1,
        (0..2_000)
            .map(|i| put(&format!("a/{i:04}"), b"seed"))
            .collect(),
    );
    let blind = Store::fresh();
    let mut reader: Engine<Store> = Engine::new(p, blind);
    reader.step(Event::Start {
        key: engine::KeySource::Provisioned(engine::Provisioned::Test),
        epochs: vec![engine::Epoch(1)],
    });
    reader.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 1,
        root: writer.published_root(),
    });
    let sub = |r: &mut Engine<Store>| {
        r.step(Event::SubscribeRange {
            client: c(1),
            sub_id: 1,
            range: range("a/", "b/"),
        })
    };
    sub(&mut reader);

    commit(&mut writer, 11, vec![put("a/0001", b"moved")]);
    let out = reader.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 2,
        root: writer.published_root(),
    });
    assert_eq!(changes(&out)[0].2, Why::Stale, "did not go stale at K=1");

    commit(&mut writer, 12, vec![put("a/0002", b"moved")]);
    let out = reader.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 3,
        root: writer.published_root(),
    });
    assert!(changes(&out).is_empty(), "a stale range kept notifying");

    // The reload. This is the whole recovery path — there is nothing else a
    // client can do, so it must work.
    sub(&mut reader);
    commit(&mut writer, 13, vec![put("a/0003", b"moved")]);
    let out = reader.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 4,
        root: writer.published_root(),
    });
    assert_eq!(
        changes(&out).len(),
        1,
        "re-subscribing did not revive the range, so a client that went \
         stale can never recover"
    );
}

/// A delta whose old root is GONE answers "reload", not "unavailable".
///
/// This is the decided degraded answer, and the point of deciding it is that
/// the client has one thing to do — a plain range read — rather than an error
/// it must invent a recovery for. A reader that was away long enough for the
/// old root's blocks to be superseded and evicted is ordinary, not broken.
///
/// The control is the OTHER arm: the same engine, the same code path, asked
/// for a `Get` it also cannot answer, must still say `Unavailable`. Without it
/// this test would pass against an engine that had simply renamed every
/// failure.
#[test]
fn a_delta_across_a_vanished_root_asks_for_a_reload_and_a_get_still_does_not() {
    let store = Store::fresh();
    let mut writer = Harness::new(params(false), store.clone());
    commit(
        &mut writer,
        1,
        (0..2_000)
            .map(|i| put(&format!("a/{i:04}"), b"seed"))
            .collect(),
    );
    let gone = writer.published_root();
    commit(&mut writer, 2, vec![put("a/0001", b"moved")]);
    let now = writer.published_root();

    // A reader holding nothing at all: every block it asks for is missed.
    let blind = Store::fresh();
    let mut reader: Engine<Store> = Engine::new(params(false), blind);
    reader.adopt_root_for_test(now);

    let mut out = reader.step(Event::ChangesSince {
        client: c(1),
        req_id: engine::read::ReqId(1),
        from: gone,
        range: range("a/", "b/"),
        max_entries: 100,
    });
    // Every fetch it asks for comes back missed, until it gives up.
    let mut guard = 0;
    let mut answer = None;
    let mut i = 0;
    while i < out.len() {
        guard += 1;
        assert!(guard < 10_000, "the delta never settled");
        match &out[i] {
            Effect::FetchBlock { id, .. } => {
                let id = *id;
                i += 1;
                let more = reader.step(Event::BlockMissed(id));
                out.extend(more);
                continue;
            }
            Effect::Reply { result, .. } => answer = Some(result.clone()),
            other => common::no_answer_owed(other),
        }
        i += 1;
    }
    let answer = answer.expect("the delta must be ANSWERED, never dropped");
    assert!(
        matches!(answer, engine::read::ReadResult::FullReloadRequired { .. }),
        "a delta whose old root is unreachable answered {answer:?}; the \
         client then has to invent its own fallback, which is what deciding \
         this answer was supposed to avoid"
    );

    // THE CONTROL. The same engine cannot answer this `Get` either, and it
    // must NOT say `FullReloadRequired` -- otherwise that is just what this
    // engine says when anything goes wrong. A Get WAITS (rule 7): it is never
    // answered while its blocks are NotFound, and the block stays awaited so
    // the page keeps asking.
    let mut out = reader.step(Event::Get {
        client: c(1),
        req_id: engine::read::ReqId(2),
        key: b"a/0001".to_vec(),
    });
    let mut answer = None;
    let mut asked = None;
    for _ in 0..16 {
        let fetch = out.iter().find_map(|f| match f {
            Effect::FetchBlock { id, .. } => Some(*id),
            _ => None,
        });
        if let Some(r) = out.iter().find_map(|f| match f {
            Effect::Reply { result, .. } => Some(result.clone()),
            _ => None,
        }) {
            answer = Some(r);
        }
        let Some(id) = fetch else { break };
        asked = Some(id);
        out = reader.step(Event::BlockMissed(id));
    }
    assert_eq!(answer, None, "a Get whose blocks are NotFound was answered {answer:?}: the degraded answer is a DELTA's, and a Get waits");
    let asked = asked.expect("the Get asked for a block");
    assert!(reader.readers_of(&asked).any(), "the Get's block is no longer awaited: nothing would ask for it again");
}
