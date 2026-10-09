//! A DIRECT SWAP (TOKEN §5) — an APP over the token's escrow, not part of the token. One contract per DEAL ([`Deal`]):
//! party X gives `amount_x` of token `x`, party Y gives `amount_y` of token `y`, by beacon `deadline`. Each escrows
//! what it gives with THIS contract (its `Escrow` entry, terms [`TERMS`]), and its escrow is listed in the deal's
//! state — the token's standard escrow SECTION, then nothing more. The verdict:
//!
//! - both listed, while the beacon is before `deadline + GRACE`: CROSSED — each paid the other's amount, nothing back;
//!   a side whose token carries a ROYALTY (its definition's) has its creator's fee taken out of what it is paid, so the
//!   token's royalty rule takes every payout (else the item would never come out while the payment did);
//! - from `deadline + GRACE` on, one side alone: that side refunded whole.
//!
//! A verdict, once taken, is the state's for good — NEVER replaced (a refund already paid out and then a crossing would
//! pay the other side out of an escrow already gone). Crossing is taken only before `deadline + GRACE`, a refund only
//! from then on (checked as each update is taken; a state valid then stays valid). The grace is for the two escrows to
//! meet before anyone refunds. The contract checks all of it; the token checks each release against it; witnesses
//! check the listed escrows are real, accepted and on the deal's terms.

use craftworks_token_contract::escrow::{self, EscrowRef, Royalty, Section};
use craftworks_token_contract::Tok;

pub const DEAL: u8 = 0x53;
/// Steps after the deadline before a refund (for both escrows to meet first).
pub const GRACE: u64 = 4;
/// The terms every escrow to a deal carries (the deal itself is its program: nothing more to say).
pub const TERMS: &[u8] = b"craftworks swap";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deal {
    pub x: Tok,
    pub party_x: [u8; 32],
    pub amount_x: u128,
    pub y: Tok,
    pub party_y: [u8; 32],
    pub amount_y: u128,
    pub deadline: u64,
    pub beacon: [u8; 32],
}

fn put_tok(v: &mut Vec<u8>, t: &Tok) {
    v.extend_from_slice(&t.definition);
    v.extend_from_slice(&t.item);
}

impl Deal {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = vec![DEAL];
        put_tok(&mut v, &self.x);
        v.extend_from_slice(&self.party_x);
        v.extend_from_slice(&self.amount_x.to_le_bytes());
        put_tok(&mut v, &self.y);
        v.extend_from_slice(&self.party_y);
        v.extend_from_slice(&self.amount_y.to_le_bytes());
        v.extend_from_slice(&self.deadline.to_le_bytes());
        v.extend_from_slice(&self.beacon);
        v
    }
    pub fn decode(b: &[u8]) -> Option<Deal> {
        if b.len() != 1 + 64 + 32 + 16 + 64 + 32 + 16 + 8 + 32 || b[0] != DEAL {
            return None;
        }
        let a32 = |i: usize| -> [u8; 32] { b[i..i + 32].try_into().unwrap() };
        let u = |i: usize| u128::from_le_bytes(b[i..i + 16].try_into().unwrap());
        let d = Deal {
            x: Tok { definition: a32(1), item: a32(33) },
            party_x: a32(65),
            amount_x: u(97),
            y: Tok { definition: a32(113), item: a32(145) },
            party_y: a32(177),
            amount_y: u(209),
            deadline: u64::from_le_bytes(b[225..233].try_into().unwrap()),
            beacon: a32(233),
        };
        ((d.x, d.party_x) != (d.y, d.party_y) && d.amount_x > 0 && d.amount_y > 0).then_some(d)
    }
    /// The side an escrow is: `Some(true)` X's, `Some(false)` Y's.
    fn side(&self, r: &EscrowRef, amount: u128) -> Option<bool> {
        if (r.0, r.1, amount) == (self.x, self.party_x, self.amount_x) {
            Some(true)
        } else if (r.0, r.1, amount) == (self.y, self.party_y, self.amount_y) {
            Some(false)
        } else {
            None
        }
    }
}

/// The payout key of what a side receives.
pub fn pay_key(r: &EscrowRef) -> [u8; 16] {
    key_of("craftworks swap payout", r)
}
fn key_of(what: &str, r: &EscrowRef) -> [u8; 16] {
    let mut h = blake3::Hasher::new_derive_key(what);
    h.update(&r.0.definition);
    h.update(&r.0.item);
    h.update(&r.1);
    h.update(&r.2);
    h.finalize().as_bytes()[..16].try_into().unwrap()
}

pub type Why = &'static str;


/// The payout key of a creator's fee out of what a side receives.
pub fn fee_key(r: &EscrowRef) -> [u8; 16] {
    key_of("craftworks swap royalty", r)
}

/// The deal's verdicts: CROSSED (both listed; `roy`: X's and Y's tokens' royalties) or REFUNDED (one alone).
pub fn crossed(d: &Deal, roy: (Royalty, Royalty), ex: &EscrowRef, ey: &EscrowRef) -> Section {
    let h = escrow::terms_hash(TERMS);
    let mut pays = Vec::new();
    // X sold its token for Y's: X's royalty out of what X is paid (in Y's token); the same the other way.
    for (r, side, paid_in, party, amount) in [(roy.0, ex, d.y, d.party_x, d.amount_y), (roy.1, ey, d.x, d.party_y, d.amount_x)] {
        let f = escrow::royalty_fee(r, &party, amount);
        pays.push((pay_key(side), paid_in, party, amount - f.map_or(0, |f| f.1)));
        if let Some((creator, a)) = f {
            pays.push((fee_key(side), paid_in, creator, a));
        }
    }
    Section { escrows: sorted(vec![(*ex, d.amount_x, h), (*ey, d.amount_y, h)]), refunds: sorted(vec![(*ex, 0), (*ey, 0)]), pays }
}
pub fn refunded(d: &Deal, e: &EscrowRef, x_side: bool) -> Section {
    let amount = if x_side { d.amount_x } else { d.amount_y };
    Section { escrows: vec![(*e, amount, escrow::terms_hash(TERMS))], refunds: vec![(*e, amount)], pays: vec![] }
}
fn sorted<T: Ord>(mut v: Vec<T>) -> Vec<T> {
    v.sort();
    v
}
fn listed(d: &Deal, e: &[(EscrowRef, bool)]) -> Section {
    let h = escrow::terms_hash(TERMS);
    Section { escrows: sorted(e.iter().map(|(r, x)| (*r, if *x { d.amount_x } else { d.amount_y }, h)).collect()), ..Default::default() }
}

/// A deal's state HOLDS (the beacon at `now`): at most one escrow a side, each the deal's party, token and amount on
/// its terms; no verdict, or CROSSED with both, or — from `deadline + GRACE` — REFUNDED with one alone.
pub fn check(d: &Deal, roy: (Royalty, Royalty), s: &Section, now: u64) -> Result<(), Why> {
    let sides: Vec<(EscrowRef, bool)> = s.escrows.iter().map(|(r, a, h)| (*h == escrow::terms_hash(TERMS)).then(|| d.side(r, *a).map(|x| (*r, x))).flatten()).collect::<Option<Vec<_>>>().ok_or("an escrow that is not this deal's")?;
    let xs = sides.iter().filter(|s| s.1).count();
    let ys = sides.len() - xs;
    if xs > 1 || ys > 1 {
        return Err("two escrows of one side");
    }
    if s.refunds.is_empty() && s.pays.is_empty() {
        return if *s == listed(d, &sides) { Ok(()) } else { Err("a state that is not its escrows") };
    }
    if xs == 1 && ys == 1 {
        let ex = sides.iter().find(|s| s.1).unwrap().0;
        let ey = sides.iter().find(|s| !s.1).unwrap().0;
        return if *s == crossed(d, roy, &ex, &ey) { Ok(()) } else { Err("a verdict that is not the deal crossed") };
    }
    if now < d.deadline.saturating_add(GRACE) {
        return Err("a refund before the deal's deadline");
    }
    let (e, x) = sides[0];
    if *s == refunded(d, &e, x) { Ok(()) } else { Err("a verdict that is not the refund") }
}

/// THE MERGE (an update, the beacon at `now`): a verdict held is KEPT, whatever is given. None held: a given verdict
/// taken as its kind allows now (crossing before `deadline + GRACE`, a refund from then on); else the escrows' union
/// (one a side) — crossed at once when both are there, before the cutoff.
pub fn merge(d: &Deal, roy: (Royalty, Royalty), held: &Section, given: &Section, now: u64) -> Section {
    let decided = |s: &Section| !s.refunds.is_empty() || !s.pays.is_empty();
    let before = now < d.deadline.saturating_add(GRACE);
    match (decided(held), decided(given)) {
        (true, _) => held.clone(),
        (false, true) => {
            if given.pays.is_empty() != before {
                given.clone()
            } else {
                held.clone()
            }
        }
        (false, false) => {
            let mut all: Vec<(EscrowRef, bool)> = Vec::new();
            for (r, a, _) in held.escrows.iter().chain(&given.escrows) {
                if let Some(x) = d.side(r, *a) {
                    if !all.iter().any(|(_, y)| *y == x) {
                        all.push((*r, x));
                    }
                }
            }
            match (all.iter().find(|s| s.1), all.iter().find(|s| !s.1)) {
                (Some(ex), Some(ey)) if before => crossed(d, roy, &ex.0, &ey.0),
                _ => listed(d, &all),
            }
        }
    }
}

#[cfg(feature = "freenet-main-contract")]
mod contract {
    use super::{check, merge, Deal, Royalty};
    use craftworks_token_contract::escrow::Section;
    use craftworks_token_contract::Definition;
    use freenet_stdlib::prelude::*;

    pub struct Swap;

    fn height(b: &[u8]) -> Option<u64> {
        craftworks_beacon_contract::wire::decode(b).map(|s| s.height())
    }
    fn decode(b: &[u8]) -> Option<Section> {
        Section::decode_prefix(b).filter(|(_, rest)| rest.is_empty()).map(|(s, _)| s)
    }
    /// The two tokens' definition ids (what a crossing reads its royalties from).
    fn defs(d: &Deal) -> Vec<ContractInstanceId> {
        let mut v = vec![d.x.definition];
        if d.y.definition != d.x.definition {
            v.push(d.y.definition);
        }
        v.into_iter().map(ContractInstanceId::new).collect()
    }
    /// Both tokens' royalties from their definitions' states (`None`: one not there or not a definition).
    fn royalties(d: &Deal, got: &dyn Fn(&[u8; 32]) -> Option<Vec<u8>>) -> Option<(Royalty, Royalty)> {
        let roy = |t: &[u8; 32]| got(t).and_then(|b| Definition::decode(&b)).map(|x| x.royalty);
        Some((roy(&d.x.definition)?, roy(&d.y.definition)?))
    }

    #[contract]
    impl ContractInterface for Swap {
        fn validate_state(parameters: Parameters<'static>, state: State<'static>, related: RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
            let (Some(d), Some(s)) = (Deal::decode(parameters.as_ref()), decode(state.as_ref())) else { return Ok(ValidateResult::Invalid) };
            // A REFUND needs the beacon past the deadline (it only grows: valid then, valid after).
            // A CROSSING needs both tokens' definitions (its royalty legs).
            let refund = !s.refunds.is_empty() && s.pays.is_empty();
            let got = |id: &[u8; 32]| related.states().find(|(x, _)| x.as_bytes() == id).and_then(|(_, b)| b.as_ref().map(|b| b.as_ref().to_vec()));
            let now = if refund {
                match related.states().find(|(id, _)| id.as_bytes() == d.beacon) {
                    None => return Ok(ValidateResult::RequestRelated(vec![ContractInstanceId::new(d.beacon)])),
                    Some((_, b)) => b.as_ref().and_then(|b| height(b.as_ref())).unwrap_or(0),
                }
            } else {
                0
            };
            let roy = if s.pays.is_empty() {
                (None, None)
            } else {
                let missing: Vec<ContractInstanceId> = defs(&d).into_iter().filter(|c| !related.states().any(|(x, _)| x == c)).collect();
                if !missing.is_empty() {
                    return Ok(ValidateResult::RequestRelated(missing));
                }
                match royalties(&d, &got) {
                    Some(r) => r,
                    None => return Ok(ValidateResult::Invalid),
                }
            };
            Ok(if check(&d, roy, &s, now).is_ok() { ValidateResult::Valid } else { ValidateResult::Invalid })
        }

        fn update_state(parameters: Parameters<'static>, state: State<'static>, data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
            let d = Deal::decode(parameters.as_ref()).ok_or(ContractError::InvalidState)?;
            let held = decode(state.as_ref()).ok_or(ContractError::InvalidState)?;
            let mut given: Vec<Section> = Vec::new();
            let mut beacon: Option<u64> = None;
            let mut rel: Vec<([u8; 32], Vec<u8>)> = Vec::new();
            for x in data {
                let b: &[u8] = match &x {
                    UpdateData::State(s) => s.as_ref(),
                    UpdateData::Delta(x) => x.as_ref(),
                    UpdateData::StateAndDelta { state, .. } => state.as_ref(),
                    UpdateData::RelatedState { related_to, state } => {
                        if related_to.as_bytes() == d.beacon {
                            beacon = height(state.as_ref());
                        } else if let Ok(id) = <[u8; 32]>::try_from(related_to.as_bytes()) {
                            rel.push((id, state.as_ref().to_vec()));
                        }
                        continue;
                    }
                    _ => continue,
                };
                given.push(decode(b).ok_or(ContractError::InvalidUpdate)?);
            }
            let roy = royalties(&d, &|id| rel.iter().find(|x| x.0 == *id).map(|x| x.1.clone()));
            let (Some(now), Some(roy)) = (beacon, roy) else {
                let all = std::iter::once(ContractInstanceId::new(d.beacon)).chain(defs(&d));
                return UpdateModification::requires(all.map(|c| RelatedContract { contract_instance_id: c, mode: RelatedMode::StateOnce }).collect());
            };
            let mut out = held;
            for g in &given {
                check(&d, roy, g, now).map_err(|w| ContractError::InvalidUpdateWithInfo { reason: w.into() })?;
                out = merge(&d, roy, &out, g, now);
            }
            Ok(UpdateModification::valid(State::from(out.encode())))
        }

        fn summarize_state(_p: Parameters<'static>, state: State<'static>) -> Result<StateSummary<'static>, ContractError> {
            Ok(StateSummary::from(blake3::hash(state.as_ref()).as_bytes().to_vec()))
        }

        fn get_state_delta(_p: Parameters<'static>, state: State<'static>, summary: StateSummary<'static>) -> Result<StateDelta<'static>, ContractError> {
            Ok(StateDelta::from(if summary.as_ref() == blake3::hash(state.as_ref()).as_bytes() { Vec::new() } else { state.as_ref().to_vec() }))
        }
    }
}

#[cfg(test)]
mod tests;
