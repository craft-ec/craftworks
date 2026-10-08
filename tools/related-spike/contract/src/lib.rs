//! SPIKE: one code, two roles by params. A LOCK (`[0]`) holds any state. A FILL (`[1] ‖ lock id`) is valid only if
//! the lock's state, fetched by the node as a RELATED contract, starts with `LOCK`.
use freenet_stdlib::prelude::*;

fn lock_of(params: &[u8]) -> Option<ContractInstanceId> {
    (params.len() == 33 && params[0] == 1).then(|| ContractInstanceId::new(params[1..].try_into().unwrap()))
}

pub struct Spike;

#[contract]
impl ContractInterface for Spike {
    fn validate_state(parameters: Parameters<'static>, _state: State<'static>, related: RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
        let Some(lock) = lock_of(parameters.as_ref()) else { return Ok(ValidateResult::Valid) };
        for (id, st) in related.states() {
            if *id == lock {
                return Ok(match st {
                    Some(s) if s.as_ref().starts_with(b"LOCK") => ValidateResult::Valid,
                    _ => ValidateResult::Invalid,
                });
            }
        }
        Ok(ValidateResult::RequestRelated(vec![lock]))
    }

    fn update_state(_parameters: Parameters<'static>, state: State<'static>, data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
        for d in data {
            if let UpdateData::State(s) = d {
                return Ok(UpdateModification::valid(s));
            }
        }
        Ok(UpdateModification::valid(state))
    }

    fn summarize_state(_parameters: Parameters<'static>, state: State<'static>) -> Result<StateSummary<'static>, ContractError> {
        Ok(StateSummary::from(state.as_ref().len().to_le_bytes().to_vec()))
    }

    fn get_state_delta(_parameters: Parameters<'static>, state: State<'static>, _summary: StateSummary<'static>) -> Result<StateDelta<'static>, ContractError> {
        Ok(StateDelta::from(state.as_ref().to_vec()))
    }
}
