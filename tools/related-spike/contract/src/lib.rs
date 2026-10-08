//! SPIKE: one code, two roles by params. A LOCK (`[0]`) holds any state. A FILL (`[1] ‖ lock id`) is valid only if
//! the lock's state, fetched by the node as a RELATED contract, starts with `LOCK`.
use freenet_stdlib::prelude::*;

fn lock_of(params: &[u8]) -> Option<ContractInstanceId> {
    (params.len() == 33 && params[0] == 1).then(|| ContractInstanceId::new(params[1..].try_into().unwrap()))
}
/// A GUARD (`[2] ‖ lock id`, 10-08, token step 5b's question): any state is VALID, but an UPDATE is taken only if its
/// lock — asked for from `update_state` as a related contract — starts with LOCK.
fn guard_of(params: &[u8]) -> Option<ContractInstanceId> {
    (params.len() == 33 && params[0] == 2).then(|| ContractInstanceId::new(params[1..].try_into().unwrap()))
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

    fn update_state(parameters: Parameters<'static>, state: State<'static>, data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
        if let Some(lock) = guard_of(parameters.as_ref()) {
            let mut lock_state: Option<Vec<u8>> = None;
            let mut new: Option<State<'static>> = None;
            for d in data {
                match d {
                    UpdateData::RelatedState { related_to, state } if related_to == lock => lock_state = Some(state.as_ref().to_vec()),
                    UpdateData::State(s) => new = Some(s),
                    _ => {}
                }
            }
            let Some(l) = lock_state else {
                return UpdateModification::requires(vec![RelatedContract { contract_instance_id: lock, mode: RelatedMode::StateOnce }]);
            };
            if !l.starts_with(b"LOCK") {
                return Err(ContractError::InvalidUpdate);
            }
            return Ok(UpdateModification::valid(new.unwrap_or(state)));
        }
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
