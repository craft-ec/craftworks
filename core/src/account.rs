//! A person's ACCOUNT on the network (ARCHITECTURE §4): the DID and its vault.
//!
//! - **Owner seat:** a Register under the OWNER key, label `seat`. Its contract id IS the DID
//!   (`did:craftec:<id>`); its value names the first member.
//! - **Vault:** a Register under the owner key, label `vault`. Its value is the owner key encrypted under a slow hash of
//!   the person's passphrase. Anyone can copy it and guess offline, so the passphrase must be long.
//!
//! The page makes both at the first "log in with this device": it mints the owner key (the caller passes randomness;
//! this crate has none), signs the two records, and forgets the owner key. The passphrase gets it back.

use argon2::{Algorithm, Argon2, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use craftec_register_contract::wire::{Params, RegState, Record, Signed};
use ed25519_dalek::{Signer, SigningKey};
use freenet_stdlib::prelude::{ContractContainer, WrappedState};

pub const SEAT_LABEL: &[u8] = b"seat";
pub const VAULT_LABEL: &[u8] = b"vault";
const SEAT_MAGIC: &[u8; 4] = b"ST01";
const VAULT_MAGIC: &[u8; 4] = b"VT01";
/// argon2id: 19 MiB, 2 passes, 1 lane (OWASP's floor), written into each vault so it can be raised later.
const M_KIB: u32 = 19 * 1024;
const T: u32 = 2;
const P: u32 = 1;

/// One Register to PUT: its contract, its state, and the id the node names it by.
pub struct Put {
    pub id: String,
    pub id_bytes: [u8; 32],
    pub contract: ContractContainer,
    pub state: WrappedState,
}

/// A one-key Register holding one record at seq 1, signed by `key`.
fn first_record(register_code: &[u8], key: &SigningKey, label: &[u8], value: Vec<u8>) -> Put {
    let params = wire::register_params(&key.verifying_key().to_bytes(), label);
    let p = Params::parse(&params).expect("our own params parse");
    let value_hash = *blake3::hash(&value).as_bytes();
    let sig = key.sign(&p.signed_message(false, 1, &value_hash)).to_bytes();
    let signed = Signed { terminal: false, seq: 1, value_hash, bitmap: 0, sigs: vec![sig] };
    let record = Record::new(signed, value).expect("a value within the cap, hashed above");
    let state = RegState { record: Some(record), evidence: None }.encode(&p.authority);
    let (id, contract, state) = wire::puts::contract(register_code, &params, &state);
    let id_bytes = contract.key().id().as_bytes().try_into().expect("a contract id is 32 bytes");
    Put { id, id_bytes, contract, state }
}

/// The vault key from a passphrase.
fn vault_key(passphrase: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Option<[u8; 32]> {
    let params = argon2::Params::new(m, t, p, Some(32)).ok()?;
    let mut k = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params).hash_password_into(passphrase.as_bytes(), salt, &mut k).ok()?;
    Some(k)
}

/// `VT01 ‖ m u32 ‖ t u32 ‖ p u32 ‖ salt 16 ‖ nonce 12 ‖ ChaCha20-Poly1305(owner seed)`.
pub fn seal(owner_seed: &[u8; 32], passphrase: &str, salt: &[u8; 16], nonce: &[u8; 12]) -> Vec<u8> {
    let k = vault_key(passphrase, salt, M_KIB, T, P).expect("fixed argon2 params are valid");
    let sealed = ChaCha20Poly1305::new(&k.into()).encrypt(Nonce::from_slice(nonce), &owner_seed[..]).expect("encrypts");
    [&VAULT_MAGIC[..], &M_KIB.to_le_bytes(), &T.to_le_bytes(), &P.to_le_bytes(), salt, nonce, &sealed].concat()
}

/// The owner seed from a vault value and the passphrase; `None` for a wrong passphrase or not a vault.
pub fn open(vault: &[u8], passphrase: &str) -> Option<[u8; 32]> {
    let r = vault.strip_prefix(VAULT_MAGIC)?;
    let u = |b: &[u8]| u32::from_le_bytes(b.try_into().unwrap_or_default());
    let (m, r) = r.split_at_checked(4)?;
    let (t, r) = r.split_at_checked(4)?;
    let (p, r) = r.split_at_checked(4)?;
    let (salt, r) = r.split_at_checked(16)?;
    let (nonce, sealed) = r.split_at_checked(12)?;
    // A vault names its own cost; refuse one that would take the page's memory.
    if u(m) > 256 * 1024 {
        return None;
    }
    let k = vault_key(passphrase, salt, u(m), u(t), u(p))?;
    ChaCha20Poly1305::new(&k.into()).decrypt(Nonce::from_slice(nonce), sealed).ok()?.try_into().ok()
}

/// What the first login puts: the owner seat (whose id is the DID) and the vault.
pub struct NewAccount {
    pub seat: Put,
    pub vault: Put,
}

impl NewAccount {
    pub fn did(&self) -> String {
        format!("did:craftec:{}", self.seat.id)
    }
}

/// Make an account: the owner key from `owner_seed`, its first member `member_public`, sealed under `passphrase`.
pub fn create(
    register_code: &[u8],
    owner_seed: &[u8; 32],
    member_public: &[u8; 32],
    passphrase: &str,
    salt: &[u8; 16],
    nonce: &[u8; 12],
) -> NewAccount {
    let owner = SigningKey::from_bytes(owner_seed);
    let seat = first_record(register_code, &owner, SEAT_LABEL, [&SEAT_MAGIC[..], member_public].concat());
    let vault = first_record(register_code, &owner, VAULT_LABEL, seal(owner_seed, passphrase, salt, nonce));
    NewAccount { seat, vault }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &[u8] = b"\0asm\x01\0\0\0";

    #[test]
    fn the_vault_opens_with_its_passphrase_only() {
        let v = seal(&[4; 32], "correct horse battery staple", &[1; 16], &[2; 12]);
        assert_eq!(open(&v, "correct horse battery staple"), Some([4; 32]));
        assert_eq!(open(&v, "correct horse battery stapler"), None);
        assert_eq!(open(b"nonsense", "x"), None);
    }

    #[test]
    fn both_records_verify_under_the_owner_key_and_the_did_is_the_seat() {
        let a = create(CODE, &[4; 32], &[9; 32], "pass phrase words", &[1; 16], &[2; 12]);
        for (put, label) in [(&a.seat, SEAT_LABEL), (&a.vault, VAULT_LABEL)] {
            let params = wire::register_params(&SigningKey::from_bytes(&[4; 32]).verifying_key().to_bytes(), label);
            let (_, st) = craftec_register_contract::read(&params, put.state.as_ref()).expect("a valid Register state");
            assert_eq!(st.record.unwrap().signed.seq, 1);
        }
        assert_eq!(a.did(), format!("did:craftec:{}", a.seat.id));
        assert_ne!(a.seat.id, a.vault.id);
        // The same inputs name the same DID: it is derived, never looked up.
        assert_eq!(create(CODE, &[4; 32], &[9; 32], "other words", &[3; 16], &[5; 12]).seat.id, a.seat.id);
    }
}
