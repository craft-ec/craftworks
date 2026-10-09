//! THE QUEUE COUNTS WHAT IT HOLDS (sdk#450, rule 9): `QueueFull` is measured against `Engine::queued_bytes` -- the
//! queued writes' OPS bytes plus the W pin's warm-apply blocks, each held block once. Before, only the ops counted,
//! and a 40k-write queue pinned 227 MB of warm blocks behind a 32 MiB bound (#411's probe B).

use engine::{ClientId, Effect, Event, Op, Params, State, WriteId};

mod common;
use common::new_store_params;

fn write(id: u64, key: &str, value: &[u8]) -> Event {
    Event::forced_write(ClientId(1), WriteId(id), vec![(key.as_bytes().to_vec(), Op::Put(value.to_vec()))])
}

/// Step `e` and keep what it emitted, as the page does (its store IS the page's memory): a queued write's warm
/// apply reads the tree the one before it made.
fn step(e: &mut engine::Engine<common::Store>, ev: Event) -> Vec<Effect> {
    let fx = e.step(ev);
    e.blocks().absorb(&fx);
    fx
}

fn queue_full(fx: &[Effect]) -> bool {
    fx.iter().any(|f| matches!(f, Effect::Notify { state: State::QueueFull { .. }, .. }))
}

/// **QueueFull comes at ops + warm, not at ops alone.** A commit is in flight (none of its PUTs answered), and the
/// same session queues 1 KiB writes behind it until `QueueFull`. Each queued write's warm apply emits blocks the
/// page must keep (the only copy): the bound counts them. Mutant "ops only" -> several times more writes admitted
/// -> red on the COUNT.
#[test]
fn queue_full_is_measured_against_ops_plus_the_warm_blocks_the_queue_pins() {
    let limit = 64 * 1024;
    let mut e = new_store_params(Params { max_queue_bytes: limit, ..Params::default() });
    let first = step(&mut e, write(1, "k/00000", &[1u8; 1024]));
    assert!(first.iter().any(|f| matches!(f, Effect::PutBlock { .. })), "THE SETUP: the first write did not open a commit");
    let mut admitted = 0u64;
    for i in 2..10_000u64 {
        let fx = step(&mut e, write(i, &format!("k/{i:05}"), &[(i % 251) as u8; 1024]));
        if queue_full(&fx) {
            break;
        }
        admitted += 1;
    }
    let ops_only = (limit / 1024) as u64;
    let (_, bytes) = e.queue_load();
    println!("limit {limit} B: {admitted} writes admitted before QueueFull (ops alone would admit ~{ops_only}); queued bytes {bytes}");
    assert!(admitted > 0, "THE SETUP: nothing was admitted behind the commit");
    assert!(admitted * 2 < ops_only, "{admitted} 1 KiB writes admitted under a {limit} B bound: the warm blocks they pin were not counted");
}

/// **A block two queued writes emit is counted ONCE** (the architect): it is held once. Behind a commit, write A sets
/// `k` to X, write B to Y, and write C to X again: C's warm apply lands the tree back on A's and re-emits A's
/// content-addressed blocks. The queued bytes grow by C's OPS only. Mutant "sum per write" (counted per emission)
/// -> C's re-emitted blocks counted twice -> red.
#[test]
fn a_block_two_queued_writes_emit_is_counted_once() {
    let mut e = new_store_params(Params::default());
    let _ = step(&mut e, write(1, "k/00000", &[1u8; 1024]));
    let a = step(&mut e, write(2, "k/00001", &[7u8; 1024]));
    let _ = step(&mut e, write(3, "k/00001", &[8u8; 1024]));
    let (_, before_c) = e.queue_load();
    let c = step(&mut e, write(4, "k/00001", &[7u8; 1024]));
    let keep = |fx: &[Effect]| -> Vec<([u8; 32], usize)> { fx.iter().filter_map(|f| match f { Effect::Keep { id, bytes } => Some((*id, bytes.len())), _ => None }).collect() };
    let (ka, kc) = (keep(&a), keep(&c));
    let shared: usize = kc.iter().filter(|(id, _)| ka.iter().any(|(x, _)| x == id)).map(|(_, n)| n).sum();
    let (_, after_c) = e.queue_load();
    let ops = "k/00001".len() + 1024;
    println!("queued bytes {before_c} -> {after_c}; C's ops ~{ops} B; C re-emitted {shared} B of A's blocks");
    assert!(shared > 0, "THE SETUP: C's warm apply re-emitted none of A's blocks, so nothing could be counted twice");
    // C's ops cost a little more than key + value (per-op overhead); counting its re-emitted blocks again would add
    // all `shared` more.
    assert!(after_c - before_c < ops + shared / 2, "C grew the queued bytes by {} (ops ~{ops}, re-emitted {shared}): a block already held was counted again", after_c - before_c);
}
