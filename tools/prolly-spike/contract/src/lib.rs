//! SPIKE: validation = `freenet_prolly::proof::verify_bytes` on the proof the state carries; valid iff ABSENT.
use freenet_stdlib::prelude::*;

pub struct Spike;

#[contract]
impl ContractInterface for Spike {
    fn validate_state(_p: Parameters<'static>, s: State<'static>, _r: RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
        let b = s.as_ref();
        if b.len() < 34 {
            return Ok(ValidateResult::Invalid);
        }
        let root: [u8; 32] = b[..32].try_into().unwrap();
        let n = u16::from_le_bytes([b[32], b[33]]) as usize;
        if b.len() < 34 + n {
            return Ok(ValidateResult::Invalid);
        }
        let (key, proof) = (&b[34..34 + n], &b[34 + n..]);
        Ok(match freenet_prolly::proof::verify_bytes(&root, key, proof) {
            Ok(freenet_prolly::proof::ProvenOwned::Absent) => ValidateResult::Valid,
            _ => ValidateResult::Invalid,
        })
    }
    fn update_state(_p: Parameters<'static>, s: State<'static>, data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
        for d in data {
            if let UpdateData::State(n) = d {
                return Ok(UpdateModification::valid(n));
            }
        }
        Ok(UpdateModification::valid(s))
    }
    fn summarize_state(_p: Parameters<'static>, s: State<'static>) -> Result<StateSummary<'static>, ContractError> {
        Ok(StateSummary::from(s.as_ref().len().to_le_bytes().to_vec()))
    }
    fn get_state_delta(_p: Parameters<'static>, s: State<'static>, _m: StateSummary<'static>) -> Result<StateDelta<'static>, ContractError> {
        Ok(StateDelta::from(s.as_ref().to_vec()))
    }
}
