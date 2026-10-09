//! DEFERRED WRITES (sdk#350): a write can say "commit only in company". The
//! owner's rule is that VIEWING writes nothing, and a published app's schema
//! defines on its own tree are sent at open; marked deferred they are held,
//! `Accepted`, until the person's first write, and then ride that write's cut.
//!
//! * defines alone: no cut, however long they wait;
//! * the first data write: ONE cut carrying the defines and the write;
//! * a define NOT deferred (the builder's) commits alone, as before;
//! * a data write that is REFUSED takes nothing with it: the defines stay held;
//! * a held define is not UNSAVED (nothing the person made).

use engine::{ClientId, Effect, Engine, Event, Expect, Op, Params, State, WriteId};
use std::collections::BTreeSet;

mod common;
use common::Store;

fn put(k: &str, v: &[u8]) -> (Vec<u8>, Op) {
    (k.as_bytes().to_vec(), Op::Put(v.to_vec()))
}

fn define(id: u64, k: &str) -> Event {
    Event::forced_write(ClientId(1), WriteId(id), vec![put(k, b"schema")]).deferred()
}

fn data(id: u64, k: &str) -> Event {
    Event::forced_write(ClientId(1), WriteId(id), vec![put(k, b"row")])
}

fn heads(fx: &[Effect]) -> Vec<u64> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::UpdateHead { seq, .. } => Some(*seq),
            _ => None,
        })
        .collect()
}

fn puts(fx: &[Effect]) -> usize {
    fx.iter().filter(|f| matches!(f, Effect::PutBlock { .. })).count()
}

fn told(fx: &[Effect], id: u64) -> Vec<State> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::Notify { write_id, state, .. } if *write_id == WriteId(id) => Some(*state),
            _ => None,
        })
        .collect()
}

/// Step `ev`, then ack every PUT and head the engine asks for, until it asks
/// for none. Every effect, in order.
fn settle(e: &mut Engine<Store>, ev: Event) -> Vec<Effect> {
    let mut all = Vec::new();
    let mut q = e.step(ev);
    let mut guard = 0;
    while !q.is_empty() {
        guard += 1;
        assert!(guard < 10_000, "the engine did not settle");
        e.blocks().absorb(&q);
        let mut next = Vec::new();
        for f in &q {
            match f {
                Effect::PutBlock { id, .. } => next.extend(e.step(Event::PutConfirmed(*id))),
                Effect::UpdateHead { seq, .. } => next.extend(e.step(Event::HeadConfirmed(*seq))),
                other => common::no_answer_owed(other),
            }
        }
        all.append(&mut q);
        q = next;
    }
    all
}

fn engine() -> Engine<Store> {
    common::new_store_params(Params::default())
}

#[test]
fn defines_alone_are_held_accepted_and_never_cut() {
    let mut e = engine();
    let mut all = Vec::new();
    for (id, k) in [(1, "s/notes"), (2, "s/guests")] {
        all.extend(settle(&mut e, define(id, k)));
    }
    for _ in 0..50 {
        all.extend(settle(&mut e, Event::Tick(1_000_000)));
    }
    assert_eq!(puts(&all), 0, "a held define put a block");
    assert!(heads(&all).is_empty(), "a held define moved the head");
    assert_eq!(told(&all, 1), vec![State::Accepted], "a held define was told more than Accepted");
    assert_eq!(e.queued_writes(), 2, "the defines left the queue");
    assert_eq!(e.unsaved_writes(), 0, "a held define counts as unsaved");
}

#[test]
fn the_first_data_write_takes_the_defines_in_one_cut() {
    let mut e = engine();
    let mut all = settle(&mut e, define(1, "s/notes"));
    all.extend(settle(&mut e, define(2, "s/guests")));
    let fx = settle(&mut e, data(3, "r/0001"));
    all.extend(fx.clone());
    assert_eq!(heads(&fx), vec![1], "not ONE cut for the defines and the write");
    for id in [1, 2, 3] {
        assert!(told(&all, id).contains(&State::Published), "write {id} not published: {:?}", told(&all, id));
    }
    assert_eq!(e.queued_writes(), 0);
    assert_eq!(e.unsaved_writes(), 0);
}

/// THE CONTROL: the builder's define is not deferred, and commits alone.
#[test]
fn a_define_not_deferred_commits_alone() {
    let mut e = engine();
    let fx = settle(&mut e, Event::forced_write(ClientId(1), WriteId(1), vec![put("s/notes", b"schema")]));
    assert_eq!(heads(&fx), vec![1], "a builder define did not commit");
    assert!(told(&fx, 1).contains(&State::Published));
}

/// A data write REFUSED where it lands (a read that no longer holds) takes
/// nothing with it: 0 commits, the defines held -- and the next good write
/// still carries them.
#[test]
fn a_refused_data_write_leaves_the_defines_held() {
    let mut e = engine();
    // A published row, so a read of it as ABSENT is stale.
    let base = settle(&mut e, data(1, "r/0001"));
    assert_eq!(heads(&base), vec![1]);
    let mut all = settle(&mut e, define(2, "s/notes"));
    let stale = Event::Write { client: ClientId(1), write_id: WriteId(3), ops: vec![put("r/0001", b"mine")], reads: vec![(b"r/0001".to_vec(), Expect::Absent)], deferred: false };
    let fx = settle(&mut e, stale);
    all.extend(fx.clone());
    assert!(told(&fx, 3).contains(&State::Conflict), "the stale write was not refused: {:?}", told(&fx, 3));
    assert_eq!(puts(&all), 0, "a define committed with a refused write");
    assert!(heads(&all).is_empty(), "the head moved for the defines alone");
    assert_eq!(e.unsaved_writes(), 0, "the held define counts as unsaved");
    let good = settle(&mut e, data(4, "r/0002"));
    assert_eq!(heads(&good), vec![2], "the next write did not carry the held define in one cut");
    for id in [2, 4] {
        assert!(told(&good, id).contains(&State::Published), "write {id}: {:?}", told(&good, id));
    }
}

/// UNSAVED, its one owner: a held define is not unsaved; once a data write's
/// cut carries it, it is -- until that commit publishes.
#[test]
fn unsaved_counts_a_define_once_a_cut_carries_it() {
    let mut e = engine();
    let _ = settle(&mut e, define(1, "s/notes"));
    assert_eq!(e.unsaved_writes(), 0);
    let fx = e.step(data(2, "r/0001"));
    e.blocks().absorb(&fx);
    assert!(!heads(&fx).is_empty() || puts(&fx) > 0, "the data write was not cut");
    assert_eq!(e.unsaved_writes(), 2, "the define in the cut is not unsaved, or the write is not");
    let written: BTreeSet<u64> = fx
        .iter()
        .filter_map(|f| match f {
            Effect::Notify { write_id, state: State::Accepted, .. } => Some(write_id.0),
            _ => None,
        })
        .collect();
    assert!(written.contains(&2));
}
