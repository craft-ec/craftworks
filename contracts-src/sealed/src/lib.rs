//! SEALED: one tree block of a private table, SEALED whole (ARCHITECTURE: sealing covers whole tree nodes). Its
//! address is a keyed hash of the block's id under the table's address key, so only the account can name it, and
//! nothing about the table (its blocks, their count, how they link) shows to anyone else.
//!
//! The contract cannot check the contents (it holds no key): whoever reads opens the bytes and checks them against
//! the id it asked for, so a wrong block is a missing one (rebuilt from its group). It keeps what it is given: the
//! FIRST valid state stays (a block is immutable), and a later one is ignored — the same block put again, under a
//! newer key, finds the older copy there, which every member holding that key still opens.
//!
//! Params = the address (32 bytes). State = `format(1) ‖ key reference ‖ nonce(24) ‖ ciphertext`, at most
//! [`STATE_MAX`] bytes: the tree's largest block and the seal's overhead.

/// The largest tree block (a 256 KiB value, freenet-prolly's MAX_VALUE) and the seal's header, nonce and tag.
pub const STATE_MAX: usize = 256 * 1024 + 1024;
/// The smallest: a format byte, a key reference, a nonce and a tag.
pub const STATE_MIN: usize = 1 + 1 + 24 + 16;

/// Whether `state` may be held at `params`.
pub fn valid(params: &[u8], state: &[u8]) -> bool {
    params.len() == 32 && (STATE_MIN..=STATE_MAX).contains(&state.len())
}

#[cfg(feature = "freenet-main-contract")]
mod contract {
    use super::*;
    use freenet_stdlib::prelude::*;

    pub struct Sealed;

    #[contract]
    impl ContractInterface for Sealed {
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
            let p = parameters.as_ref();
            // Held: it stays. Nothing held yet: the first valid state given.
            if valid(p, state.as_ref()) {
                return Ok(UpdateModification::valid(state));
            }
            for item in data {
                let given: Option<&[u8]> = match &item {
                    UpdateData::State(s) => Some(s.as_ref()),
                    UpdateData::Delta(d) => Some(d.as_ref()),
                    UpdateData::StateAndDelta { state, .. } => Some(state.as_ref()),
                    _ => None,
                };
                if let Some(s) = given.filter(|s| valid(p, s)) {
                    return Ok(UpdateModification::valid(State::from(s.to_vec())));
                }
            }
            Err(ContractError::InvalidState)
        }

        fn summarize_state(_parameters: Parameters<'static>, state: State<'static>) -> Result<StateSummary<'static>, ContractError> {
            // Held or not: the one fact a peer needs.
            Ok(StateSummary::from(vec![u8::from(!state.as_ref().is_empty())]))
        }

        fn get_state_delta(
            _parameters: Parameters<'static>,
            state: State<'static>,
            summary: StateSummary<'static>,
        ) -> Result<StateDelta<'static>, ContractError> {
            // A holder needs nothing; anyone else, the whole block.
            Ok(StateDelta::from(if summary.as_ref() == [1] { Vec::new() } else { state.as_ref().to_vec() }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds() {
        assert!(valid(&[0; 32], &[0; STATE_MIN]));
        assert!(valid(&[0; 32], &vec![0; STATE_MAX]));
        assert!(!valid(&[0; 32], &[0; STATE_MIN - 1]));
        assert!(!valid(&[0; 32], &vec![0; STATE_MAX + 1]));
        assert!(!valid(&[0; 31], &[0; STATE_MIN]));
    }
}
