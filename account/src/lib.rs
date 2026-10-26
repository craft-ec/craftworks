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
//! - **Members** (the account's nodes) are the members of its MLS group (the `mls` crate): one list, never a second.
//!
//! The page makes the entropy (this crate has no randomness) and the identity delegate keeps it with the member, so
//! the Account page can show the words when the person sets up recovery.

use craftworks_identity::unhex32;
use bip39::Mnemonic;
use ed25519_dalek::SigningKey;
#[cfg(feature = "puts")]
use ed25519_dalek::Signer;
#[cfg(feature = "puts")]
use freenet_stdlib::prelude::{ContractContainer, WrappedState};
use hmac::{Hmac, Mac};
use sha2::Sha512;

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

/// One Register to PUT: its contract, its state, and the id the node names it by (feature `puts`: a node client).
#[cfg(feature = "puts")]
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

/// The words' encryption PUBLIC key at index `j` (what their key-log events publish, and escrows are sealed to).
pub fn enc_public_at(entropy: &[u8], j: u32) -> Option<[u8; 32]> {
    Some(enc_public_of(&enc_seed_at(entropy, j)?))
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

/// RECOVERY BY PASSPHRASE: the words' entropy sealed under a key stretched from a passphrase (Argon2id, 64 MiB, 3
/// passes), so a new device opens the account with the passphrase instead of typing the words. The sealed copy lives on
/// the network where anyone may fetch it, so the stretch is what stands between a guess and the account: the
/// passphrase must be long. The DID is bound in (associated data): a copy moved to another account does not open.
/// `CWR1 ‖ salt (16) ‖ nonce (24) ‖ sealed`. The caller supplies the randomness (salt, nonce).
const RECOVERY: &[u8; 4] = b"CWR1";
const RECOVERY_M_KIB: u32 = 64 * 1024;
const RECOVERY_T: u32 = 3;

fn recovery_key(passphrase: &str, salt: &[u8; 16]) -> Option<[u8; 32]> {
    let params = argon2::Params::new(RECOVERY_M_KIB, RECOVERY_T, 1, Some(32)).ok()?;
    let mut key = [0u8; 32];
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .ok()?;
    Some(key)
}

pub fn passphrase_seal(passphrase: &str, did: &[u8; 32], entropy: &[u8], salt: [u8; 16], nonce: [u8; 24]) -> Option<Vec<u8>> {
    use chacha20poly1305::{aead::{Aead, Payload}, KeyInit, XChaCha20Poly1305, XNonce};
    let key = recovery_key(passphrase, &salt)?;
    let ct = XChaCha20Poly1305::new((&key).into()).encrypt(XNonce::from_slice(&nonce), Payload { msg: entropy, aad: did }).ok()?;
    Some([&RECOVERY[..], &salt, &nonce, &ct].concat())
}

pub fn passphrase_open(passphrase: &str, did: &[u8; 32], sealed: &[u8]) -> Option<Vec<u8>> {
    use chacha20poly1305::{aead::{Aead, Payload}, KeyInit, XChaCha20Poly1305, XNonce};
    let rest = sealed.strip_prefix(RECOVERY)?;
    let (salt, rest) = rest.split_first_chunk::<16>()?;
    let (nonce, ct) = rest.split_first_chunk::<24>()?;
    let key = recovery_key(passphrase, salt)?;
    XChaCha20Poly1305::new((&key).into()).decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: did }).ok()
}

/// ESCROW: a secret (an MLS epoch's) sealed to the account's encryption PUBLIC key (in its key log), so the recovery
/// words alone open it — and a node, which holds no words, never can (a removed node cannot open what is escrowed after
/// its removal). X25519 with a one-time key (`eph_seed`, fresh randomness from the caller), then XChaCha20-Poly1305:
/// `one-time public ‖ nonce ‖ sealed`.
pub fn escrow_seal(enc_public: &[u8; 32], secret: &[u8; 32], eph_seed: [u8; 32]) -> Vec<u8> {
    craftworks_identity::seal_to(enc_public, secret, eph_seed)
}

/// Open an escrow with the words (their encryption key at index 0: the key the words' own events publish).
pub fn escrow_open(entropy: &[u8], blob: &[u8]) -> Option<[u8; 32]> {
    craftworks_identity::open_with(&enc_seed_at(entropy, 0)?, blob)?.try_into().ok()
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

/// ROTATE with the same words (removing a device, owner 10-09): the words' committed key j+1 takes over, committing to
/// j+2; the data and encryption keys and the vault stay (the words did not change). Every credential on key j stops
/// counting ([`members`]): the devices that stay are credentialed again ([`reissue`]).
pub fn rotate(log: &Log, words: &[u8]) -> Option<Log> {
    let j = position(words, log)?;
    let head = log.head();
    let e = Event::signed(head.seq + 1, head.id(), &owner_seed_at(words, j + 1)?, commit(&public_of(&owner_seed_at(words, j + 2)?)), head.data, head.enc, head.vault.clone())?;
    let mut out = log.clone();
    out.events.push(e);
    Some(out)
}

/// CREDENTIALS AGAIN, on the log's current key (`words` hold it): each device that stays keeps its node and MLS keys,
/// signed now by the current owner key — after a rotation, they alone count.
pub fn reissue(did: &[u8; 32], log: &Log, words: &[u8], creds: &[Vec<u8>]) -> Option<Vec<Vec<u8>>> {
    let j = position(words, log)?;
    let owner = owner_seed_at(words, j)?;
    creds.iter().map(|c| read_credential(c).filter(|(d, ..)| d == did).map(|(_, _, node, mls)| credential(did, &node, &mls, &owner))).collect()
}

/// The key event log's contract for a DID: its PUT (with a state) or its address (without).
#[cfg(feature = "puts")]
pub fn idlog_put(code: &[u8], did: &[u8; 32], log: Option<&Log>) -> Put {
    put(code, did, &log.map(Log::encode).unwrap_or_default())
}

/// The account's DEVICES contract (`craftworks_devices_contract`): its PUT with these credentials (the node keeps the
/// union with what it holds), or its address with none. Its params name the key log's code by its hash.
#[cfg(feature = "puts")]
pub fn devices_put(devices_code: &[u8], idlog_code: &[u8], did: &[u8; 32], creds: &[Vec<u8>]) -> Put {
    let params = craftworks_devices_contract::params(did, blake3::hash(idlog_code).as_bytes());
    put(devices_code, &params, &craftworks_devices_contract::encode(creds))
}

/// The `whoami` Register of words W: under W's first key, it names the DID W were rotated into.
pub fn whoami_params(entropy: &[u8]) -> Option<Vec<u8>> {
    Some(contract_keys::register_params(&public_of(&owner_seed_at(entropy, 0)?), WHOAMI_LABEL))
}

#[cfg(feature = "puts")]
pub fn whoami_put(register_code: &[u8], entropy: &[u8], did: &[u8; 32]) -> Option<Put> {
    let params = whoami_params(entropy)?;
    let state = contract_keys_head(&params, &owner_seed_at(entropy, 0)?, did)?;
    Some(put(register_code, &params, &state))
}

#[cfg(feature = "puts")]
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
#[cfg(feature = "puts")]
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

#[cfg(feature = "puts")]
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

/// The DID a text names (`did:craftec:<base58>`, or the base58 alone): its 32 bytes.
pub fn did_bytes(text: &str) -> Option<[u8; 32]> {
    bs58::decode(text.trim().trim_start_matches("did:craftec:")).into_vec().ok()?.try_into().ok()
}


// A member's CREDENTIAL (its format and check) is the identity's: one source, for the account's nodes and for a DID's
// member in a space.
pub use craftworks_identity::{credential, read_credential};

/// The node a credential makes a member of the account `did` whose key log is `log`: signed by the log's CURRENT owner
/// key (owner 10-09: a rotation — changing the words, removing a device — leaves out every device not credentialed
/// again; the devices that stay are, by [`reissue`], so nobody is orphaned — and a removed or stolen one is cut off).
pub fn member_node(did: &[u8; 32], log: &Log, cred: &[u8]) -> Option<[u8; 32]> {
    let (d, owner, node, _) = read_credential(cred)?;
    (d == *did && log.head().key == owner).then_some(node)
}

/// A DID's MEMBER in a space (its credential signed by the account's DATA key, as its key log holds it now): whether
/// `cred` is one — what a request's key package must carry to be let in under that DID (`rules::admission`).
pub fn space_member(did: &[u8; 32], log: &Log, cred: &[u8]) -> bool {
    read_credential(cred).is_some_and(|(d, signer, _, _)| d == *did && log.head().data == signer)
}

/// WHO BELONGS (ARCHITECTURE: membership is self-certifying and gossiped): the account's nodes from every credential
/// and removal gathered from the nodes' feeds. A node counts if a credential names it (checked against the key log). A
/// removal `(by, node, epoch)` — found in `by`'s own feed — counts if `by` counts and was not itself removed first:
/// removals are taken in epoch order, and within one epoch by the remover's key, so two nodes removing each other at
/// once end the same way everywhere. A removal is never taken back.
pub fn members(did: &[u8; 32], log: &Log, creds: &[Vec<u8>], removals: &[([u8; 32], [u8; 32], u64)]) -> Vec<[u8; 32]> {
    let valid: std::collections::BTreeSet<[u8; 32]> = creds.iter().filter_map(|c| member_node(did, log, c)).collect();
    let mut order: Vec<&([u8; 32], [u8; 32], u64)> = removals.iter().filter(|(by, _, _)| valid.contains(by)).collect();
    order.sort_by_key(|(by, node, epoch)| (*epoch, *by, *node));
    let mut removed = std::collections::BTreeSet::new();
    for (by, node, _) in order {
        if !removed.contains(by) {
            removed.insert(*node);
        }
    }
    valid.into_iter().filter(|n| !removed.contains(n)).collect()
}

/// AN ACCOUNT's DEVICES NOW — the one resolution, the page's and the node's: its DEVICES contract's credentials on the
/// key log's current key (`craftworks_devices_contract::current`); where it holds none there (not read, or none on
/// that key yet), its CARD's credentials by the members rule (`members`, no removals: a credential on an older key
/// counts for nothing either way). (10-16: the page returned none where the node fell back to the card.)
pub fn devices(did: &[u8; 32], log: &Log, contract: Option<&[Vec<u8>]>, card: &[Vec<u8>]) -> Vec<[u8; 32]> {
    if let Some(all) = contract {
        let now: Vec<[u8; 32]> = craftworks_devices_contract::current(did, log, all).into_iter().map(|(n, _)| n).collect();
        if !now.is_empty() {
            return now;
        }
    }
    members(did, log, card, &[])
}

/// THE ACCOUNT TREE's MEMBERS (K8.5): the group key each of its devices' credentials names (the field MLS's signing key
/// fills), on the key log's CURRENT key only (`craftworks_devices_contract::current`) — the account's `AccountCheck`
/// `current`. A device whose credentials name several (a words login made a new one) has each: only it holds their seeds.
pub fn device_members(did: &[u8; 32], log: &Log, contract: &[Vec<u8>]) -> Vec<[u8; 32]> {
    let head = log.head().key;
    let mut out: Vec<[u8; 32]> = contract
        .iter()
        .filter_map(|c| craftworks_identity::read_credential(c))
        .filter(|(d, owner, _, _)| d == did && *owner == head)
        .filter_map(|(_, _, _, key)| key.try_into().ok())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// EVERY DEVICE THE ACCOUNT TREE EVER HAD (K8.5): the group key of each credential signed by ANY key its key log has
/// had — whose operations count as authored (a device removed since still made the root and the adds after it; judged
/// by who it was, never only by who is current: fx/fy 10-26, a removal made every earlier operation unauthorized).
pub fn device_members_ever(did: &[u8; 32], log: &Log, contract: &[Vec<u8>]) -> Vec<[u8; 32]> {
    let keys: Vec<[u8; 32]> = log.events.iter().map(|e| e.key).collect();
    let mut out: Vec<[u8; 32]> = contract
        .iter()
        .filter_map(|c| craftworks_identity::read_credential(c))
        .filter(|(d, owner, _, _)| d == did && keys.contains(owner))
        .filter_map(|(_, _, _, key)| key.try_into().ok())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// A member's DEVICES, STEP BY STEP — the one resolution's order and failures, page and node (each only fetches): their
/// key log, then their card, then their devices contract ONLY where the card says they have one (owner 10-25: a read
/// goes only where something lists it — the contract asked "in case" when the card did not read or named no device was
/// a GET of a contract nobody made: a 60 s not-found on the node, every upkeep round, and the delegate's requests,
/// the page's first, queued behind it). `log`: None — not read. `card`: its device credentials and whether it says they have a devices contract;
/// None — not read. `contract`: None — not asked yet; `Some(None)` — asked, not read. A READ that failed is said
/// (`unread`) — never a list of no devices taken for a member with none (10-19: the node took an unread card for a
/// short list before it asked the devices contract, which would have answered).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicesStep {
    /// Read their devices contract, then step again.
    ReadDevices,
    /// Their devices; `unread`: what did not read, if that is why there are none.
    Done { devices: Vec<[u8; 32]>, unread: Option<String> },
}
pub fn devices_step(did: &[u8; 32], log: Option<&Log>, card: Option<(&[Vec<u8>], bool)>, contract: Option<Option<&[Vec<u8>]>>) -> DevicesStep {
    let Some(log) = log else { return DevicesStep::Done { devices: Vec::new(), unread: Some("their key log not read".into()) } };
    let Some((creds, marked)) = card else { return DevicesStep::Done { devices: Vec::new(), unread: Some("their card not read".into()) } };
    if !marked {
        return DevicesStep::Done { devices: members(did, log, creds, &[]), unread: None };
    }
    let Some(contract) = contract else { return DevicesStep::ReadDevices };
    let devices = devices(did, log, contract, creds);
    let unread = (devices.is_empty() && contract.is_none()).then(|| "their devices contract not read".into());
    DevicesStep::Done { devices, unread }
}

/// EVERY DEVICE THE ACCOUNT HAS HAD: each credential its devices contract holds for this DID (a grow-only set — each
/// signed by a key its key log has had, `craftworks_devices_contract::valid`), current or not. What a dropped device
/// wrote before it was dropped is still its account's (`writers`: up to where it stood) — this is how it is known.
pub fn devices_ever(did: &[u8; 32], contract: &[Vec<u8>]) -> Vec<[u8; 32]> {
    let mut out: Vec<[u8; 32]> = contract.iter().filter_map(|c| read_credential(c)).filter(|(d, ..)| d == did).map(|(_, _, node, _)| node).collect();
    out.sort();
    out.dedup();
    out
}

/// One WRITER of a space's rows: whose it is (its DID) and — a DEPARTED one — up to where each of its feeds counts
/// (`caps`: table name → the seq it counts to; None: a current device, uncapped).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Writer {
    pub node: [u8; 32],
    pub did: String,
    pub caps: Option<std::collections::BTreeMap<String, u64>>,
}

/// WHO WROTE A SPACE's ROWS — the one map, the page's and the node's (owner 10-16: "any question about access should
/// be centralized"): every CURRENT device of every member → its DID, uncapped; and every DEPARTED writer → its DID, its
/// feeds counted up to their heads. DEPARTED: the space's `departed` rows (`node hex` → `{ did, heads: { table: seq } }`),
/// written when a member was put out OR a device was dropped from its account — what it wrote stays its author's, up to
/// there, never after (a stolen device writes nothing that counts). A node that is a current device is never capped; a
/// departed row naming no DID or no heads counts for nothing. (Before: each reader mapped current devices only, so a
/// dropped device's rows had no author — the owner's space read as having no public acts, 10-16.)
pub fn writers(members: &[(String, Vec<[u8; 32]>)], departed: &[(String, String)]) -> Vec<Writer> {
    let mut out: std::collections::BTreeMap<[u8; 32], Writer> = std::collections::BTreeMap::new();
    for (did, nodes) in members {
        for n in nodes {
            out.entry(*n).or_insert(Writer { node: *n, did: did.clone(), caps: None });
        }
    }
    for (node, value) in departed {
        let Some(node) = unhex32(node) else { continue };
        if out.contains_key(&node) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(value) else { continue };
        let Some(did) = v["did"].as_str().filter(|d| d.starts_with("did:craftec:")) else { continue };
        let caps: std::collections::BTreeMap<String, u64> = v["heads"].as_object().map(|o| o.iter().filter_map(|(t, s)| Some((t.clone(), s.as_u64()?))).collect()).unwrap_or_default();
        if caps.is_empty() {
            continue;
        }
        out.insert(node, Writer { node, did: did.to_string(), caps: Some(caps) });
    }
    out.into_values().collect()
}

/// AN ACCOUNT's BEEKEM MEMBER from its card (page and node): the card's `beekem` value (`{ cred, leaf }`, hex) — its
/// credential names this DID and is signed by its DATA key (its key log's head). `(member id, leaf key)`, or why not.
pub fn beekem_member(did: &[u8; 32], data_key: &[u8; 32], value: &[u8]) -> Result<([u8; 32], [u8; 32]), String> {
    let j: serde_json::Value = serde_json::from_slice(value).map_err(|_| "its card's member does not read".to_string())?;
    let b = |s: &str| -> Option<Vec<u8>> { (0..s.len() / 2).map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect() };
    let (Some(cred), Some(leaf)) = (j["cred"].as_str().and_then(b), j["leaf"].as_str().and_then(b)) else { return Err("its card's member does not read".into()) };
    let (d, owner, _, member) = read_credential(&cred).ok_or("its member credential does not read")?;
    if d != *did {
        return Err(format!("its credential names {}", did_short(&d)));
    }
    if owner != *data_key {
        return Err("its credential is not signed by its data key".into());
    }
    Ok((member.try_into().map_err(|_| "its member id does not read")?, leaf.try_into().map_err(|_| "its leaf key does not read")?))
}
fn did_short(d: &[u8; 32]) -> String {
    did(d).get(12..20).unwrap_or_default().to_string()
}

/// WHOSE DEVICES a space's tables are read from (page and node): its group's members, and its OWNER — the owner's
/// acts always count, in the group or not (10-19: the node added the owner, the page did not).
pub fn reading_dids(group: &[String], owner: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = group.to_vec();
    out.extend(owner.map(str::to_string));
    out.sort();
    out.dedup();
    out
}

/// THE WRITERS A SPACE's TABLE IS READ FROM (page and node): the members' devices the space's writers bag LISTS and, a
/// PUBLIC table, every writer the space's OPEN list names for it (needing no key: what a member not keyed yet reads
/// first). Never `me`. An entry of the bag this reader cannot open names no one here — skipped, never "read every
/// device" (owner 10-25: a read goes only where something lists it; every member's device read for one blind entry
/// asked for feeds nobody made, each a 60 s not-found).
pub fn table_writers(devices: &[[u8; 32]], listed: &std::collections::BTreeSet<[u8; 32]>, named: &[[u8; 32]], public: bool, me: &[u8; 32]) -> Vec<[u8; 32]> {
    let mut out: Vec<[u8; 32]> = devices.iter().filter(|d| listed.contains(*d)).copied().collect();
    if public {
        out.extend(named.iter().copied());
    }
    out.retain(|d| d != me);
    out.sort();
    out.dedup();
    out
}


#[cfg(test)]
mod tests {
    /// THE CARD's MEMBER (page and node): taken when its credential names the DID and is signed by its data key; why
    /// not otherwise.
    #[test]
    fn beekem_member_checks_the_did_and_the_data_key() {
        let (did, other) = ([7u8; 32], [8u8; 32]);
        let data = [5u8; 32];
        let dk = ed25519_dalek::SigningKey::from_bytes(&data).verifying_key().to_bytes();
        let cred = credential(&did, &[1u8; 32], &[2u8; 32], &data);
        let v = |c: &[u8]| serde_json::json!({ "cred": c.iter().map(|b| format!("{b:02x}")).collect::<String>(), "leaf": "33".repeat(32) }).to_string();
        assert!(beekem_member(&did, &dk, v(&cred).as_bytes()).is_ok());
        assert!(beekem_member(&other, &dk, v(&cred).as_bytes()).unwrap_err().contains("names"));
        assert!(beekem_member(&did, &[9; 32], v(&cred).as_bytes()).unwrap_err().contains("data key"));
        assert!(beekem_member(&did, &dk, b"{}").is_err());
    }

    /// THE WRITERS A TABLE IS READ FROM: listed devices only, a blind bag too; a public table also the
    /// open list's writers; never this device. And the owner always among whose devices are read.
    #[test]
    fn table_writers_and_reading_dids_are_the_one_rule() {
        let (a, b, c, me) = ([1u8; 32], [2u8; 32], [3u8; 32], [9u8; 32]);
        let listed: std::collections::BTreeSet<[u8; 32]> = [a, me].into_iter().collect();
        assert_eq!(table_writers(&[a, b, me], &listed, &[c], false, &me), vec![a], "listed only, not me, not the open list");
        assert_eq!(table_writers(&[a, b], &listed, &[c, me], true, &me), vec![a, c], "a public table: the open list too");
        assert_eq!(reading_dids(&["did:b".into(), "did:a".into()], Some("did:o")), vec!["did:a", "did:b", "did:o"]);
        assert_eq!(reading_dids(&["did:o".into()], Some("did:o")), vec!["did:o"], "the owner once");
    }

    /// K8.5 THE ACCOUNT TREE's MEMBERS: the group keys of the credentials on the CURRENT key — after a device is removed
    /// (the key log rotated, the kept device credentialed again), the removed one's is gone. Control: before, both.
    #[test]
    fn device_members_are_the_group_keys_on_the_current_key() {
        let words = [0x51u8; 16];
        let log0 = Log { events: vec![inception(&words).unwrap()] };
        let did = log0.events[0].id();
        let (ka, kb) = ([0xA5u8; 32], [0xB5u8; 32]);
        let owner = owner_seed(&words).unwrap();
        let creds = vec![craftworks_identity::credential(&did, &[0x0A; 32], &ka, &owner), craftworks_identity::credential(&did, &[0x0B; 32], &kb, &owner)];
        assert_eq!(device_members(&did, &log0, &creds), vec![ka, kb], "both, on the first key");
        let rotated = rotate(&log0, &words).unwrap();
        let again = reissue(&did, &rotated, &words, &creds[..1]).unwrap();
        let all: Vec<Vec<u8>> = creds.iter().chain(&again).cloned().collect();
        assert_eq!(device_members(&did, &rotated, &all), vec![ka], "B removed: its key is not current");
        assert_eq!(device_members_ever(&did, &rotated, &all), vec![ka, kb], "…but it was: its operations stay authored");
    }

    /// THE ONE DEVICES STEP (page and node): the devices contract asked ONLY where the card says it exists — an unread
    /// card, or one naming no device, asks nothing more (owner 10-25: read only what is listed).
    #[test]
    fn devices_step_reads_the_contract_only_where_the_card_lists_it() {
        let words = [3u8; 16];
        let e = inception(&words).unwrap();
        let did = e.id();
        let log = Log { events: vec![e] };
        let node = [9u8; 32];
        let cred = credential(&did, &node, &[1u8; 32], &owner_seed(&words).unwrap());
        // A card read with a device: done, no contract asked.
        assert_eq!(devices_step(&did, Some(&log), Some((&[cred.clone()], false)), None), DevicesStep::Done { devices: vec![node], unread: None });
        // The card NOT read: nothing asked, said.
        assert_eq!(devices_step(&did, Some(&log), None, None), DevicesStep::Done { devices: vec![], unread: Some("their card not read".into()) });
        // A card read with NO device and no mark: none, nothing asked.
        assert_eq!(devices_step(&did, Some(&log), Some((&[], false)), None), DevicesStep::Done { devices: vec![], unread: None });
        // A card that MARKS a devices contract: asked…
        assert_eq!(devices_step(&did, Some(&log), Some((&[], true)), None), DevicesStep::ReadDevices);
        // …it answers: its devices.
        let contract = [cred.clone()];
        assert!(matches!(devices_step(&did, Some(&log), Some((&[], true)), Some(Some(&contract))), DevicesStep::Done { unread: None, .. }));
        // …or does not read: said.
        assert_eq!(devices_step(&did, Some(&log), Some((&[], true)), Some(None)), DevicesStep::Done { devices: vec![], unread: Some("their devices contract not read".into()) });
        // The key log not read: said.
        assert!(matches!(devices_step(&did, None, Some((&[cred], false)), None), DevicesStep::Done { unread: Some(_), .. }));
    }

    use super::*;

    #[test]
    fn a_departed_writer_stays_its_authors_up_to_its_heads_and_a_current_device_is_never_capped() {
        let (a, b, gone, junk) = ([1u8; 32], [2u8; 32], [3u8; 32], [4u8; 32]);
        let hx = |n: &[u8; 32]| n.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let members = vec![("did:craftec:A".to_string(), vec![a]), ("did:craftec:B".to_string(), vec![b])];
        let departed = vec![
            // A device dropped from A's account: its rows A's, up to seq 7 of `acts`.
            (hx(&gone), r#"{"did":"did:craftec:A","heads":{"x1-acts":7}}"#.to_string()),
            // A row for a CURRENT device: it stays uncapped.
            (hx(&b), r#"{"did":"did:craftec:B","heads":{"x1-acts":1}}"#.to_string()),
            // No heads / no DID: nothing.
            (hx(&junk), r#"{"did":"did:craftec:C","heads":{}}"#.to_string()),
            ("zz".to_string(), r#"{"did":"did:craftec:D","heads":{"t":1}}"#.to_string()),
        ];
        let w = writers(&members, &departed);
        let of = |n: [u8; 32]| w.iter().find(|x| x.node == n).cloned();
        assert_eq!(of(a).unwrap().caps, None);
        assert_eq!(of(b).unwrap().caps, None, "a current device is never capped");
        let g = of(gone).expect("a dropped device's rows keep their author");
        assert_eq!(g.did, "did:craftec:A");
        assert_eq!(g.caps.unwrap().get("x1-acts"), Some(&7));
        assert!(of(junk).is_none(), "a departed row with no heads counts for nothing");
        assert_eq!(w.len(), 3);
    }



    use craftworks_identity::hex;

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
    fn an_escrow_opens_with_the_words_and_nothing_else() {
        let (words, other) = ([3u8; 16], [4u8; 16]);
        let e = inception(&words).unwrap();
        let secret = [77u8; 32];
        let blob = escrow_seal(&e.enc, &secret, [9; 32]);
        assert_eq!(escrow_open(&words, &blob), Some(secret));
        assert_eq!(escrow_open(&other, &blob), None, "control: other words open nothing");
        assert!(!blob.windows(32).any(|w| w == secret), "the secret is not in the blob");
        // After the words changed, the head names the new words' key: escrows sealed to it open with them.
        let rotated = change_words(&Log { events: vec![e] }, &words, &other).unwrap();
        let blob2 = escrow_seal(&rotated.head().enc, &secret, [8; 32]);
        assert_eq!(escrow_open(&other, &blob2), Some(secret));
        assert_eq!(escrow_open(&words, &blob2), None);
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

    /// MEMBERSHIP: credentials checked against the key log (control: another account's words sign nothing here), and
    /// removals applied in epoch order by removers still in — two nodes removing each other end the same way.
    /// RECOVERY: the passphrase opens the entropy for its DID only; a wrong passphrase, another DID, or a changed
    /// byte opens nothing.
    #[test]
    fn a_passphrase_opens_the_words_for_its_account_only() {
        let (did, entropy) = ([0xD1; 32], [7u8; 16]);
        let sealed = passphrase_seal("correct horse battery staple", &did, &entropy, [1; 16], [2; 24]).unwrap();
        assert_eq!(passphrase_open("correct horse battery staple", &did, &sealed).as_deref(), Some(&entropy[..]));
        assert_eq!(passphrase_open("correct horse battery stapler", &did, &sealed), None, "a wrong passphrase");
        assert_eq!(passphrase_open("correct horse battery staple", &[0xD2; 32], &sealed), None, "another account");
        let mut bent = sealed.clone();
        *bent.last_mut().unwrap() ^= 1;
        assert_eq!(passphrase_open("correct horse battery staple", &did, &bent), None, "a changed byte");
        assert_ne!(&sealed[44..], &entropy[..], "sealed, not plain");
    }

    #[test]
    fn members_are_the_credentials_minus_the_removals_that_count() {
        let words = [3u8; 16];
        let e = inception(&words).unwrap();
        let did = e.id();
        let log = Log { events: vec![e] };
        let owner = owner_seed(&words).unwrap();
        let (a, b, c) = ([0xA; 32], [0xB; 32], [0xC; 32]);
        let cred = |n: &[u8; 32]| credential(&did, n, &[1; 32], &owner);
        let creds = vec![cred(&a), cred(&b), cred(&c)];
        assert_eq!(members(&did, &log, &creds, &[]), vec![a, b, c]);
        // Another account's owner signs a credential for this DID: not a member.
        let stranger = credential(&did, &[0xD; 32], &[1; 32], &owner_seed(&[9; 16]).unwrap());
        assert_eq!(members(&did, &log, &[stranger], &[]), Vec::<[u8; 32]>::new());
        // A removes C: C is out.
        assert_eq!(members(&did, &log, &creds, &[(a, c, 2)]), vec![a, b]);
        // A removal made by a node that is no member counts for nothing.
        assert_eq!(members(&did, &log, &creds, &[([0xE; 32], a, 1)]), vec![a, b, c]);
        // C removed A at epoch 3, after A removed C at 2: C was out already, so A stays.
        assert_eq!(members(&did, &log, &creds, &[(c, a, 3), (a, c, 2)]), vec![a, b]);
        // A and B remove each other at the same epoch: the lower key's removal is taken first — the same everywhere.
        let x = members(&did, &log, &creds, &[(a, b, 4), (b, a, 4)]);
        assert_eq!(x, members(&did, &log, &creds, &[(b, a, 4), (a, b, 4)]));
        assert_eq!(x, vec![a, c]);
    }

    #[test]
    fn removing_a_device_rotates_the_key_and_only_reissued_devices_count() {
        let words = [3u8; 16];
        let log = Log { events: vec![inception(&words).unwrap()] };
        let did = log.events[0].id();
        let (a, b) = ([0xA; 32], [0xB; 32]);
        let creds = vec![credential(&did, &a, &[1; 32], &owner_seed(&words).unwrap()), credential(&did, &b, &[2; 32], &owner_seed(&words).unwrap())];
        assert_eq!(members(&did, &log, &creds, &[]), vec![a, b]);
        // B REMOVED: the key rotates (same words), A credentialed again — B is not.
        let rotated = rotate(&log, &words).unwrap();
        assert!(rotated.verify(&did), "a rotation the log takes");
        assert_eq!(position(&words, &rotated), Some(1));
        let again = reissue(&did, &rotated, &words, &creds[..1]).unwrap();
        let all: Vec<Vec<u8>> = creds.iter().chain(&again).cloned().collect();
        assert_eq!(members(&did, &rotated, &all, &[]), vec![a], "only the re-issued device");
        assert_eq!(read_credential(&again[0]).unwrap().3, vec![1; 32], "its MLS key kept");
        // MEASURED before (10-09): an old-key credential counted after the words changed. Now it does not.
        let changed = change_words(&log, &words, &[4u8; 16]).unwrap();
        assert_eq!(members(&did, &changed, &creds, &[]), Vec::<[u8; 32]>::new(), "old-key credentials do not count");
        let renewed = reissue(&did, &changed, &[4u8; 16], &creds).unwrap();
        assert_eq!(members(&did, &changed, &renewed, &[]), vec![a, b], "re-issued under the new words: both again");
        // Words that are not current re-issue nothing.
        assert!(reissue(&did, &changed, &words, &creds).is_none());
    }

}

