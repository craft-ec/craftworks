//! STEALTH CHAINS (TOKEN §7f, owner 10-10: what an account holds is not linkable to its DID). A payment goes to a
//! ONE-TIME key P that only the sender and the recipient can tell is the recipient's: the sender derives it alone from
//! the recipient's published SCAN and SPEND keys and a random r (the recipient may be offline); the recipient finds it
//! from the announced R with its scan key, and spends it with b + h. The chain is a multisig-owned chain of one key
//! (`set_id({1, [P]})`): the token takes it as it is — no key log, nothing pointing at the DID.
//!
//!   S = s·G, B = b·G (the card's)    R = r·G    h = H(r·S) = H(s·R)    P = B + h·G    its key: b + h
//!
//! Pure: the caller brings the seed (the account's `wallet` table) and the randomness.

use craftworks_token_contract as t;
use curve25519_dalek::{constants::ED25519_BASEPOINT_TABLE as G, edwards::CompressedEdwardsY, EdwardsPoint, Scalar};

fn scalar(domain: &str, parts: &[&[u8]]) -> Scalar {
    let mut h = blake3::Hasher::new_derive_key(domain);
    for p in parts {
        h.update(p);
    }
    let mut wide = [0u8; 64];
    h.finalize_xof().fill(&mut wide);
    Scalar::from_bytes_mod_order_wide(&wide)
}

/// A point from its 32 bytes — one of prime order only (a small-order or mixed point is no one's key).
fn point(b: &[u8; 32]) -> Option<EdwardsPoint> {
    let p = CompressedEdwardsY(*b).decompress()?;
    p.is_torsion_free().then_some(p).filter(|p| !p.is_small_order())
}

/// A wallet's KEYS, from its seed: scan (s) and spend (b).
pub struct Keys {
    scan: Scalar,
    spend: Scalar,
}

/// What a wallet PUBLISHES (on the account's card): its scan key S and spend key B.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Public {
    pub scan: [u8; 32],
    pub spend: [u8; 32],
}

/// A PAYMENT's one-time destination: the announced R, the one-time key P, and the chain owner it pays (`to`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Paid {
    pub r: [u8; 32],
    pub key: [u8; 32],
    pub to: [u8; 32],
}

/// A one-time key this wallet FOUND as its own: its public key and its secret (b + h).
pub struct OneTime {
    pub key: [u8; 32],
    secret: Scalar,
}

/// The CHAIN OWNER a one-time key owns: its 1-of-1 key set's id.
pub fn owner(key: &[u8; 32]) -> [u8; 32] {
    t::set_id(&set(key))
}

/// The 1-of-1 KEY SET of a one-time key (what its chain's first spend names).
pub fn set(key: &[u8; 32]) -> t::authority::KeySet {
    t::authority::KeySet { k: 1, keys: vec![*key] }
}

fn shared(secret: &Scalar, other: &EdwardsPoint, r: &[u8; 32]) -> Scalar {
    scalar("craftworks stealth 2026-10-10 shared", &[(secret * other).compress().as_bytes(), r])
}

impl Keys {
    /// From the wallet SEED (32 random bytes, the account's).
    pub fn from_seed(seed: &[u8; 32]) -> Keys {
        Keys { scan: scalar("craftworks stealth 2026-10-10 scan", &[seed]), spend: scalar("craftworks stealth 2026-10-10 spend", &[seed]) }
    }

    pub fn public(&self) -> Public {
        Public { scan: (G * &self.scan).compress().to_bytes(), spend: (G * &self.spend).compress().to_bytes() }
    }

    /// Is the payment announced with `r` to `to` this wallet's? Its one-time key if so.
    pub fn find(&self, r: &[u8; 32], to: &[u8; 32]) -> Option<OneTime> {
        let h = shared(&self.scan, &point(r)?, r);
        let secret = self.spend + h;
        let key = (G * &secret).compress().to_bytes();
        (owner(&key) == *to).then_some(OneTime { key, secret })
    }
}

/// PAY `to` (its published keys) with the random bytes `rand`: the one-time destination, or `None` when its keys are
/// not keys (a card that does not read).
pub fn pay(to: &Public, rand: &[u8; 32]) -> Option<Paid> {
    let (scan, spend) = (point(&to.scan)?, point(&to.spend)?);
    let r = scalar("craftworks stealth 2026-10-10 r", &[rand]);
    let big_r = (G * &r).compress().to_bytes();
    let h = shared(&r, &scan, &big_r);
    let key = (spend + G * &h).compress().to_bytes();
    Some(Paid { r: big_r, key, to: owner(&key) })
}

impl OneTime {
    /// SIGN with the one-time key (ed25519: verified as any signature by `key`).
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        let esk = ed25519_dalek::hazmat::ExpandedSecretKey { scalar: self.secret, hash_prefix: *blake3::Hasher::new_derive_key("craftworks stealth 2026-10-10 nonce prefix").update(self.secret.as_bytes()).finalize().as_bytes() };
        let vk = ed25519_dalek::VerifyingKey::from_bytes(&self.key).expect("a one-time key is a point");
        ed25519_dalek::hazmat::raw_sign::<sha2::Sha512>(&esk, message, &vk).to_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;
    use t::{Auth, Body, ChainParams, Definition, Entry, State};

    #[test]
    fn only_the_recipient_finds_and_spends_a_payment() {
        let (alice, bob) = (Keys::from_seed(&[1; 32]), Keys::from_seed(&[2; 32]));
        let p = pay(&bob.public(), &[9; 32]).unwrap();
        // Bob finds it; Alice (another wallet) does not; nor does Bob under another announcement.
        let one = bob.find(&p.r, &p.to).expect("bob's");
        assert_eq!(one.key, p.key);
        assert!(alice.find(&p.r, &p.to).is_none());
        // The SCAN KEY is what links: Bob's spend key under anyone else's scan key finds nothing (an outsider holding
        // Bob's card — S and B — cannot compute h without s).
        assert!(Keys { scan: alice.scan, spend: bob.spend }.find(&p.r, &p.to).is_none());
        let q = pay(&bob.public(), &[10; 32]).unwrap();
        assert!(bob.find(&q.r, &p.to).is_none());
        // Two payments to the same wallet: different keys, different owners — nothing in common to the eye.
        assert_ne!((p.key, p.to, p.r), (q.key, q.to, q.r));
        assert_ne!(p.key, bob.public().spend);
        // Its signature is an ordinary ed25519 signature by P.
        let sig = one.sign(b"hello");
        assert!(ed25519_dalek::VerifyingKey::from_bytes(&p.key).unwrap().verify(b"hello", &ed25519_dalek::Signature::from_bytes(&sig)).is_ok());
        // A card whose keys are not points: no payment.
        assert!(pay(&Public { scan: [0; 32], spend: bob.public().spend }, &[9; 32]).is_none());
        // A small-order R announced: found by no one.
        let mut small = [0u8; 32];
        small[0] = 1; // the identity point
        assert!(bob.find(&small, &p.to).is_none());
        // A card whose spend key is small-order (the identity: P would be h·G, spendable by the SENDER): refused.
        assert!(pay(&Public { scan: bob.public().scan, spend: small }, &[9; 32]).is_none());
    }

    /// END TO END against the token's rules: Alice's stealth chain (the token's creator) begins, takes its genesis and
    /// SENDS to Bob's one-time owner; Bob's chain RECEIVES (worked, no signature) and SPENDS — its first spend naming
    /// its 1-of-1 set, signed by the key only Bob derives. Nothing in either chain names a DID.
    #[test]
    fn a_stealth_chain_the_token_takes() {
        let (alice, bob) = (Keys::from_seed(&[1; 32]), Keys::from_seed(&[2; 32]));
        let a_paid = pay(&alice.public(), &[4; 32]).unwrap();
        let a = alice.find(&a_paid.r, &a_paid.to).unwrap();
        let d = Definition { code_hash: [9; 32], decimals: 0, min_send: 1, genesis_owner: a_paid.to, genesis_amount: 100, name: b"W".to_vec(), nonce: [0; 16], beacon: [7; 32], margin: 2, idlog: [6; 32], mint: None, freeze: None, ids: t::Ids::None, royalty: None, cap: 0, item: [0; 32], version: [0; 32], soulbound: false, transfer_fee: None, sale: None, keepers: None };
        let tok = t::id(&d.code_hash, &d.encode());
        let ap = ChainParams { code_hash: d.code_hash, definition: tok, owner: a_paid.to, beacon: d.beacon, idlog: d.idlog, item: [0; 32] };
        let push = |s: &mut State, body: Body, balance: u128, stamp: u64, by: Option<&OneTime>| {
            let seq = s.entries.last().map_or(0, |e| e.seq + 1);
            let mut e = Entry { seq, prev: s.head(), nonce: [seq as u8 + 1; 16], stamp, body, balance, auth: Auth::Work(0) };
            let m = e.message();
            e.auth = match by {
                Some(k) => Auth::Multi(vec![(0, k.sign(&m))]),
                None => Auth::Work((0u64..).find(|w| t::work_ok(&m, *w)).unwrap()),
            };
            s.entries.push(e);
        };
        let ok = |p: &ChainParams, s: &State, others: &[(&[u8; 32], &State)]| t::check(p, &d, s, &|_, o| others.iter().find(|(w, _)| *w == o).map(|(_, s)| (*s).clone()), &|_| None, None, &t::Extra::NONE);
        let mut sa = State::default();
        push(&mut sa, Body::Members { set: set(&a.key), set_seq: 0 }, 0, 1, Some(&a));
        push(&mut sa, Body::Genesis { amount: 100 }, 100, 1, Some(&a));
        let paid = pay(&bob.public(), &[3; 32]).unwrap();
        push(&mut sa, Body::Send { to: paid.to, amount: 30 }, 70, 2, Some(&a));
        assert_eq!(ok(&ap, &sa, &[]), Ok(()));
        // Bob finds the payment from its announcement, receives it, and spends.
        let b = bob.find(&paid.r, &paid.to).expect("bob's");
        let bp = ChainParams { owner: paid.to, ..ap };
        let mut sb = State::default();
        let sent = sa.entries.last().unwrap().clone();
        push(&mut sb, Body::Receive { from: a_paid.to, nonce: sent.nonce, amount: 30, cited: 2, ev: t::Evidence::default() }, 30, 4, None);
        push(&mut sb, Body::Members { set: set(&b.key), set_seq: 0 }, 30, 4, Some(&b));
        push(&mut sb, Body::Send { to: a_paid.to, amount: 5 }, 25, 4, Some(&b));
        assert_eq!(ok(&bp, &sb, &[(&a_paid.to, &sa)]), Ok(()));
        // Bob's chain named by Alice's key (her set is not this chain's owner), or signed by her over Bob's set: refused.
        let mut wrong = sb.clone();
        wrong.entries.truncate(1);
        push(&mut wrong, Body::Members { set: set(&a.key), set_seq: 0 }, 30, 4, Some(&a));
        assert_eq!(ok(&bp, &wrong, &[(&a_paid.to, &sa)]), Err("a key set that is not the chain's owner"));
        let mut forged = sb.clone();
        forged.entries.truncate(1);
        push(&mut forged, Body::Members { set: set(&b.key), set_seq: 0 }, 30, 4, Some(&a));
        assert!(ok(&bp, &forged, &[(&a_paid.to, &sa)]).is_err());
    }
}
