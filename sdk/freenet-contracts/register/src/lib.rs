//! Register contract: one signed value per writer (ARCHITECTURE.md §1).
//!
//! Params = `"RG01" ‖ mode(1) ‖ authority ‖ label`, so the contract key commits
//! to who may write it and a host needs nothing else to check a signature.
//! State  = `"RG01" ‖ flags(1) ‖ record? ‖ evidence?`, or the empty byte string
//!          for a register that exists but has not been written yet.
//! Valid  = every signature in it verifies under the params, and every encoding
//!          in it is the canonical one.
//! Merge  = `J((r1,e1),(r2,e2)) = (max r, min e)`, a semilattice; fork detection
//!          is layered on top (`crate::merge`), never folded into the join.
//!
//! **Signatures are bound to the params, not to the contract code.** After a code
//! upgrade anyone can re-publish the latest record under the new code and it
//! still verifies; max-merge makes republished older records harmless. That is
//! intended.
//!
//! **The contract is neutral about consequences.** It proves a fork happened and
//! keeps the proof; what to do about a forked register is the reader's policy.

// The CONTRACT (`freenet-main-contract`, the default) and only it uses freenet-stdlib: with no default features this
// crate is a plain library of read/parse/verify, with no client stack (closure-gate.sh).
#[cfg(feature = "freenet-main-contract")]
use freenet_stdlib::prelude::*;

pub mod merge;
/// Naming a successor. Native-only: no host calls it, and register.wasm
/// must not move for a helper.
#[cfg(not(target_arch = "wasm32"))]
pub mod succ;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod wire;

// Used by the contract's merge path (and its tests and cost measures), not by the library's read/parse/verify.
#[cfg(any(feature = "freenet-main-contract", test, feature = "testing"))]
use merge::update;
#[cfg(any(feature = "freenet-main-contract", test, feature = "testing"))]
use wire::Evidence;
use wire::{Params, RegState};

/// Largest state: a full value, a full 16-of-16 record, and evidence of the same
/// size minus the values. Generous — the encodings are checked exactly.
pub const MAX_STATE: usize = 64 * 1024;

/// Parse and fully verify a state against its params. This is what
/// `validate_state` does: a state offered to a host is checked in full.
pub fn read(params: &[u8], state: &[u8]) -> Option<(Params, RegState)> {
    let (p, s) = read_unverified(params, state)?;
    s.verify(&p).then_some((p, s))
}

/// Structure only, for the state a host ALREADY HOLDS.
///
/// Its signatures were checked by `validate_state` before it was ever stored,
/// and re-checking them on every update, summary and delta would cost `k`
/// verifications each time for an answer that cannot have changed — the same
/// waste that makes a replayed record expensive. Candidates arriving from the
/// network are a different matter and are verified in `absorb`.
pub fn read_unverified(params: &[u8], state: &[u8]) -> Option<(Params, RegState)> {
    if state.len() > MAX_STATE {
        return None;
    }
    let p = Params::parse(params)?;
    let s = RegState::parse_unverified(state, &p)?;
    Some((p, s))
}

/// `terminal ‖ seq ‖ BLAKE3(value) ‖ BLAKE3(the evidence's pair of decisions)`,
/// with the hash of the empty string for a part that is absent.
///
/// Built from DECISIONS, never from encodings. Two replicas can legitimately
/// hold different witnesses of one decision; if the summary read the witness
/// they would see each other as out of date forever and re-send the state on
/// every exchange, for a difference neither of them needs to resolve.
#[cfg(any(feature = "freenet-main-contract", test))]
pub(crate) fn summary_of(s: &RegState) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 8 + 32 + 32);
    let d = s.record.as_ref().map(|r| r.decision());
    out.push(u8::from(d.is_some_and(|d| d.terminal)));
    out.extend_from_slice(&d.map_or(0, |d| d.seq).to_le_bytes());
    out.extend_from_slice(&d.map_or([0u8; 32], |d| d.value_hash));
    let pair = s.evidence.as_ref().map(|e| {
        let (x, y) = e.decisions();
        [x.encode(), y.encode()].concat()
    });
    out.extend_from_slice(blake3::hash(&pair.unwrap_or_default()).as_bytes());
    out
}

/// Merge a parsed-but-unverified candidate into `held`, verifying only the
/// parts that could change anything.
///
/// Anyone can send anything, `k` can be 16, and the cheapest attack on a host
/// is to replay records it already has. So the decision is compared first and
/// signatures are checked only for a candidate that could win — a greater
/// decision, one that conflicts with the held record (it becomes evidence), or
/// a lower evidence pair. A stale record, or another witness of the decision
/// already held, costs a parse.
///
/// Skipping those checks is safe precisely because they could not matter: a
/// candidate that loses the decision order is discarded whether its signatures
/// hold or not.
#[cfg(any(feature = "freenet-main-contract", test, feature = "testing"))]
fn absorb(held: &RegState, cand: RegState, p: &Params) -> RegState {
    let mut out = held.clone();
    if let Some(r) = cand.record {
        let useful = match &held.record {
            None => true,
            Some(h) => {
                if merge::order(&r, h) == core::cmp::Ordering::Greater {
                    true
                } else {
                    // It cannot win the record slot, so it can only matter as
                    // evidence — and only if that evidence beats what is held.
                    // The pair's key comes from the two DECISIONS, so this is
                    // decided before any signature is checked: replaying the
                    // losing record of a fork already proved costs nothing,
                    // however often it arrives.
                    match Evidence::new(&r.signed, &h.signed, &p.authority) {
                        None => false,
                        Some(e) => held
                            .evidence
                            .as_ref()
                            .is_none_or(|held_e| e.key() < held_e.key()),
                    }
                }
            }
        };
        if useful && r.verify(p) {
            let c = RegState {
                record: Some(r),
                evidence: None,
            };
            out = update(&out, &c, &p.authority);
        }
    }
    if let Some(e) = cand.evidence {
        let useful = held.evidence.as_ref().is_none_or(|h| e.key() < h.key());
        if useful && e.verify(p) {
            out = update(
                &out,
                &RegState {
                    record: None,
                    evidence: Some(e),
                },
                &p.authority,
            );
        }
    }
    out
}

/// The two update paths, side by side, so the saving from verifying late can be
/// MEASURED rather than asserted. Both mirror `update_state` for a single
/// candidate; only the verification policy differs.
#[cfg(any(test, feature = "testing"))]
pub mod cost {
    use super::*;

    /// What the contract does now: parse, compare decisions, verify only a
    /// candidate that could win.
    pub fn verify_late(params: &[u8], held: &[u8], cand: &[u8]) -> Option<Vec<u8>> {
        let (p, h) = read_unverified(params, held)?;
        let c = RegState::parse_unverified(cand, &p)?;
        Some(absorb(&h, c, &p).encode(&p.authority))
    }

    /// What it did before: verify the held state and every candidate in full,
    /// then merge.
    pub fn verify_eagerly(params: &[u8], held: &[u8], cand: &[u8]) -> Option<Vec<u8>> {
        let (p, h) = read(params, held)?;
        let c = RegState::parse(cand, &p)?;
        Some(update(&h, &c, &p.authority).encode(&p.authority))
    }
}

#[cfg(feature = "freenet-main-contract")]
pub struct Register;

#[cfg(feature = "freenet-main-contract")]
#[contract]
impl ContractInterface for Register {
    fn validate_state(
        parameters: Parameters<'static>,
        state: State<'static>,
        _related: RelatedContracts<'static>,
    ) -> Result<ValidateResult, ContractError> {
        Ok(match read(parameters.as_ref(), state.as_ref()) {
            Some(_) => ValidateResult::Valid,
            None => ValidateResult::Invalid,
        })
    }

    fn update_state(
        parameters: Parameters<'static>,
        state: State<'static>,
        data: Vec<UpdateData<'static>>,
    ) -> Result<UpdateModification<'static>, ContractError> {
        let Some((p, mut held)) = read_unverified(parameters.as_ref(), state.as_ref()) else {
            // The held state is not ours to fix; refusing is the only honest
            // answer, and it cannot happen for a state this contract produced.
            return Err(ContractError::InvalidState);
        };
        for item in data {
            let candidates: [Option<&[u8]>; 2] = match &item {
                UpdateData::State(s) => [Some(s.as_ref()), None],
                UpdateData::Delta(d) => [Some(d.as_ref()), None],
                UpdateData::StateAndDelta { state, delta } => {
                    [Some(state.as_ref()), Some(delta.as_ref())]
                }
                _ => [None, None],
            };
            for bytes in candidates.into_iter().flatten() {
                // An unreadable candidate is ignored, never fatal: one bad
                // sender must not stop a good one in the same batch.
                if let Some(c) = RegState::parse_unverified(bytes, &p) {
                    held = absorb(&held, c, &p);
                }
            }
        }
        Ok(UpdateModification::valid(State::from(
            held.encode(&p.authority),
        )))
    }

    fn summarize_state(
        parameters: Parameters<'static>,
        state: State<'static>,
    ) -> Result<StateSummary<'static>, ContractError> {
        let Some((_, s)) = read_unverified(parameters.as_ref(), state.as_ref()) else {
            return Err(ContractError::InvalidState);
        };
        Ok(StateSummary::from(summary_of(&s)))
    }

    fn get_state_delta(
        parameters: Parameters<'static>,
        state: State<'static>,
        summary: StateSummary<'static>,
    ) -> Result<StateDelta<'static>, ContractError> {
        let Some((p, s)) = read_unverified(parameters.as_ref(), state.as_ref()) else {
            return Err(ContractError::InvalidState);
        };
        // Registers are small and the merge is a join, so the whole state is the
        // delta: sending it twice costs bytes, never correctness.
        if summary.as_ref() == summary_of(&s) {
            return Ok(StateDelta::from(Vec::new()));
        }
        Ok(StateDelta::from(s.encode(&p.authority)))
    }
}

#[cfg(all(test, feature = "freenet-main-contract"))]
mod tests {
    use super::*;
    use crate::testing::*;
    use crate::wire::{Evidence, Record, MAX_VALUE};

    fn validate(p: &[u8], s: &[u8]) -> ValidateResult {
        Register::validate_state(
            Parameters::from(p.to_vec()),
            State::from(s.to_vec()),
            Default::default(),
        )
        .unwrap()
    }
    fn valid(p: &[u8], s: &[u8]) -> bool {
        validate(p, s) == ValidateResult::Valid
    }
    fn update_with(p: &[u8], held: &[u8], data: Vec<Vec<u8>>) -> Vec<u8> {
        let data = data
            .into_iter()
            .map(|d| UpdateData::State(State::from(d)))
            .collect();
        Register::update_state(
            Parameters::from(p.to_vec()),
            State::from(held.to_vec()),
            data,
        )
        .unwrap()
        .new_state
        .map(|s| s.as_ref().to_vec())
        .unwrap_or_default()
    }

    /// Every refusal below is paired with this: the unmodified record IS
    /// accepted, so a refusal can never come from the setup being broken.
    fn control_is_accepted(w: &World, r: &Record) {
        assert!(
            valid(&w.params_bytes, &w.encode(&w.state(r.clone()))),
            "the control record must be accepted"
        );
    }

    #[test]
    fn an_empty_register_is_valid_and_a_first_write_is_adopted() {
        let w = world();
        assert!(valid(&w.params_bytes, &[]));
        let s = w.encode(&w.state(w.record(false, 1, b"hello")));
        assert_eq!(update_with(&w.params_bytes, &[], vec![s.clone()]), s);
    }

    #[test]
    fn mode_0_single_key_works_end_to_end() {
        let w = keyset(1, 1, true);
        let r = w.record(false, 5, b"solo");
        control_is_accepted(&w, &r);
        let s = w.encode(&w.state(r));
        assert_eq!(update_with(&w.params_bytes, &[], vec![s.clone()]), s);
    }

    /// Every keyset size the format allows, end to end. `n = 16` is the one that
    /// matters: the signer bitmap is a `u16`, so the mask for "a bit outside the
    /// keyset" is exactly the width of the word there, and an implementation
    /// that builds it by shifting up refuses every full keyset. A suite that
    /// only ever tests a 4-key register cannot see it.
    #[test]
    fn every_keyset_size_from_1_of_1_to_16_of_16_works() {
        for n in 1..=wire::MAX_N {
            for k in [1, n] {
                let w = keyset(k, n, false);
                let r = w.record(false, 3, b"v");
                let s = w.encode(&w.state(r));
                assert!(valid(&w.params_bytes, &s), "k = {k} of n = {n}");
                assert_eq!(update_with(&w.params_bytes, &[], vec![s.clone()]), s);
                // And one signature short is still refused at every size.
                if k > 1 {
                    let who: Vec<usize> = (0..k - 1).collect();
                    let under = w.record_signed_by(false, 3, b"v", &who);
                    assert!(
                        !valid(&w.params_bytes, &w.encode(&w.state(under))),
                        "k - 1 accepted at k = {k} of n = {n}"
                    );
                }
            }
        }
    }

    #[test]
    fn params_are_refused_unless_canonical() {
        let w = keyset(2, 4, false);
        let keys: Vec<Vec<u8>> = w
            .signers
            .iter()
            .map(|s| s.verifying_key().to_bytes().to_vec())
            .collect();
        let build = |k: u8, n: u8, ks: &[Vec<u8>], label: &[u8]| {
            let mut p = Vec::from(*wire::MAGIC);
            p.push(1);
            p.push(k);
            p.push(n);
            for key in ks {
                p.extend_from_slice(key);
            }
            p.extend_from_slice(label);
            p
        };
        // Control.
        assert!(Params::parse(&build(2, 4, &keys, b"head")).is_some());
        let unsorted = vec![
            keys[1].clone(),
            keys[0].clone(),
            keys[2].clone(),
            keys[3].clone(),
        ];
        let dup = vec![
            keys[0].clone(),
            keys[0].clone(),
            keys[2].clone(),
            keys[3].clone(),
        ];
        let small_order = vec![
            vec![0u8; 32],
            keys[1].clone(),
            keys[2].clone(),
            keys[3].clone(),
        ];
        // y = p: a second spelling of a point that already has a canonical one.
        let mut noncanonical = vec![0xffu8; 32];
        noncanonical[0] = 0xed;
        noncanonical[31] = 0x7f;
        let nc = vec![
            noncanonical,
            keys[1].clone(),
            keys[2].clone(),
            keys[3].clone(),
        ];
        for (what, p) in [
            ("k = 0", build(0, 4, &keys, b"head")),
            ("k > n", build(5, 4, &keys, b"head")),
            ("n = 0", build(0, 0, &[], b"head")),
            ("n > 16", build(1, 17, &keys, b"head")),
            ("unsorted keys", build(2, 4, &unsorted, b"head")),
            ("duplicate key", build(2, 4, &dup, b"head")),
            ("small-order key", build(2, 4, &small_order, b"head")),
            ("non-canonical key", build(2, 4, &nc, b"head")),
            ("label too long", build(2, 4, &keys, &[b'x'; 65])),
            ("trailing bytes", {
                let mut p = build(2, 4, &keys, b"head");
                p.extend_from_slice(&[0u8; 80]); // pushes the label over its cap
                p
            }),
            ("unknown mode", {
                let mut p = Vec::from(*wire::MAGIC);
                p.push(2);
                p.extend_from_slice(&keys[0]);
                p
            }),
            ("no magic", b"XX01".to_vec()),
        ] {
            assert!(Params::parse(&p).is_none(), "{what} must be refused");
            // And nothing is valid under params a host cannot read.
            assert!(!valid(&p, &[]), "{what}: no state is valid under it");
        }
    }

    #[test]
    fn a_record_is_refused_unless_every_signature_holds() {
        let w = world();
        let good = w.record(false, 4, b"value");
        control_is_accepted(&w, &good);

        let mut unsigned = good.clone();
        unsigned.signed.sigs = vec![[0u8; 64]; w.k];

        // A genuinely different keyset, not the same keys rebuilt.
        let other = keyset_seeded(1, 2, 4, false);
        assert_ne!(
            other.params_bytes, w.params_bytes,
            "the fixture must differ"
        );
        let mut wrong_key = good.clone();
        wrong_key.signed.sigs = other.record(false, 4, b"value").signed.sigs;

        // A signature over a different register: same keys, other label.
        let mut elsewhere_params = w.params_bytes.clone();
        elsewhere_params.truncate(elsewhere_params.len() - 4);
        elsewhere_params.extend_from_slice(b"tail");
        let elsewhere = Params::parse(&elsewhere_params).unwrap();
        let mut other_label = good.clone();
        {
            let msg = elsewhere.signed_message(false, 4, &good.signed.value_hash);
            use ed25519_dalek::Signer;
            other_label.signed.sigs = (0..w.k)
                .map(|i| w.signers[i].sign(&msg).to_bytes())
                .collect();
        }

        // S + L: the same signature, malleated. Only rejected because
        // verify_strict checks S is fully reduced.
        let mut malleated = good.clone();
        malleated.signed.sigs[0] = add_group_order(&good.signed.sigs[0]);

        // Swapped order: each signature is checked against its own signer.
        let mut swapped = good.clone();
        swapped.signed.sigs.swap(0, 1);

        for (what, r) in [
            ("unsigned", unsigned),
            ("wrong key", wrong_key),
            ("signed for another label", other_label),
            ("malleated S + L", malleated),
            ("signatures out of key order", swapped),
        ] {
            let s = w.encode(&w.state(r));
            assert!(!valid(&w.params_bytes, &s), "{what} must be refused");
        }
        control_is_accepted(&w, &good);
    }

    #[test]
    fn the_signature_count_must_be_exactly_k() {
        let w = world();
        let good = w.record(false, 4, b"value");
        control_is_accepted(&w, &good);

        let under = w.record_signed_by(false, 4, b"value", &[0]); // k - 1
        let over = w.record_signed_by(false, 4, b"value", &[0, 1, 2]); // k + 1
        for (what, r) in [("k - 1 signatures", under), ("k + 1 signatures", over)] {
            let s = w.encode(&w.state(r));
            assert!(!valid(&w.params_bytes, &s), "{what} must be refused");
        }

        // One signer counted twice: the bitmap is a set, so the only way to
        // express it is to flag two keys and send one signer's signature twice.
        let mut twice = good.clone();
        twice.signed.sigs[1] = twice.signed.sigs[0];
        assert!(!valid(&w.params_bytes, &w.encode(&w.state(twice))));

        // Bitmap and signature count disagreeing, both directions.
        let mut wide = good.clone();
        wide.signed.bitmap |= 1 << 2;
        assert!(!valid(&w.params_bytes, &w.encode(&w.state(wide))));
        let mut outside = good.clone();
        outside.signed.bitmap = 0b1_0001; // a bit for a key that does not exist
        assert!(!valid(&w.params_bytes, &w.encode(&w.state(outside))));
    }

    #[test]
    fn a_lower_seq_never_displaces_and_a_value_over_the_cap_is_refused() {
        let w = world();
        let high = w.encode(&w.state(w.record(false, 9, b"current")));
        let low = w.encode(&w.state(w.record(false, 2, b"stale")));
        assert_eq!(update_with(&w.params_bytes, &high, vec![low]), high);

        // 4 KiB exactly is fine; one more byte is not.
        let at_cap = w.record(false, 1, &vec![7u8; MAX_VALUE]);
        control_is_accepted(&w, &at_cap);
        let over = w.record(false, 1, &vec![7u8; MAX_VALUE + 1]);
        assert!(!valid(&w.params_bytes, &w.encode(&w.state(over))));
        // And a value past the length FIELD's own range, where a narrowing cast
        // would write a small length in front of a long value and a reader
        // would accept the truncation as a short value.
        let huge = w.record(false, 1, &vec![7u8; u16::MAX as usize + 2]);
        let bytes = w.encode(&w.state(huge));
        assert!(
            !valid(&w.params_bytes, &bytes),
            "an over-long value is refused"
        );
        assert!(
            bytes.len() > u16::MAX as usize,
            "the full value is still written, so nothing was silently dropped"
        );
    }

    #[test]
    fn a_state_is_refused_unless_its_encoding_is_canonical() {
        let w = world();
        let s = w.encode(&w.state(w.record(false, 4, b"value")));
        assert!(valid(&w.params_bytes, &s));
        let mut empty_spelled_long = Vec::from(*wire::MAGIC);
        empty_spelled_long.push(0); // a second spelling of the empty register
        for (what, bad) in [
            ("no magic", b"ZZ01\x01".to_vec()),
            ("empty spelled as flags = 0", empty_spelled_long),
            ("unknown flag bit", {
                let mut b = s.clone();
                b[4] |= 0b100;
                b
            }),
            ("trailing byte", {
                let mut b = s.clone();
                b.push(0);
                b
            }),
            ("truncated", s[..s.len() - 1].to_vec()),
            ("evidence flag with no evidence", {
                let mut b = s.clone();
                b[4] |= 0b10;
                b
            }),
        ] {
            assert!(!valid(&w.params_bytes, &bad), "{what} must be refused");
        }
    }

    #[test]
    fn garbage_and_empty_candidates_cannot_displace_a_held_state() {
        let w = world();
        let held = w.encode(&w.state(w.record(false, 6, b"held")));
        let junk = vec![
            Vec::new(),
            b"garbage".to_vec(),
            vec![0u8; 3],
            w.encode(&w.state(w.record(false, 2, b"older"))),
        ];
        assert_eq!(update_with(&w.params_bytes, &held, junk), held);
    }

    /// A bad candidate in the batch must not cost the good one its update.
    #[test]
    fn one_bad_candidate_does_not_block_a_good_one() {
        let w = world();
        let newer = w.encode(&w.state(w.record(false, 8, b"newer")));
        let got = update_with(
            &w.params_bytes,
            &[],
            vec![b"nonsense".to_vec(), newer.clone(), vec![0xff; 40]],
        );
        assert_eq!(got, newer);
    }

    #[test]
    fn equivocation_is_detected_and_the_proof_is_permanent() {
        let w = world();
        let a = w.encode(&w.state(w.record(false, 5, b"one")));
        let b = w.encode(&w.state(w.record(false, 5, b"two")));
        let forked = update_with(&w.params_bytes, &a, vec![b.clone()]);
        let (_, parsed) = read(&w.params_bytes, &forked).unwrap();
        assert!(parsed.forked());
        // Order does not change the result.
        assert_eq!(update_with(&w.params_bytes, &b, vec![a]), forked);
        // And no later update, however ordinary, can clear it.
        let later = w.encode(&w.state(w.record(false, 99, b"much later")));
        let after = update_with(&w.params_bytes, &forked, vec![later]);
        let (_, parsed) = read(&w.params_bytes, &after).unwrap();
        assert!(parsed.forked(), "evidence must be sticky");
        assert_eq!(
            parsed.evidence,
            read(&w.params_bytes, &forked).unwrap().1.evidence
        );
    }

    /// Evidence proves the fork to someone who holds neither value — that is the
    /// point of carrying hashes rather than values.
    #[test]
    fn evidence_verifies_without_the_values_and_is_accepted_from_anyone() {
        let w = world();
        let (x, y) = (w.record(false, 5, b"one"), w.record(false, 5, b"two"));
        let e = w.evidence_of(&x, &y);
        let alone = RegState {
            record: None,
            evidence: Some(e.clone()),
        };
        let bytes = w.encode(&alone);
        assert!(valid(&w.params_bytes, &bytes), "third-party evidence");
        // It carries no value anywhere in it.
        assert!(!contains(&bytes, b"one") && !contains(&bytes, b"two"));
        // A held register picks it up, and is then forked without ever having
        // seen the losing value.
        let held = w.encode(&w.state(x.clone()));
        let after = update_with(&w.params_bytes, &held, vec![bytes]);
        assert!(read(&w.params_bytes, &after).unwrap().1.forked());
    }

    #[test]
    fn evidence_that_does_not_prove_a_fork_is_refused() {
        let w = world();
        let x = w.record(false, 5, b"one");
        let same_value_other_subset = w
            .alternate(false, 5, b"one")
            .expect("2-of-4 has a spare key");
        let different_seq = w.record(false, 6, b"two");
        for (what, a, b) in [
            (
                "same value, two subsets",
                x.clone(),
                same_value_other_subset,
            ),
            ("different seq, not terminal", x.clone(), different_seq),
            ("a side against itself", x.clone(), x.clone()),
        ] {
            assert!(
                Evidence::new(&a.signed, &b.signed, &w.auth).is_none(),
                "{what} is not a fork"
            );
        }
        // Forged evidence: a real pair, one side's signatures replaced.
        let y = w.record(false, 5, b"two");
        let mut forged = w.evidence_of(&x, &y);
        forged.b.sigs = vec![[0u8; 64]; w.k];
        let s = RegState {
            record: None,
            evidence: Some(forged),
        };
        assert!(!valid(&w.params_bytes, &w.encode(&s)));
        // And evidence stored in the wrong order is a second encoding of one
        // fork, so it is refused too.
        let e = w.evidence_of(&x, &y);
        let swapped = Evidence { a: e.b, b: e.a };
        let s = RegState {
            record: None,
            evidence: Some(swapped),
        };
        assert!(!valid(&w.params_bytes, &w.encode(&s)));
    }

    #[test]
    fn sync_sends_the_state_only_to_a_peer_that_lacks_it() {
        let w = world();
        let s = w.encode(&w.state(w.record(false, 3, b"v")));
        let summary = |bytes: &[u8]| {
            Register::summarize_state(
                Parameters::from(w.params_bytes.clone()),
                State::from(bytes.to_vec()),
            )
            .unwrap()
            .as_ref()
            .to_vec()
        };
        let delta = |bytes: &[u8], sum: Vec<u8>| {
            Register::get_state_delta(
                Parameters::from(w.params_bytes.clone()),
                State::from(bytes.to_vec()),
                StateSummary::from(sum),
            )
            .unwrap()
            .as_ref()
            .to_vec()
        };
        assert!(delta(&s, summary(&s)).is_empty(), "peer is up to date");
        assert_eq!(delta(&s, summary(&[])), s, "peer has nothing");
        // A forked register and an unforked one at the same record differ.
        let other = w.encode(&w.state(w.record(false, 5, b"w")));
        let forked = update_with(
            &w.params_bytes,
            &s,
            vec![w.encode(&w.state(w.record(false, 3, b"x")))],
        );
        assert_ne!(summary(&forked), summary(&s));
        assert_ne!(summary(&other), summary(&s));
    }

    /// A second witness of the decision already held must leave the state byte
    /// for byte unchanged — for another signer subset AND for a fresh signature
    /// by the same subset. Anything less and a key holder can make every host
    /// rewrite its state and wake every subscriber, for free, forever.
    #[test]
    fn a_second_witness_of_the_held_decision_is_a_no_op() {
        let w = world();
        let held = w.encode(&w.state(w.record(false, 4, b"the value")));
        for (what, other) in [
            (
                "another signer subset",
                w.alternate(false, 4, b"the value").expect("spare key"),
            ),
            (
                "a fresh signature by the same subset",
                w.resigned(false, 4, b"the value", 7),
            ),
        ] {
            let bytes = w.encode(&w.state(other));
            assert_ne!(bytes, held, "{what}: must really be a different encoding");
            assert!(valid(&w.params_bytes, &bytes), "{what}: and a valid one");
            assert_eq!(
                update_with(&w.params_bytes, &held, vec![bytes]),
                held,
                "{what}: the state changed"
            );
        }
    }

    /// The same for evidence: a second proof of one fork does not rewrite the
    /// state either.
    #[test]
    fn a_second_witness_of_the_held_evidence_is_a_no_op() {
        let w = world();
        let (x, y) = (w.record(false, 5, b"one"), w.record(false, 5, b"two"));
        let forked = update_with(
            &w.params_bytes,
            &w.encode(&w.state(x.clone())),
            vec![w.encode(&w.state(y.clone()))],
        );
        assert!(read(&w.params_bytes, &forked).unwrap().1.forked());
        for (what, a, b) in [
            (
                "another signer subset",
                w.alternate(false, 5, b"one").expect("spare key"),
                w.alternate(false, 5, b"two").expect("spare key"),
            ),
            (
                "fresh signatures",
                w.resigned(false, 5, b"one", 11),
                w.resigned(false, 5, b"two", 12),
            ),
        ] {
            let other = RegState {
                record: None,
                evidence: Some(w.evidence_of(&a, &b)),
            };
            let bytes = w.encode(&other);
            assert!(valid(&w.params_bytes, &bytes), "{what}");
            assert_eq!(
                update_with(&w.params_bytes, &forked, vec![bytes]),
                forked,
                "{what}: the proof was swapped for an equivalent one"
            );
        }
    }

    /// Two replicas can hold different witnesses of one decision. They must
    /// report the SAME summary — if the summary read the encoding they would see
    /// each other as stale forever and re-send on every exchange.
    #[test]
    fn different_witnesses_of_one_decision_report_one_summary() {
        let w = world();
        let a = w.encode(&w.state(w.record(false, 6, b"v")));
        let b = w.encode(&w.state(w.resigned(false, 6, b"v", 3)));
        assert_ne!(a, b, "the two replicas really do hold different bytes");
        assert!(valid(&w.params_bytes, &a) && valid(&w.params_bytes, &b));
        assert_eq!(summary(&w, &a), summary(&w, &b), "summaries must agree");
        // So neither asks the other for anything.
        assert!(delta(&w, &a, summary(&w, &b)).is_empty());
        assert!(delta(&w, &b, summary(&w, &a)).is_empty());
    }

    /// Verification is the expensive part and anyone can ask for it, so a
    /// candidate that cannot change the state must not buy any. Counted, not
    /// argued.
    #[test]
    fn only_a_candidate_that_could_win_costs_signature_checks() {
        let w = keyset(3, 5, false);
        let held = w.encode(&w.state(w.record(false, 5, b"held")));
        let cost = |cand: Vec<u8>| {
            wire::verifications::reset();
            update_with(&w.params_bytes, &held, vec![cand]);
            wire::verifications::count()
        };
        assert_eq!(
            cost(w.encode(&w.state(w.record(false, 2, b"stale")))),
            0,
            "a replayed older record must cost a parse, not k verifications"
        );
        assert_eq!(
            cost(w.encode(&w.state(w.resigned(false, 5, b"held", 9)))),
            0,
            "another witness of the held decision must cost nothing"
        );
        assert_eq!(
            cost(w.encode(&w.state(w.record(false, 9, b"newer")))),
            w.k,
            "a greater decision costs exactly k"
        );
        // A full state offered for validation is still checked in full.
        wire::verifications::reset();
        assert!(valid(&w.params_bytes, &held));
        assert_eq!(wire::verifications::count(), w.k);
    }

    /// The most dangerous line in the contract, exercised through the entry
    /// point an attacker actually reaches.
    ///
    /// `update_state` accepts evidence from anyone — that is deliberate, since a
    /// fork must be provable by a third party. If it ever stopped checking the
    /// signatures on that evidence, anyone could mark any register forked by
    /// sending two made-up decisions, the global head included, and readers are
    /// told not to follow a forked register. Nothing about that failure is
    /// visible from `validate_state`, so every case here goes through
    /// `update_state`.
    #[test]
    fn unsigned_or_foreign_evidence_cannot_fork_a_register_through_update() {
        let w = world();
        let held = w.encode(&w.state(w.record(false, 5, b"held")));
        let (x, y) = (w.record(false, 5, b"one"), w.record(false, 5, b"two"));
        let genuine = w.evidence_of(&x, &y);

        let garbage = {
            let mut e = genuine.clone();
            e.a.sigs = vec![[0u8; 64]; w.k];
            e.b.sigs = vec![[1u8; 64]; w.k];
            e
        };
        let half = {
            let mut e = genuine.clone();
            e.b.sigs = vec![[2u8; 64]; w.k]; // one side still verifies
            e
        };
        // Real signatures, real fork — but for a DIFFERENT register. Same keyset
        // shape, so it parses here; the signatures are bound to other params.
        let other = keyset_seeded(2, 2, 4, false);
        let foreign = other.evidence_of(
            &other.record(false, 5, b"one"),
            &other.record(false, 5, b"two"),
        );

        for (what, e) in [
            ("both sides unsigned", garbage),
            ("one side unsigned", half),
            ("signed for another register", foreign),
        ] {
            let bytes = w.encode(&RegState {
                record: None,
                evidence: Some(e),
            });
            let after = update_with(&w.params_bytes, &held, vec![bytes]);
            assert_eq!(after, held, "{what}: the state changed");
            assert!(
                !read(&w.params_bytes, &after).unwrap().1.forked(),
                "{what}: forged evidence marked the register forked"
            );
        }

        // Control: genuine evidence for THIS register does fork it, so the three
        // refusals above are not just "evidence never works".
        let bytes = w.encode(&RegState {
            record: None,
            evidence: Some(genuine),
        });
        let after = update_with(&w.params_bytes, &held, vec![bytes]);
        assert_ne!(after, held);
        assert!(read(&w.params_bytes, &after).unwrap().1.forked());
    }

    /// Once a fork is proved, replaying its losing record must be free. It
    /// conflicts with the held record, so a naive "does it conflict?" test would
    /// verify it every time — and the losing record is exactly what an attacker
    /// has a copy of.
    #[test]
    fn replaying_the_losing_record_of_a_known_fork_costs_nothing() {
        let w = keyset(3, 5, false);
        let (x, y) = (w.record(false, 5, b"one"), w.record(false, 5, b"two"));
        let forked = update_with(
            &w.params_bytes,
            &w.encode(&w.state(x.clone())),
            vec![w.encode(&w.state(y.clone()))],
        );
        let parsed = read(&w.params_bytes, &forked).unwrap().1;
        assert!(parsed.forked());
        // Whichever of the two lost the record slot is the one to replay.
        let winner = parsed.record.as_ref().unwrap().value.clone();
        let loser = if winner == x.value { y } else { x };

        wire::verifications::reset();
        let after = update_with(&w.params_bytes, &forked, vec![w.encode(&w.state(loser))]);
        assert_eq!(
            wire::verifications::count(),
            0,
            "replaying the losing record bought signature checks"
        );
        assert_eq!(after, forked, "and it must change nothing");
    }

    /// A terminal record's value is the successor's IDENTITY, so its length is
    /// part of the format.
    #[test]
    fn a_terminal_value_must_be_exactly_32_bytes() {
        let w = world();
        let ok = w.record(true, 3, &[4u8; 32]);
        control_is_accepted(&w, &ok);
        for len in [0usize, 31, 33, 64] {
            let bad = w.record(true, 3, &vec![4u8; len]);
            assert!(
                !valid(&w.params_bytes, &w.encode(&w.state(bad))),
                "a {len}-byte terminal value must be refused"
            );
        }
    }

    /// Every bound `parse` enforces, the merge must enforce on its OUTPUT.
    ///
    /// A merge that can produce a state the contract refuses is worse than a
    /// bug in the merge: under F23 the host validates its own result, refuses
    /// it, and the two replicas never converge. The Set had exactly that — its
    /// deny list was unioned past the cap it parses — so the property is
    /// asserted here rather than assumed, over every pair of a set of states
    /// that includes the empty register, plain records, a terminal one and a
    /// forked one.
    #[test]
    fn the_merge_of_any_two_states_that_validate_validates() {
        let w = world();
        let forked = {
            let a = w.encode(&w.state(w.record(false, 5, b"one")));
            let b = w.encode(&w.state(w.record(false, 5, b"two")));
            update_with(&w.params_bytes, &a, vec![b])
        };
        let states: Vec<Vec<u8>> = vec![
            Vec::new(),
            w.encode(&w.state(w.record(false, 1, b"first"))),
            w.encode(&w.state(w.record(false, 9, b"ninth"))),
            w.encode(&w.state(w.record(false, 9, b"ninth, another value"))),
            w.encode(&w.state(w.record(true, 12, &[7u8; 32]))),
            forked.clone(),
            w.encode(&w.state(w.resigned(false, 9, b"ninth", 5))),
        ];
        for s in &states {
            assert!(
                valid(&w.params_bytes, s),
                "a generated state does not validate"
            );
        }
        let mut forks = 0;
        for a in &states {
            for b in &states {
                let out = update_with(&w.params_bytes, a, vec![b.clone()]);
                assert!(
                    valid(&w.params_bytes, &out),
                    "the merge produced a state the contract refuses"
                );
                if read(&w.params_bytes, &out).unwrap().1.forked() {
                    forks += 1;
                }
            }
        }
        assert!(
            forks > 0,
            "no pair produced evidence: the branch that grows the state is untested"
        );
    }

    fn summary(w: &World, s: &[u8]) -> Vec<u8> {
        Register::summarize_state(
            Parameters::from(w.params_bytes.clone()),
            State::from(s.to_vec()),
        )
        .unwrap()
        .as_ref()
        .to_vec()
    }

    fn delta(w: &World, s: &[u8], sum: Vec<u8>) -> Vec<u8> {
        Register::get_state_delta(
            Parameters::from(w.params_bytes.clone()),
            State::from(s.to_vec()),
            StateSummary::from(sum),
        )
        .unwrap()
        .as_ref()
        .to_vec()
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    /// S + L mod 2²⁵⁶ — the classic malleability, which a non-strict verifier
    /// accepts and `verify_strict` does not.
    fn add_group_order(sig: &[u8; 64]) -> [u8; 64] {
        // L = 2^252 + 27742317777372353535851937790883648493
        const L: [u8; 32] = [
            0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9,
            0xde, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x10,
        ];
        let mut out = *sig;
        let mut carry = 0u16;
        for i in 0..32 {
            let sum = out[32 + i] as u16 + L[i] as u16 + carry;
            out[32 + i] = sum as u8;
            carry = sum >> 8;
        }
        out
    }
}
