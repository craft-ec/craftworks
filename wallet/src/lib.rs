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

/// What a wallet READ for collecting (W2): the chains of this token it looked in (its counterparties', from the
/// token's activity — the caller's to find), the programs it holds escrows with or is paid by, the supply shards and
/// records it was minted in.
pub struct Sources<'a> {
    /// Others' chains of this token, by their owners.
    pub chains: &'a [([u8; 32], State)],
    /// Programs' states, by their ids.
    pub programs: &'a [([u8; 32], Vec<u8>)],
    /// Supply shards / records of this token.
    pub shards: &'a [t::supply::Shard],
}

impl Wallet<'_> {
    /// Every CREDIT `h` is owed and can take now (`now`: the beacon's height) — each only once FINAL (the margin past
    /// what it cites, its step's or delay's time come), never twice (what the chain already took, or this list). Each
    /// with its work; checked against the token's shape rules. (Names a CUT of this chain may hold — stamped at or
    /// before it — need its absence proofs: W3.)
    pub fn collect(&self, h: &Held, src: &Sources, now: u64) -> Result<Vec<Entry>, Why> {
        let (p, d) = (&h.params, &h.definition);
        let m = d.margin;
        let mut s = h.state.clone();
        if s.fork.is_some() {
            return Err("a forked chain");
        }
        let cut_at = s.entries.first().filter(|e| matches!(e.body, Body::Cut { .. })).map(|e| e.stamp);
        let behind = |at: u64| cut_at.is_some_and(|c| at <= c);
        let mut taken: Vec<Vec<u8>> = t::cut::received_keys(&s.entries);
        let mut out: Vec<Entry> = Vec::new();
        let stamp = now.max(s.entries.last().map_or(0, |e| e.stamp));
        let push = |s: &mut State, body: Body, plus: u128, out: &mut Vec<Entry>| {
            let seq = s.entries.last().map_or(0, |e| e.seq + 1);
            let mut e = Entry { seq, prev: s.head(), nonce: nonce_of(p, seq), stamp, body, balance: s.balance() + plus, auth: t::Auth::Work(0) };
            let msg = e.message();
            e.auth = t::Auth::Work((0u64..).find(|w| t::work_ok(&msg, *w)).unwrap());
            s.entries.push(e.clone());
            out.push(e);
        };
        let ev = t::Evidence::default;
        // RECEIVES and VESTING STEPS from others' chains; FEES if this account is the collector.
        let collector = d.transfer_fee.is_some_and(|(_, _, c)| c == p.owner);
        for (from, c) in src.chains {
            if c.fork.is_some() {
                continue;
            }
            for x in &c.entries {
                if x.stamp.saturating_add(m) > stamp || behind(x.stamp) {
                    continue;
                }
                match &x.body {
                    Body::Send { to, amount } | Body::Spend { to, amount } if *to == p.owner => {
                        let k = t::received_key(t::RECEIVED, from, &x.nonce);
                        if !taken.contains(&k) {
                            taken.push(k);
                            push(&mut s, Body::Receive { from: *from, nonce: x.nonce, amount: *amount, cited: x.stamp, ev: ev() }, *amount, &mut out);
                        }
                    }
                    Body::Vest { to, schedule } if *to == p.owner => {
                        for (i, (at, amount)) in schedule.iter().enumerate() {
                            let k = t::vested_key(from, &x.nonce, i as u32);
                            if *at <= stamp && !taken.contains(&k) {
                                taken.push(k);
                                push(&mut s, Body::Vested { from: *from, vest: x.nonce, step: i as u32, amount: *amount, at: *at, cited: x.stamp, ev: ev() }, *amount, &mut out);
                            }
                        }
                    }
                    _ => {}
                }
                if collector {
                    let gross = match &x.body {
                        Body::Send { amount, .. } | Body::Spend { amount, .. } | Body::Escrow { amount, .. } => Some(*amount),
                        Body::Vest { schedule, .. } => schedule.iter().try_fold(0u128, |a, s| a.checked_add(s.1)),
                        _ => None,
                    };
                    let fee = gross.map_or(0, |g| t::transfer_fee(d, from, g));
                    let k = t::received_key(t::FEE, from, &x.nonce);
                    if fee > 0 && !taken.contains(&k) {
                        taken.push(k);
                        push(&mut s, Body::FeeClaim { from: *from, nonce: x.nonce, gross: gross.unwrap(), amount: fee, cited: x.stamp, ev: ev() }, fee, &mut out);
                    }
                }
            }
        }
        // RELEASES and PAYOUTS by programs' verdicts; RECLAIMS of escrows no program took, from their time on.
        let open: Vec<([u8; 16], u128, [u8; 32], u64)> = {
            let mut o = Vec::new();
            for x in &s.entries {
                match &x.body {
                    Body::Escrow { program, amount, until, .. } => o.push((x.nonce, *amount, *program, *until)),
                    Body::Unescrow { escrow, .. } | Body::Reclaim { escrow } => o.retain(|e: &([u8; 16], u128, [u8; 32], u64)| e.0 != *escrow),
                    _ => {}
                }
            }
            o
        };
        for (n, amount, prog, until) in open {
            let sec = src.programs.iter().find(|x| x.0 == prog).and_then(|x| t::escrow::Section::decode_prefix(&x.1)).map(|x| x.0);
            let mine = (p.tok(), p.owner, n);
            match sec.as_ref().and_then(|sc| sc.refund_of(&mine).map(|r| (r, sc.hash()))) {
                Some((refund, verdict)) => push(&mut s, Body::Unescrow { escrow: n, refund, verdict }, refund, &mut out),
                None if until > 0 && stamp >= until && !sec.is_some_and(|sc| sc.escrows.iter().any(|(r, ..)| *r == mine)) => push(&mut s, Body::Reclaim { escrow: n }, amount, &mut out),
                None => {}
            }
        }
        for (prog, state) in src.programs {
            let Some((sec, _)) = t::escrow::Section::decode_prefix(state) else { continue };
            for (key, tok, to, amount) in &sec.pays {
                let k = t::received_key(t::PAYOUT, prog, key);
                if *tok == p.tok() && *to == p.owner && !taken.contains(&k) && cut_at.is_none() {
                    taken.push(k);
                    push(&mut s, Body::Payout { program: *prog, key: *key, amount: *amount, verdict: sec.hash(), ev: ev() }, *amount, &mut out);
                }
            }
        }
        // MINTS to this owner (of this chain's item) in the shards read.
        for sh in src.shards {
            for mt in t::supply::admitted(d, sh) {
                if mt.to != p.owner || behind(mt.stamp) || mt.stamp > stamp {
                    continue;
                }
                // A NUMBERED copy: this chain's, if its number is in the mint's range (one copy).
                if d.ids == t::Ids::Numbered {
                    let first = u128::from_be_bytes(mt.nonce);
                    let Some(n) = (first..first.saturating_add(mt.amount)).find(|n| t::numbered_item(&p.definition, *n) == p.item) else { continue };
                    let k = t::received_key(t::MINTED, &t::MINTER, &n.to_be_bytes());
                    if !taken.contains(&k) {
                        taken.push(k);
                        push(&mut s, Body::CappedMint { nonce: n.to_be_bytes(), amount: 1, minted: mt.stamp, mint: mt.clone(), ev: ev() }, 1, &mut out);
                    }
                    continue;
                }
                let k = t::received_key(t::MINTED, &t::MINTER, &mt.nonce);
                let plain = mt.item == p.item && !d.capped();
                if (plain || (d.capped() && d.ids != t::Ids::Numbered && mt.item == p.item)) && !taken.contains(&k) {
                    taken.push(k);
                    let body = if d.capped() {
                        Body::CappedMint { nonce: mt.nonce, amount: mt.amount, minted: mt.stamp, mint: mt.clone(), ev: ev() }
                    } else {
                        Body::Mint { nonce: mt.nonce, amount: mt.amount, minted: mt.stamp, mint: mt.clone(), ev: ev() }
                    };
                    push(&mut s, body, mt.amount, &mut out);
                }
            }
        }
        // RELEASES of unbonded stake, their delay over.
        let released: Vec<[u8; 16]> = s.entries.iter().filter_map(|x| match &x.body {
            Body::Release { unstake } => Some(*unstake),
            _ => None,
        }).collect();
        for (n, amount, at) in t::unbonding_of(&s) {
            if at <= stamp && !released.contains(&n) {
                push(&mut s, Body::Release { unstake: n }, amount, &mut out);
            }
        }
        if !out.is_empty() {
            t::walk(p, d, &s, &|_, _| None, &|_| None, Some(self.keys), 0, usize::MAX, &t::Extra::NONE).map_err(|(_, w)| w)?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
