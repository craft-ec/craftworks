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
//! - **Sign:** the Register's one signed message (`Params::signed_message`, non-terminal). A tail signs the same
//!   message over its body hash, so one verb covers both. Rules, in order:
//!   0. an open session names the member;
//!   1. the params name ONE key, and it is the member's key or the account's data key;
//!   2. ORIGIN RULE: an app signs only records whose label begins with its own contract id;
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
        Request::Sign { params, seq, value_hash } => {
            let Some(a) = session(h, &app) else { return Refused(Why::NoSession) };
            let Some(p) = Params::parse(&params) else { return Refused(Why::BadParams) };
            // The member's own key, or the account's data key: whichever one key the record names.
            let key = match &p.authority {
                Authority::One(v) if *v == a.key().verifying_key() => a.key(),
                Authority::One(v) if a.data_key().is_some_and(|d| d.verifying_key() == *v) => a.data_key().expect("checked"),
                _ => return Refused(Why::NotThisKey),
            };
            if !p.label.starts_with(&app) {
                return Refused(Why::NotYourRecord);
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

/// THE GATE, then the whole message: `app` is the web app the node attests, or `None` for any caller it does not
/// (unattested, or another delegate), which is answered nothing but a refusal. An unreadable request is answered under
/// id 0.
pub fn serve_bytes<H: Host>(h: &mut H, payload: &[u8], app: Option<[u8; 32]>) -> Vec<u8> {
    match (decode_request(payload), app) {
        (Some((id, _)), None) => encode_answer(id, &Answer::Refused(Why::NotAttested)),
        (None, _) => encode_answer(0, &Answer::Refused(Why::Unreadable)),
        (Some((id, req)), Some(app)) => encode_answer(id, &serve(h, req, app)),
    }
}

#[cfg(test)]
mod tests;
