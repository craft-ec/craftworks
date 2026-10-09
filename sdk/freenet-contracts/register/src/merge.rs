//! The merge: a semilattice over DECISIONS, and fork detection layered on top.
//!
//! [`join`] is `(max of records, min of evidence)` — commutative, associative
//! and idempotent, with no tie-break that looks at which side is "held".
//! Detection is deliberately NOT part of it: [`detect`] originates evidence when
//! two records equivocate, and [`update`] is `join(join(a, b), detect(a, b))`.
//! That split bounds the effect of delivery order: what an unlucky order can
//! change is only WHETHER a fork was noticed, never what two synced replicas
//! decided.
//!
//! **The lattice is over decisions, not over bytes.** A decision is
//! `(terminal, seq, BLAKE3(value))`; the signatures that witness it are not part
//! of its identity, and no comparison here ever reads one. The laws therefore
//! hold on decisions: two replicas that have synced hold the same decision and
//! the same summary, and may hold different witnesses for it — a difference
//! nothing needs to reconcile, because either witness proves the same fact.
//!
//! The reason is not tidiness. An Ed25519 signature is not unique to its signer:
//! whoever holds a key can mint unlimited distinct valid signatures of one
//! message by picking another nonce. An order that broke ties on the encoding
//! would hand a single member of a keyset an endless supply of strictly
//! "better" states carrying the decision everybody already holds — each one
//! replacing the state on every host and waking every subscriber, with no
//! equivocation and nothing attributable to anyone. So when two candidates
//! carry the same decision, the held bytes win and the state does not change.

use crate::wire::{conflicts, Authority, Evidence, Record, RegState};
use core::cmp::Ordering;

/// Order two records by what they decide. Never reads a signature, and returns
/// `Equal` for two witnesses of one decision.
pub fn order(x: &Record, y: &Record) -> Ordering {
    x.decision().rank().cmp(&y.decision().rank())
}

/// The greater decision; on equal decisions the FIRST argument, which callers
/// pass as the held state. Keeping the held bytes is what makes a re-signed
/// duplicate a no-op instead of an update.
fn max_record(x: Option<Record>, y: Option<Record>) -> Option<Record> {
    match (x, y) {
        (Some(x), Some(y)) => Some(match order(&x, &y) {
            Ordering::Less => y,
            _ => x,
        }),
        (x, y) => x.or(y),
    }
}

/// Evidence is sticky, and of two proofs the lower pair of decisions wins — on
/// an equal pair the held witness stays, for the same reason records do.
fn min_evidence(x: Option<Evidence>, y: Option<Evidence>) -> Option<Evidence> {
    match (x, y) {
        (Some(x), Some(y)) => Some(if y.key() < x.key() { y } else { x }),
        (x, y) => x.or(y),
    }
}

/// `J((r1,e1),(r2,e2)) = (max(r1,r2), min(e1,e2))`, over decisions.
pub fn join(x: &RegState, y: &RegState, _a: &Authority) -> RegState {
    RegState {
        record: max_record(x.record.clone(), y.record.clone()),
        evidence: min_evidence(x.evidence.clone(), y.evidence.clone()),
    }
}

/// Evidence originated from two states whose records equivocate. Nothing else
/// about the states matters, and this is the only place evidence is created.
pub fn detect(x: &RegState, y: &RegState, a: &Authority) -> RegState {
    let evidence = match (&x.record, &y.record) {
        (Some(r), Some(s)) if conflicts(&r.signed, &s.signed) => {
            Evidence::new(&r.signed, &s.signed, a)
        }
        _ => None,
    };
    RegState {
        record: None,
        evidence,
    }
}

/// The contract's merge: join, then fold in anything the pair itself proves.
pub fn update(x: &RegState, y: &RegState, a: &Authority) -> RegState {
    join(&join(x, y, a), &detect(x, y, a), a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;
    use crate::wire::Signed;

    /// The order this contract deliberately does NOT use: it breaks ties on the
    /// record's ENCODING. Kept as the control for the churn test — the attack it
    /// enables has to be shown working against something.
    pub fn record_hash(r: &Record, a: &Authority) -> [u8; 32] {
        *blake3::hash(&r.encode(a)).as_bytes()
    }

    fn order_by_encoding(x: &Record, y: &Record, a: &Authority) -> Ordering {
        order(x, y).then_with(|| record_hash(y, a).cmp(&record_hash(x, a)))
    }

    /// The old merge, for the control only: prefers the lower record hash.
    pub fn update_by_encoding(x: &RegState, y: &RegState, a: &Authority) -> RegState {
        let mut out = update(x, y, a);
        if let (Some(xr), Some(yr)) = (&x.record, &y.record) {
            if order_by_encoding(xr, yr, a) == Ordering::Less {
                out.record = Some(yr.clone());
            }
        }
        out
    }

    /// The laws hold on decisions. `update` is not asserted associative — it is
    /// not, by design; the laws belong to the join.
    #[test]
    fn join_is_a_semilattice_over_decisions() {
        for w in [world(), keyset(1, 1, true)] {
            check_laws(&w);
        }
    }

    fn check_laws(w: &World) {
        let states = w.sample_states();
        assert!(states.len() >= 12, "{} states", states.len());
        // The fixture must contain two different values at one seq, or dropping
        // the value-hash step from the order would not break commutativity and
        // these laws would not be pinning it.
        let mut by_seq: Vec<(u64, [u8; 32])> = states
            .iter()
            .filter_map(|s| s.record.as_ref())
            .map(|r| (r.signed.seq, r.signed.value_hash))
            .collect();
        by_seq.sort();
        by_seq.dedup();
        assert!(
            by_seq.windows(2).any(|p| p[0].0 == p[1].0),
            "no two values at one seq: the laws cannot detect a broken tie-break"
        );
        for x in &states {
            assert_eq!(decisions(&join(x, x, &w.auth)), decisions(x), "idempotent");
            for y in &states {
                assert_eq!(
                    decisions(&join(x, y, &w.auth)),
                    decisions(&join(y, x, &w.auth)),
                    "commutative:\n{x:?}\n{y:?}"
                );
                for z in &states {
                    assert_eq!(
                        decisions(&join(&join(x, y, &w.auth), z, &w.auth)),
                        decisions(&join(x, &join(y, z, &w.auth), &w.auth)),
                        "associative"
                    );
                }
            }
        }
    }

    /// Every replica that has synced decided the same thing and reports the same
    /// summary, whatever order the records reached it in — and if any replica
    /// noticed the fork, all of them hold the same proof of it.
    #[test]
    fn replicas_converge_in_every_order() {
        let w = world();
        for seed in 0..40u64 {
            for with_terminal in [false, true] {
                let (ends, _) = w.gossip(seed, with_terminal, update);
                let first = decisions(&ends[0]);
                for e in &ends {
                    assert_eq!(
                        decisions(e),
                        first,
                        "replicas disagree (seed {seed}, terminal {with_terminal})"
                    );
                    assert_eq!(
                        crate::summary_of(e),
                        crate::summary_of(&ends[0]),
                        "same decision must mean the same summary"
                    );
                }
                let forked = ends[0].forked();
                assert!(ends.iter().all(|e| e.forked() == forked));
            }
        }
    }

    /// Negative control: a merge that keeps the HELD record when the order
    /// cannot separate two DECISIONS — the shape a "first writer wins"
    /// implementation naturally has — must fail to converge, or the convergence
    /// test above proves nothing.
    #[test]
    fn a_merge_that_prefers_the_held_decision_fails_to_converge() {
        let w = world();
        let biased = |x: &RegState, y: &RegState, a: &Authority| -> RegState {
            let mut out = update(x, y, a);
            if let (Some(xr), Some(yr)) = (&x.record, &y.record) {
                if (xr.signed.terminal, xr.signed.seq) == (yr.signed.terminal, yr.signed.seq) {
                    out.record = Some(xr.clone());
                }
            }
            out
        };
        // No terminal record: a terminal beats everything on the first field and
        // would settle every run before a tie-break was reached.
        let diverged = (0..40u64).any(|seed| {
            let (ends, _) = w.gossip(seed, false, biased);
            let first = decisions(&ends[0]);
            ends.iter().any(|e| decisions(e) != first)
        });
        assert!(
            diverged,
            "the biased merge converged, so the convergence test cannot detect bias"
        );
    }

    /// The attack the decision lattice exists to stop. An Ed25519 signature is
    /// not unique to its signer, so one member of a keyset can mint endless
    /// valid re-signatures of the record everyone already holds. Ordered by
    /// encoding, each one is strictly "better" and replaces the state on every
    /// host; ordered by decision, none of them changes anything.
    #[test]
    fn a_stream_of_re_signatures_of_the_held_decision_changes_nothing() {
        let w = world();
        let held = w.state(w.record(false, 4, b"the value"));
        let before = w.encode(&held);
        // Descending record hash, so under the old order each one wins in turn.
        let mut stream: Vec<Record> = (0..1000)
            .map(|n| w.resigned(false, 4, b"the value", n))
            .collect();
        stream.sort_by_key(|r| core::cmp::Reverse(record_hash(r, &w.auth)));
        assert_eq!(
            stream
                .iter()
                .map(|r| record_hash(r, &w.auth))
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            1000,
            "the re-signatures must really be distinct encodings"
        );

        let mut state = held.clone();
        for r in &stream {
            state = update(&state, &w.state(r.clone()), &w.auth);
            assert_eq!(w.encode(&state), before, "a re-signature changed the state");
        }
        assert!(!state.forked(), "re-signing is not equivocation");

        // Control: the same stream against the order this contract rejected.
        // It starts from the highest-hashing re-signature so that every later
        // one is strictly lower and must win — otherwise only the part of the
        // stream that happens to hash below the original would churn, and the
        // count would say nothing.
        let mut churned = w.state(stream[0].clone());
        let mut changes = 0;
        for r in &stream[1..] {
            let next = update_by_encoding(&churned, &w.state(r.clone()), &w.auth);
            if w.encode(&next) != w.encode(&churned) {
                changes += 1;
            }
            churned = next;
        }
        assert_eq!(
            changes,
            stream.len() - 1,
            "the control must churn on every one, or this test proves nothing"
        );
    }

    /// Which VALUE wins a fork is fixed once both values exist, whoever signed
    /// either of them. Under an encoding-ordered merge it would flip every time
    /// a new signer subset published the losing value.
    #[test]
    fn which_value_wins_a_fork_does_not_depend_on_who_signed_it() {
        let w = world();
        let (lo, hi) = {
            let (a, b) = (b"value-alpha".to_vec(), b"value-beta".to_vec());
            if blake3::hash(&a).as_bytes() <= blake3::hash(&b).as_bytes() {
                (a, b)
            } else {
                (b, a)
            }
        };
        let winners = w.all_encodings(false, 5, &lo);
        let losers = w.all_encodings(false, 5, &hi);
        assert_eq!(
            (winners.len(), losers.len()),
            (6, 6),
            "2 of 4 has six quorums"
        );
        // Only meaningful where the two orders disagree for some pair.
        assert!(
            losers.iter().any(|l| winners
                .iter()
                .any(|win| record_hash(l, &w.auth) < record_hash(win, &w.auth))),
            "no pair of subsets separates the two orders, so this proves nothing"
        );
        for win in &winners {
            for lose in &losers {
                let m = update(&w.state(win.clone()), &w.state(lose.clone()), &w.auth);
                assert!(m.forked(), "same seq, different values");
                assert_eq!(m.record.as_ref().unwrap().value, lo);
            }
        }
        let mut held = w.state(winners[0].clone());
        for lose in &losers {
            held = update(&held, &w.state(lose.clone()), &w.auth);
            assert_eq!(held.record.as_ref().unwrap().value, lo);
        }
    }

    /// Evidence is ordered by the decisions it proves, numerically in `seq` — a
    /// fork at seq 1 beats one at seq 256, which comparing little-endian bytes
    /// would get backwards.
    #[test]
    fn evidence_is_ordered_by_decision_and_seq_is_numeric() {
        let w = world();
        let fork_at =
            |seq: u64| w.evidence_of(&w.record(false, seq, b"one"), &w.record(false, seq, b"two"));
        let (low, high) = (fork_at(1), fork_at(256));
        assert!(low.key() < high.key(), "seq 1 must sort below seq 256");
        let a = RegState {
            record: None,
            evidence: Some(low.clone()),
        };
        let b = RegState {
            record: None,
            evidence: Some(high),
        };
        for (x, y) in [(&a, &b), (&b, &a)] {
            let m = update(x, y, &w.auth);
            assert_eq!(m.evidence.unwrap().decisions(), low.decisions());
        }
        // A terminal pair outranks any seq pair.
        let terminals = w.evidence_of(
            &w.record(true, 9, &[1u8; 32]),
            &w.record(true, 9, &[2u8; 32]),
        );
        assert!(terminals.key() < low.key(), "a terminal pair sorts first");
    }

    #[test]
    fn a_terminal_record_can_never_be_displaced() {
        let w = world();
        let terminal = w.state(w.record(true, 0, &[7u8; 32]));
        let loud = w.state(w.record(false, u64::MAX, b"later"));
        let merged = update(&terminal, &loud, &w.auth);
        assert_eq!(
            decisions(&merged).0,
            decisions(&terminal).0,
            "seq must not beat terminal"
        );
        assert!(!merged.forked(), "a higher seq is not an equivocation");
        let other = w.state(w.record(true, u64::MAX, &[9u8; 32]));
        let ab = update(&terminal, &other, &w.auth);
        let ba = update(&other, &terminal, &w.auth);
        assert!(ab.forked() && ba.forked());
        assert_eq!(decisions(&ab), decisions(&ba));
    }

    /// The same decision signed by two different quorums is one decision spelled
    /// two ways: the held bytes stay and nothing is a fork.
    #[test]
    fn two_signer_subsets_of_one_decision_are_not_a_fork() {
        let w = world();
        let a = w.record(false, 3, b"same");
        let b = w
            .alternate(false, 3, b"same")
            .expect("2-of-4 has a spare key");
        assert_ne!(a.encode(&w.auth), b.encode(&w.auth), "different encodings");
        let held = w.state(a);
        let m = update(&held, &w.state(b), &w.auth);
        assert!(!m.forked(), "same value, so no equivocation");
        assert_eq!(w.encode(&m), w.encode(&held), "the held witness stays");
    }

    /// Detection is order-sensitive and the join is not.
    #[test]
    fn only_whether_a_fork_is_caught_depends_on_order() {
        let w = world();
        let (a, b) = (w.record(false, 7, b"one"), w.record(false, 7, b"two"));
        let (x, y) = (w.state(a), w.state(b));
        assert!(update(&x, &y, &w.auth).forked());
        let empty = RegState::default();
        let stepwise = update(&update(&empty, &x, &w.auth), &y, &w.auth);
        assert!(stepwise.forked());
        assert_eq!(decisions(&stepwise), decisions(&update(&x, &y, &w.auth)));
    }

    /// Signatures are never consulted by the order.
    #[test]
    fn the_order_reads_no_signature() {
        let w = world();
        let r = w.record(false, 2, b"v");
        let mut forged = r.clone();
        forged.signed.sigs = vec![[0u8; 64]; w.k];
        assert_eq!(order(&r, &forged), Ordering::Equal);
        let _ = Signed::decision(&r.signed);
    }
}
