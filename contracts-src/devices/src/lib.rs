//! DEVICES: an account's DEVICE CREDENTIALS, one contract per DID (ARCHITECTURE §4, owner 10-09: a stolen device cut
//! off for real). Readers count an account's devices here, not on its card: the card is the account's DATA key's, which
//! every device holds, so any device could rewrite it (measured 10-09: one put took fx's devices from 1 to 0).
//!
//! - Params: `did ‖ key log code hash` — the address is derived from the DID, never looked up; the key log (whose keys
//!   sign the credentials) is the contract at `instance(key log code, did)`, read as a related contract.
//! - State: `"CWDV" ‖ n(u16) ‖ (len(u16) ‖ credential)*`, the credentials sorted and unique — each the account's own
//!   node credential (`craftworks_identity::credential`), for this DID, signed by a key its key log has had.
//! - Merge: the UNION. Nothing is ever taken out, so nobody — no device holding the data key, no stolen one — can
//!   delete a device's credential, or roll the set back.
//! - Who counts NOW is the readers' rule ([`current`]): a credential signed by the log's CURRENT key. Removing a device
//!   ROTATES the key (with the words) and re-issues credentials to the devices that stay; the removed one's, on the old
//!   key, stops counting — and a stolen device cannot sign a new one (no node holds the words).

/// The most credentials one account holds (every device ever credentialed, on every key).
pub const MAX: usize = 256;
const MAGIC: &[u8; 4] = b"CWDV";
const CRED_MAX: usize = 512;

/// A devices contract's params: the DID and the key log's code hash.
pub fn params(did: &[u8; 32], idlog_code_hash: &[u8; 32]) -> Vec<u8> {
    [&did[..], idlog_code_hash].concat()
}
fn read_params(p: &[u8]) -> Option<([u8; 32], [u8; 32])> {
    (p.len() == 64).then(|| (p[..32].try_into().unwrap(), p[32..].try_into().unwrap()))
}
/// The DID's key log: its contract id.
pub fn keys_id(params: &[u8]) -> Option<[u8; 32]> {
    let (did, code) = read_params(params)?;
    Some(contract_keys::instance(&code, &did))
}

/// The set, as bytes (sorted, unique).
pub fn encode(creds: &[Vec<u8>]) -> Vec<u8> {
    let mut c: Vec<&Vec<u8>> = creds.iter().collect();
    c.sort();
    c.dedup();
    let mut v = MAGIC.to_vec();
    v.extend_from_slice(&(c.len() as u16).to_le_bytes());
    for x in c {
        v.extend_from_slice(&(x.len() as u16).to_le_bytes());
        v.extend_from_slice(x);
    }
    v
}
/// The set from bytes: exact, sorted and unique, within bounds — or nothing.
pub fn decode(b: &[u8]) -> Option<Vec<Vec<u8>>> {
    let b = b.strip_prefix(MAGIC)?;
    let (n, mut b) = b.split_at_checked(2)?;
    let n = u16::from_le_bytes([n[0], n[1]]) as usize;
    if n > MAX {
        return None;
    }
    let mut out: Vec<Vec<u8>> = Vec::with_capacity(n);
    for _ in 0..n {
        let (l, rest) = b.split_at_checked(2)?;
        let l = u16::from_le_bytes([l[0], l[1]]) as usize;
        if l > CRED_MAX {
            return None;
        }
        let (c, rest) = rest.split_at_checked(l)?;
        if out.last().is_some_and(|p| p.as_slice() >= c) {
            return None;
        }
        out.push(c.to_vec());
        b = rest;
    }
    b.is_empty().then_some(out)
}

/// Every credential is the DID's, signed by a key its key log has had (`log`: read and verified).
pub fn valid(did: &[u8; 32], log: &craftworks_idlog_contract::Log, creds: &[Vec<u8>]) -> bool {
    creds.iter().all(|c| craftworks_identity::read_credential(c).is_some_and(|(d, owner, _, _)| d == *did && log.events.iter().any(|e| e.key == owner)))
}

/// The UNION of two sets (the merge): sorted, unique — `None` past [`MAX`].
pub fn union(a: &[Vec<u8>], b: &[Vec<u8>]) -> Option<Vec<Vec<u8>>> {
    let mut u: Vec<Vec<u8>> = a.iter().chain(b).cloned().collect();
    u.sort();
    u.dedup();
    (u.len() <= MAX).then_some(u)
}

/// The account's devices NOW (the readers' rule): the node keys of the credentials signed by the log's CURRENT key, each
/// with its credential.
pub fn current(did: &[u8; 32], log: &craftworks_idlog_contract::Log, creds: &[Vec<u8>]) -> Vec<([u8; 32], Vec<u8>)> {
    let head = log.head().key;
    let mut out: Vec<([u8; 32], Vec<u8>)> = creds
        .iter()
        .filter_map(|c| craftworks_identity::read_credential(c).filter(|(d, owner, _, _)| d == did && *owner == head).map(|(_, _, node, _)| (node, c.clone())))
        .collect();
    out.sort();
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

#[cfg(feature = "freenet-main-contract")]
mod contract {
    use super::*;
    use freenet_stdlib::prelude::*;

    pub struct Devices;

    fn related_log(related: &RelatedContracts<'static>, keys: &[u8; 32], did: &[u8; 32]) -> Option<Option<craftworks_idlog_contract::Log>> {
        related.states().find(|(id, _)| id.as_bytes() == keys).map(|(_, s)| s.as_ref().and_then(|s| craftworks_idlog_contract::read(did, s.as_ref())))
    }

    #[contract]
    impl ContractInterface for Devices {
        fn validate_state(parameters: Parameters<'static>, state: State<'static>, related: RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
            let p = parameters.as_ref();
            let (Some((did, _)), Some(keys)) = (read_params(p), keys_id(p)) else { return Ok(ValidateResult::Invalid) };
            let Some(creds) = decode(state.as_ref()) else { return Ok(ValidateResult::Invalid) };
            // Its credentials are checked against the DID's key log — asked for in the one round.
            match related_log(&related, &keys, &did) {
                None => Ok(ValidateResult::RequestRelated(vec![ContractInstanceId::new(keys)])),
                Some(None) => Ok(ValidateResult::Invalid),
                Some(Some(log)) => Ok(if valid(&did, &log, &creds) { ValidateResult::Valid } else { ValidateResult::Invalid }),
            }
        }

        fn update_state(_parameters: Parameters<'static>, state: State<'static>, data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
            // The UNION — what it brings is checked by validation (against the key log), never taken out.
            let mut held = decode(state.as_ref()).ok_or(ContractError::InvalidState)?;
            for d in data {
                let b: &[u8] = match &d {
                    UpdateData::State(s) => s.as_ref(),
                    UpdateData::Delta(x) => x.as_ref(),
                    UpdateData::StateAndDelta { state, .. } => state.as_ref(),
                    _ => continue,
                };
                let given = decode(b).ok_or(ContractError::InvalidUpdate)?;
                held = union(&held, &given).ok_or(ContractError::InvalidUpdate)?;
            }
            Ok(UpdateModification::valid(State::from(encode(&held))))
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
