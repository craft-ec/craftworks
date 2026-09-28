//! A person's ACCOUNT on the network (ARCHITECTURE §4): the DID, its key event log, and RECOVERY WORDS.
//!
//! - **Words:** BIP39, 12 or 24 (16 or 32 bytes of entropy), the same words a deterministic wallet uses. Keys are
//!   derived from them by SLIP-0010 (ed25519) on paths of Craftworks' own (25458 = "cr"), so words shared with a
//!   bitcoin wallet give keys unrelated to any of that wallet's.
//! - **The DID names the account's KEY EVENT LOG, not a key** (`did:craftec:<base58>`, the id of the log's first
//!   event; the `craftworks-idlog-contract` crate). The keys rotate under it and the DID never changes — the account's
//!   "phone number". Never a contract's address either: that moves with every release of the contract's code
//!   (freenet-agent-skills, "Identity must not be a contract key"); the log's ADDRESS is derived from the DID.
//! - **Epoch keys:** the owner key of words W at index j is [`owner_seed_at`] (j = 0 is the words' first key), its
//!   encryption key [`enc_seed_at`]. Each log event reveals the epoch's owner key and commits to the next one, so the
//!   next rotation needs the words (the pre-committed key), never a key a node held.
//! - **Changing the words** is two rotations: the old words' committed key hands to the new words' first key, then
//!   that key takes over. Afterwards the old words have no power. New words find their DID through a small Register
//!   under their first key (`whoami`); the original words re-derive it (the inception is theirs, deterministically).
//! - **The data key** (the account's tables) is derived once, from the ORIGINAL words, and never rotates with them: every
//!   event carries it sealed in the VAULT, openable with that epoch's words. So new words reach the same tables.
//! - **Members:** a Set under the owner key (owner-only), label `members`. Each member (a node's key) is its own item,
//!   keyed by the member's public key and signed by the owner key: two nodes joining at once are two items, never a
//!   conflict. Leaving is a tombstone. DID → member Set, derived: nothing to look up.
//!
//! The page makes the entropy (this crate has no randomness) and the identity delegate keeps it with the member, so
//! the Account page can show the words when the person sets up recovery.

use bip39::Mnemonic;
use ed25519_dalek::{Signer, SigningKey};
use freenet_stdlib::prelude::{ContractContainer, WrappedState};
use craftec_set_contract::wire::{Admission, Item, Params as SetParams, SetState};
use hmac::{Hmac, Mac};
use sha2::Sha512;

pub const MEMBERS_LABEL: &[u8] = b"members";
/// A member item's payload: `MB01 ‖ class ‖ name`. Class 0 = one of the person's own devices.
const MEMBER_MAGIC: &[u8] = b"MB01";
const MAX_NAME: usize = 48;
/// SLIP-0010 ed25519, every step hardened: m/44'/25458'/0' (25458 = "cr"; no coin's, so no wallet key collides).
pub const PATH: [u32; 3] = [44, 25458, 0];
/// The account's DATA key: m/44'/25458'/1'. Every node of the account holds it (the identity delegate keeps it), so
/// the account's tables are the account's, not one node's. Recoverable from the words like the owner key, and,
/// unlike the owner, not the account itself: it could be rotated.
pub const DATA_PATH: [u32; 3] = [44, 25458, 1];
/// The account's ENCRYPTION keys (X25519): m/44'/25458'/2'/j'.
pub const ENC_PATH: [u32; 3] = [44, 25458, 2];
/// The Register a set of words names its DID in, once they were rotated in (their DID is not their inception's).
pub const WHOAMI_LABEL: &[u8] = b"did";
use craftworks_idlog_contract::{commit, Event, Log};

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

/// The owner key's seed from the words' entropy (BIP39 seed with no passphrase, then [`PATH`]): the words' FIRST key.
pub fn owner_seed(entropy: &[u8]) -> Option<[u8; 32]> {
    owner_seed_at(entropy, 0)
}

fn bip39_seed(entropy: &[u8]) -> Option<[u8; 64]> {
    Some(Mnemonic::from_entropy(entropy).ok()?.to_seed_normalized(""))
}

/// The words' owner key at index `j`: [`PATH`] for 0 (so an account's first key is where it always was), then
/// m/44'/25458'/0'/j'.
pub fn owner_seed_at(entropy: &[u8], j: u32) -> Option<[u8; 32]> {
    Some(owner_at(&bip39_seed(entropy)?, j))
}

/// [`owner_seed_at`] from the BIP39 seed (derived once: it is 2048 rounds of PBKDF2).
fn owner_at(seed: &[u8; 64], j: u32) -> [u8; 32] {
    if j == 0 {
        slip10(seed, &PATH)
    } else {
        slip10(seed, &[PATH[0], PATH[1], PATH[2], j])
    }
}

/// The words' encryption (X25519) secret at index `j`.
pub fn enc_seed_at(entropy: &[u8], j: u32) -> Option<[u8; 32]> {
    Some(slip10(&bip39_seed(entropy)?, &[ENC_PATH[0], ENC_PATH[1], ENC_PATH[2], j]))
}

fn public_of(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

fn enc_public_of(seed: &[u8; 32]) -> [u8; 32] {
    x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(*seed)).to_bytes()
}

/// THE VAULT of an epoch: the data key's seed, sealed so only the words of that epoch open it. XChaCha20-Poly1305
/// under a key derived from the epoch's encryption secret, with the nonce derived from key and content (the same seed
/// sealed twice is the same bytes, which keeps an inception — and so its DID — a function of the words alone).
fn vault_key(enc_seed: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("craftworks 2026-09-28 idlog vault key", enc_seed)
}

pub fn vault_seal(enc_seed: &[u8; 32], data_seed: &[u8; 32]) -> Vec<u8> {
    use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
    let key = vault_key(enc_seed);
    let n = blake3::keyed_hash(&key, data_seed);
    let nonce = XNonce::from_slice(&n.as_bytes()[..24]);
    let ct = XChaCha20Poly1305::new((&key).into()).encrypt(nonce, data_seed.as_slice()).expect("sealing 32 bytes cannot fail");
    [&n.as_bytes()[..24], &ct[..]].concat()
}

pub fn vault_open(enc_seed: &[u8; 32], vault: &[u8]) -> Option<[u8; 32]> {
    use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
    let (n, ct) = vault.split_at_checked(24)?;
    let pt = XChaCha20Poly1305::new((&vault_key(enc_seed)).into()).decrypt(XNonce::from_slice(n), ct).ok()?;
    pt.try_into().ok()
}

/// The first event of the account these words make: owner key 0, the commitment to key 1, the data key (from these
/// words), encryption key 0, and the vault. A function of the words alone, so the words re-derive their DID.
pub fn inception(entropy: &[u8]) -> Option<Event> {
    let data = data_seed(entropy)?;
    let enc = enc_seed_at(entropy, 0)?;
    Event::signed(
        0,
        [0; 32],
        &owner_seed_at(entropy, 0)?,
        commit(&public_of(&owner_seed_at(entropy, 1)?)),
        public_of(&data),
        enc_public_of(&enc),
        vault_seal(&enc, &data),
    )
}

/// Where words W stand in a log: `Some(j)` if the log's current key is W's key `j`; `None` if W's keys are not
/// current (replaced by newer words, or another account's words).
pub fn position(entropy: &[u8], log: &Log) -> Option<u32> {
    let head = log.head().key;
    let seed = bip39_seed(entropy)?;
    (0..craftworks_idlog_contract::EVENTS_MAX as u32 + 1).find(|j| public_of(&owner_at(&seed, *j)) == head)
}

/// What words W hold in a verified log: their index, the current owner key's seed, and the data key's seed (from the
/// vault). `None` if W are not the current words.
pub fn open_log(entropy: &[u8], log: &Log) -> Option<(u32, [u8; 32], [u8; 32])> {
    let j = position(entropy, log)?;
    // The vault of the head is sealed to the encryption key of the words that hold it now: index 0 of those words
    // (they took over at their first key) — or, for the original words, that same index 0.
    let data = vault_open(&enc_seed_at(entropy, 0)?, &log.head().vault)?;
    Some((j, owner_seed_at(entropy, j)?, data))
}

/// CHANGE THE WORDS: from `old` (current in `log`, at their index j) to `new`. Two events: old's committed key j+1
/// hands to new's key 0 (committing to it), then new's key 0 takes over, committing to new's key 1. The vault moves
/// to the new words; the data key stays. Returns the longer log.
pub fn change_words(log: &Log, old: &[u8], new: &[u8]) -> Option<Log> {
    let j = position(old, log)?;
    let (_, _, data) = open_log(old, log)?;
    let head = log.head();
    let enc = enc_seed_at(new, 0)?;
    let (enc_pub, vault) = (enc_public_of(&enc), vault_seal(&enc, &data));
    let new0 = owner_seed_at(new, 0)?;
    let a = Event::signed(head.seq + 1, head.id(), &owner_seed_at(old, j + 1)?, commit(&public_of(&new0)), head.data, enc_pub, vault.clone())?;
    let b = Event::signed(a.seq + 1, a.id(), &new0, commit(&public_of(&owner_seed_at(new, 1)?)), head.data, enc_pub, vault)?;
    let mut out = log.clone();
    out.events.extend([a, b]);
    Some(out)
}

/// The key event log's contract for a DID: its PUT (with a state) or its address (without).
pub fn idlog_put(code: &[u8], did: &[u8; 32], log: Option<&Log>) -> Put {
    put(code, did, &log.map(Log::encode).unwrap_or_default())
}

/// The `whoami` Register of words W: under W's first key, it names the DID W were rotated into.
pub fn whoami_params(entropy: &[u8]) -> Option<Vec<u8>> {
    Some(wire::register_params(&public_of(&owner_seed_at(entropy, 0)?), WHOAMI_LABEL))
}

pub fn whoami_put(register_code: &[u8], entropy: &[u8], did: &[u8; 32]) -> Option<Put> {
    let params = whoami_params(entropy)?;
    let state = contract_keys_head(&params, &owner_seed_at(entropy, 0)?, did)?;
    Some(put(register_code, &params, &state))
}

pub fn whoami_address(register_code: &[u8], entropy: &[u8]) -> Option<Put> {
    Some(put(register_code, &whoami_params(entropy)?, &[]))
}

/// The DID a whoami Register's state names, verified by the Register crate against its params.
pub fn whoami_did(entropy: &[u8], state: &[u8]) -> Option<[u8; 32]> {
    let params = whoami_params(entropy)?;
    let (_, reg) = craftec_register_contract::read(&params, state)?;
    reg.record?.value.as_slice().try_into().ok()
}

/// A mode-0 Register state holding `value`, signed by `seed` (seq 1: a whoami is written once).
fn contract_keys_head(params: &[u8], seed: &[u8; 32], value: &[u8]) -> Option<Vec<u8>> {
    use craftec_register_contract::wire::{Params, RegState, Record, Signed};
    let p = Params::parse(params)?;
    let sk = SigningKey::from_bytes(seed);
    let value_hash: [u8; 32] = *blake3::hash(value).as_bytes();
    let sig = sk.sign(&p.signed_message(false, 1, &value_hash)).to_bytes();
    let signed = Signed { terminal: false, seq: 1, value_hash, bitmap: 0, sigs: vec![sig] };
    let record = Record::new(signed, value.to_vec())?;
    Some(RegState { record: Some(record), evidence: None }.encode(&p.authority))
}

fn put(code: &[u8], params: &[u8], state: &[u8]) -> Put {
    let (id, contract, state) = wire::puts::contract(code, params, state);
    let id_bytes = contract.key().id().as_bytes().try_into().expect("a contract id is 32 bytes");
    Put { id, id_bytes, contract, state }
}

/// The account's data key seed from the words' entropy.
pub fn data_seed(entropy: &[u8]) -> Option<[u8; 32]> {
    let m = Mnemonic::from_entropy(entropy).ok()?;
    Some(slip10(&m.to_seed_normalized(""), &DATA_PATH))
}

/// The owner's first public key for these words.
pub fn owner(entropy: &[u8]) -> Option<[u8; 32]> {
    Some(public_of(&owner_seed(entropy)?))
}

/// `did:craftec:<base58 of the DID's 32 bytes>` (the id of the account's first event).
pub fn did(did: &[u8; 32]) -> String {
    format!("did:craftec:{}", bs58::encode(did).into_string())
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
pub fn admit(set_code: &[u8], owner_seed: &[u8; 32], member: &[u8; 32], ts: u64, name: &str) -> Option<Put> {
    let owner = SigningKey::from_bytes(owner_seed);
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


#[cfg(test)]
mod tests {
    use super::*;


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
    fn the_did_is_the_id_of_the_inception_the_words_give() {
        let e = inception(&[3; 16]).unwrap();
        assert_eq!(inception(&[3; 16]).unwrap(), e, "a function of the words alone");
        assert_ne!(inception(&[4; 16]).unwrap().id(), e.id());
        let log = Log { events: vec![e.clone()] };
        assert!(log.verify(&e.id()));
        assert_eq!(e.key, owner(&[3; 16]).unwrap(), "the first key is the words' first owner key");
        let d = did(&e.id());
        assert!(d.starts_with("did:craftec:"));
        assert_eq!(bs58::decode(d.trim_start_matches("did:craftec:")).into_vec().unwrap(), e.id());
    }

    #[test]
    fn new_words_take_over_the_same_did_and_the_same_data_and_the_old_words_are_powerless() {
        let (old, new) = ([3u8; 16], [5u8; 16]);
        let e = inception(&old).unwrap();
        let did = e.id();
        let log = Log { events: vec![e] };
        let (j, _, data) = open_log(&old, &log).unwrap();
        assert_eq!((j, data), (0, data_seed(&old).unwrap()));
        assert!(open_log(&new, &log).is_none(), "control: the new words hold nothing yet");
        let rotated = change_words(&log, &old, &new).unwrap();
        assert!(rotated.verify(&did), "the same DID");
        assert_eq!(rotated.events.len(), 3);
        let (j, owner_now, data_now) = open_log(&new, &rotated).unwrap();
        assert_eq!(j, 0);
        assert_eq!(public_of(&owner_now), owner(&new).unwrap());
        assert_eq!(data_now, data, "the same tables");
        assert!(open_log(&old, &rotated).is_none(), "the old words hold nothing now");
        assert!(change_words(&rotated, &old, &[7; 16]).is_none(), "and cannot rotate again");
        // A second change, from the new words: the chain goes on.
        let again = change_words(&rotated, &new, &[7; 16]).unwrap();
        assert!(again.verify(&did));
        assert_eq!(open_log(&[7; 16], &again).unwrap().2, data);
    }

    #[test]
    fn whoami_names_the_did_and_the_vault_opens_only_for_its_words() {
        const REG: &[u8] = b"\0asm\x01\0\0\0reg";
        let did = [42u8; 32];
        let p = whoami_put(REG, &[5; 16], &did).unwrap();
        assert_eq!(whoami_did(&[5; 16], p.state.as_ref()), Some(did));
        assert_eq!(whoami_did(&[6; 16], p.state.as_ref()), None, "another's words read nothing");
        assert_eq!(whoami_address(REG, &[5; 16]).unwrap().id, p.id);
        let (enc, data) = (enc_seed_at(&[3; 16], 0).unwrap(), [9u8; 32]);
        let v = vault_seal(&enc, &data);
        assert_eq!(vault_open(&enc, &v), Some(data));
        assert_eq!(vault_open(&enc_seed_at(&[4; 16], 0).unwrap(), &v), None);
    }

    #[test]
    fn the_data_key_is_its_own_key_from_the_same_words() {
        let (owner, data) = (owner_seed(&[3; 16]).unwrap(), data_seed(&[3; 16]).unwrap());
        assert_ne!(owner, data);
        assert_eq!(data_seed(&[3; 16]), Some(data), "the same on every node");
        assert_eq!(data, slip10(&Mnemonic::from_entropy(&[3; 16]).unwrap().to_seed_normalized(""), &DATA_PATH));
    }

    #[test]
    fn members_are_owner_signed_items_and_two_admissions_are_a_union() {
        const SET: &[u8] = b"\0asm\x01\0\0\0set";
        let words = [3u8; 16];
        let seed = owner_seed(&words).unwrap();
        let owner = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
        let a = admit(SET, &seed, &[7; 32], 1_000, "laptop").unwrap();
        let b = admit(SET, &seed, &[8; 32], 2_000, "phone").unwrap();
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
