//! PIECE: one sealed piece of a FILE (a fragment or an index piece, ARCHITECTURE §6) at an address only the file's key
//! computes. The contract cannot open it (it holds no key); a reader checks it against the index.
//!
//! The FIRST valid write stays (a piece is immutable) — and names its BURN HASH: sha-256 of a secret only those who
//! held the file's space salt compute. Showing that secret BURNS the piece: its bytes are replaced by the secret alone,
//! for good. So a file re-keyed away from someone (removed from its space) stops being readable from the network, not
//! merely no longer kept. A public file's burn hash is all zero: no secret hashes to it, so it is never burned.
//!
//! Params = the address (32 bytes). State = `LIVE(2) ‖ burn hash(32) ‖ payload`, or `BURNED(3) ‖ burn hash(32) ‖
//! secret(32)`. The payload is at most [`PAYLOAD_MAX`]: a file's largest piece and its seal.

use sha2::{Digest, Sha256};

pub const LIVE: u8 = 2;
pub const BURNED: u8 = 3;
/// The largest piece (a 256 KiB chunk and its coefficients, or an index piece) with its seal.
pub const PAYLOAD_MAX: usize = 256 * 1024 + 1024;
pub const PAYLOAD_MIN: usize = 1;

/// What a state is: `Some(true)` live, `Some(false)` burned (its secret checked), `None` not valid.
pub fn kind(state: &[u8]) -> Option<bool> {
    let (&f, rest) = state.split_first()?;
    let (hash, body) = rest.split_first_chunk::<32>()?;
    match f {
        LIVE => (PAYLOAD_MIN..=PAYLOAD_MAX).contains(&body.len()).then_some(true),
        BURNED => (body.len() == 32 && hash != &[0; 32] && Sha256::digest(body).as_slice() == hash).then_some(false),
        _ => None,
    }
}

/// Whether `state` may be held at `params`.
pub fn valid(params: &[u8], state: &[u8]) -> bool {
    params.len() == 32 && kind(state).is_some()
}

/// What `held` becomes when `given` arrives: the first valid state; a live piece burned by ITS secret; a burned one
/// stays. `None`: nothing valid either way.
pub fn next<'a>(held: &'a [u8], given: &'a [u8]) -> Option<&'a [u8]> {
    match (kind(held), kind(given)) {
        (None, Some(_)) => Some(given),
        (Some(true), Some(false)) if held[1..33] == given[1..33] => Some(given),
        (Some(_), _) => Some(held),
        (None, None) => None,
    }
}

#[cfg(feature = "freenet-main-contract")]
mod contract {
    use super::*;
    use freenet_stdlib::prelude::*;

    pub struct Piece;

    #[contract]
    impl ContractInterface for Piece {
        fn validate_state(
            parameters: Parameters<'static>,
            state: State<'static>,
            _related: RelatedContracts<'static>,
        ) -> Result<ValidateResult, ContractError> {
            Ok(if valid(parameters.as_ref(), state.as_ref()) { ValidateResult::Valid } else { ValidateResult::Invalid })
        }

        fn update_state(
            parameters: Parameters<'static>,
            state: State<'static>,
            data: Vec<UpdateData<'static>>,
        ) -> Result<UpdateModification<'static>, ContractError> {
            if parameters.as_ref().len() != 32 {
                return Err(ContractError::InvalidState);
            }
            let mut held = state.as_ref().to_vec();
            for item in data {
                let given: Option<&[u8]> = match &item {
                    UpdateData::State(s) => Some(s.as_ref()),
                    UpdateData::Delta(d) => Some(d.as_ref()),
                    UpdateData::StateAndDelta { state, .. } => Some(state.as_ref()),
                    _ => None,
                };
                if let Some(n) = given.and_then(|g| next(&held, g)) {
                    held = n.to_vec();
                }
            }
            if kind(&held).is_none() {
                return Err(ContractError::InvalidState);
            }
            Ok(UpdateModification::valid(State::from(held)))
        }

        fn summarize_state(_parameters: Parameters<'static>, state: State<'static>) -> Result<StateSummary<'static>, ContractError> {
            // Nothing, live, or burned: all a peer needs to know what to send.
            Ok(StateSummary::from(vec![match kind(state.as_ref()) {
                None => 0,
                Some(true) => 1,
                Some(false) => 2,
            }]))
        }

        fn get_state_delta(
            _parameters: Parameters<'static>,
            state: State<'static>,
            summary: StateSummary<'static>,
        ) -> Result<StateDelta<'static>, ContractError> {
            let theirs = summary.as_ref().first().copied().unwrap_or(0);
            let ours = match kind(state.as_ref()) {
                None => 0,
                Some(true) => 1,
                Some(false) => 2,
            };
            // Only forward: a holder of a live piece gets the burn; one holding the same or more, nothing.
            Ok(StateDelta::from(if ours > theirs { state.as_ref().to_vec() } else { Vec::new() }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(hash: [u8; 32], payload: &[u8]) -> Vec<u8> {
        [&[LIVE][..], &hash, payload].concat()
    }
    fn burned(secret: [u8; 32]) -> Vec<u8> {
        [&[BURNED][..], Sha256::digest(secret).as_slice(), &secret].concat()
    }

    #[test]
    fn first_write_stays_and_only_its_secret_burns_it() {
        let secret = [7; 32];
        let hash: [u8; 32] = Sha256::digest(secret).into();
        let a = live(hash, b"payload a");
        let b = live(hash, b"payload b");
        assert_eq!(next(&[], &a), Some(&a[..]));
        assert_eq!(next(&a, &b), Some(&a[..]), "first write wins");
        let wrong = burned([8; 32]);
        assert_eq!(next(&a, &wrong), Some(&a[..]), "another secret's burn is ignored");
        let burn = burned(secret);
        assert_eq!(next(&a, &burn), Some(&burn[..]));
        assert_eq!(next(&burn, &a), Some(&burn[..]), "burned for good");
    }

    #[test]
    fn a_public_piece_is_never_burned_and_bad_states_are_refused() {
        let pubp = live([0; 32], b"public");
        assert!(kind(&pubp) == Some(true));
        // No secret hashes to zero: a burned state naming a zero hash is not valid.
        let fake = [&[BURNED][..], &[0; 32], &[1; 32]].concat();
        assert_eq!(kind(&fake), None);
        assert_eq!(next(&pubp, &fake), Some(&pubp[..]));
        assert_eq!(kind(&[LIVE]), None);
        assert_eq!(kind(&live([1; 32], &[])), None, "an empty payload");
        assert_eq!(kind(&live([1; 32], &vec![0; PAYLOAD_MAX + 1])), None);
        assert!(!valid(&[0; 31], &pubp));
        let bad_secret = [&[BURNED][..], &[9; 32], &[1; 32]].concat();
        assert_eq!(kind(&bad_secret), None, "a secret that does not hash to the burn hash");
    }
}
