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
use keyhive_crypto::{digest::Digest, share_key::{ShareKey, ShareSecretKey}, signed::Signed, signer::memory::MemorySigner};
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
    /// ITS LEAF, SIGNED by this member (K8.5: a device's leaf in the account's channel, `d/<member>`) — what an add
    /// encrypts to, so no other holder of the channel's key can put a leaf of its own in a device's place.
    pub fn signed_leaf(&self) -> ([u8; 32], [u8; 64]) {
        use ed25519_dalek::Signer;
        let leaf = self.leaf_key().to_bytes();
        let sig = ed25519_dalek::SigningKey::from_bytes(&self.seed).sign(&[LEAF_SIGNED, &leaf[..]].concat());
        (leaf, sig.to_bytes())
    }
}
const LEAF_SIGNED: &[u8] = b"craftworks 2026-10-26 account leaf";
/// A LEAF taken only when its member signed it (`Account::signed_leaf`).
pub fn leaf_checks(member: &[u8; 32], leaf: &[u8; 32], sig: &[u8; 64]) -> bool {
    use ed25519_dalek::Verifier;
    let Ok(k) = ed25519_dalek::VerifyingKey::from_bytes(member) else { return false };
    k.verify(&[LEAF_SIGNED, &leaf[..]].concat(), &ed25519_dalek::Signature::from_bytes(sig)).is_ok()
}

/// OUR CHECK, the list's: who a member id is (its account), whether the list names it now, and whether an act allows
/// an add or a remove of a member. ADMISSION judges FIXED FACTS only; WHO RECEIVES is decided by the one who seals
/// (`Group::writable` — FOUNDATION "WHO RECEIVES IS DECIDED BY THE ONE WHO SEALS").
pub trait Check {
    /// The account (DID) a member id belongs to — from its published credential; `None`: unknown.
    fn account_of(&self, member: &[u8; 32]) -> Option<String>;
    /// Whether the list names this account NOW.
    fn listed(&self, did: &str) -> bool;
    /// Whether the act whose hash is `act` allows ADDING (`add`) or REMOVING the member `target` — a fixed fact (the acts
    /// only grow): an add stays valid after its member leaves.
    fn allows(&self, act: &[u8; 32], add: bool, target: &[u8; 32]) -> bool;
    /// Whether an account was EVER let in (a fixed fact): an operation's SIGNER counts by it, never by who is listed now
    /// (10-28 re-review: judged by "now", a member kicked made every operation it ever wrote unauthorized on later fresh
    /// merges — the removal and everything after waited on them, and the tree forked).
    fn ever_in(&self, did: &str) -> bool {
        self.listed(did)
    }
    /// Whether a PLACED member is IN NOW (a space: its account listed, its member not one its account replaced; the
    /// account's tree: a current device): a tree placing one who is not is never written with (`Group::writable`).
    fn in_now(&self, member: &[u8; 32]) -> bool {
        !self.superseded(member) && self.account_of(member).is_some_and(|d| self.listed(&d))
    }
    /// Whether this check can JUDGE at all: the account's tree with NO current device — its key log moved and the devices
    /// that stay not credentialed on the new key yet (`removeDevice` puts the log, then the devices) — judges nothing:
    /// every operation WAITS (none refused for a window; page and node alike).
    fn whole(&self) -> bool {
        true
    }
    /// THE ONE ACCOUNT that may make the space's tree: its owner (a space with none: the lowest listed DID). `None`:
    /// not known yet — no root is taken.
    fn creator(&self) -> Option<String>;
    /// The LEAF KEY a member PUBLISHED (its card's, a device's signed leaf): an add is taken only with it (10-27: an add
    /// carried any leaf — a member could plant a leaf it held in another's place and read on after its own removal).
    /// `None`: not known yet — an add of it waits.
    fn leaf_of(&self, member: &[u8; 32]) -> Option<[u8; 32]>;
    /// A member id its account REPLACED (K8.7: its data seed rotated — a new member on its card): still the author of
    /// what it wrote, and taken out with no act.
    fn superseded(&self, _member: &[u8; 32]) -> bool {
        false
    }
}

/// A card's member the tree should PLACE: listed, not placed. One the tree should TAKE OUT: not listed, and an act put
/// them out (none: left — nothing decided without the act). (`reconcile` and `due`, the one pair of conditions.)
fn wants_in(placed: &[[u8; 32]], id: &[u8; 32], did: &str, listed: &dyn Fn(&str) -> bool) -> bool {
    !placed.contains(id) && listed(did)
}
fn wants_out(did: &str, listed: &dyn Fn(&str) -> bool, outs: &std::collections::BTreeMap<String, [u8; 32]>) -> bool {
    !listed(did) && outs.contains_key(did)
}

/// OUR CHECK FROM THE LIST (page and node, one implementation): whose each member id is (`by_member`, from their cards),
/// who is listed (and the owner), and which act lets whom in or out (`acts`: sha256 of the act's id → (in?, DID)). A
/// refusal is kept with what it saw (`misses`). 10-19: the page built this in JavaScript, the node as its own `List`.
#[derive(Debug, Default)]
pub struct ListCheck {
    pub by_member: std::collections::BTreeMap<[u8; 32], String>,
    /// Each member's published leaf key (its card's).
    pub leaves: std::collections::BTreeMap<[u8; 32], [u8; 32]>,
    /// Member ids their accounts replaced (K8.7) — in `by_member` too: the authors of what they wrote.
    pub superseded: std::collections::BTreeSet<[u8; 32]>,
    pub listed: Vec<String>,
    pub owner: String,
    pub acts: std::collections::BTreeMap<[u8; 32], (bool, String)>,
    pub misses: core::cell::RefCell<Vec<String>>,
}
impl Check for ListCheck {
    fn account_of(&self, member: &[u8; 32]) -> Option<String> {
        self.by_member.get(member).cloned()
    }
    /// Listed NOW: the roster (the governance's — its owner on it as any member; the space's founder is no exception:
    /// put out after handing the space on, they are out — 10-28 re-review).
    fn listed(&self, did: &str) -> bool {
        self.listed.iter().any(|d| d == did)
    }
    /// EVER let in: by an act, or the founder (its tree's first placement, by no act).
    fn ever_in(&self, did: &str) -> bool {
        did == self.owner || self.listed(did) || self.acts.values().any(|(i, d)| *i && d == did)
    }
    fn allows(&self, act: &[u8; 32], add: bool, target: &[u8; 32]) -> bool {
        let t = self.by_member.get(target);
        let ok = if *act == [0; 32] { add && t.is_some_and(|t| *t == self.owner) } else { self.acts.get(act).is_some_and(|(i, d)| *i == add && Some(d) == t) };
        if !ok {
            let hex8 = |b: &[u8]| b.iter().take(4).map(|x| format!("{x:02x}")).collect::<String>();
            self.misses.borrow_mut().push(format!("act {} {} ({} acts), {} {}", hex8(act), if self.acts.contains_key(act) { "known" } else { "unknown" }, self.acts.len(), if add { "add" } else { "remove" }, t.map(|d| d.get(12..20).unwrap_or(d).to_string()).unwrap_or_else(|| format!("unmapped {}", hex8(target)))));
        }
        ok
    }
    fn creator(&self) -> Option<String> {
        if !self.owner.is_empty() {
            return Some(self.owner.clone());
        }
        self.listed.iter().min().cloned()
    }
    fn leaf_of(&self, member: &[u8; 32]) -> Option<[u8; 32]> {
        self.leaves.get(member).copied()
    }
    fn superseded(&self, member: &[u8; 32]) -> bool {
        self.superseded.contains(member)
    }
}

/// THE ACCOUNT's CHECK (K8.5, FOUNDATION "THE ACCOUNT's GROUP ON BEEKEM"): the account's own tree, whose members are
/// its DEVICES (each device's member: the group key its credential names), and WHO IS IN IS THE DEVICES LIST —
/// `current`: the members its devices contract credentials under its key log's current key (`account::device_members`).
/// An operation's AUTHOR, and an add's target, count when the device was EVER the account's (`ever`: credentialed under
/// any key its key log has had — a fixed fact: a device removed since still made the root and the adds before; fx/fy
/// 10-26, judged by the current list alone every earlier operation went unauthorized and the removal never landed). A
/// removal: of one no longer current (no act: the devices list is the word). IN NOW (`Group::writable`): a current
/// device. The ROOT is the lowest member ever the account's among the authors of the root operations in the account's
/// channel (`writers`) — never a second tree from two makers.
#[derive(Debug, Default)]
pub struct AccountCheck {
    pub current: Vec<[u8; 32]>,
    pub ever: Vec<[u8; 32]>,
    pub writers: Vec<[u8; 32]>,
    /// Each device's SIGNED leaf (`d/<member>` in the account's channel, `leaf_checks`): an add is taken only with it.
    pub leaves: std::collections::BTreeMap<[u8; 32], [u8; 32]>,
}
fn hex32(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
impl Check for AccountCheck {
    fn account_of(&self, member: &[u8; 32]) -> Option<String> {
        Some(hex32(member))
    }
    fn listed(&self, device: &str) -> bool {
        self.current.iter().chain(&self.ever).any(|c| hex32(c) == device)
    }
    fn allows(&self, _act: &[u8; 32], add: bool, target: &[u8; 32]) -> bool {
        if add { self.listed(&hex32(target)) } else { !self.current.contains(target) }
    }
    fn in_now(&self, member: &[u8; 32]) -> bool {
        self.current.contains(member)
    }
    fn whole(&self) -> bool {
        !self.current.is_empty()
    }
    fn creator(&self) -> Option<String> {
        self.current.iter().chain(&self.ever).filter(|c| self.writers.contains(c)).min().map(hex32)
    }
    fn leaf_of(&self, member: &[u8; 32]) -> Option<[u8; 32]> {
        self.leaves.get(member).copied()
    }
}

/// AN OPERATION described: its hash, its kind (`add`, `remove`, `update`), the hashes it builds on, and its AUTHOR (the
/// member that signed it) — the account tree's root maker is read from it (K8.5). `None`: bytes that are not one.
pub fn op_info(op: &[u8]) -> Option<([u8; 32], &'static str, Vec<[u8; 32]>, [u8; 32])> {
    let op = bincode::deserialize::<Signed<CgkaOperation>>(op).ok()?;
    let h = keyhive_crypto::digest::Digest::hash(&op);
    let kind = match &op.payload {
        CgkaOperation::Add { .. } => "add",
        CgkaOperation::Remove { .. } => "remove",
        CgkaOperation::Update { .. } => "update",
    };
    let preds = op.payload.predecessors().iter().filter_map(|d| <[u8; 32]>::try_from(d.as_slice()).ok()).collect();
    Some((<[u8; 32]>::try_from(h.as_slice()).ok()?, kind, preds, op.issuer.to_bytes()))
}

/// What merging every operation did (`merge_all`).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MergedAll {
    pub applied: usize,
    pub waiting: usize,
    pub refused: Vec<String>,
    /// WHY each still waits (`waiting_why`): what it waits on, said — a count alone named nothing (10-28: 3 of 6 waited on
    /// both devices, for good, and nothing said which or on what).
    pub waits: Vec<String>,
    /// The ACTS named by adds that wait on their member's CARD (not read here): whose card to read NOW — the one who
    /// added them read it (10-29, fz: a node asked a new member's card once before it was written, then waited 10 min
    /// to ask again, and its tree never placed them).
    pub waiting_cards: Vec<[u8; 32]>,
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
    /// WHOSE each member is, as our check said when its add was TAKEN (10-28 review: a kicked member rewrote its card
    /// without its member — no card named it, nothing could take it out, and the tree was never clean again). Saved
    /// BESIDE the tree (`save_known`: the tree's own save stays what every build reads — 10-28 review: a new format broke
    /// a device of the same account still on the build before, and any rollback); a card that names it still decides.
    known: std::collections::BTreeMap<[u8; 32], String>,
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
        Group::create_by(space, account, &[0; 32])
    }
    /// A NEW tree made by a listed member placing itself, naming the act that let it in (K7: an existing space's tree,
    /// made by whichever listed member opens it first — its owner names none).
    pub fn create_by(space: &[u8; 32], account: Account, act: &[u8; 32]) -> Result<(Group, Vec<u8>), String> {
        let mut g = Group::empty(space, account);
        let op = g.add_member(&g.account.id.0.to_bytes(), &g.account.leaf_key(), act)?.ok_or("the maker could not place itself")?;
        Ok((g, op))
    }
    /// An empty replica of `space`'s group for `account` (it learns the tree from the group's operations).
    pub fn empty(space: &[u8; 32], account: Account) -> Group {
        let mut sks = ShareKeyMap::new();
        sks.insert(account.leaf_key(), account.leaf);
        Group { cgka: Cgka::new(tree_id(space), account.id, sks), account, known: Default::default() }
    }
    pub fn save(&self) -> Result<Vec<u8>, String> {
        bincode::serialize(&self.cgka).map_err(|x| x.to_string())
    }
    /// WHOSE each member is (`known`), saved beside the tree.
    pub fn save_known(&self) -> Vec<u8> {
        bincode::serialize(&self.known).unwrap_or_default()
    }
    /// …taken back (a save that does not read: none — learned again as operations are taken).
    pub fn take_known(&mut self, bytes: &[u8]) {
        if let Ok(k) = bincode::deserialize::<std::collections::BTreeMap<[u8; 32], String>>(bytes) {
            self.known.extend(k);
        }
    }
    pub fn load(bytes: &[u8], account: Account) -> Result<Group, String> {
        let cgka: Cgka = bincode::deserialize(bytes).map_err(|x| x.to_string())?;
        let known = std::collections::BTreeMap::new();
        // SAVED BY ANOTHER MEMBER of this account (K8.7: its data key rotated — a new member): the operations kept, the
        // owner this one, with its own leaf only — the old member's keys never used again; it is placed once it enters
        // (fx 10-28: the old owner loaded with the new signer, "Identifier not found" on every change).
        if cgka.owner_id != account.id {
            let mut sks = ShareKeyMap::new();
            sks.insert(account.leaf_key(), account.leaf.clone());
            return Ok(Group { cgka: cgka.with_new_owner(account.id, sks).map_err(e)?, account, known });
        }
        Ok(Group { cgka, account, known })
    }
    /// Whether this account is in the tree (its view).
    pub fn placed(&mut self) -> bool {
        // (Its pending concurrent operations replayed first — read alone, `placed` could be stale: 10-28 review.)
        let _ = self.members();
        self.cgka.owner_leaf_key().is_some()
    }
    /// The member ids the tree places (its view).
    pub fn members(&mut self) -> Vec<[u8; 32]> {
        self.cgka.member_ids().map(|it| it.map(|m| m.0.to_bytes()).collect()).unwrap_or_default()
    }

    /// MERGE another's operation — through OUR CHECK first (fixed facts only: `admitted`).
    pub fn merge(&mut self, op: &[u8], check: &impl Check) -> Merged {
        let Ok(op) = bincode::deserialize::<Signed<CgkaOperation>>(op) else { return Merged::Refused("not an operation".into()) };
        // (WHOSE an added member is, while a card names it — recorded for every add offered, one held too: the adder's
        // own add comes back held, and was never judged here — 10-28 review.)
        if let CgkaOperation::Add { added_id, .. } = &op.payload {
            if let Some(d) = check.account_of(&added_id.0.to_bytes()) {
                self.known.insert(added_id.0.to_bytes(), d);
            }
        }
        // HELD already: judged and verified when it was taken — nothing again (each operation checked once per device).
        let held: beekem::collections::Set<_> = [Digest::hash(&op)].into_iter().collect();
        if self.cgka.contains_predecessors(&held) {
            return Merged::Applied;
        }
        // AUTHENTIC: signed by the member it names (10-27: nothing verified it — our check judged the signer an operation
        // NAMES, so whoever could write where we read could write a key change in another member's name).
        if op.try_verify().is_err() {
            return Merged::Refused("its signature does not verify".into());
        }
        // NOTHING JUDGED YET: a check that cannot judge (`Check::whole`) — it waits.
        if !check.whole() {
            return Merged::Waiting;
        }
        match admitted(&op, check) {
            Ok(()) => {}
            Err(None) => return Merged::Waiting,
            Err(Some(why)) => return Merged::Refused(why),
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

    /// THE GENERATION TO SEAL WITH (FOUNDATION "WHO RECEIVES IS DECIDED BY THE ONE WHO SEALS"): the current one, only
    /// when the tree places nobody the list puts out NOW (`Check::in_now`) and has one current (no concurrent heads).
    /// `None`: not clean — the reconcile takes them out first (its removal, then a key change); writes stay on the last
    /// clean generation meanwhile. Every other generation reached is for READING only.
    pub fn writable(&mut self, check: &impl Check) -> Option<(u64, [u8; 32])> {
        self.writable_why(check).ok()
    }
    /// `writable`, or WHY NOT (the node's trace says it — 10-29: a re-key held on a mark never written, cause unread).
    pub fn writable_why(&mut self, check: &impl Check) -> Result<(u64, [u8; 32]), String> {
        if !check.whole() {
            return Err("the check judges nothing (no current device)".into());
        }
        let out: Vec<String> = self.members().iter().filter(|m| !self.in_now(check, m)).map(|m| ::hex::encode(&m[..4])).collect();
        if !out.is_empty() {
            return Err(format!("placed but not in now: {}", out.join(",")));
        }
        let (h, k) = self.cgka.current_generation_why().map_err(|e| format!("no current generation: {e}"))?;
        Ok((self.number(&h), k.0.to_bytes()))
    }
    /// IN NOW (`Check::in_now`) — a member no card names any more judged by WHOSE it was when taken (`known`): listed now,
    /// or not (then out — the reconcile takes it out by its account's act).
    fn in_now(&self, check: &impl Check, m: &[u8; 32]) -> bool {
        match check.account_of(m) {
            Some(_) => check.in_now(m),
            None => !check.superseded(m) && self.known.get(m).is_some_and(|d| check.listed(d)),
        }
    }
    /// WHOSE a placed member is: its card's account, else the one it was taken as (`known`).
    pub fn whose(&self, m: &[u8; 32]) -> Option<&str> {
        self.known.get(m).map(String::as_str)
    }
    /// MERGE EVERY operation, in any order (page and node, one loop): each offered again while its predecessors arrive,
    /// up to 64 rounds. How many applied, the ones still waiting, why each refused.
    pub fn merge_all(&mut self, ops: &[Vec<u8>], check: &impl Check) -> MergedAll {
        let mut out = MergedAll::default();
        let mut pending: Vec<&Vec<u8>> = ops.iter().collect();
        // (The operations REFUSED, by hash: an operation built on one waits forever — said so, not as "not held".)
        let mut refused = std::collections::BTreeSet::new();
        for _ in 0..64 {
            let before = pending.len();
            pending.retain(|op| match self.merge(op, check) {
                Merged::Applied => (out.applied += 1, false).1,
                Merged::Waiting => true,
                Merged::Refused(w) => {
                    if let Some((h, ..)) = op_info(op) {
                        refused.insert(h);
                    }
                    (out.refused.push(w), false).1
                }
            });
            if pending.is_empty() || pending.len() == before {
                break;
            }
        }
        out.waiting = pending.len();
        let waiting: std::collections::BTreeSet<[u8; 32]> = pending.iter().filter_map(|op| op_info(op).map(|x| x.0)).collect();
        out.waits = pending.iter().map(|op| waiting_why(op, check, &refused, &waiting)).collect();
        out.waiting_cards = pending
            .iter()
            .filter_map(|op| bincode::deserialize::<Signed<CgkaOperation>>(op).ok())
            .filter_map(|op| match op.payload {
                CgkaOperation::Add { added_id, authorization: CgkaAuthorization::Delegation(act), .. } if check.account_of(&added_id.0.to_bytes()).is_none() && act != [0; 32] => Some(act),
                _ => None,
            })
            .collect();
        out
    }

    /// The operations this tree HOLDS that `published` lacks and our check admits (10-24, live: the Ivvor DM's root was on
    /// no feed — the owner's own operations waited on it everywhere, Ivvor never placed — while the owner's saved tree
    /// held it). Written again by a device that holds them, their authors' signatures as they were.
    pub fn unpublished(&self, published: &[Vec<u8>], check: &impl Check) -> Vec<Vec<u8>> {
        let have: std::collections::BTreeSet<Vec<u8>> = published.iter().filter_map(|o| bincode::deserialize::<Signed<CgkaOperation>>(o).ok()).map(|o| Digest::hash(&o).as_slice().to_vec()).collect();
        let Ok(batches) = self.cgka.ops() else { return Vec::new() };
        batches
            .into_iter()
            .flat_map(|b| b.into_iter())
            .filter(|op| !have.contains(Digest::hash(op.as_ref()).as_slice()) && admitted(op, check).is_ok())
            .filter_map(|op| bincode::serialize(op.as_ref()).ok())
            .collect()
    }

    /// ADD a member (its id, its first leaf key), authorized by the act `act`: the operation's bytes, or `None` if the
    /// tree places it already.
    pub fn add_member(&mut self, member: &[u8; 32], leaf: &ShareKey, act: &[u8; 32]) -> Result<Option<Vec<u8>>, String> {
        let id = MemberId(ed25519_dalek::VerifyingKey::from_bytes(member).map_err(|x| x.to_string())?);
        let op = now(self.cgka.add::<Local, _>(id, *leaf, CgkaAuthorization::Delegation(*act), &self.account.signer)).map_err(e)?;
        op.map(|o| bincode::serialize(&o).map_err(|x| x.to_string())).transpose()
    }
    /// ADD by a card's raw leaf key (32 bytes) — the node's (K5).
    pub fn add_member_raw(&mut self, member: &[u8; 32], leaf: &[u8; 32], act: &[u8; 32]) -> Result<Option<Vec<u8>>, String> {
        self.add_member(member, &ShareKey::from(x25519_dalek::PublicKey::from(*leaf)), act)
    }
    /// A KEY CHANGE drawing from `seed` (the node's randomness: upkeep's pool — K5).
    /// NO CURRENT ROOT here (after a pending replay): a removal blanked it, or the heads are concurrent — a key change due.
    pub fn rootless(&mut self) -> bool {
        self.cgka.current_heads();
        !self.cgka.has_tree_root_secret()
    }
    pub fn rotate_seeded(&mut self, seed: [u8; 32]) -> Result<Vec<u8>, String> {
        use rand::SeedableRng;
        Ok(self.rotate(&mut rand::rngs::StdRng::from_seed(seed))?.0)
    }
    /// THE MEMBERS THEIR ACCOUNTS REPLACED (K8.7), taken out — each placed one in `superseded` — and one key change:
    /// the operations (none: none placed). The account's new member is added as any listed member is (`reconcile`).
    pub fn retire_superseded(&mut self, superseded: &[[u8; 32]], seed: [u8; 32]) -> Result<Vec<Vec<u8>>, String> {
        let placed = self.members();
        let mut ops = Vec::new();
        for id in superseded.iter().filter(|id| placed.contains(id)) {
            if let Some(op) = self.remove_member(id, &[0; 32])? {
                ops.push(op);
            }
        }
        if !ops.is_empty() {
            ops.push(self.rotate_seeded(seed)?);
        }
        Ok(ops)
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
    /// The current generation, if there is one with no key change (a single head, its root reached here).
    pub fn current_now(&mut self) -> Option<(u64, [u8; 32])> {
        let (h, k) = self.cgka.current_generation()?;
        Some((self.number(&h), k.0.to_bytes()))
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

/// A LISTED ACCOUNT's member, from its card: `(DID, member id, first leaf key)`.
pub type Card = (String, [u8; 32], [u8; 32]);

/// WHAT RECONCILING DID (K7: page and node, the one plan): the accounts added, those removed (by the act putting them
/// out), and the operations to send (one key change after them).
#[derive(Debug, Default)]
pub struct Reconciled {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub ops: Vec<Vec<u8>>,
    /// Why one was not added or removed (the operation would not build).
    pub not: Vec<String>,
}

impl Group {
    /// RECONCILE the tree to the list (K7, the one plan — `members.reconcile` on a page, `keys_round` on a node): ADD each
    /// card's member whose account is `listed` and the tree does not place, NAMING the act that let it in (`ins`: so the
    /// add stays valid when a later act puts it out — every replay judges it by that act); REMOVE each placed member
    /// whose account is not, by the act `outs` names (none: left — nothing decided without the act); then ONE key change
    /// (`seed`: its randomness). Only from a WHOLE read of the list: the caller's guard.
    /// WHETHER A RECONCILE WOULD CHANGE ANYTHING (page and node — before drawing randomness): a listed card's member the
    /// tree does not place, or a placed one no longer listed that an act put out. `reconcile`'s own conditions.
    /// (And a tree with NO CURRENT ROOT — a removal blanked it, or concurrent heads — is due its one key change: the
    /// removed one cannot make it, and none in step would; 10-29, fz: a self-leave left it rootless, the re-key held
    /// forever on the clean mark only a root gives. `current`'s rule, page and node.)
    pub fn due(&mut self, cards: &[Card], listed: &dyn Fn(&str) -> bool, outs: &std::collections::BTreeMap<String, [u8; 32]>) -> bool {
        if self.rootless() {
            return true;
        }
        let placed = self.members();
        cards.iter().any(|(did, id, _)| wants_in(&placed, id, did, listed)) || placed.iter().any(|id| cards.iter().find(|c| c.1 == *id).map(|c| c.0.clone()).or_else(|| self.known.get(id).cloned()).is_some_and(|did| wants_out(&did, listed, outs)))
    }

    pub fn reconcile(&mut self, cards: &[Card], listed: &dyn Fn(&str) -> bool, ins: &std::collections::BTreeMap<String, [u8; 32]>, outs: &std::collections::BTreeMap<String, [u8; 32]>, seed: [u8; 32]) -> Reconciled {
        let placed = self.members();
        let mut r = Reconciled::default();
        for (did, id, leaf) in cards {
            if !wants_in(&placed, id, did, listed) || r.added.contains(did) {
                continue;
            }
            match self.add_member_raw(id, leaf, ins.get(did).unwrap_or(&[0; 32])) {
                Ok(Some(op)) => (r.ops.push(op), r.added.push(did.clone())).1,
                Ok(None) => {}
                Err(why) => r.not.push(format!("{did}: not added — {why}")),
            }
        }
        for id in &placed {
            // (Its card's account; no card names it any more: the one it was taken as — `known`.)
            let Some(did) = cards.iter().find(|c| c.1 == *id).map(|c| c.0.clone()).or_else(|| self.known.get(id).cloned()) else { continue };
            let did = &did;
            if !wants_out(did, listed, outs) {
                continue;
            }
            let Some(act) = outs.get(did) else { continue };
            match self.remove_member(id, act) {
                Ok(Some(op)) => (r.ops.push(op), r.removed.push(did.clone())).1,
                Ok(None) => {}
                Err(why) => r.not.push(format!("{did}: not removed — {why}")),
            }
        }
        if !r.ops.is_empty() || self.rootless() {
            match self.rotate_seeded(seed) {
                Ok(op) => r.ops.push(op),
                Err(why) => r.not.push(format!("no key change — {why}")),
            }
        }
        r
    }
}

/// The lowest generation NUMBER (depth 1): every generation is at or above it, every MLS epoch below — a key held at
/// one says the space is on BeeKEM.
pub const FIRST_GENERATION: u64 = 1 << 24;

/// A generation's NUMBER (the storage's epoch, whose NEWEST — the highest — is written with): its update's DEPTH in the
/// operations (a later generation is always deeper), and 3 bytes of its hash below it (concurrent ones at one depth).
fn generation_number(depth: u64, h: &keyhive_crypto::digest::Digest<Signed<CgkaOperation>>) -> u64 {
    let low = u64::from(h.as_slice()[0]) << 16 | u64::from(h.as_slice()[1]) << 8 | u64::from(h.as_slice()[2]);
    depth << 24 | low
}

/// OUR CHECK (the list's): the signer listed; an add or a remove authorized by an act that allows exactly that.
/// WHAT an operation WAITS on (`merge_all`'s `waits`): the check not whole, its added member's card or published leaf
/// not read, else a predecessor — REFUSED (it waits for good: built on a branch our check refuses), itself waiting, or on
/// no feed read here — and whose operation, which kind. (10-29: "not held" was said of a refused predecessor too, and
/// read as a lost one — Craftworks' three were built on a stray root the creator rule refuses.)
fn waiting_why(op: &[u8], check: &impl Check, refused: &std::collections::BTreeSet<[u8; 32]>, waiting: &std::collections::BTreeSet<[u8; 32]>) -> String {
    let Ok(op) = bincode::deserialize::<Signed<CgkaOperation>>(op) else { return "not an operation".into() };
    let short = |b: &[u8]| ::hex::encode(&b[..4]);
    let (kind, target) = match &op.payload {
        CgkaOperation::Add { added_id, .. } => ("add", Some(added_id.0.to_bytes())),
        CgkaOperation::Remove { id, .. } => ("remove", Some(id.0.to_bytes())),
        _ => ("key change", None),
    };
    let on = if !check.whole() {
        "the check not whole".to_string()
    } else if let Some(t) = target.filter(|t| kind == "add" && check.account_of(t).is_none()) {
        format!("{}'s card not read", short(&t))
    } else if let Some(t) = target.filter(|t| kind == "add" && check.leaf_of(t).is_none()) {
        format!("{}'s published leaf not read", short(&t))
    } else {
        let preds: Vec<[u8; 32]> = op.payload.predecessors().iter().filter_map(|d| <[u8; 32]>::try_from(d.as_slice()).ok()).collect();
        let named = |set: &std::collections::BTreeSet<[u8; 32]>| preds.iter().filter(|p| set.contains(*p)).map(|p| short(p)).collect::<Vec<_>>().join(", ");
        if preds.iter().any(|p| refused.contains(p)) {
            format!("built on a REFUSED operation {} (waits for good)", named(refused))
        } else if preds.iter().any(|p| waiting.contains(p)) {
            format!("on a waiting operation {}", named(waiting))
        } else {
            format!("{} predecessor(s) on no feed read here", preds.len())
        }
    };
    format!("{kind}{} by {}: {on}", target.map(|t| format!(" {}", short(&t))).unwrap_or_default(), short(&op.issuer().to_bytes()))
}

/// `Err(None)`: not judged yet — WAITS (its added member's card not read: its account or its published leaf unknown).
fn admitted(op: &Signed<CgkaOperation>, check: &impl Check) -> Result<(), Option<String>> {
    let signer = op.issuer().to_bytes();
    let did = check.account_of(&signer).ok_or(Some("signed by a member id no listed account publishes".to_string()))?;
    // (EVER let in — never "listed now": one put out since is not WRITTEN for — `Group::writable`.)
    if !check.ever_in(&did) {
        return Err(Some(format!("signed by {did}, never on the list")));
    }
    // A TREE's ROOT (no predecessors) only by its creator: two members each making a tree — each page seeing none yet
    // — forked a space into two (10-19, live: Craftworks' tree made by a member, the owner's own operations left
    // waiting forever on a root that lost; nobody was ever added). One account may make it, so there is one root.
    if op.payload.predecessors().is_empty() && check.creator().as_deref() != Some(did.as_str()) {
        return Err(Some(format!("a tree's root signed by {did}, not the space's creator")));
    }
    match &op.payload {
        // AN ADD: by the act it NAMES (a space: the act letting that account in — the owner's own placement, none; the
        // account's tree: a device the account ever credentialed) — never by who is listed NOW (10-28 re-review: an add
        // judged by the list of the day went unauthorized once its member was kicked, and every later fresh merge left
        // the kick waiting on it — the tree forked between those who merged before and after).
        CgkaOperation::Add { added_id, pk, authorization, .. } => {
            let target = added_id.0.to_bytes();
            // (Whose it is not known yet — its card not read: it waits.)
            if check.account_of(&target).is_none() {
                return Err(None);
            }
            if !matches!(authorization, CgkaAuthorization::Delegation(act) if check.allows(act, true, &target)) {
                return Err(Some("an add of an account not listed, no act allowing it".into()));
            }
            // ITS LEAF: the one the added member published, never one the adder chose (not read yet: it waits).
            match check.leaf_of(&target) {
                Some(l) if l == pk.to_bytes() => Ok(()),
                Some(_) => Err(Some("an add with a leaf its member did not publish".into())),
                None => Err(None),
            }
        }
        CgkaOperation::Remove { id, authorization, .. } => match authorization {
            // A member its account REPLACED (K8.7): out with no act — the tree holds each account's current member.
            _ if check.superseded(&id.0.to_bytes()) => Ok(()),
            CgkaAuthorization::Revocation(act) if check.allows(act, false, &id.0.to_bytes()) => Ok(()),
            _ => Err(Some("a removal no act allows".into())),
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

    use ::hex::encode as hex;
    fn err(x: impl core::fmt::Display) -> JsValue {
        JsValue::from_str(&x.to_string())
    }
    fn b32(b: &[u8]) -> Result<[u8; 32], JsValue> {
        b.try_into().map_err(|_| err("32 bytes expected"))
    }
    fn bytes(v: Option<Vec<u8>>) -> JsValue {
        v.map(|b| js_sys::Uint8Array::from(&b[..]).into()).unwrap_or(JsValue::NULL)
    }
    /// OUR CHECK (`ListCheck`, the node's too), built from the list: `byMember` `[[member hex, did]…]`, `listed` `[did…]`,
    /// the owner, `acts` `[[act hash hex, in?, did]…]`.
    #[wasm_bindgen]
    pub struct BeeList(Chk);
    /// The check a group is merged under: a space's list, or the account's devices list (K8.5).
    enum Chk {
        List(ListCheck),
        Account(AccountCheck),
    }
    impl Check for Chk {
        fn account_of(&self, m: &[u8; 32]) -> Option<String> {
            match self { Chk::List(c) => c.account_of(m), Chk::Account(c) => c.account_of(m) }
        }
        fn listed(&self, d: &str) -> bool {
            match self { Chk::List(c) => c.listed(d), Chk::Account(c) => c.listed(d) }
        }
        fn allows(&self, act: &[u8; 32], add: bool, t: &[u8; 32]) -> bool {
            match self { Chk::List(c) => c.allows(act, add, t), Chk::Account(c) => c.allows(act, add, t) }
        }
        fn creator(&self) -> Option<String> {
            match self { Chk::List(c) => c.creator(), Chk::Account(c) => c.creator() }
        }
        fn leaf_of(&self, m: &[u8; 32]) -> Option<[u8; 32]> {
            match self { Chk::List(c) => c.leaf_of(m), Chk::Account(c) => c.leaf_of(m) }
        }
        fn superseded(&self, m: &[u8; 32]) -> bool {
            match self { Chk::List(c) => c.superseded(m), Chk::Account(c) => c.superseded(m) }
        }
        fn ever_in(&self, d: &str) -> bool {
            match self { Chk::List(c) => c.ever_in(d), Chk::Account(c) => c.ever_in(d) }
        }
        fn in_now(&self, m: &[u8; 32]) -> bool {
            match self { Chk::List(c) => c.in_now(m), Chk::Account(c) => c.in_now(m) }
        }
        fn whole(&self) -> bool {
            match self { Chk::List(c) => c.whole(), Chk::Account(c) => c.whole() }
        }

    }
    #[wasm_bindgen]
    impl BeeList {
        #[wasm_bindgen(constructor)]
        /// `by_member` `[[member hex, did, leaf hex]…]` (each card's member and published leaf); `superseded` the same for
        /// the members their accounts replaced (K8.7: still their history's authors, taken out with no act).
        pub fn new(by_member: js_sys::Array, listed: js_sys::Array, owner: String, acts: js_sys::Array, superseded: Option<js_sys::Array>) -> Result<BeeList, JsValue> {
            let h32 = |v: JsValue| -> Result<[u8; 32], JsValue> {
                let s = v.as_string().ok_or_else(|| err("a hex string"))?;
                let b: Option<Vec<u8>> = (0..s.len() / 2).map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect();
                b32(&b.ok_or_else(|| err("hex"))?)
            };
            let mut l = ListCheck { owner, ..ListCheck::default() };
            for e in by_member.iter() {
                let e = js_sys::Array::from(&e);
                let m = h32(e.get(0))?;
                l.by_member.insert(m, e.get(1).as_string().unwrap_or_default());
                if let Ok(leaf) = h32(e.get(2)) {
                    l.leaves.insert(m, leaf);
                }
            }
            for e in superseded.map(|a| a.iter().collect::<Vec<_>>()).unwrap_or_default() {
                let e = js_sys::Array::from(&e);
                let m = h32(e.get(0))?;
                l.by_member.insert(m, e.get(1).as_string().unwrap_or_default());
                if let Ok(leaf) = h32(e.get(2)) {
                    l.leaves.insert(m, leaf);
                }
                l.superseded.insert(m);
            }
            l.listed = listed.iter().filter_map(|d| d.as_string()).collect();
            let acts = acts
                .iter()
                .map(|e| {
                    let e = js_sys::Array::from(&e);
                    Ok((h32(e.get(0))?, (e.get(1).as_bool().unwrap_or(false), e.get(2).as_string().unwrap_or_default())))
                })
                .collect::<Result<Vec<_>, JsValue>>()?;
            l.acts.extend(acts);
            Ok(BeeList(Chk::List(l)))
        }
        /// THE ACCOUNT's CHECK (`AccountCheck`): `current` `[member hex…]` — the group keys of its devices' credentials
        /// on its key log's current key; `writers` `[member hex…]` — the devices whose `account-keyops` the directory lists.
        /// … `leaves` `[[member hex, leaf hex]…]` — each device's signed leaf (an add is taken only with it).
        pub fn account(current: js_sys::Array, ever: js_sys::Array, writers: js_sys::Array, leaves: js_sys::Array) -> Result<BeeList, JsValue> {
            let h32 = |v: JsValue| -> Result<[u8; 32], JsValue> {
                let s = v.as_string().ok_or_else(|| err("a hex string"))?;
                let b: Option<Vec<u8>> = (0..s.len() / 2).map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect();
                b32(&b.ok_or_else(|| err("hex"))?)
            };
            let mut lv = std::collections::BTreeMap::new();
            for e in leaves.iter() {
                let e = js_sys::Array::from(&e);
                lv.insert(h32(e.get(0))?, h32(e.get(1))?);
            }
            Ok(BeeList(Chk::Account(AccountCheck { current: current.iter().map(h32).collect::<Result<_, _>>()?, ever: ever.iter().map(h32).collect::<Result<_, _>>()?, writers: writers.iter().map(h32).collect::<Result<_, _>>()?, leaves: lv })))
        }
    }

    /// AN OPERATION described (diagnostics, 10-23): its hash, its kind and the hashes of the operations it builds on —
    /// `[hash hex, kind, [predecessor hex…]]`, or null for bytes that are not one.
    #[wasm_bindgen]
    pub fn beekem_op_info(op: &[u8]) -> JsValue {
        let Some((h, kind, preds, issuer)) = op_info(op) else { return JsValue::NULL };
        let preds: js_sys::Array = preds.iter().map(|d| JsValue::from(hex(d))).collect();
        // (Its AUTHOR — the member that signed it: the account tree's root maker is read from it, K8.5.)
        [JsValue::from(hex(&h)), JsValue::from(kind), preds.into(), JsValue::from(hex(&issuer))].into_iter().collect::<js_sys::Array>().into()
    }


    /// A DEVICE's LEAF, SIGNED by its member (K8.5, `Account::signed_leaf`): `[leaf, signature]`.
    #[wasm_bindgen]
    pub fn beekem_signed_leaf(member_seed: &[u8]) -> Result<js_sys::Array, JsValue> {
        let (leaf, sig) = Account::of(&b32(member_seed)?).signed_leaf();
        Ok([js_sys::Uint8Array::from(&leaf[..]), js_sys::Uint8Array::from(&sig[..])].into_iter().map(JsValue::from).collect())
    }
    /// Whether a device's leaf is signed by its member (`leaf_checks`).
    #[wasm_bindgen]
    pub fn beekem_leaf_checks(member: &[u8], leaf: &[u8], sig: &[u8]) -> bool {
        let (Ok(m), Ok(l), Ok(s)) = (b32(member), b32(leaf), <[u8; 64]>::try_from(sig)) else { return false };
        leaf_checks(&m, &l, &s)
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
        pub fn create(space: &[u8], member_seed: &[u8], act: Option<Vec<u8>>) -> Result<js_sys::Array, JsValue> {
            let act = act.map(|a| b32(&a)).transpose()?.unwrap_or([0; 32]);
            let (g, op) = Group::create_by(&b32(space)?, Account::of(&b32(member_seed)?), &act).map_err(err)?;
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
        /// WHOSE each member is (`Group::save_known`), saved beside the tree.
        pub fn save_known(&self) -> js_sys::Uint8Array {
            js_sys::Uint8Array::from(&self.0.save_known()[..])
        }
        pub fn take_known(&mut self, bytes: &[u8]) {
            self.0.take_known(bytes);
        }
        pub fn placed(&mut self) -> bool {
            self.0.placed()
        }
        /// The member ids placed (hex).
        pub fn members(&mut self) -> js_sys::Array {
            self.0.members().iter().map(|m| JsValue::from(hex(m))).collect()
        }
        /// MERGE EVERY operation (`Group::merge_all`, the node's too) through OUR CHECK (`BeeList`, the node's too):
        /// `{ applied, waiting, refused: [why], misses: [what a refusal saw] }`.
        pub fn merge_all(&mut self, ops: js_sys::Array, list: &BeeList) -> Result<JsValue, JsValue> {
            let ops: Vec<Vec<u8>> = ops.iter().map(|o| js_sys::Uint8Array::new(&o).to_vec()).collect();
            let r = self.0.merge_all(&ops, &list.0);
            let out = js_sys::Object::new();
            js_sys::Reflect::set(&out, &"applied".into(), &(r.applied as f64).into())?;
            js_sys::Reflect::set(&out, &"waiting".into(), &(r.waiting as f64).into())?;
            js_sys::Reflect::set(&out, &"refused".into(), &r.refused.iter().map(|w| JsValue::from_str(w)).collect::<js_sys::Array>())?;
            js_sys::Reflect::set(&out, &"waits".into(), &r.waits.iter().map(|w| JsValue::from_str(w)).collect::<js_sys::Array>())?;
            let misses: Vec<String> = match &list.0 { Chk::List(l) => l.misses.borrow().clone(), Chk::Account(_) => Vec::new() };
            js_sys::Reflect::set(&out, &"misses".into(), &misses.iter().map(|w| JsValue::from_str(w)).collect::<js_sys::Array>())?;
            Ok(out.into())
        }
        /// The operations this tree holds that `ops` (the network's) lacks and our check admits (`Group::unpublished`).
        pub fn unpublished(&self, ops: js_sys::Array, list: &BeeList) -> js_sys::Array {
            let ops: Vec<Vec<u8>> = ops.iter().map(|o| js_sys::Uint8Array::new(&o).to_vec()).collect();
            self.0.unpublished(&ops, &list.0).iter().map(|o| JsValue::from(js_sys::Uint8Array::from(o.as_slice()))).collect()
        }
        /// MERGE an operation through our check (`BeeList`): "applied", "waiting", or "refused: why".
        pub fn merge(&mut self, op: &[u8], list: &BeeList) -> String {
            match self.0.merge(op, &list.0) {
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
        /// THE MEMBERS THEIR ACCOUNTS REPLACED taken out (K8.7, `Group::retire_superseded`): `superseded` `[member hex…]`
        /// → the operations to send.
        pub fn retire(&mut self, superseded: js_sys::Array) -> Result<js_sys::Array, JsValue> {
            let ids: Vec<[u8; 32]> = superseded
                .iter()
                .filter_map(|v| v.as_string())
                .filter_map(|s| (0..s.len() / 2).map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()).collect::<Option<Vec<u8>>>())
                .filter_map(|b| b.try_into().ok())
                .collect();
            let mut seed = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut seed);
            let ops = self.0.retire_superseded(&ids, seed).map_err(err)?;
            Ok(ops.iter().map(|o| JsValue::from(js_sys::Uint8Array::from(&o[..]))).collect())
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
        /// RECONCILE to the list (`Group::reconcile`, the node's too): `cards` `[[did, member, leaf]…]`, `listed` the DIDs
        /// listed now, `ins`/`outs` `{ did: act }` (32 bytes each). `{ added, removed, ops, not }`.
        pub fn reconcile(&mut self, cards: js_sys::Array, listed: js_sys::Array, ins: js_sys::Object, outs: js_sys::Object) -> Result<JsValue, JsValue> {
            let cards: Vec<Card> = cards
                .iter()
                .filter_map(|c| {
                    let c = js_sys::Array::from(&c);
                    Some((c.get(0).as_string()?, b32(&js_sys::Uint8Array::new(&c.get(1)).to_vec()).ok()?, b32(&js_sys::Uint8Array::new(&c.get(2)).to_vec()).ok()?))
                })
                .collect();
            let listed: Vec<String> = listed.iter().filter_map(|d| d.as_string()).collect();
            let acts = |o: &js_sys::Object| -> std::collections::BTreeMap<String, [u8; 32]> {
                js_sys::Object::entries(o)
                    .iter()
                    .filter_map(|e| {
                        let e = js_sys::Array::from(&e);
                        Some((e.get(0).as_string()?, b32(&js_sys::Uint8Array::new(&e.get(1)).to_vec()).ok()?))
                    })
                    .collect()
            };
            let mut seed = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut seed);
            let r = self.0.reconcile(&cards, &|d| listed.iter().any(|l| l == d), &acts(&ins), &acts(&outs), seed);
            let strs = |v: &[String]| -> js_sys::Array { v.iter().map(|s| JsValue::from(s.as_str())).collect() };
            let out = js_sys::Object::new();
            js_sys::Reflect::set(&out, &"added".into(), &strs(&r.added))?;
            js_sys::Reflect::set(&out, &"removed".into(), &strs(&r.removed))?;
            js_sys::Reflect::set(&out, &"not".into(), &strs(&r.not))?;
            js_sys::Reflect::set(&out, &"ops".into(), &r.ops.iter().map(|o| JsValue::from(js_sys::Uint8Array::from(&o[..]))).collect::<js_sys::Array>())?;
            Ok(out.into())
        }
        /// THE GENERATION TO WRITE WITH: `[number (decimal string), secret, operation or null]`.
        pub fn current(&mut self) -> Result<js_sys::Array, JsValue> {
            let (n, k, op) = self.0.current(&mut rand::rngs::OsRng).map_err(err)?;
            Ok([JsValue::from(n.to_string()), js_sys::Uint8Array::from(&k[..]).into(), bytes(op)].into_iter().collect())
        }
        /// THE GENERATION TO SEAL WITH (`Group::writable`): `[number (decimal string), secret]`, or null (not clean: the
        /// reconcile first).
        pub fn writable(&mut self, list: &BeeList) -> JsValue {
            match self.0.writable(&list.0) {
                Some((n, k)) => [JsValue::from(n.to_string()), js_sys::Uint8Array::from(&k[..]).into()].into_iter().collect::<js_sys::Array>().into(),
                None => JsValue::NULL,
            }
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

