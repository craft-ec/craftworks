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
const TABLE_KEYS: &[u8] = b"craftworks table keys";

// A member's credential (its format and check) is the ACCOUNT's: one source, shared with `membership` in the page.
pub use craftworks_account::{credential, read_credential};

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

/// WHO MAY BE IN A GROUP: the account's own nodes (`Account`), or — a SPACE's group (a server, a chat) — nodes of any
/// account, each with its account's owner-signed credential (who may ADD them is the space's rules', above MLS).
#[derive(Clone, Debug)]
pub enum Rule {
    Account(AccountIdentity),
    Space([u8; 32]),
}

impl Rule {
    fn group_id(&self) -> Vec<u8> {
        match self {
            Rule::Account(a) => a.did.to_vec(),
            Rule::Space(id) => [b"craftworks space ".as_slice(), id].concat(),
        }
    }

    fn check(&self, id: &SigningIdentity) -> Result<Vec<u8>, NotAMember> {
        match self {
            Rule::Account(a) => a.check(id),
            Rule::Space(_) => {
                let basic = id.credential.as_basic().ok_or(NotAMember("not a basic credential"))?;
                let (_, _, _, sp) = read_credential(&basic.identifier).ok_or(NotAMember("the owner's signature does not hold"))?;
                if sp != id.signature_key.as_bytes() {
                    return Err(NotAMember("the credential names another signing key"));
                }
                Ok(sp)
            }
        }
    }
}

impl IdentityProvider for Rule {
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

/// A node's KEY PACKAGES' secrets (MLS: what lets another member add this node while it is away), kept with its state:
/// a welcome may come days later, across page loads. `id` → the key package data, encoded.
#[derive(Clone, Default, Debug)]
pub struct KeyPackages(Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>);

impl mls_rs_core::key_package::KeyPackageStorage for KeyPackages {
    type Error = NotAMember;

    fn delete(&mut self, id: &[u8]) -> Result<(), Self::Error> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }

    fn insert(&mut self, id: Vec<u8>, pkg: mls_rs_core::key_package::KeyPackageData) -> Result<(), Self::Error> {
        use mls_rs::mls_rs_codec::MlsEncode;
        let b = pkg.mls_encode_to_vec().map_err(|_| NotAMember("a key package does not encode"))?;
        self.0.lock().unwrap().insert(id, b);
        Ok(())
    }

    fn get(&self, id: &[u8]) -> Result<Option<mls_rs_core::key_package::KeyPackageData>, Self::Error> {
        use mls_rs::mls_rs_codec::MlsDecode;
        Ok(self.0.lock().unwrap().get(id).and_then(|b| mls_rs_core::key_package::KeyPackageData::mls_decode(&mut &b[..]).ok()))
    }
}

impl KeyPackages {
    fn encode(&self) -> Vec<u8> {
        let m = self.0.lock().unwrap();
        let mut b = (m.len() as u32).to_le_bytes().to_vec();
        for (k, v) in m.iter() {
            for x in [k, v] {
                b.extend_from_slice(&(x.len() as u32).to_le_bytes());
                b.extend_from_slice(x);
            }
        }
        b
    }

    fn decode(mut b: &[u8]) -> Option<KeyPackages> {
        let mut take = |n: usize| -> Option<Vec<u8>> {
            let (x, r) = b.split_at_checked(n)?;
            b = r;
            Some(x.to_vec())
        };
        let n = u32::from_le_bytes(take(4)?.try_into().ok()?);
        let mut m = BTreeMap::new();
        for _ in 0..n {
            let kl = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
            let k = take(kl)?;
            let vl = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
            m.insert(k, take(vl)?);
        }
        Some(KeyPackages(Arc::new(Mutex::new(m))))
    }
}

type Config = WithIdentityProvider<
    Rule,
    WithCryptoProvider<
        RustCryptoProvider,
        mls_rs::client_builder::WithKeyPackageRepo<KeyPackages, mls_rs::client_builder::WithGroupStateStorage<Store, BaseConfig>>,
    >,
>;

/// This node, as a member of a group: the account's, or a space's.
pub struct Account {
    pub group: Group<Config>,
    /// A commit removed THIS member: it keeps what it held (the epochs before) and learns nothing newer.
    pub removed: bool,
    store: Store,
    signer: (Vec<u8>, Vec<u8>),
    ident: Rule,
    cred: Vec<u8>,
    kps: KeyPackages,
}

fn client(ident: &Rule, store: &Store, kps: &KeyPackages, signer: &(Vec<u8>, Vec<u8>), cred: &[u8]) -> Client<Config> {
    // MLS's clock on a page is the browser's (mls-rs-core, vendored, reads the host's; see Cargo.toml's patch).
    #[cfg(all(target_arch = "wasm32", feature = "page"))]
    mls_rs_core::time::set_clock(|| (js_sys::Date::now() / 1000.0) as u64);
    let id = SigningIdentity::new(BasicCredential::new(cred.to_vec()).into_credential(), SignaturePublicKey::new(signer.1.clone()));
    Client::builder()
        .group_state_storage(store.clone())
        .key_package_repo(kps.clone())
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
        Self::create_with(Rule::Account(ident), signer, cred)
    }

    fn create_with(ident: Rule, signer: (Vec<u8>, Vec<u8>), cred: Vec<u8>) -> Result<Account, String> {
        let store = Store::default();
        let kps = KeyPackages::default();
        let c = client(&ident, &store, &kps, &signer, &cred);
        let mut group = c.create_group_with_id(ident.group_id(), ExtensionList::default(), Default::default(), None).map_err(e)?;
        group.write_to_storage().map_err(e)?;
        Ok(Account { group, removed: false, store, signer, ident, cred, kps })
    }

    /// Join the account's group with the words alone: an external commit from its published group info. Returns the
    /// member and the commit, which every other member processes (the caller publishes it in the group's order).
    pub fn join(ident: AccountIdentity, owner_seed: &[u8; 32], node: &[u8; 32], group_info: &[u8]) -> Result<(Account, Vec<u8>), String> {
        let signer = new_signer()?;
        let cred = credential(&ident.did, node, &signer.1, owner_seed);
        let ident = Rule::Account(ident);
        let store = Store::default();
        let kps = KeyPackages::default();
        let c = client(&ident, &store, &kps, &signer, &cred);
        let info = MlsMessage::from_bytes(group_info).map_err(e)?;
        let (mut group, commit) = c.external_commit_builder().map_err(e)?.build(info).map_err(e)?;
        group.write_to_storage().map_err(e)?;
        Ok((Account { group, removed: false, store, signer, ident, cred, kps }, commit.to_bytes().map_err(e)?))
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

    /// UPDATE this member's own keys (a commit with no proposals: MLS gives it a fresh path — new leaf keys): what anyone
    /// holding this member's old keys (a device removed from the account) followed stops here. The commit to publish.
    pub fn update(&mut self) -> Result<Vec<u8>, String> {
        let out = self.group.commit_builder().build().map_err(e)?;
        self.group.apply_pending_commit().map_err(e)?;
        self.group.write_to_storage().map_err(e)?;
        out.commit_message.to_bytes().map_err(e)
    }

    /// Remove the member at `index` (a lost or stolen node): the commit to publish; this member's epoch moves now.
    pub fn remove(&mut self, index: u32) -> Result<Vec<u8>, String> {
        let out = self.group.commit_builder().remove_member(index).map_err(e)?.build().map_err(e)?;
        self.group.apply_pending_commit().map_err(e)?;
        self.group.write_to_storage().map_err(e)?;
        out.commit_message.to_bytes().map_err(e)
    }

    /// ADD a node (by its published key package) to this group: `(commit, welcome)` — the commit for the group's log,
    /// the welcome for the new node (delivered to it: its inbox). This member's epoch moves now.
    pub fn add(&mut self, key_package: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
        let kp = MlsMessage::from_bytes(key_package).map_err(e)?;
        let out = self.group.commit_builder().add_member(kp).map_err(e)?.build().map_err(e)?;
        self.group.apply_pending_commit().map_err(e)?;
        self.group.write_to_storage().map_err(e)?;
        let welcome = out.welcome_messages.first().ok_or("no welcome for the new member")?.to_bytes().map_err(e)?;
        Ok((out.commit_message.to_bytes().map_err(e)?, welcome))
    }

    pub fn epoch(&self) -> u64 {
        self.group.current_epoch()
    }

    /// The members — the account's NODES: `(index, node key, credential)`, each from its owner-signed credential.
    pub fn members(&self) -> Vec<(u32, Vec<u8>, Vec<u8>)> {
        self.group
            .roster()
            .members()
            .into_iter()
            .filter_map(|m| {
                let basic = m.signing_identity.credential.as_basic()?;
                let (_, _, node, _) = read_credential(&basic.identifier)?;
                Some((m.index, node.to_vec(), basic.identifier.clone()))
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
        // Its key packages' secrets (after the rest: a state saved before key packages still loads).
        b.extend_from_slice(&self.kps.encode());
        Ok(b)
    }

    pub fn load(ident: Rule, blob: &[u8]) -> Result<Account, String> {
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
        let kps = match b.get(1..) {
            Some(rest) if !rest.is_empty() => KeyPackages::decode(rest).ok_or("the key packages do not read")?,
            _ => KeyPackages::default(),
        };
        let store = Store::decode(&store).ok_or("the stored group does not read")?;
        let signer = (s, p);
        let c = client(&ident, &store, &kps, &signer, &cred);
        let group = c.load_group(&ident.group_id()).map_err(e)?;
        Ok(Account { group, removed, store, signer, ident, cred, kps })
    }
}

/// A DID's MEMBER in spaces (ARCHITECTURE: a DID is the member, a device only signs in): its MLS signing key (from a
/// seed the identity derives from the account's data seed — the same on every device), its credential (signed by the
/// account's data key), and its KEY PACKAGES' secrets, which the account's devices share (`packages`, kept in the
/// account's storage): a welcome answering one may reach any of them. It makes and joins spaces' groups.
pub struct SpaceMember {
    signer: (Vec<u8>, Vec<u8>),
    cred: Vec<u8>,
    kps: KeyPackages,
}

impl SpaceMember {
    /// From the identity's seed and credential, with the key packages kept so far (empty: none).
    pub fn new(seed: &[u8; 32], cred: Vec<u8>, packages: &[u8]) -> Result<SpaceMember, String> {
        let k = ed25519_dalek::SigningKey::from_bytes(seed);
        let (_, _, _, sp) = read_credential(&cred).ok_or("the member's credential does not hold")?;
        if sp != k.verifying_key().to_bytes() {
            return Err("the credential names another signing key".into());
        }
        let kps = if packages.is_empty() { KeyPackages::default() } else { KeyPackages::decode(packages).ok_or("the key packages do not read")? };
        Ok(SpaceMember { signer: (k.to_keypair_bytes().to_vec(), k.verifying_key().to_bytes().to_vec()), cred, kps })
    }

    /// The key packages' secrets, to keep (after making one, and after joining: a used one is gone).
    pub fn packages(&self) -> Vec<u8> {
        self.kps.encode()
    }

    /// A KEY PACKAGE to publish (on the person's card): anyone adds this DID to a space with it, while it is away.
    pub fn key_package(&self) -> Result<Vec<u8>, String> {
        let (store, ident) = (Store::default(), Rule::Space([0; 32]));
        let c = client(&ident, &store, &self.kps, &self.signer, &self.cred);
        c.generate_key_package_message(ExtensionList::default(), ExtensionList::default(), None).map_err(e)?.to_bytes().map_err(e)
    }

    /// A SPACE's group, made by this DID (its first member).
    pub fn create_space(&self, space: [u8; 32]) -> Result<Account, String> {
        Account::create_with(Rule::Space(space), self.signer.clone(), self.cred.clone())
    }

    /// JOIN a space's group from a WELCOME (a member added this DID by one of its key packages).
    pub fn join_space(&self, space: [u8; 32], welcome: &[u8]) -> Result<Account, String> {
        let ident = Rule::Space(space);
        let store = Store::default();
        let c = client(&ident, &store, &self.kps, &self.signer, &self.cred);
        let msg = MlsMessage::from_bytes(welcome).map_err(e)?;
        // Which group, BEFORE joining: a welcome to another space must not use up a key package.
        if c.examine_welcome_message(&msg).map_err(e)?.group_context().group_id() != ident.group_id() {
            return Err("that welcome is to another space".into());
        }
        let (mut group, _) = c.join_group(None, &msg, None).map_err(e)?;
        group.write_to_storage().map_err(e)?;
        Ok(Account { group, removed: false, store, signer: self.signer.clone(), ident, cred: self.cred.clone(), kps: KeyPackages::default() })
    }
}

/// A table's key in an epoch: the identity delegate's one derivation.
pub use craftworks_identity::epoch_table_key as table_key;

#[cfg(test)]
mod tests {
    use ed25519_dalek::SigningKey;
    use super::*;

    fn ident(owners: &[[u8; 32]]) -> AccountIdentity {
        AccountIdentity { did: [42; 32], owners: owners.iter().map(|s| SigningKey::from_bytes(s).verifying_key().to_bytes()).collect() }
    }

    /// A DID's member for spaces, as its account's devices each make it (the identity's seed and credential).
    fn member(did: [u8; 32], data: [u8; 32], packages: &[u8]) -> SpaceMember {
        let seed = craftworks_identity::space_member_seed(&data);
        let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
        SpaceMember::new(&seed, craftworks_identity::space_member_credential(&did, &data, &public), packages).unwrap()
    }
    fn writer(data: [u8; 32]) -> Vec<u8> {
        craftworks_identity::space_writer(&data).verifying_key().to_bytes().to_vec()
    }

    /// ADD a DID by its KEY PACKAGE: Bob's member publishes one from one device; Alice adds Bob to her space; Bob's
    /// OTHER device (the same member, the key packages the account keeps) joins from the welcome — one member per DID,
    /// the same group, the same keys.
    #[test]
    fn a_did_is_added_by_its_key_package_and_any_of_its_devices_joins() {
        let (alice, bob_phone) = (member([0xA; 32], [0xAA; 32], &[]), member([0xB; 32], [0xBB; 32], &[]));
        let kp = bob_phone.key_package().unwrap();
        let bob_laptop = member([0xB; 32], [0xBB; 32], &bob_phone.packages());
        let space = [0x5A; 32];
        let mut dm = alice.create_space(space).unwrap();
        let (commit, welcome) = dm.add(&kp).unwrap();
        assert!(!commit.is_empty());
        // A wrong space first: refused before the key package is used — the right one still joins after.
        assert_eq!(bob_laptop.join_space([0x5B; 32], &welcome).err().as_deref(), Some("that welcome is to another space"));
        let joined = bob_laptop.join_space(space, &welcome).unwrap();
        assert_eq!(joined.epoch(), dm.epoch());
        assert_eq!(joined.epoch_secret().unwrap(), dm.epoch_secret().unwrap(), "one group, one key");
        let writers: Vec<Vec<u8>> = joined.members().into_iter().map(|(_, w, _)| w).collect();
        assert_eq!(writers, vec![writer([0xAA; 32]), writer([0xBB; 32])], "one member per DID, named by its writer");
        // Control: a device without the account's key packages cannot answer the welcome.
        assert!(member([0xB; 32], [0xBB; 32], &[]).join_space(space, &welcome).is_err());
    }

    /// A SPACE's group, made by a DID's member: its own epoch secret, kept and loaded again by its space id; a member
    /// whose credential names another key is refused.
    #[test]
    fn a_did_makes_a_spaces_group_and_its_credential_must_be_its_own() {
        let a = member([0xA; 32], [0xAA; 32], &[]);
        let space = [0x5A; 32];
        let mut g = a.create_space(space).unwrap();
        assert_eq!(g.members().len(), 1);
        assert_eq!(g.members()[0].1, writer([0xAA; 32]));
        let blob = g.save().unwrap();
        let g2 = Account::load(Rule::Space(space), &blob).unwrap();
        assert_eq!(g2.epoch_secret().unwrap(), g.epoch_secret().unwrap());
        // Control: loaded as another space, it is not found.
        assert!(Account::load(Rule::Space([0x5B; 32]), &blob).is_err());
        // Another account's credential with this seed: refused.
        let seed = craftworks_identity::space_member_seed(&[0xAA; 32]);
        let other = craftworks_identity::space_member_credential(&[0xB; 32], &[0xBB; 32], &SigningKey::from_bytes(&[3; 32]).verifying_key().to_bytes());
        assert!(SpaceMember::new(&seed, other, &[]).is_err());
    }

    /// UPDATE: a DID's member refreshes its keys (a device of the account was removed). The other member follows; a
    /// copy of the member from before the update (what the removed device holds) cannot follow the next change —
    /// control: without the update, that copy follows it.
    #[test]
    fn a_members_update_leaves_a_stale_copy_behind() {
        let (alice, bob) = (member([0xA; 32], [0xAA; 32], &[]), member([0xB; 32], [0xBB; 32], &[]));
        let (carol, dave) = (member([0xC; 32], [0xCC; 32], &[]), member([0xD; 32], [0xDD; 32], &[]));
        let space = [0x5A; 32];
        let mut a = alice.create_space(space).unwrap();
        let (_, welcome) = a.add(&bob.key_package().unwrap()).unwrap();
        let mut b = bob.join_space(space, &welcome).unwrap();
        let copy_of = |b: &mut Account| Account::load(Rule::Space(space), &b.save().unwrap()).unwrap();
        // Control: a copy of Bob, keys current — follows Alice adding Carol.
        let mut copy = copy_of(&mut b);
        let (add_carol, _) = a.add(&carol.key_package().unwrap()).unwrap();
        assert!(copy.process(&add_carol).is_ok(), "control: a copy follows while its keys are current");
        b.process(&add_carol).unwrap();
        // Bob refreshes his keys; Alice follows; the copy from before cannot follow what comes next.
        let mut copy = copy_of(&mut b);
        let up = b.update().unwrap();
        a.process(&up).unwrap();
        assert_eq!(a.epoch_secret().unwrap(), b.epoch_secret().unwrap(), "Alice follows Bob's update");
        let (add_dave, _) = a.add(&dave.key_package().unwrap()).unwrap();
        b.process(&add_dave).unwrap();
        assert!(copy.process(&up).is_err(), "the member's own update is not the copy's to follow");
        assert!(copy.process(&add_dave).is_err(), "and what follows it is out of the copy's reach");
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
        let c2 = Account::load(Rule::Account(id.clone()), &c.save().unwrap()).unwrap();
        assert!(c2.removed, "kept across a save");
        assert!(!a.removed && !b.removed);
        assert_eq!(a.members().len(), 2);
        let nodes: Vec<Vec<u8>> = a.members().into_iter().map(|(_, k, _)| k).collect();
        assert!(nodes.contains(&vec![0xA; 32]) && nodes.contains(&vec![0xB; 32]) && !nodes.contains(&vec![0xC; 32]), "the roster names the nodes");
        // Saved and loaded (the delegate keeps the blob): the same member, the same epoch.
        let blob = a.save().unwrap();
        let a2 = Account::load(Rule::Account(id.clone()), &blob).unwrap();
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
        let mut a = Account::load(Rule::Account(rotated.clone()), &{ let mut a = a; a.save().unwrap() }).unwrap();
        let (d, commit) = Account::join(rotated.clone(), &next, &[0xD; 32], &a.group_info().unwrap()).unwrap();
        a.process(&commit).unwrap();
        assert_eq!(a.epoch_secret().unwrap(), d.epoch_secret().unwrap());
    }
}

/// The page's side: one `Mls` per page, this node's member of the account's group.
#[cfg(all(target_arch = "wasm32", feature = "page"))]
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
            self.0 = Some(Account::load(Rule::Account(ident(did, &log)), state).map_err(err)?);
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

        /// UPDATE this member's own keys: the commit to publish (a removed device's copy follows nothing after it).
        pub fn update(&mut self) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(&self.member()?.update().map_err(err)?[..]))
        }

        /// Remove the member at `index`: the commit to publish.
        pub fn remove(&mut self, index: u32) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(&self.member()?.remove(index).map_err(err)?[..]))
        }

        /// ADD a member (a DID) by its key package: `[commit, welcome]` (the commit for the log, the welcome for it).
        pub fn add(&mut self, key_package: &[u8]) -> Result<js_sys::Array, JsValue> {
            let (c, w) = self.member()?.add(key_package).map_err(err)?;
            Ok([js_sys::Uint8Array::from(&c[..]), js_sys::Uint8Array::from(&w[..])].into_iter().map(JsValue::from).collect())
        }

        /// A space's group, as the identity delegate kept it.
        pub fn load_space(space: &[u8], state: &[u8]) -> Result<Mls, JsValue> {
            Ok(Mls(Some(Account::load(Rule::Space(did32(space)?), state).map_err(err)?), [0; 32]))
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

        /// `{ epoch, secret, escrow, info, me, members: [{ index, key, cred }], state }`: `escrow` is this epoch's secret sealed
        /// to the account's encryption key (to publish), with a fresh one-time key.
        pub fn status(&mut self) -> Result<js_sys::Object, JsValue> {
            let enc = self.1;
            let m = self.member()?;
            let o = js_sys::Object::new();
            let set = |k: &str, v: JsValue| js_sys::Reflect::set(&o, &k.into(), &v).map(|_| ());
            set("epoch", JsValue::from(m.epoch() as f64))?;
            let secret = m.epoch_secret().map_err(err)?;
            set("secret", js_sys::Uint8Array::from(&secret[..]).into())?;
            // Escrowed to the words: the account's group only (a space's has no words).
            if enc != [0; 32] {
                let mut eph = [0u8; 32];
                getrandom::getrandom(&mut eph).map_err(err)?;
                set("escrow", js_sys::Uint8Array::from(&craftworks_account::escrow_seal(&enc, &secret, eph)[..]).into())?;
            }
            set("info", js_sys::Uint8Array::from(&m.group_info().map_err(err)?[..]).into())?;
            set("me", JsValue::from(m.my_index()))?;
            set("removed", JsValue::from(m.removed))?;
            let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
            let members: js_sys::Array = m
                .members()
                .into_iter()
                .map(|(i, k, c)| -> JsValue {
                    let x = js_sys::Object::new();
                    let _ = js_sys::Reflect::set(&x, &"index".into(), &JsValue::from(i));
                    let _ = js_sys::Reflect::set(&x, &"key".into(), &JsValue::from(hex(&k)));
                    let _ = js_sys::Reflect::set(&x, &"cred".into(), &JsValue::from(hex(&c)));
                    x.into()
                })
                .collect();
            set("members", members.into())?;
            set("state", js_sys::Uint8Array::from(&m.save().map_err(err)?[..]).into())?;
            Ok(o)
        }
    }

    /// A DID's member for spaces (the page's side of `SpaceMember`): from the identity's seed and credential and the key
    /// packages the account keeps.
    #[wasm_bindgen(js_name = SpaceMember)]
    pub struct JsSpaceMember(super::SpaceMember);

    #[wasm_bindgen(js_class = SpaceMember)]
    impl JsSpaceMember {
        #[wasm_bindgen(constructor)]
        pub fn new(seed: &[u8], credential: &[u8], packages: &[u8]) -> Result<JsSpaceMember, JsValue> {
            Ok(JsSpaceMember(super::SpaceMember::new(&did32(seed)?, credential.to_vec(), packages).map_err(err)?))
        }

        /// The key packages' secrets, to keep in the account's storage.
        pub fn packages(&self) -> js_sys::Uint8Array {
            js_sys::Uint8Array::from(&self.0.packages()[..])
        }

        /// A key package to publish on the card (keep `packages()` after).
        pub fn key_package(&self) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(&self.0.key_package().map_err(err)?[..]))
        }

        /// A space's group, made by this DID: a new `Mls`.
        pub fn create_space(&self, space: &[u8]) -> Result<Mls, JsValue> {
            Ok(Mls(Some(self.0.create_space(did32(space)?).map_err(err)?), [0; 32]))
        }

        /// JOIN a space's group from a welcome: a new `Mls` (keep `packages()` after: the one it answered is used up).
        pub fn join_space(&self, space: &[u8], welcome: &[u8]) -> Result<Mls, JsValue> {
            Ok(Mls(Some(self.0.join_space(did32(space)?, welcome).map_err(err)?), [0; 32]))
        }
    }

    impl Default for Mls {
        fn default() -> Self {
            Self::new()
        }
    }
}
