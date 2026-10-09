//! The SIGNER's wire types (sdk#209), shared by the delegate (`signer`) and the page (`wire::signer`).
//!
//! A crate of its own so the page can speak to the signer without linking the signer: the delegate needs
//! freenet-stdlib's guest side, and the client-API boundary gate keeps freenet-stdlib out of the SDK core. This is
//! serde and bincode, nothing else.
//!
//! Every message starts with [`MAGIC`], `SG02`: a node's delegate reply does not say WHICH delegate sent it once
//! `wire::unframe` has classified it, so the page tells a signer answer from an engine reply by its first four bytes.
//! The `02` is the version; a different one is not read.
//!
//! # Every answer names its request (SG02)
//!
//! After the magic comes the request's `id` (u32), and the signer ECHOES it on the answer. An answer's variant does
//! not say which request it answers: `Held{present}` names no contract, `Refused(NotProvisioned)` answers a sign or a
//! provision. With two requests in flight only the id attributes an answer -- a pacing rule (one request of each kind
//! at a time) would stand in for an identity. The page's ids are its own; `0` is [`UNATTRIBUTED`], never a page's:
//! the answer to a request whose id could not be read.

use serde::{Deserialize, Serialize};

pub mod head;

/// `SG02`: the signer's messages, version 2 (an echoed request id).
pub const MAGIC: [u8; 4] = *b"SG02";

/// The id no page request carries: an answer that cannot be attributed to one (see the crate docs).
pub const UNATTRIBUTED: u32 = 0;

/// Most contracts one `Held` may ask about.
pub const MAX_HELD: usize = 128;

/// A head: `(seq, root)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Head {
    pub seq: u64,
    pub root: [u8; 32],
}

/// The head that follows `prev`: `seq` MUST be `prev.seq + 1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Next {
    pub seq: u64,
    pub root: [u8; 32],
    /// WRITE-PATH's ledger (opaque until step 2 fixes its format). Signed as part of the value: `root` alone when
    /// empty -- exactly what every existing head reader accepts -- else `root ‖ ledger`.
    pub ledger: Vec<u8>,
}

impl Next {
    pub fn head(&self) -> Head {
        Head {
            seq: self.seq,
            root: self.root,
        }
    }
    /// The Register value this head is signed as.
    pub fn value(&self) -> Vec<u8> {
        let mut v = self.root.to_vec();
        v.extend_from_slice(&self.ledger);
        v
    }
}

/// Why a request is refused outright, rather than answered with a fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Why {
    NotSuccessor,
    NotProvisioned,
    HeadUnknown,
    RootNotHeld,
    RecordNotSaved,
    KeyAlreadyProvisioned,
    CannotSign,
    Unreadable,
    Forked {
        mine: Head,
        read: Head,
    },
    RegisterChanged,
    /// A `Held` with none, or more than [`MAX_HELD`].
    BlockCount {
        max: u32,
        got: u32,
    },
    /// The value to sign carries a ledger that is not the format ([`head::check`]): nothing is signed, because a
    /// malformed ledger, once signed, degrades every reader's merge for that head's life and nobody is told.
    BadLedger,
    /// A SITE's signature was asked for by a web app the node attests (builder#117, rule 13): a site is signed only
    /// for the person's own tools, never for a served app -- which could otherwise republish its visitor's site.
    FromApp,
    /// The label names no site this signer can sign: its app id is not one (`core_types::name::app_ok`).
    BadLabel,
}

impl Why {
    /// Is this refusal "NOT YET" -- the same ask can succeed later, so it is asked again on a backoff -- rather than
    /// final? THE ONE retryable set (sdk#486), read by every label's answer (head, landing, site): exhaustive, so a new
    /// refusal does not compile until it is placed.
    /// * `RootNotHeld`: the root's PUT is still landing. (The signer says it only for a label whose root is a block --
    ///   a site's is not, so it never refuses a site so -- but "not yet held" is retryable for any label.)
    /// * `HeadUnknown`: the node does not hold the Register yet; a read of it makes it held.
    /// * `RecordNotSaved`: the signer could not write its record, and signed nothing.
    pub fn retryable(&self) -> bool {
        match self {
            Why::RootNotHeld | Why::HeadUnknown | Why::RecordNotSaved => true,
            Why::NotSuccessor
            | Why::NotProvisioned
            | Why::KeyAlreadyProvisioned
            | Why::CannotSign
            | Why::Unreadable
            | Why::Forked { .. }
            | Why::RegisterChanged
            | Why::BlockCount { .. }
            | Why::BadLedger
            | Why::FromApp
            | Why::BadLabel => false,
        }
    }
}

/// WHICH record a signature is for (builder#117): the person's data HEAD, or one of their SITES -- a published app's
/// one stable address. ONE sign verb and ONE rule (`signer::decide`) for both; the label chooses the params, the
/// record kept, and the contract read as the truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Label {
    Head,
    /// `app`'s site. `contract` is the site contract's id, which the page derives (the signer holds no site code):
    /// the signer READS it only after verifying what it holds under the site's own params, so a wrong id can do no
    /// more than raise the version it signs from (architect, builder#117 Q1).
    Site { app: String, contract: [u8; 32] },
    /// The person's OBSERVATION tree's head (craftworks-docs OBSERVABILITY §1, sdk#399): the same authority as `Head`,
    /// under the fixed name `obs` (`wire::OBS_NAME`), so anyone who knows an identity can compute where its recordings
    /// are. Signed by the same rule as `Head`, into its own record.
    Obs,
}

/// What the signer answers. See `signer::decide` for the rule behind `Signed` / `NotNext` / `AlreadySigned`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Answer {
    /// The Register state bytes to UPDATE; the record is saved.
    Signed(Vec<u8>),
    /// `prev` is not the current head: rebase onto `current`.
    NotNext {
        current: Head,
    },
    /// `prev` was already signed from: the FIRST record's bytes.
    AlreadySigned(Vec<u8>),
    Provisioned,
    /// READ-LOCAL: for each contract asked, in order, whether THIS node holds its state.
    Held {
        present: Vec<bool>,
    },
    Refused(Why),
    /// The Register this signer signs for — its params — or `None` when it holds no key (never provisioned). Appended
    /// LAST, so every earlier answer keeps its encoding.
    Register {
        params: Option<Vec<u8>>,
    },
}

/// What the page asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Sign {
        prev: Head,
        next: Next,
        /// Which record (builder#117): the head, or a site.
        label: Label,
    },
    Provision {
        signing_key: Vec<u8>,
        register_code: Vec<u8>,
        register_params: Vec<u8>,
        block_code: Vec<u8>,
    },
    /// READ-LOCAL: does this node hold these contracts' state? The signer's synchronous local read, per contract --
    /// never a network fetch, so it cannot park. 1..=[`MAX_HELD`] contracts.
    Held {
        contracts: Vec<[u8; 32]>,
    },
    /// WHICH REGISTER DO YOU SIGN FOR? A page that opens must reopen the person's own tree, not mint a new identity
    /// on every load: this is how it learns whether the signer already holds a key, and for which Register. Needs
    /// nothing; answers [`Answer::Register`]. The key itself never leaves the signer. Appended LAST, so every earlier
    /// request keeps its encoding.
    Register,
}

fn encode<T: Serialize>(t: &T) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend(bincode::serialize(t).expect("a signer message encodes"));
    out
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8], limit: u64) -> Option<T> {
    use bincode::Options;
    let rest = bytes.strip_prefix(&MAGIC)?;
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
        .with_limit(limit)
        .deserialize(rest)
        .ok()
}

/// Request `id` (a page's own, never [`UNATTRIBUTED`]); the answer carries it back.
pub fn encode_request(id: u32, r: &Request) -> Vec<u8> {
    encode(&(id, r))
}
/// An answer to request `id`.
pub fn encode_answer(id: u32, a: &Answer) -> Vec<u8> {
    encode(&(id, a))
}
/// `(id, request)`; `None` for anything that is not a signer request of this version, or does not decode EXACTLY.
pub fn decode_request(bytes: &[u8]) -> Option<(u32, Request)> {
    decode(bytes, 8 * 1024 * 1024)
}
/// The id of a request that does not decode, so its `Unreadable` still names it: the four bytes after the magic,
/// [`UNATTRIBUTED`] when there are not four, or no magic.
pub fn request_id(bytes: &[u8]) -> u32 {
    match bytes.strip_prefix(&MAGIC).and_then(|r| r.get(..4)) {
        Some(id) => u32::from_le_bytes([id[0], id[1], id[2], id[3]]),
        None => UNATTRIBUTED,
    }
}
/// `(id, answer)`; `None` for anything that is not a signer answer of this version -- an engine reply included -- or does not decode
/// EXACTLY.
pub fn decode_answer(bytes: &[u8]) -> Option<(u32, Answer)> {
    decode(bytes, 1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_round_trips_and_carries_the_magic() {
        let reqs = [
            Request::Sign {
                prev: Head {
                    seq: 3,
                    root: [1; 32],
                },
                next: Next {
                    seq: 4,
                    root: [2; 32],
                    ledger: vec![9],
                },
                label: Label::Site { app: "notes".into(), contract: [7; 32] },
            },
            Request::Provision {
                signing_key: vec![1],
                register_code: vec![2],
                register_params: vec![3],
                block_code: vec![4],
            },
            Request::Held {
                contracts: vec![[6; 32], [7; 32]],
            },
        ];
        for (id, r) in (1..).zip(reqs) {
            let b = encode_request(id, &r);
            assert_eq!(&b[..4], b"SG02");
            assert_eq!(request_id(&b), id);
            assert_eq!(decode_request(&b), Some((id, r)));
        }
        let a = Answer::Register { params: Some(vec![5]) };
        assert_eq!(decode_answer(&encode_answer(7, &a)), Some((7, a)));
        let h = Answer::Held {
            present: vec![true, false],
        };
        assert_eq!(
            decode_answer(&encode_answer(u32::MAX, &h)),
            Some((u32::MAX, h))
        );
    }

    #[test]
    fn what_is_not_a_signer_message_is_not_read() {
        let good = encode_answer(3, &Answer::Provisioned);
        assert_eq!(
            decode_answer(&good[1..]),
            None,
            "a message without its magic was read"
        );
        let mut v2 = good.clone();
        v2[3] = b'1';
        assert_eq!(decode_answer(&v2), None, "version 1 was read as version 2");
        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(
            decode_answer(&trailing),
            None,
            "trailing bytes were accepted"
        );
        assert_eq!(decode_answer(&[]), None);
        assert_eq!(
            request_id(b"SG02\x05\x00"),
            UNATTRIBUTED,
            "a short id was read"
        );
        assert_eq!(
            request_id(b"SG01\x05\x00\x00\x00"),
            UNATTRIBUTED,
            "another version's id was read"
        );
        assert_eq!(request_id(b"SG02\x05\x00\x00\x00garbage"), 5);
        assert_eq!(
            decode_answer(&[0x04, 0, 0, 0, 1]),
            None,
            "an engine reply was read as a signer answer"
        );
    }
}
