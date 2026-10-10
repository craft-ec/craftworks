//! THE WALLET CORE (TOKEN §7c): what an account's device does with the token, as pure functions over what the caller
//! read — the chains it holds, their definitions, the beacon's height — returning the entries to append. It has no keys
//! and no network of its own: the account's DEVICE signs (the delegate holds its key: [`Device`]), and the caller
//! fetches and publishes.
//!
//! - DEBITS ([`Wallet::debit`]): send, spend, escrow, vest, burn, allow, stake, unstake — each the chain's next entry,
//!   its carried balance computed (a transfer fee on top), its name derived from its place, signed by this device; the
//!   device's WRITER entry first when another device of the account (or none) wrote last (TOKEN §1: one writer at a
//!   time — the account decides which device; this core only hands the chain over when told). Every entry built is
//!   checked against the token's own shape rules before it is returned: the wallet never writes what the contract
//!   refuses.

use craftworks_token_contract as t;
use t::{Body, ChainParams, Definition, Entry, Signer, State, Why};

/// This account's DEVICE: its node key (public), its signature, and its CREDENTIAL — the account's owner key vouching
/// for the node key at key seq `key_seq` (the account's key log).
pub trait Device {
    fn public(&self) -> [u8; 32];
    fn sign(&self, message: &[u8]) -> [u8; 64];
    fn credential(&self) -> Vec<u8>;
    fn key_seq(&self) -> u32;
}

/// One chain this account holds: its params, its token's definition, its state as last read.
#[derive(Clone, Debug)]
pub struct Held {
    pub params: ChainParams,
    pub definition: Definition,
    pub state: State,
}

/// The wallet of one account on one device.
pub struct Wallet<'a> {
    pub device: &'a dyn Device,
    /// The account's key log (its owner keys: what a writer credential stands on).
    pub keys: &'a craftworks_idlog_contract::Log,
}

/// An entry's NAME from its place: unique per chain and seq (a transfer's nonce is named once, TOKEN §1e).
pub fn nonce_of(p: &ChainParams, seq: u64) -> [u8; 16] {
    let mut h = blake3::Hasher::new_derive_key("craftworks wallet entry nonce");
    h.update(&p.id());
    h.update(&seq.to_le_bytes());
    h.finalize().as_bytes()[..16].try_into().unwrap()
}

/// What a DEBIT takes from the balance (its transfer fee on top, TOKEN §1e); `None`: not a debit this core builds.
fn taken(d: &Definition, owner: &[u8; 32], body: &Body) -> Option<u128> {
    let gross = |x: u128| x.checked_add(t::transfer_fee(d, owner, x));
    match body {
        Body::Send { amount, .. } | Body::Escrow { amount, .. } => gross(*amount),
        Body::Vest { schedule, .. } => gross(schedule.iter().try_fold(0u128, |a, s| a.checked_add(s.1))?),
        Body::Burn { amount } | Body::Stake { amount } => Some(*amount),
        Body::Allow { .. } | Body::Unstake { .. } => Some(0),
        _ => None,
    }
}

impl Wallet<'_> {
    /// This device as a chain's signer.
    pub fn signer(&self) -> Signer {
        Signer::Device(self.device.public(), self.device.key_seq())
    }

    fn signed(&self, s: &State, nonce: [u8; 16], stamp: u64, body: Body, balance: u128) -> Entry {
        let mut e = Entry { seq: s.entries.last().map_or(0, |e| e.seq + 1), prev: s.head(), nonce, stamp, body, balance, auth: t::Auth::Work(0) };
        e.auth = t::Auth::Sig(self.device.sign(&e.message()));
        e
    }

    /// The entries that put `body` — a debit of this account's — at the end of `h`, stamped `now` (the beacon's height):
    /// this device's WRITER entry first if it is not the chain's writer, then the debit. Checked against the token's
    /// shape rules (refused: why); `h` is left as it was — the caller appends what it publishes.
    pub fn debit(&self, h: &Held, body: Body, now: u64) -> Result<Vec<Entry>, Why> {
        if h.state.fork.is_some() {
            return Err("a forked chain");
        }
        let take = taken(&h.definition, &h.params.owner, &body).ok_or("not a debit this wallet builds")?;
        let mut s = h.state.clone();
        let stamp = now.max(s.entries.last().map_or(0, |e| e.stamp));
        let mut out = Vec::new();
        if t::writer_at(&s, s.entries.len()) != Some(self.signer()) {
            let seq = s.entries.last().map_or(0, |e| e.seq + 1);
            let w = self.signed(&s, nonce_of(&h.params, seq), stamp, Body::Writer { cred: self.device.credential(), key_seq: self.device.key_seq() }, s.balance());
            s.entries.push(w.clone());
            out.push(w);
        }
        let balance = s.balance().checked_sub(take).ok_or("a debit over the balance")?;
        let seq = s.entries.last().map_or(0, |e| e.seq + 1);
        let e = self.signed(&s, nonce_of(&h.params, seq), stamp, body, balance);
        s.entries.push(e.clone());
        out.push(e);
        // The contract's own rules over the chain as it would be (no citations: a debit cites nothing).
        t::walk(&h.params, &h.definition, &s, &|_, _| None, &|_| None, Some(self.keys), 0, usize::MAX, &t::Extra::NONE).map_err(|(_, w)| w)?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
