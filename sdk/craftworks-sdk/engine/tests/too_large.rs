//! craftworks-sdk#136: a write the engine can NEVER accept is `TooLarge`,
//! once, with its bound and its count -- and refusing it changes nothing.
//!
//! Both bounds answered `Busy`, the word for "try again", and the client's
//! outbox tried again for ever. `Busy` stays for what is transient (a commit
//! in flight, a cold write declined while its path is fetched).

use engine::{ClientId, Effect, Engine, Event, Op, Params, State, WriteBound, WriteId};

mod common;
use common::Store;

fn states(fx: &[Effect]) -> Vec<State> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::Notify { state, .. } => Some(*state),
            _ => None,
        })
        .collect()
}

fn write(id: u64, ops: Vec<(Vec<u8>, Op)>) -> Event {
    Event::forced_write(ClientId(1), WriteId(id), ops)
}

/// `n` DISTINCT values of `size` bytes, one key each. Distinct on purpose:
/// `max_commit_blocks` counts distinct BLOCKS, so identical values collapse
/// into one and a 200-value write of one repeated byte is ACCEPTED -- see
/// `identical_values_are_one_block_so_count_is_not_the_bound`.
fn batch(n: usize, size: usize) -> Vec<(Vec<u8>, Op)> {
    (0..n)
        .map(|i| {
            let mut v = vec![0u8; size];
            v[..8].copy_from_slice(&(i as u64).to_le_bytes());
            (format!("bulk/{i:05}").into_bytes(), Op::Put(v))
        })
        .collect()
}

/// An engine with history, built under `params`: three writes committed and
/// published, so it holds owed parity, a head, seen writes -- more than an
/// empty one would, and more for a leaked half-commit to disturb.
fn history(params: Params) -> Engine<Store> {
    let mut e = common::new_store_params(params);
    for w in 1..=3u64 {
        let out = stepped!(
            e,
            write(
                w,
                batch(20, 64)
                    .into_iter()
                    .map(|(k, op)| ([k, format!("/{w}").into_bytes()].concat(), op))
                    .collect()
            )
        );
        for f in &out {
            if let Effect::PutPack { id, .. } | Effect::PutBlock { id, .. } = f {
                let o = stepped!(e, Event::PutConfirmed(*id));
                if let Some(seq) = o.iter().find_map(|x| match x {
                    Effect::UpdateHead { seq, .. } => Some(*seq),
                    _ => None,
                }) {
                    let _ = stepped!(e, Event::HeadConfirmed(seq));
                }
            }
        }
    }
    e
}

/// The same history, REOPENED under `params` the way a page reload reopens
/// it: a new engine recovering from the published head, over the node's
/// blocks.
///
/// The history is ALWAYS built at the default params. Building it under the
/// params being tested made the tree depend on the budget -- under a budget of
/// one block the history's own writes were refused -- and the block count
/// under test moved with it (205 vs 206, measured).
fn lived_in(params: Params) -> Engine<Store> {
    let h = history(Params::default());
    let mut e = Engine::new(params, h.blocks().clone());
    let _ = e.step(Event::Start { key: engine::KeySource::SecretStore, epochs: vec![engine::Epoch(1)] });
    let _ = e.step(Event::HeadRead { epoch: engine::Epoch(1), seq: h.published_seq(), root: h.published_root() });
    assert_eq!((e.published_seq(), e.root()), (h.published_seq(), h.published_root()), "the reopened engine is not on the history's head");
    e
}

/// What one write is answered on a lived-in engine with `params`.
fn answer(params: Params, ops: Vec<(Vec<u8>, Op)>) -> Vec<State> {
    let mut e = lived_in(params);
    states(&stepped!(e, write(99, ops)))
}

#[test]
fn over_the_commit_block_budget_is_too_large_with_its_count_and_the_boundary_is_exact() {
    let ops = batch(200, 2048);
    // Read the write's block count off the engine itself: under a budget of
    // one, it is refused and says how many it needed.
    let got = match answer(
        Params {
            max_commit_blocks: 1,
            ..Params::default()
        },
        ops.clone(),
    )
    .as_slice()
    {
        [State::TooLarge {
            bound: WriteBound::CommitBlocks,
            limit: 1,
            got,
        }] => *got,
        other => panic!("under a budget of one block, answered {other:?}"),
    };
    assert!(
        got > 128,
        "the fixture needs {got} blocks, which is under the default budget -- it tests nothing"
    );
    // AT the count: accepted. One under: refused, naming exactly this count.
    assert_eq!(
        answer(
            Params {
                max_commit_blocks: got,
                ..Params::default()
            },
            ops.clone()
        ),
        vec![State::Accepted],
        "a write needing exactly {got} blocks was refused at a budget of {got}"
    );
    assert_eq!(
        answer(
            Params {
                max_commit_blocks: got - 1,
                ..Params::default()
            },
            ops.clone()
        ),
        vec![State::TooLarge {
            bound: WriteBound::CommitBlocks,
            limit: got - 1,
            got
        }],
    );
    // And at the default budget, the ordinary case: refused ONCE, not Busy.
    assert_eq!(
        answer(Params::default(), ops),
        vec![State::TooLarge {
            bound: WriteBound::CommitBlocks,
            limit: Params::default().max_commit_blocks,
            got
        }],
    );
    println!(
        "  200 x 2 KiB distinct values: {got} blocks; accepted at {got}, TooLarge at {}",
        got - 1
    );
}

/// The fixture trap, pinned: the budget counts distinct BLOCKS, not ops.
#[test]
fn identical_values_are_one_block_so_count_is_not_the_bound() {
    let same: Vec<(Vec<u8>, Op)> = (0..200)
        .map(|i| {
            (
                format!("bulk/{i:05}").into_bytes(),
                Op::Put(vec![7u8; 2048]),
            )
        })
        .collect();
    assert_eq!(
        answer(Params::default(), same),
        vec![State::Accepted],
        "200 identical values were refused -- the budget no longer counts distinct blocks"
    );
}

#[test]
fn over_the_write_byte_bound_is_too_large_with_its_size_and_the_boundary_is_exact() {
    let ops = vec![(b"k".to_vec(), Op::Put(vec![1u8; 5000]))];
    // The op (key 1 + value 5000) AND its read: every write names what it read
    // (sdk#235, W8), this one forced — `k` read as `Any` — and a read costs its
    // key plus 33 bytes in the engine's count (`on_write`'s `size`).
    let size = (1 + 5000) + (1 + 33);
    assert_eq!(
        answer(
            Params {
                max_write_bytes: size,
                ..Params::default()
            },
            ops.clone()
        ),
        vec![State::Accepted],
        "a write of exactly {size} bytes was refused at a bound of {size}"
    );
    assert_eq!(
        answer(
            Params {
                max_write_bytes: size - 1,
                ..Params::default()
            },
            ops
        ),
        vec![State::TooLarge {
            bound: WriteBound::WriteBytes,
            limit: size - 1,
            got: size
        }],
    );
}

/// CONDITION 1, as the architect amended it: a refusal leaves the engine
/// EXACTLY as it found it -- the whole context, byte for byte, so nothing it
/// grows later can escape the comparison. For both bounds, on an engine with
/// history.
#[test]
fn a_too_large_refusal_leaves_the_whole_context_byte_identical() {
    for (label, params, ops) in [
        ("commit blocks", Params::default(), batch(200, 2048)),
        (
            "write bytes",
            Params {
                max_write_bytes: 4000,
                ..Params::default()
            },
            vec![(b"k".to_vec(), Op::Put(vec![1u8; 5000]))],
        ),
    ] {
        let mut e = history(params);
        let before = common::fingerprint(&e);
        let root = e.root();
        let out = stepped!(e, write(99, ops));
        assert!(
            matches!(states(&out).as_slice(), [State::TooLarge { .. }]),
            "{label}: not refused TooLarge: {:?}",
            states(&out)
        );
        assert!(
            !out.iter().any(|f| matches!(
                f,
                Effect::PutBlock { .. } | Effect::PutPack { .. } | Effect::UpdateHead { .. }
            )),
            "{label}: a refused write put something on the network"
        );
        assert_eq!(e.root(), root, "{label}: the tree moved");
        assert_eq!(common::fingerprint(&e), before, "{label}: the refusal changed the engine's state");
    }
}

/// THE CONTROL: a write under every bound, during a commit, is TAKEN into
/// the page's queue (R-b) -- `Accepted`, nothing put -- never `TooLarge`, so
/// the refusals above are the bounds and not the commit in flight.
#[test]
fn control_a_write_during_a_commit_is_queued() {
    let mut e = common::new_store_params(Params::default());
    let _ = stepped!(e, write(1, vec![(b"a".to_vec(), Op::Put(vec![1u8; 100]))]));
    let out = stepped!(e, write(2, vec![(b"b".to_vec(), Op::Put(vec![2u8; 100]))]));
    assert_eq!(states(&out), vec![State::Accepted]);
    assert!(
        !out.iter().any(|f| matches!(f, Effect::PutBlock { .. } | Effect::PutPack { .. } | Effect::UpdateHead { .. })),
        "a queued write put something on the network before its turn"
    );
    assert_eq!(e.queued_writes(), 2, "the write was not queued behind the commit");
}
