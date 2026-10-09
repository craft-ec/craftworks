//! W8 — A WRITE NAMES WHAT IT READ (sdk#235, M2 ruling 1; WRITE-PATH.md).
//!
//! Every key a write changes has a read entry, or the engine refuses the write
//! as `Unread` AT THE DOOR: syntactic (the write's own op keys against its own
//! read keys), before any other answer, reading no state. So it can never
//! follow `Accepted` — including for a write parked before its head, which is
//! answered `Accepted` at once (sdk#223).

use engine::{ClientId, Effect, Engine, Event, Expect, Op, Params, State, WriteId};

mod common;
use common::Store;

fn write(id: u64, ops: Vec<(&str, Op)>, reads: Vec<(&str, Expect)>) -> Event {
    Event::Write {
        client: ClientId(1),
        write_id: WriteId(id),
        ops: ops.into_iter().map(|(k, o)| (k.as_bytes().to_vec(), o)).collect(),
        reads: reads.into_iter().map(|(k, e)| (k.as_bytes().to_vec(), e)).collect(),
        deferred: false,
    }
}

fn told(fx: &[Effect], id: u64) -> Vec<State> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::Notify { write_id, state, .. } if write_id.0 == id => Some(*state),
            _ => None,
        })
        .collect()
}

fn unread_key(fx: &[Effect]) -> Option<Vec<u8>> {
    fx.iter().find_map(|f| match f {
        Effect::Unread { key, .. } => Some(key.clone()),
        _ => None,
    })
}

/// A recovered engine, through the engine tests' fixture (it tells a fresh
/// engine its tree is new).
fn recovered() -> Engine<Store> {
    common::new_store_params(Params::default())
}

/// **A key written and not read is refused AT THE DOOR, naming it, and the
/// context is byte-identical** — no number consumed, nothing parked, no op in
/// the tree.
#[test]
fn a_key_written_unread_is_refused_at_the_door_and_nothing_moves() {
    let mut e = recovered();
    let before = common::fingerprint(&e);
    let fx = e.step(write(1, vec![("k/a", Op::Put(b"1".to_vec())), ("k/b", Op::Put(b"2".to_vec()))], vec![("k/a", Expect::Absent)]));
    assert_eq!(told(&fx, 1), vec![State::Unread], "not refused as Unread: {fx:?}");
    assert_eq!(unread_key(&fx).as_deref(), Some(&b"k/b"[..]), "the refusal does not name the unread key");
    assert_eq!(fx.len(), 2, "anything but the verdict and its key was emitted: {fx:?}");
    assert_eq!(common::fingerprint(&e), before, "the engine's state moved for a write refused at the door");
    assert_eq!(e.forced_writes(), 0);
    // THE CONTROL: the same write with both reads is taken.
    let ok = e.step(write(1, vec![("k/a", Op::Put(b"1".to_vec())), ("k/b", Op::Put(b"2".to_vec()))], vec![("k/a", Expect::Absent), ("k/b", Expect::Absent)]));
    assert!(told(&ok, 1).contains(&State::Accepted), "THE CONTROL: a write that read its keys was not taken: {ok:?}");
}

/// **A plain write (no reads at all) is refused the same way.**
#[test]
fn a_write_with_no_reads_is_refused() {
    let mut e = recovered();
    let fx = e.step(write(1, vec![("k/a", Op::Delete)], vec![]));
    assert_eq!(told(&fx, 1), vec![State::Unread]);
    assert_eq!(unread_key(&fx).as_deref(), Some(&b"k/a"[..]));
}

// Before the head: see testkit/tests/writes_within_reads_before_the_head.rs —
// an engine that has not read its head is a PAGE that has not asked
// `Identity`, which the testkit fixture builds (fixture-gate).

/// **During a commit: Unread** — the door comes before the queue, so a
/// malformed write is refused in the same words whatever the engine is doing.
#[test]
fn during_a_commit_an_unread_write_is_unread_not_busy() {
    let mut e = recovered();
    let first = e.step(write(1, vec![("k/a", Op::Put(vec![1; 3000]))], vec![("k/a", Expect::Absent)]));
    assert!(told(&first, 1).contains(&State::Accepted), "{first:?}");
    // The page keeps what the engine emits (its warm blocks among them).
    e.blocks().absorb(&first);
    let fx = e.step(write(2, vec![("k/b", Op::Put(b"2".to_vec()))], vec![]));
    assert_eq!(told(&fx, 2), vec![State::Unread], "judged after the queue, not at the door: {fx:?}");
    assert_eq!(e.queued_writes(), 1, "a write refused at the door joined the queue");
    // THE CONTROL: a well-formed write now QUEUES behind the commit (R-b), so
    // a commit IS in flight.
    let queued = e.step(write(3, vec![("k/c", Op::Put(b"3".to_vec()))], vec![("k/c", Expect::Absent)]));
    assert_eq!(told(&queued, 3), vec![State::Accepted], "THE CONTROL: a well-formed write was not taken");
    assert_eq!(e.queued_writes(), 2, "THE CONTROL: no commit was in flight, so this proves nothing");
}

/// **`Any` is accepted, holds against any tree, and is COUNTED.**
#[test]
fn a_forced_write_is_taken_and_counted() {
    let mut e = recovered();
    let a = e.step(write(1, vec![("k/a", Op::Put(b"1".to_vec()))], vec![("k/a", Expect::Absent)]));
    assert!(told(&a, 1).contains(&State::Accepted));
    assert_eq!(e.forced_writes(), 0, "THE CONTROL: a write that read its key is not forced");
    drive(&mut e, a);
    // k/a now holds "1"; a forced write over it holds whatever is there.
    let f = e.step(write(2, vec![("k/a", Op::Put(b"2".to_vec()))], vec![("k/a", Expect::Any)]));
    assert!(told(&f, 2).contains(&State::Accepted), "a forced write was not taken: {f:?}");
    assert_eq!(e.forced_writes(), 1, "a forced write was not counted");
}

/// Confirm every put the engine asked for, from these effects on, until the
/// commit has published — as `common::tree` drives one.
fn drive(e: &mut Engine<Store>, first: Vec<Effect>) {
    e.blocks().absorb(&first);
    let mut queue = first;
    for _ in 0..100_000 {
        let Some(f) = queue.pop() else { return };
        let next = match f {
            Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } => Event::PutConfirmed(id),
            Effect::UpdateHead { seq, .. } => Event::HeadConfirmed(seq),
            _ => continue,
        };
        let out = e.step(next);
        e.blocks().absorb(&out);
        queue.extend(out);
    }
    panic!("the commit did not settle");
}

/// **A write with AT LEAST ONE `Any` read is counted** — a MIXED one included
/// (WRITE-PATH W8: "each accepted one is counted", and what `Session.tick()`
/// shows is this count). One key forced, one key read for real: 0 → 1.
#[test]
fn a_write_mixing_one_forced_key_with_one_real_read_is_counted() {
    let mut e = recovered();
    assert_eq!(e.forced_writes(), 0);
    let fx = e.step(write(
        1,
        vec![("k/a", Op::Put(b"1".to_vec())), ("k/b", Op::Put(b"2".to_vec()))],
        vec![("k/a", Expect::Any), ("k/b", Expect::Absent)],
    ));
    assert!(told(&fx, 1).contains(&State::Accepted), "the mixed write was not taken: {fx:?}");
    assert_eq!(e.forced_writes(), 1, "a write with one forced key and one real read was not counted as forced");
}
