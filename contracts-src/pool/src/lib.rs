//! A BATCH-AUCTION POOL (TOKEN §5) — an APP over the token's escrow, not part of the token. One contract per batch
//! (params [`Batch`]: the pool's two tokens, its epoch length and grace, the beacon, the batch's epoch). A trader
//! escrows what it gives with THIS contract (an `Escrow` in its own chain, its terms [`terms`]: the side and the limit
//! price), and the order goes into the batch's state. The state is the token's standard escrow SECTION — the orders as
//! escrows, then the verdict — followed by the orders themselves:
//!
//! - until the batch SETTLES (its close plus grace, read from the beacon) it takes orders and holds no verdict;
//! - from then on it takes nothing new, and its verdict is EXACTLY the batch's uniform-price clearing (`clear`): each
//!   order's unspent part back (a refund), what it bought paid out in the other token — less, when the token it SOLD
//!   carries a royalty (its definition's), the creator's fee, paid out beside it.
//!
//! The contract checks all of that itself; the token checks each release against the verdict; witnesses check that
//! every order is a real, accepted escrow on the terms its owner signed. A verdict held is never replaced: two copies
//! settled on different order sets (an order reaching one node only after the settle height — the grace is there so
//! the sets meet before it) each keep their own, and since every release names its verdict, only the one all its
//! escrows are released under is ever paid out.

pub mod clear;

use clear::{Cleared, Side};
use craftworks_token_contract::escrow::{self, EscrowRef, Royalty, Section};
use craftworks_token_contract::Tok;

pub const BATCH: u8 = 0x50;
/// The most orders one batch holds.
pub const ORDERS_MAX: usize = 1024;

/// A batch's PARAMS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Batch {
    pub a: Tok,
    pub b: Tok,
    pub epoch_len: u64,
    pub grace: u64,
    pub beacon: [u8; 32],
    pub epoch: u64,
}

fn put_tok(v: &mut Vec<u8>, t: &Tok) {
    v.extend_from_slice(&t.definition);
    v.extend_from_slice(&t.item);
}

struct R<'a>(&'a [u8]);
impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        (self.0.len() >= n).then(|| {
            let (a, b) = self.0.split_at(n);
            self.0 = b;
            a
        })
    }
    fn arr<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.arr()?))
    }
    fn u128(&mut self) -> Option<u128> {
        Some(u128::from_le_bytes(self.arr()?))
    }
    fn tok(&mut self) -> Option<Tok> {
        Some(Tok { definition: self.arr()?, item: self.arr()? })
    }
}

impl Batch {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = vec![BATCH];
        put_tok(&mut v, &self.a);
        put_tok(&mut v, &self.b);
        v.extend_from_slice(&self.epoch_len.to_le_bytes());
        v.extend_from_slice(&self.grace.to_le_bytes());
        v.extend_from_slice(&self.beacon);
        v.extend_from_slice(&self.epoch.to_le_bytes());
        v
    }
    pub fn decode(b: &[u8]) -> Option<Batch> {
        let mut r = R(b);
        (r.u8()? == BATCH).then_some(())?;
        let p = Batch { a: r.tok()?, b: r.tok()?, epoch_len: r.u64()?, grace: r.u64()?, beacon: r.arr()?, epoch: r.u64()? };
        (r.0.is_empty() && p.a != p.b && p.epoch_len > 0).then_some(p)
    }
    /// Its close (orders stamped before it) and its settle height (close plus grace).
    pub fn window(&self) -> (u64, u64) {
        let close = (self.epoch + 1).saturating_mul(self.epoch_len);
        (close, close.saturating_add(self.grace))
    }
    /// The token an order of `side` gives.
    pub fn gives(&self, side: Side) -> Tok {
        if side == Side::Sell {
            self.a
        } else {
            self.b
        }
    }
}

/// An order's TERMS, as its escrow carries them: its side and its limit price.
pub fn terms(side: Side, limit: u128) -> Vec<u8> {
    [&[if side == Side::Sell { 0 } else { 1 }][..], &limit.to_le_bytes()].concat()
}

/// One ORDER: its escrow, its amount, its side and limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Order {
    pub escrow: EscrowRef,
    pub amount: u128,
    pub side: Side,
    pub limit: u128,
}

/// A batch's STATE: its section (orders as escrows, then the verdict) and its orders, sorted by escrow.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub section: Section,
    pub orders: Vec<Order>,
}

/// The payout key of what an order bought.
pub fn pay_key(r: &EscrowRef) -> [u8; 16] {
    key_of("craftworks pool payout", r)
}
/// The payout key of the creator's fee out of what an order bought.
pub fn fee_key(r: &EscrowRef) -> [u8; 16] {
    escrow::fee_key(r)
}
fn key_of(what: &str, r: &EscrowRef) -> [u8; 16] {
    let mut h = blake3::Hasher::new_derive_key(what);
    h.update(&r.0.definition);
    h.update(&r.0.item);
    h.update(&r.1);
    h.update(&r.2);
    h.finalize().as_bytes()[..16].try_into().unwrap()
}

impl State {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = self.section.encode();
        v.extend_from_slice(b"CWPB");
        v.extend_from_slice(&(self.orders.len() as u32).to_le_bytes());
        for o in &self.orders {
            put_tok(&mut v, &o.escrow.0);
            v.extend_from_slice(&o.escrow.1);
            v.extend_from_slice(&o.escrow.2);
            v.extend_from_slice(&o.amount.to_le_bytes());
            v.push(if o.side == Side::Sell { 0 } else { 1 });
            v.extend_from_slice(&o.limit.to_le_bytes());
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<State> {
        let (section, rest) = Section::decode_prefix(b)?;
        let mut r = R(rest.strip_prefix(b"CWPB")?);
        let n = u32::from_le_bytes(r.arr()?) as usize;
        (n <= ORDERS_MAX).then_some(())?;
        let orders = (0..n)
            .map(|_| {
                let escrow = (r.tok()?, r.arr()?, r.arr()?);
                let amount = r.u128()?;
                let side = match r.u8()? {
                    0 => Side::Sell,
                    1 => Side::Buy,
                    _ => return None,
                };
                Some(Order { escrow, amount, side, limit: r.u128()? })
            })
            .collect::<Option<Vec<_>>>()?;
        r.0.is_empty().then_some(State { section, orders })
    }
    /// Settled: it holds a verdict.
    pub fn settled(&self) -> bool {
        !self.section.refunds.is_empty() || !self.section.pays.is_empty()
    }
}

/// The section a batch's orders make, with NO verdict.
fn listed(orders: &[Order]) -> Vec<(EscrowRef, u128, [u8; 32])> {
    orders.iter().map(|o| (o.escrow, o.amount, escrow::terms_hash(&terms(o.side, o.limit)))).collect()
}

/// The VERDICT of a batch's orders (`roy`: tokens A's and B's royalties): each order's refund, and what it bought paid
/// out in the other token, less the creator's fee of the token it sold (paid out beside it).
pub fn verdict(p: &Batch, roy: (Royalty, Royalty), orders: &[Order]) -> Section {
    let c: Cleared = clear::clear(&orders.iter().map(|o| clear::Order { side: o.side, amount: o.amount, limit: o.limit }).collect::<Vec<_>>());
    let mut s = Section { escrows: listed(orders), ..Default::default() };
    for (o, f) in orders.iter().zip(&c.fills) {
        s.refunds.push((o.escrow, o.amount - f.paid));
        if f.got > 0 {
            let (other, sold) = if o.side == Side::Sell { (p.b, roy.0) } else { (p.a, roy.1) };
            let fee = escrow::royalty_fee(sold, &o.escrow.1, f.got);
            s.pays.push((pay_key(&o.escrow), other, o.escrow.1, f.got - fee.map_or(0, |x| x.1)));
            if let Some((creator, a)) = fee {
                s.pays.push((fee_key(&o.escrow), other, creator, a));
            }
        }
    }
    s
}

pub type Why = &'static str;

/// A batch's state HOLDS: its orders sorted, each in the chain of the token its side gives; its section exactly its
/// orders — with no verdict before the batch settles (`now`: the beacon's height), and exactly the clearing after.
pub fn check(p: &Batch, roy: (Royalty, Royalty), s: &State, now: u64) -> Result<(), Why> {
    if !s.orders.windows(2).all(|w| w[0].escrow < w[1].escrow) {
        return Err("orders not in their order");
    }
    if s.orders.iter().any(|o| o.escrow.0 != p.gives(o.side) || o.limit == 0 || o.amount == 0) {
        return Err("an order in the wrong token, or of nothing");
    }
    if !s.settled() {
        return if s.section == (Section { escrows: listed(&s.orders), ..Default::default() }) { Ok(()) } else { Err("a section that is not its orders") };
    }
    if now < p.window().1 {
        return Err("a verdict before the batch settles");
    }
    if s.section != verdict(p, roy, &s.orders) {
        return Err("a verdict that is not the batch's clearing");
    }
    s.section.holds()
}

/// THE MERGE (an update, the beacon at `now`): before the batch settles, the orders' union (no verdict); from then on,
/// nothing new — a settled copy taken over an unsettled one; a settled one held, kept.
pub fn merge(p: &Batch, held: &State, given: &State, now: u64) -> State {
    let settles = p.window().1;
    if now < settles && !held.settled() && !given.settled() {
        let mut orders: Vec<Order> = held.orders.iter().chain(&given.orders).copied().collect();
        orders.sort_by_key(|x| x.escrow);
        orders.dedup_by(|x, y| x.escrow == y.escrow);
        orders.truncate(ORDERS_MAX);
        return State { section: Section { escrows: listed(&orders), ..Default::default() }, orders };
    }
    // A verdict held is KEPT (as a swap's): two settled copies are a split order set — and a release names its
    // verdict, so only the one every escrow is released under is ever paid (audit 10-10: "more orders wins" let a
    // late order replace a verdict already paid out).
    match (held.settled(), given.settled()) {
        (false, true) if now >= settles => given.clone(),
        _ => held.clone(),
    }
}

#[cfg(feature = "freenet-main-contract")]
mod contract {
    use super::{check, merge, Batch, State as S};
    use craftworks_token_contract::escrow::Royalty;
    use craftworks_token_contract::Definition;
    use freenet_stdlib::prelude::*;

    pub struct Pool;

    fn height(b: &[u8]) -> Option<u64> {
        craftworks_beacon_contract::wire::decode(b).map(|s| s.height())
    }
    /// The two tokens' definition ids (what a verdict reads its royalties from).
    fn defs(p: &Batch) -> Vec<ContractInstanceId> {
        let mut v = vec![p.a.definition];
        if p.b.definition != p.a.definition {
            v.push(p.b.definition);
        }
        v.into_iter().map(ContractInstanceId::new).collect()
    }
    /// Both tokens' royalties from their definitions' states (`None`: one not there or not a definition).
    fn royalties(p: &Batch, got: &dyn Fn(&[u8; 32]) -> Option<Vec<u8>>) -> Option<(Royalty, Royalty)> {
        let roy = |t: &[u8; 32]| got(t).and_then(|b| Definition::decode(&b)).map(|x| x.royalty);
        Some((roy(&p.a.definition)?, roy(&p.b.definition)?))
    }

    #[contract]
    impl ContractInterface for Pool {
        fn validate_state(parameters: Parameters<'static>, state: State<'static>, related: RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
            let (Some(p), Some(s)) = (Batch::decode(parameters.as_ref()), S::decode(state.as_ref())) else { return Ok(ValidateResult::Invalid) };
            // A verdict needs the beacon past the settle height (it only grows: valid then, valid after), and both
            // tokens' definitions (its royalty legs).
            if !s.settled() {
                return Ok(if check(&p, (None, None), &s, 0).is_ok() { ValidateResult::Valid } else { ValidateResult::Invalid });
            }
            let want: Vec<ContractInstanceId> = std::iter::once(ContractInstanceId::new(p.beacon)).chain(defs(&p)).filter(|c| !related.states().any(|(x, _)| x == c)).collect();
            if !want.is_empty() {
                return Ok(ValidateResult::RequestRelated(want));
            }
            let got = |id: &[u8; 32]| related.states().find(|(x, _)| x.as_bytes() == id).and_then(|(_, b)| b.as_ref().map(|b| b.as_ref().to_vec()));
            let now = got(&p.beacon).and_then(|b| height(&b)).unwrap_or(0);
            let Some(roy) = royalties(&p, &got) else { return Ok(ValidateResult::Invalid) };
            Ok(if check(&p, roy, &s, now).is_ok() { ValidateResult::Valid } else { ValidateResult::Invalid })
        }

        fn update_state(parameters: Parameters<'static>, state: State<'static>, data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
            let p = Batch::decode(parameters.as_ref()).ok_or(ContractError::InvalidState)?;
            let held = S::decode(state.as_ref()).ok_or(ContractError::InvalidState)?;
            let mut given: Vec<S> = Vec::new();
            let mut beacon: Option<u64> = None;
            let mut rel: Vec<([u8; 32], Vec<u8>)> = Vec::new();
            for d in data {
                let b: &[u8] = match &d {
                    UpdateData::State(s) => s.as_ref(),
                    UpdateData::Delta(x) => x.as_ref(),
                    UpdateData::StateAndDelta { state, .. } => state.as_ref(),
                    UpdateData::RelatedState { related_to, state } => {
                        if related_to.as_bytes() == p.beacon {
                            beacon = height(state.as_ref());
                        } else if let Ok(id) = <[u8; 32]>::try_from(related_to.as_bytes()) {
                            rel.push((id, state.as_ref().to_vec()));
                        }
                        continue;
                    }
                    _ => continue,
                };
                given.push(S::decode(b).ok_or(ContractError::InvalidUpdate)?);
            }
            let roy = royalties(&p, &|id| rel.iter().find(|x| x.0 == *id).map(|x| x.1.clone()));
            let (Some(now), Some(roy)) = (beacon, roy) else {
                let all = std::iter::once(ContractInstanceId::new(p.beacon)).chain(defs(&p));
                return UpdateModification::requires(all.map(|c| RelatedContract { contract_instance_id: c, mode: RelatedMode::StateOnce }).collect());
            };
            let mut out = held;
            for g in &given {
                check(&p, roy, g, now).map_err(|w| ContractError::InvalidUpdateWithInfo { reason: w.into() })?;
                out = merge(&p, &out, g, now);
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
