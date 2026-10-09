//! The head, as a Register record.
//!
//! The engine's head is `(seq, root)`, and a Register holds one signed value
//! per writer — so the head IS a Register record, and writing it means
//! signing one. This builds and signs that record.
//!
//! **The format is the Register crate's, not this file's** (sdk#364): the
//! params are parsed by its `Params::parse`, the message a writer signs is its
//! `Params::signed_message`, and the state is its `RegState::encode` — the
//! code every node validates with. Nothing here lays out an RG01 byte. (It was
//! a hand-written copy pinned by known-answer vectors; since the crate became a
//! stdlib-free library, freenet-contracts#53, the copy is gone.)
//!
//! Mode 0 only — a single writer key, which is what a device's own head is.
//! A k-of-n keyset head is a Phase 6 question (several devices), and this
//! refuses rather than guesses at it.

use craftec_register_contract::wire::{Authority, Params, RegState, Record, Signed};
use ed25519_dalek::{Signer, SigningKey};

/// The Register contract's cap on a record's value (the contract's number: `signer_proto::head`).
use signer_proto::head::VALUE_MAX as MAX_VALUE;

/// Why a head could not be built.
#[derive(Debug, PartialEq, Eq)]
pub enum HeadError {
    /// The params are not a mode-0 Register's.
    ///
    /// Refused rather than guessed: mode 1 is a k-of-n keyset, and signing
    /// one slot of a quorum with a device key would produce a record that
    /// looks valid and decides nothing.
    NotSingleWriter,
    /// The signing key is not 32 bytes.
    BadKey,
    /// The value is over the Register's cap.
    ValueTooLong,
    /// The params name a key this delegate cannot sign for.
    ///
    /// Caught HERE rather than by the node: a record signed by the wrong key
    /// is refused on arrival, and the commit would then wait on a head that
    /// is never going to land.
    WrongWriter,
}

/// The writer key a mode-0 Register is created under: its params, parsed by the Register crate.
fn writer_key(params: &[u8]) -> Result<[u8; 32], HeadError> {
    match Params::parse(params).map(|p| p.authority) {
        Some(Authority::One(k)) => Ok(k.to_bytes()),
        _ => Err(HeadError::NotSingleWriter),
    }
}

/// Build and sign the state that says "the head is `value` at `seq`": the Register crate's own `Record`, signed over
/// its own `signed_message`, encoded by its own `RegState::encode` -- so it is byte-for-byte what the contract parses
/// (the round-trip test in tests/register.rs reads it back with the crate's `read`, which VERIFIES it).
pub fn head_state(
    params: &[u8],
    signing_key: &[u8],
    seq: u64,
    value: &[u8],
) -> Result<Vec<u8>, HeadError> {
    if value.len() > MAX_VALUE {
        return Err(HeadError::ValueTooLong);
    }
    let key: [u8; 32] = signing_key.try_into().map_err(|_| HeadError::BadKey)?;
    let sk = SigningKey::from_bytes(&key);
    if sk.verifying_key().to_bytes() != writer_key(params)? {
        return Err(HeadError::WrongWriter);
    }
    let p = Params::parse(params).ok_or(HeadError::NotSingleWriter)?;
    let value_hash: [u8; 32] = *blake3::hash(value).as_bytes();
    // `terminal` is false: a head is superseded by the next head, never final.
    let sig = sk.sign(&p.signed_message(false, seq, &value_hash)).to_bytes();
    let signed = Signed { terminal: false, seq, value_hash, bitmap: 0, sigs: vec![sig] };
    let record = Record::new(signed, value.to_vec()).ok_or(HeadError::ValueTooLong)?;
    Ok(RegState { record: Some(record), evidence: None }.encode(&p.authority))
}

/// Read `(seq, root)` out of a Register state, TOLERANTLY: the root is the value's first 32 bytes whatever ledger
/// follows (signer_proto::head, the one value format). The record is read by the Register crate
/// (`signer_proto::head::record_of`). The seq is taken from the RECORD, never assumed to be the one that was written:
/// a different seq there is another writer's head, which is a conflict and not a confirmation. This does NOT verify
/// the signature -- the contract did that before the node stored it.
pub fn head_of(state: &[u8]) -> Option<(u64, [u8; 32])> {
    let (seq, value) = signer_proto::head::record_of(state)?;
    Some((seq, signer_proto::head::read_value(value)?.root))
}
