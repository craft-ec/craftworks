//! MEMBERS AND KEYS ON BEEKEM (docs/proposals/members-and-keys-beekem.md). One group per space (and per chat, audience,
//! circle, level group): its KEYS are a BeeKEM tree; who should hold them is the list (the acts, `gov`). This crate is
//! the one door to BeeKEM, the page's core and the identity delegate alike:
//!
//! - ONE MEMBER PER ACCOUNT: its signing key and first leaf key derived from the account's data key, the same on every
//!   device of the account (`Account::of`).
//! - OUR CHECK BEFORE BEEKEM (BeeKEM merges what it is given — authorization is the layer above): an operation is
//!   merged only when its signer is a member the list names, and an add or remove names, as its authorization, the hash
//!   of an act that allows exactly that (`Check`).
//! - OPERATIONS AS BYTES, merged in causal order (one whose predecessors are not held yet: `Merged::Waiting`).
//! - CONTENT through BeeKEM's content API: sealed under a NAMED update's secret, opened by deriving that update's —
//!   never the tree's live root (which can differ by arrival order).
use beekem::{
    cgka::Cgka,
    encrypted::EncryptedContent,
    error::CgkaError,
    id::{MemberId, TreeId},
    keys::ShareKeyMap,
    operation::{CgkaAuthorization, CgkaOperation},
};
use future_form::Local;
use keyhive_crypto::{share_key::{ShareKey, ShareSecretKey}, signed::Signed, signer::memory::MemorySigner};
use std::sync::Arc;

/// A space's TREE ID: a key derived from the space's id (BeeKEM names a tree by a verifying key).
pub fn tree_id(space: &[u8; 32]) -> TreeId {
    TreeId(ed25519_dalek::SigningKey::from_bytes(&blake3::derive_key("craftworks 2026-10-17 beekem tree", space)).verifying_key())
}

/// AN ACCOUNT's one member (open question 1, decided: one slot per account): its signer (the account's space member
/// key — `craftworks_identity::space_member_seed`), its member id (that key's public half), its first leaf secret.
#[derive(Clone)]
pub struct Account {
    pub signer: MemorySigner,
    pub id: MemberId,
    pub leaf: ShareSecretKey,
}

impl Account {
    /// From the account's space member seed (every device of the account derives the same).
    pub fn of(member_seed: &[u8; 32]) -> Account {
        let key = ed25519_dalek::SigningKey::from_bytes(member_seed);
        let leaf = ShareSecretKey::from(x25519_dalek::StaticSecret::from(blake3::derive_key("craftworks 2026-10-17 beekem leaf", member_seed)));
        Account { id: MemberId(key.verifying_key()), signer: MemorySigner::from(key), leaf }
    }
    pub fn leaf_key(&self) -> ShareKey {
        self.leaf.share_key()
    }
}

/// OUR CHECK, the list's: who a member id is (its account), whether the list names it, and whether an act allows an
/// add or a remove of a member.
pub trait Check {
    /// The account (DID) a member id belongs to — from its published credential; `None`: unknown.
    fn account_of(&self, member: &[u8; 32]) -> Option<String>;
    /// Whether the list names this account now (an operation's signer must be listed).
    fn listed(&self, did: &str) -> bool;
    /// Whether the act whose hash is `act` allows ADDING (`add`) or REMOVING the member `target`.
    fn allows(&self, act: &[u8; 32], add: bool, target: &[u8; 32]) -> bool;
}

/// What merging an operation did.
#[derive(Debug, PartialEq, Eq)]
pub enum Merged {
    /// Taken (or held already).
    Applied,
    /// Its predecessors are not held yet: kept by the caller and offered again later (causal order).
    Waiting,
    /// Refused by our check (why) — never merged.
    Refused(String),
}

/// ONE GROUP: the BeeKEM tree as this account's device holds it.
pub struct Group {
    cgka: Cgka,
    account: Account,
}

/// BeeKEM's operations finish at once with an in-memory signer: poll once; a future that would wait is a fault here.
fn now<T>(f: impl core::future::Future<Output = T>) -> T {
    use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
    fn noop(_: *const ()) {}
    fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(core::ptr::null(), &VTABLE)
    }
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    let waker = unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) };
    let mut f = core::pin::pin!(f);
    match f.as_mut().poll(&mut Context::from_waker(&waker)) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("a BeeKEM operation waited: none should (in-memory signer)"),
    }
}

fn e(x: CgkaError) -> String {
    format!("{x:?}")
}

impl Group {
    /// A NEW group for `space`, made by `account` (placed as its first member).
    pub fn create(space: &[u8; 32], account: Account) -> Result<(Group, Vec<u8>), String> {
        let mut g = Group::empty(space, account);
        let op = g.add_member(&g.account.id.0.to_bytes(), &g.account.leaf_key(), &[0; 32])?.ok_or("the maker could not place itself")?;
        Ok((g, op))
    }
    /// An empty replica of `space`'s group for `account` (it learns the tree from the group's operations).
    pub fn empty(space: &[u8; 32], account: Account) -> Group {
        let mut sks = ShareKeyMap::new();
        sks.insert(account.leaf_key(), account.leaf);
        Group { cgka: Cgka::new(tree_id(space), account.id, sks), account }
    }
    pub fn save(&self) -> Result<Vec<u8>, String> {
        bincode::serialize(&self.cgka).map_err(|x| x.to_string())
    }
    pub fn load(bytes: &[u8], account: Account) -> Result<Group, String> {
        Ok(Group { cgka: bincode::deserialize(bytes).map_err(|x| x.to_string())?, account })
    }
    /// Whether this account is in the tree (its view).
    pub fn placed(&self) -> bool {
        self.cgka.owner_leaf_key().is_some()
    }
    /// The member ids the tree places (its view).
    pub fn members(&mut self) -> Vec<[u8; 32]> {
        self.cgka.member_ids().map(|it| it.map(|m| m.0.to_bytes()).collect()).unwrap_or_default()
    }

    /// MERGE another's operation — through OUR CHECK first.
    pub fn merge(&mut self, op: &[u8], check: &impl Check) -> Merged {
        let Ok(op) = bincode::deserialize::<Signed<CgkaOperation>>(op) else { return Merged::Refused("not an operation".into()) };
        if let Err(why) = admitted(&op, check) {
            return Merged::Refused(why);
        }
        match self.cgka.merge_concurrent_operation(Arc::new(op)) {
            Ok(_) => Merged::Applied,
            Err(CgkaError::OutOfOrderOperation) => Merged::Waiting,
            Err(x) => Merged::Refused(e(x)),
        }
    }

    /// ADD a member (its id, its first leaf key), authorized by the act `act`: the operation's bytes, or `None` if the
    /// tree places it already.
    pub fn add_member(&mut self, member: &[u8; 32], leaf: &ShareKey, act: &[u8; 32]) -> Result<Option<Vec<u8>>, String> {
        let id = MemberId(ed25519_dalek::VerifyingKey::from_bytes(member).map_err(|x| x.to_string())?);
        let op = now(self.cgka.add::<Local, _>(id, *leaf, CgkaAuthorization::Delegation(*act), &self.account.signer)).map_err(e)?;
        op.map(|o| bincode::serialize(&o).map_err(|x| x.to_string())).transpose()
    }
    /// REMOVE a member, authorized by the act `act`: the operation's bytes, or `None` if not placed.
    pub fn remove_member(&mut self, member: &[u8; 32], act: &[u8; 32]) -> Result<Option<Vec<u8>>, String> {
        let id = MemberId(ed25519_dalek::VerifyingKey::from_bytes(member).map_err(|x| x.to_string())?);
        let op = now(self.cgka.remove::<Local, _>(id, CgkaAuthorization::Revocation(*act), &self.account.signer)).map_err(e)?;
        op.map(|o| bincode::serialize(&o).map_err(|x| x.to_string())).transpose()
    }
    /// ROTATE this account's leaf (after an add or a remove; when the root is unresolved): the operation, and the new
    /// leaf secret — which the ACCOUNT hands to its other devices.
    pub fn rotate<R: rand::CryptoRng + rand::RngCore>(&mut self, rng: &mut R) -> Result<(Vec<u8>, ShareSecretKey), String> {
        let sk = ShareSecretKey::generate(rng);
        let (_, op, _) = now(self.cgka.update::<Local, _, R>(sk.share_key(), sk, &self.account.signer, rng)).map_err(e)?;
        Ok((bincode::serialize(&op).map_err(|x| x.to_string())?, sk))
    }
    /// A leaf secret this account's other device made (handed through the account).
    pub fn take_leaf(&mut self, sk: ShareSecretKey) {
        self.cgka.owner_sks.insert(sk.share_key(), sk);
    }

    /// SEAL `plaintext` (content `reference`): the sealed bytes, and an operation to send when a key change was needed
    /// first (concurrent heads).
    pub fn seal<R: rand::CryptoRng + rand::RngCore>(&mut self, reference: [u8; 32], plaintext: &[u8], rng: &mut R) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
        let (secret, op, _) = now(self.cgka.new_app_secret_for::<Local, _, [u8; 32], R>(&reference, plaintext, &Vec::new(), &self.account.signer, rng)).map_err(e)?;
        let sealed: EncryptedContent<Vec<u8>, [u8; 32]> = secret.try_encrypt(plaintext).map_err(|x| x.to_string())?;
        Ok((bincode::serialize(&sealed).map_err(|x| x.to_string())?, op.map(|o| bincode::serialize(&o)).transpose().map_err(|x| x.to_string())?))
    }
    /// OPEN sealed bytes: the plaintext — by the named update's secret.
    pub fn open(&mut self, sealed: &[u8]) -> Result<Vec<u8>, String> {
        let sealed: EncryptedContent<Vec<u8>, [u8; 32]> = bincode::deserialize(sealed).map_err(|x| x.to_string())?;
        let key = self.cgka.decryption_key_for(&sealed).map_err(e)?;
        sealed.try_decrypt(key).map_err(|x| x.to_string())
    }
}

/// OUR CHECK (the list's): the signer listed; an add or a remove authorized by an act that allows exactly that.
fn admitted(op: &Signed<CgkaOperation>, check: &impl Check) -> Result<(), String> {
    let signer = op.issuer().to_bytes();
    let did = check.account_of(&signer).ok_or("signed by a member id no listed account publishes")?;
    if !check.listed(&did) {
        return Err(format!("signed by {did}, not on the list"));
    }
    match &op.payload {
        CgkaOperation::Add { added_id, authorization, .. } => match authorization {
            CgkaAuthorization::Delegation(act) if check.allows(act, true, &added_id.0.to_bytes()) => Ok(()),
            _ => Err("an add no act allows".into()),
        },
        CgkaOperation::Remove { id, authorization, .. } => match authorization {
            CgkaAuthorization::Revocation(act) if check.allows(act, false, &id.0.to_bytes()) => Ok(()),
            _ => Err("a removal no act allows".into()),
        },
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests;
