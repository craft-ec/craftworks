//! A person's ACCOUNT on the network (ARCHITECTURE §4): the DID, derived from RECOVERY WORDS.
//!
//! - **Words:** BIP39, 12 or 24 (16 or 32 bytes of entropy), the same words a deterministic wallet uses. The OWNER key
//!   is derived from them by SLIP-0010 (ed25519) at [`PATH`], a path of Craftworks' own, so words shared with a
//!   bitcoin wallet give an owner key unrelated to any of that wallet's keys.
//! - **Owner seat:** a Register under the owner key, label `seat`, holding one fixed record that names the owner's
//!   public key. Its contract id IS the DID (`did:craftec:<id>`). So the words alone name the account, and a reader
//!   holding only the DID learns the owner key from the seat and checks it: that key's seat must BE this DID.
//! - **Members:** a Set under the owner key (owner-only), label `members`. Each member (a device key) is its own item,
//!   keyed by the member's public key and signed by the owner key: two devices joining at once are two items, never a
//!   conflict. Leaving is a tombstone. DID → seat → owner key → members.
//! - The seat's record is the same bytes whoever writes it (ed25519 signs deterministically), so putting it again
//!   from another device, or when logging in with the words, is the same PUT and never a second record.
//!
//! The page makes the entropy (this crate has no randomness) and the identity delegate keeps it with the member, so
//! the Account page can show the words when the person sets up recovery.

use bip39::Mnemonic;
use craftec_register_contract::wire::{Params, RegState, Record, Signed};
use ed25519_dalek::{Signer, SigningKey};
use freenet_stdlib::prelude::{ContractContainer, WrappedState};
use craftec_set_contract::wire::{Admission, Item, Params as SetParams, SetState};
use hmac::{Hmac, Mac};
use sha2::Sha512;

pub const SEAT_LABEL: &[u8] = b"seat";
/// The seat's record: `ST01 ‖ owner public key`. Members live in the member Set, never here.
const SEAT_MAGIC: &[u8] = b"ST01";
/// The seat's record is written at seq 2: the first seats (2026-09-27) held a bare `ST01` at seq 1, and a different
/// record at the same seq would be a fork; seq 2 replaces them cleanly and is where every new seat starts.
const SEAT_SEQ: u64 = 2;
pub const MEMBERS_LABEL: &[u8] = b"members";
/// A member item's payload: `MB01 ‖ class ‖ name`. Class 0 = one of the person's own devices.
const MEMBER_MAGIC: &[u8] = b"MB01";
const MAX_NAME: usize = 48;
/// SLIP-0010 ed25519, every step hardened: m/44'/25458'/0' (25458 = "cr"; no coin's, so no wallet key collides).
pub const PATH: [u32; 3] = [44, 25458, 0];

/// One Register to PUT: its contract, its state, and the id the node names it by.
pub struct Put {
    pub id: String,
    pub id_bytes: [u8; 32],
    pub contract: ContractContainer,
    pub state: WrappedState,
}

/// The words for this entropy (16 or 32 bytes).
pub fn words(entropy: &[u8]) -> Option<String> {
    matches!(entropy.len(), 16 | 32).then_some(())?;
    Some(Mnemonic::from_entropy(entropy).ok()?.to_string())
}

/// The entropy of 12 or 24 words; `None` for anything that is not a valid BIP39 English phrase (checksum included).
pub fn entropy(words: &str) -> Option<Vec<u8>> {
    let m = Mnemonic::parse_normalized(&words.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()).ok()?;
    matches!(m.word_count(), 12 | 24).then(|| m.to_entropy())
}

fn hmac512(key: &[u8], parts: &[&[u8]]) -> [u8; 64] {
    let mut mac = Hmac::<Sha512>::new_from_slice(key).expect("HMAC takes any key");
    for p in parts {
        mac.update(p);
    }
    mac.finalize().into_bytes().into()
}

/// SLIP-0010 ed25519: the private key at `path` (every index hardened) from a BIP32 seed.
pub fn slip10(seed: &[u8], path: &[u32]) -> [u8; 32] {
    let i = hmac512(b"ed25519 seed", &[seed]);
    let (mut k, mut c) = (<[u8; 32]>::try_from(&i[..32]).unwrap(), <[u8; 32]>::try_from(&i[32..]).unwrap());
    for &n in path {
        let i = hmac512(&c, &[&[0], &k, &(n | 0x8000_0000).to_be_bytes()]);
        k.copy_from_slice(&i[..32]);
        c.copy_from_slice(&i[32..]);
    }
    k
}

/// The owner key's seed from the words' entropy (BIP39 seed with no passphrase, then [`PATH`]).
pub fn owner_seed(entropy: &[u8]) -> Option<[u8; 32]> {
    let m = Mnemonic::from_entropy(entropy).ok()?;
    Some(slip10(&m.to_seed_normalized(""), &PATH))
}

fn put(code: &[u8], params: &[u8], state: &[u8]) -> Put {
    let (id, contract, state) = wire::puts::contract(code, params, state);
    let id_bytes = contract.key().id().as_bytes().try_into().expect("a contract id is 32 bytes");
    Put { id, id_bytes, contract, state }
}

/// The owner seat for these words: the Register to PUT, whose id is the DID.
pub fn seat(register_code: &[u8], entropy: &[u8]) -> Option<Put> {
    let owner = SigningKey::from_bytes(&owner_seed(entropy)?);
    let public = owner.verifying_key().to_bytes();
    let params = wire::register_params(&public, SEAT_LABEL);
    let p = Params::parse(&params).expect("our own params parse");
    let value = [SEAT_MAGIC, &public[..]].concat();
    let value_hash = *blake3::hash(&value).as_bytes();
    let sig = owner.sign(&p.signed_message(false, SEAT_SEQ, &value_hash)).to_bytes();
    let signed = Signed { terminal: false, seq: SEAT_SEQ, value_hash, bitmap: 0, sigs: vec![sig] };
    let record = Record::new(signed, value).expect("a value within the cap, hashed above");
    let state = RegState { record: Some(record), evidence: None }.encode(&p.authority);
    Some(put(register_code, &params, &state))
}

/// The owner key a seat names, checked: the seat must verify under that key AND that key's seat must be `did`.
/// `None` for a state that is not this DID's seat.
pub fn owner_of_seat(register_code: &[u8], did: &[u8; 32], state: &[u8]) -> Option<[u8; 32]> {
    let (_, _, value) = craftec_register_contract::wire::record_head(state)?;
    let owner: [u8; 32] = value.strip_prefix(SEAT_MAGIC)?.try_into().ok()?;
    let params = wire::register_params(&owner, SEAT_LABEL);
    craftec_register_contract::read(&params, state)?;
    (put(register_code, &params, &[]).id_bytes == *did).then_some(owner)
}

/// The member Set's params for an owner key.
pub fn members_params(owner: &[u8; 32]) -> Option<Vec<u8>> {
    let p = SetParams {
        owner: ed25519_dalek::VerifyingKey::from_bytes(owner).ok()?,
        admission: Admission::OwnerOnly,
        m: 64,
        quota: 64,
        decision_bits: 0,
        payload_cap: (MEMBER_MAGIC.len() + 1 + MAX_NAME) as u16,
        bucket: 0,
        label: MEMBERS_LABEL.to_vec(),
    };
    Some(p.encode())
}

/// The member Set's address for an owner key (to GET it), with no state.
pub fn members_address(set_code: &[u8], owner: &[u8; 32]) -> Option<Put> {
    Some(put(set_code, &members_params(owner)?, &[]))
}

/// Admit `member` to the account of these words: the member Set holding its one owner-signed item. PUT to a Set that
/// exists merges into it (a union), so this is the same step for the first member and every later one.
pub fn admit(set_code: &[u8], entropy: &[u8], member: &[u8; 32], ts: u64, name: &str) -> Option<Put> {
    let owner = SigningKey::from_bytes(&owner_seed(entropy)?);
    let params = members_params(&owner.verifying_key().to_bytes())?;
    let p = SetParams::parse(&params)?;
    let name: String = name.chars().take(MAX_NAME).collect();
    let mut name = name.into_bytes();
    name.truncate(MAX_NAME);
    let mut item = Item {
        signer: owner.verifying_key(),
        item_key: member.to_vec(),
        ts,
        tombstone: false,
        sealed: false,
        payload: [MEMBER_MAGIC, &[0], &name].concat(),
        cap: None,
        stamp_nonce: [0; 8],
        sig: [0; 64],
    };
    item.sig = owner.sign(&item.signed_bytes(&p.hash())).to_bytes();
    let state = craftec_set_contract::collect([item], &p).encode();
    Some(put(set_code, &params, &state))
}

/// One member as the Set says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub key: [u8; 32],
    pub name: String,
    pub since: u64,
}

/// The members a member Set state names, every signature verified; `None` for a state that is not this owner's Set.
pub fn members(owner: &[u8; 32], state: &[u8]) -> Option<Vec<Member>> {
    let p = SetParams::parse(&members_params(owner)?)?;
    let s = SetState::parse(state, &p)?;
    Some(
        s.visible()
            .filter(|h| !h.item.tombstone && h.item.signer == p.owner)
            .filter_map(|h| {
                let rest = h.item.payload.strip_prefix(MEMBER_MAGIC)?;
                let (_class, name) = rest.split_first()?;
                Some(Member {
                    key: h.item.item_key.as_slice().try_into().ok()?,
                    name: String::from_utf8_lossy(name).into_owned(),
                    since: h.item.ts,
                })
            })
            .collect(),
    )
}

pub fn did(p: &Put) -> String {
    format!("did:craftec:{}", p.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &[u8] = b"\0asm\x01\0\0\0";

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// SLIP-0010's own test vector 1 for ed25519 (seed 000102…0f): an oracle that is not this code.
    #[test]
    fn slip10_matches_the_published_ed25519_vectors() {
        let seed: Vec<u8> = (0u8..16).collect();
        assert_eq!(hex(&slip10(&seed, &[])), "2b4be7f19ee27bbf30c667b642d5f4aa69fd169872f8fc3059c08ebae2eb19e7");
        assert_eq!(hex(&slip10(&seed, &[0])), "68e0fe46dfb67e368c75379acec591dad19df3cde26e63b93a8e704f1dade7a3");
        assert_eq!(hex(&slip10(&seed, &[0, 1])), "b1d0bad404bf35da785a64ca1ac54b2617211d2777696fbffaf208f746ae84f2");
    }

    /// BIP39's own vector: 16 zero bytes are "abandon ×11 about".
    #[test]
    fn words_round_trip_and_bad_words_are_refused() {
        let w = words(&[0; 16]).unwrap();
        assert_eq!(w, "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about");
        assert_eq!(entropy(&w.to_uppercase()), Some(vec![0; 16]));
        assert_eq!(entropy(&words(&[7; 32]).unwrap()), Some(vec![7; 32]));
        // A wrong checksum word, and a count that is not 12 or 24.
        assert_eq!(entropy(&w.replace("about", "abandon")), None);
        assert_eq!(entropy("abandon abandon"), None);
        assert_eq!(words(&[0; 20]), None);
    }

    #[test]
    fn the_words_alone_name_the_did_and_the_seat_is_the_same_put_every_time() {
        let a = seat(CODE, &[3; 16]).unwrap();
        let b = seat(CODE, &[3; 16]).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.state.as_ref(), b.state.as_ref(), "the same record bytes: a second PUT is the same PUT");
        assert_ne!(seat(CODE, &[4; 16]).unwrap().id, a.id);
        let owner = SigningKey::from_bytes(&owner_seed(&[3; 16]).unwrap());
        let params = wire::register_params(&owner.verifying_key().to_bytes(), SEAT_LABEL);
        let (_, st) = craftec_register_contract::read(&params, a.state.as_ref()).expect("a valid Register state");
        assert_eq!(st.record.unwrap().signed.seq, SEAT_SEQ);
        assert!(did(&a).starts_with("did:craftec:"));
        // A reader holding only the DID learns the owner key from the seat, checked.
        let owner = owner.verifying_key().to_bytes();
        assert_eq!(owner_of_seat(CODE, &a.id_bytes, a.state.as_ref()), Some(owner));
        // Another account's seat offered for this DID is refused.
        let other = seat(CODE, &[4; 16]).unwrap();
        assert_eq!(owner_of_seat(CODE, &a.id_bytes, other.state.as_ref()), None);
    }

    #[test]
    fn members_are_owner_signed_items_and_two_admissions_are_a_union() {
        const SET: &[u8] = b"\0asm\x01\0\0\0set";
        let words = [3u8; 16];
        let owner = SigningKey::from_bytes(&owner_seed(&words).unwrap()).verifying_key().to_bytes();
        let a = admit(SET, &words, &[7; 32], 1_000, "laptop").unwrap();
        let b = admit(SET, &words, &[8; 32], 2_000, "phone").unwrap();
        // One Set, derived from the owner key alone.
        assert_eq!(a.id, b.id);
        assert_eq!(members_address(SET, &owner).unwrap().id, a.id);
        // What the contract does with two PUTs: the union of the two states.
        let p = SetParams::parse(&members_params(&owner).unwrap()).unwrap();
        let sa = SetState::parse(a.state.as_ref(), &p).unwrap();
        let sb = SetState::parse(b.state.as_ref(), &p).unwrap();
        let both = craftec_set_contract::merge::join(&sa, &sb, &p).encode();
        let mut got = members(&owner, &both).unwrap();
        got.sort_by_key(|m| m.since);
        assert_eq!(got.iter().map(|m| (m.key, m.name.as_str())).collect::<Vec<_>>(), vec![([7; 32], "laptop"), ([8; 32], "phone")]);
        // Another account's owner key reads nothing from this Set.
        let stranger = SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes();
        assert_eq!(members(&stranger, &both), None);
    }
}
