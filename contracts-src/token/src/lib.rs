//! TOKEN (docs/TOKEN.md): the standard's DEFINITION and ACCOUNT CHAIN — one code, two kinds by their params. Step 3,
//! the core: `genesis`, `send`, `receive`, `burn`; carried balances; the merge; strict equivocation; work on appends by
//! anyone. (Allowances, offers, locks, ids, the beacon's times: later steps.)
//!
//! An id is Freenet's: `blake3(code hash ‖ params)`, the code hash being `blake3(wasm)`. Every params names the code
//! hash, so a chain recomputes the id of every chain it cites (TOKEN §3a): a look-alike contract's "send" is not a send.
//!
//! WHAT THE CONTRACT PROVES (TOKEN §3a): a chain is WELL-FORMED — linked, signed, its carried balance right, no debit
//! over it, no receive twice — and the receives among its last [`RECENT`] entries are checked against their senders'
//! chains (with the definition: at most 10 related contracts, Freenet's limit). It does not prove an older receive:
//! value is checked by whoever receives it (wallets, checkpoints). Not yet: the received-set ACCUMULATOR (needed only
//! once old entries are pruned — this core keeps every entry and scans them).

use ed25519_dalek::{Signature, VerifyingKey};

pub const DEFINITION: u8 = 1;
pub const CHAIN: u8 = 2;
/// The receives among this many newest entries are checked against their senders (+ the definition: 10 related).
pub const RECENT: usize = 9;
/// Work on an entry anyone may append (a receive): this many leading zero bits of `blake3(message ‖ work)`.
pub const WORK_BITS: u32 = 16;
/// A token's name, at most (its length is one byte on the wire).
pub const NAME_MAX: usize = 64;

pub fn id(code_hash: &[u8; 32], params: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(code_hash);
    h.update(params);
    *h.finalize().as_bytes()
}

// ---- reading bytes, exactly ----
struct R<'a>(&'a [u8]);
impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn arr<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.arr()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.arr()?))
    }
    fn u128(&mut self) -> Option<u128> {
        Some(u128::from_le_bytes(self.arr()?))
    }
    fn done(&self) -> bool {
        self.0.is_empty()
    }
}

/// A token's DEFINITION: its rules, fixed. Its state is its params again (a related read returns state, not params).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub code_hash: [u8; 32],
    pub decimals: u8,
    /// The smallest send (spam costs: TOKEN §2).
    pub min_send: u128,
    /// `fixed(genesis)`: the one chain owner whose seq 0 may be the genesis, and its amount.
    pub genesis_owner: [u8; 32],
    pub genesis_amount: u128,
    pub name: Vec<u8>,
    pub nonce: [u8; 16],
}

impl Definition {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = vec![DEFINITION];
        v.extend_from_slice(&self.code_hash);
        v.push(self.decimals);
        v.extend_from_slice(&self.min_send.to_le_bytes());
        v.extend_from_slice(&self.genesis_owner);
        v.extend_from_slice(&self.genesis_amount.to_le_bytes());
        v.push(self.name.len() as u8);
        v.extend_from_slice(&self.name);
        v.extend_from_slice(&self.nonce);
        v
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        (r.u8()? == DEFINITION).then_some(())?;
        let code_hash = r.arr()?;
        let decimals = r.u8()?;
        let min_send = r.u128()?;
        let genesis_owner = r.arr()?;
        let genesis_amount = r.u128()?;
        let n = r.u8()? as usize;
        let name = r.take(n)?.to_vec();
        let nonce = r.arr()?;
        (r.done() && decimals <= 18 && min_send > 0 && name.len() <= NAME_MAX).then_some(Self { code_hash, decimals, min_send, genesis_owner, genesis_amount, name, nonce })
    }
}

/// An ACCOUNT CHAIN's params: one token, one writer (a device's key: TOKEN §1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainParams {
    pub code_hash: [u8; 32],
    pub definition: [u8; 32],
    pub owner: [u8; 32],
}

impl ChainParams {
    pub fn encode(&self) -> Vec<u8> {
        [&[CHAIN][..], &self.code_hash, &self.definition, &self.owner].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        let mut r = R(b);
        (r.u8()? == CHAIN).then_some(())?;
        let p = Self { code_hash: r.arr()?, definition: r.arr()?, owner: r.arr()? };
        r.done().then_some(p)
    }
    pub fn id(&self) -> [u8; 32] {
        id(&self.code_hash, &self.encode())
    }
    /// The chain of `owner` for the same token (whose sends this chain cites).
    pub fn of(&self, owner: [u8; 32]) -> Self {
        Self { owner, ..*self }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    /// seq 0 of the definition's genesis owner only: the whole fixed supply.
    Genesis { amount: u128 },
    /// A debit to `to`'s chain (named by its owner key).
    Send { to: [u8; 32], amount: u128 },
    /// A credit citing the send `nonce` in `from`'s chain — appended by ANYONE (with work).
    Receive { from: [u8; 32], nonce: [u8; 16], amount: u128 },
    /// A debit with no credit anywhere.
    Burn { amount: u128 },
    /// The owner lets `spender` debit up to `limit` (replacing any earlier allowance; 0 revokes) — ERC-20 `approve`.
    Allow { spender: [u8; 32], limit: u128 },
    /// A debit to `to`'s chain written and signed by a SPENDER within their allowance — ERC-20 `transferFrom`.
    Spend { to: [u8; 32], amount: u128 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Auth {
    /// The chain owner's signature over the entry's message.
    Sig([u8; 64]),
    /// Another key's signature (a spender's, within their allowance): `by` signed it.
    Delegated { by: [u8; 32], sig: [u8; 64] },
    /// Work: `blake3(message ‖ work)` has [`WORK_BITS`] leading zero bits.
    Work(u64),
}

/// One ENTRY: its place, its link, its name, what it does, the BALANCE after it, and who vouches for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub seq: u64,
    pub prev: [u8; 32],
    /// Its NAME, kept if it is re-appended at another seq (TOKEN §3).
    pub nonce: [u8; 16],
    pub body: Body,
    pub balance: u128,
    pub auth: Auth,
}

impl Entry {
    /// What is signed or worked on: everything but the auth.
    pub fn message(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(120);
        v.extend_from_slice(&self.seq.to_le_bytes());
        v.extend_from_slice(&self.prev);
        v.extend_from_slice(&self.nonce);
        match &self.body {
            Body::Genesis { amount } => {
                v.push(0);
                v.extend_from_slice(&amount.to_le_bytes());
            }
            Body::Send { to, amount } => {
                v.push(1);
                v.extend_from_slice(to);
                v.extend_from_slice(&amount.to_le_bytes());
            }
            Body::Receive { from, nonce, amount } => {
                v.push(2);
                v.extend_from_slice(from);
                v.extend_from_slice(nonce);
                v.extend_from_slice(&amount.to_le_bytes());
            }
            Body::Burn { amount } => {
                v.push(3);
                v.extend_from_slice(&amount.to_le_bytes());
            }
            Body::Allow { spender, limit } => {
                v.push(4);
                v.extend_from_slice(spender);
                v.extend_from_slice(&limit.to_le_bytes());
            }
            Body::Spend { to, amount } => {
                v.push(5);
                v.extend_from_slice(to);
                v.extend_from_slice(&amount.to_le_bytes());
            }
        }
        v.extend_from_slice(&self.balance.to_le_bytes());
        v
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut v = self.message();
        match &self.auth {
            Auth::Sig(s) => {
                v.push(0);
                v.extend_from_slice(s);
            }
            Auth::Work(w) => {
                v.push(1);
                v.extend_from_slice(&w.to_le_bytes());
            }
            Auth::Delegated { by, sig } => {
                v.push(2);
                v.extend_from_slice(by);
                v.extend_from_slice(sig);
            }
        }
        v
    }
    fn read(r: &mut R) -> Option<Self> {
        let seq = r.u64()?;
        let prev = r.arr()?;
        let nonce = r.arr()?;
        let body = match r.u8()? {
            0 => Body::Genesis { amount: r.u128()? },
            1 => Body::Send { to: r.arr()?, amount: r.u128()? },
            2 => Body::Receive { from: r.arr()?, nonce: r.arr()?, amount: r.u128()? },
            3 => Body::Burn { amount: r.u128()? },
            4 => Body::Allow { spender: r.arr()?, limit: r.u128()? },
            5 => Body::Spend { to: r.arr()?, amount: r.u128()? },
            _ => return None,
        };
        let balance = r.u128()?;
        let auth = match r.u8()? {
            0 => Auth::Sig(r.arr()?),
            1 => Auth::Work(r.u64()?),
            2 => Auth::Delegated { by: r.arr()?, sig: r.arr()? },
            _ => return None,
        };
        Some(Self { seq, prev, nonce, body, balance, auth })
    }
    pub fn hash(&self) -> [u8; 32] {
        *blake3::hash(&self.encode()).as_bytes()
    }
    /// Written by the chain's writer (signed), not by anyone (worked).
    pub fn by_owner(&self) -> bool {
        matches!(self.auth, Auth::Sig(_))
    }
    fn signed_by(&self, owner: &[u8; 32]) -> bool {
        let Auth::Sig(s) = &self.auth else { return false };
        sig_ok(owner, &self.message(), s)
    }
    /// Signed by `key` as a delegate (a spender): its signature, and its key named.
    fn delegated_by(&self, key: &[u8; 32]) -> bool {
        let Auth::Delegated { by, sig } = &self.auth else { return false };
        by == key && sig_ok(by, &self.message(), sig)
    }
    fn worked(&self) -> bool {
        let Auth::Work(w) = self.auth else { return false };
        work_ok(&self.message(), w)
    }
}

fn sig_ok(key: &[u8; 32], message: &[u8], sig: &[u8; 64]) -> bool {
    VerifyingKey::from_bytes(key).is_ok_and(|k| k.verify_strict(message, &Signature::from_bytes(sig)).is_ok())
}

pub fn work_ok(message: &[u8], work: u64) -> bool {
    let mut h = blake3::Hasher::new();
    h.update(message);
    h.update(&work.to_le_bytes());
    let d = h.finalize();
    let b = d.as_bytes();
    u128::from_be_bytes(b[..16].try_into().unwrap()).leading_zeros() >= WORK_BITS
}

/// A chain's STATE: its entries — or, once its writer signed two at one seq, the entries before and the two
/// (EQUIVOCATION, TOKEN §3b: the chain is frozen; every debit after the fork is void).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct State {
    pub entries: Vec<Entry>,
    pub fork: Option<(Entry, Entry)>,
}

impl State {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = (self.entries.len() as u32).to_le_bytes().to_vec();
        let put = |v: &mut Vec<u8>, e: &Entry| {
            let b = e.encode();
            v.extend_from_slice(&(b.len() as u32).to_le_bytes());
            v.extend_from_slice(&b);
        };
        for e in &self.entries {
            put(&mut v, e);
        }
        match &self.fork {
            None => v.push(0),
            Some((a, b)) => {
                v.push(1);
                put(&mut v, a);
                put(&mut v, b);
            }
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.is_empty() {
            return Some(Self::default());
        }
        let mut r = R(b);
        let read = |r: &mut R| -> Option<Entry> {
            let n = r.u32()? as usize;
            let mut one = R(r.take(n)?);
            let e = Entry::read(&mut one)?;
            one.done().then_some(e)
        };
        let n = r.u32()? as usize;
        if n > b.len() {
            return None;
        }
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            entries.push(read(&mut r)?);
        }
        let fork = match r.u8()? {
            0 => None,
            1 => Some((read(&mut r)?, read(&mut r)?)),
            _ => return None,
        };
        r.done().then_some(Self { entries, fork })
    }
    pub fn head(&self) -> [u8; 32] {
        self.entries.last().map_or([0; 32], Entry::hash)
    }
    pub fn balance(&self) -> u128 {
        self.entries.last().map_or(0, |e| e.balance)
    }
}

/// The sender chains this chain must be shown (the receives among its last [`RECENT`] entries), as their owners.
pub fn senders(s: &State) -> Vec<[u8; 32]> {
    let mut out: Vec<[u8; 32]> = Vec::new();
    for e in s.entries.iter().rev().take(RECENT) {
        if let Body::Receive { from, .. } = e.body {
            if !out.contains(&from) {
                out.push(from);
            }
        }
    }
    out
}

/// Why a chain is refused (named, for the tests and the node's log).
pub type Why = &'static str;

/// THE LOCAL RULES (TOKEN §3a) of chain `p` for definition `d`; `sender` gives the chain of a sender named in a recent
/// receive (`None`: it does not exist).
pub fn check(p: &ChainParams, d: &Definition, s: &State, sender: &dyn Fn(&[u8; 32]) -> Option<State>) -> Result<(), Why> {
    if d.code_hash != p.code_hash {
        return Err("the definition is not this standard's");
    }
    let mut balance: u128 = 0;
    let mut prev = [0u8; 32];
    let mut received: Vec<([u8; 32], [u8; 16])> = Vec::new();
    // The ALLOWANCES as they stand (spender → what is left), from the entries so far.
    let mut allowed: Vec<([u8; 32], u128)> = Vec::new();
    let recent_from = s.entries.len().saturating_sub(RECENT);
    for (i, e) in s.entries.iter().enumerate() {
        if e.seq != i as u64 {
            return Err("a seq out of place");
        }
        if e.prev != prev {
            return Err("a broken link");
        }
        balance = match &e.body {
            Body::Genesis { amount } => {
                if i != 0 || p.owner != d.genesis_owner || *amount != d.genesis_amount {
                    return Err("a genesis that is not the definition's");
                }
                if !e.signed_by(&p.owner) {
                    return Err("an entry not signed by the chain's writer");
                }
                *amount
            }
            Body::Send { to, amount } => {
                if !e.signed_by(&p.owner) {
                    return Err("an entry not signed by the chain's writer");
                }
                if *to == p.owner {
                    return Err("a send to itself");
                }
                if *amount < d.min_send {
                    return Err("a send under the minimum");
                }
                balance.checked_sub(*amount).ok_or("a debit over the balance")?
            }
            Body::Burn { amount } => {
                if !e.signed_by(&p.owner) {
                    return Err("an entry not signed by the chain's writer");
                }
                if *amount == 0 {
                    return Err("an empty burn");
                }
                balance.checked_sub(*amount).ok_or("a debit over the balance")?
            }
            Body::Allow { spender, limit } => {
                if !e.signed_by(&p.owner) {
                    return Err("an entry not signed by the chain's writer");
                }
                if *spender == p.owner {
                    return Err("an allowance to itself");
                }
                allowed.retain(|(k, _)| k != spender);
                if *limit > 0 {
                    allowed.push((*spender, *limit));
                }
                balance
            }
            Body::Spend { to, amount } => {
                let Auth::Delegated { by, .. } = &e.auth else { return Err("a spend not signed by a spender") };
                if !e.delegated_by(by) {
                    return Err("a spend not signed by a spender");
                }
                let left = allowed.iter_mut().find(|(k, _)| k == by).ok_or("a spend with no allowance")?;
                left.1 = left.1.checked_sub(*amount).ok_or("a spend over its allowance")?;
                if *to == p.owner {
                    return Err("a send to itself");
                }
                if *amount < d.min_send {
                    return Err("a send under the minimum");
                }
                balance.checked_sub(*amount).ok_or("a debit over the balance")?
            }
            Body::Receive { from, nonce, amount } => {
                // (Never from itself: a send to itself is refused — and asking for its own chain as a related
                // contract is an error at the node, not a refusal.)
                if *from == p.owner {
                    return Err("a receive from itself");
                }
                if !e.worked() {
                    return Err("a receive without its work");
                }
                if *amount < d.min_send {
                    return Err("a receive under the minimum");
                }
                if received.contains(&(*from, *nonce)) {
                    return Err("a send received twice");
                }
                received.push((*from, *nonce));
                if i >= recent_from {
                    let chain = sender(from).ok_or("the sender's chain is not there")?;
                    // (A frozen sender's entries stop at its fork: a send after it — in the fork — is never found
                    // here, so every debit after an equivocation is void, TOKEN §3b.)
                    let found = chain.entries.iter().any(|x| {
                        x.nonce == *nonce && matches!(x.body, Body::Send { to, amount: a } | Body::Spend { to, amount: a } if to == p.owner && a == *amount)
                    });
                    if !found {
                        return Err("a receive whose send is not in its sender's chain");
                    }
                }
                balance.checked_add(*amount).ok_or("a balance overflow")?
            }
        };
        if e.balance != balance {
            return Err("a carried balance that does not add up");
        }
        prev = e.hash();
    }
    if let Some((a, b)) = &s.fork {
        let n = s.entries.len() as u64;
        if a.seq != n || b.seq != n || a.prev != prev || b.prev != prev || a == b || a.hash() >= b.hash() {
            return Err("a fork that is not two writer entries at the next seq, in order");
        }
        if !a.signed_by(&p.owner) || !b.signed_by(&p.owner) {
            return Err("a fork not signed twice by the chain's writer");
        }
    }
    Ok(())
}

/// THE MERGE (TOKEN §3, §3b): the same result in any order. A chain is a list of CELLS — its entries, then (frozen)
/// its fork. Where two copies first differ: an entry against nothing (one copy longer) — the longer; two entries by the
/// WRITER — equivocation, the chain frozen there with both; two entries not both the writer's — the lower hash, with
/// its branch; a fork against anything at that seq — the fork stands, with the two lowest writer entries known there
/// (a frozen chain stays frozen; the earliest fork wins).
pub fn merge(a: &State, b: &State) -> State {
    enum Cell<'a> {
        E(&'a Entry),
        F(&'a Entry, &'a Entry),
    }
    fn cell(s: &State, i: usize) -> Option<Cell<'_>> {
        if i < s.entries.len() {
            Some(Cell::E(&s.entries[i]))
        } else if i == s.entries.len() {
            s.fork.as_ref().map(|(x, y)| Cell::F(x, y))
        } else {
            None
        }
    }
    let len = |s: &State| s.entries.len() + s.fork.is_some() as usize;
    let same = |x: &Option<Cell>, y: &Option<Cell>| match (x, y) {
        (Some(Cell::E(p)), Some(Cell::E(q))) => p == q,
        (Some(Cell::F(p1, p2)), Some(Cell::F(q1, q2))) => p1 == q1 && p2 == q2,
        (None, None) => true,
        _ => false,
    };
    let Some(i) = (0..len(a).max(len(b))).find(|&i| !same(&cell(a, i), &cell(b, i))) else { return a.clone() };
    let frozen = |prefix: &[Entry], mut writers: Vec<Entry>| {
        writers.sort_by_key(Entry::hash);
        writers.dedup();
        State { entries: prefix.to_vec(), fork: Some((writers[0].clone(), writers[1].clone())) }
    };
    match (cell(a, i), cell(b, i)) {
        (Some(_), None) => a.clone(),
        (None, Some(_)) => b.clone(),
        (Some(Cell::E(x)), Some(Cell::E(y))) if x.by_owner() && y.by_owner() => frozen(&a.entries[..i], vec![x.clone(), y.clone()]),
        (Some(Cell::E(x)), Some(Cell::E(y))) => if x.hash() < y.hash() { a.clone() } else { b.clone() },
        (Some(Cell::F(x1, x2)), Some(Cell::F(y1, y2))) => frozen(&a.entries[..i], vec![x1.clone(), x2.clone(), y1.clone(), y2.clone()]),
        (Some(Cell::F(x1, x2)), Some(Cell::E(e))) | (Some(Cell::E(e)), Some(Cell::F(x1, x2))) => {
            let mut w = vec![x1.clone(), x2.clone()];
            if e.by_owner() {
                w.push(e.clone());
            }
            frozen(&a.entries[..i], w)
        }
        (None, None) => a.clone(),
    }
}

#[cfg(feature = "freenet-main-contract")]
mod contract {
    use super::{check, id, merge, senders, ChainParams, Definition, DEFINITION};
    use freenet_stdlib::prelude::*;

    pub struct Token;

    fn related_state(related: &RelatedContracts<'static>, want: &[u8; 32]) -> Option<Option<Vec<u8>>> {
        related.states().find(|(id, _)| id.as_bytes() == want).map(|(_, s)| s.as_ref().map(|s| s.as_ref().to_vec()))
    }

    #[contract]
    impl ContractInterface for Token {
        fn validate_state(parameters: Parameters<'static>, state: State<'static>, related: RelatedContracts<'static>) -> Result<ValidateResult, ContractError> {
            let params = parameters.as_ref();
            // A DEFINITION: its state is its params, and they read as one.
            if params.first() == Some(&DEFINITION) {
                return Ok(if state.as_ref() == params && Definition::decode(params).is_some() { ValidateResult::Valid } else { ValidateResult::Invalid });
            }
            let Some(p) = ChainParams::decode(params) else { return Ok(ValidateResult::Invalid) };
            let Some(s) = super::State::decode(state.as_ref()) else { return Ok(ValidateResult::Invalid) };
            // What it must be shown: its definition and the chains of its recent receives' senders — asked ALL AT ONCE
            // (one round only).
            if super::senders(&s).contains(&p.owner) {
                return Ok(ValidateResult::Invalid);
            }
            let mut want = vec![p.definition];
            want.extend(senders(&s).into_iter().map(|o| p.of(o).id()));
            let missing: Vec<ContractInstanceId> = want.iter().filter(|w| related_state(&related, w).is_none()).map(|w| ContractInstanceId::new(*w)).collect();
            if !missing.is_empty() {
                return Ok(ValidateResult::RequestRelated(missing));
            }
            let Some(Some(db)) = related_state(&related, &p.definition) else { return Ok(ValidateResult::Invalid) };
            if id(&p.code_hash, &db) != p.definition {
                return Ok(ValidateResult::Invalid);
            }
            let Some(d) = Definition::decode(&db) else { return Ok(ValidateResult::Invalid) };
            let sender = |owner: &[u8; 32]| related_state(&related, &p.of(*owner).id()).flatten().and_then(|b| super::State::decode(&b));
            Ok(if check(&p, &d, &s, &sender).is_ok() { ValidateResult::Valid } else { ValidateResult::Invalid })
        }

        fn update_state(parameters: Parameters<'static>, state: State<'static>, data: Vec<UpdateData<'static>>) -> Result<UpdateModification<'static>, ContractError> {
            if parameters.as_ref().first() == Some(&DEFINITION) {
                return Ok(UpdateModification::valid(state));
            }
            let mut held = super::State::decode(state.as_ref()).ok_or(ContractError::InvalidState)?;
            for d in data {
                let b: &[u8] = match &d {
                    UpdateData::State(s) => s.as_ref(),
                    UpdateData::Delta(x) => x.as_ref(),
                    UpdateData::StateAndDelta { state, .. } => state.as_ref(),
                    _ => continue,
                };
                let given = super::State::decode(b).ok_or(ContractError::InvalidUpdate)?;
                held = merge(&held, &given);
            }
            Ok(UpdateModification::valid(State::from(held.encode())))
        }

        fn summarize_state(_parameters: Parameters<'static>, state: State<'static>) -> Result<StateSummary<'static>, ContractError> {
            Ok(StateSummary::from(blake3::hash(state.as_ref()).as_bytes().to_vec()))
        }

        fn get_state_delta(_parameters: Parameters<'static>, state: State<'static>, summary: StateSummary<'static>) -> Result<StateDelta<'static>, ContractError> {
            // The whole chain, unless they hold exactly this one (chains are small in this core).
            Ok(StateDelta::from(if summary.as_ref() == blake3::hash(state.as_ref()).as_bytes() { Vec::new() } else { state.as_ref().to_vec() }))
        }
    }
}

#[cfg(test)]
mod tests;
