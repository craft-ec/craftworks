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
//!   counts too: it reveals that PIN opens a member). A good unlock clears the count. A locked node is opened by a key
//!   file: `Provision` with a member's own key sets its PIN anew and clears the lock.
//! - **Sessions:** an unlock opens a session for that APP on this node, naming one member, kept here until the app
//!   logs out (a reload, another tab or another browser on the node is still logged in). One app's session is never
//!   another's. Who, Sign and Export all go through it.
//! - **Export** (the key file): the member's home app only.
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

#[cfg(feature = "freenet-main-delegate")]
mod delegate;

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
    /// Sign `params.signed_message(false, seq, value_hash)` with the session's member or the account's data key.
    Sign { params: Vec<u8>, seq: u64, value_hash: [u8; HASH_LEN] },
    /// The session member's seed, for a key file. Its home app only.
    Export,
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
}

/// What the identity answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Answer {
    /// The session is open on this member: its key, its DID (the owner's public key), and the public key of the
    /// account's data key when this node holds it.
    Unlocked { public: [u8; KEY_LEN], did: [u8; 32], data: Option<[u8; KEY_LEN]> },
    /// No member has this PIN.
    WrongPin { tries_left: u8 },
    /// Too many wrong PINs: only a key file opens this node again.
    Locked,
    /// The session is closed.
    LoggedOut,
    /// The 64-byte ed25519 signature.
    Signed { sig: Vec<u8> },
    Exported { seed: [u8; 32] },
    /// A member, handed to the next version of this delegate.
    Handed { seed: [u8; 32], did: [u8; 32], data: Option<[u8; 32]> },
    Granted { tables: Vec<String> },
    Grants { list: Vec<([u8; 32], String)> },
    Revoked,
    Refused(Why),
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
    /// A table name is 1 to 32 of a-z, 0-9, `-`, `_`; a grant names 1 to 16 of them.
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
}

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

/// `MEMBER ‖ public key` → the member (seed, DID, PIN hash, home, data key): one secret, one write.
pub const MEMBER: &[u8] = b"identity_member/";
/// `PIN ‖ PIN hash` → the public key of the member with that PIN (empty: none).
pub const PIN: &[u8] = b"identity_pin/";
/// Wrong PINs in a row on this node (one byte).
pub const TRIES: &[u8] = b"identity_tries";
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
            // counted, and not answered at all once the node is locked (only a key file gets through then).
            let t = tries(h);
            if existing.is_none() && t >= MAX_TRIES {
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
            // The member before its PIN entry: a crash between leaves a member its key file can reach, never a
            // PIN that points at nothing.
            if !h.set_secret(&[MEMBER, &public[..]].concat(), &a.encode())
                || !h.set_secret(&[PIN, &pin[..]].concat(), &public)
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
        Request::Export => match session(h, &app) {
            None => Refused(Why::NoSession),
            Some(a) if a.home != app => Refused(Why::NotHome),
            Some(a) => Exported { seed: a.seed },
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
        Request::Sign { params, seq, value_hash } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            let Some(p) = Params::parse(&params) else { return Refused(Why::BadParams) };
            // The account's data key, for one of the account's tables, by a site the person allowed.
            let Some(key) = a.data_key() else { return Refused(Why::NoDataKey) };
            if !matches!(&p.authority, Authority::One(v) if *v == key.verifying_key()) {
                return Refused(Why::NotThisKey);
            }
            let Some(table) = p.label.strip_prefix(TABLE).and_then(|t| std::str::from_utf8(t).ok()).filter(|t| table_ok(t)) else {
                return Refused(Why::NotATable);
            };
            if !granted(h, &a.public(), &app, table) {
                return Refused(Why::NotGranted { table: table.into() });
            }
            let guard = [GUARD, &p.hash[..]].concat();
            if let Some(g) = h.get_secret(&guard) {
                // Only this code writes a guard; one it cannot read signs nothing (u64::MAX: nothing is above it).
                let (last_seq, last_hash) = match g.split_at_checked(8) {
                    Some((s, v)) if v.len() == HASH_LEN => (u64::from_le_bytes(s.try_into().unwrap_or([0xff; 8])), v),
                    _ => (u64::MAX, &[][..]),
                };
                let same = seq == last_seq && last_hash == value_hash;
                if !same && seq <= last_seq {
                    return Refused(Why::WouldFork { last_seq });
                }
            }
            let mut g = seq.to_le_bytes().to_vec();
            g.extend_from_slice(&value_hash);
            if !h.set_secret(&guard, &g) {
                return Refused(Why::NotSaved);
            }
            Signed { sig: key.sign(&p.signed_message(false, seq, &value_hash)).to_bytes().to_vec() }
        }
    }
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
    if tables.is_empty() || tables.len() > 16 || !tables.iter().all(|t| table_ok(t)) {
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
