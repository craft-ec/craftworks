//! Tail contract: a writer's live head and write buffer (ARCHITECTURE.md §1, Tail).
//!
//! A Register whose value is a [`Body`] — the tree's root, how far the tree covers, and the rows written since —
//! and which also moves by small signed DELTAS, not only by whole states.
//!
//! Params = the Register's params (`"RG01" ‖ mode ‖ authority ‖ label`), read by the Register's own parser: one
//!          rule for who may write, and signatures over the Register's own signed message.
//! State  = empty (a tail nobody has written) or `"TL01" ‖ signed ‖ body`.
//! Delta  = `"TD01" ‖ signed ‖ ops`: operations to apply to the held body as the next `seq`, and the writer's
//!          signature over the RESULT's hash.
//! Valid  = the body is canonical, its hash is what the signature covers, and the signature verifies.
//! Merge  = a whole state: the Register's decision order (higher `seq`, then the lower body hash) — a semilattice.
//!          A delta: applied only as the exact next `seq`; anything else is ignored and the state sync (summary →
//!          whole state) carries the host forward.
//!
//! **No clock anywhere.** The writer decides when to flush; the order is `seq`.
//!
//! Not yet (stated, not hidden): terminal (`moved-to`) records, and keeping evidence of a fork. Two different
//! bodies at one `seq` resolve by the lower hash, as the Register's order does, but the proof is not kept.

#[cfg(feature = "freenet-main-contract")]
use freenet_stdlib::prelude::*;

pub mod body;

use body::{hash_encoded, parse_ops, HASH_LEN};
pub use body::{Body, Entry, Op};
use craftec_register_contract::wire::{Authority, Params, Signed};

pub const STATE_MAGIC: &[u8; 4] = b"TL01";
pub const DELTA_MAGIC: &[u8; 4] = b"TD01";

/// A written tail: the writer's signed decision and the body it covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tail {
    pub signed: Signed,
    pub body: Body,
}

impl Tail {
    pub fn seq(&self) -> u64 {
        self.signed.seq
    }

    pub fn encode(&self, a: &Authority) -> Vec<u8> {
        encode_state(a, &self.signed, &self.body)
    }

    fn verify(&self, p: &Params) -> bool {
        !self.signed.terminal
            && self.signed.seq >= 1
            && self.signed.value_hash == self.body.hash()
            && self.signed.verify(p)
    }
}

/// The message the writer signs for the body that results at `seq`. The signer delegate signs exactly this.
pub fn message(p: &Params, seq: u64, body_hash: &[u8; HASH_LEN]) -> Vec<u8> {
    p.signed_message(false, seq, body_hash)
}

pub fn encode_state(a: &Authority, signed: &Signed, body: &Body) -> Vec<u8> {
    [&STATE_MAGIC[..], &signed.encode(a), &body.encode()].concat()
}

pub fn encode_delta(a: &Authority, signed: &Signed, ops: &[Op]) -> Vec<u8> {
    [&DELTA_MAGIC[..], &signed.encode(a), &body::encode_ops(ops)].concat()
}

/// Largest encoded state.
pub const MAX_STATE: usize = 4 + 1024 + body::MAX_BODY;

/// Structure only: `None` for the empty tail, and refused (outer `None`) if the bytes are not one canonical state.
pub fn parse_unverified(state: &[u8], p: &Params) -> Option<Option<Tail>> {
    if state.is_empty() {
        return Some(None);
    }
    if state.len() > MAX_STATE {
        return None;
    }
    let rest = state.strip_prefix(STATE_MAGIC)?;
    let (sb, bb) = rest.split_at_checked(Signed::len_for(&p.authority))?;
    let signed = Signed::parse_for(sb, &p.authority)?;
    let body = Body::parse(bb, signed.seq)?;
    Some(Some(Tail { signed, body }))
}

/// Parse and fully verify a state against its params: what `validate_state` does.
pub fn read(params: &[u8], state: &[u8]) -> Option<(Params, Option<Tail>)> {
    let p = Params::parse(params)?;
    let t = parse_unverified(state, &p)?;
    if let Some(t) = &t {
        t.verify(&p).then_some(())?;
    }
    Some((p, t))
}

/// `seq ‖ body hash`, or nothing for an empty tail. Built from the DECISION, never the witness bytes.
pub fn summary_of(t: &Option<Tail>) -> Vec<u8> {
    match t {
        None => Vec::new(),
        Some(t) => [&t.signed.seq.to_le_bytes()[..], &t.signed.value_hash].concat(),
    }
}

fn rank(t: &Option<Tail>) -> Option<(bool, u64, core::cmp::Reverse<[u8; HASH_LEN]>)> {
    t.as_ref().map(|t| t.signed.decision().rank())
}

/// Merge one candidate — a whole state or a delta — into `held`. A candidate that cannot win costs a parse and no
/// signature check; one that fails anything leaves `held` as it was.
pub fn absorb(held: Option<Tail>, cand: &[u8], p: &Params) -> Option<Tail> {
    if cand.starts_with(DELTA_MAGIC) {
        return match apply_delta(&held, cand, p) {
            Some(next) => Some(next),
            None => held,
        };
    }
    match parse_unverified(cand, p) {
        Some(c @ Some(_)) if rank(&c) > rank(&held) && c.as_ref().is_some_and(|t| t.verify(p)) => c,
        _ => held,
    }
}

/// A delta applies only as the exact next `seq`, and only if the signature covers the body it produces.
fn apply_delta(held: &Option<Tail>, d: &[u8], p: &Params) -> Option<Tail> {
    let rest = d.strip_prefix(DELTA_MAGIC)?;
    let (sb, ob) = rest.split_at_checked(Signed::len_for(&p.authority))?;
    let signed = Signed::parse_for(sb, &p.authority)?;
    let held_seq = held.as_ref().map_or(0, |t| t.signed.seq);
    if signed.terminal || signed.seq != held_seq + 1 {
        return None;
    }
    let base = held.as_ref().map(|t| t.body.clone()).unwrap_or_default();
    let body = base.apply(&parse_ops(ob)?, signed.seq)?;
    if hash_encoded(&body.encode()) != signed.value_hash || !signed.verify(p) {
        return None;
    }
    Some(Tail { signed, body })
}

#[cfg(feature = "freenet-main-contract")]
pub struct TailContract;

#[cfg(feature = "freenet-main-contract")]
#[contract]
impl ContractInterface for TailContract {
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
        let p = Params::parse(parameters.as_ref()).ok_or(ContractError::InvalidState)?;
        // The held state passed validate_state when it was stored: structure only.
        let mut held = parse_unverified(state.as_ref(), &p).ok_or(ContractError::InvalidState)?;
        for item in data {
            let cands: [Option<&[u8]>; 2] = match &item {
                UpdateData::State(s) => [Some(s.as_ref()), None],
                UpdateData::Delta(d) => [Some(d.as_ref()), None],
                // The state first, so a delta built on it can then apply.
                UpdateData::StateAndDelta { state, delta } => {
                    [Some(state.as_ref()), Some(delta.as_ref())]
                }
                _ => [None, None],
            };
            for c in cands.into_iter().flatten() {
                // One bad candidate never blocks a good one in the same batch.
                held = absorb(held, c, &p);
            }
        }
        let out = held.map(|t| t.encode(&p.authority)).unwrap_or_default();
        Ok(UpdateModification::valid(State::from(out)))
    }

    fn summarize_state(
        parameters: Parameters<'static>,
        state: State<'static>,
    ) -> Result<StateSummary<'static>, ContractError> {
        let p = Params::parse(parameters.as_ref()).ok_or(ContractError::InvalidState)?;
        let t = parse_unverified(state.as_ref(), &p).ok_or(ContractError::InvalidState)?;
        Ok(StateSummary::from(summary_of(&t)))
    }

    fn get_state_delta(
        parameters: Parameters<'static>,
        state: State<'static>,
        summary: StateSummary<'static>,
    ) -> Result<StateDelta<'static>, ContractError> {
        let p = Params::parse(parameters.as_ref()).ok_or(ContractError::InvalidState)?;
        let t = parse_unverified(state.as_ref(), &p).ok_or(ContractError::InvalidState)?;
        // A tail is small and the whole-state merge is a join: the whole state is the delta for a peer that differs.
        if summary.as_ref() == summary_of(&t) || t.is_none() {
            return Ok(StateDelta::from(Vec::new()));
        }
        Ok(StateDelta::from(
            t.map(|t| t.encode(&p.authority)).unwrap_or_default(),
        ))
    }
}

#[cfg(test)]
mod tests;
