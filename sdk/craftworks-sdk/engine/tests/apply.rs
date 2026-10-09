//! A write onto a tree path the node does not hold (craftworks-sdk#32).
//!
//! A delegate reads the tree through a SYNC host call that returns what the
//! node happens to hold at that instant (F14), and that call registers no
//! demand (F33). So an apply can stop on a cold block with nothing wrong: the
//! engine must fetch it and run the apply again, not tell the client its write
//! failed.

use engine::read::{ReadResult, ReqId};
use engine::{ClientId, Effect, Event, Op, Params, State, WriteId};
use freenet_prolly::store::{Blocks, MemBlocks};
use freenet_prolly::Cid;
use std::collections::BTreeMap;

mod common;
use common::{rebuild, tree, Harness, Store};

/// A tree of `n` keys, and every block in it.
fn fixture(n: u32) -> (BTreeMap<Vec<u8>, Vec<u8>>, Cid, MemBlocks) {
    let records: BTreeMap<Vec<u8>, Vec<u8>> = (0..n)
        .map(|i| (format!("k/{i:05}").into_bytes(), vec![(i % 251) as u8; 40]))
        .collect();
    let (root, all) = tree(&records);
    (records, root, all)
}

fn started(params: Params, store: Store, root: Cid) -> Harness {
    let mut h = Harness::new(params, store);
    let _ = h.step(Event::Start {
        key: engine::KeySource::SecretStore,
        epochs: vec![engine::Epoch(1)],
    });
    let _ = h.step(Event::HeadRead {
        epoch: engine::Epoch(1),
        seq: 1,
        root,
    });
    h
}

fn fetches(fx: &[Effect]) -> Vec<Cid> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::FetchBlock { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

fn states(fx: &[Effect]) -> Vec<State> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::Notify { state, .. } => Some(*state),
            _ => None,
        })
        .collect()
}

fn write_one(key: &str, val: &[u8]) -> Event {
    Event::forced_write(ClientId(1), WriteId(1), vec![(key.as_bytes().to_vec(), Op::Put(val.to_vec()))])
}

/// A write onto a cold path is parked, fetched for, and applies.
///
/// The regression: this reported `Failed`, which says *this edit is not in
/// the tree and never will be*. It was a transient miss, and a client acting
/// on that word gives up on a write one fetch away from landing.
///
/// The control is the SAME write against a warm store: it must ask for
/// nothing and be accepted in one step. Without it, "the cold write was
/// parked" would read identically if every write were parked.
#[test]
fn a_write_onto_a_cold_path_is_parked_and_then_applies() {
    // LIVE ONLY (R-b): the write waits in the page's queue, which is page
    // memory and never in a context -- the page keeps its engine; no delegate
    // rebuilds one between steps any more.
    let (mut records, root, all) = fixture(400);

    // --- the control: a warm store ---
    let warm = Store::fresh();
    for (id, bytes) in all.0.iter() {
        warm.put(*id, bytes);
    }
    let mut hw = started(Params::default(), warm, root);
    let out = hw.step(write_one("k/00100", b"warm"));
    assert!(
        fetches(&out).is_empty(),
        "a write against a store holding the whole tree asked \
         for {} block(s), so parking is not what the cold case shows",
        fetches(&out).len()
    );
    assert_eq!(
        states(&out).first(),
        Some(&State::Accepted),
        "a warm write was not accepted in one step"
    );

    // --- the case: only the root is held ---
    let cold = Store::fresh();
    cold.put(root, all.get(&root).expect("the root"));
    let mut h = started(Params::default(), cold.clone(), root);

    let out = h.step(write_one("k/00100", b"cold"));
    assert!(
        !fetches(&out).is_empty(),
        "a write onto a path the node does not hold asked for \
         nothing"
    );
    assert!(
        states(&out).is_empty(),
        "a parked write was told {:?} before it had applied",
        states(&out)
    );

    // Feed it what it asks for, one block at a time, until it applies.
    let mut queue = fetches(&out);
    let mut rounds = 0;
    let mut accepted = false;
    let mut delivered = 0usize;
    while let Some(id) = queue.pop() {
        rounds += 1;
        assert!(rounds < 1000, "the parked write never settled");
        let bytes = all.get(&id).expect("the fixture holds every block");
        cold.put(id, bytes);
        delivered += 1;
        let out = h.step(Event::BlockArrived {
            id,
            bytes: bytes.to_vec(),
        });
        queue.extend(fetches(&out));
        if states(&out).contains(&State::Accepted) {
            accepted = true;
            break;
        }
    }
    assert!(
        accepted,
        "the parked write was fed every block it asked for and \
         was never accepted"
    );
    // It read a PATH, not the tree: 400 keys is several levels, and a
    // write that pulled the whole thing would be hundreds of blocks.
    assert!(
        delivered < 20,
        "the parked write pulled {delivered} blocks to apply one \
         key, which is a whole-tree walk rather than a path"
    );

    records.insert(b"k/00100".to_vec(), b"cold".to_vec());
    assert_eq!(
        h.root(),
        rebuild(&records),
        "the parked write applied to the wrong tree"
    );
    println!("  cold write parked, {delivered} blocks, then applied");
}

/// A write whose blocks never arrive is refused, not left waiting.
///
/// The bound is on ROUNDS, not on misses: this drives it with misses, and the
/// sibling assertion is that nothing was applied — `Failed` is only honest
/// when the edit really is not in the tree. Repair is OFF here so every fetch
/// is the write's own round (sdk#405: with it on, a miss also asks the lost
/// block's group, which is not a round -- that case is the next test).
#[test]
fn a_write_whose_blocks_never_arrive_is_refused_and_applies_nothing() {
    // Two different bounds. One run stopping at its bound could be a
    // coincidence of the fixture's depth; two runs each stopping at their own
    // shows the bound is the thing doing it.
    for rounds_allowed in [8u32, 3] {
        let params = Params {
            max_apply_rounds: rounds_allowed,
            repair_reads: false,
            ..Params::default()
        };
        let (_, root, all) = fixture(400);
        let cold = Store::fresh();
        cold.put(root, all.get(&root).expect("the root"));
        let mut h = started(params, cold, root);
        let before = h.root();

        let mut queue = fetches(&h.step(write_one("k/00100", b"never")));
        assert!(!queue.is_empty(), "the write asked for nothing");
        let mut asked = 1u32;
        let mut failed = 0usize;
        while let Some(id) = queue.pop() {
            let out = h.step(Event::BlockMissed(id));
            failed += states(&out).iter().filter(|s| matches!(s, State::Failed { .. })).count();
            let next = fetches(&out);
            if next.is_empty() {
                break;
            }
            asked += 1;
            assert!(
                asked <= rounds_allowed,
                "the write has been re-asked {asked} times against a \
                 bound of {rounds_allowed}"
            );
            queue = next;
        }
        assert_eq!(
            failed, 1,
            "a write that could never apply was reported Failed \
             {failed} times; a client needs exactly one answer"
        );
        assert_eq!(
            h.root(),
            before,
            "the write was reported Failed and changed the tree \
             anyway, so a client that re-submits applies it twice"
        );
        // The control for the bound: it stopped where the bound put it, not
        // on the first miss and not at some fixed depth of its own.
        assert_eq!(
            asked, rounds_allowed,
            "the write was refused after {asked} round(s) against a \
             bound of {rounds_allowed}, so the bound is not what stops it"
        );
        println!("  bound {rounds_allowed}: refused after {asked} rounds, tree unchanged");
    }
}

/// The same, with REPAIR ON (sdk#405): a miss also asks the lost block's group, whose members never arrive either.
/// The write is still refused exactly once and changes nothing -- the repair is an alternative source, never a
/// way round the round bound.
#[test]
fn with_repair_on_a_write_whose_blocks_never_arrive_is_still_refused_once() {
    let params = Params { max_apply_rounds: 3, ..Params::default() };
    assert!(params.repair_reads, "THE SETUP: repair is off by default");
    let (_, root, all) = fixture(400);
    let cold = Store::fresh();
    cold.put(root, all.get(&root).expect("the root"));
    let mut h = started(params, cold, root);
    let before = h.root();
    // FIFO, as a paced page answers: every ask takes its turn, so the group's re-asks never starve the write's own.
    let mut queue: std::collections::VecDeque<Cid> = fetches(&h.step(write_one("k/00100", b"never"))).into_iter().collect();
    let mut failed = 0usize;
    let mut steps = 0;
    while let Some(id) = queue.pop_front() {
        steps += 1;
        assert!(steps < 10_000, "the write never ended");
        let out = h.step(Event::BlockMissed(id));
        failed += states(&out).iter().filter(|s| matches!(s, State::Failed { .. })).count();
        queue.extend(fetches(&out));
    }
    assert_eq!(failed, 1, "a write that could never apply was reported Failed {failed} times");
    assert_eq!(h.root(), before, "the write was reported Failed and changed the tree anyway");
    let (started, _, _) = h.repair_counts();
    println!("  repair on: refused once after {steps} misses; repairs started {started}");
    assert!(started > 0, "no repair was started: the case is the previous test's");
}

/// A second write while one is parked WAITS ITS TURN (R-b; COMMIT-LIFE
/// footnote 7: arrival order is apply order). It is `Applying` behind the
/// first -- told nothing yet, in no root -- and applies right after it, even
/// though its own path is warm enough to be judged at once.
///
/// It used to be refused `Busy`, and an outbox re-sent it for ever.
#[test]
fn a_second_write_while_one_is_parked_waits_its_turn_and_applies_after_it() {
    let (mut records, root, all) = fixture(400);
    let cold = Store::fresh();
    cold.put(root, all.get(&root).expect("the root"));
    let mut h = started(Params::default(), cold.clone(), root);

    let out = h.step(write_one("k/00100", b"first"));
    let mut queue = fetches(&out);
    assert!(!queue.is_empty(), "the first write did not park");

    let out = h.step(Event::forced_write(ClientId(2), WriteId(2), vec![(b"k/00200".to_vec(), Op::Put(b"second".to_vec()))]));
    assert_eq!(states(&out), Vec::<State>::new(), "a write behind a parked one was answered before its turn");
    assert_eq!(h.root(), rebuild(&records), "a write behind a parked one joined the warm root before it");

    // Feed the first its path; the second applies right behind it.
    let mut told: Vec<(ClientId, State)> = Vec::new();
    let mut guard = 0;
    while let Some(id) = queue.pop() {
        guard += 1;
        assert!(guard < 1000, "the parked write never settled");
        let bytes = all.get(&id).expect("held");
        cold.put(id, bytes);
        let out = h.step(Event::BlockArrived { id, bytes: bytes.to_vec() });
        queue.extend(fetches(&out));
        told.extend(out.iter().filter_map(|f| match f {
            Effect::Notify { client, state: State::Accepted, .. } => Some((*client, State::Accepted)),
            _ => None,
        }));
        if told.len() == 2 {
            break;
        }
    }
    assert_eq!(
        told,
        vec![(ClientId(1), State::Accepted), (ClientId(2), State::Accepted)],
        "the two writes were not accepted in the order they arrived"
    );
    records.insert(b"k/00100".to_vec(), b"first".to_vec());
    records.insert(b"k/00200".to_vec(), b"second".to_vec());
    assert_eq!(h.root(), rebuild(&records), "the warm root is not both writes, in order");
    println!("  a second write while one is parked: waited, then applied after it");
}

/// A cold write of ANY size parks, fetches its path, and applies (R-b).
///
/// Its ops ride in the page's queue, not in a context, so nothing bounds
/// what may park. Two shapes the old context bound refused: one large value,
/// and many tiny ops whose serialized cost is far above their payload
/// (craftworks-sdk#136; the parked-write cost cap). Each used to be `Busy`
/// with fetches; now each parks once and is accepted when its path arrives.
#[test]
fn a_cold_write_of_any_size_parks_and_applies() {
    let (_, root, all) = fixture(400);
    let big: Vec<(Vec<u8>, Op)> = vec![(b"k/00100".to_vec(), Op::Put(vec![7u8; 256 * 1024]))];
    let tiny: Vec<(Vec<u8>, Op)> = (0..4096).map(|i| (format!("t{i:05}").into_bytes(), Op::Put(vec![1u8]))).collect();
    for (name, ops) in [("one 256 KiB value", big), ("4096 tiny ops", tiny)] {
        let cold = Store::fresh();
        cold.put(root, all.get(&root).expect("the root"));
        let mut h = started(Params::default(), cold.clone(), root);
        let out = h.step(Event::forced_write(ClientId(1), WriteId(1), ops));
        assert!(
            states(&out).is_empty() && !fetches(&out).is_empty(),
            "{name}: a cold write did not park: {:?}",
            states(&out)
        );
        let mut queue = fetches(&out);
        let mut accepted = false;
        let mut guard = 0;
        while let Some(id) = queue.pop() {
            guard += 1;
            assert!(guard < 1000, "{name}: the parked write never settled");
            let bytes = all.get(&id).expect("held");
            cold.put(id, bytes);
            let out = h.step(Event::BlockArrived { id, bytes: bytes.to_vec() });
            assert!(!states(&out).iter().any(|s| matches!(s, State::Failed { .. })), "{name}: a parked write was refused");
            queue.extend(fetches(&out));
            if states(&out).contains(&State::Accepted) {
                accepted = true;
                break;
            }
        }
        assert!(accepted, "{name}: fed its path, the write was never accepted");
        println!("  {name}: parked, fed, accepted");
    }
}

/// A parked write and a parked read wait on the same block, and both are
/// answered by its one arrival.
#[test]
fn one_arrival_answers_both_a_parked_read_and_a_parked_write() {
    let (_, root, all) = fixture(400);
    let cold = Store::fresh();
    cold.put(root, all.get(&root).expect("the root"));
    // Live: the write waits in the page's queue (R-b), never in a context.
    let mut h = started(Params::default(), cold.clone(), root);

    // A read of the key the write is about to touch: both descend the same
    // path, so they stop on the same block.
    let r = h.step(Event::Get {
        client: ClientId(1),
        req_id: ReqId(1),
        key: b"k/00100".to_vec(),
    });
    let w = h.step(write_one("k/00100", b"both"));
    // ONE ask per block (W5, WANTED-LIFE): the write stopping on a block the read already asked rides that GET --
    // it asks nothing of its own for it -- and the block has BOTH as readers.
    let shared: Vec<Cid> = fetches(&r)
        .into_iter()
        .filter(|i| h.readers_of(i).reads && h.readers_of(i).parked_write)
        .collect();
    assert!(
        !shared.is_empty(),
        "the read and the write stopped on different blocks, so this test is \
         not about one arrival answering two waiters"
    );
    assert!(
        shared.iter().all(|i| !fetches(&w).contains(i)),
        "the write asked again for a block the read already has in flight (W5: one ask per block)"
    );

    let mut queue = shared;
    let (mut answered, mut accepted) = (false, false);
    let mut guard = 0;
    while let Some(id) = queue.pop() {
        guard += 1;
        assert!(guard < 1000, "neither waiter settled");
        let bytes = all.get(&id).expect("held");
        cold.put(id, bytes);
        let out = h.step(Event::BlockArrived {
            id,
            bytes: bytes.to_vec(),
        });
        queue.extend(fetches(&out));
        if states(&out).contains(&State::Accepted) {
            accepted = true;
        }
        if out.iter().any(|f| {
            matches!(
                f,
                Effect::Reply {
                    result: ReadResult::Value(_),
                    ..
                }
            )
        }) {
            answered = true;
        }
        if answered && accepted {
            break;
        }
    }
    assert!(
        answered,
        "the read was never answered: the write's arrivals swallowed it"
    );
    assert!(
        accepted,
        "the write never applied: the read's arrivals swallowed it"
    );
    println!("  one block stream answered a parked read and a parked write");
}
