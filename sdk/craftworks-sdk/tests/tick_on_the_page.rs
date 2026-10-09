//! WHAT A TICK CAUSES, ON THE PAGE PATH (re-stated from the Shell's
//! `what_tick_causes.rs`, deleted with the Shell).
//!
//! Each behaviour runs twice through a real tab (`page::Server` over the page
//! path's node): once with no `Tick` at all, once with a `Tick` past the
//! deadline. A row that fires in both columns does not need time; a row
//! that fires in neither is a finding of its own. A commit is held STUCK by
//! holding the node's answers: its puts land, and nothing is acknowledged.

use testkit::page_node::Served;
use testkit::{PageConn, PageNode};

fn write(n: u64) -> protocol::Request {
    protocol::Request::forced_write(n, vec![protocol::Op::Put(format!("k/{n:04}").into_bytes(), vec![0x5A; 64])])
}

/// Ticks between calls: a throttled tab, 30 s apart. Five steps are 150 s,
/// well past `max_accept_age` (64).
const STEP: u64 = 30;

/// The write states in a run's replies, for write 1.
fn states(frames: &[Vec<u8>]) -> Vec<protocol::WriteState> {
    frames
        .iter()
        .filter_map(|f| protocol::decode_reply(f).ok())
        .filter_map(|r| match r {
            protocol::Reply::WriteState { write_id: 1, state } | protocol::Reply::SessionWriteState { write_id: 1, state, .. } => Some(state),
            _ => None,
        })
        .collect()
}

/// A tab with write 1 made and held stuck (every node answer held).
fn stuck() -> (PageNode, PageConn, Vec<protocol::WriteState>) {
    let node = PageNode::new();
    let mut c = node.connect();
    c.client(&protocol::Request::Identity);
    c.hold_answers();
    let seen = states(&c.client(&write(1)));
    (node, c, seen)
}

/// The SDK's `Tick` at `now` seconds after the start, as a page sends it:
/// the page's clock moves with it.
fn tick(c: &mut PageConn, start_ms: u64, secs: u64) -> Vec<protocol::WriteState> {
    states(&c.tick_at(start_ms + secs * 1_000))
}

/// Run a stuck write, then either tick past the deadline or ask after it with
/// no time, the same number of calls; say whether `Stalled` was reported.
fn stalled_reported(with_tick: bool) -> bool {
    let (_node, mut c, mut seen) = stuck();
    let start = c.now_ms();
    for i in 1..=5u64 {
        if with_tick {
            seen.extend(tick(&mut c, start, i * STEP));
        } else {
            seen.extend(states(&c.client(&protocol::Request::AskWrite { write_id: 1 })));
        }
    }
    seen.contains(&protocol::WriteState::Stalled)
}

/// **A stuck commit is reported `Stalled`**, once time has passed the bound.
#[test]
fn a_stuck_commit_is_reported_stalled_after_the_bound() {
    assert!(stalled_reported(true), "a commit stuck well past max_accept_age was never reported Stalled: nothing would ever tell a person their write is held, not saved, and not moving");
}

/// THE CONTROL: with no time sent, it is never reported.
#[test]
fn control_without_time_a_stuck_commit_is_never_stalled() {
    assert!(!stalled_reported(false), "Stalled was reported with no Tick at all, so it does not depend on time");
}

/// **Reported ONCE, not on every tick.**
#[test]
fn stalled_is_reported_once_not_on_every_tick() {
    let (_node, mut c, _) = stuck();
    let start = c.now_ms();
    let mut stalls = 0;
    for i in 1..=6u64 {
        stalls += tick(&mut c, start, i * STEP).iter().filter(|s| **s == protocol::WriteState::Stalled).count();
    }
    assert_eq!(stalls, 1, "a stuck commit was reported Stalled {stalls} times over six ticks; a notice repeated every tick is one a caller learns to ignore");
}

/// A commit that PUBLISHES in time is never called stalled.
#[test]
fn control_a_commit_that_publishes_is_never_stalled() {
    let node = PageNode::new();
    let mut c = node.connect();
    c.client(&protocol::Request::Identity);
    let mut seen = states(&c.client(&write(1)));
    let start = c.now_ms();
    for i in 1..=6u64 {
        seen.extend(tick(&mut c, start, i * STEP));
    }
    assert!(seen.contains(&protocol::WriteState::Published), "the control never published: {seen:?}");
    assert!(!seen.contains(&protocol::WriteState::Stalled), "a commit that published was reported Stalled: {seen:?}");
}

/// THE ZERO THIS SCENARIO REALLY MEASURES: 40 small writes owe no parity
/// group (no leaf lists a trio), and neither a Tick nor a Flush causes a put.
/// The parity rows with a group owed are `parity_on_tick_and_flush.rs`'s.
#[test]
fn with_nothing_owed_a_tick_or_a_flush_causes_no_put() {
    let run = |with_tick: bool, with_flush: bool| {
        let node = PageNode::new();
        let mut c = node.connect();
        c.client(&protocol::Request::Identity);
        for i in 1..=40 {
            c.client(&write(i));
        }
        assert_eq!(c.with_server(|s| s.page.owed_groups()), 0, "the precondition failed: a group IS owed");
        let before = c.served(Served::Put);
        if with_tick {
            let t = c.now_ms() + 10_000_000;
            c.tick_at(t);
        }
        if with_flush {
            c.client(&protocol::Request::Flush);
        }
        c.served(Served::Put) - before
    };
    assert_eq!((run(false, false), run(true, false), run(false, true)), (0, 0, 0), "a put WAS caused with nothing owed");
}

/// **THE ENGINE'S DEADLINES ARE COUNTS OF `protocol::TICK_MS`.**
///
/// `max_accept_age` is 64, and it reads as 64 SECONDS only because the page
/// quantises its clock at a second. Move `TICK_MS` and every deadline in the
/// engine rescales at once, silently, with nothing failing. (`parity_age`
/// went with the owed-parity machine: under race put, COMMIT-LIFE §P, parity
/// goes with its commit and nothing is paced.)
///
/// Two crates, no shared dependency (`engine` does not depend on `protocol`),
/// so nothing but this can hold them together. It is a tripwire, not a
/// calculation.
#[test]
fn the_engine_s_bounds_are_stated_in_the_page_s_unit() {
    let p = engine::Params::default();
    assert_eq!(
        protocol::TICK_MS,
        1_000,
        "TICK_MS moved. `max_accept_age` ({}) is a COUNT of it, so it now means {} of the new quantum rather \
         than the seconds its doc claims. Convert it in the same commit, or say in `Params` what the unit is now.",
        p.max_accept_age,
        p.max_accept_age
    );
    assert_eq!(
        p.max_accept_age * protocol::TICK_MS / 1_000,
        64,
        "max_accept_age is documented as 64 seconds"
    );
}
