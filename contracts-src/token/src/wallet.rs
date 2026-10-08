//! WALLET VERIFICATION (docs/TOKEN.md §3a, step 6): what makes a balance VALUE. The contract checks a chain's shape and
//! its last [`RECENT`](super::RECENT) citations; a wallet, before it treats a payment as its own, checks EVERY citation
//! and walks each cited chain back — to genesis, or to a CHECKPOINT it accepts — so a chain whose past does not add up
//! (a fake receive buried under real ones; a loop of chains citing each other into money from nothing) is refused,
//! with a FRAUD PROOF naming the entry. Checkpoints are public and signed; others VOUCH for them; a false one's proof
//! names its maker AND every voucher, and a wallet skips all of theirs from then on.

use super::{check_window, sig_ok, Body, ChainParams, Definition, State, Why};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

/// The most chains one verification reads: past it, the payment is refused as too long to verify (a wallet never
/// spins forever, whatever graph it is handed). Checkpoints keep honest walks far below it.
pub const MAX_READS: usize = 100_000;

/// "Chain `chain` is honest up to entry `seq`, whose hash is `head`, its balance after it `balance`" — signed by `by`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    pub chain: ChainParams,
    pub seq: u64,
    pub head: [u8; 32],
    pub balance: u128,
    pub by: [u8; 32],
    pub sig: [u8; 64],
}

impl Checkpoint {
    pub fn message(chain: &ChainParams, seq: u64, head: &[u8; 32], balance: u128) -> Vec<u8> {
        [&b"craftworks token checkpoint"[..], &chain.id(), &seq.to_le_bytes(), head, &balance.to_le_bytes()].concat()
    }
    pub fn signed(&self) -> bool {
        sig_ok(&self.by, &Self::message(&self.chain, self.seq, &self.head, self.balance), &self.sig)
    }
    pub fn hash(&self) -> [u8; 32] {
        *blake3::hash(&[&Self::message(&self.chain, self.seq, &self.head, self.balance)[..], &self.by, &self.sig].concat()).as_bytes()
    }
}

/// "I checked that checkpoint too" — signed by `by`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vouch {
    pub checkpoint: [u8; 32],
    pub by: [u8; 32],
    pub sig: [u8; 64],
}

impl Vouch {
    pub fn message(checkpoint: &[u8; 32]) -> Vec<u8> {
        [&b"craftworks token vouch"[..], checkpoint].concat()
    }
    pub fn signed(&self) -> bool {
        sig_ok(&self.by, &Self::message(&self.checkpoint), &self.sig)
    }
}

/// Why a chain is not value: the chain and the entry where it fails, and why — or, a false CHECKPOINT: it too, and
/// who stood behind it (its maker and every voucher).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FraudProof {
    pub chain: ChainParams,
    pub seq: u64,
    pub why: Why,
    pub checkpoint: Option<Checkpoint>,
    pub vouchers: Vec<[u8; 32]>,
}

/// Where a wallet reads chains and definitions from (the network, through a node).
pub struct Source<'a> {
    pub chain: &'a dyn Fn(&ChainParams) -> Option<State>,
    pub definition: &'a dyn Fn(&[u8; 32]) -> Option<Definition>,
}

/// A WALLET's verifier: whom it trusts for checkpoints, what it has verified (its memory), what fraud it knows.
#[derive(Default)]
pub struct Wallet {
    /// This wallet's own key (its own checkpoints are trusted).
    pub me: [u8; 32],
    /// Keys whose checkpoints — or vouches — it accepts.
    pub trusted: Vec<[u8; 32]>,
    /// A checkpoint vouched for by this many keys (none distrusted, none the chain's owner) is accepted too (0: never
    /// by count alone).
    pub min_vouches: usize,
    pub checkpoints: Vec<Checkpoint>,
    pub vouches: Vec<Vouch>,
    pub frauds: Vec<FraudProof>,
    /// Chains verified: id → (up to seq, that entry's hash).
    verified: HashMap<[u8; 32], (u64, [u8; 32])>,
    /// Chains being verified right now, a STACK of (id, up to seq) — a chain may be walked again for an earlier
    /// entry inside its own walk (legitimately); a citation loop needs it at or past where its walk began.
    walking: RefCell<Vec<([u8; 32], u64)>>,
    /// How many chains were read — what a checkpoint saves.
    pub reads: Cell<usize>,
}

impl Wallet {
    pub fn new(me: [u8; 32]) -> Self {
        Self { me, ..Self::default() }
    }

    /// A key behind a proven fraud — as a checkpoint's maker or one of its vouchers.
    pub fn distrusted(&self, k: &[u8; 32]) -> bool {
        self.frauds.iter().any(|f| f.checkpoint.as_ref().is_some_and(|c| &c.by == k) || f.vouchers.contains(k))
    }

    /// The vouches for a checkpoint that count: signed, not by its chain's owner, not by anyone distrusted.
    fn vouchers_of(&self, c: &Checkpoint) -> Vec<[u8; 32]> {
        let h = c.hash();
        let mut v: Vec<[u8; 32]> = self.vouches.iter().filter(|x| x.checkpoint == h && x.signed() && x.by != c.chain.owner && !self.distrusted(&x.by)).map(|x| x.by).collect();
        v.sort();
        v.dedup();
        v
    }

    /// A checkpoint this wallet stops its walk at, covering `chain` up to `seq`: signed, not by the chain's own owner
    /// (they are the one who could forge it), by no one distrusted — and made by this wallet, or by or vouched for by a
    /// key it trusts, or vouched for by enough keys.
    pub fn accepted(&self, chain: &ChainParams, seq: u64) -> Option<&Checkpoint> {
        self.checkpoints.iter().filter(|c| c.chain == *chain && c.seq >= seq && c.signed() && c.by != chain.owner && !self.distrusted(&c.by)).find(|c| {
            let v = self.vouchers_of(c);
            c.by == self.me || self.trusted.contains(&c.by) || v.iter().any(|k| *k == self.me || self.trusted.contains(k)) || (self.min_vouches > 0 && v.len() >= self.min_vouches)
        })
    }

    /// VERIFY `chain` up to entry `upto`: every citation, every cited chain up to the entry cited — to genesis or an
    /// accepted checkpoint. `Err`: the proof of where it fails (in this chain or one it rests on). ITERATIVE: an
    /// explicit stack of tasks (a chain VISITED — read, checked, its cited chains queued — then EXITED once they are
    /// verified), so a long history never deepens the call stack (a browser wallet's is small).
    pub fn verify(&mut self, src: &Source, chain: &ChainParams, upto: u64) -> Result<(), FraudProof> {
        enum Task {
            Visit(ChainParams, u64),
            Exit([u8; 32], u64, [u8; 32]),
        }
        let mut tasks = vec![Task::Visit(*chain, upto)];
        let proof = |c: &ChainParams, seq: u64, why: Why| FraudProof { chain: *c, seq, why, checkpoint: None, vouchers: vec![] };
        let start = self.reads.get();
        while let Some(t) = tasks.pop() {
            if self.reads.get() - start > MAX_READS {
                return Err(proof(chain, upto, "a history too long to verify"));
            }
            let (chain, upto) = match t {
                Task::Exit(id, upto, head) => {
                    self.walking.borrow_mut().pop();
                    self.verified.insert(id, (upto, head));
                    continue;
                }
                Task::Visit(c, u) => (c, u),
            };
            let id = chain.id();
            if self.verified.get(&id).is_some_and(|(s, _)| *s >= upto) {
                continue;
            }
            // A citation LOOP: this chain is on the path being walked, and now something needs it at or past where
            // its walk began.
            if self.walking.borrow().iter().any(|(w, at)| *w == id && upto >= *at) {
                return Err(proof(&chain, upto, "a citation loop: money from nothing"));
            }
            let read = |p: &ChainParams| {
                self.reads.set(self.reads.get() + 1);
                (src.chain)(p)
            };
            let s = read(&chain).ok_or_else(|| proof(&chain, upto, "a chain that is not there"))?;
            if s.entries.len() as u64 <= upto {
                return Err(proof(&chain, upto, "an entry that is not in its chain"));
            }
            // An accepted checkpoint covering it: the chain is the one checkpointed (that entry's hash) — no walk.
            if let Some(c) = self.accepted(&chain, upto) {
                if s.entries.get(c.seq as usize).is_some_and(|e| e.hash() == c.head && e.balance == c.balance) {
                    let (seq, head) = (c.seq, c.head);
                    self.verified.insert(id, (seq, head));
                    continue;
                }
            }
            let d = (src.definition)(&chain.definition).ok_or_else(|| proof(&chain, 0, "a token whose definition is not there"))?;
            let mut prefix = s.clone();
            prefix.entries.truncate(upto as usize + 1);
            prefix.fork = None;
            // EVERY citation checked (the shape rules, the window the whole chain) — each cited chain read once.
            let cache: RefCell<HashMap<[u8; 32], Option<State>>> = RefCell::new(HashMap::new());
            let cited = |def: &[u8; 32], owner: &[u8; 32]| {
                let p = chain.at(*def, *owner);
                cache.borrow_mut().entry(p.id()).or_insert_with(|| read(&p)).clone()
            };
            check_window(&chain, &d, &prefix, &cited, usize::MAX).map_err(|(seq, why)| proof(&chain, seq, why))?;
            // What it rests on: each cited entry's own chain, up to it.
            let mut needs: Vec<(ChainParams, u64)> = Vec::new();
            let seq_of = |p: &ChainParams, pick: &dyn Fn(&super::Entry) -> bool| cited(&p.definition, &p.owner).and_then(|c| c.entries.iter().find(|e| pick(e)).map(|e| e.seq));
            for e in &prefix.entries {
                match &e.body {
                    Body::Receive { from, nonce, .. } => {
                        let p = chain.at(chain.definition, *from);
                        if let Some(q) = seq_of(&p, &|x| x.nonce == *nonce) {
                            needs.push((p, q));
                        }
                    }
                    Body::Fill { offer, taker, lock, .. } => {
                        let get = prefix.entries.iter().find_map(|x| match &x.body {
                            Body::Offer { get, .. } if &x.nonce == offer => Some(*get),
                            _ => None,
                        });
                        if let Some(get) = get {
                            let p = chain.at(get, *taker);
                            if let Some(q) = seq_of(&p, &|x| x.nonce == *lock) {
                                needs.push((p, q));
                            }
                        }
                    }
                    Body::Claim { taker, lock, token, cited: at, .. } => {
                        let pl = chain.at(chain.definition, *taker);
                        if let Some(q) = seq_of(&pl, &|x| x.nonce == *lock) {
                            needs.push((pl, q));
                        }
                        let pf = chain.at(*token, chain.owner);
                        if let Some(q) = seq_of(&pf, &|x| x.stamp == *at && matches!(x.body, Body::Fill { taker: t, lock: l, .. } if t == *taker && l == *lock)) {
                            needs.push((pf, q));
                        }
                    }
                    _ => {}
                }
            }
            // Exit after everything it rests on (a stack: pushed first, popped last).
            self.walking.borrow_mut().push((id, upto));
            tasks.push(Task::Exit(id, upto, prefix.entries[upto as usize].hash()));
            for (p, q) in needs.into_iter().rev() {
                tasks.push(Task::Visit(p, q));
            }
        }
        Ok(())
    }

    /// AUDIT a checkpoint: verify its chain from scratch (trusting no checkpoint) and that it says what the chain says.
    /// `Err`: the fraud proof naming its maker and every voucher.
    pub fn audit(&self, src: &Source, c: &Checkpoint) -> Result<(), FraudProof> {
        let mut fresh = Wallet::new([0; 32]);
        let false_ = |why: Why, seq: u64| FraudProof { chain: c.chain, seq, why, checkpoint: Some(c.clone()), vouchers: self.vouches.iter().filter(|v| v.checkpoint == c.hash() && v.signed()).map(|v| v.by).collect() };
        fresh.verify(src, &c.chain, c.seq).map_err(|f| false_(f.why, f.seq))?;
        let s = (src.chain)(&c.chain).ok_or_else(|| false_("a chain that is not there", c.seq))?;
        let e = s.entries.get(c.seq as usize).ok_or_else(|| false_("an entry that is not in its chain", c.seq))?;
        if e.hash() != c.head || e.balance != c.balance {
            return Err(false_("a checkpoint that is not what the chain says", c.seq));
        }
        Ok(())
    }
}

/// THE BAGS (TOKEN §3a) a token's wallets share: one per kind, named by the token, each item one record below.
pub fn bag_name(kind: &str, definition: &[u8; 32]) -> String {
    format!("token {kind} {}", definition.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// The records' BYTES — exact: trailing bytes refused.
pub mod bytes {
    use super::{Checkpoint, FraudProof, Vouch};
    use crate::ChainParams;

    fn take<'a>(b: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
        if b.len() < n {
            return None;
        }
        let (x, rest) = b.split_at(n);
        *b = rest;
        Some(x)
    }
    fn arr<const N: usize>(b: &mut &[u8]) -> Option<[u8; N]> {
        take(b, N)?.try_into().ok()
    }

    pub fn checkpoint(c: &Checkpoint) -> Vec<u8> {
        [&c.chain.encode()[..], &c.seq.to_le_bytes(), &c.head, &c.balance.to_le_bytes(), &c.by, &c.sig].concat()
    }
    fn read_checkpoint(b: &mut &[u8]) -> Option<Checkpoint> {
        let chain = ChainParams::decode(take(b, 1 + 32 * 4)?)?;
        Some(Checkpoint { chain, seq: u64::from_le_bytes(arr(b)?), head: arr(b)?, balance: u128::from_le_bytes(arr(b)?), by: arr(b)?, sig: arr(b)? })
    }
    pub fn read_checkpoint_exact(mut b: &[u8]) -> Option<Checkpoint> {
        let c = read_checkpoint(&mut b)?;
        b.is_empty().then_some(c)
    }

    pub fn vouch(v: &Vouch) -> Vec<u8> {
        [&v.checkpoint[..], &v.by, &v.sig].concat()
    }
    pub fn read_vouch(mut b: &[u8]) -> Option<Vouch> {
        let v = Vouch { checkpoint: arr(&mut b)?, by: arr(&mut b)?, sig: arr(&mut b)? };
        b.is_empty().then_some(v)
    }

    /// A fraud proof travels as its CLAIM — the chain, the entry, and (a false checkpoint) the checkpoint and its
    /// vouchers; its WHY is re-derived by whoever checks it (they verify the chain themselves), never trusted.
    pub fn fraud(f: &FraudProof) -> Vec<u8> {
        let mut v = [&f.chain.encode()[..], &f.seq.to_le_bytes()].concat();
        match &f.checkpoint {
            None => v.push(0),
            Some(c) => {
                v.push(1);
                v.extend_from_slice(&checkpoint(c));
            }
        }
        v.extend_from_slice(&(f.vouchers.len() as u32).to_le_bytes());
        for k in &f.vouchers {
            v.extend_from_slice(k);
        }
        v
    }
    pub fn read_fraud(mut b: &[u8]) -> Option<FraudProof> {
        let chain = ChainParams::decode(take(&mut b, 1 + 32 * 4)?)?;
        let seq = u64::from_le_bytes(arr(&mut b)?);
        let checkpoint = match take(&mut b, 1)?[0] {
            0 => None,
            1 => Some(read_checkpoint(&mut b)?),
            _ => return None,
        };
        let n = u32::from_le_bytes(arr(&mut b)?) as usize;
        if n > 1024 {
            return None;
        }
        let vouchers = (0..n).map(|_| arr(&mut b)).collect::<Option<Vec<[u8; 32]>>>()?;
        b.is_empty().then_some(FraudProof { chain, seq, why: "claimed: re-check it", checkpoint, vouchers })
    }
}
