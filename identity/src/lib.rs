//! # The IDENTITY delegate (ARCHITECTURE §4)
//!
//! One per node. It holds the MEMBERS on this device and signs records for them. Nothing else holds a key: a page
//! asks, and gets a signature back. The DID is the account; a member is one device key admitted to it (the DID's
//! member Set), opened here by a PIN.
//!
//! - **Members, chosen by PIN** (a shared computer): each member is a device key, the DID it belongs to (its owner
//!   seat Register's contract id) and the app that made it (its HOME). "Log in with this device" (`Provision`) makes
//!   one, with the PIN the person sets; no two members share a PIN. Later, the same device and the same PIN
//!   (`Unlock`) open the same member, so the same DID. Several members on one device may belong to one DID or to
//!   different people's.
//! - **Wrong PINs:** five in a row, by anyone, lock PIN unlock on this device (a new member refused for a taken PIN
//!   counts too: it reveals that PIN opens a member). A good unlock clears the count. A locked
//!   device is opened by a key file: `Provision` with a member's own key sets its PIN anew and clears the lock.
//! - **Sessions:** an unlock opens a session for that APP on this device, naming one member, kept here until the app
//!   logs out (so a reload, another tab or tomorrow's visit is still logged in, as a website's is). The node attests
//!   which app asks, so one app's session is never another's. Who, Sign and Export all go through it. The person's
//!   own tools (no origin) have one session too.
//! - **Export** (the key file): the member's home app, or the person's own tools.
//! - **Recovery words:** a member made by "register with this device" (or by logging in with the words) also keeps
//!   its account's RECOVERY ENTROPY: the BIP39 words the owner key derives from. The Account page shows them
//!   (`Recovery`: home app or own tools, with a session), so the person can write them down whenever they choose.
//! - **Sign:** the Register's one signed message (`Params::signed_message`, non-terminal). A tail signs the same
//!   message over its body hash, so one verb covers both. Rules, in order:
//!   0. an open session names the member;
//!   1. the params name ONE key, and it is that member's key;
//!   2. ORIGIN RULE: an app signs only records whose label begins with its own contract id (the site it was served
//!      from); the person's own tools sign any;
//!   3. SEQ GUARD: never two different values at one seq, and never a seq below the last one signed for that record
//!      (an identical re-ask gets the same signature back);
//!   4. the guard is saved BEFORE the signature is returned (the node syncs a secret to disk before `set_secret`
//!      returns), so a crash can lose a signature nobody received, never sign twice.
//!
//! The PIN is a check this delegate makes with a try limit, never a key: whoever can read the node's secret store
//! can read the keys beside it anyway.
//!
//! [`serve`] is the whole request over a [`Host`] (just the secret store), so every rule is tested natively with a
//! map. `delegate` (the entry the node calls) is `serve` over the real context.

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
    /// "Log in with this device": a new member (a 32-byte ed25519 seed the page minted, and the DID it belongs to)
    /// under a PIN no other member on this device has. An member's own key again (a key file) sets its PIN anew.
    /// Opens a session on it.
    /// `recovery`: the account's BIP39 entropy (16 or 32 bytes), when this device holds it; empty otherwise.
    Provision { seed: [u8; 32], did: [u8; 32], pin: String, recovery: Vec<u8> },
    /// Open a session on the member this PIN belongs to.
    Unlock { pin: String },
    /// Close this app's session.
    Lock,
    /// The session's member.
    Who,
    /// Sign `params.signed_message(false, seq, value_hash)` with the session's member.
    Sign { params: Vec<u8>, seq: u64, value_hash: [u8; HASH_LEN] },
    /// The session member's seed, for a key file. Its home app, or the person's own tools.
    Export,
    /// The session member's recovery entropy (its account's words). Home app or own tools.
    Recovery,
}

/// What the identity answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Answer {
    /// The session is open on this member: its device key and its DID (the owner seat Register's contract id).
    Unlocked { public: [u8; KEY_LEN], did: [u8; 32] },
    /// No member has this PIN.
    WrongPin { tries_left: u8 },
    /// Too many wrong PINs: only a key file opens this device again.
    Locked,
    /// The session is closed.
    LoggedOut,
    /// The 64-byte ed25519 signature.
    Signed { sig: Vec<u8> },
    Exported { seed: [u8; 32] },
    Recovery { entropy: Vec<u8> },
    Refused(Why),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Why {
    /// Another member on this device has that PIN: choose another.
    PinTaken,
    /// This key's member belongs to another DID.
    OtherDid,
    /// Export asked by an app that is not the member's home.
    NotHome,
    /// This app has no open session on this device: ask for the PIN.
    NoSession,
    /// A PIN must be 6 to 64 characters.
    BadPin,
    /// Recovery entropy is 16 or 32 bytes (12 or 24 words).
    BadRecovery,
    /// This member's device does not hold its account's recovery words.
    NoRecovery,
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
    /// A delegate relaying a request: none is answered to one.
    NotForDelegates,
    /// Not a request this identity reads.
    Unreadable,
}

/// Wrong PINs in a row allowed before PIN unlock is locked on this device.
pub const MAX_TRIES: u8 = 5;

/// Who asked, as the node attests it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// No origin: the person's own tools.
    Local,
    /// A web app, by the contract it was served from.
    App([u8; 32]),
    /// Another delegate.
    Delegate,
}

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

/// `MEMBER ‖ public key` → the member (seed, DID, PIN hash, home): one secret, one write.
pub const MEMBER: &[u8] = b"identity_member/";
/// `PIN ‖ PIN hash` → the public key of the member with that PIN (empty: none).
pub const PIN: &[u8] = b"identity_pin/";
/// Wrong PINs in a row on this device (one byte).
pub const TRIES: &[u8] = b"identity_tries";
/// `SESSION ‖ app contract id` (all zero: own tools) → the public key of the member it opened (empty: closed).
pub const SESSION: &[u8] = b"identity_session/";
/// `GUARD ‖ params hash` → `seq ‖ value hash` of the last signature for that record.
pub const GUARD: &[u8] = b"identity_guard/";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Member {
    seed: [u8; 32],
    did: [u8; 32],
    pin: [u8; 32],
    /// `None`: made by the person's own tools.
    home: Option<[u8; 32]>,
    /// The account's BIP39 entropy, if this device holds it (empty: not held).
    recovery: Vec<u8>,
}

fn pin_hash(pin: &str) -> [u8; 32] {
    *blake3::Hasher::new().update(b"ID01-pin").update(pin.as_bytes()).finalize().as_bytes()
}

impl Member {
    fn encode(&self) -> Vec<u8> {
        let mut out = [&self.seed[..], &self.did, &self.pin].concat();
        match self.home {
            None => out.push(0),
            Some(h) => {
                out.push(1);
                out.extend_from_slice(&h);
            }
        }
        out.push(self.recovery.len() as u8);
        out.extend_from_slice(&self.recovery);
        out
    }

    fn decode(b: &[u8]) -> Option<Member> {
        let (seed, rest) = b.split_at_checked(32)?;
        let (did, rest) = rest.split_at_checked(32)?;
        let (pin, rest) = rest.split_at_checked(32)?;
        let (home, rest) = match rest.split_first()? {
            (0, r) => (None, r),
            (1, r) => {
                let (h, r) = r.split_at_checked(32)?;
                (Some(h.try_into().ok()?), r)
            }
            _ => return None,
        };
        let (&n, recovery) = rest.split_first()?;
        if recovery.len() != n as usize {
            return None;
        }
        Some(Member {
            seed: seed.try_into().ok()?,
            did: did.try_into().ok()?,
            pin: pin.try_into().ok()?,
            home,
            recovery: recovery.to_vec(),
        })
    }

    fn key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.seed)
    }

    fn public(&self) -> [u8; KEY_LEN] {
        self.key().verifying_key().to_bytes()
    }

    fn unlocked(&self) -> Answer {
        Answer::Unlocked { public: self.public(), did: self.did }
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

fn session_name(origin: Origin) -> Option<Vec<u8>> {
    match origin {
        Origin::Local => Some([SESSION, &[0; 32][..]].concat()),
        Origin::App(id) => Some([SESSION, &id[..]].concat()),
        Origin::Delegate => None,
    }
}

/// The member `origin`'s session opened, if it is still open.
fn session<H: Host>(h: &H, origin: Origin) -> Option<Member> {
    let public = h.get_secret(&session_name(origin)?)?;
    member(h, &public)
}

/// Open `origin`'s session on `a`, and clear the try count.
fn open<H: Host>(h: &mut H, origin: Origin, a: &Member) -> Answer {
    let Some(name) = session_name(origin) else { return Answer::Refused(Why::NotForDelegates) };
    if !h.set_secret(&name, &a.public()) || !h.set_secret(TRIES, &[0]) {
        return Answer::Refused(Why::NotSaved);
    }
    a.unlocked()
}

fn pin_ok(pin: &str) -> bool {
    (6..=64).contains(&pin.chars().count())
}

fn origin_id(origin: Origin) -> Option<[u8; 32]> {
    match origin {
        Origin::App(id) => Some(id),
        _ => None,
    }
}

/// One request, answered. Pure over the host.
pub fn serve<H: Host>(h: &mut H, req: Request, origin: Origin) -> Answer {
    use Answer::*;
    if origin == Origin::Delegate {
        return Refused(Why::NotForDelegates);
    }
    match req {
        Request::Provision { seed, did, pin, recovery } => {
            if !pin_ok(&pin) {
                return Refused(Why::BadPin);
            }
            if !matches!(recovery.len(), 0 | 16 | 32) {
                return Refused(Why::BadRecovery);
            }
            let pin = pin_hash(&pin);
            let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
            let existing = member(h, &public);
            // Refusing a taken PIN tells the asker it opens someone's member, so it is a GUESS like a wrong unlock:
            // counted, and not answered at all once the device is locked (only a key file gets through then).
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
                // The key file of a member here: a new PIN, the same DID and home.
                Some(a) if a.did != did => return Refused(Why::OtherDid),
                Some(a) => {
                    // Its old PIN no longer opens it.
                    if a.pin != pin && !h.set_secret(&[PIN, &a.pin[..]].concat(), &[]) {
                        return Refused(Why::NotSaved);
                    }
                    // A key file brings no words; words already held stay.
                    let recovery = if recovery.is_empty() { a.recovery.clone() } else { recovery };
                    Member { pin, recovery, ..a }
                }
                None => Member { seed, did, pin, home: origin_id(origin), recovery },
            };
            // The member before its PIN entry: a crash between leaves a member its key file can reach, never a
            // PIN that points at nothing.
            if !h.set_secret(&[MEMBER, &public[..]].concat(), &a.encode())
                || !h.set_secret(&[PIN, &pin[..]].concat(), &public)
            {
                return Refused(Why::NotSaved);
            }
            open(h, origin, &a)
        }
        Request::Unlock { pin } => {
            let t = tries(h);
            if t >= MAX_TRIES {
                return Locked;
            }
            match by_pin(h, &pin_hash(&pin)) {
                Some(a) => open(h, origin, &a),
                None => {
                    // The try is counted before the answer leaves, or a crash would give a free guess.
                    if !h.set_secret(TRIES, &[t + 1]) {
                        return Refused(Why::NotSaved);
                    }
                    if t + 1 >= MAX_TRIES {
                        Locked
                    } else {
                        WrongPin { tries_left: MAX_TRIES - t - 1 }
                    }
                }
            }
        }
        Request::Lock => match session_name(origin) {
            Some(name) if h.set_secret(&name, &[]) => LoggedOut,
            _ => Refused(Why::NotSaved),
        },
        Request::Who => match session(h, origin) {
            Some(a) => a.unlocked(),
            None => Refused(Why::NoSession),
        },
        Request::Export => match session(h, origin) {
            None => Refused(Why::NoSession),
            Some(a) if origin != Origin::Local && a.home != origin_id(origin) => Refused(Why::NotHome),
            Some(a) => Exported { seed: a.seed },
        },
        Request::Recovery => match session(h, origin) {
            None => Refused(Why::NoSession),
            Some(a) if origin != Origin::Local && a.home != origin_id(origin) => Refused(Why::NotHome),
            Some(a) if a.recovery.is_empty() => Refused(Why::NoRecovery),
            Some(a) => Recovery { entropy: a.recovery },
        },
        Request::Sign { params, seq, value_hash } => {
            let Some(a) = session(h, origin) else { return Refused(Why::NoSession) };
            let Some(p) = Params::parse(&params) else { return Refused(Why::BadParams) };
            let key = a.key();
            match &p.authority {
                Authority::One(v) if *v == key.verifying_key() => {}
                _ => return Refused(Why::NotThisKey),
            }
            if let Origin::App(id) = origin {
                if !p.label.starts_with(&id) {
                    return Refused(Why::NotYourRecord);
                }
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

/// The whole message: decode, serve, encode. An unreadable request is answered under id 0.
pub fn serve_bytes<H: Host>(h: &mut H, payload: &[u8], origin: Origin) -> Vec<u8> {
    match decode_request(payload) {
        Some((id, req)) => encode_answer(id, &serve(h, req, origin)),
        None => encode_answer(0, &Answer::Refused(Why::Unreadable)),
    }
}

#[cfg(test)]
mod tests;
