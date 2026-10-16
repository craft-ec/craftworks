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
/// A SELF-ENTRY's PROOF in the entrant's own leaf (a private-use leaf extension; FOUNDATION "Members and keys" 2): what
/// every reader — and the node's follower, until their own `join` act is written — judges their entry by
/// (`gov::Gov::admits`). Signed with the leaf, by the entrant.
pub const PROOF: u16 = 0xF0A1;
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

/// A COMMIT's CHANGE as the list check reads it (`Account::apply_checked`: the group before and after compared).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    /// An external commit (a device entering by itself).
    pub external: bool,
    /// Whose commit (its credential's DID).
    pub committer: String,
    /// Accounts with a new entry.
    pub added: Vec<String>,
    /// Accounts whose entry went.
    pub removed: Vec<String>,
    /// What the commit names in the clear (an entry's proof: `gov::Gov::admits`).
    pub aad: Vec<u8>,
}

/// THE LIST an entry is judged by — `gov::Gov`'s answers that never change as more acts are read (acts only grow: I5,
/// every reader alike whenever it reads): EVER on the list (and not banned); a self-entry's proof (`entry_admits`);
/// whether an account may be taken out by a member's commit (ever put out, or never listed — `may_be_taken_out`).
pub trait List {
    fn listed(&self, did: &str) -> bool;
    fn admits(&self, did: &str, proof: &[u8]) -> bool;
    fn may_take_out(&self, did: &str) -> bool;
}
/// Every credential its MLS rule accepts (the account's own group: its owner's signature IS its list).
pub struct Anyone;
impl List for Anyone {
    fn listed(&self, _: &str) -> bool {
        true
    }
    fn admits(&self, _: &str, _: &[u8]) -> bool {
        false
    }
    fn may_take_out(&self, _: &str) -> bool {
        true
    }
}

/// THE ENTRY CHECK (FOUNDATION "Members and keys", 5) — the one rule every reader (page, node) applies to every commit
/// before taking it: `Err(why)` and the commit is skipped. The committer listed (or entering by itself with a proof the
/// acts admit); everyone added listed; an entry takes out only its committer's own; a member's commit takes out only
/// accounts the acts put out or never listed (no member removes a listed account on its own).
pub fn entry_ok(c: &Change, l: &impl List) -> Result<(), String> {
    let short = |d: &str| d.get(..d.len().min(20)).unwrap_or(d).to_string();
    let entering = c.external && !l.listed(&c.committer) && l.admits(&c.committer, &c.aad);
    let listed = |d: &str| l.listed(d) || (entering && d == c.committer);
    if !listed(&c.committer) {
        return Err(format!("{} committed but is not on the list", short(&c.committer)));
    }
    if let Some(d) = c.added.iter().find(|d| !listed(d)) {
        return Err(format!("{} added but not on the list", short(d)));
    }
    if let Some(d) = c.removed.iter().find(|d| **d != c.committer && (c.external || !l.may_take_out(d))) {
        return Err(if c.external { format!("{} entering took out {} — an entry takes out only its own", short(&c.committer), short(d)) } else { format!("{} took out {} — on the list: only the list puts someone out", short(&c.committer), short(d)) });
    }
    Ok(())
}

/// What a commit did to a group: `Applied`, or `Skipped` by the entry check (the group as it was before).
#[derive(Debug, PartialEq, Eq)]
pub enum Applied {
    Applied(Change),
    Skipped(String),
}

/// WHAT A GROUP INFO SAYS (read before entering from it): its epoch, the DID of the member who signed it, and every
/// entry — `(index, its signing key, its DID)`.
#[derive(Debug)]
pub struct InfoFacts {
    pub epoch: u64,
    pub signer: String,
    pub members: Vec<(u32, Vec<u8>, String)>,
}

fn did_of(id: &SigningIdentity) -> Option<String> {
    let basic = id.credential.as_basic()?;
    read_credential(&basic.identifier).map(|(did, ..)| craftworks_account::did(&did))
}

/// A group info's facts (its tree travels in it: `group_info_message_allowing_ext_commit(true)`).
pub fn info_facts(group_info: &[u8]) -> Result<InfoFacts, String> {
    let m = MlsMessage::from_bytes(group_info).map_err(e)?;
    let info = m.as_group_info().ok_or("not a group info")?;
    let tree = info.extensions().get_as::<mls_rs::extension::built_in::RatchetTreeExt>().map_err(e)?.ok_or("the group info carries no tree")?;
    let roster = tree.tree_data.roster();
    let members: Vec<(u32, Vec<u8>, String)> =
        roster.members().into_iter().filter_map(|x| Some((x.index, x.signing_identity.signature_key.as_bytes().to_vec(), did_of(&x.signing_identity)?))).collect();
    let signer = members.iter().find(|x| x.0 == info.sender()).map(|x| x.2.clone()).ok_or("the group info's signer is not in its tree")?;
    Ok(InfoFacts { epoch: info.group_context().epoch, signer, members })
}

/// ENTER a group BY ITSELF (RFC 9420 external commit) — the ONE way in, for every kind: the account's node with the words
/// (`Account::join`), a listed account's member of a space (`SpaceMember::enter`). Its own old entry (the same signing
/// key: a space's member is the account's, shared by its devices) out in the same commit. `aad`: what the commit names
/// in the clear (its provenance). The member and the commit (the caller publishes it at the info's epoch).
fn enter_with(ident: Rule, signer: (Vec<u8>, Vec<u8>), cred: Vec<u8>, group_info: &[u8], aad: &[u8]) -> Result<(Account, Vec<u8>), String> {
    let facts = info_facts(group_info)?;
    let store = Store::default();
    let kps = KeyPackages::default();
    let c = client(&ident, &store, &kps, &signer, &cred);
    let mut b = c.external_commit_builder().map_err(e)?.with_authenticated_data(aad.to_vec());
    // Its PROOF in its own leaf too (the tree carries it to every reader, after this commit as well).
    if !aad.is_empty() {
        let mut exts = ExtensionList::default();
        exts.set(mls_rs_core::extension::Extension::new(mls_rs::extension::ExtensionType::new(PROOF), aad.to_vec()));
        b = b.with_leaf_node_extensions(exts);
    }
    if let Some(old) = facts.members.iter().find(|x| x.1 == signer.1).map(|x| x.0) {
        b = b.with_removal(old);
    }
    let (mut group, commit) = b.build(MlsMessage::from_bytes(group_info).map_err(e)?).map_err(e)?;
    group.write_to_storage().map_err(e)?;
    Ok((Account { group, removed: false, store, signer, ident, cred, kps }, commit.to_bytes().map_err(e)?))
}

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
        .extension_type(mls_rs::extension::ExtensionType::new(PROOF))
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
        enter_with(Rule::Account(ident), signer, cred, group_info, &[])
    }

    /// The group info that lets a node holding the words join (published beside the commits).
    pub fn group_info(&self) -> Result<Vec<u8>, String> {
        self.group.group_info_message_allowing_ext_commit(true).map_err(e)?.to_bytes().map_err(e)
    }

    /// A commit from another member (a join, a removal): applied, moving the epoch. One that removes THIS member marks it
    /// removed (nothing after it applies). (Unchecked: the account's group — its MLS rule is its list.)
    pub fn process(&mut self, commit: &[u8]) -> Result<(), String> {
        match self.apply_checked(commit, &Anyone)? {
            Applied::Applied(_) => Ok(()),
            Applied::Skipped(why) => Err(why),
        }
    }

    /// A commit APPLIED ONLY IF THE LIST ALLOWS IT (`entry_ok`): tried, the group before and after compared — skipped, the
    /// group stays as it was. `Err`: MLS would not apply it (not this group's next commit, a bad signature).
    pub fn apply_checked(&mut self, commit: &[u8], list: &impl List) -> Result<Applied, String> {
        if self.removed {
            return Err("this node was removed from the group".into());
        }
        let before = self.save()?;
        let was: BTreeMap<u32, String> = self.group.roster().members().iter().filter_map(|m| Some((m.index, did_of(&m.signing_identity)?))).collect();
        let me = was.get(&self.group.current_member_index()).cloned().unwrap_or_default();
        let m = MlsMessage::from_bytes(commit).map_err(e)?;
        let d = match self.group.process_incoming_message(m).map_err(e)? {
            ReceivedMessage::Commit(d) => d,
            other => return Err(format!("not a commit: {other:?}")),
        };
        let gone_me = matches!(d.effect, mls_rs::group::CommitEffect::Removed { .. });
        let now: BTreeMap<u32, String> = if gone_me {
            BTreeMap::new()
        } else {
            self.group.roster().members().iter().filter_map(|m| Some((m.index, did_of(&m.signing_identity)?))).collect()
        };
        // (Taken out by it: the new tree is not seen here — an ENTRY that takes this one out is its own account's, a
        // member's commit's committer is in the old tree.)
        let committer = if gone_me && d.is_external { me.clone() } else if gone_me { was.get(&d.committer).cloned().unwrap_or_default() } else { now.get(&d.committer).cloned().unwrap_or_default() };
        let change = Change {
            external: d.is_external,
            committer,
            added: now.iter().filter(|(i, x)| was.get(i) != Some(x)).map(|(_, x)| x.clone()).collect(),
            removed: if gone_me { vec![me] } else { was.iter().filter(|(i, x)| now.get(i) != Some(x)).map(|(_, x)| x.clone()).collect() },
            aad: d.authenticated_data.clone(),
        };
        if let Err(why) = entry_ok(&change, list) {
            *self = Account::load(self.ident.clone(), &before)?;
            return Ok(Applied::Skipped(why));
        }
        if gone_me {
            self.removed = true;
        }
        self.group.write_to_storage().map_err(e)?;
        Ok(Applied::Applied(change))
    }

    /// UPDATE this member's own keys (a commit with no proposals: MLS gives it a fresh path — new leaf keys): what anyone
    /// holding this member's old keys (a device removed from the account) followed stops here. The commit to publish.
    pub fn update(&mut self) -> Result<Vec<u8>, String> {
        let out = self.group.commit_builder().build().map_err(e)?;
        self.group.apply_pending_commit().map_err(e)?;
        self.group.write_to_storage().map_err(e)?;
        out.commit_message.to_bytes().map_err(e)
    }

    /// TAKE OUT the members at `indexes` (a lost or stolen node; whoever the list no longer names — `rules::members::
    /// follow`): ONE commit to publish; this member's epoch moves now.
    pub fn remove(&mut self, indexes: &[u32]) -> Result<Vec<u8>, String> {
        let mut b = self.group.commit_builder();
        for i in indexes {
            b = b.remove_member(*i).map_err(e)?;
        }
        let out = b.build().map_err(e)?;
        self.group.apply_pending_commit().map_err(e)?;
        self.group.write_to_storage().map_err(e)?;
        out.commit_message.to_bytes().map_err(e)
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

    /// EACH ENTRY with whose it is and the proof in its leaf (empty: none) — `(index, did, proof)`: what the follower
    /// judges (`rules::members::Leaf`).
    pub fn entries(&self) -> Vec<(u32, String, Vec<u8>)> {
        self.group
            .roster()
            .members()
            .into_iter()
            .filter_map(|m| {
                let proof = m.extensions.get(mls_rs::extension::ExtensionType::new(PROOF)).map(|x| x.extension_data).unwrap_or_default();
                Some((m.index, did_of(&m.signing_identity)?, proof))
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
/// seed the identity derives from the account's data seed — the same on every device) and its credential (signed by
/// the account's data key). It makes a space's group, or ENTERS one by itself (FOUNDATION "Members and keys").
pub struct SpaceMember {
    signer: (Vec<u8>, Vec<u8>),
    cred: Vec<u8>,
}

impl SpaceMember {
    /// From the identity's seed and credential.
    pub fn new(seed: &[u8; 32], cred: Vec<u8>) -> Result<SpaceMember, String> {
        let k = ed25519_dalek::SigningKey::from_bytes(seed);
        let (_, _, _, sp) = read_credential(&cred).ok_or("the member's credential does not hold")?;
        if sp != k.verifying_key().to_bytes() {
            return Err("the credential names another signing key".into());
        }
        Ok(SpaceMember { signer: (k.to_keypair_bytes().to_vec(), k.verifying_key().to_bytes().to_vec()), cred })
    }

    /// A SPACE's group, made by this DID (its first member).
    pub fn create_space(&self, space: [u8; 32]) -> Result<Account, String> {
        Account::create_with(Rule::Space(space), self.signer.clone(), self.cred.clone())
    }

    /// ENTER a space's group by itself (`enter_with`): a listed account's member, from the group info published at its
    /// newest epoch — its own old entry out in the same commit. The member and the commit.
    pub fn enter(&self, space: [u8; 32], group_info: &[u8], aad: &[u8]) -> Result<(Account, Vec<u8>), String> {
        enter_with(Rule::Space(space), self.signer.clone(), self.cred.clone(), group_info, aad)
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
    fn member(did: [u8; 32], data: [u8; 32]) -> SpaceMember {
        let seed = craftworks_identity::space_member_seed(&data);
        let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
        SpaceMember::new(&seed, craftworks_identity::space_member_credential(&did, &data, &public)).unwrap()
    }
    fn writer(data: [u8; 32]) -> Vec<u8> {
        craftworks_identity::space_writer(&data).verifying_key().to_bytes().to_vec()
    }

    /// ENTERING BY ITSELF (FOUNDATION "Members and keys" 1, 5): a listed account's member enters a space from its group
    /// info — nobody answers; every reader checks the entry against the list; an outsider's is skipped; a member that
    /// lost its state enters again, its old entry out in the same commit (one entry per account, always).
    #[test]
    fn a_listed_member_enters_by_itself_an_outsider_is_skipped_and_reentry_replaces_the_old_entry() {
        let space = [0x5C; 32];
        let (alice, bob, eve) = (member([0xA; 32], [0xAA; 32]), member([0xB; 32], [0xBB; 32]), member([0xE; 32], [0xEE; 32]));
        let (a, b, x) = (craftworks_account::did(&[0xA; 32]), craftworks_account::did(&[0xB; 32]), craftworks_account::did(&[0xE; 32]));
        let mut g = alice.create_space(space).unwrap();
        let facts = info_facts(&g.group_info().unwrap()).unwrap();
        assert_eq!((facts.epoch, facts.signer.clone(), facts.members.len()), (0, a.clone(), 1), "the info says its epoch and who signed it");
        // THE LIST as a reader holds it: who is listed; whose proof the acts admit (`proof`: the aad that admits them).
        struct L(Vec<String>, Option<(String, Vec<u8>)>);
        impl List for L {
            fn listed(&self, d: &str) -> bool {
                self.0.iter().any(|x| x == d)
            }
            fn admits(&self, d: &str, aad: &[u8]) -> bool {
                self.1.as_ref().is_some_and(|(x, p)| x == d && p == aad)
            }
            fn may_take_out(&self, d: &str) -> bool {
                !self.listed(d)
            }
        }
        let listed = |l: &[&String]| L(l.iter().map(|s| s.to_string()).collect(), None);
        // Bob, listed, enters by itself.
        let (mut bob1, commit) = bob.enter(space, &g.group_info().unwrap(), b"enter").unwrap();
        // Control: a reader whose list does not name Bob skips the very same commit, its group as it was.
        let mut copy = Account::load(Rule::Space(space), &g.save().unwrap()).unwrap();
        assert!(matches!(copy.apply_checked(&commit, &listed(&[&a])).unwrap(), Applied::Skipped(_)));
        assert_eq!(copy.epoch(), 0, "skipped: nothing moved");
        let Applied::Applied(c) = g.apply_checked(&commit, &listed(&[&a, &b])).unwrap() else { panic!("listed: taken") };
        assert_eq!((c.external, c.committer.clone(), c.added.clone(), c.removed.clone()), (true, b.clone(), vec![b.clone()], vec![]));
        assert_eq!((g.epoch(), g.epoch_secret().unwrap()), (bob1.epoch(), bob1.epoch_secret().unwrap()), "one group, one key");
        // Eve, NOT listed, enters the same way: every reader skips it (it never holds the position).
        let (_, evil) = eve.enter(space, &g.group_info().unwrap(), b"").unwrap();
        assert!(matches!(g.apply_checked(&evil, &listed(&[&a, &b])).unwrap(), Applied::Skipped(_)));
        // Control: with Eve on the list the same commit is taken.
        let mut copy = Account::load(Rule::Space(space), &g.save().unwrap()).unwrap();
        assert!(matches!(copy.apply_checked(&evil, &listed(&[&a, &b, &x])).unwrap(), Applied::Applied(_)));
        // Bob's device LOSES its state; Bob enters again: his old entry out in the same commit — still one entry.
        let (mut bob2, again) = bob.enter(space, &g.group_info().unwrap(), b"again").unwrap();
        let Applied::Applied(c) = g.apply_checked(&again, &listed(&[&a, &b])).unwrap() else { panic!("taken") };
        assert_eq!((c.committer.clone(), c.added.clone(), c.removed.clone()), (b.clone(), vec![], vec![]), "his entry replaced in place: the same account, nobody else touched");
        assert_eq!(g.members().len(), 2);
        assert_eq!(g.epoch_secret().unwrap(), bob2.epoch_secret().unwrap());
        // The old copy of Bob reads that commit as its own removal (it never reads past it).
        bob1.process(&again).unwrap();
        assert!(bob1.removed);
        // An entry taking out ANOTHER account: refused by the check (built by hand: `enter` only ever takes out its own).
        let c = Change { external: true, committer: b.clone(), added: vec![b.clone()], removed: vec![a.clone()], aad: vec![] };
        assert!(entry_ok(&c, &Anyone).is_err());
        // A MEMBER's commit taking out a LISTED account: skipped (only the list puts someone out); a stray: taken.
        let c = Change { external: false, committer: b.clone(), added: vec![], removed: vec![a.clone()], aad: vec![] };
        assert!(entry_ok(&c, &listed(&[&a, &b])).is_err());
        assert!(entry_ok(&Change { removed: vec!["did:stray".into()], ..c }, &listed(&[&a, &b])).is_ok());
        // A device NOT listed entering WITH A PROOF the acts admit (a code, the open door): taken; the same without it: not.
        let (_, door) = eve.enter(space, &g.group_info().unwrap(), b"proof").unwrap();
        let mut copy = Account::load(Rule::Space(space), &g.save().unwrap()).unwrap();
        assert!(matches!(copy.apply_checked(&door, &L(vec![a.clone(), b.clone()], Some((x.clone(), b"proof".to_vec())))).unwrap(), Applied::Applied(_)));
        // Its PROOF stays in its leaf, for every reader after (the follower judges it there).
        assert!(copy.entries().iter().any(|(_, d, p)| *d == x && p == b"proof"), "the proof in Eve's leaf");
        assert!(copy.entries().iter().filter(|(_, d, _)| *d != x).all(|(_, _, p)| p != b"proof"), "each leaf carries its own entry's words only");
        let mut copy = Account::load(Rule::Space(space), &g.save().unwrap()).unwrap();
        assert!(matches!(copy.apply_checked(&door, &L(vec![a.clone(), b.clone()], Some((x.clone(), b"other".to_vec())))).unwrap(), Applied::Skipped(_)));
        let _ = &mut bob2;
    }

    /// A SPACE's group, made by a DID's member: its own epoch secret, kept and loaded again by its space id; a member
    /// whose credential names another key is refused.
    #[test]
    fn a_did_makes_a_spaces_group_and_its_credential_must_be_its_own() {
        let a = member([0xA; 32], [0xAA; 32]);
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
        assert!(SpaceMember::new(&seed, other).is_err());
    }

    /// THE KEYS FOLLOW THE LIST OUT: Bob and Carol entered by themselves; Bob, no longer listed, out in ONE commit
    /// (`remove`) — Carol follows it, Bob's copy is left behind.
    #[test]
    fn the_unlisted_out_in_one_commit_the_rest_follow() {
        let (alice, bob, carol) = (member([0xA; 32], [0xAA; 32]), member([0xB; 32], [0xBB; 32]), member([0xC; 32], [0xCC; 32]));
        let space = [0x5A; 32];
        let mut g = alice.create_space(space).unwrap();
        let (mut b, cb) = bob.enter(space, &g.group_info().unwrap(), b"").unwrap();
        g.process(&cb).unwrap();
        let (mut c, cc) = carol.enter(space, &g.group_info().unwrap(), b"").unwrap();
        g.process(&cc).unwrap();
        b.process(&cc).unwrap();
        assert_eq!(c.epoch_secret().unwrap(), g.epoch_secret().unwrap(), "one group, one key");
        let bob_index = g.members().into_iter().find(|(_, w, _)| *w == writer([0xBB; 32])).unwrap().0;
        let e = g.epoch();
        let commit = g.remove(&[bob_index]).unwrap();
        assert_eq!(g.epoch(), e + 1, "one commit");
        c.process(&commit).unwrap();
        assert_eq!(c.epoch_secret().unwrap(), g.epoch_secret().unwrap(), "Carol follows");
        assert!(!g.members().iter().any(|(_, w, _)| *w == writer([0xBB; 32])), "Bob is out");
        b.process(&commit).unwrap();
        assert!(b.removed, "Bob's copy learns it is out, and reads nothing after");
    }

    /// UPDATE: a DID's member refreshes its keys (a device of the account was removed). The other member follows; a
    /// copy of the member from before the update (what the removed device holds) cannot follow the next change —
    /// control: without the update, that copy follows it.
    #[test]
    fn a_members_update_leaves_a_stale_copy_behind() {
        let (alice, bob) = (member([0xA; 32], [0xAA; 32]), member([0xB; 32], [0xBB; 32]));
        let (carol, dave) = (member([0xC; 32], [0xCC; 32]), member([0xD; 32], [0xDD; 32]));
        let space = [0x5A; 32];
        let mut a = alice.create_space(space).unwrap();
        let (mut b, cb) = bob.enter(space, &a.group_info().unwrap(), b"").unwrap();
        a.process(&cb).unwrap();
        let copy_of = |b: &mut Account| Account::load(Rule::Space(space), &b.save().unwrap()).unwrap();
        // Control: a copy of Bob, keys current — follows Carol entering.
        let mut copy = copy_of(&mut b);
        let (_, carol_in) = carol.enter(space, &a.group_info().unwrap(), b"").unwrap();
        a.process(&carol_in).unwrap();
        assert!(copy.process(&carol_in).is_ok(), "control: a copy follows while its keys are current");
        b.process(&carol_in).unwrap();
        // Bob refreshes his keys; Alice follows; the copy from before cannot follow what comes next.
        let mut copy = copy_of(&mut b);
        let up = b.update().unwrap();
        a.process(&up).unwrap();
        assert_eq!(a.epoch_secret().unwrap(), b.epoch_secret().unwrap(), "Alice follows Bob's update");
        let (_, dave_in) = dave.enter(space, &a.group_info().unwrap(), b"").unwrap();
        a.process(&dave_in).unwrap();
        b.process(&dave_in).unwrap();
        assert!(copy.process(&up).is_err(), "the member's own update is not the copy's to follow");
        assert!(copy.process(&dave_in).is_err(), "and what follows it is out of the copy's reach");
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
        let removal = a.remove(&[c.my_index()]).unwrap();
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
    use wasm_bindgen::JsCast;

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

        /// A commit APPLIED ONLY IF THE LIST ALLOWS IT (`Account::apply_checked`, `entry_ok`): `listed` — JSON `[did…]`.
        /// `{ applied: true, external, committer, added, removed }`, or `{ applied: false, why }` (skipped: the group as
        /// it was). Throws where MLS would not apply it at all.
        /// THE LIST (`gov`'s, through the core): `listed(did)`, `admits(did, aadHex)`, `mayTakeOut(did)` — each a function.
        pub fn apply_checked(&mut self, commit: &[u8], list: &js_sys::Object) -> Result<JsValue, JsValue> {
            struct Js<'a>(&'a js_sys::Object);
            impl Js<'_> {
                fn call(&self, f: &str, args: &[JsValue]) -> bool {
                    let Ok(f) = js_sys::Reflect::get(self.0, &f.into()).and_then(|f| f.dyn_into::<js_sys::Function>()) else { return false };
                    let r = match args { [a] => f.call1(self.0, a), [a, b] => f.call2(self.0, a, b), _ => return false };
                    r.ok().and_then(|v| v.as_bool()).unwrap_or(false)
                }
            }
            impl List for Js<'_> {
                fn listed(&self, did: &str) -> bool {
                    self.call("listed", &[did.into()])
                }
                fn admits(&self, did: &str, proof: &[u8]) -> bool {
                    self.call("admits", &[did.into(), proof.iter().map(|x| format!("{x:02x}")).collect::<String>().into()])
                }
                fn may_take_out(&self, did: &str) -> bool {
                    self.call("mayTakeOut", &[did.into()])
                }
            }
            let out = match self.member()?.apply_checked(commit, &Js(list)).map_err(err)? {
                Applied::Applied(c) => format!(
                    "{{\"applied\":true,\"external\":{},\"committer\":{:?},\"added\":{:?},\"removed\":{:?}}}",
                    c.external, c.committer, c.added, c.removed
                ),
                Applied::Skipped(why) => format!("{{\"applied\":false,\"why\":{why:?}}}"),
            };
            js_sys::JSON::parse(&out)
        }

        /// WHAT A GROUP INFO SAYS (`info_facts`): `{ epoch, signer, members: [[index, did]…] }`.
        pub fn info_facts(group_info: &[u8]) -> Result<JsValue, JsValue> {
            let f = super::info_facts(group_info).map_err(err)?;
            let members: Vec<String> = f.members.iter().map(|(i, _, d)| format!("[{i},{d:?}]")).collect();
            js_sys::JSON::parse(&format!("{{\"epoch\":{},\"signer\":{:?},\"members\":[{}]}}", f.epoch, f.signer, members.join(",")))
        }

        /// This group's info now (what a listed device enters from), its tree in it.
        pub fn group_info(&mut self) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(&self.member()?.group_info().map_err(err)?[..]))
        }

        /// UPDATE this member's own keys: the commit to publish (a removed device's copy follows nothing after it).
        pub fn update(&mut self) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(&self.member()?.update().map_err(err)?[..]))
        }

        /// TAKE OUT the members at `indexes` (`Account::remove`): the commit to publish.
        pub fn remove(&mut self, indexes: &[u32]) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(&self.member()?.remove(indexes).map_err(err)?[..]))
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

    /// A DID's member for spaces (the page's side of `SpaceMember`): from the identity's seed and credential.
    #[wasm_bindgen(js_name = SpaceMember)]
    pub struct JsSpaceMember(super::SpaceMember);

    #[wasm_bindgen(js_class = SpaceMember)]
    impl JsSpaceMember {
        #[wasm_bindgen(constructor)]
        pub fn new(seed: &[u8], credential: &[u8]) -> Result<JsSpaceMember, JsValue> {
            Ok(JsSpaceMember(super::SpaceMember::new(&did32(seed)?, credential.to_vec()).map_err(err)?))
        }

        /// A space's group, made by this DID: a new `Mls`.
        pub fn create_space(&self, space: &[u8]) -> Result<Mls, JsValue> {
            Ok(Mls(Some(self.0.create_space(did32(space)?).map_err(err)?), [0; 32]))
        }

        /// ENTER a space's group by itself (`SpaceMember::enter`): `[Mls, commit]` — the commit published at the info's
        /// epoch (its door), the member kept once it is taken.
        pub fn enter(&self, space: &[u8], group_info: &[u8], aad: &[u8]) -> Result<js_sys::Array, JsValue> {
            let (a, c) = self.0.enter(did32(space)?, group_info, aad).map_err(err)?;
            Ok([JsValue::from(Mls(Some(a), [0; 32])), js_sys::Uint8Array::from(&c[..]).into()].into_iter().collect())
        }
    }

    impl Default for Mls {
        fn default() -> Self {
            Self::new()
        }
    }
}
