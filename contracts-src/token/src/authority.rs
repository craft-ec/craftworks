//! AUTHORITIES (TOKEN §1a): who may MINT a token and who may FREEZE its chains — each a KEY SET (k of n keys), named
//! first by the definition, then changed by the token's AUTHORITY CHAIN (one contract per token: params
//! [`Params`]). Each change is signed by k of the set it replaces and names the next set, or none:
//! that authority DROPPED, for good. A mint or freeze is checked against the set current at its stamp ([`sets_at`]).

use crate::{sig_ok, Why};

/// The most keys one set holds.
pub const KEYS_MAX: usize = 16;
/// The most changes one authority chain holds (a change is rare and deliberate).
pub const CHANGES_MAX: usize = 256;
/// An authority chain's params tag.
pub const AUTHORITY: u8 = 3;

/// K of these keys sign together. (Foundation 4's multisig owners are such sets too.)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySet {
    pub k: u8,
    pub keys: Vec<[u8; 32]>,
}

impl KeySet {
    /// A set that can sign: 1 ≤ k ≤ n ≤ [`KEYS_MAX`], its keys distinct.
    pub fn sound(&self) -> bool {
        let mut k = self.keys.clone();
        k.sort();
        k.dedup();
        self.k >= 1 && (self.k as usize) <= self.keys.len() && self.keys.len() <= KEYS_MAX && k.len() == self.keys.len()
    }
    /// `sigs` (key index, signature) hold k DISTINCT keys of this set signing `message`.
    pub fn signed(&self, message: &[u8], sigs: &[(u8, [u8; 64])]) -> bool {
        let mut seen: Vec<u8> = Vec::new();
        for (i, s) in sigs {
            let Some(key) = self.keys.get(*i as usize) else { return false };
            if seen.contains(i) || !sig_ok(key, message, s) {
                return false;
            }
            seen.push(*i);
        }
        seen.len() >= self.k as usize
    }
    pub fn encode(&self, v: &mut Vec<u8>) {
        v.push(self.k);
        v.push(self.keys.len() as u8);
        for k in &self.keys {
            v.extend_from_slice(k);
        }
    }
    pub(crate) fn read(r: &mut crate::R) -> Option<KeySet> {
        let k = r.u8()?;
        let n = r.u8()? as usize;
        (n <= KEYS_MAX).then_some(())?;
        let keys = (0..n).map(|_| r.arr()).collect::<Option<Vec<[u8; 32]>>>()?;
        let s = KeySet { k, keys };
        s.sound().then_some(s)
    }
}

/// An optional set, as bytes: `0` none, `1` then the set.
pub fn put_opt(v: &mut Vec<u8>, s: &Option<KeySet>) {
    match s {
        None => v.push(0),
        Some(s) => {
            v.push(1);
            s.encode(v);
        }
    }
}
pub(crate) fn read_opt(r: &mut crate::R) -> Option<Option<KeySet>> {
    match r.u8()? {
        0 => Some(None),
        1 => Some(Some(KeySet::read(r)?)),
        _ => None,
    }
}
/// Signatures (key index, signature), at most [`KEYS_MAX`].
pub fn put_sigs(v: &mut Vec<u8>, sigs: &[(u8, [u8; 64])]) {
    v.push(sigs.len() as u8);
    for (i, s) in sigs {
        v.push(*i);
        v.extend_from_slice(s);
    }
}
pub(crate) fn read_sigs(r: &mut crate::R) -> Option<Vec<(u8, [u8; 64])>> {
    let n = r.u8()? as usize;
    (n <= KEYS_MAX).then_some(())?;
    (0..n).map(|_| Some((r.u8()?, r.arr()?))).collect()
}

/// The most epochs one change seals.
pub const CLOSED_MAX: usize = 4096;
fn put_closed(v: &mut Vec<u8>, c: &[(u64, [u8; 32])]) {
    v.extend_from_slice(&(c.len() as u16).to_le_bytes());
    for (e, h) in c {
        v.extend_from_slice(&e.to_le_bytes());
        v.extend_from_slice(h);
    }
}
fn read_closed(r: &mut crate::R) -> Option<Vec<(u64, [u8; 32])>> {
    let n = u16::from_le_bytes(r.arr()?) as usize;
    (n <= CLOSED_MAX).then_some(())?;
    (0..n).map(|_| Some((r.u64()?, r.arr()?))).collect()
}

/// Which authority a change is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Mint = 0,
    Freeze = 1,
}

/// One CHANGE of an authority: at `seq`, the `role`'s set becomes `next` (none: dropped for good), from beacon `stamp`.
/// A MINT change also SEALS the supply before it ([`crate::supply`]): `closed` names, for every epoch since the last
/// mint change up to its own, the hash of that shard's mints stamped before it (an epoch not named: none) — so a set
/// once replaced can never add a mint to its past, not even to a shard nobody created yet. A FREEZE change instead
/// CAPS the freeze list ([`crate::freeze`]): a record stamped before it holds only up to seq `cap` (the highest the
/// replaced set issued) — and SEALS it: `closed` names every word stamped before it (`(seq, freeze::word_seal)`), so
/// the replaced set can never back-date a word in, not even on a node that never held the list (audit 10-10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub seq: u32,
    pub role: Role,
    pub next: Option<KeySet>,
    pub stamp: u64,
    pub closed: Vec<(u64, [u8; 32])>,
    pub cap: u32,
    pub sigs: Vec<(u8, [u8; 64])>,
}

impl Change {
    /// What the replaced set signs.
    pub fn message(&self, definition: &[u8; 32]) -> Vec<u8> {
        let mut v = b"craftworks token authority change".to_vec();
        v.extend_from_slice(definition);
        v.extend_from_slice(&self.seq.to_le_bytes());
        v.push(self.role as u8);
        put_opt(&mut v, &self.next);
        v.extend_from_slice(&self.stamp.to_le_bytes());
        put_closed(&mut v, &self.closed);
        v.extend_from_slice(&self.cap.to_le_bytes());
        v
    }
    fn encode(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(&self.seq.to_le_bytes());
        v.push(self.role as u8);
        put_opt(v, &self.next);
        v.extend_from_slice(&self.stamp.to_le_bytes());
        put_closed(v, &self.closed);
        v.extend_from_slice(&self.cap.to_le_bytes());
        put_sigs(v, &self.sigs);
    }
    fn read(r: &mut crate::R) -> Option<Change> {
        let seq = r.u32()?;
        let role = match r.u8()? {
            0 => Role::Mint,
            1 => Role::Freeze,
            _ => return None,
        };
        Some(Change { seq, role, next: read_opt(r)?, stamp: r.u64()?, closed: read_closed(r)?, cap: r.u32()?, sigs: read_sigs(r)? })
    }
    pub fn hash(&self) -> [u8; 32] {
        let mut v = Vec::new();
        self.encode(&mut v);
        *blake3::hash(&v).as_bytes()
    }
}

/// An authority chain's state: its changes, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chain {
    pub changes: Vec<Change>,
}

impl Chain {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = b"CWAU".to_vec();
        v.extend_from_slice(&(self.changes.len() as u16).to_le_bytes());
        for c in &self.changes {
            c.encode(&mut v);
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<Chain> {
        let b = b.strip_prefix(b"CWAU")?;
        let mut r = crate::R(b);
        let n = u16::from_le_bytes(r.arr()?) as usize;
        (n <= CHANGES_MAX).then_some(())?;
        let changes = (0..n).map(|_| Change::read(&mut r)).collect::<Option<Vec<_>>>()?;
        r.done().then_some(Chain { changes })
    }
}

/// A token's authority chain's PARAMS: the token code's hash (so its definition's id is checked), the definition, the
/// beacon (whose height stamps an update).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub code_hash: [u8; 32],
    pub definition: [u8; 32],
    pub beacon: [u8; 32],
}

impl Params {
    pub fn encode(&self) -> Vec<u8> {
        [&[AUTHORITY][..], &self.code_hash, &self.definition, &self.beacon].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Params> {
        let mut r = crate::R(b);
        (r.u8()? == AUTHORITY).then_some(())?;
        let p = Params { code_hash: r.arr()?, definition: r.arr()?, beacon: r.arr()? };
        r.done().then_some(p)
    }
    pub fn id(&self) -> [u8; 32] {
        crate::id(&self.code_hash, &self.encode())
    }
}

/// The sets current after `chain`'s changes stamped at or before `stamp` — (mint, freeze), starting from the
/// definition's (`first`). `Err`: the chain does not hold (a change not signed by the set it replaces, a change of a
/// dropped one, seqs or stamps out of order).
pub fn sets_at(definition: &[u8; 32], first: (&Option<KeySet>, &Option<KeySet>), chain: &Chain, stamp: u64) -> Result<(Option<KeySet>, Option<KeySet>), Why> {
    // `live`: every change applied (what signs the next one); `at`: only those stamped by `stamp`.
    let mut live = [first.0.clone(), first.1.clone()];
    let mut at = live.clone();
    let mut last = 0u64;
    for (i, c) in chain.changes.iter().enumerate() {
        if c.seq as usize != i {
            return Err("an authority change out of place");
        }
        if c.stamp < last {
            return Err("an authority change stamped before the one before it");
        }
        last = c.stamp;
        if c.next.as_ref().is_some_and(|s| !s.sound()) {
            return Err("an authority set that cannot sign");
        }
        if c.role == Role::Mint && c.cap != 0 {
            return Err("a mint change capping freezes");
        }
        let r = c.role as usize;
        let Some(set) = live[r].as_ref() else { return Err("a change of an authority dropped for good") };
        if !set.signed(&c.message(definition), &c.sigs) {
            return Err("an authority change not signed by the set it replaces");
        }
        live[r] = c.next.clone();
        if c.stamp <= stamp {
            at[r] = c.next.clone();
        }
    }
    let [mint, freeze] = at;
    Ok((mint, freeze))
}

/// An UPDATE's rule: every change the merged chain holds that the held one did not is stamped within the finality
/// margin of `now` — an authority cannot reach back past the margin (a mint checked then stays checked).
pub fn fresh(held: &Chain, merged: &Chain, now: u64, margin: u64) -> Result<(), Why> {
    for c in &merged.changes {
        if !held.changes.contains(c) && (c.stamp > now || c.stamp + margin < now) {
            return Err("an authority change stamped outside the margin of now");
        }
    }
    Ok(())
}

/// The authority chain's MERGE (`a`: the copy HELD): only ever EXTENDED — `b` is taken only where it holds all of
/// `a` and more after it. A change at a seq already held is never replaced: a set's word, once taken, stands (else a
/// replaced or dropped set whose key leaked could sign a second change at a past seq and win it — by hash — taking
/// the authority back, audit 10-10). Two different changes at one seq are the replaced set's equivocation; each
/// holder keeps the one it took first.
pub fn merge(a: &Chain, b: &Chain) -> Chain {
    if b.changes.len() > a.changes.len() && b.changes.starts_with(&a.changes) {
        b.clone()
    } else {
        a.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const DEF: [u8; 32] = [5; 32];
    fn sk(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n; 32])
    }
    fn set(k: u8, ns: &[u8]) -> KeySet {
        KeySet { k, keys: ns.iter().map(|n| sk(*n).verifying_key().to_bytes()).collect() }
    }
    /// A change at `seq`, signed by the keys at `by` (indexes into `of`, whose seeds are `seeds`).
    fn change(seq: u32, role: Role, next: Option<KeySet>, stamp: u64, seeds: &[u8], by: &[u8]) -> Change {
        let mut c = Change { seq, role, next, stamp, closed: vec![], cap: 0, sigs: vec![] };
        let m = c.message(&DEF);
        c.sigs = by.iter().map(|i| (*i, sk(seeds[*i as usize]).sign(&m).to_bytes())).collect();
        c
    }

    #[test]
    fn k_of_n_needs_k_distinct_keys_of_the_set() {
        let s = set(2, &[1, 2, 3]);
        let m = b"x";
        let sig = |n: u8| sk(n).sign(m).to_bytes();
        assert!(s.signed(m, &[(0, sig(1)), (2, sig(3))]));
        assert!(!s.signed(m, &[(0, sig(1))]), "one of two");
        assert!(!s.signed(m, &[(0, sig(1)), (0, sig(1))]), "the same key twice");
        assert!(!s.signed(m, &[(0, sig(1)), (1, sig(4))]), "a key outside the set");
        assert!(!s.signed(m, &[(0, sig(1)), (5, sig(3))]), "an index outside the set");
        assert!(!set(0, &[1]).sound() && !set(2, &[1]).sound() && !set(2, &[1, 1]).sound());
    }

    #[test]
    fn a_change_signed_by_the_current_set_moves_it_from_its_stamp() {
        let first = (Some(set(2, &[1, 2, 3])), Some(set(1, &[9])));
        let c0 = change(0, Role::Mint, Some(set(1, &[4])), 10, &[1, 2, 3], &[0, 1]);
        let c1 = change(1, Role::Mint, None, 20, &[4], &[0]);
        let chain = Chain { changes: vec![c0, c1] };
        assert_eq!(Chain::decode(&chain.encode()).as_ref(), Some(&chain));
        let at = |t| sets_at(&DEF, (&first.0, &first.1), &chain, t).unwrap();
        assert_eq!(at(9).0, first.0, "before the change: the definition's set");
        assert_eq!(at(10).0, Some(set(1, &[4])), "from its stamp: the new set");
        assert_eq!(at(20).0, None, "dropped");
        assert_eq!(at(20).1, first.1, "the freeze set untouched");
    }

    #[test]
    fn a_change_not_signed_by_the_set_it_replaces_or_of_a_dropped_one_does_not_hold() {
        let first = (Some(set(2, &[1, 2, 3])), None);
        let one = Chain { changes: vec![change(0, Role::Mint, Some(set(1, &[4])), 10, &[1, 2, 3], &[0])] };
        assert!(sets_at(&DEF, (&first.0, &first.1), &one, 99).is_err(), "one of two");
        let freeze = Chain { changes: vec![change(0, Role::Freeze, Some(set(1, &[4])), 10, &[1, 2, 3], &[0, 1])] };
        assert!(sets_at(&DEF, (&first.0, &first.1), &freeze, 99).is_err(), "the freeze authority was never named");
        let c0 = change(0, Role::Mint, None, 10, &[1, 2, 3], &[0, 1]);
        let back = Chain { changes: vec![c0.clone(), change(1, Role::Mint, Some(set(1, &[1])), 11, &[1, 2, 3], &[0, 1])] };
        assert!(sets_at(&DEF, (&first.0, &first.1), &back, 99).is_err(), "dropped is for good");
        let early = Chain { changes: vec![change(0, Role::Mint, Some(set(1, &[4])), 10, &[1, 2, 3], &[0, 1]), change(1, Role::Mint, None, 9, &[4], &[0])] };
        assert!(sets_at(&DEF, (&first.0, &first.1), &early, 99).is_err(), "stamped before the one before it");
        let skipped = Chain { changes: vec![change(1, Role::Mint, Some(set(1, &[4])), 10, &[1, 2, 3], &[0, 1])] };
        assert_eq!(sets_at(&DEF, (&first.0, &first.1), &skipped, 99), Err("an authority change out of place"));
        let mut sealing = change(0, Role::Freeze, Some(set(1, &[4])), 10, &[1, 2, 3], &[0, 1]);
        sealing.closed = vec![(0, [1; 32])];
        let resigned = |mut c: Change| {
            let m = c.message(&DEF);
            c.sigs = [0u8, 1].iter().map(|i| (*i, sk([1, 2, 3][*i as usize]).sign(&m).to_bytes())).collect();
            c
        };
        let first_f = (None, Some(set(2, &[1, 2, 3])));
        assert!(sets_at(&DEF, (&first_f.0, &first_f.1), &Chain { changes: vec![resigned(sealing)] }, 99).is_ok(), "a freeze change seals its list's words (freeze::word_seal)");
        let mut capping = change(0, Role::Mint, Some(set(1, &[4])), 10, &[1, 2, 3], &[0, 1]);
        capping.cap = 3;
        assert_eq!(sets_at(&DEF, (&first.0, &first.1), &Chain { changes: vec![resigned(capping)] }, 99), Err("a mint change capping freezes"));
        let mut capped = change(0, Role::Freeze, Some(set(1, &[4])), 10, &[1, 2, 3], &[0, 1]);
        capped.cap = 3;
        let signed_cap = resigned(capped.clone());
        let mut lifted = signed_cap.clone();
        lifted.cap = 4;
        assert!(sets_at(&DEF, (&first_f.0, &first_f.1), &Chain { changes: vec![signed_cap] }, 99).is_ok());
        assert_eq!(sets_at(&DEF, (&first_f.0, &first_f.1), &Chain { changes: vec![lifted] }, 99), Err("an authority change not signed by the set it replaces"), "the cap is signed");
        let mut tampered = change(0, Role::Mint, Some(set(1, &[4])), 10, &[1, 2, 3], &[0, 1]);
        tampered.closed = vec![(0, [1; 32])];
        assert_eq!(sets_at(&DEF, (&first.0, &first.1), &Chain { changes: vec![tampered] }, 99), Err("an authority change not signed by the set it replaces"), "the seal is signed");
        let unsound = Chain { changes: vec![change(0, Role::Mint, Some(set(2, &[4])), 10, &[1, 2, 3], &[0, 1])] };
        assert_eq!(sets_at(&DEF, (&first.0, &first.1), &unsound, 99), Err("an authority set that cannot sign"));
    }

    #[test]
    fn an_update_cannot_reach_back_past_the_margin_and_a_held_change_stands() {
        let a = Chain { changes: vec![change(0, Role::Mint, Some(set(1, &[4])), 10, &[1, 2, 3], &[0, 1])] };
        let b = Chain { changes: vec![change(0, Role::Mint, Some(set(1, &[5])), 10, &[1, 2, 3], &[0, 1])] };
        assert!(fresh(&Chain::default(), &a, 11, 2).is_ok());
        assert!(fresh(&Chain::default(), &a, 13, 2).is_err(), "older than the margin");
        assert!(fresh(&Chain::default(), &a, 9, 2).is_err(), "from the future");
        assert!(fresh(&a, &a, 99, 2).is_ok(), "a held change is not re-stamped");
        // Two changes at one seq (the replaced set signed both — its key leaked): whichever is HELD stands, whatever
        // the hashes; the other never replaces it (audit 10-10: else the leaked key retakes the authority).
        assert_eq!(merge(&a, &b), a);
        assert_eq!(merge(&b, &a), b);
        let longer = Chain { changes: vec![a.changes[0].clone(), change(1, Role::Mint, None, 12, &[4], &[0])] };
        assert_eq!(merge(&a, &longer), longer, "extended");
        assert_eq!(merge(&longer, &a), longer, "never shortened");
        let forked = Chain { changes: vec![b.changes[0].clone(), change(1, Role::Mint, None, 12, &[5], &[0])] };
        assert_eq!(merge(&a, &forked), a, "a longer copy that does not hold what is held: refused");
    }
}
