//! THE SUPPLY BAG (TOKEN §1a): one shard per token per [`EPOCH`] of beacon heights (params [`Params`]), holding every
//! MINT of that epoch — each signed by the token's mint authority current at its stamp (read from the definition and
//! the authority chain). A `Mint` entry in an account chain is valid only if its mint is ALREADY in its epoch's shard,
//! so no token comes into being where nobody can find it; `totalSupply` = genesis + Σ the shards' mints − burns.
//! A shard admits only signed mints, so nothing but the authority can fill it; a mint past [`MINTS_MAX`] waits for
//! the next epoch.

use crate::authority::{self, KeySet};
use crate::Why;

pub const SUPPLY: u8 = 4;
/// Beacon heights per shard.
pub const EPOCH: u64 = 4096;
/// The most mints one shard holds.
pub const MINTS_MAX: usize = 1024;

pub fn epoch(stamp: u64) -> u64 {
    stamp / EPOCH
}

/// One MINT: `amount` to the chain of DID `to`, named by `nonce`, at beacon `stamp`, signed by k of the mint set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mint {
    pub to: [u8; 32],
    pub amount: u128,
    pub nonce: [u8; 16],
    pub stamp: u64,
    pub sigs: Vec<(u8, [u8; 64])>,
}

impl Mint {
    /// What the mint set signs.
    pub fn message(&self, definition: &[u8; 32]) -> Vec<u8> {
        let mut v = b"craftworks token mint".to_vec();
        v.extend_from_slice(definition);
        v.extend_from_slice(&self.to);
        v.extend_from_slice(&self.amount.to_le_bytes());
        v.extend_from_slice(&self.nonce);
        v.extend_from_slice(&self.stamp.to_le_bytes());
        v
    }
    fn encode(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(&self.to);
        v.extend_from_slice(&self.amount.to_le_bytes());
        v.extend_from_slice(&self.nonce);
        v.extend_from_slice(&self.stamp.to_le_bytes());
        authority::put_sigs(v, &self.sigs);
    }
    fn read(r: &mut crate::R) -> Option<Mint> {
        Some(Mint { to: r.arr()?, amount: r.u128()?, nonce: r.arr()?, stamp: r.u64()?, sigs: authority::read_sigs(r)? })
    }
}

/// A shard's PARAMS: the token code's hash (its definition's id checked), the definition, the beacon, the epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub code_hash: [u8; 32],
    pub definition: [u8; 32],
    pub beacon: [u8; 32],
    pub epoch: u64,
}

impl Params {
    pub fn encode(&self) -> Vec<u8> {
        [&[SUPPLY][..], &self.code_hash, &self.definition, &self.beacon, &self.epoch.to_le_bytes()].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Params> {
        let mut r = crate::R(b);
        (r.u8()? == SUPPLY).then_some(())?;
        let p = Params { code_hash: r.arr()?, definition: r.arr()?, beacon: r.arr()?, epoch: r.u64()? };
        r.done().then_some(p)
    }
    pub fn id(&self) -> [u8; 32] {
        crate::id(&self.code_hash, &self.encode())
    }
    /// Its token's authority chain.
    pub fn authority(&self) -> authority::Params {
        authority::Params { code_hash: self.code_hash, definition: self.definition, beacon: self.beacon }
    }
}

/// A shard's state: its mints, sorted by nonce, each nonce once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shard {
    pub mints: Vec<Mint>,
}

impl Shard {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = b"CWSP".to_vec();
        v.extend_from_slice(&(self.mints.len() as u16).to_le_bytes());
        for m in &self.mints {
            m.encode(&mut v);
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<Shard> {
        let mut r = crate::R(b.strip_prefix(b"CWSP")?);
        let n = u16::from_le_bytes(r.arr()?) as usize;
        (n <= MINTS_MAX).then_some(())?;
        let mints = (0..n).map(|_| Mint::read(&mut r)).collect::<Option<Vec<_>>>()?;
        (r.done() && mints.windows(2).all(|w| w[0].nonce < w[1].nonce)).then_some(Shard { mints })
    }
    pub fn find(&self, nonce: &[u8; 16]) -> Option<&Mint> {
        self.mints.binary_search_by(|m| m.nonce.cmp(nonce)).ok().map(|i| &self.mints[i])
    }
}

/// The SEAL of a shard's mints stamped before `before`: what a mint change names for that epoch (`None`: there are
/// none).
pub fn seal(s: &Shard, before: u64) -> Option<[u8; 32]> {
    let prefix = Shard { mints: s.mints.iter().filter(|m| m.stamp < before).cloned().collect() };
    (!prefix.mints.is_empty()).then(|| *blake3::hash(&prefix.encode()).as_bytes())
}

/// Every mint in the shard is of its epoch, for something, and signed by the mint set current at its stamp — and
/// every MINT CHANGE whose span holds this epoch sealed what it holds before that change, exactly.
pub fn holds(p: &Params, first: (&Option<KeySet>, &Option<KeySet>), chain: &authority::Chain, s: &Shard) -> Result<(), Why> {
    let mut from = 0u64;
    for c in chain.changes.iter().filter(|c| c.role == authority::Role::Mint) {
        let upto = epoch(c.stamp);
        if (from..=upto).contains(&p.epoch) && seal(s, c.stamp) != c.closed.iter().find(|(e, _)| *e == p.epoch).map(|(_, h)| *h) {
            return Err("a shard that is not what its mint authority sealed");
        }
        from = upto;
    }
    for m in &s.mints {
        if epoch(m.stamp) != p.epoch {
            return Err("a mint of another epoch");
        }
        if m.amount == 0 {
            return Err("a mint of nothing");
        }
        let (mint, _) = authority::sets_at(&p.definition, first, chain, m.stamp)?;
        let set = mint.ok_or("a mint with no mint authority")?;
        if !set.signed(&m.message(&p.definition), &m.sigs) {
            return Err("a mint not signed by the mint authority");
        }
    }
    Ok(())
}

/// The MERGE: the union by nonce (two mints under one nonce: the lower hash — the authority signed both, its fault).
/// `None`: the union is past [`MINTS_MAX`] — refused; the mint goes in the next epoch.
pub fn merge(a: &Shard, b: &Shard) -> Option<Shard> {
    let h = |x: &Mint| {
        let mut v = Vec::new();
        x.encode(&mut v);
        *blake3::hash(&v).as_bytes()
    };
    let mut all: Vec<Mint> = a.mints.iter().chain(&b.mints).cloned().collect();
    all.sort_by_cached_key(|m| (m.nonce, h(m)));
    all.dedup_by(|later, first| later.nonce == first.nonce);
    (all.len() <= MINTS_MAX).then_some(Shard { mints: all })
}

/// An UPDATE's rule: every mint the merged shard holds that the held one did not is stamped within the margin of
/// `now` — a mint authority cannot back-date (nor a dropped one mint in its past).
pub fn fresh(held: &Shard, merged: &Shard, now: u64, margin: u64) -> Result<(), Why> {
    for m in &merged.mints {
        if !held.mints.contains(m) && (m.stamp > now || m.stamp + margin < now) {
            return Err("a mint stamped outside the margin of now");
        }
    }
    Ok(())
}

/// totalSupply: the genesis plus every mint in these shards (burns: TOKEN §1a, counted from the chains).
pub fn minted(shards: &[Shard]) -> Option<u128> {
    shards.iter().flat_map(|s| &s.mints).try_fold(0u128, |t, m| t.checked_add(m.amount))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const DEF: [u8; 32] = [5; 32];
    fn p(epoch: u64) -> Params {
        Params { code_hash: [9; 32], definition: DEF, beacon: [7; 32], epoch }
    }
    fn set(ns: &[u8]) -> KeySet {
        KeySet { k: 1, keys: ns.iter().map(|n| SigningKey::from_bytes(&[*n; 32]).verifying_key().to_bytes()).collect() }
    }
    /// The mint set (key 1) moves to key 2 at `stamp`, sealing `closed`.
    fn change(stamp: u64, closed: Vec<(u64, [u8; 32])>) -> authority::Chain {
        let mut c = authority::Change { seq: 0, role: authority::Role::Mint, next: Some(set(&[2])), stamp, closed, cap: 0, sigs: vec![] };
        c.sigs = vec![(0, SigningKey::from_bytes(&[1; 32]).sign(&c.message(&DEF)).to_bytes())];
        authority::Chain { changes: vec![c] }
    }
    fn mint(n: u8, stamp: u64, by: u8) -> Mint {
        let mut m = Mint { to: [3; 32], amount: 50, nonce: [n; 16], stamp, sigs: vec![] };
        m.sigs = vec![(0, SigningKey::from_bytes(&[by; 32]).sign(&m.message(&DEF)).to_bytes())];
        m
    }

    #[test]
    fn a_shard_holds_only_mints_signed_by_the_authority_of_their_stamp_and_epoch() {
        let first = (Some(set(&[1])), None);
        let none = authority::Chain::default();
        let s = Shard { mints: vec![mint(1, 10, 1), mint(2, 11, 1)] };
        assert_eq!(Shard::decode(&s.encode()).as_ref(), Some(&s));
        assert_eq!(holds(&p(0), (&first.0, &first.1), &none, &s), Ok(()));
        assert_eq!(holds(&p(1), (&first.0, &first.1), &none, &s), Err("a mint of another epoch"));
        let mut zero = mint(3, 10, 1);
        zero.amount = 0;
        zero.sigs = vec![(0, SigningKey::from_bytes(&[1; 32]).sign(&zero.message(&DEF)).to_bytes())];
        assert_eq!(holds(&p(0), (&first.0, &first.1), &none, &Shard { mints: vec![zero] }), Err("a mint of nothing"));
        let forged = Shard { mints: vec![mint(1, 10, 2)] };
        assert_eq!(holds(&p(0), (&first.0, &first.1), &none, &forged), Err("a mint not signed by the mint authority"));
        assert_eq!(holds(&p(0), (&None, &None), &none, &s), Err("a mint with no mint authority"), "fixed supply");
        // The set moves to key 2 at stamp 11, sealing epoch 0 as it holds key 1's mint at 10.
        let sealed = Shard { mints: vec![mint(1, 10, 1)] };
        let moved = change(11, vec![(0, seal(&sealed, 11).unwrap())]);
        assert_eq!(holds(&p(0), (&first.0, &first.1), &moved, &sealed), Ok(()));
        let after = merge(&sealed, &Shard { mints: vec![mint(2, 11, 2)] }).unwrap();
        assert_eq!(holds(&p(0), (&first.0, &first.1), &moved, &after), Ok(()), "the new set mints on");
        let stale = merge(&sealed, &Shard { mints: vec![mint(5, 11, 1)] }).unwrap();
        assert_eq!(holds(&p(0), (&first.0, &first.1), &moved, &stale), Err("a mint not signed by the mint authority"), "the old set, after");
        let backdated = merge(&sealed, &Shard { mints: vec![mint(3, 9, 1)] }).unwrap();
        assert_eq!(holds(&p(0), (&first.0, &first.1), &moved, &backdated), Err("a shard that is not what its mint authority sealed"), "the old set, into its past");
    }

    #[test]
    fn a_change_seals_every_epoch_since_the_last_one_even_a_shard_never_created() {
        let first = (Some(set(&[1])), None);
        let at = 2 * EPOCH + 5;
        let e1 = Shard { mints: vec![mint(1, EPOCH + 3, 1)] };
        let moved = change(at, vec![(1, seal(&e1, at).unwrap())]);
        assert_eq!(holds(&p(1), (&first.0, &first.1), &moved, &e1), Ok(()));
        assert_eq!(holds(&p(0), (&first.0, &first.1), &moved, &Shard::default()), Ok(()), "epoch 0: sealed empty");
        assert_eq!(holds(&p(0), (&first.0, &first.1), &moved, &Shard { mints: vec![mint(2, 7, 1)] }), Err("a shard that is not what its mint authority sealed"), "a shard first put after the seal");
        assert_eq!(holds(&p(2), (&first.0, &first.1), &moved, &Shard { mints: vec![mint(2, 2 * EPOCH, 1)] }), Err("a shard that is not what its mint authority sealed"), "its own epoch, before it");
        assert_eq!(holds(&p(2), (&first.0, &first.1), &moved, &Shard { mints: vec![mint(2, at, 2)] }), Ok(()), "its own epoch, after it");
        assert_eq!(holds(&p(3), (&first.0, &first.1), &moved, &Shard { mints: vec![mint(2, 3 * EPOCH, 2)] }), Ok(()), "beyond it: open");
        // The NEXT change (key 2 → 3, epoch 3) seals from the epoch of the one before: key 2's mint after it in epoch 2.
        let mut two = moved.clone();
        let mut c = authority::Change { seq: 1, role: authority::Role::Mint, next: Some(set(&[3])), stamp: 3 * EPOCH, closed: vec![], cap: 0, sigs: vec![] };
        c.sigs = vec![(0, SigningKey::from_bytes(&[2; 32]).sign(&c.message(&DEF)).to_bytes())];
        two.changes.push(c);
        assert_eq!(holds(&p(2), (&first.0, &first.1), &two, &Shard { mints: vec![mint(2, at, 2)] }), Err("a shard that is not what its mint authority sealed"), "left out by the next seal");
    }

    #[test]
    fn the_merge_is_a_union_one_answer_capped_and_no_back_dating() {
        let a = Shard { mints: vec![mint(1, 10, 1)] };
        let b = Shard { mints: vec![mint(2, 10, 1), mint(1, 10, 2)] };
        assert_eq!(merge(&a, &b), merge(&b, &a));
        assert_eq!(merge(&a, &b).unwrap().mints.len(), 2);
        let full = Shard { mints: (0..MINTS_MAX as u16).map(|i| { let mut m = mint(0, 10, 1); m.nonce[..2].copy_from_slice(&i.to_be_bytes()); m }).collect() };
        assert!(merge(&full, &Shard { mints: vec![mint(0xff, 10, 1)] }).is_none(), "past the cap");
        assert!(fresh(&Shard::default(), &a, 11, 2).is_ok());
        assert!(fresh(&Shard::default(), &a, 13, 2).is_err(), "back-dated");
        assert!(fresh(&Shard::default(), &a, 9, 2).is_err(), "from the future");
        assert!(fresh(&a, &a, 99, 2).is_ok());
        assert_eq!(minted(&[a, b]), Some(150));
    }
}
