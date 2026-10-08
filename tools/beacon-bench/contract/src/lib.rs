//! MEASURE: validation = N sequential blake3 iterations from the state's first 32 bytes; valid iff the result's first
//! byte is not 0xFF... (always computed, never skipped: the result decides).
use freenet_stdlib::prelude::*;
pub struct Bench;
#[contract]
impl ContractInterface for Bench {
    fn validate_state(p: Parameters<'static>, s: State<'static>, _r: RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
        let n = u64::from_le_bytes(p.as_ref().get(..8).and_then(|b| b.try_into().ok()).unwrap_or([0; 8]));
        let mut x = [0u8; 32];
        let b = s.as_ref();
        x[..b.len().min(32)].copy_from_slice(&b[..b.len().min(32)]);
        for _ in 0..n {
            x = *blake3::hash(&x).as_bytes();
        }
        // The result is used: a state whose chain ends in 0xFF 0xFF is refused (practically never).
        Ok(if x[0] == 0xFF && x[1] == 0xFF { ValidateResult::Invalid } else { ValidateResult::Valid })
    }
    fn update_state(_p: Parameters<'static>, state: State<'static>, data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
        for d in data { if let UpdateData::State(s) = d { return Ok(UpdateModification::valid(s)); } }
        Ok(UpdateModification::valid(state))
    }
    fn summarize_state(_p: Parameters<'static>, s: State<'static>) -> Result<StateSummary<'static>, ContractError> {
        Ok(StateSummary::from(s.as_ref().len().to_le_bytes().to_vec()))
    }
    fn get_state_delta(_p: Parameters<'static>, s: State<'static>, _m: StateSummary<'static>) -> Result<StateDelta<'static>, ContractError> {
        Ok(StateDelta::from(s.as_ref().to_vec()))
    }
}
