//! THE KEEPERS REGISTRY (TOKEN §7a, K0b): who keeps in standing epoch `epoch` — each keeper's ACCOUNT (its DID) and the
//! credential making its standing-record key (a member key) a device of that account, listed in a chain of public bags
//! (`craftworks_bag_contract::chained`). A listing proves nothing kept: the standing record does (its answers); the
//! registry makes keepers FINDABLE (the statement's `kept`, the assignment's rendezvous) and ties a record to an
//! account. Anyone may list a real credential; one that does not hold against the account's key log is not listed.

use crate::Did;

/// One keeper listed: its account, and the credential (its owner key at `key_seq` of the account's key log) naming
/// its record's key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keeper {
    pub did: Did,
    pub cred: Vec<u8>,
    pub key_seq: u32,
}

const KEEPER: &[u8; 4] = b"CWKP";

impl Keeper {
    pub fn encode(&self) -> Vec<u8> {
        let n = u16::try_from(self.cred.len()).unwrap_or(u16::MAX);
        [&KEEPER[..], &self.did, &self.key_seq.to_le_bytes(), &n.to_le_bytes(), &self.cred[..n as usize]].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Keeper> {
        let rest = b.strip_prefix(KEEPER)?;
        let did: Did = rest.get(..32)?.try_into().ok()?;
        let key_seq = u32::from_le_bytes(rest.get(32..36)?.try_into().ok()?);
        let n = u16::from_le_bytes(rest.get(36..38)?.try_into().ok()?) as usize;
        let cred = rest.get(38..)?;
        (cred.len() == n).then(|| Keeper { did, cred: cred.to_vec(), key_seq })
    }
    /// Its record's KEY, when the credential holds against the account's key log (`None`: it does not).
    pub fn key(&self, log: &craftworks_idlog_contract::Log) -> Option<[u8; 32]> {
        craftworks_identity::credential_node(&self.cred, &self.did, log.events.get(self.key_seq as usize).map(|e| e.key)).ok()
    }
}

/// Registry bag `k` of standing epoch `epoch`.
pub fn address(epoch: u64, k: u32) -> [u8; 32] {
    craftworks_bag_contract::chained::address("craftworks 2026-10-10 keepers registry", epoch, k)
}

/// The KEEPERS listed in an epoch's registry items: `(account, record key)`, each key once, those whose credential
/// holds against their account's key log (`log_of`: `None` — not read, or no log — not listed).
pub fn listed(items: &[Vec<u8>], log_of: &dyn Fn(&Did) -> Option<craftworks_idlog_contract::Log>) -> Vec<(Did, [u8; 32])> {
    let mut out: Vec<(Did, [u8; 32])> = items.iter().filter_map(|b| Keeper::decode(b)).filter_map(|k| Some((k.did, k.key(&log_of(&k.did)?)?))).collect();
    out.sort_by_key(|x| x.1);
    out.dedup_by_key(|x| x.1);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::tests::{did, log, logs, owner_seed};
    use ed25519_dalek::SigningKey;

    /// Person `n`'s keeper: its device key `[n+10; 32]` credentialed by its owner key (seq 0).
    fn keeper(n: u8) -> (Keeper, [u8; 32]) {
        let key = SigningKey::from_bytes(&[n + 10; 32]).verifying_key().to_bytes();
        (Keeper { did: did(n), cred: craftworks_identity::credential(&did(n), &key, &[], &owner_seed(n)), key_seq: 0 }, key)
    }

    #[test]
    fn a_listed_keeper_names_its_account_and_record_key_and_a_forged_one_is_not_listed() {
        let (k1, key1) = keeper(1);
        let (k2, key2) = keeper(2);
        assert_eq!(Keeper::decode(&k1.encode()), Some(k1.clone()));
        assert_eq!(Keeper::decode(&k1.encode()[..k1.encode().len() - 1]), None);
        assert_eq!(k1.key(&log(1)), Some(key1));
        assert_eq!(k1.key(&log(3)), None, "another account's key log");
        // Listed: each real one once; a credential claimed for another account, an owner key not at that seq, or an
        // account whose log is not read — not listed.
        let stolen = Keeper { did: did(3), ..k1.clone() };
        let wrong_seq = Keeper { key_seq: 1, ..k2.clone() };
        let items = vec![k1.encode(), k2.encode(), k1.encode(), stolen.encode(), wrong_seq.encode(), b"junk".to_vec()];
        let mut want = vec![(did(1), key1), (did(2), key2)];
        want.sort_by_key(|x| x.1);
        assert_eq!(listed(&items, &logs), want);
        assert_eq!(listed(&items, &|d| (*d == did(1)).then(|| log(1))), vec![(did(1), key1)]);
        assert_ne!(address(4, 0), address(4, 1));
        assert_ne!(address(4, 0), address(5, 0));
    }
}
