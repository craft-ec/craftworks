//! # The IDENTITY delegate (ARCHITECTURE §4)
//!
//! One per node. It holds the MEMBERS on this node and signs records for them. Nothing else holds a key: a page
//! asks, and gets a signature back. The DID is the account (its owner's public key, derived from the recovery words);
//! a member is one node's key admitted to it, opened here by a PIN.
//!
//! - **One gate.** Every request comes from a web app the node names (`MessageOrigin::WebApp`); anything else, an
//!   unattested caller or another delegate, is refused before any request is read (the delegate entry). So everything
//!   below knows which APP asks.
//! - **Members, chosen by PIN** (a shared computer): each member is a key, the DID it belongs to, the app that made
//!   it (its HOME) and the account's data key. "Register with this node" / "log in with recovery words" (`Provision`)
//!   make one, with the PIN the person sets; no two members share a PIN. The same node and the same PIN (`Unlock`)
//!   open the same member. Several members on one node may belong to one DID or to different people's.
//! - **Wrong PINs:** five in a row, by anyone, lock PIN unlock on this node (a new member refused for a taken PIN
//!   counts too: it reveals that PIN opens a member). A good unlock clears the count. A locked node is opened by the
//!   account's WORDS (or its passphrase, which carries them): `Provision` of a new member of an account that has a
//!   member here, with that account's data key — which only the words give — clears the lock. (A member's own key
//!   again, from a handover, sets its PIN anew too.)
//! - **Sessions:** an unlock opens a session for that APP on this node, naming one member, kept here until the app
//!   logs out (a reload, another tab or another browser on the node is still logged in). One app's session is never
//!   another's. Who and Sign go through it. (No key file: a member's key never leaves.)
//! - **No recovery words here.** The words are the account's owner: a node that kept them would lose the account to
//!   anyone who copied its disk, and an owner cannot be rotated away. Shown once at registration, typed when needed.
//! - **The account's data key:** each member keeps its account's DATA key (derived from the words, the same on every
//!   node of the account), so any of the account's nodes signs the account's data. `Unlocked` names its public key.
//! - **The account's data is the account's, not a site's.** Its TABLES (notes, pins, labels, …) are tails under the
//!   data key, labelled `t/<table>`, and any site the person allows reads and writes them: another developer's front
//!   end over the same notes, or a second address for the same app. A site may WRITE a table only with a GRANT: the
//!   first time it asks (`Grant`), the node itself prompts the person ("Delegate says: allow … ?", naming the asking
//!   app from its own records, never from anything a site sent). A site cannot fake that answer. Grants are per member
//!   (so per person on a shared node), per site, per table; the home app (the site the member was made with, the
//!   person's own choice of app) is granted without a prompt, and lists and revokes them.
//! - **Sign:** the Register's one signed message (`Params::signed_message`, non-terminal). A tail signs the same
//!   message over its body hash, so one verb covers both. Rules, in order:
//!   0. an open session names the member;
//!   1. the params name ONE key, and it is the account's data key;
//!   2. the label is `t/<table>`, and the asking site holds this member's grant for that table;
//!   3. SEQ GUARD: never two different values at one seq, and never a seq below the last one signed for that record
//!      (an identical re-ask gets the same signature back);
//!   4. the guard is saved BEFORE the signature is returned (the node syncs a secret to disk before `set_secret`
//!      returns), so a crash can lose a signature nobody received, never sign twice.
//! - **Handover** (the next version of this delegate): a new build is a new delegate key, and a delegate's secrets
//!   stay with its key; freenet does not carry them over (freenet-agent-skills, "Delegate WASM Upgrade & Secret
//!   Migration"). So this build ANSWERS its successor: `Handover { pin }` returns the member that PIN opens, to that
//!   member's home app, counted as an unlock try. The successor's page asks here on a PIN its own delegate does not
//!   know, and provisions what comes back. Person by person, on their next login; the PIN still keeps people on one
//!   node apart. Forward-only: builds before this one cannot answer.
//!
//! The PIN is a check this delegate makes with a try limit, never a key: whoever can read the node's secret store
//! can read the keys beside it anyway.
//!
//! [`serve`] is the whole request over a [`Host`] (just the secret store), so every rule is tested natively with a
//! map. `delegate` (the entry the node calls) is the gate, then `serve` over the real context.

use craftec_register_contract::wire::{Authority, Params, HASH_LEN, KEY_LEN};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};



/// Every identity message starts with this, so a page tells an identity answer from any other delegate's.
pub const MAGIC: &[u8; 4] = b"ID01";

/// What a page asks. `id` is the page's own, returned with the answer. The session is the asking app's (the node
/// names the app), so no request carries one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// A new member: a 32-byte ed25519 seed the page minted, the DID (the owner's public key) it belongs to, a PIN no
    /// other member on this node has, and the account's data key seed (32 bytes, or empty). A member's own key again
    /// (a key file, or a handover) sets its PIN anew. Opens a session on it.
    Provision { seed: [u8; 32], did: [u8; 32], pin: String, data: Vec<u8> },
    /// Open a session on the member this PIN belongs to.
    Unlock { pin: String },
    /// Close this app's session.
    Lock,
    /// The session's member.
    Who,
    /// Sign `params.signed_message(false, seq, value_hash)` with the session's member, the account's data key, or the
    /// key of an epoch's log of `space` (`None`: the account).
    /// `table`: the table's NAME, for a tail whose label is blinded (`blind_name`): checked against the label, and the
    /// grant is the name's.
    Sign { params: Vec<u8>, seq: u64, value_hash: [u8; HASH_LEN], space: Option<[u8; 32]>, table: Option<String> },
    /// For the next version of this delegate: the member this PIN opens, to its home app. Counted as an unlock try.
    Handover { pin: String },
    /// Leave to write these tables of the session member's account: ONE prompt for all the ones not yet held (an app
    /// asks for every table it uses at once). Answered at once if all are held; otherwise the node prompts the person,
    /// and the answer comes when they choose (or after the node's timeout, as a denial).
    Grant { tables: Vec<String> },
    /// The grants the session member gave: all of them to its home app, or the asking site's own.
    Grants,
    /// Withdraw one: the home app withdraws any, a site its own.
    Revoke { app: [u8; 32], table: String },
    /// The KEY that seals table `table` of the session member's account (generation `gen`): given only to a site the
    /// person allowed that table (its home app always), because with it a site READS the table. Derived here from the
    /// account's data key, which never leaves.
    TableKey { table: String, gen: u8 },
    /// KEEP this member's MLS state of a SPACE's group (`None`: the account's; run by the page) and the secret of its
    /// current epoch. The home site only: whoever holds the state can act in the group.
    MlsSave { space: Option<[u8; 32]>, state: Vec<u8>, epoch: u64, secret: [u8; 32] },
    /// The member's MLS state of a space's group, to the home site only.
    MlsLoad { space: Option<[u8; 32]> },
    /// The key of table `table` in MLS epoch `epoch` (`None`: the newest this member holds) of a space's group: from
    /// that epoch's secret, kept here; the account's to a site allowed that table, another space's to the home site.
    TableKeyAt { table: String, epoch: Option<u64>, space: Option<[u8; 32]> },
    /// KEEP an earlier epoch's secret of a space's group (recovered from escrow, or walked): the home site only.
    EpochKeep { space: Option<[u8; 32]>, epoch: u64, secret: [u8; 32] },
    /// FORGET the session's member: its key, PIN, grants, MLS state and every epoch's secret — for a node REMOVED
    /// from the account (its group told it so), so what it held can no longer be taken from it. The home site only.
    /// The account is untouched: its recovery words make this node a new member again.
    Forget,
    /// With `Handover`, to the next build: the member's KEYS on the right PIN — its MLS state and every epoch's
    /// secret it holds — so an update never costs a member its place in the account's group, or what it could read.
    /// Its home only.
    HandoverKeys { pin: String },
    /// The public half of the account's INBOX key (for its card): what is sealed to it only the account's nodes open.
    InboxKey,
    /// OPEN items sealed to the account's inbox key: each opened, or `None` (not sealed to it). The home site only: the
    /// inbox is the person's, not a site's.
    InboxOpen { items: Vec<Vec<u8>> },
    /// With `HandoverKeys`, to the next build: the member's SPACES on the right PIN — each space's group state and every
    /// epoch's secret it holds — so an update never costs a member a server or a conversation. Its home only.
    HandoverSpaces { pin: String },
    /// The DID's MEMBER for spaces (the same on every device): its MLS signing key's seed and public key, its
    /// credential (signed by the data key), and its feed key. The home site only.
    SpaceMember,
    /// UPKEEP with no page open (the node wakes this delegate: its manifest's `upkeep`): the contract the session
    /// account's INBOX is (a bag's instance id, the page computes it) — watched and read at each wake-up; with the
    /// page's RANDOMNESS (stirred into upkeep's pool) and TIME in seconds (upkeep's clock). Home only.
    UpkeepWatch { inbox: [u8; 32], seed: [u8; 32], now: u64 },
    /// What upkeep did: the wake-ups run, and the inbox as last read (its state's length; `None`: not read yet).
    UpkeepStatus,
    /// The contracts upkeep writes (their CODE: a PUT needs it) — the bag (inboxes, invite requests) and the tail
    /// (epoch logs) — and the key log's code HASH (it only reads those). Home only; once per build.
    /// The contracts upkeep writes and reads: the Bag's and Tail's code, the key log's hash — and for RE-KEYS (no page
    /// open) the Sealed and Piece codes and the Block code's hash.
    UpkeepCodes { bag: Vec<u8>, tail: Vec<u8>, idlog: [u8; 32], sealed: Vec<u8>, piece: Vec<u8>, block: [u8; 32] },
    /// The MANDATE (what a page that may invite knows now): per space, how people get in and who is in, and its MLS
    /// group. Upkeep admits askers by it while no page runs. A space's mandate older than the group upkeep itself moved
    /// is not taken (answered in `stale`): the page loads upkeep's newer group first. Home only.
    /// `spent`: the key packages this account used already (`kp_tag`s): never used again.
    UpkeepMandate { me: String, spaces: Vec<Mandate>, spent: Vec<[u8; 16]> },
    /// The admissions the page has written as acts (and the groups it has loaded): forgotten here.
    UpkeepAck { admitted: Vec<([u8; 32], String)> },
    /// The secrets this member holds of a space's EARLIER epochs (below `below`): handed to a member let in later (a
    /// welcome, or their ask), so they read the space's whole history without walking its logs. The home site only.
    EpochSecrets { space: [u8; 32], below: u64 },
}

/// A space as upkeep may admit into it: the page's knowledge when it handed it over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mandate {
    pub space: [u8; 32],
    pub name: String,
    pub kind: String,
    /// Its governance, as the welcome names it: the owner's DID and the nonce.
    pub owner: String,
    pub nonce: Option<String>,
    /// The table its epoch logs are named by (the space's channel).
    pub channel: String,
    /// Anyone may join (who asks at `open <space id>` is let in).
    pub open: bool,
    /// Invite codes in force: `(code, expires ms or 0, uses left or 0 for no limit)`.
    pub codes: Vec<(String, u64, u32)>,
    pub bans: Vec<String>,
    pub members: Vec<String>,
    /// The group: its epoch and its MLS state.
    pub epoch: u64,
    pub state: Vec<u8>,
}

/// Someone upkeep let in: into `space`, by `code` ("open": no code), at `at` (ms), the group then at `epoch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Admitted {
    pub space: [u8; 32],
    pub did: String,
    pub code: String,
    pub at: u64,
    pub epoch: u64,
    /// The key package it used (`kp_tag`): the page records it as spent.
    pub kp: [u8; 16],
}

/// A KEY PACKAGE's tag (SHA-256 of its hex, 16 bytes) — how the account records one as used (a key package works once),
/// the same in the page (its table `keypacks`) and in upkeep.
pub fn kp_tag(hex: &str) -> [u8; 16] {
    use sha2::Digest;
    sha2::Sha256::digest(hex.as_bytes())[..16].try_into().expect("16")
}

/// What the identity answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Answer {
    /// The session is open on this member: its key, its DID (the owner's public key), and the public key of the
    /// account's data key when this node holds it.
    Unlocked { public: [u8; KEY_LEN], did: [u8; 32], data: Option<[u8; KEY_LEN]> },
    /// No member has this PIN.
    WrongPin { tries_left: u8 },
    /// Too many wrong PINs: only the account's words (a new member with its data key) open this node again.
    Locked,
    /// The session is closed.
    LoggedOut,
    /// The 64-byte ed25519 signature.
    Signed { sig: Vec<u8> },
    /// A member, handed to the next version of this delegate.
    Handed { seed: [u8; 32], did: [u8; 32], data: Option<[u8; 32]> },
    Granted { tables: Vec<String> },
    Grants { list: Vec<([u8; 32], String)> },
    Revoked,
    Refused(Why),
    TableKey { key: [u8; 32] },
    MlsSaved,
    MlsState { state: Option<Vec<u8>> },
    TableKeyAt { epoch: u64, key: [u8; 32] },
    /// A member's keys, handed to the next build.
    HandedKeys { mls: Option<Vec<u8>>, epochs: Vec<(u64, [u8; 32])> },
    InboxKey { public: [u8; 32] },
    Opened { items: Vec<Option<Vec<u8>>> },
    /// A member's spaces, handed to the next build: `(space id, its group state, its epochs' secrets)`.
    HandedSpaces { spaces: Vec<([u8; 32], Option<Vec<u8>>, Vec<(u64, [u8; 32])>)> },
    SpaceMember { seed: [u8; 32], public: [u8; 32], credential: Vec<u8> },
    /// Upkeep so far: wake-ups run, the watched inbox (if set) and its state's length when last read.
    Upkeep {
        wakeups: u64,
        inbox: Option<[u8; 32]>,
        inbox_len: Option<u64>,
        now: Option<u64>,
        /// The codes' hash upkeep holds (`upkeep_codes_hash`), if any.
        codes: Option<[u8; 32]>,
        /// Who upkeep let in, not yet acknowledged; and each space whose group it moved: `(space, epoch, state)`.
        admitted: Vec<Admitted>,
        groups: Vec<([u8; 32], u64, Vec<u8>)>,
        /// Spaces of the last mandate not taken (upkeep's group is newer); what upkeep last did or met.
        stale: Vec<[u8; 32]>,
        said: Option<String>,
    },
    EpochSecrets { epochs: Vec<(u64, [u8; 32])> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Why {
    /// Another member on this node has that PIN: choose another.
    PinTaken,
    /// This key's member belongs to another DID.
    OtherDid,
    /// Asked by an app that is not the member's home.
    NotHome,
    /// This app has no open session on this node: ask for the PIN.
    NoSession,
    /// A PIN must be 6 to 64 characters.
    BadPin,
    /// A data key seed is 32 bytes.
    BadDataKey,
    /// The bytes are not one Register's params.
    BadParams,
    /// The params name a quorum or another key.
    NotThisKey,
    /// The label does not begin with the asking app's contract id.
    NotYourRecord,
    /// A different value at a seq already signed, or a seq below the last one.
    WouldFork { last_seq: u64 },
    /// A secret could not be saved, so nothing changed.
    NotSaved,
    /// The caller is not a web app the node names (unattested, or another delegate): nothing is answered to it.
    NotAttested,
    /// A table name is 1 to 32 of a-z, 0-9, `-`, `_`; a grant names 1 to `MAX_GRANT` of them.
    BadTable,
    /// The params are not one of the account's tables (`t/<table>` under its data key).
    NotATable,
    /// This site has no grant for that table: ask for one (`Grant`).
    NotGranted { table: String },
    /// The person said no (or did not answer).
    Denied,
    /// This node holds no data key for the member.
    NoDataKey,
    /// Not a request this identity reads.
    Unreadable,
    /// No MLS epoch secret is held here (for that epoch, or at all yet).
    NoEpoch,
}

/// The most tables one grant names (an app's `uses`, asked in one prompt).
pub const MAX_GRANT: usize = 32;

/// Wrong PINs in a row allowed before PIN unlock is locked on this node.
pub const MAX_TRIES: u8 = 5;

pub fn encode_request(id: u32, r: &Request) -> Vec<u8> {
    [&MAGIC[..], &bincode::serialize(&(id, r)).expect("a request encodes")].concat()
}

pub fn decode_request(b: &[u8]) -> Option<(u32, Request)> {
    bincode::deserialize(b.strip_prefix(MAGIC)?).ok()
}

pub fn encode_answer(id: u32, a: &Answer) -> Vec<u8> {
    [&MAGIC[..], &bincode::serialize(&(id, a)).expect("an answer encodes")].concat()
}

pub fn decode_answer(b: &[u8]) -> Option<(u32, Answer)> {
    bincode::deserialize(b.strip_prefix(MAGIC)?).ok()
}

/// What the identity reaches: its secret store, nothing more.
pub trait Host {
    fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>>;
    fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool;
}

/// UPKEEP's own record (no page open: the node wakes this delegate): the inbox contract to watch, how many wake-ups
/// ran, and the inbox's state length when last read.
pub const UPKEEP_INBOX: &[u8] = b"identity_upkeep/inbox";
pub const UPKEEP_WAKEUPS: &[u8] = b"identity_upkeep/wakeups";
pub const UPKEEP_INBOX_LEN: &[u8] = b"identity_upkeep/inbox_len";
fn u64_of(v: Option<Vec<u8>>) -> Option<u64> {
    v.and_then(|b| b.try_into().ok()).map(u64::from_le_bytes)
}
/// Upkeep's status; its mandate, admissions and groups are the member's (`m`: the session's), none without one.
pub fn upkeep_status<H: Host>(h: &H, m: Option<&[u8; KEY_LEN]>) -> Answer {
    let mine = |f: &dyn Fn(&[u8; KEY_LEN]) -> Option<Vec<u8>>| m.and_then(f);
    Answer::Upkeep {
        wakeups: u64_of(h.get_secret(UPKEEP_WAKEUPS)).unwrap_or(0),
        inbox: h.get_secret(UPKEEP_INBOX).and_then(|b| b.try_into().ok()),
        inbox_len: u64_of(h.get_secret(UPKEEP_INBOX_LEN)),
        now: upkeep_now(h),
        codes: h.get_secret(UPKEEP_CODES_HASH).and_then(|b| b.try_into().ok()),
        admitted: m.map(|m| upkeep_admitted(h, m)).unwrap_or_default(),
        groups: m
            .and_then(|m| upkeep_mandate(h, m).map(|(_, ms)| ms.into_iter().filter(|x| upkeep_moved(h, m, &x.space)).map(|x| (x.space, x.epoch, x.state)).collect()))
            .unwrap_or_default(),
        stale: mine(&|m| h.get_secret(&of(UPKEEP_STALE, m))).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default(),
        // Its lines, and why the last wake-up began no re-key round (`identity_upkeep/rekey/why`).
        said: {
            let lines = mine(&|m| h.get_secret(&of(UPKEEP_SAID, m))).and_then(|b| String::from_utf8(b).ok()).unwrap_or_default();
            let why = h.get_secret(b"identity_upkeep/rekey/why").and_then(|b| String::from_utf8(b).ok()).map(|w| format!("re-key: {w}"));
            let all: Vec<String> = lines.lines().map(str::to_string).chain(why).filter(|l| !l.is_empty()).collect();
            (!all.is_empty()).then(|| all.join("\n"))
        },
    }
}

/// UPKEEP's MANDATES and what it did (see `Request::UpkeepMandate`), PER MEMBER (a household shares a node: each
/// person's page hands over their own): the members with a mandate, each one's mandate, admissions, the spaces whose
/// group it moved, when their page last ticked. The contracts' code is the node's.
pub const UPKEEP_MEMBERS: &[u8] = b"identity_upkeep/members";
pub const UPKEEP_BAG: &[u8] = b"identity_upkeep/bag";
pub const UPKEEP_TAIL: &[u8] = b"identity_upkeep/tail";
pub const UPKEEP_IDLOG: &[u8] = b"identity_upkeep/idlog";
/// The Sealed contract's code (a table's tree blocks) and the Piece contract's (a file's pieces): upkeep RE-KEYS.
pub const UPKEEP_SEALED: &[u8] = b"identity_upkeep/sealed";
pub const UPKEEP_PIECE: &[u8] = b"identity_upkeep/piece";
/// The Block contract's code HASH (a public table's tree blocks, read).
pub const UPKEEP_BLOCK: &[u8] = b"identity_upkeep/block";
pub const UPKEEP_CODES_HASH: &[u8] = b"identity_upkeep/codes";
pub const UPKEEP_MANDATE: &[u8] = b"identity_upkeep/mandate/";
pub const UPKEEP_ADMITTED: &[u8] = b"identity_upkeep/admitted/";
pub const UPKEEP_MOVED: &[u8] = b"identity_upkeep/moved/";
pub const UPKEEP_STALE: &[u8] = b"identity_upkeep/stale/";
pub const UPKEEP_SAID: &[u8] = b"identity_upkeep/said/";
/// The key packages the member's account used already (from the page, with the mandate).
pub const UPKEEP_SPENT: &[u8] = b"identity_upkeep/spent/";
pub fn upkeep_spent<H: Host>(h: &H, m: &[u8; KEY_LEN]) -> Vec<[u8; 16]> {
    h.get_secret(&of(UPKEEP_SPENT, m)).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}
/// When the member's page last handed the mandate over (the wake-up count then): a page that ticks keeps upkeep out
/// of its way.
pub const UPKEEP_TICK: &[u8] = b"identity_upkeep/tick/";
fn of(prefix: &[u8], m: &[u8; KEY_LEN]) -> Vec<u8> {
    [prefix, &m[..]].concat()
}

pub fn upkeep_codes_hash(bag: &[u8], tail: &[u8], idlog: &[u8; 32], sealed: &[u8], piece: &[u8], block: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks identity upkeep codes");
    h.update(blake3::hash(bag).as_bytes()).update(blake3::hash(tail).as_bytes()).update(idlog);
    h.update(blake3::hash(sealed).as_bytes()).update(blake3::hash(piece).as_bytes()).update(block);
    *h.finalize().as_bytes()
}
/// The members with a mandate here.
pub fn upkeep_members<H: Host>(h: &H) -> Vec<[u8; KEY_LEN]> {
    h.get_secret(UPKEEP_MEMBERS).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}
/// A member's mandate: `(their DID, spaces)`.
pub fn upkeep_mandate<H: Host>(h: &H, m: &[u8; KEY_LEN]) -> Option<(String, Vec<Mandate>)> {
    h.get_secret(&of(UPKEEP_MANDATE, m)).and_then(|b| bincode::deserialize(&b).ok())
}
pub fn upkeep_set_mandate<H: Host>(h: &mut H, m: &[u8; KEY_LEN], me: &str, spaces: &[Mandate]) -> bool {
    let mut all = upkeep_members(h);
    if !all.contains(m) {
        all.push(*m);
        if !h.set_secret(UPKEEP_MEMBERS, &bincode::serialize(&all).expect("members encode")) {
            return false;
        }
    }
    h.set_secret(&of(UPKEEP_MANDATE, m), &bincode::serialize(&(me, spaces)).expect("a mandate encodes"))
}
pub fn upkeep_admitted<H: Host>(h: &H, m: &[u8; KEY_LEN]) -> Vec<Admitted> {
    h.get_secret(&of(UPKEEP_ADMITTED, m)).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}
pub fn upkeep_set_admitted<H: Host>(h: &mut H, m: &[u8; KEY_LEN], all: &[Admitted]) -> bool {
    h.set_secret(&of(UPKEEP_ADMITTED, m), &bincode::serialize(all).expect("admissions encode"))
}
/// Whether upkeep moved this space's group (for this member) since their page last handed it over.
pub fn upkeep_moved<H: Host>(h: &H, m: &[u8; KEY_LEN], space: &[u8; 32]) -> bool {
    h.get_secret(&[UPKEEP_MOVED, &m[..], &space[..]].concat()).is_some_and(|b| b == [1])
}
pub fn upkeep_set_moved<H: Host>(h: &mut H, m: &[u8; KEY_LEN], space: &[u8; 32], moved: bool) -> bool {
    h.set_secret(&[UPKEEP_MOVED, &m[..], &space[..]].concat(), &[u8::from(moved)])
}
/// What upkeep did, SAID: the last 12 lines (each with upkeep's clock, s), newest last — a page shows them.
pub fn upkeep_say<H: Host>(h: &mut H, m: &[u8; KEY_LEN], what: &str) {
    let had = h.get_secret(&of(UPKEEP_SAID, m)).and_then(|b| String::from_utf8(b).ok()).unwrap_or_default();
    let mut lines: Vec<String> = had.lines().filter(|l| !l.is_empty()).map(str::to_string).collect();
    lines.push(format!("[{}] {what}", upkeep_now(h).unwrap_or(0)));
    let over = lines.len().saturating_sub(12);
    lines.drain(..over);
    h.set_secret(&of(UPKEEP_SAID, m), lines.join("\n").as_bytes());
}
/// Wake-ups since the member's page last handed the mandate over.
pub fn upkeep_since_tick<H: Host>(h: &H, m: &[u8; KEY_LEN]) -> u64 {
    let now = u64_of(h.get_secret(UPKEEP_WAKEUPS)).unwrap_or(0);
    now.saturating_sub(u64_of(h.get_secret(&of(UPKEEP_TICK, m))).unwrap_or(0))
}
pub fn upkeep_set_tick<H: Host>(h: &mut H, m: &[u8; KEY_LEN]) -> bool {
    let now = u64_of(h.get_secret(UPKEEP_WAKEUPS)).unwrap_or(0);
    h.set_secret(&of(UPKEEP_TICK, m), &now.to_le_bytes())
}

/// A space's EPOCH SECRET this member holds, and keeping one (upkeep moves a group while no page runs).
pub fn epoch_secret<H: Host>(h: &H, m: &[u8; KEY_LEN], space: [u8; 32], epoch: u64) -> Option<[u8; 32]> {
    secret_in(h, m, &Some(space), epoch)
}
/// The newest epoch of a space the member holds the secret of (the group may have moved past what a mandate says).
pub fn latest_epoch<H: Host>(h: &H, m: &[u8; KEY_LEN], space: [u8; 32]) -> Option<u64> {
    latest_in(h, m, &Some(space))
}
pub fn keep_epoch<H: Host>(h: &mut H, m: &[u8; KEY_LEN], space: Option<[u8; 32]>, epoch: u64, secret: &[u8; 32]) -> bool {
    let latest = latest_in(h, m, &space);
    // The secret, then the newest mark if this epoch is newer (a node joining with the words keeps the epochs it
    // walks before its group's state exists: it signs the log of the one it joins at).
    note_space(h, m, &space)
        && h.set_secret(&[in_space(EPOCH, m, &space), epoch.to_be_bytes().to_vec()].concat(), secret)
        && (latest.is_some_and(|l| epoch <= l) || h.set_secret(&in_space(EPOCH_LATEST, m, &space), &epoch.to_be_bytes()))
}

/// The address of an INVITE CODE's bag (its requests): the inbox derivation over the code's hash — no DID is 32 bytes
/// of a hash of text. `open <space id>`: an open space's.
pub fn invite_address(code: &str) -> [u8; 32] {
    use sha2::Digest;
    let h: [u8; 32] = sha2::Sha256::digest(format!("craftworks invite {}", code.trim().to_lowercase()).as_bytes()).into();
    inbox_address(&h)
}
/// The address of an account's INBOX (a bag): from its DID.
pub fn inbox_address(did: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("craftworks 2026-09-28 inbox address", did)
}
/// A WAKE-UP: counted; the inbox to read (and watch), if one is set.
pub fn upkeep_woke<H: Host>(h: &mut H) -> Option<[u8; 32]> {
    let n = u64_of(h.get_secret(UPKEEP_WAKEUPS)).unwrap_or(0) + 1;
    h.set_secret(UPKEEP_WAKEUPS, &n.to_le_bytes());
    h.get_secret(UPKEEP_INBOX).and_then(|b| b.try_into().ok())
}
/// The inbox upkeep watches, if one was handed over.
pub fn upkeep_inbox<H: Host>(h: &H) -> Option<[u8; 32]> {
    h.get_secret(UPKEEP_INBOX).and_then(|b| b.try_into().ok())
}
/// The inbox as read at a wake-up: its state's length kept (what the page reads back).
pub fn upkeep_read<H: Host>(h: &mut H, len: u64) {
    h.set_secret(UPKEEP_INBOX_LEN, &len.to_le_bytes());
}

/// RANDOMNESS for upkeep (a delegate has none of its own): a POOL the page stirs with the browser's (`UpkeepWatch`'s
/// seed), ratcheted on every draw — the next pool is stored BEFORE the draw is returned, so no draw ever repeats, even
/// across a crash. Nothing is drawn before a page has stirred it.
pub const UPKEEP_POOL: &[u8] = b"identity_upkeep/pool";
/// The CLOCK (a delegate has none either): `now ‖ wake-ups then`, the page's time when it last handed it over.
pub const UPKEEP_CLOCK: &[u8] = b"identity_upkeep/clock";
/// Wake-ups come every `UPKEEP_EVERY` seconds at the soonest (the manifest's `upkeep = 60`).
pub const UPKEEP_EVERY: u64 = 60;
pub fn upkeep_stir<H: Host>(h: &mut H, seed: &[u8; 32]) {
    let mut x = h.get_secret(UPKEEP_POOL).unwrap_or_default();
    x.extend_from_slice(seed);
    h.set_secret(UPKEEP_POOL, &blake3::derive_key("craftworks identity upkeep pool", &x));
}
pub fn upkeep_random<H: Host>(h: &mut H) -> Option<[u8; 32]> {
    let pool: [u8; 32] = h.get_secret(UPKEEP_POOL)?.try_into().ok()?;
    if !h.set_secret(UPKEEP_POOL, &blake3::derive_key("craftworks identity upkeep next", &pool)) {
        return None;
    }
    Some(blake3::derive_key("craftworks identity upkeep draw", &pool))
}
pub fn upkeep_set_clock<H: Host>(h: &mut H, now: u64) {
    let wakeups = u64_of(h.get_secret(UPKEEP_WAKEUPS)).unwrap_or(0);
    h.set_secret(UPKEEP_CLOCK, &[now.to_le_bytes(), wakeups.to_le_bytes()].concat());
}
/// Now, in seconds, ROUGHLY: the page's last time plus a minute per wake-up since. The node fires a little early at times
/// (measured: +12 s over 3 wake-ups) and not at all while it sleeps (slow): good for MLS lifetimes (days), not for
/// ordering. None before a page has handed one over.
pub fn upkeep_now<H: Host>(h: &H) -> Option<u64> {
    let c = h.get_secret(UPKEEP_CLOCK)?;
    let (then, at) = (u64_of(Some(c.get(..8)?.to_vec()))?, u64_of(Some(c.get(8..16)?.to_vec()))?);
    let wakeups = u64_of(h.get_secret(UPKEEP_WAKEUPS)).unwrap_or(0);
    Some(then + wakeups.saturating_sub(at) * UPKEEP_EVERY)
}

/// `MEMBER ‖ public key` → the member (seed, DID, PIN hash, home, data key): one secret, one write.
pub const MEMBER: &[u8] = b"identity_member/";
/// `PIN ‖ PIN hash` → the public key of the member with that PIN (empty: none).
pub const PIN: &[u8] = b"identity_pin/";
/// Wrong PINs in a row on this node (one byte).
pub const TRIES: &[u8] = b"identity_tries";
/// Set once this build holds a member (provisioned, or handed over from an earlier build): before, a PIN has nothing to
/// open here, so an unknown one is not a guess and is not counted (a new build, its members still to move in).
pub const ANY_MEMBER: &[u8] = b"identity_any_member";
/// `SESSION ‖ app contract id` → the public key of the member it opened (empty: closed).
pub const SESSION: &[u8] = b"identity_session/";
/// `GUARD ‖ params hash` → `seq ‖ value hash` of the last signature for that record.
pub const GUARD: &[u8] = b"identity_guard/";
/// `GRANT ‖ member ‖ app ‖ table` → `[1]` (empty: none).
pub const GRANT: &[u8] = b"identity_grant/";
/// `GRANTS ‖ member` → the member's grants, each `app ‖ len ‖ table` (the list the home app shows).
pub const GRANTS: &[u8] = b"identity_grants/";
/// `PENDING ‖ prompt id` → `page id ‖ member ‖ app ‖ table`: a grant waiting on the person.
pub const PENDING: &[u8] = b"identity_pending/";
/// The last prompt id used (u32).
pub const PROMPTS: &[u8] = b"identity_prompts";
/// A table's label prefix.
pub const TABLE: &[u8] = b"t/";
/// The account's CATALOG: the table listing its tables, so a page reads only tables that exist and creates the rest.
/// Written by the member's home site, or by any site the person allowed some table: listing a table is part of using
/// it.
pub const CATALOG: &str = "tables";
/// The account's MEMBERS table: every node's credentials and the removals it made, gossiped in its own feed. Read with
/// any grant (whose feeds count is part of reading any table); written by the home site only (where the group runs).
pub const MEMBERS: &str = "members";
/// The DID's MEMBER in every space other than the account (ARCHITECTURE: a DID is the member, a device only signs in):
/// its MLS key and the key its feeds are written under, both from the account's data seed — the same on every device
/// of the account. The MLS key goes to the page (MLS runs there); the feed key signs only here.
pub fn space_member_seed(data_seed: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("craftworks 2026-09-28 space member mls", data_seed)
}
pub fn space_writer(data_seed: &[u8; 32]) -> SigningKey {
    SigningKey::from_bytes(&blake3::derive_key("craftworks 2026-09-28 space member writer", data_seed))
}
/// The DID's member credential: its writer and MLS keys, signed by the account's data key.
pub fn space_member_credential(did: &[u8; 32], data_seed: &[u8; 32], mls_pub: &[u8]) -> Vec<u8> {
    credential(did, &space_writer(data_seed).verifying_key().to_bytes(), mls_pub, data_seed)
}

/// A member's CREDENTIAL (self-certifying): its account, a key it writes with, its MLS signing key, and a signature over
/// them by a key the account's key log names — `CWMB ‖ did ‖ signer ‖ writer ‖ MLS key ‖ signature`. A NODE of the
/// account: signed by an owner key, `writer` its node key. A DID's MEMBER in a space: signed by the account's DATA key,
/// `writer` the key its feeds in that space are written under. Anyone checks the signer against the key log.
const CRED: &[u8; 4] = b"CWMB";

pub fn credential(did: &[u8; 32], node: &[u8; 32], signing_pub: &[u8], owner_seed: &[u8; 32]) -> Vec<u8> {
    let owner = SigningKey::from_bytes(owner_seed);
    let sig = owner.sign(&cred_message(did, node, signing_pub)).to_bytes();
    [&CRED[..], did, &owner.verifying_key().to_bytes(), node, signing_pub, &sig].concat()
}

fn cred_message(did: &[u8; 32], node: &[u8; 32], signing_pub: &[u8]) -> Vec<u8> {
    [b"craftworks mls member".as_slice(), did, node, signing_pub].concat()
}

/// `(did, owner key, node key, signing key)` of a credential whose owner signature holds.
pub fn read_credential(b: &[u8]) -> Option<([u8; 32], [u8; 32], [u8; 32], Vec<u8>)> {
    let rest = b.strip_prefix(CRED)?;
    let (did, rest) = rest.split_at_checked(32)?;
    let (owner, rest) = rest.split_at_checked(32)?;
    let (node, rest) = rest.split_at_checked(32)?;
    let (sp, sig) = rest.split_at_checked(rest.len().checked_sub(64)?)?;
    let did: [u8; 32] = did.try_into().ok()?;
    let node: [u8; 32] = node.try_into().ok()?;
    let vk = ed25519_dalek::VerifyingKey::from_bytes(owner.try_into().ok()?).ok()?;
    ed25519_dalek::Verifier::verify(&vk, &cred_message(&did, &node, sp), &ed25519_dalek::Signature::from_slice(sig).ok()?).ok()?;
    Some((did, owner.try_into().ok()?, node, sp.to_vec()))
}

/// The account's CHANNEL: its shared tail (the account's data key) — the pointer to its group's first epoch, and its
/// commits until they move onto the epoch logs.
pub const CHANNEL: &str = "mls";
/// A table whose key comes with ANY grant of the site.
fn with_any_grant(table: &str) -> bool {
    table == CATALOG || table == MEMBERS
}

/// The key that seals a table: from the account's data key, the table's name and its generation. The same on every node
/// of the account; a new generation is a new key (what a revoked site held does not open what is written after).
/// A table's key in an MLS epoch: from the epoch's secret and the table's name. The ONE derivation (the core's `mls` uses
/// this).
pub fn epoch_table_key(epoch_secret: &[u8; 32], table: &str) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-09-28 mls table key");
    h.update(epoch_secret);
    h.update(table.as_bytes());
    *h.finalize().as_bytes()
}

/// SEALED TO A PUBLIC KEY (X25519): a one-time key (`eph_seed`, fresh randomness from the caller), the shared secret,
/// then XChaCha20-Poly1305 — `one-time public ‖ nonce ‖ sealed`. Only the holder of the secret for `public` opens it.
/// The ONE definition: the escrow of epoch secrets (to the words' key) and the inbox (to the account's inbox key).
pub fn seal_to(public: &[u8; 32], data: &[u8], eph_seed: [u8; 32]) -> Vec<u8> {
    use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
    let eph = x25519_dalek::StaticSecret::from(eph_seed);
    let eph_pub = x25519_dalek::PublicKey::from(&eph).to_bytes();
    let shared = eph.diffie_hellman(&x25519_dalek::PublicKey::from(*public));
    let key = sealed_key(shared.as_bytes(), &eph_pub, public);
    let n = blake3::keyed_hash(&key, b"nonce");
    let nonce = XNonce::from_slice(&n.as_bytes()[..24]);
    let ct = XChaCha20Poly1305::new((&key).into()).encrypt(nonce, data).expect("sealing cannot fail");
    [&eph_pub[..], &n.as_bytes()[..24], &ct].concat()
}

/// Open what was sealed to the public key of `secret_seed`.
pub fn open_with(secret_seed: &[u8; 32], blob: &[u8]) -> Option<Vec<u8>> {
    use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
    let (eph_pub, rest) = blob.split_at_checked(32)?;
    let (n, ct) = rest.split_at_checked(24)?;
    let secret = x25519_dalek::StaticSecret::from(*secret_seed);
    let public = x25519_dalek::PublicKey::from(&secret).to_bytes();
    let eph_pub: [u8; 32] = eph_pub.try_into().ok()?;
    let shared = secret.diffie_hellman(&x25519_dalek::PublicKey::from(eph_pub));
    let key = sealed_key(shared.as_bytes(), &eph_pub, &public);
    XChaCha20Poly1305::new((&key).into()).decrypt(XNonce::from_slice(n), ct).ok()
}

fn sealed_key(shared: &[u8; 32], eph_pub: &[u8; 32], public: &[u8; 32]) -> [u8; 32] {
    // (The escrow's derivation name, kept: escrows sealed before it was general still open.)
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-09-28 escrow key");
    h.update(shared);
    h.update(eph_pub);
    h.update(public);
    *h.finalize().as_bytes()
}

/// The account's INBOX key: from its data key's seed, so every node of the account holds it (in this delegate) and
/// nothing else does. Its public half is on the account's card; what is sealed to it only the account's nodes open.
pub fn inbox_seed(data_seed: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("craftworks 2026-09-28 inbox key", data_seed)
}

pub fn inbox_public(data_seed: &[u8; 32]) -> [u8; 32] {
    x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(inbox_seed(data_seed))).to_bytes()
}

/// The ADDRESS key of a space's table (where its sealed tree blocks live, and the key its tail is opened under): from the
/// space's id and the table, so every member names the same blocks. Its rows and blocks are sealed with the space's
/// epoch keys, never with this. The ONE derivation.
pub fn space_table_key(space: &[u8; 32], table: &str) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-09-28 space table key");
    h.update(space);
    h.update(table.as_bytes());
    *h.finalize().as_bytes()
}

/// The signing key of an epoch's LOG (the account's MLS commits: one tail per epoch): from the epoch's secret, so only
/// the nodes in the group at that epoch can write it — a removed node writes no later epoch's. The ONE derivation.
pub fn epoch_log_key(epoch_secret: &[u8; 32]) -> SigningKey {
    SigningKey::from_bytes(blake3::Hasher::new_derive_key("craftworks 2026-09-28 epoch log key").update(epoch_secret).finalize().as_bytes())
}

/// `MLS ‖ member` → this member's MLS state; `EPOCH ‖ member ‖ epoch (u64 BE)` → that epoch's secret;
/// `EPOCH_LATEST ‖ member` → the newest epoch held (u64 BE).
pub const MLS: &[u8] = b"identity_mls/";
pub const EPOCH: &[u8] = b"identity_epoch/";
pub const EPOCH_LATEST: &[u8] = b"identity_epoch_latest/";
/// `SPACES ‖ member` → the ids (32 bytes each) of the spaces other than the account whose group this member keeps.
pub const SPACES: &[u8] = b"identity_spaces/";

/// Where a member keeps something of a SPACE's group: the account's (`None`) under the keys it always had; another
/// space's with the space's id after the member.
fn in_space(prefix: &[u8], m: &[u8; KEY_LEN], space: &Option<[u8; 32]>) -> Vec<u8> {
    let mut k = [prefix, &m[..]].concat();
    if let Some(sp) = space {
        k.extend_from_slice(sp);
    }
    k
}

fn spaces_of<H: Host>(h: &H, m: &[u8; KEY_LEN]) -> Vec<[u8; 32]> {
    h.get_secret(&[SPACES, &m[..]].concat()).unwrap_or_default().chunks_exact(32).map(|c| c.try_into().expect("32")).collect()
}

/// Note a space among the member's (once).
fn note_space<H: Host>(h: &mut H, m: &[u8; KEY_LEN], space: &Option<[u8; 32]>) -> bool {
    let Some(sp) = space else { return true };
    let mut all = spaces_of(h, m);
    if all.contains(sp) {
        return true;
    }
    all.push(*sp);
    h.set_secret(&[SPACES, &m[..]].concat(), &all.concat())
}

fn latest_in<H: Host>(h: &H, m: &[u8; KEY_LEN], space: &Option<[u8; 32]>) -> Option<u64> {
    h.get_secret(&in_space(EPOCH_LATEST, m, space)).and_then(|b| b.try_into().ok()).map(u64::from_be_bytes)
}

fn secret_in<H: Host>(h: &H, m: &[u8; KEY_LEN], space: &Option<[u8; 32]>, epoch: u64) -> Option<[u8; 32]> {
    h.get_secret(&[in_space(EPOCH, m, space), epoch.to_be_bytes().to_vec()].concat()).and_then(|b| b.try_into().ok())
}

pub fn table_key(data_seed: &[u8; 32], table: &str, gen: u8) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-09-28 table key");
    h.update(data_seed);
    h.update(&[gen]);
    h.update(table.as_bytes());
    *h.finalize().as_bytes()
}

/// A table's BLINDED NAME (phase 4, Lifecycle): what its tail's label carries instead of the name — a keyed hash of the
/// name under the table's own key, so only who holds the key (who reads the table) can tell which table a tail is, or
/// link a space's tables by their names. `~` and 30 hex (120 bits; a table's name is at most 32 bytes).
pub fn blind_name(table_key: &[u8; 32], table: &str) -> String {
    let mut h = blake3::Hasher::new_keyed(table_key);
    h.update(b"craftworks 2026-09-30 blinded table name ");
    h.update(table.as_bytes());
    let b = h.finalize();
    format!("~{}", b.as_bytes()[..15].iter().map(|x| format!("{x:02x}")).collect::<String>())
}

fn table_ok(t: &str) -> bool {
    (1..=32).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn grant_key(member: &[u8; KEY_LEN], app: &[u8; 32], table: &str) -> Vec<u8> {
    [GRANT, &member[..], &app[..], table.as_bytes()].concat()
}

fn granted<H: Host>(h: &H, member: &[u8; KEY_LEN], app: &[u8; 32], table: &str) -> bool {
    h.get_secret(&grant_key(member, app, table)).is_some_and(|v| v == [1])
}

fn grants<H: Host>(h: &H, member: &[u8; KEY_LEN]) -> Vec<([u8; 32], String)> {
    let mut out = Vec::new();
    let Some(b) = h.get_secret(&[GRANTS, &member[..]].concat()) else { return out };
    let mut r = &b[..];
    while let Some((app, rest)) = r.split_at_checked(32) {
        let Some((&n, rest)) = rest.split_first() else { break };
        let Some((t, rest)) = rest.split_at_checked(n as usize) else { break };
        out.push((app.try_into().expect("32"), String::from_utf8_lossy(t).into_owned()));
        r = rest;
    }
    out
}

fn set_grants<H: Host>(h: &mut H, member: &[u8; KEY_LEN], list: &[([u8; 32], String)]) -> bool {
    let mut b = Vec::new();
    for (app, t) in list {
        b.extend_from_slice(app);
        b.push(t.len() as u8);
        b.extend_from_slice(t.as_bytes());
    }
    h.set_secret(&[GRANTS, &member[..]].concat(), &b)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Member {
    seed: [u8; 32],
    did: [u8; 32],
    pin: [u8; 32],
    /// The app that made it.
    home: [u8; 32],
    /// The account's data key seed, if this node holds it.
    data: Option<[u8; 32]>,
}

fn pin_hash(pin: &str) -> [u8; 32] {
    *blake3::Hasher::new().update(b"ID01-pin").update(pin.as_bytes()).finalize().as_bytes()
}

impl Member {
    /// `seed ‖ did ‖ pin hash ‖ home ‖ 0 | 1 ‖ data`.
    fn encode(&self) -> Vec<u8> {
        let mut out = [&self.seed[..], &self.did, &self.pin, &self.home].concat();
        match self.data {
            None => out.push(0),
            Some(d) => {
                out.push(1);
                out.extend_from_slice(&d);
            }
        }
        out
    }

    fn decode(b: &[u8]) -> Option<Member> {
        let (seed, rest) = b.split_at_checked(32)?;
        let (did, rest) = rest.split_at_checked(32)?;
        let (pin, rest) = rest.split_at_checked(32)?;
        let (home, rest) = rest.split_at_checked(32)?;
        let data = match rest {
            [0] => None,
            [1, d @ ..] if d.len() == 32 => Some(d.try_into().ok()?),
            _ => return None,
        };
        Some(Member {
            seed: seed.try_into().ok()?,
            did: did.try_into().ok()?,
            pin: pin.try_into().ok()?,
            home: home.try_into().ok()?,
            data,
        })
    }

    fn key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.seed)
    }

    fn public(&self) -> [u8; KEY_LEN] {
        self.key().verifying_key().to_bytes()
    }

    fn data_key(&self) -> Option<SigningKey> {
        self.data.map(|d| SigningKey::from_bytes(&d))
    }

    fn unlocked(&self) -> Answer {
        Answer::Unlocked { public: self.public(), did: self.did, data: self.data_key().map(|k| k.verifying_key().to_bytes()) }
    }
}

/// `ACCOUNT_CHECK ‖ DID` → a one-way check of the account's data key: a new member bringing that key proves the
/// account's words (they alone give it), which is what opens a node locked by wrong PINs.
pub const ACCOUNT_CHECK: &[u8] = b"identity_account_check/";
fn account_check(data: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("craftworks identity account check", data)
}
fn owns_account<H: Host>(h: &H, did: &[u8; 32], data: &[u8; 32]) -> bool {
    h.get_secret(&[ACCOUNT_CHECK, &did[..]].concat()).is_some_and(|c| c == account_check(data))
}

fn member<H: Host>(h: &H, public: &[u8]) -> Option<Member> {
    h.get_secret(&[MEMBER, public].concat()).and_then(|b| Member::decode(&b))
}

/// The member whose PIN this is.
fn by_pin<H: Host>(h: &H, pin: &[u8; 32]) -> Option<Member> {
    let public = h.get_secret(&[PIN, &pin[..]].concat()).filter(|p| p.len() == KEY_LEN)?;
    member(h, &public).filter(|a| a.pin == *pin)
}

fn tries<H: Host>(h: &H) -> u8 {
    h.get_secret(TRIES).and_then(|b| b.first().copied()).unwrap_or(0)
}

fn session_name(app: &[u8; 32]) -> Vec<u8> {
    [SESSION, &app[..]].concat()
}

/// The member `app`'s session opened, if it is still open.
fn session<H: Host>(h: &H, app: &[u8; 32]) -> Option<Member> {
    member(h, &h.get_secret(&session_name(app))?)
}

/// Open `app`'s session on `a`, and clear the try count.
fn open<H: Host>(h: &mut H, app: &[u8; 32], a: &Member) -> Answer {
    if !h.set_secret(&session_name(app), &a.public()) || !h.set_secret(TRIES, &[0]) {
        return Answer::Refused(Why::NotSaved);
    }
    a.unlocked()
}

fn pin_ok(pin: &str) -> bool {
    (6..=64).contains(&pin.chars().count())
}

/// The member a PIN opens, counting a wrong PIN as a try (before the answer leaves: a crash never gives a free
/// guess). `Err` is the answer to give instead.
fn try_pin<H: Host>(h: &mut H, pin: &str) -> Result<Member, Answer> {
    let t = tries(h);
    if t >= MAX_TRIES {
        return Err(Answer::Locked);
    }
    match by_pin(h, &pin_hash(pin)) {
        Some(a) => Ok(a),
        // Nothing held here yet: not a guess at anything (the page asks the earlier builds next).
        None if h.get_secret(ANY_MEMBER).is_none() => Err(Answer::WrongPin { tries_left: MAX_TRIES - t }),
        None => {
            if !h.set_secret(TRIES, &[t + 1]) {
                return Err(Answer::Refused(Why::NotSaved));
            }
            Err(if t + 1 >= MAX_TRIES { Answer::Locked } else { Answer::WrongPin { tries_left: MAX_TRIES - t - 1 } })
        }
    }
}

/// One request from the web app `app` (the node's attested origin), answered. Pure over the host.
pub fn serve<H: Host>(h: &mut H, req: Request, app: [u8; 32]) -> Answer {
    use Answer::*;
    match req {
        Request::Provision { seed, did, pin, data } => {
            if !pin_ok(&pin) {
                return Refused(Why::BadPin);
            }
            let data: Option<[u8; 32]> = match data.len() {
                0 => None,
                32 => Some(data.try_into().expect("32 bytes")),
                _ => return Refused(Why::BadDataKey),
            };
            let pin = pin_hash(&pin);
            let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
            let existing = member(h, &public);
            // Refusing a taken PIN tells the asker it opens someone's member, so it is a GUESS like a wrong unlock:
            // counted, and not answered at all once the node is locked — except for the account's words: a new member
            // of an account that has one here, bringing that account's data key (only its words give it).
            let t = tries(h);
            if existing.is_none() && t >= MAX_TRIES && !data.is_some_and(|d| owns_account(h, &did, &d)) {
                return Locked;
            }
            if by_pin(h, &pin).is_some_and(|a| a.public() != public) {
                if !h.set_secret(TRIES, &[t.saturating_add(1)]) {
                    return Refused(Why::NotSaved);
                }
                return Refused(Why::PinTaken);
            }
            let a = match existing {
                // The key of a member here: a new PIN, the same DID and home.
                Some(a) if a.did != did => return Refused(Why::OtherDid),
                Some(a) => {
                    // Its old PIN no longer opens it.
                    if a.pin != pin && !h.set_secret(&[PIN, &a.pin[..]].concat(), &[]) {
                        return Refused(Why::NotSaved);
                    }
                    // A key file may bring no data key; one already held stays.
                    let data = data.or(a.data);
                    Member { pin, data, ..a }
                }
                None => Member { seed, did, pin, home: app, data },
            };
            // The member before its PIN entry: a crash between leaves a member a handover can reach, never a PIN that
            // points at nothing. And the account's check: what its words prove on a locked node.
            if !h.set_secret(&[MEMBER, &public[..]].concat(), &a.encode())
                || !h.set_secret(&[PIN, &pin[..]].concat(), &public)
                || !h.set_secret(ANY_MEMBER, &[1])
                || a.data.is_some_and(|d| !h.set_secret(&[ACCOUNT_CHECK, &a.did[..]].concat(), &account_check(&d)))
            {
                return Refused(Why::NotSaved);
            }
            open(h, &app, &a)
        }
        Request::Unlock { pin } => match try_pin(h, &pin) {
            Ok(a) => open(h, &app, &a),
            Err(answer) => answer,
        },
        Request::Lock => {
            if h.set_secret(&session_name(&app), &[]) {
                LoggedOut
            } else {
                Refused(Why::NotSaved)
            }
        }
        Request::Who => match session(h, &app) {
            Some(a) => a.unlocked(),
            None => Refused(Why::NoSession),
        },
        Request::Handover { pin } => match try_pin(h, &pin) {
            Err(answer) => answer,
            // The right PIN from another app is not a handover: that app never held this member.
            Ok(a) if a.home != app => Refused(Why::NotHome),
            Ok(a) => {
                if !h.set_secret(TRIES, &[0]) {
                    return Refused(Why::NotSaved);
                }
                Handed { seed: a.seed, did: a.did, data: a.data }
            }
        },
        // A Grant that needs the person goes through `serve_bytes` (a prompt); here only its immediate answers.
        Request::Grant { tables } => match grant(h, 0, &tables, app) {
            Ok(answer) => answer,
            Err(p) => {
                // Only `serve_bytes` can ask the person; a prompt made here is dropped with its pending record.
                h.set_secret(&[PENDING, &p.id.to_le_bytes()[..]].concat(), &[]);
                Refused(Why::NotGranted { table: tables.join(",") })
            }
        },
        Request::Grants => match session(h, &app) {
            None => Refused(Why::NoSession),
            Some(a) => Grants { list: grants(h, &a.public()).into_iter().filter(|(g, _)| a.home == app || *g == app).collect() },
        },
        Request::Revoke { app: whose, table } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if a.home != app && whose != app {
                return Refused(Why::NotHome);
            }
            let member = a.public();
            let list: Vec<_> = grants(h, &member).into_iter().filter(|(g, t)| !(*g == whose && *t == table)).collect();
            if !h.set_secret(&grant_key(&member, &whose, &table), &[]) || !set_grants(h, &member, &list) {
                return Refused(Why::NotSaved);
            }
            Revoked
        }
        Request::MlsSave { space, state, epoch, secret } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if a.home != app {
                return Refused(Why::NotHome);
            }
            let m = a.public();
            let latest = latest_in(h, &m, &space);
            // Epoch secret first, then the state, then the newest mark: a crash between leaves a secret nobody points
            // at, never a state whose epoch has no secret.
            if !note_space(h, &m, &space)
                || !h.set_secret(&[in_space(EPOCH, &m, &space), epoch.to_be_bytes().to_vec()].concat(), &secret)
                || !h.set_secret(&in_space(MLS, &m, &space), &state)
                || (latest.is_none_or(|l| epoch >= l) && !h.set_secret(&in_space(EPOCH_LATEST, &m, &space), &epoch.to_be_bytes()))
            {
                return Refused(Why::NotSaved);
            }
            MlsSaved
        }
        Request::EpochKeep { space, epoch, secret } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if a.home != app {
                return Refused(Why::NotHome);
            }
            if !keep_epoch(h, &a.public(), space, epoch, &secret) {
                return Refused(Why::NotSaved);
            }
            MlsSaved
        }
        Request::Forget => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if a.home != app {
                return Refused(Why::NotHome);
            }
            let m = a.public();
            let latest = h.get_secret(&[EPOCH_LATEST, &m[..]].concat()).and_then(|b| b.try_into().ok()).map(u64::from_be_bytes);
            // The secrets first and the member last: a crash between leaves a member holding less, never keys nobody
            // can reach to forget.
            let mut ok = true;
            for e in 0..=latest.unwrap_or(0) {
                ok &= h.set_secret(&[EPOCH, &m[..], &e.to_be_bytes()].concat(), &[]);
            }
            // Every other space's group too: its state and every epoch's secret.
            for sp in spaces_of(h, &m).into_iter().map(Some) {
                for e in 0..=latest_in(h, &m, &sp).unwrap_or(0) {
                    ok &= h.set_secret(&[in_space(EPOCH, &m, &sp), e.to_be_bytes().to_vec()].concat(), &[]);
                }
                ok &= h.set_secret(&in_space(EPOCH_LATEST, &m, &sp), &[]) && h.set_secret(&in_space(MLS, &m, &sp), &[]);
            }
            ok &= h.set_secret(&[SPACES, &m[..]].concat(), &[]);
            for (g, t) in grants(h, &m) {
                ok &= h.set_secret(&grant_key(&m, &g, &t), &[]);
            }
            ok = ok
                && set_grants(h, &m, &[])
                && h.set_secret(&[EPOCH_LATEST, &m[..]].concat(), &[])
                && h.set_secret(&[MLS, &m[..]].concat(), &[])
                && h.set_secret(&[PIN, &a.pin[..]].concat(), &[])
                && h.set_secret(&[MEMBER, &m[..]].concat(), &[])
                && h.set_secret(&session_name(&app), &[]);
            if ok {
                LoggedOut
            } else {
                Refused(Why::NotSaved)
            }
        }
        Request::HandoverKeys { pin } => match try_pin(h, &pin) {
            Err(answer) => answer,
            Ok(a) if a.home != app => Refused(Why::NotHome),
            Ok(a) => {
                if !h.set_secret(TRIES, &[0]) {
                    return Refused(Why::NotSaved);
                }
                let m = a.public();
                let latest = h.get_secret(&[EPOCH_LATEST, &m[..]].concat()).and_then(|b| b.try_into().ok()).map(u64::from_be_bytes);
                let epochs = match latest {
                    None => Vec::new(),
                    Some(l) => (0..=l)
                        .filter_map(|e| Some((e, h.get_secret(&[EPOCH, &m[..], &e.to_be_bytes()].concat())?.try_into().ok()?)))
                        .collect(),
                };
                HandedKeys { mls: h.get_secret(&[MLS, &m[..]].concat()).filter(|s| !s.is_empty()), epochs }
            }
        },
        Request::HandoverSpaces { pin } => match try_pin(h, &pin) {
            Err(answer) => answer,
            Ok(a) if a.home != app => Refused(Why::NotHome),
            Ok(a) => {
                if !h.set_secret(TRIES, &[0]) {
                    return Refused(Why::NotSaved);
                }
                let m = a.public();
                let spaces = spaces_of(h, &m)
                    .into_iter()
                    .map(|id| {
                        let sp = Some(id);
                        let epochs = match latest_in(h, &m, &sp) {
                            None => Vec::new(),
                            Some(l) => (0..=l).filter_map(|e| Some((e, secret_in(h, &m, &sp, e)?))).collect(),
                        };
                        (id, h.get_secret(&in_space(MLS, &m, &sp)).filter(|s| !s.is_empty()), epochs)
                    })
                    .collect();
                HandedSpaces { spaces }
            }
        },
        Request::SpaceMember => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if a.home != app {
                return Refused(Why::NotHome);
            }
            let Some(data) = a.data else { return Refused(Why::NoDataKey) };
            let seed = space_member_seed(&data);
            let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
            SpaceMember { seed, public, credential: space_member_credential(&a.did, &data, &public) }
        }
        Request::UpkeepWatch { inbox, seed, now } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if a.home != app {
                return Refused(Why::NotHome);
            }
            h.set_secret(UPKEEP_INBOX, &inbox);
            upkeep_stir(h, &seed);
            upkeep_set_clock(h, now);
            upkeep_status(h, Some(&a.public()))
        }
        Request::UpkeepCodes { bag, tail, idlog, sealed, piece, block } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if a.home != app {
                return Refused(Why::NotHome);
            }
            let saved = h.set_secret(UPKEEP_BAG, &bag)
                && h.set_secret(UPKEEP_TAIL, &tail)
                && h.set_secret(UPKEEP_IDLOG, &idlog)
                && h.set_secret(UPKEEP_SEALED, &sealed)
                && h.set_secret(UPKEEP_PIECE, &piece)
                && h.set_secret(UPKEEP_BLOCK, &block);
            if !saved {
                return Refused(Why::NotSaved);
            }
            h.set_secret(UPKEEP_CODES_HASH, &upkeep_codes_hash(&bag, &tail, &idlog, &sealed, &piece, &block));
            upkeep_status(h, Some(&a.public()))
        }
        Request::UpkeepMandate { me, spaces, spent } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if a.home != app {
                return Refused(Why::NotHome);
            }
            let m = a.public();
            // A space whose group upkeep moved since keeps upkeep's (newer) group until the page loads it.
            let held = upkeep_mandate(h, &m).map(|(_, s)| s).unwrap_or_default();
            let mut stale = Vec::new();
            let spaces: Vec<Mandate> = spaces
                .into_iter()
                .map(|m| match held.iter().find(|x| x.space == m.space) {
                    Some(x) if x.epoch > m.epoch => {
                        stale.push(m.space);
                        x.clone()
                    }
                    _ => m,
                })
                .collect();
            if !(upkeep_set_mandate(h, &m, &me, &spaces)
                && h.set_secret(&of(UPKEEP_STALE, &m), &bincode::serialize(&stale).expect("ids encode"))
                && h.set_secret(&of(UPKEEP_SPENT, &m), &bincode::serialize(&spent).expect("tags encode"))
                && upkeep_set_tick(h, &m))
            {
                return Refused(Why::NotSaved);
            }
            upkeep_status(h, Some(&m))
        }
        Request::EpochSecrets { space, below } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if a.home != app {
                return Refused(Why::NotHome);
            }
            let m = a.public();
            let space = Some(space);
            EpochSecrets { epochs: (0..below.min(100_000)).filter_map(|e| secret_in(h, &m, &space, e).map(|s| (e, s))).collect() }
        }
        Request::UpkeepAck { admitted } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if a.home != app {
                return Refused(Why::NotHome);
            }
            let m = a.public();
            let left: Vec<Admitted> = upkeep_admitted(h, &m).into_iter().filter(|x| !admitted.iter().any(|(s, d)| *s == x.space && *d == x.did)).collect();
            for (s, _) in &admitted {
                if !left.iter().any(|x| x.space == *s) {
                    upkeep_set_moved(h, &m, s, false);
                }
            }
            if !upkeep_set_admitted(h, &m, &left) {
                return Refused(Why::NotSaved);
            }
            upkeep_status(h, Some(&m))
        }
        Request::UpkeepStatus => upkeep_status(h, session(h, &app).map(|a| a.public()).as_ref()),
        Request::InboxKey => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            let Some(data) = a.data else { return Refused(Why::NoDataKey) };
            InboxKey { public: inbox_public(&data) }
        }
        Request::InboxOpen { items } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if a.home != app {
                return Refused(Why::NotHome);
            }
            let Some(data) = a.data else { return Refused(Why::NoDataKey) };
            let seed = inbox_seed(&data);
            Opened { items: items.iter().map(|b| open_with(&seed, b)).collect() }
        }
        Request::MlsLoad { space } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if a.home != app {
                return Refused(Why::NotHome);
            }
            MlsState { state: h.get_secret(&in_space(MLS, &a.public(), &space)).filter(|s| !s.is_empty()) }
        }
        Request::TableKeyAt { table, epoch, space } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if !table_ok(&table) {
                return Refused(Why::BadTable);
            }
            let m = a.public();
            // Another space's tables: the home site (spaces are the person's, not a site's, and not granted per table).
            let may = if space.is_some() {
                a.home == app
            } else {
                a.home == app || granted(h, &m, &app, &table) || (with_any_grant(&table) && grants(h, &m).iter().any(|(g, _)| *g == app))
            };
            if !may {
                return Refused(Why::NotGranted { table });
            }
            let epoch = match epoch.or_else(|| latest_in(h, &m, &space)) {
                Some(e) => e,
                None => return Refused(Why::NoEpoch),
            };
            match secret_in(h, &m, &space, epoch) {
                Some(secret) => TableKeyAt { epoch, key: epoch_table_key(&secret, &table) },
                None => Refused(Why::NoEpoch),
            }
        }
        Request::TableKey { table, gen } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            if !table_ok(&table) {
                return Refused(Why::BadTable);
            }
            let member = a.public();
            let may = a.home == app
                || granted(h, &member, &app, &table)
                || (with_any_grant(&table) && grants(h, &member).iter().any(|(g, _)| *g == app));
            if !may {
                return Refused(Why::NotGranted { table });
            }
            let Some(data) = a.data else { return Refused(Why::NoDataKey) };
            TableKey { key: table_key(&data, &table, gen) }
        }
        Request::Sign { params, seq, value_hash, space, table: named } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            let Some(p) = Params::parse(&params) else { return Refused(Why::BadParams) };
            // One of the account's tables, by a site the person allowed: this node's own FEED of it (the member's key),
            // or the account's shared tail (its data key; the account's ordering until it moves onto the feeds).
            let own = a.key();
            // An EPOCH's log, by the home site: signed with the key of an epoch whose secret this member holds.
            let epoch_log = || -> Option<SigningKey> {
                if a.home != app {
                    return None;
                }
                let m = a.public();
                let latest = latest_in(h, &m, &space)?;
                (0..=latest).rev().find_map(|e| {
                    let secret = secret_in(h, &m, &space, e)?;
                    let k = epoch_log_key(&secret);
                    matches!(&p.authority, Authority::One(v) if *v == k.verifying_key()).then_some(k)
                })
            };
            let key = match &p.authority {
                Authority::One(v) if *v == own.verifying_key() => own,
                // A space's feed: the DID's writer (the same on every device of the account).
                Authority::One(v) if space.is_some() && a.data.is_some_and(|d| space_writer(&d).verifying_key() == *v) => space_writer(&a.data.expect("checked")),
                Authority::One(v) if a.data_key().is_some_and(|d| d.verifying_key() == *v) => a.data_key().expect("checked"),
                _ => match epoch_log() {
                    Some(k) => k,
                    None => return Refused(if a.data_key().is_none() { Why::NoDataKey } else { Why::NotThisKey }),
                },
            };
            let Some(label) = p.label.strip_prefix(TABLE).and_then(|t| std::str::from_utf8(t).ok()) else {
                return Refused(Why::NotATable);
            };
            // A BLINDED label (`~…`): the name it stands for, given, must blind to it under the table's key (a space's
            // from its id; the account's from its data key) — then the grant is the name's.
            let table: &str = if label.starts_with('~') {
                let Some(name) = named.as_deref().filter(|t| table_ok(t)) else { return Refused(Why::NotATable) };
                let tk = match space {
                    Some(sp) => space_table_key(&sp, name),
                    None => match a.data {
                        Some(d) => table_key(&d, name, 0),
                        None => return Refused(Why::NoDataKey),
                    },
                };
                if blind_name(&tk, name) != label {
                    return Refused(Why::NotATable);
                }
                name
            } else if table_ok(label) {
                label
            } else {
                return Refused(Why::NotATable);
            };
            let member = a.public();
            let may = if table == CATALOG {
                a.home == app || grants(h, &member).iter().any(|(g, _)| *g == app)
            } else if table == MEMBERS || space.is_some() {
                // The account's members, and another space's tables: the home site.
                a.home == app
            } else {
                granted(h, &member, &app, table)
            };
            if !may {
                return Refused(Why::NotGranted { table: table.into() });
            }
            match guarded(h, &p.hash, seq, &value_hash) {
                Ok(()) => Signed { sig: key.sign(&p.signed_message(false, seq, &value_hash)).to_bytes().to_vec() },
                Err(why) => Refused(why),
            }
        }
    }
}

/// THE FORK GUARD of a tail (`GUARD ‖ its params' hash` → the last step signed: `seq ‖ value hash`): a step at or below
/// it is signed only if it is that same step — never two different steps at one place, by a page or by upkeep.
fn guarded<H: Host>(h: &mut H, params_hash: &[u8], seq: u64, value_hash: &[u8]) -> Result<(), Why> {
    let guard = [GUARD, params_hash].concat();
    if let Some(g) = h.get_secret(&guard) {
        // Only this code writes a guard; one it cannot read signs nothing (u64::MAX: nothing is above it).
        let (last_seq, last_hash) = match g.split_at_checked(8) {
            Some((s, v)) if v.len() == HASH_LEN => (u64::from_le_bytes(s.try_into().unwrap_or([0xff; 8])), v),
            _ => (u64::MAX, &[][..]),
        };
        let same = seq == last_seq && last_hash == value_hash;
        if !same && seq <= last_seq {
            return Err(Why::WouldFork { last_seq });
        }
    }
    let mut g = seq.to_le_bytes().to_vec();
    g.extend_from_slice(value_hash);
    if !h.set_secret(&guard, &g) {
        return Err(Why::NotSaved);
    }
    Ok(())
}

/// UPKEEP's KEY in a space: the member's space writer (from its data seed, held here) — the public half, whose feeds
/// are the member's in every space. `None`: this node does not hold the member's data seed.
pub fn upkeep_space_writer<H: Host>(h: &H, member: &[u8; KEY_LEN]) -> Option<[u8; 32]> {
    Some(space_writer(&member_(h, member)?.data?).verifying_key().to_bytes())
}
fn member_<H: Host>(h: &H, public: &[u8]) -> Option<Member> {
    member(h, public)
}

/// UPKEEP SIGNS a step of one of the member's SPACE feeds with no page open: the member's space writer, through the
/// same fork guard as a page's signing (neither ever signs a step the other signed differently).
pub fn upkeep_sign_space<H: Host>(h: &mut H, member: &[u8; KEY_LEN], params: &[u8], seq: u64, value_hash: &[u8; 32]) -> Result<[u8; 64], String> {
    let data = member_(h, member).and_then(|a| a.data).ok_or("this node does not hold the member's data key")?;
    let key = space_writer(&data);
    let p = Params::parse(params).ok_or("not a tail's params")?;
    if !matches!(&p.authority, Authority::One(v) if *v == key.verifying_key()) {
        return Err("not one of the member's space feeds".into());
    }
    guarded(h, &p.hash, seq, value_hash).map_err(|w| format!("{w:?}"))?;
    Ok(key.sign(&p.signed_message(false, seq, value_hash)).to_bytes())
}

/// A question for the person, which the node itself shows (the delegate entry turns it into `RequestUserInput`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub id: u32,
    pub message: String,
    pub choices: Vec<String>,
}

/// What a message comes to: an answer for the page now, or a question for the person first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    Answer(Vec<u8>),
    Ask(Prompt),
}

pub const ALLOW: &str = "Allow";
pub const DENY: &str = "Don't allow";

/// A grant: answered at once (all held already, or refused), or `Err(prompt)` naming the ones not held, with the
/// request kept until the answer.
fn grant<H: Host>(h: &mut H, page_id: u32, tables: &[String], app: [u8; 32]) -> Result<Answer, Prompt> {
    let Some(a) = session(h, &app) else { return Ok(Answer::Refused(Why::NoSession)) };
    if tables.is_empty() || tables.len() > MAX_GRANT || !tables.iter().all(|t| table_ok(t)) {
        return Ok(Answer::Refused(Why::BadTable));
    }
    let member = a.public();
    let missing: Vec<String> = tables.iter().filter(|t| !granted(h, &member, &app, t)).cloned().collect();
    if missing.is_empty() {
        return Ok(Answer::Granted { tables: tables.to_vec() });
    }
    // The site the person made this member with (its home) is their own choice of app: granted without a prompt.
    if a.home == app {
        return Ok(record_grants(h, &member, app, &missing, tables));
    }
    let id = h.get_secret(PROMPTS).and_then(|b| b.try_into().ok()).map_or(0, u32::from_le_bytes).wrapping_add(1);
    let pending = [&page_id.to_le_bytes()[..], &member, &app, missing.join(",").as_bytes()].concat();
    if !h.set_secret(PROMPTS, &id.to_le_bytes()) || !h.set_secret(&[PENDING, &id.to_le_bytes()[..]].concat(), &pending) {
        return Ok(Answer::Refused(Why::NotSaved));
    }
    Err(Prompt {
        id,
        message: format!(
            "Allow this app to read and write your {} on this node? It is your account’s data, the same in every app you allow.",
            names(&missing)
        ),
        choices: vec![ALLOW.into(), DENY.into()],
    })
}

/// "“notes”", "“notes” and “pins”", "“a”, “b” and “c”".
fn names(tables: &[String]) -> String {
    let quoted: Vec<String> = tables.iter().map(|t| format!("“{t}”")).collect();
    match quoted.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// Record `missing` as granted; answer with all of `asked`.
fn record_grants<H: Host>(h: &mut H, member: &[u8; KEY_LEN], app: [u8; 32], missing: &[String], asked: &[String]) -> Answer {
    for t in missing {
        if let Answer::Refused(why) = record_grant(h, member, app, t) {
            return Answer::Refused(why);
        }
    }
    Answer::Granted { tables: asked.to_vec() }
}

fn record_grant<H: Host>(h: &mut H, member: &[u8; KEY_LEN], app: [u8; 32], table: &str) -> Answer {
    let mut list = grants(h, member);
    if !list.iter().any(|(g, t)| *g == app && t == table) {
        list.push((app, table.into()));
    }
    if h.set_secret(&grant_key(member, &app, table), &[1]) && set_grants(h, member, &list) {
        Answer::Granted { tables: vec![table.into()] }
    } else {
        Answer::Refused(Why::NotSaved)
    }
}

/// The person's answer to prompt `id`: the grant is recorded on "Allow", and the page that asked is answered (under
/// its own request id). A prompt nobody is waiting on is ignored.
pub fn serve_answer<H: Host>(h: &mut H, id: u32, choice: &[u8]) -> Option<Vec<u8>> {
    let name = [PENDING, &id.to_le_bytes()[..]].concat();
    let p = h.get_secret(&name).filter(|p| p.len() > 4 + KEY_LEN + 32)?;
    h.set_secret(&name, &[]);
    let page_id = u32::from_le_bytes(p[..4].try_into().ok()?);
    let member: [u8; KEY_LEN] = p[4..4 + KEY_LEN].try_into().ok()?;
    let app: [u8; 32] = p[4 + KEY_LEN..4 + KEY_LEN + 32].try_into().ok()?;
    let tables: Vec<String> = String::from_utf8(p[4 + KEY_LEN + 32..].to_vec()).ok()?.split(',').map(String::from).collect();
    let answer = if choice == ALLOW.as_bytes() {
        record_grants(h, &member, app, &tables, &tables)
    } else {
        Answer::Refused(Why::Denied)
    };
    Some(encode_answer(page_id, &answer))
}

/// THE GATE, then the whole message: `app` is the web app the node attests, or `None` for any caller it does not
/// (unattested, or another delegate), which is answered nothing but a refusal. An unreadable request is answered under
/// id 0.
pub fn serve_bytes<H: Host>(h: &mut H, payload: &[u8], app: Option<[u8; 32]>) -> Out {
    Out::Answer(match (decode_request(payload), app) {
        (Some((id, _)), None) => encode_answer(id, &Answer::Refused(Why::NotAttested)),
        (None, _) => encode_answer(0, &Answer::Refused(Why::Unreadable)),
        (Some((id, Request::Grant { tables })), Some(app)) => match grant(h, id, &tables, app) {
            Ok(answer) => encode_answer(id, &answer),
            Err(prompt) => return Out::Ask(prompt),
        },
        (Some((id, req)), Some(app)) => encode_answer(id, &serve(h, req, app)),
    })
}

#[cfg(test)]
mod tests;
