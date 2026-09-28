//! IDLOG: an account's KEY EVENT LOG. The account's DID is the hash of its first event, so the DID never changes while
//! the keys under it rotate: the DID is the account's "phone number", the keys are its SIM.
//!
//! Self-certifying (KERI's shape): every event is checked against the one before it and the first against the DID
//! itself, so the log is true wherever it is stored and anyone may re-publish it; nobody can forge or alter it.
//!
//! - **Inception** (seq 0): the current signing key K0, a COMMITMENT to the next key `H(K1)`, the account's data key
//!   and encryption key, and the vault. Signed by K0. `DID = id(inception)`.
//! - **Rotation** (seq n ≥ 1): names the event before it, REVEALS K_n — which must be the key the event before
//!   committed to — signs itself with K_n, and commits to `H(K_{n+1})`. So a rotation needs the pre-committed key:
//!   whoever steals the current key alone cannot rotate the account away. The data and encryption keys and the vault
//!   ride in every event; the LAST event's are current.
//!
//! Params = the DID (32 bytes): the log's address is derived from the DID, never looked up. State = `"CWIL" ‖ n(u16)
//! ‖ events`. Valid = the chain verifies from the DID. Merge = the better chain of the two: at the first event where
//! they differ, the LOWER event id wins (two different rotations at one seq are an equivocation by the committed key's
//! holder, resolved the same way everywhere); a chain that extends the other wins over its prefix. That order is total,
//! so merging is a semilattice join and every host converges.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

pub const MAGIC: &[u8; 4] = b"CWIL";
/// The most a vault may hold (a sealed 32-byte seed needs 32 + 16 + 32 + 24).
pub const VAULT_MAX: usize = 256;
/// The most events one log holds: a rotation is a rare, deliberate act.
pub const EVENTS_MAX: usize = 64;
const KEY: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub seq: u32,
    /// The id of the event before (zeros for the inception).
    pub prev: [u8; 32],
    /// This epoch's signing key (the owner key), revealed.
    pub key: [u8; 32],
    /// `commit(next key)`: the only key that may sign the next event.
    pub next: [u8; 32],
    /// The account's data key (public): its tables are addressed by it.
    pub data: [u8; 32],
    /// The account's encryption key (X25519, public): what is sealed to the account is sealed to it.
    pub enc: [u8; 32],
    /// The data key's seed, sealed to this epoch's encryption key: how words entered after a rotation reach the data.
    pub vault: Vec<u8>,
    pub sig: [u8; 64],
}

/// The commitment an event makes to the next key.
pub fn commit(next_key: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"CWID-next");
    h.update(next_key);
    *h.finalize().as_bytes()
}

impl Event {
    /// Everything but the signature, in its one encoding.
    pub fn body(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(4 + 5 * KEY + 2 + self.vault.len());
        b.extend_from_slice(&self.seq.to_le_bytes());
        for k in [&self.prev, &self.key, &self.next, &self.data, &self.enc] {
            b.extend_from_slice(k);
        }
        b.extend_from_slice(&(self.vault.len() as u16).to_le_bytes());
        b.extend_from_slice(&self.vault);
        b
    }

    /// The event's id: what the next event names, and (for the inception) the DID.
    pub fn id(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(b"CWID-id");
        h.update(&self.body());
        *h.finalize().as_bytes()
    }

    fn message(&self) -> Vec<u8> {
        [b"CWID-ev".as_slice(), &self.body()].concat()
    }

    /// Build and sign an event with `signing` (whose public half becomes `key`).
    #[allow(clippy::too_many_arguments)]
    pub fn signed(seq: u32, prev: [u8; 32], signing: &[u8; 32], next: [u8; 32], data: [u8; 32], enc: [u8; 32], vault: Vec<u8>) -> Option<Event> {
        if vault.len() > VAULT_MAX {
            return None;
        }
        let sk = SigningKey::from_bytes(signing);
        let mut e = Event { seq, prev, key: sk.verifying_key().to_bytes(), next, data, enc, vault, sig: [0; 64] };
        e.sig = sk.sign(&e.message()).to_bytes();
        Some(e)
    }

    fn signature_ok(&self) -> bool {
        match VerifyingKey::from_bytes(&self.key) {
            Ok(vk) => vk.verify_strict(&self.message(), &Signature::from_bytes(&self.sig)).is_ok(),
            Err(_) => false,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        [self.body(), self.sig.to_vec()].concat()
    }

    fn parse(b: &[u8]) -> Option<(Event, &[u8])> {
        let (seq, b) = b.split_at_checked(4)?;
        let mut keys = [[0u8; 32]; 5];
        let mut b = b;
        for k in keys.iter_mut() {
            let (x, rest) = b.split_at_checked(KEY)?;
            k.copy_from_slice(x);
            b = rest;
        }
        let (len, b) = b.split_at_checked(2)?;
        let len = u16::from_le_bytes([len[0], len[1]]) as usize;
        if len > VAULT_MAX {
            return None;
        }
        let (vault, b) = b.split_at_checked(len)?;
        let (sig, b) = b.split_at_checked(64)?;
        let [prev, key, next, data, enc] = keys;
        let e = Event {
            seq: u32::from_le_bytes([seq[0], seq[1], seq[2], seq[3]]),
            prev,
            key,
            next,
            data,
            enc,
            vault: vault.to_vec(),
            sig: sig.try_into().ok()?,
        };
        Some((e, b))
    }
}

/// A log, verified or not (see [`Log::verify`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Log {
    pub events: Vec<Event>,
}

impl Log {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(&(self.events.len() as u16).to_le_bytes());
        for e in &self.events {
            b.extend_from_slice(&e.encode());
        }
        b
    }

    /// Parse, WITHOUT verifying. Exact: trailing bytes are refused.
    pub fn parse(b: &[u8]) -> Option<Log> {
        let b = b.strip_prefix(MAGIC)?;
        let (n, mut b) = b.split_at_checked(2)?;
        let n = u16::from_le_bytes([n[0], n[1]]) as usize;
        if n == 0 || n > EVENTS_MAX {
            return None;
        }
        let mut events = Vec::with_capacity(n);
        for _ in 0..n {
            let (e, rest) = Event::parse(b)?;
            events.push(e);
            b = rest;
        }
        b.is_empty().then_some(Log { events })
    }

    /// Does this chain verify from `did`: the inception hashes to the DID and signs itself, and each later event
    /// names the one before, reveals the key it committed to, and is signed by that key.
    pub fn verify(&self, did: &[u8; 32]) -> bool {
        let Some(first) = self.events.first() else { return false };
        if first.seq != 0 || first.prev != [0; 32] || first.id() != *did || !first.signature_ok() {
            return false;
        }
        self.events.windows(2).all(|w| {
            let (a, b) = (&w[0], &w[1]);
            b.seq == a.seq + 1 && b.prev == a.id() && commit(&b.key) == a.next && b.signature_ok()
        }) && self.events.len() <= EVENTS_MAX
    }

    /// The current event: its keys are the account's now.
    pub fn head(&self) -> &Event {
        self.events.last().expect("a parsed log has at least its inception")
    }

    /// The better of two verified chains (the join): at the first difference the lower event id; else the longer.
    pub fn better(self, other: Log) -> Log {
        for (a, b) in self.events.iter().zip(&other.events) {
            let (ia, ib) = (a.id(), b.id());
            if ia != ib {
                return if ia < ib { self } else { other };
            }
        }
        if other.events.len() > self.events.len() {
            other
        } else {
            self
        }
    }
}

/// The DID a log's params are: exactly 32 bytes.
pub fn did_of_params(params: &[u8]) -> Option<[u8; 32]> {
    params.try_into().ok()
}

/// A state, parsed and verified against its params.
pub fn read(params: &[u8], state: &[u8]) -> Option<Log> {
    let did = did_of_params(params)?;
    let log = Log::parse(state)?;
    log.verify(&did).then_some(log)
}

#[cfg(feature = "freenet-main-contract")]
mod contract {
    use super::*;
    use freenet_stdlib::prelude::*;

    pub struct IdLog;

    #[contract]
    impl ContractInterface for IdLog {
        fn validate_state(
            parameters: Parameters<'static>,
            state: State<'static>,
            _related: RelatedContracts<'static>,
        ) -> Result<ValidateResult, ContractError> {
            Ok(match read(parameters.as_ref(), state.as_ref()) {
                Some(_) => ValidateResult::Valid,
                None => ValidateResult::Invalid,
            })
        }

        fn update_state(
            parameters: Parameters<'static>,
            state: State<'static>,
            data: Vec<UpdateData<'static>>,
        ) -> Result<UpdateModification<'static>, ContractError> {
            let p = parameters.as_ref();
            let mut held = read(p, state.as_ref());
            for item in data {
                let candidates: [Option<&[u8]>; 2] = match &item {
                    UpdateData::State(s) => [Some(s.as_ref()), None],
                    UpdateData::Delta(d) => [Some(d.as_ref()), None],
                    UpdateData::StateAndDelta { state, delta } => [Some(state.as_ref()), Some(delta.as_ref())],
                    _ => [None, None],
                };
                // An unreadable or unverifiable candidate is ignored, never fatal.
                for c in candidates.into_iter().flatten().filter_map(|b| read(p, b)) {
                    held = Some(match held {
                        Some(h) => h.better(c),
                        None => c,
                    });
                }
            }
            match held {
                Some(h) => Ok(UpdateModification::valid(State::from(h.encode()))),
                None => Err(ContractError::InvalidState),
            }
        }

        fn summarize_state(
            parameters: Parameters<'static>,
            state: State<'static>,
        ) -> Result<StateSummary<'static>, ContractError> {
            let log = read(parameters.as_ref(), state.as_ref()).ok_or(ContractError::InvalidState)?;
            Ok(StateSummary::from(log.head().id().to_vec()))
        }

        fn get_state_delta(
            parameters: Parameters<'static>,
            state: State<'static>,
            summary: StateSummary<'static>,
        ) -> Result<StateDelta<'static>, ContractError> {
            let log = read(parameters.as_ref(), state.as_ref()).ok_or(ContractError::InvalidState)?;
            // The same head: nothing to send. Otherwise the whole log (it is small), which the other side joins.
            if summary.as_ref() == log.head().id() {
                return Ok(StateDelta::from(Vec::new()));
            }
            Ok(StateDelta::from(log.encode()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> [u8; 32] {
        [n; 32]
    }
    fn public(n: u8) -> [u8; 32] {
        SigningKey::from_bytes(&key(n)).verifying_key().to_bytes()
    }

    /// A log whose epoch keys are key(1), key(2), …; rotation i reveals key(i+1).
    fn chain(rotations: u8) -> (Log, [u8; 32]) {
        let e0 = Event::signed(0, [0; 32], &key(1), commit(&public(2)), [7; 32], [8; 32], vec![1, 2, 3]).unwrap();
        let did = e0.id();
        let mut events = vec![e0];
        for i in 1..=rotations {
            let prev = events.last().unwrap().id();
            let e = Event::signed(i as u32, prev, &key(i + 1), commit(&public(i + 2)), [7; 32], [8 + i; 32], vec![i]).unwrap();
            events.push(e);
        }
        (Log { events }, did)
    }

    #[test]
    fn a_chain_verifies_from_its_did_and_round_trips() {
        let (log, did) = chain(3);
        assert!(log.verify(&did));
        let bytes = log.encode();
        assert_eq!(read(&did, &bytes), Some(log.clone()));
        assert_eq!(log.head().key, public(4), "the current key is the last revealed");
        // Controls: another DID, a trailing byte, a truncated state.
        assert!(read(&[9; 32], &bytes).is_none());
        assert!(read(&did, &[bytes.clone(), vec![0]].concat()).is_none());
        assert!(read(&did, &bytes[..bytes.len() - 1]).is_none());
    }

    #[test]
    fn only_the_committed_key_may_rotate() {
        let (log, did) = chain(1);
        // A rotation signed by the CURRENT key (stolen) rather than the committed next one: refused.
        let prev = log.head().id();
        let thief = Event::signed(2, prev, &key(2), commit(&public(9)), [7; 32], [8; 32], vec![]).unwrap();
        let mut bad = log.clone();
        bad.events.push(thief);
        assert!(!bad.verify(&did), "the current key alone cannot rotate");
        // The committed key may.
        let good = Event::signed(2, prev, &key(3), commit(&public(4)), [7; 32], [8; 32], vec![]).unwrap();
        let mut ok = log.clone();
        ok.events.push(good);
        assert!(ok.verify(&did));
    }

    #[test]
    fn a_changed_event_breaks_the_chain() {
        let (log, did) = chain(2);
        let mut tampered = log.clone();
        tampered.events[1].data = [0; 32]; // the signature no longer covers it
        assert!(!tampered.verify(&did));
        let mut reordered = log.clone();
        reordered.events.swap(1, 2);
        assert!(!reordered.verify(&did));
        let mut dropped = log.clone();
        dropped.events.remove(1);
        assert!(!dropped.verify(&did), "a gap breaks the chain");
    }

    #[test]
    fn the_join_is_the_same_whichever_order_it_meets_them() {
        let (long, did) = chain(3);
        let short = Log { events: long.events[..2].to_vec() };
        // A fork at seq 2: the committed key's holder signed two different rotations.
        let prev = long.events[1].id();
        let alt = Event::signed(2, prev, &key(3), commit(&public(5)), [7; 32], [99; 32], vec![]).unwrap();
        let fork = Log { events: [long.events[..2].to_vec(), vec![alt]].concat() };
        assert!(fork.verify(&did) && long.verify(&did));
        // Extension beats prefix; at a fork the lower id wins; the same in either order (commutative).
        assert_eq!(short.clone().better(long.clone()), long);
        assert_eq!(long.clone().better(short.clone()), long);
        let a = long.clone().better(fork.clone());
        let b = fork.clone().better(long.clone());
        assert_eq!(a, b);
        let winner = if long.events[2].id() < fork.events[2].id() { &long } else { &fork };
        assert_eq!(&a, winner);
        // Associative over three.
        let x = short.clone().better(long.clone()).better(fork.clone());
        let y = short.better(long.better(fork));
        assert_eq!(x, y);
    }
}
