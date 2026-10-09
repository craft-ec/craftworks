//! A commit that hears nothing (COMMIT-LIFE rev 5, C5; rule 8 "No time cut-offs"): SILENCE NEVER ENDS IT. A Racing
//! commit's PUTs re-send in `Page::send` until answered -- the engine re-puts nothing on a clock and ends nothing on
//! one -- and a Heading commit's head is ASKED about on its pace (E12), a question whose answer is E6, E7 or E8. The
//! later writes QUEUE behind it (K1: never `Busy`).
//!
//! (Before sdk#481 a silent Racing commit was re-put from its carried ops and, past three settle rounds or with ops
//! too large to carry, ended `Lost`: sdk#150 E1/E2, delegate-era. The two tests that pinned it are rewritten below to
//! the C5 cell.)
use engine::{ClientId, Effect, Event, Op, Params, State, WriteId};
use freenet_prolly::Cid;
use std::collections::BTreeSet;

mod common;
use common::{Harness, Store};

const T0: u64 = 1_790_000_000;

fn write(id: u64, ops: Vec<(Vec<u8>, Op)>) -> Event {
    Event::forced_write(ClientId(1), WriteId(id), ops)
}

fn told(fx: &[Effect], id: u64) -> Vec<State> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::Notify { write_id, state, .. } if write_id.0 == id => Some(*state),
            _ => None,
        })
        .collect()
}

fn puts(fx: &[Effect]) -> Vec<Cid> {
    fx.iter()
        .filter_map(|f| match f {
            Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

/// Small ops: a few keys, a few KiB in all -- carried with the commit.
fn small() -> Vec<(Vec<u8>, Op)> {
    (0..8u8)
        .map(|i| (format!("s/{i}").into_bytes(), Op::Put(vec![i; 700])))
        .collect()
}

/// Ops too large to carry: one 30 KiB value.
fn large() -> Vec<(Vec<u8>, Op)> {
    vec![(b"l/big".to_vec(), Op::Put(vec![9u8; 30 * 1024]))]
}

fn harness() -> Harness {
    let mut h = Harness::new(Params::default(), Store::fresh());
    let _ = h.step(Event::Tick(T0));
    h
}

/// Answer everything in `fx` (blocks confirmed, head confirmed), and what
/// that produces, to a standstill.
fn answer(h: &mut Harness, mut fx: Vec<Effect>) -> Vec<Effect> {
    let mut all = fx.clone();
    for _ in 0..50 {
        let mut next = Vec::new();
        for f in &fx {
            match f {
                Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } => {
                    h.store.remember(id);
                    next.extend(h.step(Event::PutConfirmed(*id)))
                }
                Effect::UpdateHead { seq, .. } => next.extend(h.step(Event::HeadConfirmed(*seq))),
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

/// Tick until something is told to `id`, up to `n` ticks; everything emitted.
fn tick_until(h: &mut Harness, from: u64, n: u64, id: u64) -> (u64, Vec<Effect>) {
    let mut out = Vec::new();
    for k in 1..=n {
        let fx = h.step(Event::Tick(T0 + from + k));
        let heard = !told(&fx, id).is_empty() || !puts(&fx).is_empty();
        out.extend(fx);
        if heard {
            return (from + k, out);
        }
    }
    (from + n, out)
}

/// **Racing x E12, small ops** (was: "a dropped commit with carried ops is re-put and publishes"). Every data put
/// DROPPED, and the engine hears nothing for many paces: it re-puts NOTHING on the clock (the page's `send` owns the
/// retry) and ends nothing -- no `Lost`, no try spent. When the blocks land, it publishes.
#[test]
fn a_silent_racing_commit_is_not_re_put_on_a_clock_and_publishes_when_its_puts_land() {
    let p = Params::default();
    let mut h = harness();
    let first = h.step(write(1, small()));
    assert_eq!(told(&first, 1), vec![State::Accepted]);
    let sent: BTreeSet<Cid> = puts(&first).into_iter().collect();
    assert!(!sent.is_empty());
    for id in &sent {
        h.store.forget(*id); // the puts never arrived
    }
    let (_, fx) = tick_until(&mut h, 0, 20 * p.reask_after, 1);
    assert!(puts(&fx).is_empty(), "the engine re-put on a clock: {} PUTs", puts(&fx).len());
    assert!(!told(&fx, 1).iter().any(|s| matches!(s, State::Lost | State::Failed { .. })), "silence ended the write: {:?}", told(&fx, 1));
    let resent: Vec<Effect> = sent.iter().map(|id| Effect::PutBlock { id: *id, bytes: Vec::new(), after: Vec::new() }).collect();
    let all = answer(&mut h, resent);
    assert!(told(&all, 1).contains(&State::Published), "{:?}", told(&all, 1));
}

/// **Racing x E12, large ops** (was: "a dropped commit too large to carry is Lost and the engine released"). The same
/// silence with ops that were too large to carry: nothing is carried any more, and nothing ends on time -- the write
/// is never `Lost`, and the next write QUEUES behind the commit (`Accepted`, never `Busy`, K1).
#[test]
fn a_silent_racing_commit_of_large_ops_is_never_lost_and_the_next_write_queues() {
    let p = Params::default();
    let mut h = harness();
    let first = h.step(write(1, large()));
    for id in puts(&first) {
        h.store.forget(id);
    }
    let (_, fx) = tick_until(&mut h, 0, 20 * p.reask_after, 1);
    assert!(!told(&fx, 1).contains(&State::Lost), "silence ended the write Lost");
    // Taken into the queue (its path may be cold -- the commit's blocks never reached the node -- so it may wait
    // `Applying`): never refused.
    let next = h.step(write(2, small()));
    assert!(!told(&next, 2).iter().any(|s| matches!(s, State::Busy | State::Failed { .. } | State::Lost)), "the next write was refused: {:?}", told(&next, 2));
    assert_eq!(h.engine().queued_writes(), 2, "the next write was not taken into the queue");
}


/// E1: the head was sent and never landed -- the read back shows the OLDER
/// seq. That is not a conflict: the same head is issued again, and the
/// commit publishes.
#[test]
fn a_head_that_never_landed_is_issued_again_not_taken_for_a_conflict() {
    let p = Params::default();
    let mut h = harness();
    let first = h.step(write(1, small()));
    // Blocks confirmed as answered; the head effect is NOT answered.
    let mut head = None;
    for f in answer_blocks_only(&mut h, first) {
        if let Effect::UpdateHead { seq, root, .. } = f {
            head = Some((seq, root));
        }
    }
    let (seq, root) = head.expect("the head was sent once the blocks were in");
    // Silence past the pace: the engine asks what is true.
    let (_, fx) = tick_until(&mut h, 0, 2 * p.reask_after, 1);
    assert!(fx.iter().any(|f| matches!(f, Effect::ReadHead { .. })), "the head was never read");
    // The Register still holds the older (empty) head.
    let reissued = h.step(Event::HeadConflict { seq: seq - 1, root: h.published_root() });
    assert!(
        reissued.iter().any(|f| matches!(f, Effect::UpdateHead { seq: s, root: r, .. } if *s == seq && *r == root)),
        "an older head was taken for a conflict: {:?}",
        told(&reissued, 1)
    );
    assert!(!told(&reissued, 1).contains(&State::Lost));
    let done = h.step(Event::HeadConfirmed(seq));
    assert!(told(&done, 1).contains(&State::Published));
}

/// E1's other side (sdk#225): the head this commit was BUILT ON is displaced
/// — the Register shows ITS seq under ANOTHER root (another device of the
/// same identity won the tie-break). That is not "my head never landed": the
/// commit is dead, its writes are told `Lost`, and the winner is adopted. E1
/// alone let the page ask to sign from a displaced prev for ever.
#[test]
fn a_displaced_prev_under_a_pending_commit_is_a_conflict_not_an_unlanded_head() {
    let p = Params::default();
    let mut h = harness();
    let first = h.step(write(1, small()));
    let mut head = None;
    for f in answer_blocks_only(&mut h, first) {
        if let Effect::UpdateHead { seq, .. } = f {
            head = Some(seq);
        }
    }
    let seq = head.expect("the head was sent once the blocks were in");
    let (_, _) = tick_until(&mut h, 0, 2 * p.reask_after, 1);
    let winner: Cid = [7; 32];
    assert_ne!(winner, h.published_root());
    let fx = h.step(Event::HeadConflict { seq: seq - 1, root: winner });
    assert!(told(&fx, 1).contains(&State::Lost), "a displaced prev was taken for an unlanded head: {:?}", told(&fx, 1));
    assert_eq!(h.published_root(), winner, "the winner was not adopted");
}

/// Answer only the block puts in `fx` and what they cause; return every
/// effect seen, head effects unanswered.
fn answer_blocks_only(h: &mut Harness, mut fx: Vec<Effect>) -> Vec<Effect> {
    let mut all = fx.clone();
    for _ in 0..50 {
        let mut next = Vec::new();
        for f in &fx {
            if let Effect::PutBlock { id, .. } | Effect::PutPack { id, .. } = f {
                next.extend(h.step(Event::PutConfirmed(*id)));
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

