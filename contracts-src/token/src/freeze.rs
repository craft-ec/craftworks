//! FREEZE / THAW (TOKEN §2): the token's FREEZE LIST — one contract per token (params [`Params`]), a record per frozen
//! (or thawed) account, each signed by the freeze authority current at its stamp. An APPEND rule: while an owner's
//! record says frozen, no new DEBIT of theirs is appended (credits still are). Nothing the owner signs can move it:
//! the list is not their chain. A token whose definition names no freeze authority never reads it.

use crate::authority::{self, KeySet};
use crate::Why;

pub const FROZEN: u8 = 5;
/// The account a record names to freeze EVERY account of the token at once (TOKEN §2: pause) — a whole-token switch,
/// the same word, signatures and rules as one account's.
pub const EVERYONE: [u8; 32] = [0xff; 32];
/// The most accounts one list names.
pub const RECORDS_MAX: usize = 2048;

/// The freeze authority's word on one ACCOUNT (a DID): frozen or not, its `seq`-th word (a later one replaces it),
/// at beacon `stamp`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub owner: [u8; 32],
    pub frozen: bool,
    pub seq: u32,
    pub stamp: u64,
    pub sigs: Vec<(u8, [u8; 64])>,
}

impl Record {
    pub fn message(&self, definition: &[u8; 32]) -> Vec<u8> {
        let mut v = b"craftworks token freeze".to_vec();
        v.extend_from_slice(definition);
        v.extend_from_slice(&self.owner);
        v.push(self.frozen as u8);
        v.extend_from_slice(&self.seq.to_le_bytes());
        v.extend_from_slice(&self.stamp.to_le_bytes());
        v
    }
    fn encode(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(&self.owner);
        v.push(self.frozen as u8);
        v.extend_from_slice(&self.seq.to_le_bytes());
        v.extend_from_slice(&self.stamp.to_le_bytes());
        authority::put_sigs(v, &self.sigs);
    }
    fn read(r: &mut crate::R) -> Option<Record> {
        let owner = r.arr()?;
        let frozen = match r.u8()? {
            0 => false,
            1 => true,
            _ => return None,
        };
        Some(Record { owner, frozen, seq: r.u32()?, stamp: r.u64()?, sigs: authority::read_sigs(r)? })
    }
    pub fn hash(&self) -> [u8; 32] {
        let mut v = Vec::new();
        self.encode(&mut v);
        *blake3::hash(&v).as_bytes()
    }
}

/// A freeze list's PARAMS: the token code's hash, the definition, the beacon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub code_hash: [u8; 32],
    pub definition: [u8; 32],
    pub beacon: [u8; 32],
}

impl Params {
    pub fn encode(&self) -> Vec<u8> {
        [&[FROZEN][..], &self.code_hash, &self.definition, &self.beacon].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Params> {
        let mut r = crate::R(b);
        (r.u8()? == FROZEN).then_some(())?;
        let p = Params { code_hash: r.arr()?, definition: r.arr()?, beacon: r.arr()? };
        r.done().then_some(p)
    }
    pub fn id(&self) -> [u8; 32] {
        crate::id(&self.code_hash, &self.encode())
    }
    pub fn authority(&self) -> authority::Params {
        authority::Params { code_hash: self.code_hash, definition: self.definition, beacon: self.beacon }
    }
}

/// A freeze list: one record per account, sorted by account.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct List {
    pub records: Vec<Record>,
}

impl List {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = b"CWFZ".to_vec();
        v.extend_from_slice(&(self.records.len() as u16).to_le_bytes());
        for r in &self.records {
            r.encode(&mut v);
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<List> {
        let mut r = crate::R(b.strip_prefix(b"CWFZ")?);
        let n = u16::from_le_bytes(r.arr()?) as usize;
        (n <= RECORDS_MAX).then_some(())?;
        let records = (0..n).map(|_| Record::read(&mut r)).collect::<Option<Vec<_>>>()?;
        (r.done() && records.windows(2).all(|w| w[0].owner < w[1].owner)).then_some(List { records })
    }
    /// Is `owner` frozen now?
    /// Was `owner` frozen at beacon `stamp`, as far as this list shows — its own or the whole token's latest word
    /// says frozen, from a stamp at or before it (a later word may have replaced an earlier one: a witness reads what
    /// the list says now).
    pub fn frozen_at(&self, owner: &[u8; 32], stamp: u64) -> bool {
        let says = |o: &[u8; 32]| self.records.binary_search_by(|r| r.owner.cmp(o)).is_ok_and(|i| self.records[i].frozen && self.records[i].stamp <= stamp);
        says(owner) || says(&EVERYONE)
    }
    /// Is `owner` frozen — its own record, or the whole token's ([`EVERYONE`])?
    pub fn frozen(&self, owner: &[u8; 32]) -> bool {
        let says = |o: &[u8; 32]| self.records.binary_search_by(|r| r.owner.cmp(o)).is_ok_and(|i| self.records[i].frozen);
        says(owner) || says(&EVERYONE)
    }
}

/// Every record signed by the freeze set current at its stamp, and within the CAP of the first freeze change after it.
/// A word's entry in a freeze change's SEAL: its seq, and the hash of its account and itself.
pub fn word_seal(r: &Record) -> (u64, [u8; 32]) {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-10 token freeze seal");
    h.update(&r.owner);
    h.update(&r.hash());
    (r.seq as u64, *h.finalize().as_bytes())
}

/// The SEAL a freeze change at `stamp` carries: every word of `l` stamped before it.
pub fn seal_words(l: &List, stamp: u64) -> Vec<(u64, [u8; 32])> {
    l.records.iter().filter(|r| r.stamp < stamp).map(word_seal).collect()
}

pub fn holds(p: &Params, first: (&Option<KeySet>, &Option<KeySet>), chain: &authority::Chain, l: &List) -> Result<(), Why> {
    for r in &l.records {
        // Every freeze change after it SEALED it (a word the replaced set back-dates is in no seal).
        if chain.changes.iter().any(|c| c.role == authority::Role::Freeze && c.stamp > r.stamp && !c.closed.contains(&word_seal(r))) {
            return Err("a freeze word that is not what its authority sealed");
        }
        if chain.changes.iter().find(|c| c.role == authority::Role::Freeze && c.stamp > r.stamp).is_some_and(|c| r.seq > c.cap) {
            return Err("a freeze past the cap of the change after it");
        }
        let (_, freeze) = authority::sets_at(&p.definition, first, chain, r.stamp)?;
        let set = freeze.ok_or("a freeze with no freeze authority")?;
        if !set.signed(&r.message(&p.definition), &r.sigs) {
            return Err("a freeze not signed by the freeze authority");
        }
    }
    Ok(())
}

/// The MERGE: per account, the higher seq (at one seq, two words — the authority signed both — the lower hash).
/// `None`: past [`RECORDS_MAX`].
pub fn merge(a: &List, b: &List) -> Option<List> {
    let mut all: Vec<Record> = a.records.iter().chain(&b.records).cloned().collect();
    all.sort_by_cached_key(|r| (r.owner, std::cmp::Reverse(r.seq), r.hash()));
    all.dedup_by(|later, first| later.owner == first.owner);
    (all.len() <= RECORDS_MAX).then_some(List { records: all })
}

/// An UPDATE's rule: a record the held list did not hold is stamped within the margin of `now`.
pub fn fresh(held: &List, merged: &List, now: u64, margin: u64) -> Result<(), Why> {
    for r in &merged.records {
        if !held.records.contains(r) && (r.stamp > now || r.stamp + margin < now) {
            return Err("a freeze stamped outside the margin of now");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const DEF: [u8; 32] = [5; 32];
    fn p() -> Params {
        Params { code_hash: [9; 32], definition: DEF, beacon: [7; 32] }
    }
    fn set(n: u8) -> KeySet {
        KeySet { k: 1, keys: vec![SigningKey::from_bytes(&[n; 32]).verifying_key().to_bytes()] }
    }
    fn word(owner: u8, frozen: bool, seq: u32, stamp: u64, by: u8) -> Record {
        let mut r = Record { owner: [owner; 32], frozen, seq, stamp, sigs: vec![] };
        r.sigs = vec![(0, SigningKey::from_bytes(&[by; 32]).sign(&r.message(&DEF)).to_bytes())];
        r
    }

    #[test]
    fn the_whole_token_is_paused_by_one_word_and_unpaused_by_the_next() {
        let first = (None, Some(set(1)));
        let none = authority::Chain::default();
        let mut all = Record { owner: EVERYONE, frozen: true, seq: 0, stamp: 5, sigs: vec![] };
        all.sigs = vec![(0, SigningKey::from_bytes(&[1; 32]).sign(&all.message(&DEF)).to_bytes())];
        let paused = merge(&List::default(), &List { records: vec![all] }).unwrap();
        assert_eq!(holds(&p(), (&first.0, &first.1), &none, &paused), Ok(()));
        assert!(paused.frozen(&[3; 32]) && paused.frozen(&[200; 32]), "every account");
        assert!(!List::default().frozen(&[3; 32]));
        let one = merge(&List::default(), &List { records: vec![word(3, true, 0, 5, 1)] }).unwrap();
        assert!(one.frozen(&[3; 32]) && !one.frozen(&[4; 32]), "one account only");
        let mut off = Record { owner: EVERYONE, frozen: false, seq: 1, stamp: 6, sigs: vec![] };
        off.sigs = vec![(0, SigningKey::from_bytes(&[1; 32]).sign(&off.message(&DEF)).to_bytes())];
        let unpaused = merge(&paused, &List { records: vec![off] }).unwrap();
        assert!(!unpaused.frozen(&[3; 32]), "the later word stands");
        let mut forged = Record { owner: EVERYONE, frozen: true, seq: 2, stamp: 7, sigs: vec![] };
        forged.sigs = vec![(0, SigningKey::from_bytes(&[2; 32]).sign(&forged.message(&DEF)).to_bytes())];
        assert_eq!(holds(&p(), (&first.0, &first.1), &none, &List { records: vec![forged] }), Err("a freeze not signed by the freeze authority"));
    }

    #[test]
    fn the_freeze_authority_alone_freezes_and_the_later_word_stands() {
        let first = (None, Some(set(1)));
        let none = authority::Chain::default();
        let l = merge(&List { records: vec![word(3, true, 0, 10, 1)] }, &List { records: vec![word(4, true, 0, 10, 1)] }).unwrap();
        assert_eq!(List::decode(&l.encode()).as_ref(), Some(&l));
        assert_eq!(holds(&p(), (&first.0, &first.1), &none, &l), Ok(()));
        assert!(l.frozen(&[3; 32]) && l.frozen(&[4; 32]) && !l.frozen(&[5; 32]));
        assert_eq!(holds(&p(), (&first.0, &first.1), &none, &List { records: vec![word(3, true, 0, 10, 2)] }), Err("a freeze not signed by the freeze authority"));
        assert_eq!(holds(&p(), (&None, &None), &none, &l), Err("a freeze with no freeze authority"));
        let thawed = merge(&l, &List { records: vec![word(3, false, 1, 11, 1)] }).unwrap();
        assert!(!thawed.frozen(&[3; 32]), "the thaw, a later word");
        assert_eq!(merge(&List { records: vec![word(3, false, 1, 11, 1)] }, &l), Some(thawed.clone()), "one answer either way");
        assert!(merge(&thawed, &List { records: vec![word(3, true, 0, 12, 1)] }).unwrap().records.contains(&word(3, false, 1, 11, 1)), "an older word does not come back");
        // The freeze set moves 1 → 2 at 20, SEALING key 1's one word then (account 6, seq 1): key 1 back-dating any other
        // word is refused — a pause of EVERYONE included, on a node that never held the list (audit 10-10).
        let sealed = word(6, true, 1, 15, 1);
        let mut c = authority::Change { seq: 0, role: authority::Role::Freeze, next: Some(set(2)), stamp: 20, closed: seal_words(&List { records: vec![sealed.clone()] }, 20), cap: 1, sigs: vec![] };
        c.sigs = vec![(0, SigningKey::from_bytes(&[1; 32]).sign(&c.message(&DEF)).to_bytes())];
        let moved = authority::Chain { changes: vec![c] };
        assert_eq!(holds(&p(), (&first.0, &first.1), &moved, &List { records: vec![sealed.clone()] }), Ok(()));
        let not_sealed = Err("a freeze word that is not what its authority sealed");
        assert_eq!(holds(&p(), (&first.0, &first.1), &moved, &List { records: vec![word(6, true, u32::MAX, 15, 1)] }), not_sealed);
        assert_eq!(holds(&p(), (&first.0, &first.1), &moved, &List { records: vec![word(6, false, 1, 15, 1)] }), not_sealed, "same seq, another word");
        assert_eq!(holds(&p(), (&first.0, &first.1), &moved, &List { records: vec![word(7, true, 0, 15, 1)] }), not_sealed, "another account");
        let mut pause = Record { owner: EVERYONE, frozen: true, seq: 0, stamp: 15, sigs: vec![] };
        pause.sigs = vec![(0, SigningKey::from_bytes(&[1; 32]).sign(&pause.message(&DEF)).to_bytes())];
        assert_eq!(holds(&p(), (&first.0, &first.1), &moved, &List { records: vec![pause] }), not_sealed, "a back-dated pause");
        // Key 2's word after it needs no seal, and replaces the sealed one.
        let over = merge(&List { records: vec![sealed] }, &List { records: vec![word(6, false, 2, 21, 2)] }).unwrap();
        assert_eq!(holds(&p(), (&first.0, &first.1), &moved, &over), Ok(()));
        assert!(!over.frozen(&[6; 32]), "the next set outranks it");
        assert!(fresh(&l, &thawed, 12, 2).is_ok());
        assert!(fresh(&l, &thawed, 14, 2).is_err(), "back-dated");
        assert!(fresh(&l, &thawed, 10, 2).is_err(), "from the future");
    }
}
