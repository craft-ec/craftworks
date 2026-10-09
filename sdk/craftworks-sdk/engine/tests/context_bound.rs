//! The context's size bound, at construction (kept from `owed_cap.rs`, which
//! race put deleted with the owed machinery -- these two were never about
//! owed parity).
use engine::Params;

mod common;
use common::Store;

/// Construction refuses params whose caps cannot fit the context's bound.
#[test]
#[should_panic(expected = "the context's caps do not fit its bound")]
fn caps_that_cannot_fit_the_bound_are_refused_at_construction() {
    let _ = engine::Engine::new(
        Params {
            max_commit_blocks: 16 * 1024,
            ..Params::default()
        },
        Store::fresh(),
    );
}

/// ...and the default params fit, with room for parked reads.
#[test]
fn the_default_caps_fit_the_bound() {
    let _ = engine::Engine::new(Params::default(), Store::fresh());
}
