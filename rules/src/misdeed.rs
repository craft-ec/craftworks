//! THE MISDEED REGISTRY (TOKEN §7d decision 3): a CHAINED public bag per standing epoch (`craftworks_bag_contract::
//! chained`, as the vouch and keepers registries) of SELF-PROVING misdeed proofs — anyone submits one, nobody need be
//! online. Every statement build reads the registries of the epochs before it (`statement::Inputs::misdeeds`) and the
//! reputation rule (`craftworks_reputation::misdeed`) zeroes each proven cheat and charges its direct vouchers.
//!
//! A proof holds STANDALONE (from its bytes and the account's key log — never a chain that may be cut or forked later,
//! so every builder at any later time decides it alike):
//! - **EQUIVOCATION** (the token's own check, `witness::equivocation`): two witness signatures by one key at one
//!   (chain, seq) naming different entries — with the CREDENTIAL by which the account's owner key (at `key_seq` of its
//!   key log) made that key one of its devices: the misdeed is the account's.
//!
//! (A FALSE CREDIT — `Body::Misdeed`, a credit whose cited debit is not in the cited chain — is NOT standalone: its
//! absence is shown only by reading the cited chain whole and unforked, and chains are cut; a later builder may no
//! longer be able to show it. It burns the witness's stake as before; reaching reputation needs a carried mark — TOKEN
//! §8.)
use craftworks_reputation::Did;
use craftworks_token_contract::witness::{self, WitnessSig};
use std::collections::BTreeMap;

/// Registry bag `k` of standing epoch `epoch`.
pub fn registry_address(epoch: u64, k: u32) -> [u8; 32] {
    craftworks_bag_contract::chained::address("craftworks 2026-10-11 misdeed registry", epoch, k)
}

/// An EQUIVOCATION proof against account `did`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Equivocation {
    pub did: Did,
    /// The credential naming `key` a device of `did` (its owner key at `key_seq`).
    pub cred: Vec<u8>,
    pub key_seq: u32,
    pub key: [u8; 32],
    pub a: WitnessSig,
    pub b: WitnessSig,
}

const MAGIC: &[u8; 4] = b"CWMD";
const EQUIVOCATION: u8 = 1;
const SIG: usize = 32 + 8 + 32 + 8 + 64;

fn put_sig(v: &mut Vec<u8>, s: &WitnessSig) {
    v.extend_from_slice(&s.chain);
    v.extend_from_slice(&s.seq.to_le_bytes());
    v.extend_from_slice(&s.entry);
    v.extend_from_slice(&s.at.to_le_bytes());
    v.extend_from_slice(&s.sig);
}
fn read_sig(b: &[u8]) -> Option<WitnessSig> {
    (b.len() == SIG).then(|| WitnessSig {
        chain: b[..32].try_into().unwrap(),
        seq: u64::from_le_bytes(b[32..40].try_into().unwrap()),
        entry: b[40..72].try_into().unwrap(),
        at: u64::from_le_bytes(b[72..80].try_into().unwrap()),
        sig: b[80..].try_into().unwrap(),
    })
}

impl Equivocation {
    pub fn encode(&self) -> Vec<u8> {
        let n = u16::try_from(self.cred.len()).unwrap_or(u16::MAX);
        let mut v = [&MAGIC[..], &[EQUIVOCATION], &self.did, &self.key_seq.to_le_bytes(), &n.to_le_bytes(), &self.cred[..n as usize], &self.key].concat();
        put_sig(&mut v, &self.a);
        put_sig(&mut v, &self.b);
        v
    }
    /// Exact: trailing bytes refused.
    pub fn decode(b: &[u8]) -> Option<Equivocation> {
        let b = b.strip_prefix(MAGIC)?.strip_prefix(&[EQUIVOCATION])?;
        let did: Did = b.get(..32)?.try_into().ok()?;
        let key_seq = u32::from_le_bytes(b.get(32..36)?.try_into().ok()?);
        let n = u16::from_le_bytes(b.get(36..38)?.try_into().ok()?) as usize;
        let cred = b.get(38..38 + n)?.to_vec();
        let key: [u8; 32] = b.get(38 + n..70 + n)?.try_into().ok()?;
        let rest = b.get(70 + n..)?;
        if rest.len() != 2 * SIG {
            return None;
        }
        Some(Equivocation { did, cred, key_seq, key, a: read_sig(&rest[..SIG])?, b: read_sig(&rest[SIG..])? })
    }
    /// HOLDS against `did`'s key log: `key` is a device of `did` by its credential, and it signed two different entries
    /// at one place.
    pub fn holds(&self, log: &craftworks_idlog_contract::Log) -> bool {
        craftworks_identity::credential_node(&self.cred, &self.did, log.events.get(self.key_seq as usize).map(|e| e.key)).is_ok_and(|k| k == self.key) && witness::equivocation(&self.key, &self.a, &self.b)
    }
}

/// The accounts the registry's items accuse (whose key logs a build reads).
pub fn accused(items: &[(u64, Vec<u8>)]) -> impl Iterator<Item = Did> + '_ {
    items.iter().filter_map(|(_, b)| Equivocation::decode(b)).map(|e| e.did)
}

/// PROVEN: each account with a proof that HOLDS among `items` (each `(the registry epoch it was in, its bytes)`), and the
/// earliest such epoch. An item that does not decode or does not hold (`log_of`: `None` — no key log) counts for
/// nothing. Order-independent.
pub fn proven(items: &[(u64, Vec<u8>)], log_of: &dyn Fn(&Did) -> Option<craftworks_idlog_contract::Log>) -> BTreeMap<Did, u64> {
    let mut out: BTreeMap<Did, u64> = BTreeMap::new();
    for (e, b) in items {
        let Some(x) = Equivocation::decode(b) else { continue };
        if !log_of(&x.did).is_some_and(|l| x.holds(&l)) {
            continue;
        }
        let p = out.entry(x.did).or_insert(*e);
        *p = (*p).min(*e);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};

    pub(crate) fn owner_seed(n: u8) -> [u8; 32] {
        [n.wrapping_add(100); 32]
    }
    pub(crate) fn log(n: u8) -> craftworks_idlog_contract::Log {
        let next = SigningKey::from_bytes(&[n.wrapping_add(150); 32]).verifying_key().to_bytes();
        let e = craftworks_idlog_contract::Event::signed(0, [0; 32], &owner_seed(n), craftworks_idlog_contract::commit(&next), [n; 32], [n; 32], vec![]).unwrap();
        craftworks_idlog_contract::Log { events: vec![e] }
    }
    pub(crate) fn did(n: u8) -> Did {
        log(n).events[0].id()
    }
    pub(crate) fn device(n: u8) -> SigningKey {
        SigningKey::from_bytes(&[n.wrapping_add(10); 32])
    }
    pub(crate) fn wsig(k: &SigningKey, chain: u8, seq: u64, entry: u8, at: u64) -> WitnessSig {
        let (chain, entry) = ([chain; 32], [entry; 32]);
        WitnessSig { chain, seq, entry, at, sig: k.sign(&WitnessSig::message(&chain, seq, &entry, at)).to_bytes() }
    }
    /// Person `n`'s device signed two entries at one place: the proof against `n`.
    pub(crate) fn equivocation(n: u8) -> Equivocation {
        let k = device(n);
        let key = k.verifying_key().to_bytes();
        Equivocation { did: did(n), cred: craftworks_identity::credential(&did(n), &key, &[], &owner_seed(n)), key_seq: 0, key, a: wsig(&k, 9, 4, 1, 100), b: wsig(&k, 9, 4, 2, 101) }
    }
    pub(crate) fn logs(d: &Did) -> Option<craftworks_idlog_contract::Log> {
        (1..=9).find(|n| did(*n) == *d).map(log)
    }

    #[test]
    fn an_equivocation_proves_its_account_and_nothing_else_does() {
        let x = equivocation(1);
        assert_eq!(Equivocation::decode(&x.encode()), Some(x.clone()));
        assert_eq!(Equivocation::decode(&[x.encode(), vec![0]].concat()), None, "trailing bytes");
        assert!(x.holds(&log(1)));
        // NOT PROVING: the same entry twice; two places; another key's signature; the credential of another account;
        // the key claimed for an account whose owner never credentialed it; an owner key not at that seq.
        let k = device(1);
        let not = [
            Equivocation { b: x.a.clone(), ..x.clone() },
            Equivocation { b: wsig(&k, 9, 5, 2, 101), ..x.clone() },
            Equivocation { b: wsig(&device(2), 9, 4, 2, 101), ..x.clone() },
            Equivocation { cred: equivocation(2).cred, ..x.clone() },
            Equivocation { did: did(3), ..x.clone() },
            Equivocation { key_seq: 1, ..x.clone() },
        ];
        for (i, n) in not.iter().enumerate() {
            assert!(!n.holds(&logs(&n.did).unwrap()), "case {i} proved");
        }
        // PROVEN: the earliest registry epoch with a holding proof; junk, forgeries and unknown accounts ignored; any order.
        let mut items: Vec<(u64, Vec<u8>)> = vec![(7, x.encode()), (5, x.encode()), (2, b"junk".to_vec()), (1, not[4].encode()), (3, equivocation(2).encode())];
        let want: BTreeMap<Did, u64> = [(did(1), 5), (did(2), 3)].into_iter().collect();
        assert_eq!(proven(&items, &logs), want);
        items.reverse();
        assert_eq!(proven(&items, &logs), want);
        assert_eq!(proven(&items, &|d| (*d == did(1)).then(|| log(1))), [(did(1), 5)].into_iter().collect(), "no key log: not proven");
        assert_ne!(registry_address(4, 0), registry_address(4, 1));
        assert_ne!(registry_address(4, 0), craftworks_reputation::item::registry_address(4, 0));
    }
}
