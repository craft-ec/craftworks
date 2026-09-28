//! MLS (RFC 9420, mls-rs): the ONE key manager. First use: YOUR ACCOUNT as a group whose members are your nodes; its
//! epoch secret gives every table key (`table key = derive(epoch secret, table)`), so removing a node moves the epoch
//! and a removed node reads nothing written afterwards.
//!
//! - **Members** carry a credential signed by an OWNER key of the account's key log (`CWMB ‖ did ‖ owner ‖ node key ‖
//!   member signing key ‖ owner signature`): whoever holds the recovery words proves the node is theirs, and the NODE
//!   KEY (the identity delegate's member key) names which node it is — the group's roster IS the account's node list. Any owner key the log ever had
//!   is accepted, so rotating the words does not orphan the nodes already in.
//! - **A node joins by itself** with an EXTERNAL COMMIT from the group's published group info: the words sign its
//!   credential, and no other node needs to be online.
//! - **State** is one blob (`save`/`load`): the group's storage and this member's signing key. The identity delegate
//!   keeps it; the page runs the protocol (mls-rs needs the page's randomness and clock).
//!
//! Where the commits and the group info travel (one agreed order) is the caller's: see `storage`.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use mls_rs::client_builder::{BaseConfig, WithCryptoProvider, WithIdentityProvider};
use mls_rs::group::ReceivedMessage;
use mls_rs::identity::basic::BasicCredential;
use mls_rs::identity::SigningIdentity;
use mls_rs::{CipherSuite, CipherSuiteProvider, Client, CryptoProvider, ExtensionList, Group, MlsMessage};
use mls_rs_core::crypto::{SignaturePublicKey, SignatureSecretKey};
use mls_rs_core::group::{EpochRecord, GroupState, GroupStateStorage};
use mls_rs_core::identity::{CredentialType, IdentityProvider, MemberValidationContext};
use mls_rs_core::time::MlsTime;
use mls_rs_crypto_rustcrypto::RustCryptoProvider;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

pub const SUITE: CipherSuite = CipherSuite::CURVE25519_AES128;
const CRED: &[u8; 4] = b"CWMB";
const TABLE_KEYS: &[u8] = b"craftworks table keys";

/// A member's credential: its account, its node key, its MLS signing key, and an owner key's signature over them.
pub fn credential(did: &[u8; 32], node: &[u8; 32], signing_pub: &[u8], owner_seed: &[u8; 32]) -> Vec<u8> {
    let owner = SigningKey::from_bytes(owner_seed);
    let sig = owner.sign(&cred_message(did, node, signing_pub)).to_bytes();
    [&CRED[..], did, &owner.verifying_key().to_bytes(), node, signing_pub, &sig].concat()
}

fn cred_message(did: &[u8; 32], node: &[u8; 32], signing_pub: &[u8]) -> Vec<u8> {
    [b"craftworks mls member".as_slice(), did, node, signing_pub].concat()
}

/// `(did, owner key, node key, signing key)` of a credential whose owner signature holds.
fn read_credential(b: &[u8]) -> Option<([u8; 32], [u8; 32], [u8; 32], Vec<u8>)> {
    let rest = b.strip_prefix(CRED)?;
    let (did, rest) = rest.split_at_checked(32)?;
    let (owner, rest) = rest.split_at_checked(32)?;
    let (node, rest) = rest.split_at_checked(32)?;
    let (sp, sig) = rest.split_at_checked(rest.len().checked_sub(64)?)?;
    let did: [u8; 32] = did.try_into().ok()?;
    let node: [u8; 32] = node.try_into().ok()?;
    let vk = VerifyingKey::from_bytes(owner.try_into().ok()?).ok()?;
    vk.verify(&cred_message(&did, &node, sp), &ed25519_dalek::Signature::from_slice(sig).ok()?).ok()?;
    Some((did, owner.try_into().ok()?, node, sp.to_vec()))
}

/// THE ACCOUNT'S RULE for who is a member: a credential of THIS account, signed by one of its owner keys, whose
/// signing key is the one the member uses.
#[derive(Clone, Debug)]
pub struct AccountIdentity {
    pub did: [u8; 32],
    pub owners: Vec<[u8; 32]>,
}

#[derive(Debug)]
pub struct NotAMember(&'static str);
impl std::fmt::Display for NotAMember {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for NotAMember {}
impl mls_rs_core::error::IntoAnyError for NotAMember {}

impl AccountIdentity {
    fn check(&self, id: &SigningIdentity) -> Result<Vec<u8>, NotAMember> {
        let basic = id.credential.as_basic().ok_or(NotAMember("not a basic credential"))?;
        let (did, owner, _node, sp) = read_credential(&basic.identifier).ok_or(NotAMember("the owner's signature does not hold"))?;
        if did != self.did {
            return Err(NotAMember("another account's member"));
        }
        if !self.owners.contains(&owner) {
            return Err(NotAMember("signed by a key that was never this account's owner"));
        }
        if sp != id.signature_key.as_bytes() {
            return Err(NotAMember("the credential names another signing key"));
        }
        Ok(sp)
    }
}

impl IdentityProvider for AccountIdentity {
    type Error = NotAMember;

    fn validate_member(&self, id: &SigningIdentity, _t: Option<MlsTime>, _c: MemberValidationContext<'_>) -> Result<(), Self::Error> {
        self.check(id).map(|_| ())
    }

    fn validate_external_sender(&self, _id: &SigningIdentity, _t: Option<MlsTime>, _e: Option<&ExtensionList>) -> Result<(), Self::Error> {
        Err(NotAMember("no external senders"))
    }

    fn identity(&self, id: &SigningIdentity, _e: &ExtensionList) -> Result<Vec<u8>, Self::Error> {
        self.check(id)
    }

    fn valid_successor(&self, a: &SigningIdentity, b: &SigningIdentity, _e: &ExtensionList) -> Result<bool, Self::Error> {
        Ok(self.check(a)? == self.check(b)?)
    }

    fn supported_types(&self) -> Vec<CredentialType> {
        vec![BasicCredential::credential_type()]
    }
}

/// The group's storage, as ONE blob: its state and the epochs it keeps.
#[derive(Clone, Default, Debug)]
pub struct Store(Arc<Mutex<BTreeMap<Vec<u8>, (Vec<u8>, BTreeMap<u64, Vec<u8>>)>>>);

impl GroupStateStorage for Store {
    type Error = NotAMember;

    fn state(&self, group_id: &[u8]) -> Result<Option<Zeroizing<Vec<u8>>>, Self::Error> {
        Ok(self.0.lock().unwrap().get(group_id).map(|(s, _)| Zeroizing::new(s.clone())))
    }

    fn epoch(&self, group_id: &[u8], epoch_id: u64) -> Result<Option<Zeroizing<Vec<u8>>>, Self::Error> {
        Ok(self.0.lock().unwrap().get(group_id).and_then(|(_, e)| e.get(&epoch_id)).map(|d| Zeroizing::new(d.clone())))
    }

    fn write(&mut self, state: GroupState, inserts: Vec<EpochRecord>, updates: Vec<EpochRecord>) -> Result<(), Self::Error> {
        let mut m = self.0.lock().unwrap();
        let entry = m.entry(state.id.clone()).or_default();
        entry.0 = state.data.to_vec();
        for e in inserts.into_iter().chain(updates) {
            entry.1.insert(e.id, e.data.to_vec());
        }
        Ok(())
    }

    fn max_epoch_id(&self, group_id: &[u8]) -> Result<Option<u64>, Self::Error> {
        Ok(self.0.lock().unwrap().get(group_id).and_then(|(_, e)| e.keys().next_back().copied()))
    }
}

impl Store {
    fn encode(&self) -> Vec<u8> {
        let m = self.0.lock().unwrap();
        let mut b = Vec::new();
        let put = |b: &mut Vec<u8>, x: &[u8]| {
            b.extend_from_slice(&(x.len() as u32).to_le_bytes());
            b.extend_from_slice(x);
        };
        b.extend_from_slice(&(m.len() as u32).to_le_bytes());
        for (id, (state, epochs)) in m.iter() {
            put(&mut b, id);
            put(&mut b, state);
            b.extend_from_slice(&(epochs.len() as u32).to_le_bytes());
            for (n, d) in epochs {
                b.extend_from_slice(&n.to_le_bytes());
                put(&mut b, d);
            }
        }
        b
    }

    fn decode(mut b: &[u8]) -> Option<Store> {
        fn u32_(b: &mut &[u8]) -> Option<u32> {
            let (x, r) = b.split_at_checked(4)?;
            *b = r;
            Some(u32::from_le_bytes(x.try_into().ok()?))
        }
        fn bytes(b: &mut &[u8]) -> Option<Vec<u8>> {
            let n = u32_(b)? as usize;
            let (x, r) = b.split_at_checked(n)?;
            *b = r;
            Some(x.to_vec())
        }
        let mut m = BTreeMap::new();
        for _ in 0..u32_(&mut b)? {
            let id = bytes(&mut b)?;
            let state = bytes(&mut b)?;
            let mut epochs = BTreeMap::new();
            for _ in 0..u32_(&mut b)? {
                let (n, r) = b.split_at_checked(8)?;
                b = r;
                epochs.insert(u64::from_le_bytes(n.try_into().ok()?), bytes(&mut b)?);
            }
            m.insert(id, (state, epochs));
        }
        b.is_empty().then(|| Store(Arc::new(Mutex::new(m))))
    }
}

type Config = WithIdentityProvider<AccountIdentity, WithCryptoProvider<RustCryptoProvider, mls_rs::client_builder::WithGroupStateStorage<Store, BaseConfig>>>;

/// This node, as a member of the account's group.
pub struct Account {
    pub group: Group<Config>,
    /// A commit removed THIS member: it keeps what it held (the epochs before) and learns nothing newer.
    pub removed: bool,
    store: Store,
    signer: (Vec<u8>, Vec<u8>),
    ident: AccountIdentity,
    cred: Vec<u8>,
}

fn client(ident: &AccountIdentity, store: &Store, signer: &(Vec<u8>, Vec<u8>), cred: &[u8]) -> Client<Config> {
    let id = SigningIdentity::new(BasicCredential::new(cred.to_vec()).into_credential(), SignaturePublicKey::new(signer.1.clone()));
    Client::builder()
        .group_state_storage(store.clone())
        .crypto_provider(RustCryptoProvider::default())
        .identity_provider(ident.clone())
        .signing_identity(id, SignatureSecretKey::new(signer.0.clone()), SUITE)
        .build()
}

/// A fresh MLS signing key for a member (the page supplies no seed: mls-rs draws from the page's randomness).
pub fn new_signer() -> Result<(Vec<u8>, Vec<u8>), String> {
    let cs = RustCryptoProvider::default().cipher_suite_provider(SUITE).ok_or("no cipher suite")?;
    let (s, p) = cs.signature_key_generate().map_err(|e| format!("{e:?}"))?;
    Ok((s.as_bytes().to_vec(), p.as_bytes().to_vec()))
}

fn e<E: std::fmt::Debug>(x: E) -> String {
    format!("{x:?}")
}

impl Account {
    /// The account's group, made by its first node (whose credential the owner key signs).
    pub fn create(ident: AccountIdentity, owner_seed: &[u8; 32], node: &[u8; 32]) -> Result<Account, String> {
        let signer = new_signer()?;
        let cred = credential(&ident.did, node, &signer.1, owner_seed);
        let store = Store::default();
        let c = client(&ident, &store, &signer, &cred);
        let mut group = c.create_group_with_id(ident.did.to_vec(), ExtensionList::default(), Default::default(), None).map_err(e)?;
        group.write_to_storage().map_err(e)?;
        Ok(Account { group, removed: false, store, signer, ident, cred })
    }

    /// Join the account's group with the words alone: an external commit from its published group info. Returns the
    /// member and the commit, which every other member processes (the caller publishes it in the group's order).
    pub fn join(ident: AccountIdentity, owner_seed: &[u8; 32], node: &[u8; 32], group_info: &[u8]) -> Result<(Account, Vec<u8>), String> {
        let signer = new_signer()?;
        let cred = credential(&ident.did, node, &signer.1, owner_seed);
        let store = Store::default();
        let c = client(&ident, &store, &signer, &cred);
        let info = MlsMessage::from_bytes(group_info).map_err(e)?;
        let (mut group, commit) = c.external_commit_builder().map_err(e)?.build(info).map_err(e)?;
        group.write_to_storage().map_err(e)?;
        Ok((Account { group, removed: false, store, signer, ident, cred }, commit.to_bytes().map_err(e)?))
    }

    /// The group info that lets a node holding the words join (published beside the commits).
    pub fn group_info(&self) -> Result<Vec<u8>, String> {
        self.group.group_info_message_allowing_ext_commit(true).map_err(e)?.to_bytes().map_err(e)
    }

    /// A commit from another member (a join, a removal): applied, moving the epoch. One that removes THIS member marks it
    /// removed (nothing after it applies).
    pub fn process(&mut self, commit: &[u8]) -> Result<(), String> {
        if self.removed {
            return Err("this node was removed from the account".into());
        }
        let m = MlsMessage::from_bytes(commit).map_err(e)?;
        match self.group.process_incoming_message(m).map_err(e)? {
            ReceivedMessage::Commit(d) => {
                if matches!(d.effect, mls_rs::group::CommitEffect::Removed { .. }) {
                    self.removed = true;
                }
            }
            other => return Err(format!("not a commit: {other:?}")),
        }
        self.group.write_to_storage().map_err(e)
    }

    /// Remove the member at `index` (a lost or stolen node): the commit to publish; this member's epoch moves now.
    pub fn remove(&mut self, index: u32) -> Result<Vec<u8>, String> {
        let out = self.group.commit_builder().remove_member(index).map_err(e)?.build().map_err(e)?;
        self.group.apply_pending_commit().map_err(e)?;
        self.group.write_to_storage().map_err(e)?;
        out.commit_message.to_bytes().map_err(e)
    }

    pub fn epoch(&self) -> u64 {
        self.group.current_epoch()
    }

    /// The members — the account's NODES: `(index, node key)`, each from its owner-signed credential.
    pub fn members(&self) -> Vec<(u32, Vec<u8>)> {
        self.group
            .roster()
            .members()
            .into_iter()
            .filter_map(|m| {
                let basic = m.signing_identity.credential.as_basic()?;
                let (_, _, node, _) = read_credential(&basic.identifier)?;
                Some((m.index, node.to_vec()))
            })
            .collect()
    }

    pub fn my_index(&self) -> u32 {
        self.group.current_member_index()
    }

    /// This epoch's secret for TABLE KEYS: every member derives the same; a removed member cannot.
    pub fn epoch_secret(&self) -> Result<[u8; 32], String> {
        let s = self.group.export_secret(TABLE_KEYS, b"", 32).map_err(e)?;
        s.as_bytes().try_into().map_err(|_| "32 bytes".into())
    }

    /// The whole member state, for the identity delegate to keep.
    pub fn save(&mut self) -> Result<Vec<u8>, String> {
        self.group.write_to_storage().map_err(e)?;
        let put = |b: &mut Vec<u8>, x: &[u8]| {
            b.extend_from_slice(&(x.len() as u32).to_le_bytes());
            b.extend_from_slice(x);
        };
        let mut b = b"CWMS".to_vec();
        put(&mut b, &self.signer.0);
        put(&mut b, &self.signer.1);
        put(&mut b, &self.cred);
        put(&mut b, &self.store.encode());
        b.push(u8::from(self.removed));
        Ok(b)
    }

    pub fn load(ident: AccountIdentity, blob: &[u8]) -> Result<Account, String> {
        let mut b = blob.strip_prefix(b"CWMS").ok_or("not a member state")?;
        let mut take = || -> Result<Vec<u8>, String> {
            let (n, r) = b.split_at_checked(4).ok_or("short")?;
            let n = u32::from_le_bytes(n.try_into().expect("4")) as usize;
            let (x, r) = r.split_at_checked(n).ok_or("short")?;
            b = r;
            Ok(x.to_vec())
        };
        let (s, p, cred, store) = (take()?, take()?, take()?, take()?);
        let removed = b.first() == Some(&1);
        let store = Store::decode(&store).ok_or("the stored group does not read")?;
        let signer = (s, p);
        let c = client(&ident, &store, &signer, &cred);
        let group = c.load_group(&ident.did).map_err(e)?;
        Ok(Account { group, removed, store, signer, ident, cred })
    }
}

/// A table's key in an epoch: the identity delegate's one derivation.
pub use craftworks_identity::epoch_table_key as table_key;

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(owners: &[[u8; 32]]) -> AccountIdentity {
        AccountIdentity { did: [42; 32], owners: owners.iter().map(|s| SigningKey::from_bytes(s).verifying_key().to_bytes()).collect() }
    }

    #[test]
    fn nodes_join_with_the_words_share_the_epoch_and_a_removed_node_is_left_behind() {
        let owner = [1u8; 32];
        let id = ident(&[owner]);
        let mut a = Account::create(id.clone(), &owner, &[0xA; 32]).unwrap();
        // Node B joins by itself (no one else online) from the published group info.
        let (mut b, commit) = Account::join(id.clone(), &owner, &[0xB; 32], &a.group_info().unwrap()).unwrap();
        a.process(&commit).unwrap();
        assert_eq!(a.epoch(), b.epoch());
        assert_eq!(a.epoch_secret().unwrap(), b.epoch_secret().unwrap(), "one epoch, one secret");
        assert_eq!(table_key(&a.epoch_secret().unwrap(), "notes"), table_key(&b.epoch_secret().unwrap(), "notes"));
        // Node C, then C is removed (lost): A and B move on; C's last secret is not theirs any more.
        let (c, commit) = Account::join(id.clone(), &owner, &[0xC; 32], &a.group_info().unwrap()).unwrap();
        a.process(&commit).unwrap();
        b.process(&commit).unwrap();
        let c_secret = c.epoch_secret().unwrap();
        let removal = a.remove(c.my_index()).unwrap();
        b.process(&removal).unwrap();
        assert_eq!(a.epoch_secret().unwrap(), b.epoch_secret().unwrap());
        assert_ne!(a.epoch_secret().unwrap(), c_secret, "the removed node's secret opens nothing written now");
        let mut c = c;
        c.process(&removal).unwrap();
        assert!(c.removed, "the removed node learns it");
        assert!(c.process(&removal).is_err(), "and applies nothing after");
        let c2 = Account::load(id.clone(), &c.save().unwrap()).unwrap();
        assert!(c2.removed, "kept across a save");
        assert!(!a.removed && !b.removed);
        assert_eq!(a.members().len(), 2);
        let nodes: Vec<Vec<u8>> = a.members().into_iter().map(|(_, k)| k).collect();
        assert!(nodes.contains(&vec![0xA; 32]) && nodes.contains(&vec![0xB; 32]) && !nodes.contains(&vec![0xC; 32]), "the roster names the nodes");
        // Saved and loaded (the delegate keeps the blob): the same member, the same epoch.
        let blob = a.save().unwrap();
        let a2 = Account::load(id.clone(), &blob).unwrap();
        assert_eq!((a2.epoch(), a2.epoch_secret().unwrap()), (a.epoch(), a.epoch_secret().unwrap()));
    }

    #[test]
    fn only_the_owner_admits_and_a_rotated_owner_keeps_the_nodes() {
        let (owner, stranger, next) = ([1u8; 32], [9u8; 32], [2u8; 32]);
        let a = Account::create(ident(&[owner]), &owner, &[0xA; 32]).unwrap();
        // A credential not signed by the account's owner: the join is refused.
        assert!(Account::join(ident(&[owner]), &stranger, &[0xB; 32], &a.group_info().unwrap()).is_err(), "control: a stranger cannot join");
        // After the words changed (a new owner key in the log), a node admitted by the old owner is still a member,
        // and the new owner admits more.
        let rotated = ident(&[owner, next]);
        let mut a = Account::load(rotated.clone(), &{ let mut a = a; a.save().unwrap() }).unwrap();
        let (d, commit) = Account::join(rotated.clone(), &next, &[0xD; 32], &a.group_info().unwrap()).unwrap();
        a.process(&commit).unwrap();
        assert_eq!(a.epoch_secret().unwrap(), d.epoch_secret().unwrap());
    }
}

/// The page's side: one `Mls` per page, this node's member of the account's group.
#[cfg(target_arch = "wasm32")]
mod js {
    use super::*;
    use wasm_bindgen::prelude::*;

    fn err(e: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&e.to_string())
    }

    fn did32(b: &[u8]) -> Result<[u8; 32], JsValue> {
        b.try_into().map_err(|_| err("a DID is 32 bytes"))
    }

    /// The account's rule from its key log's bytes: verified here against the DID, never taken on trust.
    fn log_of(did: &[u8; 32], log: &[u8]) -> Result<craftworks_idlog_contract::Log, JsValue> {
        craftworks_idlog_contract::read(did, log).ok_or_else(|| err("the account's key log does not verify"))
    }

    fn ident(did: [u8; 32], log: &craftworks_idlog_contract::Log) -> AccountIdentity {
        AccountIdentity { did, owners: log.events.iter().map(|e| e.key).collect() }
    }

    /// This node's member, and the account's encryption public key (its key log's head): where escrows are sealed.
    #[wasm_bindgen]
    pub struct Mls(Option<Account>, [u8; 32]);

    #[wasm_bindgen]
    impl Mls {
        #[wasm_bindgen(constructor)]
        pub fn new() -> Mls {
            Mls(None, [0; 32])
        }

        /// With the WORDS (their owner key is derived here and never leaves): the group made (`group_info` empty) or
        /// joined by an external commit. `[kind, commit]`: "created" (no commit) or "joined".
        pub fn with_words(&mut self, did: &[u8], log: &[u8], entropy: &[u8], node: &[u8], group_info: &[u8]) -> Result<js_sys::Array, JsValue> {
            let node: [u8; 32] = node.try_into().map_err(|_| err("a node key is 32 bytes"))?;
            let did = did32(did)?;
            let log = log_of(&did, log)?;
            let (_, owner, _) = craftworks_account::open_log(entropy, &log).ok_or_else(|| err("these recovery words were replaced by newer ones"))?;
            let (acc, kind, commit) = if group_info.is_empty() {
                (Account::create(ident(did, &log), &owner, &node).map_err(err)?, "created", Vec::new())
            } else {
                let (a, c) = Account::join(ident(did, &log), &owner, &node, group_info).map_err(err)?;
                (a, "joined", c)
            };
            self.0 = Some(acc);
            self.1 = log.head().enc;
            Ok([JsValue::from(kind), js_sys::Uint8Array::from(&commit[..]).into()].into_iter().collect())
        }

        /// This node's member state, as the identity delegate kept it.
        pub fn load(&mut self, did: &[u8], log: &[u8], state: &[u8]) -> Result<(), JsValue> {
            let did = did32(did)?;
            let log = log_of(&did, log)?;
            self.0 = Some(Account::load(ident(did, &log), state).map_err(err)?);
            self.1 = log.head().enc;
            Ok(())
        }

        fn member(&mut self) -> Result<&mut Account, JsValue> {
            self.0.as_mut().ok_or_else(|| err("not a member of the account's group here yet"))
        }

        /// Another member's commit (a join, a removal), applied.
        pub fn process(&mut self, commit: &[u8]) -> Result<(), JsValue> {
            self.member()?.process(commit).map_err(err)
        }

        /// Remove the member at `index`: the commit to publish.
        pub fn remove(&mut self, index: u32) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(&self.member()?.remove(index).map_err(err)?[..]))
        }

        /// NEW WORDS: an escrow sealed for the old words, sealed again for the new (both are in hand while they change).
        pub fn reseal_escrow(old: &[u8], new: &[u8], blob: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            let secret = craftworks_account::escrow_open(old, blob).ok_or_else(|| err("that escrow does not open with the old words"))?;
            let enc = craftworks_account::enc_public_at(new, 0).ok_or_else(|| err("new recovery entropy: 16 or 32 bytes"))?;
            let mut eph = [0u8; 32];
            getrandom::getrandom(&mut eph).map_err(err)?;
            Ok(js_sys::Uint8Array::from(&craftworks_account::escrow_seal(&enc, &secret, eph)[..]))
        }

        /// From now on, escrows are sealed for these words (after they replaced the old ones).
        pub fn escrow_to(&mut self, entropy: &[u8]) -> Result<(), JsValue> {
            self.1 = craftworks_account::enc_public_at(entropy, 0).ok_or_else(|| err("recovery entropy: 16 or 32 bytes"))?;
            Ok(())
        }

        /// An epoch's secret out of its escrow, with the words (a node joining with them recovers the epochs before).
        pub fn open_escrow(entropy: &[u8], blob: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            let s = craftworks_account::escrow_open(entropy, blob).ok_or_else(|| err("that escrow does not open with these words"))?;
            Ok(js_sys::Uint8Array::from(&s[..]))
        }

        /// `{ epoch, secret, escrow, info, me, members: [{ index, key }], state }`: `escrow` is this epoch's secret sealed
        /// to the account's encryption key (to publish), with a fresh one-time key.
        pub fn status(&mut self) -> Result<js_sys::Object, JsValue> {
            let enc = self.1;
            let m = self.member()?;
            let o = js_sys::Object::new();
            let set = |k: &str, v: JsValue| js_sys::Reflect::set(&o, &k.into(), &v).map(|_| ());
            set("epoch", JsValue::from(m.epoch() as f64))?;
            let secret = m.epoch_secret().map_err(err)?;
            set("secret", js_sys::Uint8Array::from(&secret[..]).into())?;
            let mut eph = [0u8; 32];
            getrandom::getrandom(&mut eph).map_err(err)?;
            set("escrow", js_sys::Uint8Array::from(&craftworks_account::escrow_seal(&enc, &secret, eph)[..]).into())?;
            set("info", js_sys::Uint8Array::from(&m.group_info().map_err(err)?[..]).into())?;
            set("me", JsValue::from(m.my_index()))?;
            set("removed", JsValue::from(m.removed))?;
            let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
            let members: js_sys::Array = m
                .members()
                .into_iter()
                .map(|(i, k)| -> JsValue {
                    let x = js_sys::Object::new();
                    let _ = js_sys::Reflect::set(&x, &"index".into(), &JsValue::from(i));
                    let _ = js_sys::Reflect::set(&x, &"key".into(), &JsValue::from(hex(&k)));
                    x.into()
                })
                .collect();
            set("members", members.into())?;
            set("state", js_sys::Uint8Array::from(&m.save().map_err(err)?[..]).into())?;
            Ok(o)
        }
    }

    impl Default for Mls {
        fn default() -> Self {
            Self::new()
        }
    }
}
