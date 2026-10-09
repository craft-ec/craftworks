//! THE MODEL TESTS' SEEDS (sdk#536): ONE knob for how many seeds every model test runs, and ONE runner that walks
//! them on every core.
//!
//! - **The knob.** `CRAFTWORKS_MODEL_SEEDS` is a count against [`REFERENCE`] (the page model's full count, 40). A
//!   model whose full sweep is `full` seeds runs [`share`]`(full)` = `full × knob / 40` of them (at least one). So the
//!   batch gate (the knob unset, or 40) runs every model's full sweep exactly as it always ran, `gate.sh --pr` a small
//!   share of each, and `CRAFTWORKS_MODEL_SEEDS=160` four times each.
//! - **In parallel.** [`run`] hands seeds to one worker per core. Each seed's run builds its own world (nothing is
//!   shared between seeds), and the results come back IN SEED ORDER, so what a test folds and prints does not depend
//!   on the core count or the schedule.
//! - **Each seed alone.** `CRAFTWORKS_MODEL_SEED=<n>` runs seed n and nothing else; a failing seed's panic names that
//!   line. A test asserts its coverage floors (the sweep reached the situations it is about) only over a whole sweep:
//!   [`alone`] says when it is not one.
use std::ops::Range;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// The page model's full count: the knob's unit.
pub const REFERENCE: u64 = 40;

/// The knob's value: a positive integer, else [`REFERENCE`].
pub fn knob_from(v: Option<&str>) -> u64 {
    v.and_then(|s| s.trim().parse().ok())
        .filter(|&n: &u64| n > 0)
        .unwrap_or(REFERENCE)
}

/// `CRAFTWORKS_MODEL_SEEDS`, read.
pub fn knob() -> u64 {
    knob_from(std::env::var("CRAFTWORKS_MODEL_SEEDS").ok().as_deref())
}

/// This run's share of a sweep whose full count is `full`: `full × knob / REFERENCE`, at least 1.
pub fn share(full: u64) -> u64 {
    share_of(full, knob())
}

pub fn share_of(full: u64, knob: u64) -> u64 {
    (full * knob / REFERENCE).max(1)
}

/// `CRAFTWORKS_MODEL_SEED=<n>`: the one seed to run alone, if set.
pub fn alone() -> Option<u64> {
    std::env::var("CRAFTWORKS_MODEL_SEED")
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Runs `f` for every seed in `seeds` (only the [`alone`] seed, when set), one worker per core, and returns
/// `(seed, result)` IN SEED ORDER. A seed that panics fails the call, naming the lowest such seed and how to run it
/// alone.
pub fn run<R: Send>(seeds: Range<u64>, f: impl Fn(u64) -> R + Sync) -> Vec<(u64, R)> {
    run_alone(seeds, alone(), f)
}

/// [`run`], with the one seed to run alone given, not read.
fn run_alone<R: Send>(
    seeds: Range<u64>,
    alone: Option<u64>,
    f: impl Fn(u64) -> R + Sync,
) -> Vec<(u64, R)> {
    let list: Vec<u64> = match alone {
        Some(s) => seeds.filter(|&x| x == s).collect(),
        None => seeds.collect(),
    };
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(list.len())
        .max(1);
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<(u64, std::thread::Result<R>)>> =
        Mutex::new(Vec::with_capacity(list.len()));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(&seed) = list.get(i) else { break };
                let r = catch_unwind(AssertUnwindSafe(|| f(seed)));
                done.lock()
                    .expect("a worker panicked holding the results")
                    .push((seed, r));
            });
        }
    });
    let mut done = done
        .into_inner()
        .expect("a worker panicked holding the results");
    done.sort_by_key(|(seed, _)| *seed);
    let mut out = Vec::with_capacity(done.len());
    for (seed, r) in done {
        match r {
            Ok(v) => out.push((seed, v)),
            Err(e) => {
                let why = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                panic!("seed {seed} panicked: {why}\n  run it alone: CRAFTWORKS_MODEL_SEED={seed}");
            }
        }
    }
    out
}

/// A sweep with a coverage floor: seeds `first..first + min` (in parallel), then further batches of `min` while
/// `reached` (over every result so far, in seed order) is false, never past `first + cap`. The batch size is `min`,
/// never the core count, so which seeds run is the same on every machine.
pub fn until<R: Send>(
    first: u64,
    min: u64,
    cap: u64,
    f: impl Fn(u64) -> R + Sync,
    reached: impl Fn(&[(u64, R)]) -> bool,
) -> Vec<(u64, R)> {
    let min = min.max(1);
    let end = first + cap.max(min);
    let mut all = run(first..first + min, &f);
    let mut at = first + min;
    while at < end && alone().is_none() && !reached(&all) {
        let to = (at + min).min(end);
        all.extend(run(at..to, &f));
        at = to;
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_knob_is_a_positive_count_or_the_reference() {
        assert_eq!(knob_from(None), REFERENCE);
        assert_eq!(knob_from(Some("4")), 4);
        assert_eq!(knob_from(Some("0")), REFERENCE);
        assert_eq!(knob_from(Some("many")), REFERENCE);
    }

    #[test]
    fn a_share_is_the_full_count_at_the_reference_and_never_zero() {
        assert_eq!(share_of(200, REFERENCE), 200);
        assert_eq!(share_of(200, 4), 20);
        assert_eq!(share_of(40, 160), 160);
        assert_eq!(share_of(3, 1), 1);
    }

    #[test]
    fn results_come_back_in_seed_order_whatever_the_schedule() {
        // Later seeds finish first: the order is still the seeds'.
        let r = run(0..32, |s| {
            std::thread::sleep(std::time::Duration::from_millis(32 - s));
            s * 10
        });
        assert_eq!(r, (0..32).map(|s| (s, s * 10)).collect::<Vec<_>>());
    }

    #[test]
    fn the_seeds_really_run_side_by_side() {
        // THE CONTROL for "parallel": 8 seeds that each wait for all 8 to have started. On one worker this never
        // finishes; the timeout is the failure.
        if std::thread::available_parallelism().map_or(1, |n| n.get()) < 8 {
            return;
        }
        let started = AtomicUsize::new(0);
        let r = run(0..8, |_| {
            started.fetch_add(1, Ordering::SeqCst);
            let by = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while started.load(Ordering::SeqCst) < 8 {
                assert!(
                    std::time::Instant::now() < by,
                    "the seeds ran one after another"
                );
                std::thread::yield_now();
            }
        });
        assert_eq!(r.len(), 8);
    }

    #[test]
    fn a_panicking_seed_is_named_with_how_to_run_it_alone() {
        let e =
            catch_unwind(|| run(0..10, |s| assert!(s != 7 && s != 3, "boom at {s}"))).unwrap_err();
        let msg = e.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(
            msg.contains("seed 3 panicked") && msg.contains("CRAFTWORKS_MODEL_SEED=3"),
            "{msg}"
        );
    }

    #[test]
    fn a_seed_run_alone_is_that_seed_and_nothing_else() {
        let ran = AtomicUsize::new(0);
        let r = run_alone(0..40, Some(17), |s| {
            ran.fetch_add(1, Ordering::SeqCst);
            s
        });
        assert_eq!(r, vec![(17, 17)]);
        assert_eq!(
            ran.load(Ordering::SeqCst),
            1,
            "other seeds ran beside the one asked for"
        );
        assert!(
            run_alone(0..10, Some(17), |s| s).is_empty(),
            "a seed outside the sweep ran"
        );
    }

    #[test]
    fn until_stops_at_the_floor_and_never_passes_the_cap() {
        let r = until(1, 4, 40, |s| s, |all| all.iter().any(|(s, _)| *s >= 10));
        assert_eq!(r.len(), 12, "batches of 4 from 1: 1..5, 5..9, 9..13");
        let r = until(1, 4, 10, |s| s, |_| false);
        assert_eq!(r.len(), 10);
    }
}
