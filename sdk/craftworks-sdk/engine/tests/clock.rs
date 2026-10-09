//! sdk#150: the engine's clock survives the call.
//!
//! `in_flight_since` joined the context in sdk#81 and the clock it is
//! measured against did not, so every call but a Tick's began at `now = 0`.
//! Two consequences, pinned here with the engine rebuilt from its context on
//! EVERY step, as a delegate is:
//!
//! * a commit started in a call with no Tick took `in_flight_since = 0`, and
//!   against an epoch-seconds tick was `Stalled` at once;
//! * a commit's age was measured from the wrong moment, so `Stalled` came at
//!   the wrong tick -- the carried clock is what makes it exactly
//!   `max_accept_age` ticks after the commit started.

use engine::{ClientId, Effect, Event, Op, Params, State, WriteId};

mod common;
use common::{Harness, Store};

fn stalled(fx: &[Effect]) -> bool {
    fx.iter().any(|f| {
        matches!(
            f,
            Effect::Notify {
                state: State::Stalled,
                ..
            }
        )
    })
}

fn write() -> Event {
    Event::forced_write(ClientId(1), WriteId(1), vec![(b"k".to_vec(), Op::Put(vec![1u8; 40]))])
}

/// Epoch seconds, as `protocol::tick_of` produces them.
const T0: u64 = 1_790_000_000;

#[test]
fn a_commit_is_stalled_exactly_max_accept_age_ticks_after_it_started() {
    let age = Params::default().max_accept_age;
    let mut h = Harness::new(Params::default(), Store::fresh());
    // The clock is known BEFORE the write, and the write's own call has no
    // tick: only a carried clock can date the commit's start.
    let _ = h.step(Event::Tick(T0));
    let _ = h.step(write()); // its puts are never confirmed: it stays in flight
    let before: Vec<bool> = (1..age)
        .map(|k| stalled(&h.step(Event::Tick(T0 + k))))
        .collect();
    assert!(
        !before.iter().any(|s| *s),
        "Stalled before max_accept_age ({age}) ticks: at tick {:?}",
        before.iter().position(|s| *s).map(|i| i + 1)
    );
    assert!(
        stalled(&h.step(Event::Tick(T0 + age))),
        "not Stalled at exactly max_accept_age ({age}) ticks after the commit started"
    );
}

#[test]
fn a_commit_started_before_any_clock_is_dated_from_the_first_one() {
    let age = Params::default().max_accept_age;
    let mut h = Harness::new(Params::default(), Store::fresh());
    let _ = h.step(write()); // no tick has ever arrived
    assert!(
        !stalled(&h.step(Event::Tick(T0))),
        "Stalled on the first tick: a start before any clock was measured from 0"
    );
    let early = (1..age).any(|k| stalled(&h.step(Event::Tick(T0 + k))));
    assert!(
        !early,
        "Stalled before max_accept_age ticks from the first clock"
    );
    assert!(stalled(&h.step(Event::Tick(T0 + age))));
}
