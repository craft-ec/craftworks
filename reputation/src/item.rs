//! A SIGNED VOUCH (docs/REPUTATION.md, TOKEN §7d "the two rules"): "`from` vouches for `to`" — or withdraws it — at
//! beacon height `at`, signed by one of `from`'s DEVICES (its node key, the credential its owner key gave it at key log
//! seq `key_seq`). Signed, it is the row in the voucher's public vouch table AND an item ANYONE may relay into the
//! epoch's vouch registry (a bag) — the voucher need not be online. Whoever computes reputation verifies every item
//! against the voucher's key log; a forged item counts for nothing.
use crate::{Did, Vouch};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

/// The VOUCH REGISTRY of standing epoch `epoch`, bag `k` (CHAINED: `bag::chained` — audit 10-11: one bag an epoch
/// let anyone with more work evict every real vouch from it): where anyone relays the signed vouches made in it.
pub fn registry_address(epoch: u64, k: u32) -> [u8; 32] {
    craftworks_bag_contract::chained::address("craftworks 2026-10-11 vouch registry", epoch, k)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub from: Did,
    pub to: Did,
    pub at: u64,
    /// `true`: vouches; `false`: withdraws.
    pub on: bool,
    pub cred: Vec<u8>,
    pub key_seq: u32,
    pub sig: [u8; 64],
}

impl Item {
    /// What the device signs: under the identity's vouch domain.
    pub fn message(from: &Did, to: &Did, at: u64, on: bool) -> Vec<u8> {
        [craftworks_identity::VOUCH_DOMAIN, from, to, &at.to_le_bytes(), &[on as u8]].concat()
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut v = b"CWVO".to_vec();
        v.extend_from_slice(&self.from);
        v.extend_from_slice(&self.to);
        v.extend_from_slice(&self.at.to_le_bytes());
        v.push(self.on as u8);
        v.extend_from_slice(&(self.cred.len() as u16).to_le_bytes());
        v.extend_from_slice(&self.cred);
        v.extend_from_slice(&self.key_seq.to_le_bytes());
        v.extend_from_slice(&self.sig);
        v
    }
    pub fn decode(b: &[u8]) -> Option<Item> {
        let b = b.strip_prefix(b"CWVO")?;
        let arr = |b: &[u8], i: usize, n: usize| b.get(i..i + n).map(<[u8]>::to_vec);
        let from: Did = arr(b, 0, 32)?.try_into().ok()?;
        let to: Did = arr(b, 32, 32)?.try_into().ok()?;
        let at = u64::from_le_bytes(arr(b, 64, 8)?.try_into().ok()?);
        let on = match *b.get(72)? {
            0 => false,
            1 => true,
            _ => return None,
        };
        let n = u16::from_le_bytes(arr(b, 73, 2)?.try_into().ok()?) as usize;
        let cred = arr(b, 75, n)?;
        let key_seq = u32::from_le_bytes(arr(b, 75 + n, 4)?.try_into().ok()?);
        let sig: [u8; 64] = arr(b, 79 + n, 64)?.try_into().ok()?;
        (b.len() == 143 + n).then_some(Item { from, to, at, on, cred, key_seq, sig })
    }
    /// Holds against `from`'s KEY LOG: its credential is `from`'s, signed by the owner key at `key_seq`, and the item is
    /// signed by the node key that credential names.
    pub fn holds(&self, log: &craftworks_idlog_contract::Log) -> bool {
        if self.from == self.to {
            return false;
        }
        let Ok(node) = craftworks_identity::credential_node(&self.cred, &self.from, log.events.get(self.key_seq as usize).map(|e| e.key)) else { return false };
        let (Ok(vk), Ok(sig)) = (VerifyingKey::from_bytes(&node), Signature::from_slice(&self.sig)) else { return false };
        vk.verify(&Item::message(&self.from, &self.to, self.at, self.on), &sig).is_ok()
    }
}

/// The VOUCHES standing at height `now` from verified items: for each (from, to) its LATEST item made by then (by
/// height, then a withdrawal last) — a vouch if it is one, at the height it was first made since its last withdrawal
/// (the ramp counts from there). `log_of`: each voucher's key log (`None`: its items do not count).
pub fn standing(items: &[Item], now: u64, log_of: &dyn Fn(&Did) -> Option<craftworks_idlog_contract::Log>) -> Vec<Vouch> {
    let mut ok: Vec<&Item> = items.iter().filter(|i| i.at <= now && log_of(&i.from).is_some_and(|l| i.holds(&l))).collect();
    ok.sort_by_key(|i| (i.from, i.to, i.at, i.on as u8 ^ 1));
    let mut out: Vec<Vouch> = Vec::new();
    let mut cur: Option<(Did, Did, Option<u64>)> = None;
    let flush = |c: Option<(Did, Did, Option<u64>)>, out: &mut Vec<Vouch>| {
        if let Some((from, to, Some(at))) = c {
            out.push(Vouch { from, to, at });
        }
    };
    for i in ok {
        match &mut cur {
            Some((f, t, since)) if *f == i.from && *t == i.to => {
                *since = if i.on { since.or(Some(i.at)) } else { None };
            }
            _ => {
                flush(cur.take(), &mut out);
                cur = Some((i.from, i.to, i.on.then_some(i.at)));
            }
        }
    }
    flush(cur, &mut out);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

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
    /// A vouch item by person `n`'s device (key `[n+10; 32]`).
    fn item(n: u8, to: u8, at: u64, on: bool) -> Item {
        let dev = SigningKey::from_bytes(&[n + 10; 32]);
        let cred = craftworks_identity::credential(&did(n), &dev.verifying_key().to_bytes(), &[], &owner_seed(n));
        let sig = dev.sign(&Item::message(&did(n), &did(to), at, on)).to_bytes();
        Item { from: did(n), to: did(to), at, on, cred, key_seq: 0, sig }
    }
    pub(crate) fn logs(d: &Did) -> Option<craftworks_idlog_contract::Log> {
        (1..=9).find(|n| did(*n) == *d).map(log)
    }

    #[test]
    fn a_signed_vouch_round_trips_and_holds_against_the_vouchers_key_log() {
        let i = item(1, 2, 50, true);
        assert_eq!(Item::decode(&i.encode()), Some(i.clone()));
        let mut longer = i.encode();
        longer.push(0);
        assert_eq!(Item::decode(&longer), None);
        assert!(i.holds(&log(1)));
        // Forged: another's key log, a changed target, a credential of another account, a self-vouch.
        assert!(!i.holds(&log(3)));
        assert!(!Item { to: did(4), ..i.clone() }.holds(&log(1)));
        assert!(!Item { cred: item(3, 2, 50, true).cred, ..i.clone() }.holds(&log(1)));
        assert!(!item(1, 1, 50, true).holds(&log(1)));
        assert!(!Item { key_seq: 1, ..i.clone() }.holds(&log(1)), "an owner key not at that seq");
    }

    #[test]
    fn the_standing_vouches_are_each_pairs_latest_item_by_then() {
        let items = vec![item(1, 2, 10, true), item(1, 2, 30, false), item(1, 2, 40, true), item(1, 3, 20, true), item(4, 5, 5, true)];
        let at = |h| standing(&items, h, &logs);
        let pairs = |v: Vec<Vouch>| v.into_iter().map(|x| (x.from, x.to, x.at)).collect::<Vec<_>>();
        // At 25: 1→2 since 10, 1→3 since 20, 4→5.
        let mut want = vec![(did(1), did(2), 10), (did(1), did(3), 20), (did(4), did(5), 5)];
        want.sort();
        let mut got = pairs(at(25));
        got.sort();
        assert_eq!(got, want);
        // At 35: 1→2 withdrawn.
        assert!(!pairs(at(35)).iter().any(|p| p.0 == did(1) && p.1 == did(2)));
        // At 45: vouched again — its ramp from 40, not 10.
        assert!(pairs(at(45)).contains(&(did(1), did(2), 40)));
        // Vouched again while still vouched: the earlier height stands (no ramp reset).
        let again = vec![item(1, 3, 20, true), item(1, 3, 25, true)];
        assert_eq!(pairs(standing(&again, 30, &logs)), vec![(did(1), did(3), 20)]);
        // A forged item (signed by another key) counts for nothing.
        let mut forged = item(1, 6, 10, true);
        forged.sig = item(2, 6, 10, true).sig;
        assert!(standing(&[forged], 100, &logs).is_empty());
        // A voucher whose key log is not known: its items do not count.
        assert!(standing(&[item(9, 2, 10, true)], 100, &|_| None).is_empty());
    }
}
