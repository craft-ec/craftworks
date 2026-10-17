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
    seed: [u8; 32],
    pub signer: MemorySigner,
    pub id: MemberId,
    pub leaf: ShareSecretKey,
}

impl Account {
    /// From the account's space member seed (every device of the account derives the same).
    pub fn of(member_seed: &[u8; 32]) -> Account {
        let key = ed25519_dalek::SigningKey::from_bytes(member_seed);
        let leaf = ShareSecretKey::from(x25519_dalek::StaticSecret::from(blake3::derive_key("craftworks 2026-10-17 beekem leaf", member_seed)));
        Account { seed: *member_seed, id: MemberId(key.verifying_key()), signer: MemorySigner::from(key), leaf }
    }
    /// THE LEAF SECRET of a key change made after `heads` (K5/K6): derived from the account's seed and what it follows,
    /// never drawn — so every device of the account, and its node, derives the leaf secret of any change the account
    /// made, and nothing is handed between them.
    fn leaf_after(&self, heads: &[keyhive_crypto::digest::Digest<Signed<CgkaOperation>>]) -> ShareSecretKey {
        let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-17 beekem rotation");
        h.update(&self.seed);
        for d in heads {
            h.update(d.as_slice());
        }
        ShareSecretKey::from(x25519_dalek::StaticSecret::from(*h.finalize().as_bytes()))
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
        // THIS ACCOUNT's own key change, made on another device or its node: its leaf secret derived here (K6).
        if let CgkaOperation::Update { id, new_path, predecessors, .. } = &op.payload {
            if *id == self.account.id {
                let mut heads = predecessors.clone();
                heads.sort();
                let sk = self.account.leaf_after(&heads);
                if matches!(&new_path.leaf_pk, beekem::keys::NodeKey::ShareKey(pk) if *pk == sk.share_key()) {
                    self.cgka.owner_sks.insert(sk.share_key(), sk);
                }
            }
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
        let sk = self.account.leaf_after(&self.cgka.current_heads());
        let (_, op, _) = now(self.cgka.update::<Local, _, R>(sk.share_key(), sk, &self.account.signer, rng)).map_err(e)?;
        Ok((bincode::serialize(&op).map_err(|x| x.to_string())?, sk))
    }
    /// THE GENERATION TO WRITE WITH (K4: a generation is the storage's epoch): `(number, secret, operation)` — a key
    /// change made first (its operation to send) when the group has concurrent heads or no root secret here.
    pub fn current<R: rand::CryptoRng + rand::RngCore>(&mut self, rng: &mut R) -> Result<(u64, [u8; 32], Option<Vec<u8>>), String> {
        if let Some((h, k)) = self.cgka.current_generation() {
            return Ok((self.number(&h), k.0.to_bytes(), None));
        }
        let (op, _) = self.rotate(rng)?;
        let (h, k) = self.cgka.current_generation().ok_or("no generation after a key change")?;
        Ok((self.number(&h), k.0.to_bytes(), Some(op)))
    }
    /// EVERY GENERATION this member reaches (reading: history included): `(number, secret)`.
    pub fn generations(&mut self) -> Vec<(u64, [u8; 32])> {
        self.cgka.reachable_generations().into_iter().map(|(h, k)| (self.number(&h), k.0.to_bytes())).collect()
    }
    fn number(&self, h: &keyhive_crypto::digest::Digest<Signed<CgkaOperation>>) -> u64 {
        generation_number(self.cgka.ops_graph_depth(h), h)
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

/// A generation's NUMBER (the storage's epoch, whose NEWEST — the highest — is written with): its update's DEPTH in the
/// operations (a later generation is always deeper), and 3 bytes of its hash below it (concurrent ones at one depth).
fn generation_number(depth: u64, h: &keyhive_crypto::digest::Digest<Signed<CgkaOperation>>) -> u64 {
    let low = u64::from(h.as_slice()[0]) << 16 | u64::from(h.as_slice()[1]) << 8 | u64::from(h.as_slice()[2]);
    depth << 24 | low
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

/// THE PAGE's bindings (the `page` feature: this crate as its own package, as MLS's). Bytes in and out; our check as
/// three JS functions (`accountOf(memberHex)` → DID or null, `listed(did)`, `allows(actHex, add, targetHex)`).
#[cfg(all(target_arch = "wasm32", feature = "page"))]
mod page {
    use super::*;
    use wasm_bindgen::prelude::*;

    fn err(x: impl core::fmt::Display) -> JsValue {
        JsValue::from_str(&x.to_string())
    }
    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
    fn b32(b: &[u8]) -> Result<[u8; 32], JsValue> {
        b.try_into().map_err(|_| err("32 bytes expected"))
    }
    fn bytes(v: Option<Vec<u8>>) -> JsValue {
        v.map(|b| js_sys::Uint8Array::from(&b[..]).into()).unwrap_or(JsValue::NULL)
    }
    struct JsCheck(js_sys::Object);
    impl JsCheck {
        fn call(&self, f: &str, args: &[JsValue]) -> JsValue {
            let Ok(f) = js_sys::Reflect::get(&self.0, &f.into()).and_then(|f| f.dyn_into::<js_sys::Function>()) else { return JsValue::NULL };
            let r = match args {
                [a] => f.call1(&self.0, a),
                [a, b, c] => f.call3(&self.0, a, b, c),
                _ => return JsValue::NULL,
            };
            r.unwrap_or(JsValue::NULL)
        }
    }
    impl Check for JsCheck {
        fn account_of(&self, member: &[u8; 32]) -> Option<String> {
            self.call("accountOf", &[hex(member).into()]).as_string()
        }
        fn listed(&self, did: &str) -> bool {
            self.call("listed", &[did.into()]).as_bool().unwrap_or(false)
        }
        fn allows(&self, act: &[u8; 32], add: bool, target: &[u8; 32]) -> bool {
            self.call("allows", &[hex(act).into(), add.into(), hex(target).into()]).as_bool().unwrap_or(false)
        }
    }

    /// An account's member id (hex) and first leaf key, from its space member seed.
    #[wasm_bindgen]
    pub fn beekem_member(member_seed: &[u8]) -> Result<js_sys::Array, JsValue> {
        let a = Account::of(&b32(member_seed)?);
        Ok([JsValue::from(hex(&a.id.0.to_bytes())), js_sys::Uint8Array::from(&a.leaf_key().to_bytes()[..]).into()].into_iter().collect())
    }

    #[wasm_bindgen]
    pub struct BeeGroup(Group);

    #[wasm_bindgen]
    impl BeeGroup {
        /// A new group for `space` made by this account: `[group, its first operation]`.
        pub fn create(space: &[u8], member_seed: &[u8]) -> Result<js_sys::Array, JsValue> {
            let (g, op) = Group::create(&b32(space)?, Account::of(&b32(member_seed)?)).map_err(err)?;
            Ok([JsValue::from(BeeGroup(g)), js_sys::Uint8Array::from(&op[..]).into()].into_iter().collect())
        }
        /// An empty replica of `space`'s group for this account.
        pub fn empty(space: &[u8], member_seed: &[u8]) -> Result<BeeGroup, JsValue> {
            Ok(BeeGroup(Group::empty(&b32(space)?, Account::of(&b32(member_seed)?))))
        }
        pub fn load(bytes: &[u8], member_seed: &[u8]) -> Result<BeeGroup, JsValue> {
            Ok(BeeGroup(Group::load(bytes, Account::of(&b32(member_seed)?)).map_err(err)?))
        }
        pub fn save(&self) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(&self.0.save().map_err(err)?[..]))
        }
        pub fn placed(&self) -> bool {
            self.0.placed()
        }
        /// The member ids placed (hex).
        pub fn members(&mut self) -> js_sys::Array {
            self.0.members().iter().map(|m| JsValue::from(hex(m))).collect()
        }
        /// MERGE an operation through our check: "applied", "waiting", or "refused: why".
        pub fn merge(&mut self, op: &[u8], check: js_sys::Object) -> String {
            match self.0.merge(op, &JsCheck(check)) {
                Merged::Applied => "applied".into(),
                Merged::Waiting => "waiting".into(),
                Merged::Refused(why) => format!("refused: {why}"),
            }
        }
        /// ADD a member (id, first leaf key), authorized by an act's hash: the operation, or null (placed already).
        pub fn add(&mut self, member: &[u8], leaf: &[u8], act: &[u8]) -> Result<JsValue, JsValue> {
            let leaf = keyhive_crypto::share_key::ShareKey::from(x25519_dalek::PublicKey::from(b32(leaf)?));
            Ok(bytes(self.0.add_member(&b32(member)?, &leaf, &b32(act)?).map_err(err)?))
        }
        /// REMOVE a member, authorized by an act's hash: the operation, or null (not placed).
        pub fn remove(&mut self, member: &[u8], act: &[u8]) -> Result<JsValue, JsValue> {
            Ok(bytes(self.0.remove_member(&b32(member)?, &b32(act)?).map_err(err)?))
        }
        /// ROTATE this account's leaf: `[operation, the new leaf secret]` (the account hands the secret to its devices).
        pub fn rotate(&mut self) -> Result<js_sys::Array, JsValue> {
            let (op, sk) = self.0.rotate(&mut rand::rngs::OsRng).map_err(err)?;
            Ok([js_sys::Uint8Array::from(&op[..]), js_sys::Uint8Array::from(&sk.to_bytes()[..])].into_iter().map(JsValue::from).collect())
        }
        /// A leaf secret another device of this account made.
        pub fn take_leaf(&mut self, secret: &[u8]) -> Result<(), JsValue> {
            self.0.take_leaf(ShareSecretKey::from(x25519_dalek::StaticSecret::from(b32(secret)?)));
            Ok(())
        }
        /// SEAL: `[sealed, operation or null]` (a key change first when the group has concurrent heads).
        pub fn seal(&mut self, reference: &[u8], plaintext: &[u8]) -> Result<js_sys::Array, JsValue> {
            let (sealed, op) = self.0.seal(b32(reference)?, plaintext, &mut rand::rngs::OsRng).map_err(err)?;
            Ok([JsValue::from(js_sys::Uint8Array::from(&sealed[..])), bytes(op)].into_iter().collect())
        }
        /// THE GENERATION TO WRITE WITH: `[number (decimal string), secret, operation or null]`.
        pub fn current(&mut self) -> Result<js_sys::Array, JsValue> {
            let (n, k, op) = self.0.current(&mut rand::rngs::OsRng).map_err(err)?;
            Ok([JsValue::from(n.to_string()), js_sys::Uint8Array::from(&k[..]).into(), bytes(op)].into_iter().collect())
        }
        /// EVERY GENERATION reached: `[[number (decimal string), secret]…]`.
        pub fn generations(&mut self) -> js_sys::Array {
            self.0.generations().into_iter().map(|(n, k)| -> JsValue { [JsValue::from(n.to_string()), js_sys::Uint8Array::from(&k[..]).into()].into_iter().collect::<js_sys::Array>().into() }).collect()
        }
        /// OPEN sealed bytes.
        pub fn open(&mut self, sealed: &[u8]) -> Result<js_sys::Uint8Array, JsValue> {
            Ok(js_sys::Uint8Array::from(&self.0.open(sealed).map_err(err)?[..]))
        }
    }
}
