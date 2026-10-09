//! Framing for freenet's client API — the one place that knows it.
//!
//! Everything here is a pure function over freenet's OWN types. Nothing is
//! reimplemented: a format we restate is a format that drifts, and the
//! interesting failure would be the drift nobody notices because most
//! variants still decode.
//!
//! # Why a crate of its own
//!
//! The SDK core is sans-IO and testable with no node, and its boundary gate
//! keeps `freenet-stdlib` out of it on purpose. Putting the framing HERE keeps
//! that true, keeps every decision in Rust — `unframe` is a dispatch, and a
//! dispatch in JavaScript is a decision nothing tests — and means that
//! spanning two node versions later is two implementations behind one
//! interface rather than a refactor.
//!
//! # `unframe` is a trust boundary
//!
//! **A gateway is not the user's node.** Everything that arrives here was
//! chosen by something else, so:
//!
//! * sizes and counts are bounded BEFORE anything is allocated;
//! * garbage, truncation and a wrong variant are answers, never panics —
//!   this compiles to wasm with `panic = abort`, where a panic is a dead
//!   instance rather than an error an app can catch;
//! * [`Incoming::Unusable`] is COUNTED by reason, because a client that
//!   silently ignores what it cannot read waits for ever for an answer to a
//!   message nobody understood.
//!
//! And two rules that are about what the caller may DO with what comes back,
//! stated here because this is where the temptation is:
//!
//! * [`Incoming::HeadChanged`] is a HINT. A node can fabricate, replay or
//!   suppress it. Fabrication is contained because the data is
//!   content-addressed and must verify — so it is a confusion and denial
//!   vector, not corruption — but **a root supplied by the node is never
//!   authoritative until data verifies against it.** Its FULL state, when it
//!   carries one, may do exactly one more thing (sdk#378 P3, the architect's
//!   conditions): CONFIRM the head this page itself owes, judged by the page's
//!   one read-back rule — never adopt anything, and a dropped push leaves the
//!   read-back GET to do its job on its deadline.
//! * [`Refused`] carries a reason the NODE chose. It may drive at most a
//!   bounded retry, and **it may never lead to a credential prompt.**
//!   "Refused: bad key → ask the person to re-enter their passphrase" is a
//!   phishing channel delivered through the protocol.

//! # The node's version is not on this wire
//!
//! Checked at the deciding lines of `freenet-stdlib` 0.10.0, because the right
//! response to a version skew is to REFUSE rather than to misread — and
//! refusing requires knowing.
//!
//! * `HostResponse` (`client_api/client_events.rs:771`) — `ContractResponse`,
//!   `DelegateResponse`, `QueryResponse`, `Ok`, `StreamChunk`, `StreamHeader`.
//!   No version.
//! * `QueryResponse` (`:812`) — `ConnectedPeers`, `NetworkDebug`,
//!   `NodeDiagnostics`, `NeighborHosting`. No version.
//! * `NodeInfo` (`:856`), the richest node-describing struct the API has —
//!   `peer_id`, `is_gateway`, `location`, `listening_address`,
//!   `uptime_seconds`. **No version field.**
//! * `NodeQuery` (`:901`) — nothing that asks for one.
//!
//! **So there is nothing to detect and nothing to refuse on, and no
//! negotiation is designed here.** The binary reports its version on its own
//! command line (`freenet --version`), which a page cannot reach.
//!
//! What follows for us: the version this build was written against is pinned
//! in `Cargo.toml` and belongs in the versions panel and the diagnostic bundle
//! as ENVIRONMENT, not as something checked at runtime. And the dangerous
//! skew is named so nobody later infers safety from silence: **a bump where
//! most variants still decode and one has changed** would present as ordinary
//! traffic with one thing quietly wrong. Compatibility must never be inferred
//! from "the first message parsed".

#![forbid(unsafe_code)]

use freenet_stdlib::client_api::{ClientRequest, DelegateRequest, HostResponse};
use freenet_stdlib::prelude::*;

pub mod block;
pub mod puts;
pub mod signer;
pub mod webapp;
pub mod reassemble;
/// The delegate's identity, so a caller can hold one without depending on
/// freenet itself. Opaque everywhere outside this crate.
pub use freenet_stdlib::prelude::DelegateKey;
pub use reassemble::Reassembler;

/// The largest single frame this build will decode before looking inside.
///
/// A bound BEFORE the decoder sees anything, so a length field cannot ask for
/// an allocation the sender chooses. It is the outermost door.
pub const MAX_FRAME: usize = 4 * 1024 * 1024;

/// The largest CHUNK this build will hold.
///
/// stdlib's own `CHUNK_SIZE`: a chunk larger than the size the sender's own
/// chunker produces is not a chunk, it is somebody claiming to be one. The
/// first version bounded a chunk by [`MAX_FRAME`] instead, which is sixteen
/// times larger — and that, with an eviction rule that could never fire for a
/// single stream, let one sender hold about a gigabyte in a browser tab.
pub const MAX_CHUNK: usize = freenet_stdlib::client_api::streaming::CHUNK_SIZE;

/// The largest message this build will REASSEMBLE -- and DECODE, once reassembled: ONE bound for a whole message.
///
/// stdlib's own cap, 256 chunks of its chunk size (64 MiB), sized for freenet's 50 MiB `MAX_STATE_SIZE`: a node
/// never sends more, so a message it may send is one this page reads. It used to be 8 MiB ("this client never asks
/// for a contract state"), and a reassembled message was then refused by the 4 MiB FRAME bound on decode -- but a
/// page GETs contract states now: the builder's 5.9 MB site read back as `TooLarge`, discarded, and re-asked for ever
/// (engineer1, from the page's own lines). A single FRAME keeps its own door ([`MAX_FRAME`]): a node chunks anything
/// larger.
pub const MAX_REASSEMBLED: usize = freenet_stdlib::client_api::streaming::MAX_TOTAL_CHUNKS as usize * MAX_CHUNK;

/// Room in a whole message for a GET answer's envelope: its tags, its key and the state's length prefix (a GET's
/// answer carries no contract code). Pinned against the stdlib's encoding by a test.
pub const READ_ENVELOPE: usize = 64 * 1024;

/// The largest contract STATE a GET's answer can carry and still be read ([`MAX_REASSEMBLED`] less the envelope). A
/// writer refuses a state it could not read back (a site: [`MAX_SITE_WEB`]).
pub const MAX_READ_STATE: usize = MAX_REASSEMBLED - READ_ENVELOPE;

/// The largest site WEB part a publish may write: its read-back state is the web framed behind the site's signed
/// record (`contract_keys::site::frame`), for which this leaves 64 KiB.
pub const MAX_SITE_WEB: usize = MAX_READ_STATE - 64 * 1024;

/// Why a GET that names its contract was not answered with a state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetFail {
    /// The node's explicit NotFound: no such contract. The only answer that
    /// says the contract is absent -- from a network node `ContractResponse::NotFound`, from a LOCAL node
    /// (`freenet local`) the `ContractError::MissingContract` naming it (see `decode_one`).
    NotFound,
    /// The node refused the GET, in its cause's words. Says nothing about
    /// whether the contract exists: re-asked, never read as absent.
    Refused(String),
    /// The node ANSWERED -- with a message of `bytes`, bigger than this page reads (rule 8: an answer, never a
    /// silence re-asked for ever). The GET it answers is read from the message's HEAD ([`get_key_prefix`]).
    TooLarge { bytes: usize },
}

/// What arrived, once it has been understood.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// Messages for the engine — our protocol, inside their envelope.
    ///
    /// A LIST, because one `DelegateResponse` can carry several application
    /// messages and each is its own message. The first version concatenated
    /// them into one buffer, which the exact decoder then refused as
    /// `TrailingBytes` — correctly: a run of messages is not a message, and
    /// a decoder that accepted the prefix would have read the first one and
    /// thrown the rest away.
    EngineBytes(Vec<Vec<u8>>),
    /// A subscribed contract changed.
    ///
    /// **A HINT, never an authority.** See the module docs: a root from the
    /// node means "look", not "this is where the tree is". `state` is the
    /// contract's FULL new state when the node pushed one (`UpdateData::State`),
    /// else `None` (a delta or a related-state push): only a full state may
    /// confirm this page's own owed head (sdk#378 P3).
    HeadChanged { key: String, state: Option<Vec<u8>> },
    /// A contract GET answered (ENGINE-SHAPE §2: the page reads blocks and the
    /// head itself). `id` is the contract's INSTANCE id, 32 bytes, as the node
    /// names it: the page matches it against the head it subscribed to. For a
    /// BLOCK the id is not trusted either way — the block id is recomputed
    /// from the state (`block::block_of_state`) and a state nobody asked for
    /// verifies as nothing.
    ///
    /// ASSUMPTION (the cold read's tests decide it): a node that cannot find a
    /// contract answers with `ContractError::Get` ([`Incoming::GetFailed`]),
    /// not with an empty state. An empty `GetResponse` stays unusable, and to
    /// a cold read it is a GET that did not answer.
    Got { id: [u8; 32], state: Vec<u8> },
    /// A contract GET the node REFUSED, naming the contract (its
    /// `ContractError::Get { key, .. }`), so the page can tell which of its
    /// GETs this answers. A page GET of a block this node's own delegate wrote
    /// is refused on a node with a peer (F55) — that is how a cold read learns
    /// a root is local. A refusal that names nothing stays [`Incoming::Refused`].
    ///
    /// `why` tells the node's two answers apart (#332 review): an explicit
    /// NotFound — no such contract, "after exhaustive search" — and a
    /// refusal, in its cause's words. Only the first says the contract is
    /// ABSENT; a refusal says nothing about it.
    GetFailed { id: [u8; 32], why: GetFail },
    /// A contract PUT the node REFUSED, naming the contract (its
    /// `ContractError::Put { key, cause }`) as [`AckKind::Put`] names one it
    /// accepted — so a refusal is attributed to its PUT by key, never by
    /// position. `said` is the node's `cause`: display only.
    PutFailed { key: String, said: String },
    /// A node error that names NO contract in its KIND but whose TEXT is the pinned PUT-refusal format
    /// ([`PUT_ERROR_FORMAT`], sdk#433): `key` and `said` (the reason) parsed from it. The node's words, so it is
    /// attributed only to a PUT the caller has on the wire, and final only for a pinned validation reason.
    PutFailedByText { key: String, said: String },
    /// The node accepted a request, and WHICH.
    ///
    /// **An ack is not durability.** A write becomes published by being READ
    /// BACK, never by being acknowledged — and through a gateway that is a
    /// trust requirement and not only a timing one: an ack is unauthenticated,
    /// so a hostile node could acknowledge a write it never made.
    ///
    /// The kind is for diagnostics and for `provision()`, which needs to tell
    /// "the delegate registered" from "the contract went in" while doing both
    /// in one session.
    Ack(AckKind),
    /// A delegate request answered "no such delegate on this node", NAMING the delegate (freenet 0.2.137+, #5729:
    /// `DelegateError::Missing(key)`; 0.2.136 answered EMPTY). `key` is the delegate key as [`AckKind::Registered`]
    /// names one. What it means is the caller's: to a page only ASKING whose node this is, the answer; to a page
    /// that registered the signer itself, not an answer yet (sdk#439).
    DelegateMissing { key: String },
    /// A delegate request the node would not run yet: its per-connection BACKOFF after a failure (freenet 0.2.137+,
    /// `DelegateError::ExecutionError("delegate … is rate limited after repeated failures; retry in … ms")`). Not
    /// an answer to anything: the request is re-sent on the page's clock. `said`: the node's words.
    DelegateThrottled { said: String },
    /// The node refused. The reason is the NODE's word; see [`Refused`].
    Refused(Refused),
    /// Not something this build can use. Counted, never silently dropped.
    Unusable(Unusable),
    /// A chunk of a larger message; nothing to hand on yet.
    Partial,
}

/// Which request an [`Incoming::Ack`] answers — and WHICH ONE.
///
/// **Every ack that can name what it answers, does.** The first version had a
/// bare `Put`, so two contract installs both expected `Put` and either one's
/// ack completed the other: a duplicate acknowledgement of the FIRST contract
/// advanced the SECOND, and the run reported a contract installed that had
/// never been sent. "One step in flight" does not help, because the ack that
/// arrives need not be this step's.
///
/// The node names them: `DelegateResponse` carries a `DelegateKey`
/// (`client_events.rs:773`), `PutResponse` and `SubscribeResponse` carry a
/// `ContractKey`. Pairing by position when the answer names itself is the
/// defect; carrying the name is the fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AckKind {
    /// A delegate was registered, and which.
    Registered(String),
    /// A contract was put, and which.
    Put(String),
    /// A subscription was taken, and on what.
    Subscribed(String),
    /// A contract UPDATE was answered. It carries NO information about which
    /// record the register kept (F56): the page confirms by the SUBSCRIBED
    /// head's ledger, never by this (ENGINE-SHAPE §5, correction 4).
    Updated(String),
    /// Something that needed no answer succeeded. Names nothing because there
    /// is nothing to name — and a step must not rely on this one to identify
    /// itself.
    Ok,
}

/// A refusal, as the node worded it.
///
/// **What a caller may do with this is bounded, and the bound is the point.**
/// The node chose the reason, so it steers client control flow — which makes
/// it an input from a stranger, not a diagnosis. At most a bounded retry, and
/// never, under any reason, a prompt for a credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    /// The node's own words. For display and logs. Never matched on to decide
    /// anything security-relevant.
    pub said: String,
    /// Which of the node's errors it was, as a stable code (its `ErrorKind`): what an unattributed node error is
    /// COUNTED under (sdk#433). `"unreadable"` when the error does not decode.
    pub code: &'static str,
}

// WORKAROUND(freenet-core#none filed (owner's call)): the validation refusal is recognised by its text (W9, sdk#464)
/// THE NODE'S WORDS WHEN A CONTRACT'S OWN VALIDATION REFUSES A PUT (sdk#433): a final answer, not a transient one --
/// the same bytes are refused again, for ever. Read on freenet [`VALIDATION_REFUSED_READ_ON`] (freenet-core
/// 7fa2c6605b99) with freenet-stdlib 0.10.0:
/// * `"not valid"` -- `contract_ops.rs` (a fresh PUT whose `validate_state` is not Valid);
/// * `"invalid put"` -- `ContractError::invalid_put` (stdlib `INVALID_PUT`), from `executor_impl.rs`'s upsert.
///
/// EXACT match only: an unrecognised refusal stays transient (re-sent, shown "not answering"), the safe side. A
/// freenet version bump re-runs `probe live-refusal-text` (WORKAROUNDS, the detect check), which fails if a real
/// validation refusal no longer says one of these. The node reports it keyless, in [`PUT_ERROR_FORMAT`].
pub const VALIDATION_REFUSED: [&str; 2] = ["not valid", "invalid put"];

/// The freenet versions [`VALIDATION_REFUSED`] and [`PUT_ERROR_FORMAT`] were read on (the detect probe, each).
pub const VALIDATION_REFUSED_READ_ON: &[&str] = &["0.2.136", "0.2.138"];

/// HOW THE NODE REPORTS A PUT ITS EXECUTOR REFUSED (sdk#433, probed on each of [`VALIDATION_REFUSED_READ_ON`]): a
/// KEYLESS `ErrorKind::OperationError` whose cause is exactly `put error for contract {key}, reason: {reason}` --
/// e.g. `put error for contract 7UHV…G1xY, reason: invalid put` for a Block PUT the contract refuses. The prefix and
/// the separator, as pinned.
pub const PUT_ERROR_FORMAT: (&str, &str) = ("put error for contract ", ", reason: ");

/// `(key, reason)` if `cause` is exactly the pinned PUT-refusal format; the key is non-empty and has no spaces.
pub fn parse_put_error(cause: &str) -> Option<(String, String)> {
    let rest = cause.strip_prefix(PUT_ERROR_FORMAT.0)?;
    let (key, reason) = rest.split_once(PUT_ERROR_FORMAT.1)?;
    (!key.is_empty() && !key.contains(char::is_whitespace)).then(|| (key.to_string(), reason.to_string()))
}

/// Is `said` (a keyed PUT refusal's cause) a contract's own VALIDATION refusal -- final?
pub fn is_validation_refusal(said: &str) -> bool {
    VALIDATION_REFUSED.contains(&said)
}

/// A keyless `OperationError` in the pinned PUT-refusal format: `(key, reason)`.
fn put_error_text(bytes: &[u8]) -> Option<(String, String)> {
    use freenet_stdlib::client_api::{ClientError, ErrorKind};
    let Ok(Err(e)) = bincode::deserialize::<Result<HostResponse, ClientError>>(bytes) else { return None };
    match e.kind() {
        ErrorKind::OperationError { cause } => parse_put_error(cause),
        _ => None,
    }
}

/// The stable code of a node error that names no contract (`Refused::code`).
fn node_error_code(bytes: &[u8]) -> &'static str {
    use freenet_stdlib::client_api::{ClientError, ErrorKind};
    let Ok(Err(e)) = bincode::deserialize::<Result<HostResponse, ClientError>>(bytes) else {
        return "unreadable";
    };
    match e.kind() {
        ErrorKind::ChannelClosed => "channel_closed",
        ErrorKind::DeserializationError { .. } => "deserialization_error",
        ErrorKind::Disconnect => "disconnect",
        ErrorKind::IncorrectState(_) => "incorrect_state",
        ErrorKind::NodeUnavailable => "node_unavailable",
        ErrorKind::TransportProtocolDisconnect => "transport_protocol_disconnect",
        ErrorKind::Unhandled { .. } => "unhandled",
        ErrorKind::UnknownClient(_) => "unknown_client",
        ErrorKind::RequestError(_) => "request_error",
        ErrorKind::OperationError { .. } => "operation_error",
        ErrorKind::FailedOperation => "failed_operation",
        ErrorKind::Shutdown => "shutdown",
        ErrorKind::EmptyRing => "empty_ring",
        ErrorKind::PeerNotJoined => "peer_not_joined",
        _ => "other",
    }
}

impl Refused {
    /// May a caller retry this at all? Bounded by the CALLER's own counter —
    /// this never says "retry for ever".
    pub fn retryable(&self) -> bool {
        true
    }

    /// Always false, and it is a function rather than a comment so that a
    /// future caller reaching for "should I ask for the passphrase again?"
    /// finds an answer instead of inventing one.
    ///
    /// No node-supplied reason may lead to a credential prompt. A node that
    /// could make a page ask for a passphrase has a phishing channel
    /// delivered through the protocol.
    pub fn may_prompt_for_credentials(&self) -> bool {
        false
    }
}

/// Why something could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unusable {
    /// Bigger than this build will decode, before decoding it.
    TooLarge,
    /// Not decodable as a `HostResponse` at all.
    Unparseable,
    /// Decoded, but a kind this side has no use for. NAMED: a diagnostic
    /// that says only "not for us" costs a second run to find out which.
    UnknownKind(&'static str),
    /// The node's own error reply. A message, not a failure to read one.
    NodeSaidNo,
    /// A chunked message whose shape this build refuses — too many chunks,
    /// an index outside its own total, or too many streams at once.
    BadStream,
}

/// Frame an engine message to the delegate.
///
/// Returns FRAMES, plural: stdlib chunks anything over its threshold, and the
/// engine delegate is ~721 KB, so `provision()` crosses it on the first
/// `RegisterDelegate` of every session. A `frame_*` that returned one buffer
/// would work for every small message and fail on the first real one.
pub fn frame_delegate_op(
    key: &DelegateKey,
    params: &Parameters<'static>,
    payload: Vec<u8>,
    stream_id: u32,
) -> Result<Vec<Vec<u8>>, String> {
    let req = ClientRequest::DelegateOp(DelegateRequest::ApplicationMessages {
        key: key.clone(),
        params: params.clone(),
        inbound: vec![InboundDelegateMsg::ApplicationMessage(
            ApplicationMessage::new(payload),
        )],
    });
    frames(&req, stream_id)
}

/// A contract's instance id, from the 32 bytes that name it.
///
/// So a caller can hold one without depending on freenet itself — the same
/// reason `DelegateKey` is re-exported. The engine reports a head as 32
/// bytes; this is what turns those into something the client API will accept.
pub fn contract_id(bytes: [u8; 32]) -> ContractInstanceId {
    ContractInstanceId::new(bytes)
}

/// Frame a contract PUT.
pub fn frame_put(
    contract: ContractContainer,
    state: WrappedState,
    stream_id: u32,
) -> Result<Vec<Vec<u8>>, String> {
    let req = ClientRequest::ContractOp(freenet_stdlib::client_api::ContractRequest::Put {
        contract,
        state,
        related_contracts: RelatedContracts::default(),
        subscribe: false,
        blocking_subscribe: false,
    });
    frames(&req, stream_id)
}

/// Frame a contract GET (the page reads blocks and the head itself: ENGINE-SHAPE
/// §2). `subscribe`: the HEAD is read with a subscription — a peered node serves
/// a page only what it is subscribed to or wrote (F55), and a subscription is
/// also how a head move reaches this page (§5 correction 4). A block is got
/// without one; `return_contract_code` never, the page has the code.
pub fn frame_get(id: ContractInstanceId, subscribe: bool, stream_id: u32) -> Result<Vec<Vec<u8>>, String> {
    let req = ClientRequest::ContractOp(freenet_stdlib::client_api::ContractRequest::Get {
        key: id,
        return_contract_code: false,
        subscribe,
        blocking_subscribe: false,
    });
    frames(&req, stream_id)
}

/// Frame a contract UPDATE with a whole new state — the head register's signed
/// record (§5: `Signed` → send UPDATE). An UPDATE names the contract by its
/// full KEY (code hash included), not by instance id. An UPDATE of an identical
/// record is the same record, so a silent one is re-sent as it is.
pub fn frame_update(key: ContractKey, state: Vec<u8>, stream_id: u32) -> Result<Vec<Vec<u8>>, String> {
    let req = ClientRequest::ContractOp(freenet_stdlib::client_api::ContractRequest::Update {
        key,
        data: UpdateData::State(State::from(state)),
    });
    frames(&req, stream_id)
}

/// Frame a contract UPDATE carrying one DELTA (a Tail's next step, say): `UpdateData::Delta`, which the contract's
/// `update_state` applies to what it holds. Never sent as a `State`: a node checks an incoming `State` as a whole state
/// before the contract sees it, and refuses one that is only a delta ("invalid put").
pub fn frame_update_delta(key: ContractKey, delta: Vec<u8>, stream_id: u32) -> Result<Vec<Vec<u8>>, String> {
    let req = ClientRequest::ContractOp(freenet_stdlib::client_api::ContractRequest::Update {
        key,
        data: UpdateData::Delta(StateDelta::from(delta)),
    });
    frames(&req, stream_id)
}

/// The delegate, and the key the node will know it by, from its wasm.
///
/// Both come from the same bytes and the key is DERIVED — a caller that
/// carried a key alongside the code could hold one that names a different
/// build, and would then address requests to a delegate that is not the one
/// it registered.
pub fn delegate_from_code(wasm: &[u8]) -> (DelegateContainer, DelegateKey) {
    let d = DelegateContainer::Wasm(DelegateWasmAPIVersion::V1(Delegate::from((
        &DelegateCode::from(wasm.to_vec()),
        &Parameters::from(vec![]),
    ))));
    let key = d.key().clone();
    (d, key)
}

/// The Register contract's parameters: its ONE writer is `contract_keys::register_params` (sans-IO, so a
/// delegate that cannot link this crate lays out the same bytes), re-exported here for every caller.
pub use contract_keys::register_params;

/// The name the head record is kept under.
pub const HEAD_NAME: &[u8] = b"head";

/// The name the OBSERVATION tree's head is kept under (craftworks-docs OBSERVABILITY §1, sdk#399): the same identity
/// and the same one [`register_params`], a second fixed name, so support finds a person's recordings from their
/// identity alone -- no list, no pointer, no inbox. ONE owner: the signer's label (`contract_keys::site::OBS_LABEL`,
/// which can't depend on wire), so the page's name and the signer's are one constant, not two pinned equal.
pub const OBS_NAME: &[u8] = contract_keys::site::OBS_LABEL;

/// Frame a delegate registration. This is the one that is always chunked.
pub fn frame_register_delegate(
    delegate: DelegateContainer,
    stream_id: u32,
) -> Result<Vec<Vec<u8>>, String> {
    let req = ClientRequest::DelegateOp(DelegateRequest::RegisterDelegate {
        delegate,
        cipher: [0u8; 32],
        nonce: [0u8; 24],
    });
    frames(&req, stream_id)
}

/// The websocket URL for a node's client API.
///
/// **`encodingProtocol=native` is not optional and not a default.** The node
/// falls back to Flatbuffers when the parameter is absent
/// (`client_events/websocket.rs:904` at `ea1ff5f`), while stdlib's own client
/// — and therefore everything here — speaks bincode. Built in Rust rather
/// than assembled in JavaScript for exactly that reason: it is a protocol
/// fact, and the first version of the page's socket omitted it.
/// Refuses a host that is not loopback, because `ws://` is cleartext.
///
/// Reaching a node on another machine over an unencrypted socket would put
/// every request and every reply — and the traffic pattern of a person's whole
/// session — on the wire in the clear. Whether that ever happens, and over
/// what, is a decision nobody has taken; until they have, building the URL is
/// how it happens by accident. A refusal here is a compile-time-shaped
/// problem rather than a silent downgrade.
pub fn ws_url(host: &str, port: u16) -> Result<String, String> {
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]");
    if !loopback {
        return Err(format!(
            "refusing to build a cleartext ws:// URL for `{host}`: only \
             loopback is allowed until transport security for a remote node \
             has been decided"
        ));
    }
    Ok(format!(
        "ws://{host}:{port}/v1/contract/command?encodingProtocol=native"
    ))
}

/// Understand one frame from the node.
///
/// **This is the trust boundary.** Everything arriving here was chosen by
/// something else — and a gateway is not the user's node. Sizes and counts are
/// bounded before anything is allocated for them, nothing panics, and anything
/// unusable comes back said rather than swallowed.
///
/// Stateful, because replies are chunked: a frame may be part of a message and
/// not a message. `Partial` is that case, and it is not an error.
pub fn unframe(r: &mut Reassembler, bytes: &[u8]) -> Incoming {
    if bytes.len() > MAX_FRAME {
        return too_large(bytes);
    }
    // A chunk first: a `StreamChunk` is a valid `HostResponse`, so decoding
    // has to happen before the shape is known.
    let decoded = match decode_one(bytes) {
        Ok(d) => d,
        Err(other) => return other,
    };
    let whole = match decoded {
        HostResponse::StreamChunk {
            stream_id,
            index,
            total,
            data,
        } => match r.chunk(stream_id, index, total, data.to_vec()) {
            Ok(None) => return Incoming::Partial,
            // THE SAME FUNCTION, because this is the same check at a second
            // door. The first version decoded here with a different arm, so a
            // node error arriving UNCHUNKED became `Refused` and the identical
            // error arriving CHUNKED became `Unusable` — one fact, two
            // answers, decided by a detail of transport the sender picks.
            Ok(Some(bytes)) => match decode_one(&bytes) {
                Ok(d) => d,
                Err(other) => return other,
            },
            Err(why) => return Incoming::Unusable(why),
        },
        other => other,
    };
    classify(whole)
}

/// A message too large to read: the GET it answers, named from the message's head, or -- when its head names no
/// GET -- unusable.
fn too_large(bytes: &[u8]) -> Incoming {
    match get_key_prefix(bytes) {
        Some(id) => Incoming::GetFailed { id, why: GetFail::TooLarge { bytes: bytes.len() } },
        None => Incoming::Unusable(Unusable::TooLarge),
    }
}

/// The contract a GET's answer names, read from its HEAD: `Result::Ok`, `HostResponse::ContractResponse`,
/// `ContractResponse::GetResponse`, its key, its contract option and its state's LENGTH -- everything the stdlib
/// encodes before the state, never the state itself, however large. It names a GET only if that state length fills
/// the rest of the message EXACTLY: a message of zeros (every first variant, a key of zeros) names nothing. `None` for
/// anything else.
pub fn get_key_prefix(bytes: &[u8]) -> Option<[u8; 32]> {
    // The stdlib's own variant ORDER, first variant each (a test pins it against a real encoding).
    #[derive(serde::Deserialize)]
    enum Head {
        Ok(Host),
    }
    #[derive(serde::Deserialize)]
    enum Host {
        ContractResponse(Contract),
    }
    #[derive(serde::Deserialize)]
    enum Contract {
        GetResponse { key: freenet_stdlib::prelude::ContractKey, _contract: Option<freenet_stdlib::prelude::ContractContainer>, state_len: u64 },
    }
    let mut at = std::io::Cursor::new(bytes);
    let Head::Ok(Host::ContractResponse(Contract::GetResponse { key, _contract: _, state_len })) = bincode::deserialize_from::<_, Head>(&mut at).ok()?;
    let head = usize::try_from(at.position()).ok()?;
    (usize::try_from(state_len).ok()?.checked_add(head)? == bytes.len()).then(|| key.id().as_bytes().try_into().ok()).flatten()
}

/// Decode one complete message, turning a node's own error into the message
/// it is.
///
/// One function, called at BOTH doors — unchunked and reassembled — so the
/// same bytes cannot mean two different things depending on how they arrived.
fn decode_one(bytes: &[u8]) -> Result<HostResponse, Incoming> {
    match Reassembler::decode(bytes) {
        Ok(d) => Ok(d),
        Err(Unusable::TooLarge) => Err(too_large(bytes)),
        // A LOCAL node's "no such contract" (`freenet local`; freenet-core 0.2.138 contract/executor/runtime.rs: the
        // GET and the SUBSCRIBE of a contract it never stored answer `ContractError::MissingContract { key }`): the
        // same fact as a network node's explicit NotFound, so the same message. Seen live 2026-09-27: without it a
        // page on a local node never opened (its head's first read, before the first PUT, was a refusal).
        Err(Unusable::NodeSaidNo) if missing_contract(bytes).is_some() => {
            Err(Incoming::GetFailed { id: missing_contract(bytes).expect("just checked"), why: GetFail::NotFound })
        }
        // A GET refusal that names its contract: which GET it answers.
        Err(Unusable::NodeSaidNo) if get_refused(bytes).is_some() => {
            let (id, cause) = get_refused(bytes).expect("just checked");
            Err(Incoming::GetFailed { id, why: GetFail::Refused(cause) })
        }
        // A PUT refusal that names its contract: which PUT it answers.
        Err(Unusable::NodeSaidNo) if put_refused(bytes).is_some() => {
            let (key, said) = put_refused(bytes).expect("just checked");
            Err(Incoming::PutFailed { key, said })
        }
        // "No such delegate here", naming it (0.2.137+): which delegate request it answers.
        Err(Unusable::NodeSaidNo) if delegate_missing(bytes).is_some() => {
            Err(Incoming::DelegateMissing { key: delegate_missing(bytes).expect("just checked") })
        }
        // The node's delegate backoff (0.2.137+): not an answer, re-sent on the clock.
        Err(Unusable::NodeSaidNo) if delegate_throttled(bytes).is_some() => {
            Err(Incoming::DelegateThrottled { said: delegate_throttled(bytes).expect("just checked") })
        }
        // A keyless error whose TEXT is the pinned PUT-refusal format (sdk#433): parsed, attributed by the caller.
        Err(Unusable::NodeSaidNo) if put_error_text(bytes).is_some() => {
            let (key, said) = put_error_text(bytes).expect("just checked");
            Err(Incoming::PutFailedByText { key, said })
        }
        // A node's own error reply is a MESSAGE, not a failure to read one.
        Err(Unusable::NodeSaidNo) => Err(Incoming::Refused(Refused {
            said: "the node refused the request".into(),
            code: node_error_code(bytes),
        })),
        Err(why) => Err(Incoming::Unusable(why)),
    }
}

fn classify(r: HostResponse) -> Incoming {
    use freenet_stdlib::client_api::ContractResponse;
    match r {
        HostResponse::DelegateResponse { key, values } => {
            let mut out: Vec<Vec<u8>> = Vec::new();
            for v in values {
                if let OutboundDelegateMsg::ApplicationMessage(m) = v {
                    out.push(m.payload.to_vec());
                }
            }
            if out.is_empty() {
                // A delegate response carrying no application message is the
                // node ACKNOWLEDGING — RegisterDelegate answers this way. The
                // first version called it unusable, which turned the ordinary
                // reply to the first message of every session into an error.
                Incoming::Ack(AckKind::Registered(key.to_string()))
            } else {
                Incoming::EngineBytes(out)
            }
        }
        // A GET answered with NO state is not a block (no kind byte) and not a
        // head record: unusable, as every GET answer was before the page read
        // blocks itself — a run of zeroes decodes as exactly this. ASSUMPTION
        // (the live run tells): a node that cannot find a contract answers
        // with an error, not with an empty state; to a cold read an empty one
        // is a GET that did not answer, and is re-fetched.
        HostResponse::ContractResponse(ContractResponse::GetResponse { state, .. }) if state.as_ref().is_empty() => {
            Incoming::Unusable(Unusable::UnknownKind("ContractResponse::GetResponse (no state)"))
        }
        HostResponse::ContractResponse(ContractResponse::GetResponse { key, state, .. }) => {
            let mut id = [0u8; 32];
            id.copy_from_slice(&key.id().as_bytes()[..32]);
            Incoming::Got { id, state: state.as_ref().to_vec() }
        }
        HostResponse::ContractResponse(ContractResponse::UpdateResponse { key, .. }) => {
            Incoming::Ack(AckKind::Updated(key.to_string()))
        }
        HostResponse::ContractResponse(ContractResponse::UpdateNotification { key, update }) => {
            let state = match update {
                UpdateData::State(s) => Some(s.as_ref().to_vec()),
                _ => None,
            };
            Incoming::HeadChanged {
                key: key.to_string(),
                state,
            }
        }
        HostResponse::ContractResponse(ContractResponse::PutResponse { key }) => {
            Incoming::Ack(AckKind::Put(key.to_string()))
        }
        HostResponse::ContractResponse(ContractResponse::SubscribeResponse { key, .. }) => {
            Incoming::Ack(AckKind::Subscribed(key.to_string()))
        }
        // 0.2.136 answers a GET of a contract that exists nowhere with an
        // explicit NotFound ("after exhaustive search"), not an error: the
        // same fact as a `ContractError::Get` refusal, so the same message.
        // (Seen live: the head Register's first read, before its first PUT.)
        HostResponse::ContractResponse(ContractResponse::NotFound { instance_id }) => {
            let mut id = [0u8; 32];
            id.copy_from_slice(&instance_id.as_bytes()[..32]);
            Incoming::GetFailed { id, why: GetFail::NotFound }
        }
        HostResponse::Ok => Incoming::Ack(AckKind::Ok),
        // A kind this build has no use for. NAMED, so a failure says which:
        // the first run of the live driver reported `NotForUs` and could have
        // meant either "the node refused" or "a variant we do not handle",
        // and those want opposite responses.
        other => Incoming::Unusable(Unusable::UnknownKind(kind_of(&other))),
    }
}

/// The variant's own name, for a diagnostic that does not need a second run.
fn kind_of(r: &HostResponse) -> &'static str {
    use freenet_stdlib::client_api::ContractResponse as C;
    match r {
        HostResponse::ContractResponse(C::GetResponse { .. }) => "ContractResponse::GetResponse",
        HostResponse::ContractResponse(C::UpdateResponse { .. }) => {
            "ContractResponse::UpdateResponse"
        }
        HostResponse::ContractResponse(_) => "ContractResponse(other)",
        HostResponse::DelegateResponse { .. } => "DelegateResponse",
        HostResponse::Ok => "Ok",
        _ => "another kind",
    }
}

/// Serialise and chunk, using stdlib's own rules.
fn frames(req: &ClientRequest<'static>, stream_id: u32) -> Result<Vec<Vec<u8>>, String> {
    use freenet_stdlib::client_api::streaming::{chunk_request, ensure_chunkable, CHUNK_THRESHOLD};
    let body = bincode::serialize(req).map_err(|e| e.to_string())?;
    if body.len() <= CHUNK_THRESHOLD {
        return Ok(vec![body]);
    }
    // Refuse BEFORE sending anything. stdlib's own client does this: streaming
    // an oversized payload only to have the node reject the first chunk wastes
    // the whole upload, and on a hotspot that is minutes.
    ensure_chunkable(body.len()).map_err(|e| e.to_string())?;
    chunk_request(body, stream_id)
        .iter()
        .map(|c| bincode::serialize(c).map_err(|e| e.to_string()))
        .collect()
}

/// The node's `ContractError`, if the bytes are one: the one decode both
/// refusals below read.
/// The node's error, if the bytes are one: what it says, by kind.
fn node_error(bytes: &[u8]) -> Option<freenet_stdlib::client_api::ErrorKind> {
    use freenet_stdlib::client_api::ClientError;
    let Ok(Err(e)) = bincode::deserialize::<Result<HostResponse, ClientError>>(bytes) else {
        return None;
    };
    Some(e.kind().clone())
}

fn contract_error(bytes: &[u8]) -> Option<freenet_stdlib::client_api::ContractError> {
    use freenet_stdlib::client_api::{ErrorKind, RequestError};
    match node_error(bytes)? {
        ErrorKind::RequestError(RequestError::ContractError(c)) => Some(c),
        _ => None,
    }
}

/// The contract a local node says it does not have (`ContractError::MissingContract`), if the bytes are that.
fn missing_contract(bytes: &[u8]) -> Option<[u8; 32]> {
    match contract_error(bytes)? {
        freenet_stdlib::client_api::ContractError::MissingContract { key } => key.as_bytes().get(..32)?.try_into().ok(),
        _ => None,
    }
}

/// The contract a node's GET refusal names, and the node's cause, if the bytes
/// are one.
fn get_refused(bytes: &[u8]) -> Option<([u8; 32], String)> {
    match contract_error(bytes)? {
        freenet_stdlib::client_api::ContractError::Get { key, cause } => {
            let mut id = [0u8; 32];
            id.copy_from_slice(&key.id().as_bytes()[..32]);
            Some((id, cause.to_string()))
        }
        _ => None,
    }
}

/// The contract (named as [`AckKind::Put`] names it) and the node's cause, if
/// the bytes are a PUT refusal.
/// The node's delegate error, if the bytes are one.
fn delegate_error(bytes: &[u8]) -> Option<freenet_stdlib::client_api::DelegateError> {
    use freenet_stdlib::client_api::{ErrorKind, RequestError};
    match node_error(bytes)? {
        ErrorKind::RequestError(RequestError::DelegateError(d)) => Some(d),
        _ => None,
    }
}

/// The delegate a node's "missing delegate" names, as `DelegateKey`'s Display (the form `AckKind::Registered` uses).
fn delegate_missing(bytes: &[u8]) -> Option<String> {
    match delegate_error(bytes)? {
        freenet_stdlib::client_api::DelegateError::Missing(key) => Some(key.to_string()),
        _ => None,
    }
}

// WORKAROUND(freenet-core#5729): the per-key delegate backoff is recognised by its text
/// THE NODE'S DELEGATE BACKOFF, by its words (WORKAROUNDS.md W8, until freenet-core types it): freenet-core 0.2.138
/// crates/core/src/client_events/websocket.rs:1970 sends `ExecutionError("delegate {key} is rate limited after
/// repeated failures; retry in {ms} ms")` -- an ExecutionError carries no key, so the words are all there is.
/// Any OTHER ExecutionError stays a refusal.
pub const DELEGATE_THROTTLED: &str = "is rate limited after repeated failures";

fn delegate_throttled(bytes: &[u8]) -> Option<String> {
    match delegate_error(bytes)? {
        freenet_stdlib::client_api::DelegateError::ExecutionError(said) if said.contains(DELEGATE_THROTTLED) => Some(said.to_string()),
        _ => None,
    }
}

fn put_refused(bytes: &[u8]) -> Option<(String, String)> {
    match contract_error(bytes)? {
        freenet_stdlib::client_api::ContractError::Put { key, cause } => Some((key.to_string(), cause.to_string())),
        _ => None,
    }
}

/// Handles for tests that need to build what a node would send.
///
/// Not a general surface: the alternative is a test that constructs the bytes
/// by hand and agrees with its own idea of the format, which is the drift this
/// crate exists to prevent.
#[doc(hidden)]
pub mod _test {
    pub use freenet_stdlib::client_api::ClientError as Err;
    pub use freenet_stdlib::client_api::HostResponse as Host;

    pub fn client_error(msg: &str) -> Err {
        Err::from(freenet_stdlib::client_api::ErrorKind::Unhandled {
            cause: msg.to_string().into(),
        })
    }

    /// Run a COMPLETE message through the same path a reassembled one takes.
    pub fn classify_for_test(bytes: &[u8]) -> super::Incoming {
        match super::decode_one(bytes) {
            Ok(d) => super::classify(d),
            Err(other) => other,
        }
    }
}
