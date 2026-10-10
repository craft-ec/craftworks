//! THE WREN ALLOWANCE (TOKEN §7d, decision 6) — an APP over the token's escrow, not part of the token. One contract per
//! standing EPOCH ([`Params`]): the treasury ESCROWS the epoch's pot to it (its `Escrow` entry, terms [`TERMS`], with an
//! `until` past the epoch's end: unpaid, it reclaims), and its state is the token's standard escrow SECTION then the
//! statement ROOT it pays under and each payout's PROOF. The verdict:
//!
//! - one payout per account whose line in the epoch STATEMENT (`reputation::statement`, a prolly tree DID → line) has
//!   an allowance — to that account (its DID chain; the wallet moves it to private), keyed [`pay_key`], the amount its
//!   line says, PROVEN under the root;
//! - each escrow's REST refunded, so nothing stays locked.
//!
//! Anyone computes and puts it (the page that publishes the statement does); anyone appends the release and the
//! payouts; an offline account is paid later. A verdict, once held, is NEVER replaced. What the contract cannot see —
//! that the root IS the epoch's statement (anyone may post one) and that each escrow is real and accepted — witnesses
//! check (they recompute the statement; `wallet::program_backed`), and sign no payout of a verdict under another root.

use craftworks_token_contract::escrow::{self, EscrowRef, Section};
use craftworks_token_contract::Tok;

pub type Why = &'static str;
pub type Did = [u8; 32];

pub const PARAMS: u8 = 0x41;
/// The terms every escrow to an allowance program carries (the program is its own rule: nothing more to say).
pub const TERMS: &[u8] = b"craftworks allowance";
use craftworks_token_contract::UNIT;
/// The pot per standing epoch: 59.26 WREN — 1,000 a day at B's pace of a step per 5 s (1,024 steps ≈ 85 min).
pub const POT: u128 = 1_000 * UNIT * 1_024 * 5 / 86_400;
/// Most one account takes in an epoch (2% of the pot, rounded down — 20 of 1,000 a day).
pub const CAP: u128 = POT * 2 / 100;

/// The program's params: the token it pays and the standing epoch whose statement it follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub token: Tok,
    pub epoch: u64,
}

impl Params {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = vec![PARAMS];
        v.extend_from_slice(&self.token.definition);
        v.extend_from_slice(&self.token.item);
        v.extend_from_slice(&self.epoch.to_le_bytes());
        v
    }
    pub fn decode(b: &[u8]) -> Option<Params> {
        if b.len() != 1 + 64 + 8 || b[0] != PARAMS {
            return None;
        }
        Some(Params { token: Tok { definition: b[1..33].try_into().ok()?, item: b[33..65].try_into().ok()? }, epoch: u64::from_le_bytes(b[65..73].try_into().ok()?) })
    }
}

/// The payout key of `did`'s allowance in `epoch`: once each.
pub fn pay_key(epoch: u64, did: &Did) -> [u8; 16] {
    let mut h = blake3::Hasher::new_derive_key("craftworks allowance payout");
    h.update(&epoch.to_le_bytes());
    h.update(did);
    h.finalize().as_bytes()[..16].try_into().unwrap()
}

/// The program's STATE: its section, the statement root it pays under, and each payout's proof (by DID).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub section: Section,
    pub root: [u8; 32],
    pub proofs: Vec<(Did, Vec<u8>)>,
}

const PROOF_MAX: usize = 16 * 1024;

impl State {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = self.section.encode();
        v.extend_from_slice(&self.root);
        v.extend_from_slice(&(self.proofs.len() as u32).to_le_bytes());
        for (d, p) in &self.proofs {
            v.extend_from_slice(d);
            v.extend_from_slice(&(p.len() as u32).to_le_bytes());
            v.extend_from_slice(p);
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<State> {
        let (section, rest) = Section::decode_prefix(b)?;
        if rest.is_empty() {
            return Some(State { section, ..Default::default() });
        }
        let root: [u8; 32] = rest.get(..32)?.try_into().ok()?;
        let mut r = &rest[32..];
        let take = |r: &mut &[u8], n: usize| -> Option<Vec<u8>> {
            let (a, b) = (r.get(..n)?.to_vec(), r.get(n..)?);
            *r = b;
            Some(a)
        };
        let n = u32::from_le_bytes(take(&mut r, 4)?.try_into().ok()?) as usize;
        if n > escrow::LIST_MAX {
            return None;
        }
        let mut proofs = Vec::with_capacity(n);
        for _ in 0..n {
            let d: Did = take(&mut r, 32)?.try_into().ok()?;
            let len = u32::from_le_bytes(take(&mut r, 4)?.try_into().ok()?) as usize;
            if len > PROOF_MAX {
                return None;
            }
            proofs.push((d, take(&mut r, len)?));
        }
        r.is_empty().then_some(State { section, root, proofs })
    }
    /// Decided: a verdict (anything refunded or paid), never replaced.
    pub fn decided(&self) -> bool {
        !self.section.refunds.is_empty() || !self.section.pays.is_empty()
    }
}

/// `did`'s allowance under `root`, from its proof (`None`: no line, or a proof that does not hold).
pub fn proven(root: &[u8; 32], did: &Did, proof: &[u8]) -> Option<u128> {
    craftworks_reputation::statement::line_proven(root, did, proof).map(|l| l.allowance)
}

/// The VERDICT for `escrows` (each as its `Escrow` entry says) under the statement `root`: each account's allowance
/// paid (`lines`: DID, its proof), each escrow's rest refunded (in the order listed, the first first).
pub fn verdict(p: &Params, escrows: &[(EscrowRef, u128)], root: &[u8; 32], lines: &[(Did, Vec<u8>)]) -> Result<State, Why> {
    let h = escrow::terms_hash(TERMS);
    let mut pays = Vec::new();
    let mut proofs = Vec::new();
    for (d, proof) in lines {
        let a = proven(root, d, proof).ok_or("a line that is not under the root")?;
        if a > 0 {
            pays.push((pay_key(p.epoch, d), p.token, *d, a));
            proofs.push((*d, proof.clone()));
        }
    }
    let mut left: u128 = pays.iter().map(|x| x.3).sum();
    let mut refunds = Vec::new();
    for (r, a) in escrows {
        let used = left.min(*a);
        left -= used;
        refunds.push((*r, a - used));
    }
    if left > 0 {
        return Err("more owed than escrowed");
    }
    pays.sort();
    proofs.sort();
    let mut es: Vec<(EscrowRef, u128, [u8; 32])> = escrows.iter().map(|(r, a)| (*r, *a, h)).collect();
    es.sort();
    refunds.sort();
    Ok(State { section: Section { escrows: es, refunds, pays }, root: *root, proofs })
}

/// THE RULE of an allowance program's state: empty (not decided), or a VERDICT — every escrow of this program's token on
/// its terms, every payout one account's PROVEN allowance under the root (to that account, keyed for it, once), and
/// every escrow's rest refunded (nothing locked, nothing over).
pub fn check(p: &Params, s: &State) -> Result<(), Why> {
    if !s.decided() {
        return if s.section.escrows.is_empty() && s.proofs.is_empty() { Ok(()) } else { Err("an undecided state that lists something") };
    }
    s.section.holds()?;
    let h = escrow::terms_hash(TERMS);
    if s.section.escrows.iter().any(|(r, _, t)| r.0 != p.token || *t != h) {
        return Err("an escrow that is not this program's");
    }
    if s.proofs.len() != s.section.pays.len() {
        return Err("a payout without its proof");
    }
    for (key, tok, to, amount) in &s.section.pays {
        let proof = s.proofs.iter().find(|(d, _)| d == to).map(|x| &x.1).ok_or("a payout without its proof")?;
        if *tok != p.token || *key != pay_key(p.epoch, to) {
            return Err("a payout that is not its account's");
        }
        if proven(&s.root, to, proof) != Some(*amount) || *amount == 0 {
            return Err("a payout that is not its line's allowance");
        }
    }
    let escrowed: u128 = s.section.escrows.iter().map(|e| e.1).sum();
    let paid: u128 = s.section.pays.iter().map(|x| x.3).sum();
    let refunded: u128 = s.section.refunds.iter().map(|x| x.1).sum();
    if s.section.refunds.len() != s.section.escrows.len() || paid + refunded != escrowed {
        return Err("a verdict that does not settle every escrow");
    }
    Ok(())
}

/// MERGE: a verdict held is kept; else a valid one given is taken.
pub fn merge(p: &Params, held: &State, given: &State) -> State {
    if held.decided() || check(p, given).is_err() {
        held.clone()
    } else {
        given.clone()
    }
}

#[cfg(test)]
mod tests;

#[cfg(feature = "freenet-main-contract")]
mod contract {
    use super::{check, merge, Params};
    use freenet_stdlib::prelude::*;

    pub struct Allowance;

    #[contract]
    impl ContractInterface for Allowance {
        fn validate_state(parameters: Parameters<'static>, state: State<'static>, _related: RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
            let (Some(p), Some(s)) = (Params::decode(parameters.as_ref()), super::State::decode(state.as_ref())) else { return Ok(ValidateResult::Invalid) };
            Ok(if check(&p, &s).is_ok() { ValidateResult::Valid } else { ValidateResult::Invalid })
        }

        fn update_state(parameters: Parameters<'static>, state: State<'static>, data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
            let p = Params::decode(parameters.as_ref()).ok_or(ContractError::InvalidState)?;
            let mut held = super::State::decode(state.as_ref()).ok_or(ContractError::InvalidState)?;
            for x in data {
                let b: &[u8] = match &x {
                    UpdateData::State(s) => s.as_ref(),
                    UpdateData::Delta(x) => x.as_ref(),
                    UpdateData::StateAndDelta { state, .. } => state.as_ref(),
                    _ => continue,
                };
                let given = super::State::decode(b).ok_or(ContractError::InvalidUpdate)?;
                held = merge(&p, &held, &given);
            }
            Ok(UpdateModification::valid(held.encode().into()))
        }

        fn summarize_state(_p: Parameters<'static>, state: State<'static>) -> Result<StateSummary<'static>, ContractError> {
            Ok(StateSummary::from(blake3::hash(state.as_ref()).as_bytes().to_vec()))
        }

        fn get_state_delta(_p: Parameters<'static>, state: State<'static>, summary: StateSummary<'static>) -> Result<StateDelta<'static>, ContractError> {
            Ok(StateDelta::from(if summary.as_ref() == blake3::hash(state.as_ref()).as_bytes() { vec![] } else { state.as_ref().to_vec() }))
        }
    }
}
