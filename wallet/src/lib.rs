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
//! - COLLECTING ([`Wallet::collect`], W2) and CUTS ([`Wallet::cut`], W3): see each.

pub mod chained;
pub mod queue;
pub mod registry;
pub mod stealth;

use craftworks_token_contract as t;
use t::{Body, ChainParams, Definition, Entry, Signer, State, Why};

/// This account's DEVICE: its node key (public), its signature, and its CREDENTIAL — the account's owner key vouching
/// for the node key at key seq `key_seq` (the account's key log).
pub trait Device {
    fn public(&self) -> [u8; 32];
    fn sign(&self, message: &[u8]) -> [u8; 64];
    fn credential(&self) -> Vec<u8>;
    fn key_seq(&self) -> u32;
    /// A device that IS its chain's owner key (a stealth one-time key, TOKEN §7f): its 1-of-1 key set — the chain is
    /// handed over with `Members`, signed as the set. `None` (every DID's device): a `Writer` entry, signed as a device.
    fn set(&self) -> Option<t::authority::KeySet> {
        None
    }
}

/// One chain this account holds: its params, its token's definition, its state as last read — and the BLOCKS of its
/// cuts' trees (its history and received set: what it publishes, and proves its later credits from).
#[derive(Clone, Debug)]
pub struct Held {
    pub params: ChainParams,
    pub definition: Definition,
    pub state: State,
    pub blocks: t::cut::Blocks,
}

impl Held {
    /// The chain WHOLE, seq 0 on: its cut's history (from its blocks) and its live entries.
    pub fn whole(&self) -> Result<State, Why> {
        t::cut::whole(&self.state, &|c| self.blocks.get(c).cloned()).map_err(|_| "a history whose blocks are not held")
    }
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
        match self.device.set() {
            Some(set) => Signer::Set(set, 0),
            None => Signer::Device(self.device.public(), self.device.key_seq()),
        }
    }

    fn writer(&self, s: &State, p: &ChainParams, stamp: u64) -> Entry {
        let seq = s.entries.last().map_or(0, |e| e.seq + 1);
        let body = match self.device.set() {
            Some(set) => Body::Members { set, set_seq: 0 },
            None => Body::Writer { cred: self.device.credential(), key_seq: self.device.key_seq() },
        };
        self.signed(s, nonce_of(p, seq), stamp, body, s.balance())
    }

    /// A CUT of `h` (W3): this device's writer entry first if need be, then the cut — signed, its trees' blocks returned
    /// (to publish, and to keep: the chain's `blocks` from now on — the new trees hold the whole history, the old cut's
    /// blocks are needed no more). The caller appends the
    /// entries and then prunes (`t::prune`). Refused while an escrow is open or the chain is frozen, as the contract
    /// refuses.
    pub fn cut(&self, h: &Held, now: u64) -> Result<(Vec<Entry>, t::cut::Blocks), Why> {
        let mut whole = h.whole()?;
        let stamp = now.max(whole.entries.last().map_or(0, |e| e.stamp));
        let mut out = Vec::new();
        if t::writer_at(&whole, whole.entries.len()) != Some(self.signer()) {
            let w = self.writer(&whole, &h.params, stamp);
            whole.entries.push(w.clone());
            out.push(w);
        }
        let (body, blocks) = t::cut::make(&h.params, &h.definition, &whole, Some(self.keys))?;
        let seq = whole.entries.last().map_or(0, |e| e.seq + 1);
        let e = self.signed(&whole, nonce_of(&h.params, seq), stamp, body, whole.balance());
        whole.entries.push(e.clone());
        out.push(e);
        t::walk(&h.params, &h.definition, &whole, &|_, _| None, &|_| None, Some(self.keys), 0, usize::MAX, &t::Extra::NONE).map_err(|(_, w)| w)?;
        Ok((out, blocks))
    }

    fn signed(&self, s: &State, nonce: [u8; 16], stamp: u64, body: Body, balance: u128) -> Entry {
        let mut e = Entry { seq: s.entries.last().map_or(0, |e| e.seq + 1), prev: s.head(), nonce, stamp, body, balance, auth: t::Auth::Work(0) };
        let sig = self.device.sign(&e.message());
        e.auth = if self.device.set().is_some() { t::Auth::Multi(vec![(0, sig)]) } else { t::Auth::Sig(sig) };
        e
    }

    /// The entries that put `body` — a debit of this account's — at the end of `h`, stamped `now` (the beacon's height):
    /// this device's WRITER entry first if it is not the chain's writer, then the debit. Checked against the token's
    /// shape rules (refused: why); `h` is left as it was — the caller appends what it publishes.
    pub fn debit(&self, h: &Held, body: Body, now: u64) -> Result<Vec<Entry>, Why> {
        self.debit_named(h, body, now, &|n| n)
    }

    /// A debit whose name is `name` of its place's (a payment's tagged nonce).
    fn debit_named(&self, h: &Held, body: Body, now: u64, name: &dyn Fn([u8; 16]) -> [u8; 16]) -> Result<Vec<Entry>, Why> {
        if h.state.fork.is_some() {
            return Err("a forked chain");
        }
        let take = taken(&h.definition, &h.params.owner, &body).ok_or("not a debit this wallet builds")?;
        let mut s = h.state.clone();
        let stamp = now.max(s.entries.last().map_or(0, |e| e.stamp));
        let mut out = Vec::new();
        if t::writer_at(&s, s.entries.len()) != Some(self.signer()) {
            let w = self.writer(&s, &h.params, stamp);
            s.entries.push(w.clone());
            out.push(w);
        }
        let balance = s.balance().checked_sub(take).ok_or("a debit over the balance")?;
        let seq = s.entries.last().map_or(0, |e| e.seq + 1);
        let e = self.signed(&s, name(nonce_of(&h.params, seq)), stamp, body, balance);
        s.entries.push(e.clone());
        out.push(e);
        // The contract's own rules over the chain as it would be (no citations: a debit cites nothing).
        t::walk(&h.params, &h.definition, &s, &|_, _| None, &|_| None, Some(self.keys), 0, usize::MAX, &t::Extra::NONE).map_err(|(_, w)| w)?;
        Ok(out)
    }
}

impl Wallet<'_> {
    /// BUY `amount` copies of an edition SOLD ON DEMAND (TOKEN §1c: `Definition::sale`) — no one else signs. `sold` is
    /// the edition (its definition id `sold_id`), `record` its record `k` as read; `pay` this account's chain of the
    /// sale's token. Returns the PAYMENT (this device's writer entry first if need be, then the send of the price to the
    /// payee, its nonce tagged for record `k`) and the PAID MINT citing it, of the first free numbers in the record. The
    /// caller publishes the payment, then adds the mint to the record (a race that takes the numbers first: mint
    /// again with other free numbers of the same record — the payment buys a copy in that record only).
    pub fn buy(&self, pay: &Held, sold: &Definition, sold_id: &[u8; 32], record: (u64, &t::supply::Shard), amount: u128, now: u64) -> Result<(Vec<Entry>, t::supply::Mint), Why> {
        let (tok, price, payee) = sold.sale.ok_or("an edition not sold on demand")?;
        let owner = pay.params.owner;
        let payer = ChainParams { code_hash: pay.params.code_hash, definition: tok.definition, owner, beacon: pay.params.beacon, idlog: sold.idlog, item: tok.item };
        if payer.id() != pay.params.id() {
            return Err("a payment from another chain than the sale's token's");
        }
        let first = free(sold, record, amount).ok_or("no free numbers in the record")?;
        let tag = t::supply::sale_tag(sold_id, record.0);
        let send = Body::Send { to: payee, amount: price.checked_mul(amount).ok_or("a balance overflow")? };
        let out = self.debit_named(pay, send, now, &|n| {
            let mut x = n;
            x[..8].copy_from_slice(&tag);
            x
        })?;
        let e = out.last().expect("the payment");
        let mint = t::supply::Mint { to: owner, item: [0; 32], amount, nonce: first.to_be_bytes(), stamp: e.stamp, sigs: vec![], paid: Some((pay.params.id(), e.nonce)) };
        Ok((out, mint))
    }
}

/// The paid mint for a payment ALREADY MADE (`payment`, in the chain `payer`, tagged for record `k`) when a race took
/// the numbers `buy` chose: other free numbers of record `k` as read now — or, record `k` FULL and none of its mints
/// this payment's, the first free of record `k + 1` (a spilled mint, TOKEN §1c). Returns the record and the mint.
pub fn remint(payer: &ChainParams, payment: &Entry, sold: &Definition, sold_id: &[u8; 32], (k, now_k): (u64, &t::supply::Shard), next: &t::supply::Shard) -> Result<(u64, t::supply::Mint), Why> {
    let (_, price, _) = sold.sale.ok_or("an edition not sold on demand")?;
    let Body::Send { amount: paid, .. } = payment.body else { return Err("not a payment") };
    if payment.nonce[..8] != t::supply::sale_tag(sold_id, k) || price == 0 || paid % price != 0 {
        return Err("not a payment for this record");
    }
    let amount = paid / price;
    let mint = |first: u128| t::supply::Mint { to: payer.owner, item: [0; 32], amount, nonce: first.to_be_bytes(), stamp: payment.stamp, sigs: vec![], paid: Some((payer.id(), payment.nonce)) };
    if t::supply::admitted(sold, now_k).iter().any(|m| m.paid.is_some_and(|(_, n)| n == payment.nonce)) {
        return Err("a payment its record took already");
    }
    if let Some(first) = free(sold, (k, now_k), amount) {
        return Ok((k, mint(first)));
    }
    let m = mint(0);
    t::supply::spill_ok(sold, k + 1, Some(now_k), &m)?;
    let first = free(sold, (k + 1, next), amount).ok_or("no free numbers in the record after")?;
    Ok((k + 1, mint(first)))
}

/// The first of `amount` consecutive numbers of record `k` (as read) no admitted mint holds — past the genesis, within
/// the cap.
fn free(d: &Definition, (k, shard): (u64, &t::supply::Shard), amount: u128) -> Option<u128> {
    let size = t::supply::record_size(d);
    let lo = (k as u128).checked_mul(size)?.checked_add(1)?.max(d.genesis_amount + 1);
    let hi = (k as u128 + 1).checked_mul(size)?.min(if d.cap > 0 { d.cap } else { u128::MAX });
    let held = t::supply::admitted(d, shard);
    let taken = |n: u128| held.iter().any(|m| u128::from_be_bytes(m.nonce) <= n && n - u128::from_be_bytes(m.nonce) < m.amount);
    let mut n = lo;
    while amount > 0 && n.checked_add(amount - 1)? <= hi {
        match (n..n + amount).find(|x| taken(*x)) {
            None => return Some(n),
            Some(x) => n = x + 1,
        }
    }
    None
}

impl Wallet<'_> {
    /// The GENESIS of a token (its definition's owner, seq 0 on): this device's writer entry, then the genesis amount.
    pub fn genesis(&self, h: &Held, now: u64) -> Result<Vec<Entry>, Why> {
        if !h.state.entries.is_empty() {
            return Err("a genesis of a chain begun");
        }
        let mut s = State::default();
        let w = self.writer(&s, &h.params, now);
        s.entries.push(w.clone());
        let g = self.signed(&s, nonce_of(&h.params, 1), now, Body::Genesis { amount: h.definition.genesis_amount }, h.definition.genesis_amount);
        s.entries.push(g.clone());
        t::walk(&h.params, &h.definition, &s, &|_, _| None, &|_| None, Some(self.keys), 0, usize::MAX, &t::Extra::NONE).map_err(|(_, w)| w)?;
        Ok(vec![w, g])
    }
}

/// IS IT VALUE (W4, TOKEN §7a): the debit `nonce` in the sender's chain (`sender`, as read) — what a payment to this
/// account cites — ACCEPTED by its witnesses (`src`: the lot's claims, the bags' signatures, the beacon's outputs, the
/// stakers' chains). `Ok`: the signatures counted. A wallet credits a payment only on this: a chain's contract alone
/// proves its shape, never that its money is real.
pub fn is_value(src: &t::wallet::Source, sender: &ChainParams, chain: &State, nonce: &[u8; 16]) -> Result<usize, Why> {
    let i = chain.entries.iter().position(|e| &e.nonce == nonce).ok_or("a payment whose debit is not there")?;
    t::wallet::accept(src, sender, &chain.entries[i], i.checked_sub(1).map(|j| &chain.entries[j]))
}

/// What a wallet READ for collecting (W2): the chains of this token it looked in (its counterparties', from the
/// token's activity — the caller's to find), the programs it holds escrows with or is paid by, the supply shards and
/// records it was minted in, and the BLOCKS of those chains' cut histories (W3: a debit behind its sender's cut is
/// still received, shown by its history).
pub struct Sources<'a> {
    /// Others' chains of this token, by their owners (as live: maybe starting at a cut).
    pub chains: &'a [([u8; 32], State)],
    /// Blocks of others' cut histories (by id).
    pub blocks: &'a t::cut::Blocks,
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
        // After a CUT: every name ever taken (the whole history), and each new credit's ABSENCE proof against its received
        // set (W3: what the contract asks of a name a cut may hold).
        let received_root = s.entries.first().and_then(|e| match &e.body {
            Body::Cut { received, .. } if e.seq > 0 => Some(*received),
            _ => None,
        });
        let mut taken: Vec<Vec<u8>> = t::cut::received_keys(&h.whole()?.entries);
        let mine = |key: &[u8]| -> Result<t::Evidence, Why> {
            Ok(match received_root {
                Some(root) => t::Evidence { mine: t::cut::proof(&h.blocks, &root, key)?, theirs: vec![] },
                None => t::Evidence::default(),
            })
        };
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

        // RECEIVES and VESTING STEPS from others' chains; FEES if this account is the collector.
        let collector = d.transfer_fee.is_some_and(|(_, _, c)| c == p.owner);
        for (from, c) in src.chains {
            if c.fork.is_some() {
                continue;
            }
            let cp = p.at(p.tok(), *from);
            let whole = t::cut::whole(c, &|b| src.blocks.get(b).cloned()).unwrap_or_else(|_| c.clone());
            let history_root = c.entries.first().and_then(|e| match &e.body {
                Body::Cut { history, .. } if e.seq > 0 => Some(*history),
                _ => None,
            });
            for x in &whole.entries {
                if x.stamp.saturating_add(m) > stamp {
                    continue;
                }
                // Pruned from the sender's live chain: shown by its history (a presence proof and its bytes).
                let shown = |key: &[u8]| -> Result<t::Evidence, Why> {
                    let mut ev = mine(key)?;
                    if !c.entries.iter().any(|y| y.seq == x.seq) {
                        let root = history_root.ok_or("a debit behind a cut not shown")?;
                        ev.theirs.push((cp.id(), x.seq, t::cut::proof(src.blocks, &root, &t::history_key(x.seq))?, x.encode()));
                    }
                    Ok(ev)
                };
                match &x.body {
                    Body::Send { to, amount } | Body::Spend { to, amount } if *to == p.owner => {
                        let k = t::received_key(t::RECEIVED, from, &x.nonce);
                        if !taken.contains(&k) {
                            let ev = shown(&k)?;
                            taken.push(k);
                            push(&mut s, Body::Receive { from: *from, nonce: x.nonce, amount: *amount, cited: x.stamp, ev }, *amount, &mut out);
                        }
                    }
                    Body::Vest { to, schedule } if *to == p.owner => {
                        for (i, (at, amount)) in schedule.iter().enumerate() {
                            let k = t::vested_key(from, &x.nonce, i as u32);
                            if *at <= stamp && !taken.contains(&k) {
                                let ev = shown(&k)?;
                                taken.push(k);
                                push(&mut s, Body::Vested { from: *from, vest: x.nonce, step: i as u32, amount: *amount, at: *at, cited: x.stamp, ev }, *amount, &mut out);
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
                        let ev = shown(&k)?;
                        taken.push(k);
                        push(&mut s, Body::FeeClaim { from: *from, nonce: x.nonce, gross: gross.unwrap(), amount: fee, cited: x.stamp, ev }, fee, &mut out);
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
                if *tok == p.tok() && *to == p.owner && !taken.contains(&k) {
                    let ev = mine(&k)?;
                    taken.push(k);
                    push(&mut s, Body::Payout { program: *prog, key: *key, amount: *amount, verdict: sec.hash(), ev }, *amount, &mut out);
                }
            }
        }
        // MINTS to this owner (of this chain's item) in the shards read.
        for sh in src.shards {
            for mt in t::supply::admitted(d, sh) {
                if mt.to != p.owner || mt.stamp > stamp {
                    continue;
                }
                // A NUMBERED copy: this chain's, if its number is in the mint's range (one copy).
                if d.ids == t::Ids::Numbered {
                    let first = u128::from_be_bytes(mt.nonce);
                    let Some(n) = (first..first.saturating_add(mt.amount)).find(|n| t::numbered_item(&p.definition, *n) == p.item) else { continue };
                    let k = t::received_key(t::MINTED, &t::MINTER, &n.to_be_bytes());
                    if !taken.contains(&k) {
                        let ev = mine(&k)?;
                        taken.push(k);
                        let record = t::supply::record_of(d, n).unwrap_or(0);
                        push(&mut s, Body::CappedMint { nonce: n.to_be_bytes(), amount: 1, minted: mt.stamp, record, mint: mt.clone(), ev }, 1, &mut out);
                    }
                    continue;
                }
                let k = t::received_key(t::MINTED, &t::MINTER, &mt.nonce);
                let plain = mt.item == p.item && !d.capped();
                if (plain || (d.capped() && d.ids != t::Ids::Numbered && mt.item == p.item)) && !taken.contains(&k) {
                    let ev = mine(&k)?;
                    taken.push(k);
                    let body = if d.capped() {
                        Body::CappedMint { nonce: mt.nonce, amount: mt.amount, minted: mt.stamp, record: t::supply::record_of(d, u128::from_be_bytes(mt.nonce)).unwrap_or(0), mint: mt.clone(), ev }
                    } else {
                        Body::Mint { nonce: mt.nonce, amount: mt.amount, minted: mt.stamp, mint: mt.clone(), ev }
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

/// A DEVICE that signs by ASKING (the page's: its node's identity delegate answers `TokenSign` asynchronously, and the
/// wallet's building is synchronous). Built with the signatures the delegate gave so far; every message it has none
/// for is recorded in `need` (and signed with zeros, so nothing built that way is valid). The page builds, has the
/// delegate sign the next message, and builds again — until nothing is needed (one round per entry signed: a chain's
/// entries are signed in order, each over its predecessor's head). A build that asked is refused by the token's rules
/// (its zeros): the caller reads `next` first.
pub struct Asking {
    pub public: [u8; 32],
    pub credential: Vec<u8>,
    pub key_seq: u32,
    pub sigs: Vec<(Vec<u8>, [u8; 64])>,
    pub need: std::cell::RefCell<Vec<Vec<u8>>>,
}

impl Asking {
    pub fn new(public: [u8; 32], credential: Vec<u8>, key_seq: u32, sigs: Vec<(Vec<u8>, [u8; 64])>) -> Asking {
        Asking { public, credential, key_seq, sigs, need: Default::default() }
    }
    /// The NEXT message to sign: the first asked (a later one was built over a head with zeros in it — it is asked
    /// again, right, once this one is signed). `None`: everything built was signed.
    pub fn next(&self) -> Option<Vec<u8>> {
        self.need.borrow().first().cloned()
    }
}

impl Device for Asking {
    fn public(&self) -> [u8; 32] {
        self.public
    }
    fn sign(&self, message: &[u8]) -> [u8; 64] {
        if let Some((_, s)) = self.sigs.iter().find(|(m, _)| m == message) {
            return *s;
        }
        let mut need = self.need.borrow_mut();
        if !need.iter().any(|m| m == message) {
            need.push(message.to_vec());
        }
        [0; 64]
    }
    fn credential(&self) -> Vec<u8> {
        self.credential.clone()
    }
    fn key_seq(&self) -> u32 {
        self.key_seq
    }
}
