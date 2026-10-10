//! THE STANDING CONTRACT (TOKEN §7a): one per peer (params [`Params`]: the beacon — its code, for its outputs logs, and
//! its id — and the peer), its state the peer's standing record (`craftworks_standing::record`). Checked FROM SCRATCH
//! against the beacon's outputs: the beacon for the heights it still keeps, its OUTPUTS LOG for older ones — all read
//! in one round. Copies merge record by record, the same bytes in every order.

use craftworks_beacon_contract as B;
use craftworks_standing::{codec, record};

pub const STANDING: u8 = 0x53;

/// A standing contract's PARAMS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Params {
    /// The beacon's code hash (its outputs logs are contracts of the same code).
    pub beacon_code: [u8; 32],
    /// The beacon's contract id.
    pub beacon: [u8; 32],
    pub peer: String,
}

impl Params {
    pub fn encode(&self) -> Vec<u8> {
        [&[STANDING][..], &self.beacon_code, &self.beacon, &[self.peer.len() as u8], self.peer.as_bytes()].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Params> {
        if b.len() < 66 || b[0] != STANDING {
            return None;
        }
        let n = b[65] as usize;
        if n > craftworks_standing::PEER_MAX || b.len() != 66 + n {
            return None;
        }
        Some(Params { beacon_code: b[1..33].try_into().ok()?, beacon: b[33..65].try_into().ok()?, peer: String::from_utf8(b[66..].to_vec()).ok()? })
    }
    /// A PAGE KEEPER's record params: named by its key (`craftworks_standing::keeper_peer`).
    pub fn keeper(beacon_code: [u8; 32], beacon: [u8; 32], key: &[u8; 32]) -> Params {
        Params { beacon_code, beacon, peer: craftworks_standing::keeper_peer(key) }
    }
    /// The record's contract id under the standing contract's code hash `code_hash`.
    pub fn id(&self, code_hash: &[u8; 32]) -> [u8; 32] {
        contract_keys::instance(code_hash, &self.encode())
    }
    /// The outputs log holding height `h`: its contract id.
    pub fn log_id(&self, h: u64) -> [u8; 32] {
        contract_keys::instance(&self.beacon_code, &B::log::Params::of(self.beacon, h).encode())
    }
}

/// What a check of `s` reads: the beacon, and the outputs logs of every height it answers at.
pub fn wanted(p: &Params, s: &record::State) -> Vec<[u8; 32]> {
    let mut w = vec![p.beacon];
    for e in &s.epochs {
        for a in &e.answers {
            let l = p.log_id(a.height);
            if !w.contains(&l) {
                w.push(l);
            }
        }
    }
    w
}

/// The output at `h`: from the beacon (`b`) while it keeps that step, else from the outputs log of its span (`logs`).
pub fn out_at(b: Option<&B::State>, logs: &dyn Fn(u64) -> Option<B::log::Log>, h: u64) -> Option<[u8; 32]> {
    b.and_then(|b| b.steps.iter().find(|s| s.height == h)).map(|s| s.out).or_else(|| logs(h).and_then(|l| l.get(h)))
}

/// The record of `p`'s peer holds, against the beacon and its logs (as read).
pub fn check(p: &Params, s: &record::State, b: Option<&B::State>, logs: &dyn Fn(u64) -> Option<B::log::Log>) -> Result<(), craftworks_standing::Why> {
    record::holds(&p.peer, s, &|h| out_at(b, logs, h))
}

pub fn decode(b: &[u8]) -> Option<record::State> {
    codec::decode(b)
}

#[cfg(feature = "freenet-main-contract")]
mod contract {
    use super::{check, codec, record, wanted, Params, B};
    use freenet_stdlib::prelude::*;

    pub struct Standing;

    fn related_state(related: &RelatedContracts<'static>, want: &[u8; 32]) -> Option<Option<Vec<u8>>> {
        related.states().find(|(id, _)| id.as_bytes() == want).map(|(_, s)| s.as_ref().map(|s| s.as_ref().to_vec()))
    }

    #[contract]
    impl ContractInterface for Standing {
        fn validate_state(parameters: Parameters<'static>, state: State<'static>, related: RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
            let (Some(p), Some(s)) = (Params::decode(parameters.as_ref()), codec::decode(state.as_ref())) else { return Ok(ValidateResult::Invalid) };
            let want = wanted(&p, &s);
            let missing: Vec<ContractInstanceId> = want.iter().filter(|w| related_state(&related, w).is_none()).map(|w| ContractInstanceId::new(*w)).collect();
            if !missing.is_empty() {
                return Ok(ValidateResult::RequestRelated(missing));
            }
            let b = related_state(&related, &p.beacon).flatten().and_then(|b| B::wire::decode(&b));
            let logs = |h: u64| related_state(&related, &p.log_id(h)).flatten().and_then(|l| B::log::Log::decode(&l));
            Ok(if check(&p, &s, b.as_ref(), &logs).is_ok() { ValidateResult::Valid } else { ValidateResult::Invalid })
        }

        fn update_state(parameters: Parameters<'static>, state: State<'static>, data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
            let p = Params::decode(parameters.as_ref()).ok_or(ContractError::InvalidState)?;
            let held = codec::decode(state.as_ref()).ok_or(ContractError::InvalidState)?;
            let mut merged = held.clone();
            let mut related: Vec<([u8; 32], Vec<u8>)> = Vec::new();
            for d in data {
                let b: &[u8] = match &d {
                    UpdateData::State(s) => s.as_ref(),
                    UpdateData::Delta(x) => x.as_ref(),
                    UpdateData::StateAndDelta { state, .. } => state.as_ref(),
                    UpdateData::RelatedState { related_to, state } => {
                        let mut k = [0u8; 32];
                        k.copy_from_slice(related_to.as_bytes());
                        related.push((k, state.as_ref().to_vec()));
                        continue;
                    }
                    _ => continue,
                };
                merged = record::merge(&merged, &codec::decode(b).ok_or(ContractError::InvalidUpdate)?);
            }
            if merged == held {
                return Ok(UpdateModification::valid(State::from(codec::encode(&held))));
            }
            let want = wanted(&p, &merged);
            let got = |w: &[u8; 32]| related.iter().find(|(k, _)| k == w).map(|(_, b)| b.clone());
            if want.iter().any(|w| got(w).is_none()) {
                return UpdateModification::requires(want.iter().map(|w| RelatedContract { contract_instance_id: ContractInstanceId::new(*w), mode: RelatedMode::StateOnce }).collect());
            }
            let b = got(&p.beacon).and_then(|b| B::wire::decode(&b));
            let logs = |h: u64| got(&p.log_id(h)).and_then(|l| B::log::Log::decode(&l));
            check(&p, &merged, b.as_ref(), &logs).map_err(|w| ContractError::InvalidUpdateWithInfo { reason: w.into() })?;
            Ok(UpdateModification::valid(State::from(codec::encode(&merged))))
        }

        fn summarize_state(_parameters: Parameters<'static>, state: State<'static>) -> Result<StateSummary<'static>, ContractError> {
            Ok(StateSummary::from(blake3::hash(state.as_ref()).as_bytes().to_vec()))
        }

        fn get_state_delta(_parameters: Parameters<'static>, state: State<'static>, summary: StateSummary<'static>) -> Result<StateDelta<'static>, ContractError> {
            Ok(StateDelta::from(if summary.as_ref() == blake3::hash(state.as_ref()).as_bytes() { Vec::new() } else { state.as_ref().to_vec() }))
        }
    }

}

#[cfg(test)]
mod tests;
