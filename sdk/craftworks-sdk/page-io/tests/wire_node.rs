//! `page-io` END TO END AT THE BYTE LEVEL: the SDK's protocol frames go in,
//! `page-io` frames client-API requests, and a scripted Freenet node DECODES
//! them (the real `ClientRequest` encoding), applies them — blocks, the head
//! Register through its contract's own `update_state`, and the REAL signer
//! behind a real delegate-op envelope — and answers with real encoded
//! `HostResponse`s. The replies the SDK gets back are decoded protocol replies.
//!
//! What a live node adds on top is the network; everything the page does to
//! reach it is exercised here.

use freenet_prolly::Cid;
use freenet_stdlib::client_api::{ClientRequest, ContractRequest, ContractResponse, DelegateError, DelegateRequest, HostResponse};
use freenet_stdlib::prelude::*;
use page::server::{Server, SignerFacts};
use page::{Ms, Page, PutPath};
use page_io::{Artefacts, PageIo};
use protocol::{Reply, Request, WriteState};
use std::collections::BTreeMap;

const BLOCK_CODE: &[u8] = b"wire-node block code";
const REGISTER_CODE: &[u8] = b"wire-node register code";
const SIGNER_CODE: &[u8] = b"wire-node signer code";
const SITE_CODE: &[u8] = b"wire-node site code";
const APP: &str = "notes";

type Err = freenet_stdlib::client_api::ClientError;

/// The node: contract states by instance id, and the signer's secrets.
struct WireNode {
    contracts: BTreeMap<[u8; 32], Vec<u8>>,
    register_id: [u8; 32],
    register_params: Vec<u8>,
    secrets: BTreeMap<Vec<u8>, Vec<u8>>,
    /// Frames the node answered, by kind (the test proves each path ran).
    served: BTreeMap<&'static str, usize>,
    /// PUTs of the Register (creations), counted.
    register_puts: usize,
    /// The next this many GETs of the Register are answered NotFound though it
    /// exists — a peered node's false NotFound (F55, sdk#175).
    fail_register_gets: usize,
    /// The next this many signer requests are answered EMPTY — a
    /// DelegateResponse carrying no message — as the real node answered a
    /// request that reached it before the registration had taken (#260,
    /// measured: 7 of 12 opens).
    empty_signer_answers: usize,
    /// The next this many signer answers are LOST — nothing comes back at
    /// all: only the RTO re-ask recovers from that.
    drop_signer_answers: usize,
    /// No signer delegate on this node at all (another user's node): a signer
    /// request is answered with the node's error.
    delegate_absent: bool,
    /// No signer delegate UNTIL one is registered here: its requests are
    /// answered EMPTY (as 0.2.136 answers a delegate it does not have), and
    /// a registration makes it present.
    empty_until_registered: bool,
    /// The node is 0.2.137+ (#5729, sdk#439): where 0.2.136 answers a delegate request EMPTY (above), it answers
    /// the TYPED `DelegateError::Missing(key)` -- the bytes wire/tests/node_0_2_138.rs holds as captured.
    answers_as_0_2_138: bool,
    /// The next this many signer requests are answered with the 0.2.137+ node's delegate BACKOFF (its
    /// `ExecutionError("delegate … is rate limited after repeated failures; retry in … ms")`, as captured).
    throttle_signer_answers: usize,
    /// The next this many GETs of the Register are answered with THIS older
    /// state: a node serving its cached copy from before a publish (sdk#349).
    stale_register: Option<(Vec<u8>, usize)>,
    /// The next this many GETs (of anything) are REFUSED — the node's
    /// `ContractError::Get`, whether the contract exists or not: not its
    /// NotFound, and not an answer about the contract.
    refuse_gets: usize,
    /// The next this many GETs of a BLOCK (never the head) are REFUSED, as
    /// `refuse_gets` refuses them.
    refuse_block_gets: usize,
    /// This node does not hold the head LOCALLY (a fresh node of the identity): the signer's synchronous read finds
    /// nothing, while a client GET is served from the network.
    head_not_local: bool,
    /// The next this many signer requests find the SITE not held locally (a new device's node before it fetched
    /// it): the signer's synchronous read of the site finds nothing.
    site_blind_signs: usize,
    site_hidden: bool,
    /// The next this many GETs of the SITE are refused (`ContractError::Get`).
    refuse_site_gets: usize,
    /// The next this many BLOCK PUTs are LOST: not stored, never answered -- only the page's RTO re-sends them.
    lose_block_puts: usize,
    /// The next this many PUTs of ONE PARITY block (the first parity-block PUT after it is set) are answered with an ERROR, not
    /// stored: the node's "No such file or directory (os error 2)" (sdk#431, seen live) -- naming the contract
    /// (`ContractError::Put`) when `err_keyed`, or naming nothing (`ErrorKind::OperationError`, the live bytes).
    err_block_puts: usize,
    err_keyed: bool,
    /// The node's words for that error (default: sdk#431's live "No such file or directory (os error 2)").
    err_cause: &'static str,
    /// Error the FIRST block PUT of any kind (a data block too), not only a parity one.
    err_any_block: bool,
    /// Answer in the KEYLESS text form 0.2.136/0.2.138 use (sdk#433): `OperationError` whose cause is
    /// `put error for contract {key}, reason: {err_cause}` -- naming `err_text_key` instead when that is set.
    err_text_form: bool,
    err_text_key: Option<&'static str>,
    /// The block whose PUTs are answered with the error.
    err_block: Option<[u8; 32]>,
    /// Every block PUT that reached the node: (the clock when it did, the block contract's id).
    block_puts: Vec<(u64, [u8; 32])>,
    /// The clock the harness serves at (set before each frame).
    now: u64,
    /// The node PUSHES the head register's full new state to this page (an `UpdateNotification`, as its GET's
    /// subscription asked) BEFORE it answers the UPDATE -- the order measured on 0.2.136 (sdk#378: HeadChanged
    /// 1-4 ms before the UpdateResponse).
    push_updates: bool,
    /// How the push carries the new state (sdk#378 P3's condition 1): the full `State`, or -- what must NOT be
    /// judged as a state -- a `Delta` or `StateAndDelta` carrying the same bytes.
    push_kind: PushKind,
    /// Frames the node sends unasked (pushes), delivered before the answer to the request that caused them.
    pushes: Vec<Vec<u8>>,
}

/// How the node's push carries the new state (see `WireNode::push_kind`).
#[derive(Clone, Copy, Debug)]
enum PushKind {
    State,
    Delta,
    StateAndDelta,
}

struct Host<'a>(&'a mut WireNode);
impl signer::Host for Host<'_> {
    fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.secrets.get(key).cloned()
    }
    fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool {
        self.0.secrets.insert(key.to_vec(), value.to_vec());
        true
    }
    fn contract_state(&self, id: &[u8; 32]) -> Option<Vec<u8>> {
        if self.0.head_not_local && *id == self.0.register_id {
            return None;
        }
        if self.0.site_hidden && *id == self.0.site_id() {
            return None;
        }
        self.0.contracts.get(id).cloned()
    }
}

fn id_of(k: &ContractKey) -> [u8; 32] {
    k.id().as_bytes()[..32].try_into().expect("32")
}

fn ok(r: HostResponse) -> Vec<u8> {
    bincode::serialize(&Ok::<HostResponse, Err>(r)).expect("encodes")
}

/// A node's delegate error, encoded as the node encodes it (wire/tests/node_0_2_138.rs checks the stdlib encoding
/// against bytes captured from a 0.2.138 node).
fn delegate_error(e: DelegateError) -> Vec<u8> {
    use freenet_stdlib::client_api::{ErrorKind, RequestError};
    bincode::serialize(&Err::<HostResponse, Err>(ErrorKind::RequestError(RequestError::DelegateError(e)).into())).expect("encodes")
}

/// The node's answer to a PUT it failed on its own side (sdk#431, seen live on 0.2.136): "No such file or directory
/// (os error 2)", naming the contract (`ContractError::Put`) or naming nothing (`ErrorKind::OperationError`).
fn put_error(key: ContractKey, keyed: bool, cause: &'static str) -> Vec<u8> {
    use freenet_stdlib::client_api::{ContractError, ErrorKind, RequestError};
    let kind = if keyed {
        ErrorKind::RequestError(RequestError::ContractError(ContractError::Put { key, cause: cause.into() }))
    } else {
        ErrorKind::OperationError { cause: cause.into() }
    };
    bincode::serialize(&Err::<HostResponse, Err>(kind.into())).expect("encodes")
}

/// The KEYLESS form of a PUT refusal the node sends (sdk#433, probed on 0.2.136 and 0.2.138): an `OperationError`
/// whose cause is the pinned format `put error for contract {key}, reason: {reason}`.
fn put_error_text(key: String, reason: &str) -> Vec<u8> {
    use freenet_stdlib::client_api::ErrorKind;
    let (prefix, sep) = wire::PUT_ERROR_FORMAT;
    let kind = ErrorKind::OperationError { cause: format!("{prefix}{key}{sep}{reason}").into() };
    bincode::serialize(&Err::<HostResponse, Err>(kind.into())).expect("encodes")
}

impl WireNode {
    fn new(signing_key: &[u8; 32]) -> WireNode {
        let sk = ed25519_dalek::SigningKey::from_bytes(signing_key);
        let params = wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME);
        let mut n = WireNode {
            contracts: BTreeMap::new(),
            register_id: signer::register_id(REGISTER_CODE, &params),
            register_params: params.clone(),
            secrets: BTreeMap::new(),
            served: BTreeMap::new(),
            register_puts: 0,
            fail_register_gets: 0,
            empty_signer_answers: 0,
            drop_signer_answers: 0,
            delegate_absent: false,
            empty_until_registered: false,
            answers_as_0_2_138: false,
            throttle_signer_answers: 0,
            stale_register: None,
            refuse_gets: 0,
            refuse_block_gets: 0,
            head_not_local: false,
            site_blind_signs: 0,
            site_hidden: false,
            refuse_site_gets: 0,
            lose_block_puts: 0,
            err_block_puts: 0,
            err_keyed: false,
            err_cause: "No such file or directory (os error 2)",
            err_any_block: false,
            err_text_form: false,
            err_text_key: None,
            err_block: None,
            block_puts: Vec::new(),
            now: 0,
            push_updates: false,
            push_kind: PushKind::State,
            pushes: Vec::new(),
        };
        let req = signer::Request::Provision {
            signing_key: sk.to_bytes().to_vec(),
            register_code: REGISTER_CODE.to_vec(),
            register_params: params,
            block_code: BLOCK_CODE.to_vec(),
        };
        assert_eq!(signer::serve(&mut Host(&mut n), &signer::encode_request(1, &req), signer::Origin::Local), signer::Answer::Provisioned);
        n
    }

    /// A node whose signer has NEVER been provisioned: the person's first page.
    /// `signing_key` is what that page will mint, so the test knows the register.
    fn unprovisioned(signing_key: &[u8; 32]) -> WireNode {
        let sk = ed25519_dalek::SigningKey::from_bytes(signing_key);
        let params = wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME);
        WireNode {
            contracts: BTreeMap::new(),
            register_id: signer::register_id(REGISTER_CODE, &params),
            register_params: params,
            secrets: BTreeMap::new(),
            served: BTreeMap::new(),
            register_puts: 0,
            fail_register_gets: 0,
            empty_signer_answers: 0,
            drop_signer_answers: 0,
            delegate_absent: false,
            empty_until_registered: false,
            answers_as_0_2_138: false,
            throttle_signer_answers: 0,
            stale_register: None,
            refuse_gets: 0,
            refuse_block_gets: 0,
            head_not_local: false,
            site_blind_signs: 0,
            site_hidden: false,
            refuse_site_gets: 0,
            lose_block_puts: 0,
            err_block_puts: 0,
            err_keyed: false,
            err_cause: "No such file or directory (os error 2)",
            err_any_block: false,
            err_text_form: false,
            err_text_key: None,
            err_block: None,
            block_puts: Vec::new(),
            now: 0,
            push_updates: false,
            push_kind: PushKind::State,
            pushes: Vec::new(),
        }
    }

    fn merge_register(&mut self, state: &[u8]) {
        let params = Parameters::from(self.register_params.clone());
        let next = match self.contracts.get(&self.register_id) {
            None => state.to_vec(),
            Some(cur) => <craftec_register_contract::Register as ContractInterface>::update_state(
                params,
                State::from(cur.clone()),
                vec![UpdateData::State(State::from(state.to_vec()))],
            )
            .expect("merges")
            .new_state
            .expect("a state")
            .as_ref()
            .to_vec(),
        };
        self.contracts.insert(self.register_id, next);
    }

    /// `APP`'s site contract: its id, derived as page-io derives it.
    fn site_id(&self) -> [u8; 32] {
        let c = page_io::site_contract(SITE_CODE, &self.register_params, APP).expect("a site contract");
        id_of(&c.key())
    }

    /// A site PUT, merged as the site contract merges: the META by the Register's own merge under the site's
    /// params, the web part the one whose hash the kept record names.
    fn merge_site(&mut self, state: &[u8]) {
        let id = self.site_id();
        let (meta, web) = contract_keys::site::framing(state).expect("page-io framed the site");
        let next = match self.contracts.get(&id) {
            None => state.to_vec(),
            Some(cur) => {
                let (cur_meta, cur_web) = contract_keys::site::framing(cur).expect("a framed site");
                let params = contract_keys::site::site_params(&self.register_params, APP).expect("site params");
                let kept = <craftec_register_contract::Register as ContractInterface>::update_state(
                    Parameters::from(params),
                    State::from(cur_meta.to_vec()),
                    vec![UpdateData::State(State::from(meta.to_vec()))],
                )
                .expect("merges")
                .new_state
                .expect("a state")
                .as_ref()
                .to_vec();
                let web = if kept == meta { web } else { cur_web };
                contract_keys::site::frame(&kept, web)
            }
        };
        self.contracts.insert(id, next);
    }

    /// The site as the node holds it: `(version, value, web)`.
    fn site(&self) -> Option<(u64, Vec<u8>, Vec<u8>)> {
        let st = self.contracts.get(&self.site_id())?;
        let (meta, web) = contract_keys::site::framing(st)?;
        let (seq, v) = signer_proto::head::record_of(meta)?;
        Some((seq, v.to_vec(), web.to_vec()))
    }

    fn head(&self) -> Option<(u64, Cid)> {
        let st = self.contracts.get(&self.register_id)?;
        let (seq, v) = signer_proto::head::record_of(st)?;
        Some((seq, v[..32].try_into().expect("32")))
    }

    /// One frame from a page, answered as a node would answer it.
    fn serve(&mut self, frame: &[u8]) -> Option<Vec<u8>> {
        let req: ClientRequest = bincode::deserialize(frame).expect("page-io framed a real ClientRequest");
        match req {
            ClientRequest::ContractOp(ContractRequest::Put { contract, state, .. }) => {
                let key = contract.key();
                let id: [u8; 32] = key.as_bytes()[..32].try_into().expect("32");
                if id == self.register_id {
                    *self.served.entry("put register").or_default() += 1;
                    self.register_puts += 1;
                    self.merge_register(state.as_ref());
                } else if id == self.site_id() {
                    *self.served.entry("put site").or_default() += 1;
                    self.merge_site(state.as_ref());
                } else {
                    self.block_puts.push((self.now, id));
                    if self.lose_block_puts > 0 {
                        self.lose_block_puts -= 1;
                        return None;
                    }
                    let parity = state.as_ref().first() == Some(&freenet_prolly::kind::PARITY);
                    if self.err_block_puts > 0 && (parity || self.err_any_block) && self.err_block.is_none_or(|b| b == id) {
                        self.err_block = Some(id);
                        self.err_block_puts -= 1;
                        *self.served.entry("put block answered with an error").or_default() += 1;
                        if self.err_text_form {
                            return Some(put_error_text(self.err_text_key.map(str::to_string).unwrap_or_else(|| key.to_string()), self.err_cause));
                        }
                        return Some(put_error(key, self.err_keyed, self.err_cause));
                    }
                    *self.served.entry("put block").or_default() += 1;
                    self.contracts.insert(id, state.as_ref().to_vec());
                }
                Some(ok(HostResponse::ContractResponse(ContractResponse::PutResponse { key })))
            }
            ClientRequest::ContractOp(ContractRequest::Update { key, data: UpdateData::State(s) }) => {
                *self.served.entry("update").or_default() += 1;
                assert_eq!(id_of(&key), self.register_id, "an UPDATE of something other than the head");
                self.merge_register(s.as_ref());
                if self.push_updates {
                    let state = self.contracts.get(&self.register_id).cloned().expect("merged");
                    // The DELTA kinds carry the record's own bytes: the worst case, bytes that WOULD parse as a
                    // plausible record if the decoder judged a delta as a state.
                    let update = match self.push_kind {
                        PushKind::State => UpdateData::State(State::from(state)),
                        PushKind::Delta => UpdateData::Delta(StateDelta::from(state)),
                        PushKind::StateAndDelta => UpdateData::StateAndDelta { state: State::from(state.clone()), delta: StateDelta::from(state) },
                    };
                    self.pushes.push(ok(HostResponse::ContractResponse(ContractResponse::UpdateNotification { key, update })));
                }
                Some(ok(HostResponse::ContractResponse(ContractResponse::UpdateResponse {
                    key,
                    summary: StateSummary::from(Vec::new()),
                })))
            }
            ClientRequest::ContractOp(ContractRequest::Get { key, subscribe, .. }) => {
                let id: [u8; 32] = key.as_bytes()[..32].try_into().expect("32");
                let failing = id == self.register_id && self.fail_register_gets > 0;
                if failing {
                    self.fail_register_gets -= 1;
                }
                if id == self.register_id {
                    assert!(subscribe, "the head was read WITHOUT a subscription (F55)");
                    *self.served.entry("get register").or_default() += 1;
                } else if id == self.site_id() {
                    assert!(subscribe, "the site was read WITHOUT a subscription");
                    *self.served.entry("get site").or_default() += 1;
                } else {
                    *self.served.entry("get block").or_default() += 1;
                }
                let refuse_block = id != self.register_id && id != self.site_id() && self.refuse_block_gets > 0;
                let refuse_site = id == self.site_id() && self.refuse_site_gets > 0;
                if self.refuse_gets > 0 || refuse_block || refuse_site {
                    if refuse_site {
                        self.refuse_site_gets -= 1;
                        *self.served.entry("refused site get").or_default() += 1;
                    } else if refuse_block {
                        self.refuse_block_gets -= 1;
                    } else {
                        self.refuse_gets -= 1;
                    }
                    *self.served.entry("refused get").or_default() += 1;
                    let e: Err = freenet_stdlib::client_api::ErrorKind::RequestError(
                        freenet_stdlib::client_api::RequestError::ContractError(freenet_stdlib::client_api::ContractError::Get {
                            key: ContractKey::from_id_and_code(key, CodeHash::new([0u8; 32])),
                            cause: "the node is busy".into(),
                        }),
                    )
                    .into();
                    return Some(bincode::serialize(&Err::<HostResponse, Err>(e)).expect("encodes"));
                }
                let stale = match &mut self.stale_register {
                    Some((st, n)) if id == self.register_id && *n > 0 => {
                        *n -= 1;
                        Some(st.clone())
                    }
                    _ => None,
                };
                match stale.as_ref().or(self.contracts.get(&id)).filter(|_| !failing) {
                    Some(state) => {
                        let ckey = ContractKey::from_id_and_code(key, CodeHash::new([0u8; 32]));
                        Some(ok(HostResponse::ContractResponse(ContractResponse::GetResponse {
                            key: ckey,
                            contract: None,
                            state: WrappedState::new(state.clone()),
                        })))
                    }
                    // What 0.2.136 answers for a contract it has nowhere —
                    // and, FALSELY, for a delegate-put one on a peered node
                    // (F55, `fail_register_gets`): an explicit NotFound.
                    None => Some(ok(HostResponse::ContractResponse(ContractResponse::NotFound { instance_id: key }))),
                }
            }
            ClientRequest::DelegateOp(DelegateRequest::ApplicationMessages { key, inbound, .. }) => {
                *self.served.entry("signer").or_default() += 1;
                if self.delegate_absent {
                    return Some(bincode::serialize(&Err::<HostResponse, Err>(wire::_test::client_error("delegate not found"))).expect("encodes"));
                }
                if self.drop_signer_answers > 0 {
                    self.drop_signer_answers -= 1;
                    *self.served.entry("signer answer lost").or_default() += 1;
                    return None;
                }
                if self.throttle_signer_answers > 0 {
                    self.throttle_signer_answers -= 1;
                    *self.served.entry("signer throttled").or_default() += 1;
                    return Some(delegate_error(DelegateError::ExecutionError(
                        format!("delegate {key} is rate limited after repeated failures; retry in 47 ms").into(),
                    )));
                }
                if self.empty_until_registered || self.empty_signer_answers > 0 {
                    self.empty_signer_answers = self.empty_signer_answers.saturating_sub(1);
                    if self.answers_as_0_2_138 {
                        *self.served.entry("signer missing").or_default() += 1;
                        return Some(delegate_error(DelegateError::Missing(key)));
                    }
                    return Some(ok(HostResponse::DelegateResponse { key, values: Vec::new() }));
                }
                self.site_hidden = self.site_blind_signs > 0;
                self.site_blind_signs = self.site_blind_signs.saturating_sub(1);
                let mut values = Vec::new();
                for m in inbound {
                    if let InboundDelegateMsg::ApplicationMessage(am) = m {
                        let served = signer::serve_full(&mut Host(self), &am.payload, signer::Origin::Local);
                        values.push(OutboundDelegateMsg::ApplicationMessage(ApplicationMessage::new(signer::reply(&served))));
                    }
                }
                Some(ok(HostResponse::DelegateResponse { key, values }))
            }
            // Registering the signer (again, for a reopened page: already
            // held). Answered as the REAL node answers it — measured on
            // 0.2.136 (#260's frame log): an 80-byte DelegateResponse naming
            // the delegate and carrying NO message, which `wire::unframe`
            // reads as `Ack(Registered)`. This answered `HostResponse::Ok`
            // before, which no page ever sees from a node.
            ClientRequest::DelegateOp(DelegateRequest::RegisterDelegate { delegate, .. }) => {
                *self.served.entry("register delegate").or_default() += 1;
                self.empty_until_registered = false;
                Some(ok(HostResponse::DelegateResponse { key: delegate.key().clone(), values: Vec::new() }))
            }
            other => panic!("page-io sent a request the node does not expect: {other:?}"),
        }
    }
}

/// A page REOPENING on this node: its signer was provisioned before, so it
/// did not mint the key and the register may well exist.
fn page_io(node: &WireNode) -> PageIo {
    let (_, signer) = wire::delegate_from_code(SIGNER_CODE);
    let mut io = PageIo::new(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
        Artefacts {
            block_code: BLOCK_CODE.to_vec(),
            register_code: REGISTER_CODE.to_vec(),
            register_params: node.register_params.clone(),
            signer,
        },
    );
    assert_eq!(io.register_id(), node.register_id, "page-io names the head differently from the node");
    io.signer_provisioned();
    io
}

/// Run the page against the node until nothing is left to send.
#[track_caller]
fn settle(io: &mut PageIo, node: &mut WireNode, now: &mut u64) -> Vec<Reply> {
    // Until QUIESCENT (the architect, after #383 x #393): no frame to serve and nothing owed an answer or a re-send
    // -- only the idle page's backstop read is left on its clock, and it is not run. A phase's count never depends
    // on where a round cap fell (a cap ended every settle mid-backstop-cycle, ~33 simulated hours in, and #393's
    // one extra round moved a Hint read into the next phase's window). The cap is now a failure, never an end.
    let mut replies = Vec::new();
    for _ in 0..2_000 {
        let frames = io.take_frames();
        replies.extend(io.take_replies().iter().map(|r| protocol::decode_reply(r).expect("a reply")));
        if frames.is_empty() {
            if !io.waiting() {
                replies.extend(io.take_replies().iter().map(|r| protocol::decode_reply(r).expect("a reply")));
                return replies;
            }
            // Only timers left: go to the next one (or now, if it is due).
            match io.next_due() {
                Some(Ms(t)) => {
                    *now = (*now).max(t);
                    io.tick(Ms(*now));
                    continue;
                }
                None => panic!("settle: the page is waiting with no timer to wake it"),
            }
        }
        *now += 1;
        for f in frames {
            let answer = node.serve(&f);
            for push in std::mem::take(&mut node.pushes) {
                io.inbound(&push, Ms(*now));
            }
            if let Some(answer) = answer {
                io.inbound(&answer, Ms(*now));
            }
        }
    }
    panic!("settle: the page did not settle in 2,000 rounds (still waiting: {:?})", io.not_answering());
}

/// A page the test KEEPS from settling (a signer that never answers, register reads refused or failed for ever, a
/// corrupted block re-asked): the clock runs timer to timer for 2,000 rounds and stops there -- what every settle
/// did before it ended at quiescence. Named, so a count read after it says how long it ran.
fn run_unsettled(io: &mut PageIo, node: &mut WireNode, now: &mut u64) -> Vec<Reply> {
    let mut replies = Vec::new();
    for _ in 0..2_000 {
        let frames = io.take_frames();
        replies.extend(io.take_replies().iter().map(|r| protocol::decode_reply(r).expect("a reply")));
        if frames.is_empty() {
            match io.next_due() {
                Some(Ms(t)) => {
                    *now = (*now).max(t);
                    io.tick(Ms(*now));
                    continue;
                }
                None => break,
            }
        }
        *now += 1;
        node.now = *now;
        for f in frames {
            let answer = node.serve(&f);
            for push in std::mem::take(&mut node.pushes) {
                io.inbound(&push, Ms(*now));
            }
            if let Some(answer) = answer {
                io.inbound(&answer, Ms(*now));
            }
        }
    }
    replies.extend(io.take_replies().iter().map(|r| protocol::decode_reply(r).expect("a reply")));
    replies
}

/// A request to a page the test keeps from settling ([`run_unsettled`]).
fn client_unsettled(io: &mut PageIo, node: &mut WireNode, now: &mut u64, r: &Request) -> Vec<Reply> {
    io.client(&protocol::encode_session_request(4, 9, r).expect("encodes"));
    run_unsettled(io, node, now)
}

#[track_caller]
fn client(io: &mut PageIo, node: &mut WireNode, now: &mut u64, r: &Request) -> Vec<Reply> {
    io.client(&protocol::encode_session_request(4, 9, r).expect("encodes"));
    settle(io, node, now)
}

fn write(id: u64, k: &str, v: &str) -> Request {
    Request::forced_write(id, vec![protocol::Op::Put(k.as_bytes().to_vec(), v.as_bytes().to_vec())])
}

fn states(rs: &[Reply], id: u64) -> Vec<WriteState> {
    rs.iter()
        .filter_map(|r| match r {
            Reply::WriteState { write_id, state } | Reply::SessionWriteState { write_id, state, .. } if *write_id == id => Some(*state),
            _ => None,
        })
        .collect()
}

#[test]
fn a_write_publishes_through_real_frames_and_the_first_head_creates_the_register() {
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 1_000;
    let id = client(&mut io, &mut node, &mut now, &Request::Identity);
    assert!(id.iter().any(|r| matches!(r, Reply::Identity { head_writable: true, .. })), "{id:?}");
    let r = client(&mut io, &mut node, &mut now, &write(1, "a", "1"));
    assert!(states(&r, 1).contains(&WriteState::Published), "{r:?}; unusable {:?}", io.unusable());
    assert_eq!(node.register_puts, 1, "the first head did not CREATE the register by PUT");
    let r = client(&mut io, &mut node, &mut now, &write(2, "b", "2"));
    assert!(states(&r, 2).contains(&WriteState::Published), "{r:?}");
    assert_eq!(node.register_puts, 1, "a later head was PUT again instead of UPDATEd");
    assert!(node.served.get("update").is_some_and(|n| *n >= 1), "no UPDATE was ever sent: {:?}", node.served);
    for k in ["put block", "get register", "signer"] {
        assert!(node.served.contains_key(k), "no {k} crossed the wire: {:?}", node.served);
    }
    assert_eq!(node.head().map(|h| h.0), Some(2));
    assert!(io.unusable().is_empty(), "{:?}", io.unusable());
}

/// THE FIRST-PUT RACE: two pages on ONE key, neither has seen a register, both
/// write at once. The signer signs ONE record from the genesis; the second page
/// is answered `AlreadySigned` with it and PUTs the same bytes — an equal
/// decision, held bytes win (F56). The loser's write is told `Lost`, re-sent,
/// and publishes on top.
#[test]
fn two_pages_racing_the_first_put_create_one_register() {
    let mut node = WireNode::new(&[4u8; 32]);
    let (mut a, mut b) = (page_io(&node), page_io(&node));
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    // Page b is its own page LOAD: its own session (and so its own device id
    // in heads' `through`, COMMIT-LIFE ⁵ -- two pages under one session would
    // read each other's `through` as their own).
    b.client(&protocol::encode_session_request(4, 10, &Request::Identity).expect("encodes"));
    settle(&mut b, &mut node, &mut now);
    // Both write before either has heard anything back: interleave their frames.
    a.client(&protocol::encode_session_request(4, 9, &write(1, "a", "1")).expect("encodes"));
    b.client(&protocol::encode_session_request(4, 10, &write(1, "b", "1")).expect("encodes"));
    let (mut ra, mut rb) = (Vec::new(), Vec::new());
    for _ in 0..400 {
        let (fa, fb) = (a.take_frames(), b.take_frames());
        ra.extend(a.take_replies().iter().map(|r| protocol::decode_reply(r).expect("reply")));
        rb.extend(b.take_replies().iter().map(|r| protocol::decode_reply(r).expect("reply")));
        if fa.is_empty() && fb.is_empty() {
            let due = [a.next_due(), b.next_due()].into_iter().flatten().map(|m| m.0).filter(|t| *t > now).min();
            match due {
                Some(t) => {
                    now = t;
                    a.tick(Ms(now));
                    b.tick(Ms(now));
                    continue;
                }
                None => break,
            }
        }
        now += 1;
        for f in fa {
            if let Some(ans) = node.serve(&f) {
                a.inbound(&ans, Ms(now));
            }
        }
        for f in fb {
            if let Some(ans) = node.serve(&f) {
                b.inbound(&ans, Ms(now));
            }
        }
    }
    let (sa, sb) = (states(&ra, 1), states(&rb, 1));
    let published = [&sa, &sb].iter().filter(|s| s.contains(&WriteState::Published)).count();
    let lost = [&sa, &sb].iter().filter(|s| s.contains(&WriteState::Lost)).count();
    assert_eq!((published, lost), (1, 1), "a: {sa:?}; b: {sb:?}");
    assert_eq!(node.head().map(|h| h.0), Some(1), "the race made more than one head at seq 1");
    // The loser re-sends and publishes on top of the winner.
    let (loser, sess) = if sa.contains(&WriteState::Lost) { (&mut a, 9) } else { (&mut b, 10) };
    loser.client(&protocol::encode_session_request(4, sess, &write(2, "late", "2")).expect("encodes"));
    let r = settle(loser, &mut node, &mut now);
    assert!(states(&r, 2).contains(&WriteState::Published), "the race's loser never published: {r:?}");
    assert_eq!(node.head().map(|h| h.0), Some(2));
}

/// sdk#175: a head read that FAILS is silence, never "no head". A page that
/// reopens on a node whose register exists, and whose first head reads fail,
/// must not open an EMPTY tree: it asks again on its RTO and reads the rows.
#[test]
fn a_failed_head_read_never_opens_an_empty_tree() {
    let mut node = WireNode::new(&[6u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    for (n, k) in ["x", "y", "z"].iter().enumerate() {
        let r = client(&mut a, &mut node, &mut now, &write(n as u64 + 1, k, "v"));
        assert!(states(&r, n as u64 + 1).contains(&WriteState::Published));
    }
    // A second page reopens; its first three head reads fail.
    node.fail_register_gets = 3;
    let mut b = page_io(&node);
    client(&mut b, &mut node, &mut now, &Request::Identity);
    let range = Request::Range { req_id: 5, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: None, max_entries: 100 };
    let r = client(&mut b, &mut node, &mut now, &range);
    let pages: Vec<usize> = r.iter().filter_map(|x| if let Reply::Page { req_id: 5, entries, .. } = x { Some(entries.len()) } else { None }).collect();
    assert_eq!(node.fail_register_gets, 0, "the failing reads were never asked again");
    assert_eq!(pages, vec![3], "the reopened page read {pages:?} rows: a failed head read opened an empty tree");
}

/// Main's ruling on sdk#175: a register read answered NotFound (a peered node
/// can say so FALSELY, F55) on an app that EXISTS must never open an empty
/// tree — even for a page that just provisioned (the same key again). The
/// signer holds a record, so the NotFound is only silence.
#[test]
fn a_false_not_found_on_an_existing_app_never_opens_an_empty_tree() {
    let mut node = WireNode::new(&[7u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    for (n, k) in ["p", "q"].iter().enumerate() {
        assert!(states(&client(&mut a, &mut node, &mut now, &write(n as u64 + 1, k, "v")), n as u64 + 1).contains(&WriteState::Published));
    }
    // A reopened page that PROVISIONS (the same key, accepted) and whose first
    // two head reads are answered "not found".
    node.fail_register_gets = 2;
    let mut b = page_io(&node);
    let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    let (container, _) = wire::delegate_from_code(SIGNER_CODE);
    b.provision(container, sk.to_bytes().to_vec());
    client(&mut b, &mut node, &mut now, &Request::Identity);
    let range = Request::Range { req_id: 6, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: None, max_entries: 100 };
    let r = client(&mut b, &mut node, &mut now, &range);
    let pages: Vec<usize> = r.iter().filter_map(|x| if let Reply::Page { req_id: 6, entries, .. } = x { Some(entries.len()) } else { None }).collect();
    assert_eq!(pages, vec![2], "a false NotFound on an existing app opened {pages:?}");
}

/// ONLY NotFound means absent (#332 ruling): a node REFUSES the first head reads (`ContractError::Get`) of an app
/// that EXISTS, on a FRESH signer — one that holds no record, the very case where a NotFound opens an empty tree. A
/// refusal says nothing about the head: it is re-asked on the RTO and the app's rows are read, never an empty tree
/// whose first commit would sign seq 1 over the real head.
#[test]
fn a_refused_head_read_on_a_fresh_signer_is_re_asked_never_an_empty_tree() {
    let seed = [9u8; 32];
    let mut x = WireNode::new(&seed);
    let mut a = page_io(&x);
    let mut now = 1_000;
    client(&mut a, &mut x, &mut now, &Request::Identity);
    for (n, k) in ["p", "q"].iter().enumerate() {
        assert!(states(&client(&mut a, &mut x, &mut now, &write(n as u64 + 1, k, "v")), n as u64 + 1).contains(&WriteState::Published));
    }
    // Another node of the same identity: its signer has signed nothing; the network holds the head and blocks.
    let mut y = WireNode::new(&seed);
    y.contracts = x.contracts.clone();
    y.head_not_local = true;
    // Every read refused while the page opens and is asked for its rows: nothing may open.
    y.refuse_gets = usize::MAX;
    let mut b = page_io(&y);
    client_unsettled(&mut b, &mut y, &mut now, &Request::Identity);
    let range = Request::Range { req_id: 8, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: None, max_entries: 100 };
    let pages = |r: &[Reply]| r.iter().filter_map(|x| if let Reply::Page { req_id: 8, entries, .. } = x { Some(entries.len()) } else { None }).collect::<Vec<_>>();
    let during = pages(&client_unsettled(&mut b, &mut y, &mut now, &range));
    let refused = usize::MAX - y.refuse_gets;
    assert!(refused >= 2, "THE SETUP: the refusal was not re-asked ({refused} refused)");
    assert_eq!(during, Vec::<usize>::new(), "{refused} refused head reads opened a tree: {during:?}");
    // The node answers again: the SAME open reads the app's rows.
    y.refuse_gets = 0;
    let after = pages(&settle(&mut b, &mut y, &mut now));
    assert_eq!(after, vec![2], "once answered, the page read {after:?}");
}

/// ONLY NotFound means absent, for a BLOCK too: a refused block GET is re-asked on the RTO and the read completes.
/// For a RANGE the engine re-asks a MISS as well, so here the old mapping (refusal → miss) ends in the same place:
/// this one is a regression test; the delta test below is the one the mutant fails.
#[test]
fn a_refused_block_read_is_re_asked_not_missed() {
    let mut node = WireNode::new(&[10u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    for (n, k) in ["p", "q"].iter().enumerate() {
        assert!(states(&client(&mut a, &mut node, &mut now, &write(n as u64 + 1, k, "v")), n as u64 + 1).contains(&WriteState::Published));
    }
    let mut b = page_io(&node);
    client(&mut b, &mut node, &mut now, &Request::Identity);
    let before = node.served.get("get block").copied().unwrap_or(0);
    node.refuse_gets = 2;
    let range = Request::Range { req_id: 9, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: None, max_entries: 100 };
    let r = client(&mut b, &mut node, &mut now, &range);
    let pages: Vec<usize> = r.iter().filter_map(|x| if let Reply::Page { req_id: 9, entries, .. } = x { Some(entries.len()) } else { None }).collect();
    assert!(node.served.get("get block").copied().unwrap_or(0) > before, "THE SETUP: the rows were not read by block GETs");
    assert_eq!(node.served.get("refused get"), Some(&2), "THE SETUP: the refusals were not all asked");
    assert_eq!(pages, vec![2], "a refused block read gave {pages:?}: {r:?}");
}

/// THE KILLING CASE for a block: a DELTA read turns a block MISS into `FullReloadRequired` at once (a NotFound block
/// of a delta is most likely an old root's, gone for good). So a REFUSED block read, if read as a miss, ends a delta
/// the next ask would have served; read as it is — not an answer — it is re-asked and the delta is computed.
#[test]
fn a_refused_block_under_a_delta_is_re_asked_never_a_full_reload() {
    let mut node = WireNode::new(&[11u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    for (n, k) in ["p", "q"].iter().enumerate() {
        assert!(states(&client(&mut a, &mut node, &mut now, &write(n as u64 + 1, k, "v")), n as u64 + 1).contains(&WriteState::Published));
    }
    let range = Request::Range { req_id: 9, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: None, max_entries: 100 };
    let from = client(&mut a, &mut node, &mut now, &range)
        .iter()
        .find_map(|x| if let Reply::Page { req_id: 9, at, .. } = x { Some(at.root) } else { None })
        .expect("THE SETUP: the range was read");
    assert!(states(&client(&mut a, &mut node, &mut now, &write(3, "r", "v")), 3).contains(&WriteState::Published));
    // A page that has read nothing: the delta needs both trees' blocks from the node.
    let mut b = page_io(&node);
    client(&mut b, &mut node, &mut now, &Request::Identity);
    node.refuse_block_gets = 2;
    let since = Request::ChangesSince { req_id: 12, from, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, max_entries: 100 };
    let r = client(&mut b, &mut node, &mut now, &since);
    assert_eq!(node.refuse_block_gets, 0, "THE SETUP: the delta's block reads were not refused");
    assert!(!r.iter().any(|x| matches!(x, Reply::FullReloadRequired { req_id: 12, .. })), "a refused block read ended the delta: {r:?}");
    let changes = r.iter().find_map(|x| if let Reply::Delta { req_id: 12, changes, .. } = x { Some(changes.clone()) } else { None });
    assert_eq!(changes.map(|c| c.into_iter().map(|(k, _)| k).collect::<Vec<_>>()), Some(vec![b"r".to_vec()]), "the delta after the refusals: {r:?}");
}

/// THE CONTROL: a genuinely NEW app (the signer holds no record) whose head
/// read is "not found" does open its empty tree — and publishes onto it.
#[test]
fn control_a_new_apps_not_found_is_no_head() {
    let mut node = WireNode::new(&[8u8; 32]);
    // The signer is provisioned but has signed nothing.
    let mut io = page_io(&node);
    let mut now = 1_000;
    client(&mut io, &mut node, &mut now, &Request::Identity);
    let range = Request::Range { req_id: 7, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: None, max_entries: 100 };
    let r = client(&mut io, &mut node, &mut now, &range);
    let pages: Vec<usize> = r.iter().filter_map(|x| if let Reply::Page { req_id: 7, entries, .. } = x { Some(entries.len()) } else { None }).collect();
    assert_eq!(pages, vec![0], "a new app did not open its (empty) tree: {r:?}");
}

/// Frames only, NO clock: an op re-sent on its RTO, or the idle backstop,
/// cannot be what moved the page (sdk#225's hint test must not pass on the
/// backstop).
fn pump(io: &mut PageIo, node: &mut WireNode, now: &mut u64) -> Vec<Reply> {
    let mut replies = Vec::new();
    for _ in 0..500 {
        let frames = io.take_frames();
        replies.extend(io.take_replies().iter().map(|r| protocol::decode_reply(r).expect("a reply")));
        if frames.is_empty() {
            break;
        }
        *now += 1;
        node.now = *now;
        for f in frames {
            let answer = node.serve(&f);
            for push in std::mem::take(&mut node.pushes) {
                io.inbound(&push, Ms(*now));
            }
            if let Some(answer) = answer {
                io.inbound(&answer, Ms(*now));
            }
        }
    }
    replies.extend(io.take_replies().iter().map(|r| protocol::decode_reply(r).expect("a reply")));
    replies
}

/// sdk#225's RELOAD TRIGGER through real frames: another tab moves the
/// register; the node pushes `UpdateNotification` for it to the idle tab,
/// which READS the register and adopts what it holds — with no clock
/// passing, so neither a re-send nor the backstop is what moved it — and then
/// reads the other tab's row.
#[test]
fn a_head_changed_from_the_node_makes_an_idle_page_read_and_adopt_the_new_head() {
    let mut node = WireNode::new(&[8u8; 32]);
    let (mut a, mut b) = (page_io(&node), page_io(&node));
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    assert!(states(&client(&mut a, &mut node, &mut now, &write(1, "a", "1")), 1).contains(&WriteState::Published));
    client(&mut b, &mut node, &mut now, &Request::Identity);
    assert!(states(&client(&mut b, &mut node, &mut now, &write(1, "b", "2")), 1).contains(&WriteState::Published));
    assert_eq!(node.head().map(|h| h.0), Some(2));
    // `a` is idle at seq 1. The node's push:
    let key = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(REGISTER_CODE.to_vec())),
        Parameters::from(node.register_params.clone()),
    )))
    .key();
    let state = node.contracts.get(&node.register_id).cloned().expect("a register");
    let push = ok(HostResponse::ContractResponse(ContractResponse::UpdateNotification {
        key,
        update: UpdateData::State(State::from(state)),
    }));
    a.inbound(&push, Ms(now));
    pump(&mut a, &mut node, &mut now);
    a.client(&protocol::encode_session_request(4, 9, &Request::Get { req_id: 7, key: b"b".to_vec() }).expect("encodes"));
    let r = pump(&mut a, &mut node, &mut now);
    let got = r.iter().any(|x| matches!(x, Reply::Value { req_id: 7, value: Some(v) } if v == b"2"));
    assert!(got, "the idle tab did not adopt the head the node pushed: {r:?}");
}

/// sdk#376: after a RECONNECT the page is subscribed again AT ONCE. The head
/// read (a GET with subscribe, the one path) goes out in the SAME call as the
/// reconnect -- no clock passes, so neither a re-send nor the 120 s backstop is
/// what sends it -- the subscription reads unanswered until the node answers
/// it, and a push after the reconnect reaches the page, which adopts the head.
#[test]
fn a_reconnect_re_subscribes_the_head_at_once_and_a_push_after_it_reaches_the_page() {
    let mut node = WireNode::new(&[31u8; 32]);
    let (mut a, mut b) = (page_io(&node), page_io(&node));
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    assert!(states(&client(&mut a, &mut node, &mut now, &write(1, "a", "1")), 1).contains(&WriteState::Published));
    assert!(a.head_subscription().answered, "THE SETUP: the page was never subscribed");
    assert!(a.take_frames().is_empty(), "THE SETUP: the page was not idle");

    // The socket dropped and a new one opened: the node's subscription went with the old one.
    let served_before = node.served.get("get register").copied().unwrap_or(0);
    a.reconnected(Ms(now));
    assert!(!a.head_subscription().answered, "a subscription the old socket held was still reported answered on the new one");
    let frames = a.take_frames();
    assert!(!frames.is_empty(), "nothing was sent on the reconnect: the page waits for the 120 s backstop");
    for f in frames {
        if let Some(answer) = node.serve(&f) {
            a.inbound(&answer, Ms(now));
        }
    }
    let served_after = node.served.get("get register").copied().unwrap_or(0);
    // `WireNode` asserts `subscribe` on every head GET: this one carried it.
    assert_eq!(served_after, served_before + 1, "the reconnect did not read the head (with subscribe) at once");
    pump(&mut a, &mut node, &mut now);
    assert!(a.head_subscription().answered, "the re-sent head read was answered and the subscription still not reported");

    // A push AFTER the reconnect reaches the page: another tab moves the head, the node notifies `a`.
    client(&mut b, &mut node, &mut now, &Request::Identity);
    assert!(states(&client(&mut b, &mut node, &mut now, &write(1, "b", "2")), 1).contains(&WriteState::Published));
    let key = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(REGISTER_CODE.to_vec())),
        Parameters::from(node.register_params.clone()),
    )))
    .key();
    let state = node.contracts.get(&node.register_id).cloned().expect("a register");
    let push = ok(HostResponse::ContractResponse(ContractResponse::UpdateNotification { key, update: UpdateData::State(State::from(state)) }));
    let changes = a.head_subscription().changes;
    a.inbound(&push, Ms(now));
    pump(&mut a, &mut node, &mut now);
    assert_eq!(a.head_subscription().changes, changes + 1, "the push after the reconnect was not counted as delivered");
    a.client(&protocol::encode_session_request(4, 9, &Request::Get { req_id: 7, key: b"b".to_vec() }).expect("encodes"));
    let r = pump(&mut a, &mut node, &mut now);
    assert!(r.iter().any(|x| matches!(x, Reply::Value { req_id: 7, value: Some(v) } if v == b"2")), "the page did not adopt the head pushed after the reconnect: {r:?}");
}

/// An APP's PUT (builder#104: a web container) goes out through page-io — the
/// only path to the node — beside the page's own writes, and its answer comes
/// back to the PAGE by key, which ends the PUT it sent: never taken as one of
/// the page's block or register answers, never dropped.
#[test]
fn an_apps_put_crosses_the_wire_and_its_answer_is_handed_back_by_key() {
    let mut node = WireNode::new(&[5u8; 32]);
    let mut io = page_io(&node);
    let mut now = 1_000;
    client(&mut io, &mut node, &mut now, &Request::Identity);
    let (key, c, state) = wire::puts::contract(b"an app's contract", b"its params", b"its state");
    let cid = id_of(&c.key());
    io.put_contract(c, state, Ms(now)).expect("frames");
    let r = client(&mut io, &mut node, &mut now, &write(1, "a", "1"));
    assert!(states(&r, 1).contains(&WriteState::Published), "the page's own write did not publish beside it: {r:?}");
    assert_eq!(node.contracts.get(&cid).map(Vec::as_slice), Some(b"its state".as_slice()), "the app's PUT never reached the node");
    // The PAGE sent it, so the page takes its answer: the PUT ENDED, as put.
    assert_eq!(io.app_put(&key), Some(&page::AppPut::Put), "the app's ack did not end its PUT");
    assert!(io.take_others().is_empty(), "the app's own ack was handed back as somebody else's");
    assert!(io.unusable().is_empty(), "{:?}", io.unusable());
}

/// A PUT refusal NAMING a contract this page never PUT is handed back; one
/// naming the app's PUT ends it; one naming the page's own register is the
/// page's (reported), not handed back.
#[test]
fn a_put_refusal_goes_to_whoever_owns_the_contract_it_names() {
    use freenet_stdlib::client_api::{ContractError, ErrorKind, RequestError};
    let node = WireNode::new(&[6u8; 32]);
    let mut io = page_io(&node);
    let refusal = |key: ContractKey| {
        let e: Err = ErrorKind::RequestError(RequestError::ContractError(ContractError::Put { key, cause: "invalid put".into() })).into();
        bincode::serialize(&Err::<HostResponse, Err>(e)).expect("encodes")
    };
    // A contract this page never PUT: handed back unread.
    let (key, c, _) = wire::puts::contract(b"an app's contract", b"p", b"s");
    io.inbound(&refusal(c.key()), Ms(1));
    assert_eq!(io.take_others(), vec![wire::Incoming::PutFailed { key, said: "invalid put".into() }]);
    assert!(io.unusable().is_empty(), "{:?}", io.unusable());
    // One it did PUT: the refusal ENDS it, in the node's words.
    let (mine, c, s) = wire::puts::contract(b"an app's contract", b"mine", b"s");
    io.put_contract(c.clone(), s, Ms(1)).expect("frames");
    io.inbound(&refusal(c.key()), Ms(2));
    assert_eq!(io.app_put(&mine), Some(&page::AppPut::Refused("invalid put".into())));
    assert!(io.take_others().is_empty(), "the app's own refusal was handed back as somebody else's");

    let register = ContractKey::from_id_and_code(
        ContractInstanceId::new(node.register_id),
        CodeHash::new([0u8; 32]),
    );
    io.inbound(&refusal(register), Ms(3));
    assert!(io.take_others().is_empty(), "the page's own register refusal was handed to the app");
    assert_eq!(io.unusable(), ["the node refused: invalid put".to_string()]);
}

fn reader(node: &WireNode) -> PageIo {
    PageIo::reader(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
        BLOCK_CODE.to_vec(),
        node.register_id,
        1,
    )
}

#[track_caller]
fn rows(io: &mut PageIo, node: &mut WireNode, now: &mut u64, req_id: u64) -> Vec<usize> {
    let range = Request::Range { req_id, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: None, max_entries: 100 };
    let r = client(io, node, now, &range);
    r.iter().filter_map(|x| if let Reply::Page { req_id: q, entries, .. } = x { (*q == req_id).then_some(entries.len()) } else { None }).collect()
}

/// [`rows`] on a page the test keeps from settling ([`run_unsettled`]).
fn rows_unsettled(io: &mut PageIo, node: &mut WireNode, now: &mut u64, req_id: u64) -> Vec<usize> {
    let range = Request::Range { req_id, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: None, max_entries: 100 };
    let r = client_unsettled(io, node, now, &range);
    r.iter().filter_map(|x| if let Reply::Page { req_id: q, entries, .. } = x { (*q == req_id).then_some(entries.len()) } else { None }).collect()
}

/// A PUBLISHED HEAD READ BY ANOTHER USER (sdk#239): a reader of the named
/// register sees the publisher's rows, says the head is NOT writable, and —
/// the addition main asked for — leaves NO trace on the node it reads from:
/// no delegate registered, no signer message, no PUT, no UPDATE. Only reads.
#[test]
fn a_reader_of_a_named_head_sees_the_rows_and_leaves_no_trace_on_the_node() {
    let mut node = WireNode::new(&[9u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    for (n, k) in ["x", "y", "z"].iter().enumerate() {
        assert!(states(&client(&mut a, &mut node, &mut now, &write(n as u64 + 1, k, "v")), n as u64 + 1).contains(&WriteState::Published));
    }
    let before = node.served.clone();

    let mut v = reader(&node);
    assert!(v.read_only() && v.provisioned(), "a reader must be ready with nothing to provision");
    let id = client(&mut v, &mut node, &mut now, &Request::Identity);
    assert!(id.iter().any(|r| matches!(r, Reply::Identity { head_writable: false, head_id, .. } if *head_id == node.register_id)), "{id:?}");
    assert_eq!(rows(&mut v, &mut node, &mut now, 11), vec![3], "the reader did not see the owner's rows");

    // What the READER made the node do: reads, and nothing else.
    let reader: BTreeMap<&str, usize> = node.served.iter().map(|(k, n)| (*k, n - before.get(k).copied().unwrap_or(0))).filter(|(_, n)| *n > 0).collect();
    for k in reader.keys() {
        assert!(["get register", "get block"].contains(k), "the reader made the node serve a {k}: {reader:?}");
    }
    assert!(reader.contains_key("get register"), "THE CONTROL: the count saw the reader at all: {reader:?}");

    // The publisher writes again; the reader, reading again, sees it.
    assert!(states(&client(&mut a, &mut node, &mut now, &write(4, "w", "v")), 4).contains(&WriteState::Published));
    v.server.head_hint();
    let _ = settle(&mut v, &mut node, &mut now);
    assert_eq!(rows(&mut v, &mut node, &mut now, 12), vec![4], "the reader never saw the owner's next row");
    assert!(v.unusable().is_empty(), "{:?}", v.unusable());
}

/// The SAFETY NET: a write sent to a reader anyway is never framed — no block
/// PUT, no signer request, no head update reaches the node — and never
/// reported Published. (The UI never offers one: the runtime renders a view.)
#[test]
fn a_write_to_a_reader_reaches_nothing_and_never_publishes() {
    let mut node = WireNode::new(&[10u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    assert!(states(&client(&mut a, &mut node, &mut now, &write(1, "x", "v")), 1).contains(&WriteState::Published));
    let mut v = reader(&node);
    client(&mut v, &mut node, &mut now, &Request::Identity);
    let before = node.served.clone();
    let head = node.head();
    let r = client(&mut v, &mut node, &mut now, &write(2, "intruder", "v"));
    assert!(!states(&r, 2).contains(&WriteState::Published), "a reader's write was reported Published: {r:?}");
    for k in ["put block", "put register", "update", "signer", "register delegate"] {
        assert_eq!(node.served.get(k), before.get(k), "a reader's write made the node serve a {k}");
    }
    assert_eq!(node.head(), head, "a reader's write moved the publisher's head");
    // And provisioning a reader is refused, not sent. Frames already queued
    // (a block GET the write's apply asked for -- a READ, which a reader may
    // make) are drained first, so what is checked is exactly what the
    // provisioning call framed.
    let _ = v.take_frames();
    let (container, _) = wire::delegate_from_code(SIGNER_CODE);
    // What was already queued is the view's own READS (nothing it may not
    // send); what `provision` adds is what this asks about.
    let queued = v.take_frames();
    assert!(kinds(&queued).iter().all(|k| *k == "other"), "a reader had a delegate frame queued: {:?}", kinds(&queued));
    v.provision(container, vec![0u8; 32]);
    let fr = v.take_frames();
    assert!(fr.is_empty(), "a reader framed a provisioning");
    assert!(v.unusable().iter().any(|u| u.contains("read-only")), "{:?}", v.unusable());
}

/// A failed read of a NAMED head is silence, re-asked — never an empty view,
/// and the reader asks no signer (it has none).
#[test]
fn a_readers_failed_head_read_is_silence_never_an_empty_view() {
    let mut node = WireNode::new(&[11u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    assert!(states(&client(&mut a, &mut node, &mut now, &write(1, "x", "v")), 1).contains(&WriteState::Published));
    node.fail_register_gets = 3;
    let signer_before = node.served.get("signer").copied();
    let mut v = reader(&node);
    client(&mut v, &mut node, &mut now, &Request::Identity);
    assert_eq!(rows(&mut v, &mut node, &mut now, 13), vec![1], "a failed named-head read opened an empty view");
    assert_eq!(node.fail_register_gets, 0, "the failing reads were never asked again");
    assert_eq!(node.served.get("signer").copied(), signer_before, "a reader asked a signer");
}

/// The answers to request `req_id`, as Debug text — what a client would see.
fn answers_to(r: &[Reply], req_id: u64) -> Vec<String> {
    r.iter()
        .filter(|x| match x {
            Reply::Page { req_id: q, .. } | Reply::Unavailable { req_id: q, .. } => *q == req_id,
            _ => false,
        })
        .map(|x| format!("{x:?}"))
        .collect()
}

/// ONE READ PATH (the owner's rule on sdk#239): the SAME tree read through
/// the person's OWN page and through a READER gives the same rows — and,
/// with its blocks corrupted on the node, the same refusal. A reader is the
/// same `PageIo`, `Server` and engine with writing switched off, so any
/// difference here is a second read path.
#[test]
fn the_own_page_and_a_reader_read_the_same_tree_identically_intact_and_corrupted() {
    let mut node = WireNode::new(&[12u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    for (n, k) in ["alpha", "beta", "gamma", "delta"].iter().enumerate() {
        assert!(states(&client(&mut a, &mut node, &mut now, &write(n as u64 + 1, k, &format!("value-{n}"))), n as u64 + 1).contains(&WriteState::Published));
    }
    let range = |req_id| Request::Range { req_id, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: None, max_entries: 100 };

    // INTACT: a fresh own page (reopened, same key) and a reader.
    let mut own = page_io(&node);
    client(&mut own, &mut node, &mut now, &Request::Identity);
    let mine = answers_to(&client(&mut own, &mut node, &mut now, &range(21)), 21);
    let mut v = reader(&node);
    client(&mut v, &mut node, &mut now, &Request::Identity);
    let theirs = answers_to(&client(&mut v, &mut node, &mut now, &range(21)), 21);
    // Entries print as bytes: the key `alpha` and the value `value-3`.
    let alpha = format!("{:?}", b"alpha".to_vec());
    let value3 = format!("{:?}", b"value-3".to_vec());
    assert!(mine.len() == 1 && mine[0].contains(&alpha) && mine[0].contains(&value3), "the own page did not read the tree: {mine:?}");
    assert_eq!(theirs, mine, "a reader read the same tree differently from the own page");

    // CORRUPTED: every block's bytes on the node changed (the head intact).
    let reg = node.register_id;
    for (id, st) in node.contracts.iter_mut() {
        if *id != reg {
            let last = st.len() - 1;
            st[last] ^= 0x01;
        }
    }
    let mut own = page_io(&node);
    client_unsettled(&mut own, &mut node, &mut now, &Request::Identity);
    let mine = answers_to(&client_unsettled(&mut own, &mut node, &mut now, &range(22)), 22);
    let mut v = reader(&node);
    client_unsettled(&mut v, &mut node, &mut now, &Request::Identity);
    let theirs = answers_to(&client_unsettled(&mut v, &mut node, &mut now, &range(22)), 22);
    // REFUSED, not an empty page: a block that does not hash to its id is
    // not the block, so nothing is read from it -- never "no rows". And the
    // read does not END on it either (rule 7): the block is asked for again
    // (another copy, a repair), so the answer is NONE yet, identically on
    // both paths.
    assert!(mine.is_empty(), "the own page answered from corrupted blocks: {mine:?}");
    assert_eq!(theirs, mine, "a reader and the own page treated a corrupted tree differently");
}

/// A page that OPENS as the Session does now (`begin`): it knows nothing of
/// the register yet — it asks the signer, and mints only when told "none".
/// Returns the page and how many keys it minted.
fn opening(node: &mut WireNode, now: &mut u64, mint: &[u8; 32], always_mint: bool) -> (PageIo, usize) {
    let (container, signer) = wire::delegate_from_code(SIGNER_CODE);
    let mut io = PageIo::new(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(*now)), SignerFacts::default()),
        Artefacts { block_code: BLOCK_CODE.to_vec(), register_code: REGISTER_CODE.to_vec(), register_params: Vec::new(), signer },
    );
    io.begin(container);
    settle(&mut io, node, now);
    let mut minted = 0;
    if io.needs_key() || always_mint {
        let sk = ed25519_dalek::SigningKey::from_bytes(mint);
        minted += 1;
        if always_mint && !io.needs_key() {
            // THE MUTANT'S PATH: a page that mints whatever the signer said.
            io = PageIo::new(
                Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
                Artefacts { block_code: BLOCK_CODE.to_vec(), register_code: REGISTER_CODE.to_vec(), register_params: wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME), signer: wire::delegate_from_code(SIGNER_CODE).1 },
            );
            io.provision(wire::delegate_from_code(SIGNER_CODE).0, sk.to_bytes().to_vec());
        } else {
            io.provision_with(sk.to_bytes().to_vec(), wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME));
        }
        settle(&mut io, node, now);
    }
    (io, minted)
}

fn row_count(io: &mut PageIo, node: &mut WireNode, now: &mut u64, req_id: u64) -> Option<usize> {
    let range = Request::Range { req_id, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: None, max_entries: 100 };
    client(io, node, now, &range).iter().find_map(|x| match x { Reply::Page { req_id: q, entries, .. } if *q == req_id => Some(entries.len()), _ => None })
}

/// A LOST PUT IS RE-SENT ON THE PAGE'S RTO, whatever clock the page opened at (#386 did not act live).
///
/// The browser's clock is `Date.now()`: EPOCH milliseconds. A page opens (`begin`), provisions, reads its head and
/// writes one row, the node answering each request 1 ms later and the clock moving in 100 ms ticks as a browser
/// page's does -- no long timer jumps, so the page lives on a few samples, as V's did. The node LOSES the write's
/// first block PUT: it is re-sent once its siblings' answers have set the RTO, within a second or two, at the
/// epoch clock as at a small one (THE CONTROL). Measured live (Phase 4 realnet, V's first save): four of five
/// PUTs answered on their first send, the fifth re-sent 59,999 ms after it went.
/// A browser page's life: every frame answered 1 ms later, the clock in 100 ms ticks, for `ms`.
fn live(io: &mut PageIo, node: &mut WireNode, now: &mut u64, ms: u64) -> Vec<Reply> {
    let mut replies = pump(io, node, now);
    for _ in 0..ms / 100 {
        *now += 100;
        io.tick(Ms(*now));
        replies.extend(pump(io, node, now));
    }
    replies
}

/// A page opened at clock `start` on a fresh node, provisioned, and asked its identity: ready to write.
fn opened_at(start: u64, label: &str) -> (PageIo, WireNode, u64) {
    let key = [31u8; 32];
    let mut node = WireNode::unprovisioned(&key);
    let mut now = start;
    let (container, signer) = wire::delegate_from_code(SIGNER_CODE);
    let mut io = PageIo::new(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(now)), SignerFacts::default()),
        Artefacts { block_code: BLOCK_CODE.to_vec(), register_code: REGISTER_CODE.to_vec(), register_params: Vec::new(), signer },
    );
    io.begin(container);
    live(&mut io, &mut node, &mut now, 1_000);
    if io.needs_key() {
        let sk = ed25519_dalek::SigningKey::from_bytes(&key);
        io.provision_with(sk.to_bytes().to_vec(), wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME));
        live(&mut io, &mut node, &mut now, 1_000);
    }
    assert!(io.provisioned(), "{label}: THE SETUP: the page did not provision");
    io.client(&protocol::encode_session_request(4, 9, &Request::Identity).expect("encodes"));
    live(&mut io, &mut node, &mut now, 1_000);
    (io, node, now)
}

#[test]
fn a_lost_put_is_re_sent_on_the_rto_when_the_page_opened_at_an_epoch_clock() {
    for (label, start) in [("small clock (THE CONTROL)", 1_000u64), ("epoch clock (the browser's)", 1_790_253_181_367)] {
        let (mut io, mut node, mut now) = opened_at(start, label);
        let before = node.block_puts.len();
        node.lose_block_puts = 1;
        io.client(&protocol::encode_session_request(4, 9, &write(1, "a", "1")).expect("encodes"));
        let r = live(&mut io, &mut node, &mut now, 70_000);
        assert!(states(&r, 1).contains(&WriteState::Published), "{label}: THE SETUP: the write did not publish: {r:?}");
        let puts = &node.block_puts[before..];
        let lost = puts.first().expect("THE SETUP: the write PUT no block").1;
        let sends: Vec<u64> = puts.iter().filter(|(_, id)| *id == lost).map(|(t, _)| *t).collect();
        let (rto, srtt, _) = io.server.page.clock();
        println!("  {label}: {} block PUTs; the lost one reached the node {:?} ms after its first send; page clock rto {rto} ms, srtt {srtt:?}", puts.len(), sends.iter().map(|t| t - sends[0]).collect::<Vec<_>>());
        assert!(sends.len() >= 2, "{label}: THE SETUP: the lost PUT was never re-sent");
        assert!(sends[1] - sends[0] < 5_000, "{label}: the lost PUT waited {} ms for its re-send (rto {rto} ms, srtt {srtt:?})", sends[1] - sends[0]);
    }
}

/// A BLOCK PUT THE NODE ANSWERS ONLY WITH AN ERROR (sdk#431, seen live on 0.2.136: a parity PUT answered three times
/// with "No such file or directory (os error 2)", never re-sent, and the publish's rows read backed up). An error that
/// is not the contract's own refusal ENDS NOTHING (rule 7): the PUT is not acked, the write never reaches
/// ParityComplete, and the page re-sends it on the RTO -- whether the node's error names the contract (a keyed
/// `ContractError::Put`) or names nothing (the live bytes, `OperationError`). The page's recording shows the PUT's
/// sends and no Withdrawn among them. THE CONTROL: the same errors, then the node takes the PUT -- the write is
/// ParityComplete, so the setup can reach it.
#[test]
fn a_block_put_answered_only_with_an_error_is_re_sent_and_never_backed_up() {
    for keyed in [true, false] {
        for (label, errors) in [("2 errors, then taken (THE CONTROL)", 2usize), ("errors only", usize::MAX)] {
            let label = format!("{} error, {label}", if keyed { "keyed" } else { "keyless (the live bytes)" });
            let (mut io, mut node, mut now) = opened_at(1_790_253_181_367, &label);
            io.server.page.record_into(1 << 16);
            let before = node.block_puts.len();
            node.err_block_puts = errors;
            node.err_keyed = keyed;
            io.client(&protocol::encode_session_request(4, 9, &write(1, "a", "1")).expect("encodes"));
            let r = live(&mut io, &mut node, &mut now, 70_000);
            let failed = node.err_block.unwrap_or_else(|| panic!("{label}: THE SETUP: no block PUT was answered with an error"));
            let sends = node.block_puts[before..].iter().filter(|(_, id)| *id == failed).count();
            let errored = node.served.get("put block answered with an error").copied().unwrap_or(0);
            let dump = io.server.page.dump(usize::MAX);
            let withdrawn = dump.lines().filter(|l| l.contains("page::op::put") && l.contains("Withdrawn")).count();
            println!("  {label}: the errored block reached the node {sends} time(s), {errored} answered with the error; states {:?}; put Withdrawn in the recording: {withdrawn}", states(&r, 1));
            assert_eq!(withdrawn, 0, "{label}: a block PUT was WITHDRAWN:\n{dump}");
            if errors == usize::MAX {
                assert!(!states(&r, 1).contains(&WriteState::ParityComplete), "{label}: BACKED UP (ParityComplete) with a block the node only ever answered with an error");
                assert!(sends >= 3, "{label}: the errored PUT was not re-sent on the RTO after the error ({sends} send(s) in 70 s)");
            } else {
                assert!(states(&r, 1).contains(&WriteState::ParityComplete), "{label}: THE CONTROL: the write never reached ParityComplete once the node took the PUT: {:?}", states(&r, 1));
            }
        }
    }
}

/// OUR BLOCK REJECTED BY THE NODE'S BLOCK CONTRACT (sdk#433; the architect's rulings): the node answers the first
/// block PUT with a pinned VALIDATION refusal -- each of `wire::VALIDATION_REFUSED` -- so it is FINAL: that block is
/// sent ONCE and never again, the commit that needs it ends, its write is told `Failed` (never `Lost`: re-sent it
/// would re-derive the same block), and the refusal is named. (An unrecognised keyed refusal stays transient and is
/// re-sent: the test above, sdk#431's pin.)
#[test]
fn a_block_the_nodes_contract_rejects_is_sent_once_and_its_write_fails() {
    for (text_form, cause) in [false, true].into_iter().flat_map(|t| wire::VALIDATION_REFUSED.into_iter().map(move |c| (t, c))) {
        let (mut io, mut node, mut now) = opened_at(1_790_253_181_367, cause);
        node.err_text_form = text_form;
        let before = node.block_puts.len();
        node.err_block_puts = usize::MAX;
        node.err_keyed = true;
        node.err_cause = cause;
        node.err_any_block = true;
        io.client(&protocol::encode_session_request(4, 9, &write(1, "a", "1")).expect("encodes"));
        let r = live(&mut io, &mut node, &mut now, 70_000);
        let rejected = node.err_block.unwrap_or_else(|| panic!("{cause:?}: THE SETUP: no block PUT was answered with the refusal"));
        let sends = node.block_puts[before..].iter().filter(|(_, id)| *id == rejected).count();
        println!("  {cause:?}: the rejected block reached the node {sends} time(s); states {:?}; unusable {:?}", states(&r, 1), io.unusable());
        assert_eq!(sends, 1, "{cause:?}: a block the node's contract rejected was sent again ({sends} sends in 70 s)");
        assert!(states(&r, 1).contains(&WriteState::FailedWhy { why: protocol::FailWhy::BlockRejected }), "{cause:?}: the write needing a rejected block was not told Failed, as BLOCK_REJECTED: {:?}", states(&r, 1));
        assert!(!states(&r, 1).iter().any(|s| matches!(s, WriteState::Lost | WriteState::Published)), "{cause:?}: told {:?}", states(&r, 1));
        assert!(io.unusable().iter().any(|u| u.contains("refused block") && u.contains(cause)), "{cause:?}: the rejection was not named: {:?}", io.unusable());
    }
}

/// THE NODE'S TEXT NEVER NAMES AN OP WE DO NOT HAVE, AND ONLY A PINNED REASON IS FINAL (sdk#433, the architect's
/// conditions on the keyless text form): (a) the pinned format naming ANOTHER contract's key is unattributed --
/// counted, and it ends nothing (the write still publishes); (b) the format naming OUR block with a reason that is
/// not a pinned validation refusal is attributed but transient -- re-sent on the RTO, never `Failed`.
#[test]
fn a_text_named_refusal_is_attributed_only_to_our_put_and_final_only_for_a_pinned_reason() {
    // (a) A foreign key, a pinned reason.
    let (mut io, mut node, mut now) = opened_at(1_790_253_181_367, "foreign key");
    node.err_block_puts = 1;
    node.err_any_block = true;
    node.err_text_form = true;
    node.err_cause = "invalid put";
    node.err_text_key = Some("11111111111111111111111111111111");
    io.client(&protocol::encode_session_request(4, 9, &write(1, "a", "1")).expect("encodes"));
    let r = live(&mut io, &mut node, &mut now, 70_000);
    println!("  (a) foreign key: states {:?}; node errors {:?}", states(&r, 1), io.node_errors());
    assert_eq!(io.node_errors().get("put_error_unattributed"), Some(&1), "a text-named refusal of a PUT we do not have was not counted as unattributed");
    assert!(states(&r, 1).contains(&WriteState::Published), "a refusal naming another contract ended our write: {:?}", states(&r, 1));
    // (b) Our key, a reason that is not a pinned validation refusal.
    let (mut io, mut node, mut now) = opened_at(1_790_253_181_367, "our key, i/o reason");
    let before = node.block_puts.len();
    node.err_block_puts = usize::MAX;
    node.err_any_block = true;
    node.err_text_form = true;
    io.client(&protocol::encode_session_request(4, 9, &write(1, "a", "1")).expect("encodes"));
    let r = live(&mut io, &mut node, &mut now, 70_000);
    let errored = node.err_block.expect("THE SETUP: no block PUT was answered with the error");
    let sends = node.block_puts[before..].iter().filter(|(_, id)| *id == errored).count();
    println!("  (b) our key, i/o reason: {sends} send(s); states {:?}", states(&r, 1));
    assert!(sends >= 3, "a transient text-named refusal was not re-sent on the RTO ({sends} send(s) in 70 s)");
    assert!(!states(&r, 1).iter().any(|s| matches!(s, WriteState::Failed | WriteState::FailedWhy { .. })), "a refusal with a reason that is not pinned ended the write Failed");
}

/// A NODE ERROR NAMING NO OP (sdk#433): COUNTED by its reason code, and it ends nothing and re-arms nothing -- with
/// the write's PUTs on the wire, the page sends nothing because of it (the RTO stays the one clock).
#[test]
fn a_keyless_node_error_is_counted_and_re_arms_nothing() {
    use freenet_stdlib::client_api::ErrorKind;
    let (mut io, _node, _now) = opened_at(1_790_253_181_367, "keyless");
    io.client(&protocol::encode_session_request(4, 9, &write(1, "a", "1")).expect("encodes"));
    let sent = io.take_frames();
    assert!(!sent.is_empty(), "THE SETUP: the write put nothing on the wire");
    let keyless = bincode::serialize(&Err::<HostResponse, Err>(ErrorKind::OperationError { cause: "the node could not".into() }.into())).expect("encodes");
    io.inbound(&keyless, Ms(1_790_253_181_400));
    assert_eq!(io.node_errors().get("operation_error"), Some(&1), "the keyless error was not counted by its code: {:?}", io.node_errors());
    let after = io.take_frames();
    assert!(after.is_empty(), "a keyless node error sent {} frame(s): it re-armed ops on the wire", after.len());
}

/// A RELOAD AND A SECOND TAB ARE THE SAME PERSON (the switch-over blocker): a
/// page asks the signer which Register it signs for and opens THAT one. Only
/// the first page ever — on a signer holding no key — mints.
#[test]
fn a_reload_and_a_second_tab_reopen_the_same_register_and_only_the_first_page_mints() {
    let key = [21u8; 32];
    let mut node = WireNode::unprovisioned(&key);
    let mut now = 1_000;
    let (mut first, minted) = opening(&mut node, &mut now, &key, false);
    assert_eq!(minted, 1, "the first page, on a signer with no key, must mint");
    assert!(first.provisioned(), "the minted key was not provisioned: {:?}", first.unusable());
    assert_eq!(first.register_id(), node.register_id);
    client(&mut first, &mut node, &mut now, &Request::Identity);
    for (n, k) in ["a", "b", "c"].iter().enumerate() {
        assert!(states(&client(&mut first, &mut node, &mut now, &write(n as u64 + 1, k, "v")), n as u64 + 1).contains(&WriteState::Published));
    }
    for (i, label) in ["the reload", "a second tab"].into_iter().enumerate() {
        let (mut again, minted) = opening(&mut node, &mut now, &[99u8; 32], false);
        assert_eq!(minted, 0, "{label} minted a new identity");
        assert!(again.provisioned(), "{label} did not open the signer's register: {:?}", again.unusable());
        assert_eq!(again.register_id(), node.register_id, "{label} opened another register");
        client(&mut again, &mut node, &mut now, &Request::Identity);
        // The person's rows: the first page's three, and each earlier reopen's one.
        assert_eq!(row_count(&mut again, &mut node, &mut now, 31 + i as u64), Some(3 + i), "{label} did not see the person's rows");
        let k = format!("from-{i}");
        assert!(states(&client(&mut again, &mut node, &mut now, &write(10 + i as u64, &k, "v")), 10 + i as u64).contains(&WriteState::Published), "{label} cannot write its own tree");
    }
    assert_eq!(node.register_puts, 1, "a second register was created");
}

/// THE CONTROL (the mutant a page that always mints is): a reload that mints
/// its own key is refused by the signer and never sees the person's rows.
#[test]
fn control_a_page_that_always_mints_loses_the_persons_tree() {
    let key = [22u8; 32];
    let mut node = WireNode::unprovisioned(&key);
    let mut now = 1_000;
    let (mut first, _) = opening(&mut node, &mut now, &key, false);
    client(&mut first, &mut node, &mut now, &Request::Identity);
    assert!(states(&client(&mut first, &mut node, &mut now, &write(1, "a", "v")), 1).contains(&WriteState::Published));
    let (again, minted) = opening(&mut node, &mut now, &[98u8; 32], true);
    assert_eq!(minted, 1);
    assert!(!again.provisioned(), "a second key was accepted: the check above could not have told the difference");
    assert!(again.register_id() != node.register_id, "the minting page is not on another register");
}

/// TWO PAGES TOLD "NO KEY" (sdk#343): both asked the signer before either
/// provisioned -- two tabs opened together, the builder and a published app,
/// a Provision re-sent after a lost answer -- so both mint, and the second
/// Provision is refused `KeyAlreadyProvisioned`. That is an ANSWER: the second
/// page asks again which Register the signer holds and opens it. Both end on
/// ONE register, neither refused, and both write the one tree.
#[test]
fn two_pages_told_no_key_both_open_the_one_register_the_signer_took() {
    let first_key = [23u8; 32];
    let mut node = WireNode::unprovisioned(&first_key);
    let mut now = 1_000;
    let page = || {
        let (container, signer) = wire::delegate_from_code(SIGNER_CODE);
        let mut io = PageIo::new(
            Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
            Artefacts { block_code: BLOCK_CODE.to_vec(), register_code: REGISTER_CODE.to_vec(), register_params: Vec::new(), signer },
        );
        io.begin(container);
        io
    };
    let (mut a, mut b) = (page(), page());
    settle(&mut a, &mut node, &mut now);
    settle(&mut b, &mut node, &mut now);
    assert!(a.needs_key() && b.needs_key(), "both pages must be told 'no key' before either provisions, or this is not the race");
    for (io, k) in [(&mut a, first_key), (&mut b, [24u8; 32])] {
        let sk = ed25519_dalek::SigningKey::from_bytes(&k);
        io.provision_with(sk.to_bytes().to_vec(), wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME));
    }
    settle(&mut a, &mut node, &mut now);
    settle(&mut b, &mut node, &mut now);
    for (label, io) in [("the first page", &a), ("the second page", &b)] {
        assert!(io.refused().is_none(), "{label} ended refused: {:?}", io.refused());
        assert!(io.provisioned(), "{label} is not provisioned: {:?}", io.unusable());
        assert_eq!(io.register_id(), node.register_id, "{label} is not on the register the signer took");
    }
    for (n, io) in [(1u64, &mut a), (2, &mut b)] {
        client(io, &mut node, &mut now, &Request::Identity);
        assert!(states(&client(io, &mut node, &mut now, &write(n, &format!("k{n}"), "v")), n).contains(&WriteState::Published), "page {n} cannot write the one tree");
    }
    assert_eq!(node.register_puts, 1, "a second register was created");
}

/// What each frame asks the node, by kind (a delegate registration, a signer
/// request, or anything else).
fn kinds(frames: &[Vec<u8>]) -> Vec<&'static str> {
    frames
        .iter()
        .map(|f| match bincode::deserialize::<ClientRequest>(f) {
            Ok(ClientRequest::DelegateOp(DelegateRequest::RegisterDelegate { .. })) => "register",
            Ok(ClientRequest::DelegateOp(DelegateRequest::ApplicationMessages { .. })) => "signer",
            _ => "other",
        })
        .collect()
}

/// #260, MEASURED: the signer's registration and its first request sent in
/// ONE flush was answered EMPTY by the node 7 times in 12, and the page waited
/// for ever. So the registration goes ALONE, and the first request (the
/// Register query) only once the registration is answered.
#[test]
fn the_signers_first_request_waits_for_its_registration_to_be_answered() {
    let key = [23u8; 32];
    let mut node = WireNode::unprovisioned(&key);
    let (container, signer) = wire::delegate_from_code(SIGNER_CODE);
    let mut io = PageIo::new(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
        Artefacts { block_code: BLOCK_CODE.to_vec(), register_code: REGISTER_CODE.to_vec(), register_params: Vec::new(), signer },
    );
    io.begin(container);
    let first = io.take_frames();
    assert_eq!(kinds(&first), ["register"], "the first request went out with the registration, before it was answered");
    for f in &first {
        let answer = node.serve(f).expect("the registration is answered");
        io.inbound(&answer, Ms(1_000));
    }
    assert_eq!(kinds(&io.take_frames()), ["signer"], "the registration's answer did not release the Register query");
}

/// ON A 0.2.137+ NODE (sdk#439, freenet-core #5729): a visitor's node without the signer answers the asking page's
/// query with the TYPED Missing, not EMPTY -- and that is the same answer: "no signer here", one query, then the
/// visitor claims its own tree (the signer registered, a key minted), and its first write publishes. On 0.2.138
/// before this, the Missing was flattened into a keyless refusal and the visitor's page hung "Reading…" (realnet
/// step 12). THE CONTROL: the same node answering EMPTY (0.2.136) opens the same way.
#[test]
fn on_a_0_2_138_node_a_missing_signer_is_the_asking_pages_answer_and_the_visitor_opens_its_own_tree() {
    for as_0_2_138 in [true, false] {
        let label = if as_0_2_138 { "0.2.138 (typed Missing)" } else { "0.2.136 (EMPTY, THE CONTROL)" };
        let key = [32u8; 32];
        let mut node = WireNode::unprovisioned(&key);
        node.empty_until_registered = true;
        node.answers_as_0_2_138 = as_0_2_138;
        let mut now = 1_000;
        let mut io = asker();
        settle(&mut io, &mut node, &mut now);
        assert!(matches!(io.asked(), Some(page_io::Asked::NoSigner(_))), "{label}: the node without the signer was not named as one: {:?}", io.asked());
        assert_eq!(node.served.get("signer").copied(), Some(1), "{label}: the answer was not taken as one: {:?}", node.served);
        if as_0_2_138 {
            assert_eq!(node.served.get("signer missing").copied(), Some(1), "{label}: THE SETUP: the node never answered Missing");
        }
        assert!(io.claim(wire::delegate_from_code(SIGNER_CODE).0), "{label}: the visitor's claim was refused");
        settle(&mut io, &mut node, &mut now);
        if io.needs_key() {
            let sk = ed25519_dalek::SigningKey::from_bytes(&key);
            io.provision_with(sk.to_bytes().to_vec(), wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME));
            settle(&mut io, &mut node, &mut now);
        }
        assert!(io.provisioned(), "{label}: the visitor's own tree never opened: {:?}", io.unusable());
        client(&mut io, &mut node, &mut now, &Request::Identity);
        let rs = client(&mut io, &mut node, &mut now, &write(1, "mine", "1"));
        assert!(states(&rs, 1).contains(&WriteState::Published), "{label}: the visitor's first write did not publish: {:?}", states(&rs, 1));
    }
}

/// ON A 0.2.137+ NODE, a page that REGISTERED the signer itself (`begin`) reads a Missing -- its request arrived
/// before the registration took (#260's case, answered EMPTY on 0.2.136) -- and the node's delegate BACKOFF after
/// it as NO ANSWER: it asks again on the RTO and opens. Neither is a refusal.
#[test]
fn on_a_0_2_138_node_a_missing_or_throttled_reply_after_registering_is_no_answer_and_the_page_opens() {
    let key = [33u8; 32];
    let mut node = WireNode::unprovisioned(&key);
    node.answers_as_0_2_138 = true;
    node.empty_signer_answers = 2;
    node.throttle_signer_answers = 1;
    let mut now = 1_000;
    let (io, minted) = opening(&mut node, &mut now, &key, false);
    assert!(io.provisioned(), "a Missing or a backoff after registering ended the open ({:?}): {:?}", node.served, io.unusable());
    assert!(io.unusable().iter().all(|u| !u.contains("refused")), "a Missing or a backoff was read as a refusal: {:?}", io.unusable());
    assert_eq!(minted, 1, "the page never learnt the signer holds no key");
    assert_eq!((node.served.get("signer missing").copied(), node.served.get("signer throttled").copied()), (Some(2), Some(1)), "THE SETUP: {:?}", node.served);
}

/// THE AMBIGUITY (main's ruling on #260): `wire::unframe` maps ANY empty
/// DelegateResponse to `Ack(Registered)`. Once the registration is answered,
/// an empty response to the outstanding query is "no answer" — asked again —
/// never a second registration. Here the node answers the query empty once,
/// then for real: the page asks again and opens.
#[test]
fn an_empty_reply_to_the_query_is_no_answer_and_the_re_ask_opens() {
    let key = [24u8; 32];
    let mut node = WireNode::unprovisioned(&key);
    node.empty_signer_answers = 1;
    let mut now = 1_000;
    let (io, minted) = opening(&mut node, &mut now, &key, false);
    assert_eq!(node.empty_signer_answers, 0, "the node never answered empty: the case did not happen");
    assert_eq!(minted, 1, "the page never learnt the signer holds no key");
    assert!(io.provisioned(), "an empty reply ended the open: {:?}", io.unusable());
    assert!(node.served["signer"] >= 3, "the query was not asked again after the empty reply ({} signer requests)", node.served["signer"]);
}

/// And an EMPTY reply, after this page registered the signer, is NOT an
/// answer (#260): the page's sender keeps asking — no count ends it (rule 8,
/// the old cap of three is gone) — the page says how long, and when the node
/// answers properly the opening goes on.
#[test]
fn a_signer_that_answers_only_empty_is_asked_on_and_opens_when_it_answers() {
    let key = [25u8; 32];
    let mut node = WireNode::unprovisioned(&key);
    node.empty_signer_answers = 50;
    let mut now = 1_000;
    let (io, _) = opening(&mut node, &mut now, &key, false);
    assert_eq!(node.empty_signer_answers, 0, "the node did not answer empty 50 times: the case did not happen");
    assert!(io.provisioned(), "fifty empty replies ended the open: {:?}", io.unusable());
    assert!(io.unusable().iter().all(|u| !u.contains("EMPTY")), "an empty reply was reported as an end: {:?}", io.unusable());
}

/// EVERY NODE CALL ON THE RTO (the ruling since #227): a lost answer to the
/// page's FIRST signer request — "which Register?" — is asked again, and the
/// page opens. Measured before this: 4 of 9 fresh-node page-mode opens hung.
#[test]
fn a_lost_first_signer_answer_is_asked_again_and_the_page_opens() {
    let key = [23u8; 32];
    let mut node = WireNode::unprovisioned(&key);
    node.drop_signer_answers = 1;
    let mut now = 1_000;
    let (page, minted) = opening(&mut node, &mut now, &key, false);
    assert_eq!(node.served.get("signer answer lost"), Some(&1), "THE CONTROL: the first answer was not lost, so nothing was tested");
    assert!(page.provisioned(), "a lost first answer was never asked again: {:?} {:?}", node.served, page.unusable());
    assert_eq!(minted, 1, "the re-asked question was answered 'none' and the key minted once");
    assert!(page.unusable().is_empty(), "{:?}", page.unusable());
}

/// ...and a lost PROVISIONING answer too.
#[test]
fn a_lost_provisioning_answer_is_asked_again() {
    let key = [24u8; 32];
    let mut node = WireNode::unprovisioned(&key);
    let mut now = 1_000;
    let (container, signer) = wire::delegate_from_code(SIGNER_CODE);
    let mut io = PageIo::new(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
        Artefacts { block_code: BLOCK_CODE.to_vec(), register_code: REGISTER_CODE.to_vec(), register_params: Vec::new(), signer },
    );
    io.begin(container);
    settle(&mut io, &mut node, &mut now);
    assert!(io.needs_key());
    node.drop_signer_answers = 1;
    let sk = ed25519_dalek::SigningKey::from_bytes(&key);
    io.provision_with(sk.to_bytes().to_vec(), wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME));
    settle(&mut io, &mut node, &mut now);
    assert!(io.provisioned(), "a lost provisioning answer was never asked again: {:?}", node.served);
}

/// NO CUT-OFF (rules 7, 8): a signer silent for FIVE MINUTES is asked again
/// on the page's RTO the whole time — never given up, never named ended — the
/// page says how long it has not answered, and when it answers at last the
/// opening goes on. A fixed cut-off anywhere on this path fails this test.
#[test]
fn a_signer_silent_for_five_minutes_then_answering_is_never_given_up() {
    let key = [25u8; 32];
    let mut node = WireNode::unprovisioned(&key);
    node.drop_signer_answers = usize::MAX;
    let (container, signer) = wire::delegate_from_code(SIGNER_CODE);
    let mut io = PageIo::new(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
        Artefacts { block_code: BLOCK_CODE.to_vec(), register_code: REGISTER_CODE.to_vec(), register_params: Vec::new(), signer },
    );
    io.begin(container);
    let mut now = 1_000u64;
    let silent_until = now + 300_000;
    let mut longest = 0;
    while now < silent_until {
        let frames = io.take_frames();
        if frames.is_empty() {
            let Some(Ms(t)) = io.next_due() else { break };
            now = now.max(t).min(silent_until);
            io.tick(Ms(now));
            if let Some((_, ms)) = io.not_answering() {
                longest = longest.max(ms);
            }
            continue;
        }
        now += 1;
        for f in frames {
            if let Some(a) = node.serve(&f) {
                io.inbound(&a, Ms(now));
            }
        }
    }
    let asked = node.served.get("signer").copied().unwrap_or(0);
    println!("silent 5 min: asked {asked} times, longest not answering {longest} ms, refused {:?}", io.refused());
    assert!(!io.provisioned() && io.refused().is_none(), "a silent signer was ended: {:?}", io.refused());
    assert!(asked >= 5, "the silent signer was asked only {asked} times in five minutes");
    assert!(longest >= 290_000, "the page never said it had waited: {longest} ms");
    assert!(io.unusable().iter().all(|u| !u.contains("not answering")), "a silence was reported as an END: {:?}", io.unusable());
    // It answers at last: opening goes on.
    node.drop_signer_answers = 0;
    settle(&mut io, &mut node, &mut now);
    assert!(io.needs_key(), "the signer answered after five minutes and the page did not take it: {:?}", node.served);
}

/// OPENING ENDS BY NAME (what `open()` reports): REFUSED — the signer's own
/// words — when a page provisions another key over the one it holds.
#[test]
fn opening_that_the_signer_refuses_is_refused_in_its_words() {
    let mut node = WireNode::new(&[26u8; 32]); // the signer already holds this person's key
    let mut now = 1_000;
    let (container, signer) = wire::delegate_from_code(SIGNER_CODE);
    let other = ed25519_dalek::SigningKey::from_bytes(&[27u8; 32]);
    let mut io = PageIo::new(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
        Artefacts { block_code: BLOCK_CODE.to_vec(), register_code: REGISTER_CODE.to_vec(), register_params: wire::register_params(&other.verifying_key().to_bytes(), wire::HEAD_NAME), signer },
    );
    io.provision(container, other.to_bytes().to_vec());
    settle(&mut io, &mut node, &mut now);
    assert!(!io.provisioned());
    assert!(io.refused().is_some_and(|r| r.contains("KeyAlreadyProvisioned")), "{:?}", io.refused());
    assert!(!io.stalled(), "a refusal is not also 'still waiting'");
}

/// STALLED while the opening is still waiting past its first RTO — and not
/// once it is answered. (There is no "exhausted": nothing gives up.)
#[test]
fn opening_is_stalled_while_unanswered_and_not_once_answered() {
    let key = [28u8; 32];
    let mut node = WireNode::unprovisioned(&key);
    node.drop_signer_answers = 1;
    let (container, signer) = wire::delegate_from_code(SIGNER_CODE);
    let mut io = PageIo::new(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
        Artefacts { block_code: BLOCK_CODE.to_vec(), register_code: REGISTER_CODE.to_vec(), register_params: Vec::new(), signer },
    );
    io.tick(Ms(1_000));
    io.begin(container);
    for f in io.take_frames() {
        if let Some(a) = node.serve(&f) { io.inbound(&a, Ms(1_000)); }
    }
    let _ = io.take_frames(); // the first request, whose answer is lost
    io.tick(Ms(1_000));
    assert!(!io.stalled(), "stalled before its first RTO");
    io.tick(Ms(2_500)); // past it, re-sent by the page's sender
    assert!(io.stalled(), "not stalled past its first RTO, unanswered");
    let mut now = 2_500;
    settle(&mut io, &mut node, &mut now);
    assert!(io.needs_key() && !io.stalled(), "answered, and still stalled");
}

/// sdk#259: page-io reports the head subscription AS IT IS — asked, answered,
/// and how many head moves it has delivered. On the page path this is what
/// `LiveMode` is built from: the Session's own `watching` belongs to the
/// delegate path the switch-over deleted, so a page that held the
/// subscription the whole time reported "Polled" and a probe read its number
/// as "the tick's wearing a different name".
#[test]
fn page_io_says_whether_the_head_subscription_was_asked_answered_and_delivering() {
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 0u64;

    // Before anything is sent: nothing asked, nothing answered. NOT "live".
    let h = io.head_subscription();
    assert!(!h.asked && !h.answered && h.changes == 0, "page-io claimed a subscription before it read the head: {h:?}");

    let _ = client(&mut io, &mut node, &mut now, &Request::Identity);
    let h = io.head_subscription();
    println!("  after Identity (no head yet): {h:?}");
    assert!(h.asked, "the head was never read with subscribe, so nothing can notify this page");
    assert!(!h.answered, "a head that does not exist yet was called answered");
    assert!(h.failed > 0, "the node's failure to serve a head that does not exist was not counted, so `Polled` could not say why");

    // The first write CREATES the head register, and the page reads it again.
    let _ = client(&mut io, &mut node, &mut now, &write(1, "k/1", "v"));
    let h = io.head_subscription();
    println!("  after the first write (the head exists): {h:?}");
    assert!(h.asked && h.answered, "the page holds no answered head read once the head exists: nothing can notify it");
    assert_eq!(h.ended, None, "an opening that succeeded was reported as ended");
}

/// sdk#259: an opening that ENDED — refused in the signer's words, here —
/// means no head read is coming, so no subscription either, and the report
/// says so in those words rather than "not answered yet" for ever.
#[test]
fn an_opening_that_ended_is_reported_as_the_reason_there_is_no_subscription() {
    let mut node = WireNode::new(&[26u8; 32]);
    let mut now = 1_000;
    let (container, signer) = wire::delegate_from_code(SIGNER_CODE);
    let other = ed25519_dalek::SigningKey::from_bytes(&[27u8; 32]);
    let mut io = PageIo::new(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
        Artefacts { block_code: BLOCK_CODE.to_vec(), register_code: REGISTER_CODE.to_vec(), register_params: wire::register_params(&other.verifying_key().to_bytes(), wire::HEAD_NAME), signer },
    );
    io.provision(container, other.to_bytes().to_vec());
    settle(&mut io, &mut node, &mut now);
    let h = io.head_subscription();
    assert!(!h.answered, "a page whose opening was refused reported a subscription: {h:?}");
    assert!(
        h.ended.as_deref().is_some_and(|s| s.contains("KeyAlreadyProvisioned")),
        "the reason there is no subscription is not in the report, so it would say 'not answered yet' for ever: {h:?}"
    );
}

/// sdk#259: THE MAPPING, every branch. What a page tells its app — `mode`,
/// `why`, `headChanges` — from page-io's facts. It used to live only in the
/// web Session, which no native test can build: a mutant there that made
/// "subscribed" unreachable (`h if false && h.answered`) survived every suite,
/// so the one thing this report exists to say was never pinned.
#[test]
fn the_live_mode_mapping_says_subscribed_only_when_answered_and_otherwise_why() {
    use page_io::{HeadSubscription, LiveMode};
    let h = |asked, answered, changes, failed, ended: Option<&str>| HeadSubscription { asked, answered, changes, failed, ended: ended.map(str::to_string) };

    // ANSWERED: subscribed, with what it delivered — and it WINS over a
    // failure counted before the head existed (measured on the fixture: 1000
    // failed reads, then answered once the head was created).
    let m = h(true, true, 3, 1000, None).live_mode();
    assert_eq!((m.mode, m.why.as_str(), m.head_changes), ("HeadSubscribed", "", 3), "an answered head read is not reported as a subscription: {m:?}");

    // ENDED: Polled, in the words it ended with — even if a read was asked.
    let m = h(true, false, 0, 2, Some("the signer refused: KeyAlreadyProvisioned")).live_mode();
    assert_eq!(m.mode, "Polled");
    assert!(m.why.contains("opening ended") && m.why.contains("KeyAlreadyProvisioned"), "an ended opening is not the reason given: {m:?}");

    // FAILED: Polled, how many times, and that page-io cannot tell which.
    let m = h(true, false, 0, 4, None).live_mode();
    assert_eq!(m.mode, "Polled");
    assert!(m.why.contains("failure 4 time(s)") && m.why.contains("F55"), "failed head reads are not the reason given: {m:?}");

    // ASKED, unanswered.
    let m = h(true, false, 0, 0, None).live_mode();
    assert_eq!(m.mode, "Polled");
    assert!(m.why.contains("not been answered yet"), "{m:?}");

    // NOT ASKED.
    let m = h(false, false, 0, 0, None).live_mode();
    assert_eq!(m.mode, "Polled");
    assert!(m.why.contains("has not been read"), "{m:?}");

    // No page at all.
    assert_eq!(LiveMode::no_page().mode, "Polled");

    // And end to end, through real frames: a page whose head exists reports
    // HeadSubscribed — the report the two-tab's live arm asserts.
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 0u64;
    let _ = client(&mut io, &mut node, &mut now, &Request::Identity);
    assert_eq!(io.head_subscription().live_mode().mode, "Polled", "a page reported a subscription to a head that does not exist yet");
    let _ = client(&mut io, &mut node, &mut now, &write(1, "k/1", "v"));
    let m = io.head_subscription().live_mode();
    assert_eq!(m.mode, "HeadSubscribed", "a page whose head read was answered does not say so: {m:?}");
}

/// A page that only ASKS whose node this is (`PageIo::ask`): the same
/// Register query, with no registration before it.
fn asker() -> PageIo {
    let (_, signer) = wire::delegate_from_code(SIGNER_CODE);
    let mut io = PageIo::new(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
        Artefacts { block_code: BLOCK_CODE.to_vec(), register_code: REGISTER_CODE.to_vec(), register_params: Vec::new(), signer },
    );
    io.ask();
    io
}

/// WHOSE NODE (the owner's ruling): the page asks the node's EXISTING signer
/// which Register it signs for, and nothing is registered, minted or
/// provisioned — on the publisher's node, on a node whose signer holds no key,
/// and on a node with no signer (another user's) at all.
#[test]
fn asking_whose_node_registers_mints_and_provisions_nothing() {
    // The publisher's node: its signer names the publisher's Register.
    let mut node = WireNode::new(&[7u8; 32]);
    let secrets = node.secrets.clone();
    let mut io = asker();
    let mut now = 1_000;
    settle(&mut io, &mut node, &mut now);
    assert_eq!(io.asked(), Some(&page_io::Asked::Register(node.register_id)), "{:?}", io.unusable());
    assert!(!io.provisioned(), "asking provisioned the page");
    assert_eq!(node.secrets, secrets, "asking changed the signer's secrets");
    assert_eq!(node.served.get("register delegate"), None, "asking registered the signer: {:?}", node.served);

    // A node whose signer holds no key: "no key", and nothing minted.
    let mut node = WireNode::unprovisioned(&[8u8; 32]);
    let mut io = asker();
    settle(&mut io, &mut node, &mut now);
    assert_eq!(io.asked(), Some(&page_io::Asked::NoKey));
    assert!(!io.needs_key(), "asking asked the caller to MINT a key");
    assert!(node.secrets.is_empty(), "asking put a key on a node that had none");
    assert_eq!(node.served.get("register delegate"), None, "asking registered the signer: {:?}", node.served);

    // A node with no signer (another user's): refused, in the node's words.
    let mut node = WireNode::unprovisioned(&[9u8; 32]);
    node.delegate_absent = true;
    let mut io = asker();
    settle(&mut io, &mut node, &mut now);
    assert!(matches!(io.asked(), Some(page_io::Asked::Refused(_))), "{:?}", io.asked());
    // A reader leaves no trace: the signer was never REGISTERED here.
    assert_eq!(node.served.get("register delegate"), None, "asking registered the signer: {:?}", node.served);

    // As the REAL node answers a delegate it does not have (measured on
    // 0.2.136): EMPTY, every time. Refused as well, by name.
    let mut node = WireNode::unprovisioned(&[10u8; 32]);
    node.empty_signer_answers = usize::MAX;
    let mut io = asker();
    settle(&mut io, &mut node, &mut now);
    assert!(matches!(io.asked(), Some(page_io::Asked::NoSigner(_))), "an EMPTY-answering node is not named as having no signer: {:?}", io.asked());
    assert_eq!(node.served.get("register delegate"), None, "asking registered the signer: {:?}", node.served);
}

/// ASKING LEAVES NO TRACE, frame by frame: on a node without the signer (it
/// answers EMPTY, as a real 0.2.136 node does), the asking page sends the
/// Register QUERY and nothing else — never a registration — and that one
/// EMPTY IS the node's answer ("no signer here"): one query, one answer, done
/// (rule 8: an answer ends it; no count, no clock). A page that took the
/// EMPTY for its signer's registration (it never registered one) would ask
/// the query a second time.
#[test]
fn asking_on_a_node_without_the_signer_sends_one_query_and_takes_its_empty_answer() {
    let mut node = WireNode::unprovisioned(&[11u8; 32]);
    node.empty_signer_answers = usize::MAX;
    let mut io = asker();
    let mut now = 1_000u64;
    let mut sent = Vec::new();
    for _ in 0..2_000 {
        let frames = io.take_frames();
        if frames.is_empty() {
            match io.next_due() {
                Some(Ms(t)) => {
                    now = now.max(t);
                    io.tick(Ms(now));
                    continue;
                }
                None => break,
            }
        }
        sent.extend(kinds(&frames));
        now += 1;
        for f in frames {
            if let Some(answer) = node.serve(&f) {
                io.inbound(&answer, Ms(now));
            }
        }
    }
    assert!(!sent.is_empty(), "the asking page sent nothing: the check below could not fail");
    assert!(sent.iter().all(|k| *k == "signer"), "asking sent a frame that is not the query: {sent:?}");
    assert_eq!(node.served.get("register delegate"), None, "asking registered the signer: {:?}", node.served);
    assert!(!io.provisioned() && node.secrets.is_empty(), "asking provisioned the node");
    // Named as a node with NO SIGNER: its own answer, not a refusal.
    assert!(matches!(io.asked(), Some(page_io::Asked::NoSigner(_))), "a node without the signer was not named as one: {:?}", io.asked());
    let served = node.served.get("signer").copied().unwrap_or(0);
    assert_eq!(served, 1, "the node answered EMPTY and was asked again ({served} queries): an EMPTY was taken for something else");
    assert_eq!(sent, ["signer"], "frames sent: {sent:?}");
}

/// THE CONTROL: the page that OPENS (`begin`) does register the signer — so
/// the "no registration" check above is one that can fail.
#[test]
fn control_opening_registers_the_signer() {
    let mut node = WireNode::new(&[7u8; 32]);
    let (container, signer) = wire::delegate_from_code(SIGNER_CODE);
    let mut io = PageIo::new(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
        Artefacts { block_code: BLOCK_CODE.to_vec(), register_code: REGISTER_CODE.to_vec(), register_params: Vec::new(), signer },
    );
    io.begin(container);
    let mut now = 1_000;
    settle(&mut io, &mut node, &mut now);
    assert_eq!(node.served.get("register delegate"), Some(&1), "{:?}", node.served);
}

/// THE USER'S OWN TREE (DATA-SOURCE `mine`, sdk#241): an ASKED page is
/// claimed as the person's own, on each kind of node, through `begin`'s own
/// opening from where the answer left it:
/// * the node signs for their Register: it is opened as it is, nothing
///   registered or minted;
/// * its signer holds no key: one is minted (`needs_key`), nothing registered;
/// * there is no signer here: it is registered and asked, then a key minted.
///
/// On every one the first write CREATES the head, and a reopened page reads
/// the row back (a reload keeps the user's rows).
#[test]
fn a_claimed_page_opens_the_persons_own_tree_on_each_kind_of_node() {
    for case in ["signs for their register", "signer holds no key", "no signer here"] {
        let key = [31u8; 32];
        let mut node = if case == "signs for their register" { WireNode::new(&key) } else { WireNode::unprovisioned(&key) };
        node.empty_until_registered = case == "no signer here";
        let mut now = 1_000;
        let mut io = asker();
        settle(&mut io, &mut node, &mut now);
        match (case, io.asked()) {
            ("signs for their register", Some(page_io::Asked::Register(r))) => assert_eq!(*r, node.register_id, "{case}"),
            ("signer holds no key", Some(page_io::Asked::NoKey)) | ("no signer here", Some(page_io::Asked::NoSigner(_))) => {}
            (_, other) => panic!("{case}: asked {other:?}"),
        }
        let want = io.asked().cloned();
        let (container, _) = wire::delegate_from_code(SIGNER_CODE);
        assert!(io.claim(container), "{case}: the claim was refused");
        settle(&mut io, &mut node, &mut now);
        if io.needs_key() {
            let sk = ed25519_dalek::SigningKey::from_bytes(&key);
            io.provision_with(sk.to_bytes().to_vec(), wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME));
            settle(&mut io, &mut node, &mut now);
        }
        assert!(io.provisioned(), "{case}: the claimed page never opened: {:?}", io.unusable());
        assert_eq!(io.register_id(), node.register_id, "{case}: the claimed page names another register");
        let registered = node.served.get("register delegate").copied().unwrap_or(0);
        assert_eq!(registered, usize::from(case == "no signer here"), "{case}: the signer was registered {registered} times");
        assert_eq!(io.asked().cloned(), want, "{case}: the claim rewrote the answer (the identity's one owner)");
        client(&mut io, &mut node, &mut now, &Request::Identity);
        let rs = client(&mut io, &mut node, &mut now, &write(1, "mine", "1"));
        assert!(states(&rs, 1).iter().any(|s| matches!(s, WriteState::Published)), "{case}: the user's first write did not publish: {:?}", states(&rs, 1));
        assert!(node.contracts.contains_key(&node.register_id), "{case}: the first write did not create the user's head");
        let mut again = page_io(&node);
        client(&mut again, &mut node, &mut now, &Request::Identity);
        assert_eq!(row_count(&mut again, &mut node, &mut now, 70), Some(1), "{case}: a reopened page does not read the user's row");
    }
}

/// NOTHING IS CLAIMED ON AN ANSWER NOT HAD: before the signer answers, and
/// while it stays silent (which ends nothing: rule 8), a claim says `false`
/// and nothing is registered,
/// minted or opened -- the runtime shows the inputs disabled, with why.
#[test]
fn a_claim_before_an_answer_or_on_a_silent_signer_claims_nothing() {
    let key = [32u8; 32];
    let mut node = WireNode::new(&key);
    let (container, _) = wire::delegate_from_code(SIGNER_CODE);
    let mut io = asker();
    assert_eq!(io.asked(), None, "answered before the node was asked");
    assert!(!io.claim(container.clone()), "a page claimed its tree before the signer said whose node it is");
    let mut now = 1_000;
    node.drop_signer_answers = usize::MAX;
    run_unsettled(&mut io, &mut node, &mut now);
    // RULE 8: silence is not an answer and ends nothing. The ask stays
    // unanswered, the page says what it waits on, and nothing is claimed.
    assert_eq!(io.asked(), None, "a silent signer was taken as an answer: {:?}", io.asked());
    let (what, ms) = io.not_answering().expect("a silent signer, and the page does not say what it waits on");
    assert!(what.contains("signer"), "the page names the wrong wait: {what}");
    assert!(ms > 0, "the wait has no length");
    assert!(!io.claim(container), "a silent signer's page was claimed");
    assert!(!io.provisioned() && !io.needs_key(), "a refused claim opened or minted");
    assert_eq!(node.served.get("register delegate"), None, "a refused claim registered the signer");
}

/// THE ONE DECISION (`PageIo::may_write`; DATA-SOURCE, the architect's point
/// 5): every cell of the table, answered from what the page holds -- a view,
/// the signer's answer to an ask, or an opened page -- for the page's OWN
/// tree (`None`, a `mine` component's) and for a named head.
#[test]
fn may_write_is_one_decision_read_from_the_signers_answer() {
    use page_io::MayWrite::{No, Unknown, Yes};
    let yes = |m: page_io::MayWrite| m == Yes;
    let no = |m: page_io::MayWrite| matches!(m, No(_));
    let unknown = |m: page_io::MayWrite| matches!(m, Unknown(_));
    let undecided = |m: page_io::MayWrite| matches!(m, page_io::MayWrite::Undecided(_));
    let stranger = [0xAB; 32];
    let mut now = 1_000;

    // A VIEW (`reader`): never, whose ever head.
    let node = WireNode::new(&[40u8; 32]);
    let view = reader(&node);
    assert!(no(view.may_write(None)) && no(view.may_write(Some(node.register_id))), "a view may write");

    // ASKED, not answered yet: NOT DECIDED (a write waits), for any head.
    let io = asker();
    assert!(undecided(io.may_write(None)) && undecided(io.may_write(Some(stranger))), "an unanswered ask was decided");

    // It signs for their Register: that head, and the own tree, yes; another, no.
    let mut node = WireNode::new(&[41u8; 32]);
    let mut io = asker();
    settle(&mut io, &mut node, &mut now);
    assert!(yes(io.may_write(None)) && yes(io.may_write(Some(node.register_id))), "{:?}", io.may_write(Some(node.register_id)));
    assert!(no(io.may_write(Some(stranger))), "a node that signs for one head may write another");

    // No key, and no signer: the own tree yes (made on first use); another head no.
    for empty in [false, true] {
        let mut node = WireNode::unprovisioned(&[42u8; 32]);
        node.empty_until_registered = empty;
        let mut io = asker();
        settle(&mut io, &mut node, &mut now);
        assert!(yes(io.may_write(None)), "empty={empty}: {:?}", io.asked());
        assert!(no(io.may_write(Some(stranger))), "empty={empty}: a node with no key may write a head");
    }

    // Not answering: NOT DECIDED (rule 8, silence is not an answer), for any head.
    let mut node = WireNode::new(&[43u8; 32]);
    node.drop_signer_answers = usize::MAX;
    let mut io = asker();
    run_unsettled(&mut io, &mut node, &mut now);
    assert!(undecided(io.may_write(None)) && undecided(io.may_write(Some(node.register_id))), "{:?}", io.asked());

    // Refused by the node: the own tree not known; a named head no.
    let mut node = WireNode::unprovisioned(&[44u8; 32]);
    node.delegate_absent = true;
    let mut io = asker();
    settle(&mut io, &mut node, &mut now);
    assert!(unknown(io.may_write(None)) && no(io.may_write(Some(stranger))), "{:?}", io.asked());

    // OPENED (`begin`'s, or a claimed ask): its register yes, another no.
    let mut node = WireNode::new(&[45u8; 32]);
    let (io, _) = opening(&mut node, &mut now, &[45u8; 32], false);
    assert!(io.provisioned(), "the opening did not open: {:?}", io.unusable());
    assert!(yes(io.may_write(None)) && yes(io.may_write(Some(node.register_id))) && no(io.may_write(Some(stranger))));

    // Claimed where there was no key: the MINTED register is then its own.
    let key = [46u8; 32];
    let mut node = WireNode::unprovisioned(&key);
    let mut io = asker();
    settle(&mut io, &mut node, &mut now);
    assert!(io.claim(wire::delegate_from_code(SIGNER_CODE).0));
    settle(&mut io, &mut node, &mut now);
    let sk = ed25519_dalek::SigningKey::from_bytes(&key);
    io.provision_with(sk.to_bytes().to_vec(), wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME));
    settle(&mut io, &mut node, &mut now);
    assert!(yes(io.may_write(Some(node.register_id))), "a claimed page may not write the register it minted: {:?}", io.may_write(Some(node.register_id)));
    assert!(no(io.may_write(Some(stranger))));
}

/// A VIEW PUTS BACK A BLOCK IT REBUILT, AND NOTHING WAITS ON IT (read-only
/// has one owner, the page). The publisher's first leaf is gone from the
/// node; a VIEW reading the tree rebuilds it from its group (race get) and
/// the page puts it back -- a repair restores existing content-addressed
/// bytes, no key, no head moved -- so the node serves exactly ONE block PUT
/// from the view, it is answered, and the view waits on nothing after. The
/// mutant (page-io drops a read-only page's PUT, the page keeps its
/// deadline) leaves the block un-put and the view waiting: red.
#[test]
fn a_view_puts_back_a_block_it_rebuilt_and_waits_on_nothing() {
    let mut node = WireNode::new(&[35u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    let big = "v".repeat(900);
    let ops: Vec<protocol::Op> = (0..60).map(|i| protocol::Op::Put(format!("row/{i:03}").into_bytes(), big.clone().into_bytes())).collect();
    let rs = client(&mut a, &mut node, &mut now, &Request::forced_write(1, ops));
    assert!(states(&rs, 1).contains(&WriteState::Published), "the publisher's tree did not publish");
    let (_, root) = node.head().expect("a head");
    let root_bytes = freenet_prolly::store::Blocks::get(a.server.page.blocks(), &root).expect("the writer holds its root").to_vec();
    let root_node = freenet_prolly::node::Node::parse(&root_bytes).expect("a node");
    assert!(!root_node.is_leaf() && root_node.parity_count() > 0, "THE CONTROL: the tree has no grouped children to rebuild from");
    let (lost, _) = root_node.child(0);
    let gone = wire::block::contract_for(BLOCK_CODE, &lost);
    assert!(node.contracts.remove(&gone).is_some(), "the lost leaf was not on the node");

    let mut v = reader(&node);
    client(&mut v, &mut node, &mut now, &Request::Identity);
    let served_before = node.served.clone();
    let before = node.served.get("put block").copied().unwrap_or(0);
    assert_eq!(rows(&mut v, &mut node, &mut now, 21), vec![60], "the view did not read every row through the rebuild");
    for _ in 0..20 {
        now += 1_000;
        v.tick(Ms(now));
        settle(&mut v, &mut node, &mut now);
    }
    let put = node.served.get("put block").copied().unwrap_or(0) - before;
    assert_eq!(put, 1, "the view put back {put} block(s); a rebuilt block is put back exactly once ({:?})", v.unusable());
    assert!(node.contracts.contains_key(&gone), "the rebuilt leaf is not on the node again");
    // `settle` never goes idle on a page that has its head -- the head
    // backstop is always due next -- so it stops at its step cap, and
    // whether a backstop ReadHead is in flight there is the cap's parity,
    // not the view's. Answer what was last sent, with no clock moved, and
    // THEN nothing may be waited on.
    for f in v.take_frames() {
        if let Some(a) = node.serve(&f) {
            v.inbound(&a, Ms(now));
        }
    }
    assert!(!v.server.page.waiting(), "the view still waits on something after its repair PUT was answered");
    for k in ["put register", "update", "signer", "register delegate"] {
        let by_view = node.served.get(k).copied().unwrap_or(0) - served_before.get(k).copied().unwrap_or(0);
        assert_eq!(by_view, 0, "a view made the node serve a {k}");
    }
}

/// READ-ONLY HAS ONE OWNER, THE PAGE: the door's refusal (`may_write`, what
/// the web Session asks) and the page's filter (no commit op) answer from the
/// same flag. A view is refused at the door by name, and a write that
/// reaches its page anyway makes no commit op -- named, not sent, nothing
/// waited on. Mutants that flip ONE alone go red: the door answering "yes"
/// on a view, or the page making a view's commit ops.
#[test]
fn a_views_door_refusal_and_its_pages_filter_agree() {
    let mut node = WireNode::new(&[36u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    assert!(states(&client(&mut a, &mut node, &mut now, &write(1, "x", "v")), 1).contains(&WriteState::Published));
    let mut v = reader(&node);
    client(&mut v, &mut node, &mut now, &Request::Identity);
    assert!(v.read_only() && v.server.page.read_only(), "page-io's read-only is not the page's");
    for head in [None, Some(node.register_id)] {
        assert_eq!(v.may_write(head), page_io::MayWrite::No(page_io::READ_ONLY.into()), "the door let a view write {head:?}");
    }
    let before = node.served.clone();
    let wr = client(&mut v, &mut node, &mut now, &write(2, "intruder", "v"));
    for _ in 0..200 {
        now += 1_000;
        v.tick(Ms(now));
        settle(&mut v, &mut node, &mut now);
    }
    assert!(states(&wr, 2).contains(&WriteState::FailedWhy { why: protocol::FailWhy::ReadOnly }), "a view's write was not refused at the door, as READ_ONLY: {wr:?}");
    assert!(v.server.page.unusable().iter().any(|u| u.starts_with("read-only: write 2 refused at the door")), "the view's refusal was not named: {:?}", v.server.page.unusable());
    for k in ["put block", "put register", "update", "signer"] {
        assert_eq!(node.served.get(k), before.get(k), "a view's write reached the node as a {k}");
    }
}

/// A VIEW'S DEFINE IS REFUSED AT THE DOOR, AND NOTHING IS QUEUED (the
/// architect's precision 2 on #338). A schema define is an ordinary write;
/// on a VIEW it is told `Failed` at the page's door, named, and the engine's
/// queue is EMPTY after -- beside the user's own no-tree define, which is
/// queued and visible (#342, `with_no_tree_writes_wait_visible_…`).
#[test]
fn a_views_define_is_refused_at_the_door_and_nothing_is_queued() {
    let mut node = WireNode::new(&[38u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    assert!(states(&client(&mut a, &mut node, &mut now, &write(1, "x", "v")), 1).contains(&WriteState::Published));
    let mut v = reader(&node);
    client(&mut v, &mut node, &mut now, &Request::Identity);
    assert_eq!(v.server.page.queue_load().0, 0, "THE CONTROL: the view's queue was not empty before");
    let r = client(&mut v, &mut node, &mut now, &write(2, "schema/notes", "{}"));
    assert!(states(&r, 2).contains(&WriteState::FailedWhy { why: protocol::FailWhy::ReadOnly }), "a view's define was not refused at the door, as READ_ONLY: {r:?}");
    assert!(v.server.page.unusable().iter().any(|u| u.starts_with("read-only: write 2 refused at the door")), "not named: {:?}", v.server.page.unusable());
    assert_eq!(v.server.page.queue_load().0, 0, "a view's define was queued on the engine");
}

/// OPENING THE USER'S OWN TREE WRITES NOTHING (DATA-SOURCE `mine`: the tree
/// is made on the first WRITE). On every kind of node an asked page opens
/// and READS the user's tree -- their rows where the node's signer holds
/// their key (claimed: nothing to make), EMPTY where it holds none or there
/// is no signer (nothing claimed: `no_tree_yet`) -- and the node serves no
/// PUT and no UPDATE. Then the first write claims, provisions and lands.
#[test]
fn opening_and_reading_the_users_own_tree_sends_no_put_and_the_first_write_lands() {
    for case in ["signs for their register", "signer holds no key", "no signer here"] {
        let key = [33u8; 32];
        let mut node = if case == "signs for their register" { WireNode::new(&key) } else { WireNode::unprovisioned(&key) };
        node.empty_until_registered = case == "no signer here";
        let mut now = 1_000;
        let mut io = asker();
        settle(&mut io, &mut node, &mut now);
        let (container, _) = wire::delegate_from_code(SIGNER_CODE);
        // What `Session::open_own` does: claim only an identity already here.
        if case == "signs for their register" {
            assert!(io.claim(container.clone()), "{case}: the claim was refused");
        } else {
            assert!(io.no_tree_yet(), "{case}: a user with no key here was not taken as having no tree yet");
        }
        settle(&mut io, &mut node, &mut now);
        client(&mut io, &mut node, &mut now, &Request::Identity);
        let read = row_count(&mut io, &mut node, &mut now, 71);
        let writes: BTreeMap<&str, usize> =
            node.served.iter().filter(|(k, _)| ["put block", "put register", "update"].contains(k)).map(|(k, n)| (*k, *n)).collect();
        assert!(writes.is_empty(), "{case}: opening and reading the user's own tree wrote to the node: {writes:?} (all served {:?})", node.served);
        assert_eq!(read, Some(0), "{case}: the user's own tree did not read as empty");
        // THE FIRST WRITE: what `Session::writable` does -- claim, and mint
        // where there is no key (the existing provision path) -- then it lands.
        if case != "signs for their register" {
            assert!(io.claim(container), "{case}: the first write's claim was refused");
            settle(&mut io, &mut node, &mut now);
            if io.needs_key() {
                let sk = ed25519_dalek::SigningKey::from_bytes(&key);
                io.provision_with(sk.to_bytes().to_vec(), wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME));
                settle(&mut io, &mut node, &mut now);
            }
        }
        let rs = client(&mut io, &mut node, &mut now, &write(1, "mine", "1"));
        let mut st = states(&rs, 1);
        for _ in 0..200 {
            if st.iter().any(|s| matches!(s, WriteState::Published)) { break; }
            now += 500;
            io.tick(page::Ms(now));
            st.extend(states(&settle(&mut io, &mut node, &mut now), 1));
        }
        assert!(st.iter().any(|s| matches!(s, WriteState::Published)), "{case}: the first write did not publish: {st:?} ({:?})", io.unusable());
        assert!(node.contracts.contains_key(&io.register_id()), "{case}: the first write did not create the user's head");
        let mut again = page_io(&node);
        client(&mut again, &mut node, &mut now, &Request::Identity);
        assert_eq!(row_count(&mut again, &mut node, &mut now, 73), Some(1), "{case}: a reopened page does not read the user's row");
    }
}

/// A RETURNING identity (its tree already has rows, written by another page
/// of the same person): opened and read through an asked page, the node
/// serves no PUT and no UPDATE.
#[test]
fn a_returning_identity_reads_its_rows_with_no_put() {
    let key = [34u8; 32];
    let mut node = WireNode::new(&key);
    let mut now = 1_000;
    let mut writer = page_io(&node);
    client(&mut writer, &mut node, &mut now, &Request::Identity);
    for i in 0..40u64 {
        let rs = client(&mut writer, &mut node, &mut now, &write(i + 1, &format!("mine/{i:03}"), "v"));
        assert!(states(&rs, i + 1).iter().any(|s| matches!(s, WriteState::Published)), "row {i} did not publish");
    }
    let before = node.served.clone();
    let mut io = asker();
    settle(&mut io, &mut node, &mut now);
    let (container, _) = wire::delegate_from_code(SIGNER_CODE);
    assert!(io.claim(container));
    settle(&mut io, &mut node, &mut now);
    client(&mut io, &mut node, &mut now, &Request::Identity);
    let read = row_count(&mut io, &mut node, &mut now, 72);
    for _ in 0..50 { now += 1_000; io.tick(page::Ms(now)); settle(&mut io, &mut node, &mut now); }
    let delta: BTreeMap<&str, usize> = node.served.iter().map(|(k, n)| (*k, n - before.get(k).copied().unwrap_or(0))).filter(|(_, n)| *n > 0).collect();
    println!("returning: read {read:?}; served since open {delta:?}");
    assert_eq!(read, Some(40), "the returning identity did not read its rows");
    let writes: Vec<_> = delta.iter().filter(|(k, _)| ["put block", "put register", "update"].contains(k)).collect();
    assert!(writes.is_empty(), "opening a returning identity's tree wrote to the node: {writes:?}");
}

/// NO TREE YET: WRITES WAIT, VISIBLE, UNPUT; PROVISIONING CUTS ONE COMMIT
/// (#338; engine `CanSign`). An asked page whose node holds no key (or has no
/// signer) cannot sign, so writes -- a schema define, then a data write --
/// queue on the engine and apply to the warm root: a read sees both, and the
/// node is sent NOTHING. The first data write's claim provisions; the
/// provisioning is what turns `CanSign` true, and ONE commit carries the
/// whole queue: one head, both writes published.
#[test]
fn with_no_tree_writes_wait_visible_and_provisioning_cuts_one_commit() {
    for case in ["signer holds no key", "no signer here"] {
        let key = [37u8; 32];
        let mut node = WireNode::unprovisioned(&key);
        node.empty_until_registered = case == "no signer here";
        let mut now = 1_000;
        let mut io = asker();
        settle(&mut io, &mut node, &mut now);
        assert!(io.no_tree_yet(), "{case}: not taken as no tree yet");
        client(&mut io, &mut node, &mut now, &Request::Identity);
        let r1 = client(&mut io, &mut node, &mut now, &write(1, "schema/notes", "{}"));
        let r2 = client(&mut io, &mut node, &mut now, &write(2, "mine", "1"));
        // What the web Session's reads walk (R-a): the WARM root.
        let warm = io.server.page.warm_root();
        let leaf = freenet_prolly::store::Blocks::get(io.server.page.blocks(), &warm).map(|b| b.to_vec());
        let entries = leaf.as_deref().and_then(|b| freenet_prolly::node::Node::parse(b).ok()).map(|n| n.len());
        assert_eq!(entries, Some(2), "{case}: the queued writes are not in the warm root ({:?}, {:?})", states(&r1, 1), states(&r2, 2));
        let writes: BTreeMap<&str, usize> =
            node.served.iter().filter(|(k, _)| ["put block", "put register", "update"].contains(k)).map(|(k, n)| (*k, *n)).collect();
        assert!(writes.is_empty(), "{case}: writes were sent while the page could not sign: {writes:?}");
        assert!(![&r1, &r2].iter().any(|r| states(r, 1).contains(&WriteState::Published) || states(r, 2).contains(&WriteState::Published)), "{case}: published without a key");
        // The first data write's claim (what `Session::writable` does).
        let (container, _) = wire::delegate_from_code(SIGNER_CODE);
        assert!(io.claim(container), "{case}: the claim was refused");
        let mut st = settle(&mut io, &mut node, &mut now);
        if io.needs_key() {
            let sk = ed25519_dalek::SigningKey::from_bytes(&key);
            io.provision_with(sk.to_bytes().to_vec(), wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME));
            st.extend(settle(&mut io, &mut node, &mut now));
        }
        for _ in 0..100 {
            if [1, 2].iter().all(|id| states(&st, *id).contains(&WriteState::Published)) { break; }
            now += 500;
            io.tick(Ms(now));
            st.extend(settle(&mut io, &mut node, &mut now));
        }
        assert!([1, 2].iter().all(|id| states(&st, *id).contains(&WriteState::Published)), "{case}: the queue did not publish after provisioning: {:?}", io.unusable());
        let heads = node.served.get("put register").copied().unwrap_or(0) + node.served.get("update").copied().unwrap_or(0);
        assert_eq!(heads, 1, "{case}: {heads} head writes: the held queue was not ONE commit");
        assert_eq!(node.head().map(|(s, _)| s), Some(1), "{case}: the head is not seq 1");
    }
}

/// THE PUBLISHED-HEAD FLOOR (sdk#349): a view opened with the seq its app was
/// published at never adopts a head below it. The node first serves its
/// cached copy from BEFORE the publish (seq 1); the view shows nothing from
/// it -- no rows, not "empty" -- and says it is waiting for the published
/// version, re-asking the head on its RTO (a GET with subscribe). When the
/// node answers seq 2, the view reads it. Mutant "accept any head" (no
/// floor) -> red: the view reads the pre-publish rows.
#[test]
fn a_view_never_adopts_a_head_below_its_published_seq() {
    let mut node = WireNode::new(&[39u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    assert!(states(&client(&mut a, &mut node, &mut now, &write(1, "before", "v")), 1).contains(&WriteState::Published));
    let before_publish = node.contracts[&node.register_id].clone();
    assert!(states(&client(&mut a, &mut node, &mut now, &write(2, "published", "v")), 2).contains(&WriteState::Published));
    assert_eq!(node.head().map(|(s, _)| s), Some(2), "THE CONTROL: the published version is seq 2");

    node.stale_register = Some((before_publish, usize::MAX));
    let mut v = reader(&node);
    v.server.page.set_head_floor(2);
    let t0 = now;
    client_unsettled(&mut v, &mut node, &mut now, &Request::Identity);
    let early = rows_unsettled(&mut v, &mut node, &mut now, 91);
    println!("stale phase lasted {} s of page time", (now - t0) / 1000);
    let asked = node.served.get("get register").copied().unwrap_or(0);
    let wait = v.server.page.head_floor_wait();
    println!("below the floor: rows {early:?}, head GETs {asked}, wait {wait:?}");
    assert!(early.is_empty(), "the view showed rows {early:?} from a head below its published seq");
    assert!(asked >= 2, "THE CONTROL: the head was not asked again while below the floor ({asked} GETs)");
    assert!(matches!(wait, Some((2, Some(1), n)) if n >= 2), "the view does not say it waits for the published version: {wait:?}");

    node.stale_register = None;
    let mut read = Vec::new();
    for q in 0..200u64 {
        read = rows(&mut v, &mut node, &mut now, 100 + q);
        if read == vec![2] {
            break;
        }
        now += 1_000;
        v.tick(Ms(now));
    }
    assert_eq!(read, vec![2], "the view did not read the published version once the node served it");
    assert_eq!(v.server.page.head_floor_wait(), None, "the view still says it waits after adopting the published version");
}

/// THE FLOOR IS A SEQ, NOT A ROOT (sdk#349, #225b): a head AT the published
/// seq is the published version whatever its root (a same-seq race may have
/// replaced it), so it is read at once, with no wait. Mutant "strictly above
/// the floor" -> red: the view waits for ever on the version it was given.
#[test]
fn a_head_at_the_published_seq_is_read_at_once() {
    let mut node = WireNode::new(&[40u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    for i in 1..=2u64 {
        assert!(states(&client(&mut a, &mut node, &mut now, &write(i, &format!("k{i}"), "v")), i).contains(&WriteState::Published));
    }
    let mut v = reader(&node);
    v.server.page.set_head_floor(2);
    client(&mut v, &mut node, &mut now, &Request::Identity);
    assert_eq!(rows(&mut v, &mut node, &mut now, 92), vec![2], "a head at the published seq was not read");
    assert_eq!(v.server.page.head_floor_wait(), None, "a view at its published seq says it is still waiting");
}

/// WHAT A PUBLISHER RECORDS AS ITS APP'S FLOOR IS THE ACKNOWLEDGED SEQ
/// (sdk#349, the architect's condition): with a second commit in flight --
/// signed and UPDATEd, its read-back not yet showing it -- `published_seq`
/// is still the FIRST commit's, so a publish at that moment never names a
/// head that may not land. Once the register shows it, it moves. Mutant
/// "the in-flight commit's seq" -> red.
#[test]
fn published_seq_is_the_acknowledged_head_never_one_in_flight() {
    let mut node = WireNode::new(&[41u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    assert!(states(&client(&mut a, &mut node, &mut now, &write(1, "first", "v")), 1).contains(&WriteState::Published));
    assert_eq!(a.published_seq(), 1);
    // The register's reads fail: the second commit is signed and UPDATEd, and
    // its read-back never shows it.
    node.fail_register_gets = usize::MAX;
    let r = client_unsettled(&mut a, &mut node, &mut now, &write(2, "second", "v"));
    assert!(!states(&r, 2).contains(&WriteState::Published), "THE CONTROL: the second commit was acknowledged anyway");
    assert_eq!(node.head().map(|(s, _)| s), Some(2), "THE CONTROL: the second commit's head is not on the node");
    assert_eq!(a.published_seq(), 1, "the publisher's seq moved to a commit the network has not acknowledged");
    node.fail_register_gets = 0;
    let mut st = Vec::new();
    for _ in 0..100 {
        if st.contains(&WriteState::Published) { break; }
        now += 1_000;
        a.tick(Ms(now));
        st.extend(states(&run_unsettled(&mut a, &mut node, &mut now), 2));
    }
    assert!(st.contains(&WriteState::Published), "the second commit never published: {:?}", a.unusable());
    assert_eq!(a.published_seq(), 2, "the acknowledged seq did not move with the read-back");
}

// ---------------------------------------------------------------------------------------------------------------
// A SITE through page-io (builder#117): real frames, the REAL signer, the Register's merge under the site params.

fn web(n: u8) -> Vec<u8> {
    vec![n; 300]
}

/// A site's STARTER (app-as-data P4): the build's files, without the pointer page-io adds.
fn starter(n: u8) -> Vec<(String, Vec<u8>)> {
    vec![("index.html".to_string(), vec![n; 300]), ("loader.js".to_string(), vec![n.wrapping_add(1); 50])]
}

/// The site `app`'s publish of [`starter`] `n` holds: the starter and the pointer to `node`'s register, as page-io
/// composes it (`wire::webapp`'s one owners).
fn site_web(node: &WireNode, app: &str, n: u8) -> Vec<u8> {
    let pointer = wire::webapp::site_pointer(&node.register_params, app).expect("a pointer");
    let st = starter(n);
    let mut files: Vec<(&str, &[u8])> = st.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect();
    files.push((wire::webapp::POINTER_PATH, &pointer));
    wire::webapp::app_web(&files).expect("a web")
}

fn publish(io: &mut PageIo, node: &mut WireNode, now: &mut u64, n: u8) -> Option<page::Publication> {
    io.publish_site(APP, SITE_CODE, web(n), Ms(*now)).expect("publishes");
    settle(io, node, now);
    io.publication(APP)
}

/// **One address, each publish the next version.** The node holds the signer's record framed around exactly the
/// web part it names, at the same contract every time (the stable link), and page-io keeps no bytes once each
/// publication ends.
#[test]
fn a_site_publishes_each_version_at_one_address() {
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 1_000;
    for v in 1..=2u8 {
        assert_eq!(publish(&mut io, &mut node, &mut now, v), Some(page::Publication::Published { version: v as u64 }));
        let (seq, value, w) = node.site().expect("the node holds the site");
        assert_eq!((seq, value, w.clone()), (v as u64, blake3::hash(&web(v)).as_bytes().to_vec(), web(v)), "v{v} is not (seq, blake3(web), web)");
    }
    assert_eq!(node.served.get("put site"), Some(&2), "{:?}", node.served);
    assert_eq!(node.contracts.keys().filter(|id| **id == node.site_id()).count(), 1);
    assert_eq!(node.register_puts, 0, "publishing a site PUT the person's head");
    assert!(io.unusable().is_empty(), "{:?}", io.unusable());
    // Another device of the person follows it: v3 at the same address.
    let mut other = page_io(&node);
    assert_eq!(publish(&mut other, &mut node, &mut now, 3), Some(page::Publication::Published { version: 3 }));
}

/// **A REFUSED site read is silence, never "no site"** (the GetFail split): a new device whose node does not hold
/// the site yet has its read refused once; the re-ask reads v2 and it publishes v3. Mutant "a refused site read is
/// NotFound" -> it signs v1 from the genesis, the merge keeps v2 -> Superseded -> red.
#[test]
fn a_refused_site_read_is_re_asked_never_the_genesis() {
    let mut node = WireNode::new(&[3u8; 32]);
    let mut now = 1_000;
    let mut first = page_io(&node);
    for v in 1..=2u8 {
        publish(&mut first, &mut node, &mut now, v);
    }
    // A NEW device: its signer holds no record of the site (the first device's signer kept it).
    let label = signer::Label::Site { app: APP.into(), contract: node.site_id() };
    assert!(node.secrets.remove(&signer::record_name(&label)).is_some(), "THE SETUP: the first device kept no site record");
    let mut io = page_io(&node);
    node.refuse_site_gets = 1;
    node.site_blind_signs = 2;
    assert_eq!(publish(&mut io, &mut node, &mut now, 9), Some(page::Publication::Published { version: 3 }));
    assert_eq!(node.served.get("refused site get"), Some(&1), "THE SETUP: the site read was not refused: {:?}", node.served);
}

/// **A site's PUT refused by the node ENDS the publication, in its words.**
#[test]
fn a_refused_site_put_ends_the_publication_by_name() {
    use freenet_stdlib::client_api::{ContractError, ErrorKind, RequestError};
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 1_000;
    io.publish_site(APP, SITE_CODE, web(1), Ms(now)).expect("publishes");
    // Serve until the site's PUT goes out, and refuse it.
    let site = page_io::site_contract(SITE_CODE, &node.register_params, APP).expect("site").key();
    for _ in 0..50 {
        now += 1;
        for f in io.take_frames() {
            let req: ClientRequest = bincode::deserialize(&f).expect("a request");
            let answer = match req {
                ClientRequest::ContractOp(ContractRequest::Put { contract, .. }) if contract.key() == site => {
                    let e: Err = ErrorKind::RequestError(RequestError::ContractError(ContractError::Put { key: site, cause: "invalid put".into() })).into();
                    Some(bincode::serialize(&Err::<HostResponse, Err>(e)).expect("encodes"))
                }
                _ => node.serve(&f),
            };
            if let Some(a) = answer {
                io.inbound(&a, Ms(now));
            }
        }
    }
    assert!(matches!(io.publication(APP), Some(page::Publication::Refused(w)) if w.contains("invalid put")), "{:?}", io.publication(APP));
    assert_eq!(node.site(), None);
    assert!(io.take_others().is_empty(), "the site's refusal was handed back as somebody else's");
}

/// **A reader, and a label that is no app id, publish nothing** -- by name, and no frame leaves.
#[test]
fn a_reader_or_a_bad_app_id_publishes_nothing() {
    let node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let _ = io.take_frames();
    let bad = io.publish_site("Not An App!", SITE_CODE, web(1), Ms(1));
    assert!(bad.is_err_and(|e| e.contains("not an app id")), "a bad app id was published");
    assert!(io.take_frames().is_empty(), "a bad app id sent a frame");
    let mut r = reader(&node);
    let _ = r.take_frames();
    assert!(r.publish_site(APP, SITE_CODE, web(1), Ms(1)).is_err_and(|e| e.starts_with("read-only")), "a reader published");
    assert!(r.take_frames().is_empty(), "a reader sent a frame");
}

/// **sdk#378 P3 through real frames:** the node pushes the head register's FULL new state (an
/// `UpdateNotification`, the GET's subscription) before it answers the UPDATE, and that push IS the save's
/// read-back: the write is Published and page-io sends NO register GET for it. THE CONTROL, the same write with
/// no push: one register GET (the read-back). Mutant "page-io takes a push as a bare hint" -> a GET -> red.
#[test]
fn a_pushed_full_state_is_the_saves_read_back_and_no_register_get_is_sent() {
    let mut gets = Vec::new();
    for push in [true, false] {
        let mut node = WireNode::new(&[8u8; 32]);
        let mut io = page_io(&node);
        let mut now = 1_000;
        client(&mut io, &mut node, &mut now, &Request::Identity);
        assert!(states(&client(&mut io, &mut node, &mut now, &write(1, "a", "1")), 1).contains(&WriteState::Published), "THE SETUP: the first write (the register's creation) did not publish");
        node.push_updates = push;
        let before = node.served.get("get register").copied().unwrap_or(0);
        // Counted until the write is PUBLISHED, not until the page is idle: an idle page reads the register on
        // its backstop, and `settle` would jump the clock through hundreds of those.
        io.client(&protocol::encode_session_request(4, 9, &write(2, "b", "2")).expect("encodes"));
        let mut r = Vec::new();
        for _ in 0..200 {
            r.extend(pump(&mut io, &mut node, &mut now));
            if states(&r, 2).contains(&WriteState::Published) {
                break;
            }
            now += 50;
            io.tick(Ms(now));
        }
        assert!(states(&r, 2).contains(&WriteState::Published), "push={push}: the write did not publish: {r:?}");
        assert_eq!(node.served.get("update").copied(), Some(1), "push={push}: THE SETUP: the second head was not an UPDATE");
        gets.push(node.served.get("get register").copied().unwrap_or(0) - before);
    }
    assert_eq!(gets, vec![0, 1], "register GETs for the save (with the push, without): the push did not stand in for the read-back");
}

/// **sdk#378 P3, condition 1: ONLY a full state is ever judged** (the architect). A push carrying the record's
/// bytes as a DELTA, or as `StateAndDelta`, is decoded with no state (`HeadChanged { state: None }`), so it is a
/// hint: the save's read-back GET still goes. The delta carries the register's own record, the bytes that WOULD
/// parse as a plausible head. Mutant "wire takes any update kind's bytes as the state" -> red.
#[test]
fn a_delta_push_is_never_judged_as_a_state_and_the_read_back_still_reads() {
    for kind in [PushKind::Delta, PushKind::StateAndDelta] {
        let mut node = WireNode::new(&[8u8; 32]);
        let mut io = page_io(&node);
        let mut now = 1_000;
        client(&mut io, &mut node, &mut now, &Request::Identity);
        assert!(states(&client(&mut io, &mut node, &mut now, &write(1, "a", "1")), 1).contains(&WriteState::Published), "THE SETUP: the first write did not publish");
        node.push_updates = true;
        node.push_kind = kind;
        let before = node.served.get("get register").copied().unwrap_or(0);
        io.client(&protocol::encode_session_request(4, 9, &write(2, "b", "2")).expect("encodes"));
        let mut r = Vec::new();
        for _ in 0..200 {
            r.extend(pump(&mut io, &mut node, &mut now));
            if states(&r, 2).contains(&WriteState::Published) {
                break;
            }
            now += 50;
            io.tick(Ms(now));
        }
        assert!(states(&r, 2).contains(&WriteState::Published), "{kind:?}: the write did not publish: {r:?}");
        let gets = node.served.get("get register").copied().unwrap_or(0) - before;
        assert_eq!(gets, 1, "{kind:?}: a push whose state is not a FULL state stood in for the read-back ({gets} register GETs)");
    }
    // And at the decoder itself: the same bytes as a Delta give no state; as a State, they do.
    let key = ContractKey::from_id_and_code(ContractInstanceId::new([3u8; 32]), CodeHash::new([0u8; 32]));
    let decode = |update: UpdateData<'static>| match wire::unframe(&mut wire::Reassembler::default(), &ok(HostResponse::ContractResponse(ContractResponse::UpdateNotification { key, update }))) {
        wire::Incoming::HeadChanged { state, .. } => state,
        other => panic!("not a HeadChanged: {other:?}"),
    };
    assert_eq!(decode(UpdateData::Delta(StateDelta::from(vec![1u8, 2, 3]))), None, "a delta was decoded as a state");
    assert_eq!(decode(UpdateData::State(State::from(vec![1u8, 2, 3]))), Some(vec![1u8, 2, 3]), "THE CONTROL: a full state was not decoded as one");
}

/// **settle's CAP IS A FAILURE, never an end** (main, on #403): a page the test keeps from quiescence -- its signer
/// never answers, so the ask stays owed -- driven through `settle` ITSELF (not `run_unsettled`) must panic "did not
/// settle". Without this, a settle that quietly returned at its cap would pass every count test as before.
#[test]
#[should_panic(expected = "did not settle")]
fn control_settle_panics_on_a_page_that_never_settles() {
    let mut node = WireNode::new(&[44u8; 32]);
    node.drop_signer_answers = usize::MAX;
    let mut io = asker();
    let mut now = 1_000;
    settle(&mut io, &mut node, &mut now);
}

/// THE LIVE SHAPE of sdk#431 (a parity PUT answered only with errors, sent 3 times and never again, yet the rows
/// backed up): a LATER commit re-codes the errored parity's group, and its PUT is WITHDRAWN (Effect::Withdraw,
/// COMMIT-LIFE §P) -- nobody needs it any more. BACKED_UP then rests on the RE-CODED group's parity: the node takes
/// it -> both writes ParityComplete; one of those errors too -> neither is. Keyed and keyless errors alike.
#[test]
fn an_errored_parity_superseded_by_a_later_commit_is_withdrawn_and_backed_up_only_by_the_re_coded_group() {
    for keyed in [true, false] {
        for (case, new_group_errors) in [("the re-coded group is taken", false), ("a re-coded parity errors too", true)] {
            let label = format!("{} error, {case}", if keyed { "keyed" } else { "keyless" });
            let (mut io, mut node, mut now) = opened_at(1_790_253_181_367, &label);
            io.server.page.record_into(1 << 16);
            let before = node.block_puts.len();
            node.err_block_puts = usize::MAX;
            node.err_keyed = keyed;
            io.client(&protocol::encode_session_request(4, 9, &write(1, "a", "1")).expect("encodes"));
            let mut r = live(&mut io, &mut node, &mut now, 2_000);
            let failed = node.err_block.unwrap_or_else(|| panic!("{label}: THE SETUP: no parity PUT was answered with an error"));
            if new_group_errors {
                node.err_block = None;
            }
            io.client(&protocol::encode_session_request(4, 9, &write(2, "b", "2")).expect("encodes"));
            r.extend(live(&mut io, &mut node, &mut now, 70_000));
            if new_group_errors {
                assert!(node.err_block.is_some_and(|b| b != failed), "{label}: THE SETUP: no parity of the re-coded group was answered with an error");
            }
            let sends: Vec<u64> = node.block_puts[before..].iter().filter(|(_, id)| *id == failed).map(|(t, _)| *t).collect();
            let dump = io.server.page.dump(usize::MAX);
            let withdrawn = dump.lines().filter(|l| l.contains("exit   page::op::put") && l.contains("Withdrawn")).count();
            println!("  {label}: the errored parity reached the node at +{:?} ms; write 1 {:?}; write 2 {:?}; put Withdrawn {withdrawn}", sends.iter().map(|t| t - sends[0]).collect::<Vec<_>>(), states(&r, 1), states(&r, 2));
            assert!(withdrawn >= 1, "{label}: the superseded parity PUT was not WITHDRAWN:\n{dump}");
            assert!(sends.last().expect("sent") - sends[0] < 2_000, "{label}: the superseded parity PUT was still re-sent after the later commit: {sends:?}");
            let backed = |w| states(&r, w).contains(&WriteState::ParityComplete);
            if new_group_errors {
                assert!(!backed(1) && !backed(2), "{label}: BACKED UP with a parity of the re-coded group answered only with errors");
            } else {
                assert!(backed(1) && backed(2), "{label}: THE CONTROL: the writes did not back up on the re-coded group: {:?} / {:?}", states(&r, 1), states(&r, 2));
            }
        }
    }
}

/// A reader page with `params` (sdk#411's budget).
fn reader_with(node: &WireNode, params: engine::Params) -> PageIo {
    PageIo::reader(Server::new(Page::unstarted(params, PutPath::Page, Ms(0)), SignerFacts::default()), BLOCK_CODE.to_vec(), node.register_id, 1)
}

/// Ask `key` of `io` and serve its frames until it is answered (bounded), counting every block GET by id.
fn get_counting(io: &mut PageIo, node: &mut WireNode, now: &mut u64, req_id: u64, key: &[u8], gets: &mut BTreeMap<[u8; 32], usize>) -> Option<Vec<u8>> {
    io.client(&protocol::encode_session_request(4, 9, &Request::Get { req_id, key: key.to_vec() }).expect("encodes"));
    for _ in 0..2_000 {
        let frames = io.take_frames();
        for r in io.take_replies() {
            if let Reply::Value { req_id: q, value } = protocol::decode_reply(&r).expect("a reply") {
                if q == req_id {
                    return value;
                }
            }
        }
        if frames.is_empty() {
            *now = io.next_due().map_or(*now + 1, |Ms(t)| t.max(*now + 1));
            io.tick(Ms(*now));
            continue;
        }
        *now += 1;
        for f in frames {
            if let ClientRequest::ContractOp(ContractRequest::Get { key, .. }) = bincode::deserialize::<ClientRequest>(&f).expect("a request") {
                let id: [u8; 32] = key.as_bytes()[..32].try_into().expect("32");
                if id != node.register_id {
                    *gets.entry(id).or_default() += 1;
                }
            }
            let answer = node.serve(&f);
            if let Some(a) = answer {
                io.inbound(&a, Ms(*now));
            }
        }
    }
    panic!("read {req_id} of {:?} was never answered", String::from_utf8_lossy(key));
}

/// **THE THRASH TEST (sdk#411, the architect): LRU, not FIFO.** A reader whose store budget is a third of the tree
/// reads keys across every leaf, five passes over. Every read is answered (it terminates), and the ROOT -- which
/// every read walks, so it is always the most recently used -- is fetched ONCE: eviction takes the cold leaves,
/// never the hot upper tree. Mutant FIFO (a read does not move its block to the back): the root is the oldest
/// block, evicted first, fetched again and again -> red.
#[test]
fn a_read_loop_over_more_than_the_budget_terminates_and_never_refetches_the_root() {
    let mut node = WireNode::new(&[31u8; 32]);
    let mut a = page_io(&node);
    let mut now = 1_000;
    client(&mut a, &mut node, &mut now, &Request::Identity);
    // Enough rows that one read's race-get group is a SMALL part of the tree: a budget below one read's own
    // fetch thrashes under any policy (measured at 400 rows: the tree was one group).
    for (w, lo) in (0..4_000u32).step_by(500).enumerate() {
        let ops: Vec<protocol::Op> = (lo..lo + 500).map(|i| protocol::Op::Put(format!("t/{i:05}").into_bytes(), vec![(i % 251) as u8; 60])).collect();
        assert!(states(&client(&mut a, &mut node, &mut now, &Request::forced_write(w as u64 + 1, ops)), w as u64 + 1).contains(&WriteState::Published), "THE SETUP: rows {lo}.. did not publish");
    }
    // A block's GET names its CONTRACT (the Block code over its id), not the id itself.
    let root = wire::block::contract_for(BLOCK_CODE, &node.head().expect("a head").1);
    let keys: Vec<Vec<u8>> = (0..4_000u32).step_by(50).map(|i| format!("t/{i:05}").into_bytes()).collect();

    // CALIBRATE: what the whole working set costs, read once with room for all of it.
    let mut cal = reader_with(&node, engine::Params::default());
    client(&mut cal, &mut node, &mut now, &Request::Identity);
    let mut ignore = BTreeMap::new();
    for (i, k) in keys.iter().enumerate() {
        get_counting(&mut cal, &mut node, &mut now, 100 + i as u64, k, &mut ignore);
    }
    let working = cal.server.page.blocks().bytes();

    let budget = working / 3;
    let mut v = reader_with(&node, engine::Params { max_page_block_bytes: budget, ..engine::Params::default() });
    client(&mut v, &mut node, &mut now, &Request::Identity);
    let mut gets = BTreeMap::new();
    for pass in 0..5u64 {
        for (i, k) in keys.iter().enumerate() {
            let got = get_counting(&mut v, &mut node, &mut now, 1_000 * (pass + 1) + i as u64, k, &mut gets);
            let n: u32 = std::str::from_utf8(&k[2..]).expect("utf8").parse().expect("n");
            assert_eq!(got, Some(vec![(n % 251) as u8; 60]), "pass {pass}: a read under eviction answered wrong");
        }
    }
    let stats = v.server.page.blocks().stats();
    println!("thrash: working set {working} B, budget {budget} B, evicted {} blocks / {} B, block GETs {} ({} distinct), root GETs {}", stats.evicted, stats.evicted_bytes, gets.values().sum::<usize>(), gets.len(), gets.get(&root).copied().unwrap_or(0));
    assert!(stats.evicted > 0, "THE SETUP: nothing was evicted, so this measured no thrash");
    assert_eq!(gets.get(&root).copied(), Some(1), "the ROOT was fetched more than once: eviction took the hot upper tree (not LRU)");
}

/// A writer page with `params` (sdk#411's budget).
fn page_io_with(node: &WireNode, params: engine::Params) -> PageIo {
    let (_, signer) = wire::delegate_from_code(SIGNER_CODE);
    let mut io = PageIo::new(
        Server::new(Page::unstarted(params, PutPath::Page, Ms(0)), SignerFacts::default()),
        Artefacts { block_code: BLOCK_CODE.to_vec(), register_code: REGISTER_CODE.to_vec(), register_params: node.register_params.clone(), signer },
    );
    io.signer_provisioned();
    io
}

/// **THE LONG RUN (the architect's guard on the tiered order):** tier 2 is by KIND, so the internal nodes of every
/// SUPERSEDED root a page committed stay in tier 2 and could outlive the CURRENT tree's leaves. A writer under a
/// budget commits ~200 heads, then reads the current tree: the current root is fetched at most once, and its block
/// GETs stay within the bound of a FRESH page with the same budget reading the same keys.
#[test]
fn after_two_hundred_heads_the_current_tree_reads_like_a_fresh_pages() {
    let mut node = WireNode::new(&[33u8; 32]);
    let mut now = 1_000;
    // The budget: a third of the CURRENT tree's working set for these reads (calibrated, as the thrash test).
    let rows = |lo: u32, hi: u32, tag: u8| -> Vec<protocol::Op> { (lo..hi).map(|i| protocol::Op::Put(format!("t/{i:05}").into_bytes(), vec![tag; 60])).collect() };
    let mut seed = page_io(&node);
    client(&mut seed, &mut node, &mut now, &Request::Identity);
    for (w, lo) in (0..4_000u32).step_by(500).enumerate() {
        assert!(states(&client(&mut seed, &mut node, &mut now, &Request::forced_write(w as u64 + 1, rows(lo, lo + 500, 1))), w as u64 + 1).contains(&WriteState::Published), "THE SETUP: rows {lo}.. did not publish");
    }
    let keys: Vec<Vec<u8>> = (0..4_000u32).step_by(50).map(|i| format!("t/{i:05}").into_bytes()).collect();
    let mut cal = reader_with(&node, engine::Params::default());
    client(&mut cal, &mut node, &mut now, &Request::Identity);
    let mut ignore = BTreeMap::new();
    for (i, k) in keys.iter().enumerate() {
        get_counting(&mut cal, &mut node, &mut now, 100 + i as u64, k, &mut ignore);
    }
    let budget = cal.server.page.blocks().bytes() / 3;
    let params = engine::Params { max_page_block_bytes: budget, ..engine::Params::default() };

    // THE LONG RUN: a writer under the budget commits 200 more heads, one row each, spread over the tree.
    let mut w = page_io_with(&node, params);
    client(&mut w, &mut node, &mut now, &Request::Identity);
    for n in 0..200u64 {
        let i = (n as u32 * 397) % 4_000;
        assert!(states(&client(&mut w, &mut node, &mut now, &Request::forced_write(100 + n, rows(i, i + 1, 2))), 100 + n).contains(&WriteState::Published), "THE SETUP: head {n} did not publish");
    }
    let root = wire::block::contract_for(BLOCK_CODE, &node.head().expect("a head").1);
    let read = |io: &mut PageIo, node: &mut WireNode, now: &mut u64, base: u64| -> BTreeMap<[u8; 32], usize> {
        let mut gets = BTreeMap::new();
        for pass in 0..2u64 {
            for (i, k) in keys.iter().enumerate() {
                assert!(get_counting(io, node, now, base + 1_000 * pass + i as u64, k, &mut gets).is_some(), "a read of the current tree found nothing");
            }
        }
        gets
    };
    let long = read(&mut w, &mut node, &mut now, 10_000);
    let mut fresh = reader_with(&node, params);
    client(&mut fresh, &mut node, &mut now, &Request::Identity);
    let fresh_gets = read(&mut fresh, &mut node, &mut now, 20_000);
    let (l, f) = (long.values().sum::<usize>(), fresh_gets.values().sum::<usize>());
    let st = w.server.page.blocks().stats();
    println!("long run: budget {budget} B; after 200 heads the writer's reads cost {l} block GETs (root {}); a fresh page's {f} (root {}); writer evicted {} blocks, store {} B", long.get(&root).copied().unwrap_or(0), fresh_gets.get(&root).copied().unwrap_or(0), st.evicted, w.server.page.blocks().bytes());
    assert!(long.get(&root).copied().unwrap_or(0) <= 1, "after 200 heads the CURRENT root was fetched more than once: superseded internal nodes crowded it out");
    assert!(l <= f + f / 4, "after 200 heads the writer's reads cost {l} GETs against a fresh page's {f}: superseded internal nodes crowd the current tree");
}

/// EVERY FRAME KIND `owns()` CLAIMS IS DECIDED (sdk#483): the ones that fell into `inbound`'s `_ => {}` and vanished
/// now each have an arm, and each arm's decision is visible. (That no new kind can vanish is the COMPILER's: the match
/// has no catch-all.)
#[test]
fn every_frame_kind_the_page_claims_is_decided_never_dropped() {
    let node = WireNode::new(&[4u8; 32]);
    let register = ContractKey::from_id_and_code(ContractInstanceId::new(node.register_id), CodeHash::new([0u8; 32]));

    // The node confirms the HEAD subscription: recorded as answered, where `head_subscription()` reads it. The page
    // has SENT its head read with subscribe (an ack to a page that asked nothing is impossible, sdk#490's table).
    let mut io = page_io(&node);
    io.client(&protocol::encode_session_request(4, 9, &Request::Identity).expect("encodes"));
    io.take_frames();
    assert!(io.head_subscription().asked && !io.head_subscription().answered, "THE SETUP: the head read is not out, unanswered: {:?}", io.head_subscription());
    assert!(io.inbound(&ok(HostResponse::ContractResponse(ContractResponse::SubscribeResponse { key: register, subscribed: true })), Ms(1)), "the head's subscription answer was not claimed");
    assert!(io.head_subscription().answered, "the node's Subscribed for the head was dropped");

    // `Ok` names nothing: taken, and nothing is made of it (no op ends on it, nothing is reported).
    let mut io = page_io(&node);
    assert!(io.inbound(&ok(HostResponse::Ok), Ms(1)));
    assert!(io.unusable().is_empty() && io.take_others().is_empty(), "an Ok was read as something: {:?}", io.unusable());

    // A registration of a delegate this page did not register: NAMED, not dropped.
    let mut io = page_io(&node);
    let (_, other) = wire::delegate_from_code(b"a delegate this page never registered");
    assert!(io.inbound(&ok(HostResponse::DelegateResponse { key: other.clone(), values: Vec::new() }), Ms(1)));
    assert_eq!(io.unusable(), [format!("the node registered a delegate this page did not register: {other}")]);

    // A CHUNK of a larger message: taken, and nothing handed on until the message is whole.
    let mut io = page_io(&node);
    let chunk = ok(HostResponse::StreamChunk { stream_id: 7, index: 0, total: 2, data: vec![1, 2, 3].into() });
    assert!(io.inbound(&chunk, Ms(1)), "a chunk was not claimed");
    assert!(io.unusable().is_empty(), "a first chunk was reported: {:?}", io.unusable());
}

/// ONE FINALITY RULE for an app contract's PUT (a load piece, sdk#516), as for a block's (sdk#433): FINAL only in a
/// contract's own validation words, KEYED or KEYLESS (the text form 0.2.136/0.2.138 send); any other words are
/// transient -- reported, and the PUT stays pending, re-sent on its RTO. Before, a keyless refusal of an app PUT was
/// never attributed (re-sent for ever) and a keyed one was final in ANY words. THE CONTROL: a keyless refusal naming
/// a contract this page never PUT is counted unattributed and ends nothing.
#[test]
fn an_app_contracts_put_refusal_is_final_only_in_validation_words_keyed_or_keyless() {
    use freenet_stdlib::client_api::{ContractError, ErrorKind, RequestError};
    let node = WireNode::new(&[7u8; 32]);
    let keyed = |key: ContractKey, cause: &str| {
        let e: Err = ErrorKind::RequestError(RequestError::ContractError(ContractError::Put { key, cause: cause.to_string().into() })).into();
        bincode::serialize(&Err::<HostResponse, Err>(e)).expect("encodes")
    };
    let validation = wire::VALIDATION_REFUSED[1];
    let other = "execution error: the contract's host is busy";
    assert!(wire::is_validation_refusal(validation) && !wire::is_validation_refusal(other), "THE SETUP: the words are not the rule's two kinds");
    for (form, words, fin) in [("keyed", validation, true), ("keyed", other, false), ("keyless", validation, true), ("keyless", other, false)] {
        let mut io = page_io(&node);
        let (key, c, s) = wire::puts::contract(b"a load piece", format!("{form} {words}").as_bytes(), b"s");
        io.put_contract(c.clone(), s, Ms(1)).expect("frames");
        let frame = if form == "keyed" { keyed(c.key(), words) } else { put_error_text(key.clone(), words) };
        assert!(io.inbound(&frame, Ms(2)), "{form} {words:?}: the refusal of the page's own app PUT was not taken");
        let now = io.app_put(&key).cloned();
        if fin {
            assert_eq!(now, Some(page::AppPut::Refused(words.into())), "{form}: a validation refusal did not END the app PUT");
        } else {
            assert_eq!(now, Some(page::AppPut::Pending), "{form}: a transient refusal ended the app PUT");
            assert!(io.unusable().iter().any(|l| l.contains(other)), "{form}: a transient refusal was not reported: {:?}", io.unusable());
        }
        assert!(io.take_others().is_empty(), "{form}: the page's own refusal was handed back as somebody else's");
    }
    // THE CONTROL: a keyless refusal naming no PUT of this page is counted, and ends nothing.
    let mut io = page_io(&node);
    let (stranger, _, _) = wire::puts::contract(b"a load piece", b"never put here", b"s");
    io.inbound(&put_error_text(stranger, validation), Ms(3));
    assert_eq!(io.node_errors().get("put_error_unattributed"), Some(&1), "a refusal of nothing of ours was attributed");
}

/// THE SITE TAKES THE ONE FINALITY RULE (sdk#516, main): its FIRST PUT is refused, keyed or keyless, in validation
/// words or other words. Validation words END the publication (`Refused`, in the node's words); other words are
/// transient -- reported, and the site's PUT is re-sent on its RTO until the node takes it (`Published`). Before, a
/// keyed site refusal ended the publication in ANY words, and a keyless one was never attributed. THE CONTROL: every
/// case refused exactly one PUT (so a publication that never PUT cannot pass).
#[test]
fn a_sites_put_refusal_is_final_only_in_validation_words_keyed_or_keyless() {
    use freenet_stdlib::client_api::{ContractError, ErrorKind, RequestError};
    let validation = wire::VALIDATION_REFUSED[1];
    let other = "execution error: the contract's host is busy";
    for (form, words, fin) in [("keyed", validation, true), ("keyed", other, false), ("keyless", validation, true), ("keyless", other, false)] {
        let mut node = WireNode::new(&[3u8; 32]);
        let mut io = page_io(&node);
        let mut now = 1_000;
        io.publish_site(APP, SITE_CODE, web(1), Ms(now)).expect("publishes");
        let site = page_io::site_contract(SITE_CODE, &node.register_params, APP).expect("site").key();
        let mut refuses = 0;
        for _ in 0..400 {
            now += 250;
            io.tick(Ms(now));
            for f in io.take_frames() {
                let req: ClientRequest = bincode::deserialize(&f).expect("a request");
                let answer = match req {
                    ClientRequest::ContractOp(ContractRequest::Put { contract, .. }) if contract.key() == site && refuses == 0 => {
                        refuses += 1;
                        Some(if form == "keyed" {
                            let e: Err = ErrorKind::RequestError(RequestError::ContractError(ContractError::Put { key: site, cause: words.into() })).into();
                            bincode::serialize(&Err::<HostResponse, Err>(e)).expect("encodes")
                        } else {
                            put_error_text(site.to_string(), words)
                        })
                    }
                    _ => node.serve(&f),
                };
                if let Some(a) = answer {
                    io.inbound(&a, Ms(now));
                }
            }
            if matches!(io.publication(APP), Some(page::Publication::Refused(_) | page::Publication::Published { .. })) {
                break;
            }
        }
        assert_eq!(refuses, 1, "{form} {words:?}: THE CONTROL: not exactly one site PUT was refused");
        if fin {
            assert!(matches!(io.publication(APP), Some(page::Publication::Refused(w)) if w.contains(words)), "{form}: a validation refusal did not END the publication: {:?}", io.publication(APP));
        } else {
            assert!(matches!(io.publication(APP), Some(page::Publication::Published { .. })), "{form}: a transient refusal was not re-sent to Published: {:?}", io.publication(APP));
            assert!(io.unusable().iter().any(|l| l.contains(other)), "{form}: a transient site refusal was not reported: {:?}", io.unusable());
        }
        assert!(io.take_others().is_empty(), "{form}: the site's refusal was handed back as somebody else's");
    }
}

// ---- AN APP PUBLISH ON THE WIRE (sdk#516, APP-PUBLISH.md) ------------------------------------------------------------

/// A piece set `tag` of `n` pieces (k, m), and its containers in the set's order.
fn piece_set(tag: &str, n: usize, k: usize, m: usize) -> (pieces::PieceSet, Vec<(ContractContainer, WrappedState)>) {
    let pieces: Vec<_> = (0..n).map(|i| wire::puts::contract(b"a piece's webapp code", format!("{tag} piece {i}").as_bytes(), format!("{tag} piece {i}'s state").as_bytes())).collect();
    let set = pieces::PieceSet { name: tag.into(), k, m, pieces: pieces.iter().map(|(_, c, _)| pieces::NamedPiece { address: c.key().id().encode(), sha256: [0; 32] }).collect() };
    (set, pieces.into_iter().map(|(_, c, s)| (c, s)).collect())
}

/// What the node does with one piece's PUT.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PieceFate {
    Serve,
    Withhold,
    /// A FINAL refusal (the node's validation words, keyless).
    Refuse,
}

/// Where an app's publish stands, by its case's name (`None`: never published).
fn stage(io: &PageIo, app: &str) -> Option<String> {
    io.app_publish(app).map(|a| format!("{a:?}").split([' ', '{']).next().unwrap_or_default().to_string())
}

/// Serve `io`'s frames until `done` or 400 rounds, each piece PUT by `fate` (a withheld one is kept and returned; a
/// refused one refused once, finally), a site PUT withheld when `hold_site`. Returns the withheld piece frames and the
/// piece acks served when the FIRST site PUT left.
fn drive(io: &mut PageIo, node: &mut WireNode, now: &mut u64, fate: &dyn Fn(&ContractKey) -> Option<PieceFate>, hold_site: bool, done: &dyn Fn(&PageIo) -> bool) -> (Vec<Vec<u8>>, Option<usize>) {
    let site = page_io::site_contract(SITE_CODE, &node.register_params, APP).expect("site").key();
    let (mut acked, mut site_after, mut held, mut refused) = (0usize, None, Vec::new(), std::collections::HashSet::new());
    for _ in 0..400 {
        *now += 250;
        io.tick(Ms(*now));
        for f in io.take_frames() {
            let req: ClientRequest = bincode::deserialize(&f).expect("a request");
            if let ClientRequest::ContractOp(ContractRequest::Put { contract, .. }) = &req {
                let key = contract.key();
                if key == site {
                    site_after.get_or_insert(acked);
                    if hold_site {
                        continue;
                    }
                }
                match fate(&key) {
                    Some(PieceFate::Withhold) => {
                        held.push(f);
                        continue;
                    }
                    Some(PieceFate::Refuse) if refused.insert(key) => {
                        io.inbound(&put_error_text(key.to_string(), wire::VALIDATION_REFUSED[1]), Ms(*now));
                        continue;
                    }
                    Some(PieceFate::Serve | PieceFate::Refuse) => acked += 1,
                    None => {}
                }
            }
            if let Some(a) = node.serve(&f) {
                io.inbound(&a, Ms(*now));
            }
        }
        if done(io) {
            break;
        }
    }
    (held, site_after)
}

/// Serve the withheld frames now.
fn release(io: &mut PageIo, node: &mut WireNode, now: u64, held: &[Vec<u8>]) {
    for f in held {
        if let Some(a) = node.serve(f) {
            io.inbound(&a, Ms(now));
        }
    }
}

/// AN APP PUBLISH, P1 AND P2 ON THE WIRE: a set of k 3 + m 2 pieces whose node WITHHOLDS two piece acks (batch 7a's
/// shape: the node held 2 of 50 piece PUTs ~118 s). The SITE's PUT is framed only after k acks (P1); the publish is
/// PUBLISHED with pieces still owed (P2, the builder no longer waits for all of them); the withheld acks then make it
/// BACKED_UP (P3). THE CONTROL: before k acks, no site PUT has left.
#[test]
fn an_app_publish_sends_its_site_only_at_k_and_is_published_before_every_piece_is_acked() {
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 1_000;
    let (set, containers) = piece_set("core", 5, 3, 2);
    let withheld: Vec<ContractKey> = containers[3..].iter().map(|(c, _)| c.key()).collect();
    let keys: Vec<ContractKey> = containers.iter().map(|(c, _)| c.key()).collect();
    io.publish_app(APP, (set, containers), &starter(1), SITE_CODE, Ms(now)).expect("publishes");
    assert_eq!(stage(&io, APP).as_deref(), Some("Pieces"));
    let fate = |k: &ContractKey| keys.contains(k).then(|| if withheld.contains(k) { PieceFate::Withhold } else { PieceFate::Serve });
    let (held, site_after) = drive(&mut io, &mut node, &mut now, &fate, false, &|io| stage(io, APP).as_deref() == Some("Published"));
    assert_eq!(site_after, Some(3), "P1: the site's PUT did not leave exactly when the set reached k (it left after {site_after:?} piece acks)");
    assert_eq!(stage(&io, APP).as_deref(), Some("Published"), "P2: not PUBLISHED with two pieces withheld: {:?}", io.app_publish(APP));
    let lines = |io: &PageIo| io.app_publish(APP).map(|a| a.set_lines().into_iter().map(|l| (l.name, l.acked, l.refused, l.of, l.k)).collect::<Vec<_>>());
    assert_eq!(lines(&io), Some(vec![("core".to_string(), 3, 0, 5, 3)]), "the set's acked/refused/of/k is not what the node answered");
    assert_eq!(io.app_publish(APP).and_then(page_io::AppPublish::said), None, "a publish with nothing refused says something");
    // The withheld acks arrive: BACKED_UP (P3).
    release(&mut io, &mut node, now, &held[..2]);
    assert_eq!(stage(&io, APP).as_deref(), Some("BackedUp"), "P3: every piece acked and not BACKED_UP: {:?}", io.app_publish(APP));
    assert_eq!(io.impossible_cells(), 0);
}

/// **A SHARED piece is withdrawn only when NO publish owes it (the architect's R1).** Two apps of one build publish
/// the SAME set; two of its pieces' PUTs are LOST (the node never answers those sends); one app's publish is
/// CANCELLED -- its pieces must not be withdrawn from under the other, whose re-sends (rule 7) then land, and it
/// reaches BACKED_UP. Mutant "withdraw every pending key" -> the lost PUTs are never sent again -> red.
#[test]
fn a_cancelled_publish_never_withdraws_a_piece_another_publish_owes() {
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 1_000;
    let (set, containers) = piece_set("core", 5, 3, 2);
    let keys: Vec<ContractKey> = containers.iter().map(|(c, _)| c.key()).collect();
    let withheld: Vec<ContractKey> = keys[3..].to_vec();
    const OTHER: &str = "tasks";
    io.publish_app(APP, (set.clone(), containers.clone()), &starter(1), SITE_CODE, Ms(now)).expect("publishes");
    io.publish_app(OTHER, (set, containers), &starter(2), SITE_CODE, Ms(now)).expect("the other app publishes");
    let fate = |k: &ContractKey| keys.contains(k).then(|| if withheld.contains(k) { PieceFate::Withhold } else { PieceFate::Serve });
    let (held, _) = drive(&mut io, &mut node, &mut now, &fate, false, &|io| [APP, OTHER].iter().all(|a| stage(io, a).as_deref() == Some("Published")));
    assert_eq!((stage(&io, APP).as_deref(), stage(&io, OTHER).as_deref()), (Some("Published"), Some("Published")), "THE SETUP: both publishes were not PUBLISHED with pieces owed");
    io.cancel_app_publish(APP);
    assert_eq!(stage(&io, APP).as_deref(), Some("BackupAbandoned"));
    // The withheld sends are LOST (never answered): only a re-send can land them.
    drop(held);
    let (_, _) = drive(&mut io, &mut node, &mut now, &|k: &ContractKey| keys.contains(k).then_some(PieceFate::Serve), false, &|io| stage(io, OTHER).as_deref() == Some("BackedUp"));
    assert_eq!(stage(&io, OTHER).as_deref(), Some("BackedUp"), "R1: the other app's publish lost its shared pieces to a cancel: {:?}", io.app_publish(OTHER));
    assert_eq!(io.impossible_cells(), 0);
}

/// **THE TWO-APP MODEL (R1's gate)**: 60 seeded runs of two apps publishing ONE shared set, with a random pair of its
/// pieces' sends LOST and the first app CANCELLED at a random stage (in Pieces, in Siting, once Published, or never).
/// Served from then on, the second app is BACKED_UP in every run -- its re-sends land, because a cancel never
/// withdraws a piece another publish owes. Reach floored on each cancel stage.
#[test]
fn the_two_app_model_never_withdraws_a_shared_piece_the_other_owes() {
    const OTHER: &str = "tasks";
    let mut reach = std::collections::BTreeMap::<&str, usize>::new();
    for seed in 1..=60u64 {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = |n: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % n) as usize
        };
        let mut node = WireNode::new(&[3u8; 32]);
        let mut io = page_io(&node);
        let mut now = 1_000;
        let (set, containers) = piece_set("core", 5, 3, 2);
        let keys: Vec<ContractKey> = containers.iter().map(|(c, _)| c.key()).collect();
        let (w1, w2) = (next(5), next(5));
        let withheld: Vec<ContractKey> = [keys[w1], keys[w2]].to_vec();
        io.publish_app(APP, (set.clone(), containers.clone()), &starter(1), SITE_CODE, Ms(now)).expect("publishes");
        io.publish_app(OTHER, (set, containers), &starter(2), SITE_CODE, Ms(now)).expect("the other app publishes");
        let at = ["Pieces", "Siting", "Published", "never"][next(4)];
        let fate = |k: &ContractKey| keys.contains(k).then(|| if withheld.contains(k) { PieceFate::Withhold } else { PieceFate::Serve });
        let (held, _) = if at == "Pieces" {
            (Vec::new(), None)
        } else {
            drive(&mut io, &mut node, &mut now, &fate, at == "Siting", &|io| stage(io, APP).as_deref() == Some(if at == "never" { "Published" } else { at }))
        };
        let reached = at == "never" || stage(&io, APP).as_deref() == Some(at);
        if at != "never" && reached {
            io.cancel_app_publish(APP);
            *reach.entry(at).or_default() += 1;
        }
        drop(held);
        drive(&mut io, &mut node, &mut now, &|k: &ContractKey| keys.contains(k).then_some(PieceFate::Serve), false, &|io| stage(io, OTHER).as_deref() == Some("BackedUp"));
        assert_eq!(stage(&io, OTHER).as_deref(), Some("BackedUp"), "seed {seed} (cancel at {at}, reached {reached}): the other app's publish did not back up: {:?}", io.app_publish(OTHER));
        assert_eq!(io.impossible_cells(), 0, "seed {seed}");
    }
    println!("two-app model: cancels by stage {reach:?}");
    assert!(["Pieces", "Siting", "Published"].iter().all(|s| reach.get(s).copied().unwrap_or(0) >= 8), "the model's reach is below its floor: {reach:?}");
}

/// **A cancel of the SITE ends the publish too (the architect's R2)**: the site's PUT withheld, the Session's
/// cancel_site -- the publish is `Cancelled`, never left `Siting` for an answer that cannot come.
#[test]
fn a_cancelled_site_ends_the_app_publish() {
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 1_000;
    let (set, containers) = piece_set("core", 3, 2, 1);
    let keys: Vec<ContractKey> = containers.iter().map(|(c, _)| c.key()).collect();
    io.publish_app(APP, (set, containers), &starter(1), SITE_CODE, Ms(now)).expect("publishes");
    let fate = |k: &ContractKey| keys.contains(k).then_some(PieceFate::Serve);
    drive(&mut io, &mut node, &mut now, &fate, true, &|io| stage(io, APP).as_deref() == Some("Siting"));
    assert_eq!(stage(&io, APP).as_deref(), Some("Siting"), "THE SETUP: the publish is not waiting on its site");
    io.cancel_site(APP);
    drive(&mut io, &mut node, &mut now, &fate, true, &|io| stage(io, APP).as_deref() != Some("Siting"));
    assert_eq!(stage(&io, APP).as_deref(), Some("Cancelled"), "R2: a cancelled site left the publish {:?}", io.app_publish(APP));
}

/// **PUBLISHED with a piece FINALLY refused after k SAYS that BACKED_UP will not come (the architect's R4)**: its set
/// line counts the refusal, and its words name it; the other pieces still ack. THE CONTROL: the same publish with
/// nothing refused says nothing (the first test).
#[test]
fn a_refusal_after_k_says_backed_up_will_not_come() {
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 1_000;
    let (set, containers) = piece_set("core", 5, 3, 2);
    let keys: Vec<ContractKey> = containers.iter().map(|(c, _)| c.key()).collect();
    let last = keys[4];
    io.publish_app(APP, (set, containers), &starter(1), SITE_CODE, Ms(now)).expect("publishes");
    let fate = |k: &ContractKey| keys.contains(k).then(|| if *k == last { PieceFate::Refuse } else { PieceFate::Serve });
    drive(&mut io, &mut node, &mut now, &fate, false, &|io| stage(io, APP).as_deref() == Some("Published") && io.app_publish(APP).is_some_and(|a| a.set_lines().iter().any(|l| l.acked == 4 && l.refused == 1)));
    let a = io.app_publish(APP).expect("a publish");
    assert_eq!(stage(&io, APP).as_deref(), Some("Published"), "a refusal after k ended the publish: {a:?}");
    assert_eq!(a.set_lines().into_iter().map(|l| (l.acked, l.refused, l.of)).collect::<Vec<_>>(), vec![(4, 1, 5)]);
    assert!(a.said().is_some_and(|w| w.contains("BACKED_UP will not come") && w.contains("core")), "R4: a PUBLISHED publish with a refused piece does not say BACKED_UP won't come: {:?}", a.said());
}

/// **A publish with NO piece set is refused by name** (Codex on #527, A): with nothing to PUT, no ack could ever send
/// its site, and it would sit at `pieces` for ever, blocking every later publish of the app.
#[test]
fn a_publish_with_no_piece_set_is_refused() {
    let node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let (mut set, _) = piece_set("core", 3, 2, 1);
    set.pieces.clear();
    let e = io.publish_app(APP, (set, Vec::new()), &starter(1), SITE_CODE, Ms(1)).expect_err("a publish with no piece was taken");
    assert!(e.contains("names no piece"), "{e}");
    assert!(io.app_publish(APP).is_none(), "a refused publish left a state behind");
}

/// **While an app publish is in flight, the site's DIRECT door refuses** (Codex on #527, B): a `publish_site` through
/// the old door would replace the publication the machine is waiting on, and the machine -- matching by app -- would
/// report ITS publish at the other one's version. The machine's own SendSite is not that door.
#[test]
fn a_direct_site_publish_is_refused_while_an_app_publish_is_in_flight() {
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 1_000;
    let (set, containers) = piece_set("core", 3, 2, 1);
    let keys: Vec<ContractKey> = containers.iter().map(|(c, _)| c.key()).collect();
    io.publish_app(APP, (set, containers), &starter(1), SITE_CODE, Ms(now)).expect("publishes");
    let e = io.publish_site(APP, SITE_CODE, web(7), Ms(now)).expect_err("a direct publish_site was taken while the app publish was in Pieces");
    assert!(e.contains("being published"), "{e}");
    let fate = |k: &ContractKey| keys.contains(k).then_some(PieceFate::Serve);
    drive(&mut io, &mut node, &mut now, &fate, true, &|io| stage(io, APP).as_deref() == Some("Siting"));
    assert_eq!(stage(&io, APP).as_deref(), Some("Siting"), "THE SETUP: the publish is not waiting on its site");
    assert!(io.publish_site(APP, SITE_CODE, web(7), Ms(now)).is_err(), "a direct publish_site replaced the site the machine waits on");
    drive(&mut io, &mut node, &mut now, &fate, false, &|io| stage(io, APP).as_deref() == Some("BackedUp"));
    assert_eq!(stage(&io, APP).as_deref(), Some("BackedUp"));
    assert_eq!(node.site().map(|(_, value, _)| value), Some(blake3::hash(&site_web(&node, APP, 1)).as_bytes().to_vec()), "the live site is not the app publish's own");
}

/// **A REPUBLISH does not leave the last publish's owed pieces retrying for ever** (Codex on #527, C): v1 is PUBLISHED
/// with a piece whose send was LOST; v2 names other pieces. v1's owed piece, needed by no publish now, is withdrawn:
/// it is never sent again. Mutant "a replaced publish's owed pieces are left alone" -> it is re-sent on its RTO -> red.
#[test]
fn a_republish_withdraws_the_last_publishs_pieces_nobody_owes() {
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 1_000;
    let (set1, c1) = piece_set("v1", 3, 2, 1);
    let lost = c1[2].0.key();
    let k1: Vec<ContractKey> = c1.iter().map(|(c, _)| c.key()).collect();
    io.publish_app(APP, (set1, c1), &starter(1), SITE_CODE, Ms(now)).expect("v1 publishes");
    let fate1 = |k: &ContractKey| k1.contains(k).then(|| if *k == lost { PieceFate::Withhold } else { PieceFate::Serve });
    let (held, _) = drive(&mut io, &mut node, &mut now, &fate1, false, &|io| stage(io, APP).as_deref() == Some("Published"));
    assert_eq!(stage(&io, APP).as_deref(), Some("Published"), "THE SETUP: v1 is not PUBLISHED with a piece owed");
    assert!(!held.is_empty(), "THE SETUP: v1's piece was never sent");
    let (set2, c2) = piece_set("v2", 3, 2, 1);
    let k2: Vec<ContractKey> = c2.iter().map(|(c, _)| c.key()).collect();
    io.publish_app(APP, (set2, c2), &starter(2), SITE_CODE, Ms(now)).expect("v2 publishes");
    let resent = std::cell::Cell::new(0usize);
    let fate2 = |k: &ContractKey| {
        if *k == lost {
            resent.set(resent.get() + 1);
            return Some(PieceFate::Withhold);
        }
        k2.contains(k).then_some(PieceFate::Serve)
    };
    drive(&mut io, &mut node, &mut now, &fate2, false, &|io| stage(io, APP).as_deref() == Some("BackedUp"));
    assert_eq!(stage(&io, APP).as_deref(), Some("BackedUp"), "THE SETUP: v2 did not back up");
    now += 600_000;
    drive(&mut io, &mut node, &mut now, &fate2, false, &|_| false);
    assert_eq!(resent.get(), 0, "v1's owed piece, needed by nobody, kept being re-sent after the republish");
}

/// **A publish names each piece ONCE**: a set naming an address another piece of the publish names is refused by
/// name (an answer is marked in one set only, so the other would never be).
#[test]
fn a_publish_naming_a_piece_twice_is_refused() {
    let node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let (mut a, mut ca) = piece_set("core", 3, 2, 1);
    a.pieces[1] = a.pieces[0].clone();
    ca[1] = ca[0].clone();
    let e = io.publish_app(APP, (a, ca), &starter(1), SITE_CODE, Ms(1)).expect_err("a duplicate address was taken");
    assert!(e.contains("another piece of this publish names too"), "{e}");
}

/// Is frame `f` a GET of `key`?
fn is_get_of(f: &[u8], key: &ContractKey) -> bool {
    matches!(bincode::deserialize::<ClientRequest>(f), Ok(ClientRequest::ContractOp(ContractRequest::Get { key: k, .. })) if k == *key.id())
}

/// A READ ANSWERED TOO LARGE ENDS, NAMED (rule 8; engineer1 on the builder's 5.9 MB site, whose read-back re-asked for
/// ever as "site craftworks-builder's read"): a site's read-back answered with a state bigger than this page reads
/// (one frame over `wire::MAX_FRAME`) IS an answer -- the publication ends REFUSED naming the size, and the read is
/// not asked again.
#[test]
fn a_site_read_back_answered_too_large_ends_named_and_is_not_asked_again() {
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 1_000;
    let site = page_io::site_contract(SITE_CODE, &node.register_params, APP).expect("site").key();
    let big = ok(HostResponse::ContractResponse(ContractResponse::GetResponse { key: site, contract: None, state: WrappedState::new(vec![1u8; wire::MAX_FRAME + 1]) }));
    io.publish_site(APP, SITE_CODE, web(1), Ms(now)).expect("publishes");
    let (mut site_gets, mut after_end) = (0, 0);
    for _ in 0..400 {
        let frames = io.take_frames();
        if frames.is_empty() {
            match io.next_due() {
                Some(Ms(t)) => {
                    now = now.max(t);
                    io.tick(Ms(now));
                    continue;
                }
                None => break,
            }
        }
        now += 1;
        let ended = matches!(io.publication(APP), Some(page::Publication::Refused(_)));
        for f in frames {
            if is_get_of(&f, &site) {
                site_gets += 1;
                after_end += usize::from(ended);
                io.inbound(&big, Ms(now));
            } else if let Some(a) = node.serve(&f) {
                io.inbound(&a, Ms(now));
            }
        }
    }
    assert!(site_gets > 0, "THE SETUP: the site was never read back");
    match io.publication(APP) {
        Some(page::Publication::Refused(w)) => assert!(w.contains("bytes") && w.contains("read"), "the refusal does not name the size: {w}"),
        other => panic!("a too-large read-back did not end the publication: {other:?}, {site_gets} site reads"),
    }
    assert_eq!(after_end, 0, "the site was read again after its read-back ended");
}

/// **A SITE THAT COULD NOT BE READ BACK IS REFUSED BY NAME** (as a record too large to send is, src/writes.rs): a web
/// over `wire::MAX_SITE_WEB` is refused before anything is sent -- by the direct door and by an app publish -- naming
/// its size; its publication would wait for a read-back no page could read. THE CONTROL: a site at the bound's
/// order of magnitude below it is taken.
#[test]
fn a_site_too_large_to_read_back_is_refused_before_it_is_sent() {
    let node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let big = vec![5u8; wire::MAX_SITE_WEB + 1];
    let e = io.publish_site(APP, SITE_CODE, big.clone(), Ms(1)).expect_err("a site too large to read back was taken");
    assert!(e.contains(&big.len().to_string()) && e.contains("read back"), "the refusal does not name the size: {e}");
    let (set, containers) = piece_set("core", 3, 2, 1);
    let e = io.publish_app(APP, (set, containers), &[("index.html".to_string(), big)], SITE_CODE, Ms(1)).expect_err("an app publish of a site too large to read back was taken");
    assert!(e.contains("read back"), "{e}");
    assert!(io.take_frames().is_empty(), "a frame left for a site that was refused");
    assert!(io.publication(APP).is_none() && io.app_publish(APP).is_none(), "a refused site left a publication behind");
    io.publish_site(APP, SITE_CODE, vec![5u8; 6_000_000], Ms(1)).expect("THE CONTROL: the builder's 6 MB site is taken");
}

/// **A SITE IS THE STARTER AND THE POINTER, NOTHING PER VERSION** (ARCHITECTURE §19's bootstrap; app-as-data P4): the
/// live site's web is exactly the build's starter plus `pointer.json` -- `{"app","register_params"}`, composed by
/// page-io from THIS session's register, the loader's way to the tree -- and a starter that brings its own pointer
/// or an `app.json` (an app version in a site) is refused by name, before anything is PUT.
#[test]
fn a_site_is_the_starter_and_the_pointer_to_this_register() {
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 1_000;
    let (set, containers) = piece_set("core", 3, 2, 1);
    let keys: Vec<ContractKey> = containers.iter().map(|(c, _)| c.key()).collect();
    for extra in [wire::webapp::POINTER_PATH, wire::webapp::LEGACY_APP_JSON] {
        let mut st = starter(1);
        st.push((extra.to_string(), b"{}".to_vec()));
        let e = io.publish_app(APP, (set.clone(), containers.clone()), &st, SITE_CODE, Ms(now)).expect_err("a starter with its own file was taken");
        assert!(e.contains(extra), "the refusal does not name `{extra}`: {e}");
        assert!(io.app_publish(APP).is_none(), "a refused publish left a state behind");
    }
    io.publish_app(APP, (set, containers), &starter(1), SITE_CODE, Ms(now)).expect("publishes");
    let fate = |k: &ContractKey| keys.contains(k).then_some(PieceFate::Serve);
    drive(&mut io, &mut node, &mut now, &fate, false, &|io| stage(io, APP).as_deref() == Some("BackedUp"));
    let (_, _, web) = node.site().expect("the site is live");
    assert_eq!(web, site_web(&node, APP, 1), "the live site is not exactly the starter and this register's pointer");
    let pointer = String::from_utf8(wire::webapp::site_pointer(&node.register_params, APP).unwrap()).unwrap();
    assert_eq!(pointer, format!(r#"{{"app":"{APP}","register_params":"{}"}}"#, core_types::hex::encode(&node.register_params)));
}

/// **A PLATFORM UPGRADE writes the SAME pointer** (the Q3 ruling): a second publish of the app -- a new starter, the
/// builder's decision -- goes to the same link with a byte-identical pointer; only the starter differs.
#[test]
fn an_upgrade_keeps_the_link_and_the_pointer_byte_for_byte() {
    let mut node = WireNode::new(&[3u8; 32]);
    let mut io = page_io(&node);
    let mut now = 1_000;
    let mut webs = Vec::new();
    for build in [1u8, 2] {
        let (set, containers) = piece_set(&format!("build{build}"), 3, 2, 1);
        let keys: Vec<ContractKey> = containers.iter().map(|(c, _)| c.key()).collect();
        io.publish_app(APP, (set, containers), &starter(build), SITE_CODE, Ms(now)).expect("publishes");
        let fate = |k: &ContractKey| keys.contains(k).then_some(PieceFate::Serve);
        drive(&mut io, &mut node, &mut now, &fate, false, &|io| stage(io, APP).as_deref() == Some("BackedUp"));
        webs.push(node.site().expect("live").2);
    }
    assert_eq!(webs, vec![site_web(&node, APP, 1), site_web(&node, APP, 2)], "an upgrade's site is not its starter and the SAME pointer");
}

/// Every row of `io`'s tree, read through the normal read: a full range, page by page (the engine's scan).
fn read_all(io: &mut PageIo, node: &mut WireNode, now: &mut u64, base: u64) -> usize {
    let (mut rows, mut after, mut n) = (0, None, 0u64);
    loop {
        n += 1;
        let range = Request::Range { req_id: base + n, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: after.clone(), max_entries: 500 };
        let r = client(io, node, now, &range);
        let Some((entries, cursor)) = r.iter().find_map(|x| if let Reply::Page { req_id, entries, cursor, .. } = x { (*req_id == base + n).then(|| (entries.len(), cursor.clone())) } else { None }) else {
            panic!("page {n} of the scan was never answered: replies {r:?}; unusable {:?}; repair {:?}", io.unusable(), io.server.page.repair_report());
        };
        rows += entries;
        match cursor {
            Some(c) => after = Some(c),
            None => return rows,
        }
        assert!(n < 1_000, "the scan did not end");
    }
}

/// **REPAIR IS THE READ, OVER THE WHOLE TREE** (sdk#479; the owner via core dev). The node LOSES blocks of a published
/// tree -- tree nodes, values and parity alike, every 7th block of its CURRENT tree; a COLD reader reads every row through the normal
/// read, which races each block's group, rebuilds a lost member from any `k` and re-encodes a lost parity with the
/// save's own code, and PUTs them back. Then the node holds every lost block again, byte for byte, and a FRESH reader
/// reads the whole tree from the node without one NotFound. THE CONTROL: nothing lost, nothing found missing, nothing
/// put back.
#[test]
fn a_whole_tree_read_puts_back_what_the_node_lost_and_a_fresh_reader_then_finds_nothing_missing() {
    let rows = |lo: u32, hi: u32| -> Vec<protocol::Op> { (lo..hi).map(|i| protocol::Op::Put(format!("r/{i:05}").into_bytes(), vec![i as u8; 90])).collect() };
    for lose in [true, false] {
        let mut node = WireNode::new(&[41u8; 32]);
        let mut now = 1_000;
        let mut w = page_io(&node);
        client(&mut w, &mut node, &mut now, &Request::Identity);
        for (n, lo) in (0..2_000u32).step_by(500).enumerate() {
            assert!(states(&client(&mut w, &mut node, &mut now, &Request::forced_write(n as u64 + 1, rows(lo, lo + 500))), n as u64 + 1).contains(&WriteState::Published), "THE SETUP: rows {lo}.. did not publish");
        }
        // The CURRENT tree's blocks (the node also holds superseded heads' blocks, which no read of this tree asks):
        // walked from the head's root in the writer's store -- every node, its groups' members and their parity.
        let (_, root) = node.head().expect("a head");
        let mut tree: std::collections::BTreeSet<freenet_prolly::Cid> = std::collections::BTreeSet::from([root]);
        let mut at = vec![root];
        while let Some(id) = at.pop() {
            let bytes = freenet_prolly::store::Blocks::get(w.server.page.blocks(), &id).expect("the writer holds its tree").to_vec();
            let n = freenet_prolly::node::Node::parse(&bytes).expect("a node");
            tree.extend(n.parity());
            for (_, members) in freenet_prolly::parity::group_members(&n) {
                tree.extend(members.iter().copied());
            }
            if !n.is_leaf() {
                at.extend((0..n.len()).map(|i| n.child(i).0));
            }
        }
        let blocks: Vec<[u8; 32]> = tree.iter().map(|c| wire::block::contract_for(BLOCK_CODE, c)).filter(|id| node.contracts.contains_key(id)).collect();
        let lost: BTreeMap<[u8; 32], Vec<u8>> = if lose { blocks.iter().step_by(7).map(|id| (*id, node.contracts[id].clone())).collect() } else { BTreeMap::new() };
        for id in lost.keys() {
            node.contracts.remove(id);
        }
        // REPAIR: a cold reader reads the whole tree.
        let mut r = reader_with(&node, engine::Params::default());
        r.server.page.set_repair_pass(true, true);
        client(&mut r, &mut node, &mut now, &Request::Identity);
        assert_eq!(read_all(&mut r, &mut node, &mut now, 100), 2_000, "the repairing read did not read every row");
        let report = r.server.page.repair_report();
        let back = lost.iter().filter(|(id, st)| node.contracts.get(*id) == Some(*st)).count();
        println!("lose={lose}: {} of {} blocks lost; the read found {} missing, put back {} (acked; {} race rebuilds of held blocks apart), rejected {}, gave up {}, parity mismatched {}; the node holds {back} of the lost again", lost.len(), blocks.len(), report.missing, report.put_back, report.reput, report.rejected, report.given_up, report.parity_mismatched);
        assert_eq!(back, lost.len(), "the node does not hold every lost block again, byte for byte: {back} of {}", lost.len());
        assert_eq!((report.given_up, report.rejected, report.parity_mismatched), (0, 0, 0), "a repair failed: {:?}", report.why);
        assert_eq!((report.missing, report.put_back), (lost.len() as u64, lost.len() as u64), "the report does not count exactly what was lost and put back: {report:?}");
        // A FRESH reader reads it all from the node, and nothing is missing.
        let mut f = reader_with(&node, engine::Params::default());
        client(&mut f, &mut node, &mut now, &Request::Identity);
        assert_eq!(read_all(&mut f, &mut node, &mut now, 200), 2_000, "the fresh reader did not read every row");
        assert_eq!(f.server.page.repair_report().missing, 0, "the fresh reader still found blocks missing on the node");
        if !lose {
            assert_eq!((report.missing, report.put_back), (0, 0), "THE CONTROL: a whole tree was found missing or put back");
        }
    }
}

/// **A CHECK PUTS NOTHING, AND COUNTS WHAT IS LOST** (sdk#479; the tab shows DEGRADED before "Repair now"). The node
/// loses every 7th block of a published tree's current blocks; a cold reader in CHECK mode (`put_back` false) reads
/// every row and counts each lost block missing, and the node receives ZERO block PUTs -- the lost blocks are still
/// lost after it. Then a REPAIR reader puts every one back. (The control that the check really suppresses the puts is
/// the mutant "the engine ignores put_back", which turns this red.)
#[test]
fn a_check_reads_the_whole_tree_counts_every_lost_block_and_puts_nothing_then_a_repair_puts_them_back() {
    let rows = |lo: u32, hi: u32| -> Vec<protocol::Op> { (lo..hi).map(|i| protocol::Op::Put(format!("r/{i:05}").into_bytes(), vec![i as u8; 90])).collect() };
    let mut node = WireNode::new(&[43u8; 32]);
    let mut now = 1_000;
    let mut w = page_io(&node);
    client(&mut w, &mut node, &mut now, &Request::Identity);
    for (n, lo) in (0..2_000u32).step_by(500).enumerate() {
        assert!(states(&client(&mut w, &mut node, &mut now, &Request::forced_write(n as u64 + 1, rows(lo, lo + 500))), n as u64 + 1).contains(&WriteState::Published), "THE SETUP: rows {lo}.. did not publish");
    }
    let (_, root) = node.head().expect("a head");
    let mut tree: std::collections::BTreeSet<freenet_prolly::Cid> = std::collections::BTreeSet::from([root]);
    let mut at = vec![root];
    while let Some(id) = at.pop() {
        let bytes = freenet_prolly::store::Blocks::get(w.server.page.blocks(), &id).expect("the writer holds its tree").to_vec();
        let n = freenet_prolly::node::Node::parse(&bytes).expect("a node");
        tree.extend(n.parity());
        for (_, members) in freenet_prolly::parity::group_members(&n) {
            tree.extend(members.iter().copied());
        }
        if !n.is_leaf() {
            at.extend((0..n.len()).map(|i| n.child(i).0));
        }
    }
    let blocks: Vec<[u8; 32]> = tree.iter().map(|c| wire::block::contract_for(BLOCK_CODE, c)).filter(|id| node.contracts.contains_key(id)).collect();
    let lost: Vec<[u8; 32]> = blocks.iter().step_by(7).copied().collect();
    for id in &lost {
        node.contracts.remove(id);
    }
    // THE CHECK.
    let puts_before = node.served.get("put block").copied().unwrap_or(0);
    let mut c = reader_with(&node, engine::Params::default());
    c.server.page.set_repair_pass(true, false);
    client(&mut c, &mut node, &mut now, &Request::Identity);
    assert_eq!(read_all(&mut c, &mut node, &mut now, 300), 2_000, "the check did not read every row");
    let check = c.server.page.repair_report();
    let puts = node.served.get("put block").copied().unwrap_or(0) - puts_before;
    println!("check: {} of {} current-tree blocks lost; missing {}, put back {}, given up {}; block PUTs on the wire {puts}", lost.len(), blocks.len(), check.missing, check.put_back, check.given_up);
    assert_eq!(puts, 0, "a CHECK sent {puts} block PUTs");
    assert_eq!(check.missing, lost.len() as u64, "the check did not count exactly the lost blocks: {check:?}");
    assert_eq!((check.put_back, check.given_up), (0, 0), "the check put back or gave up: {check:?}");
    assert!(lost.iter().all(|id| !node.contracts.contains_key(id)), "a lost block is back after a CHECK");
    // THE REPAIR afterwards.
    let mut r = reader_with(&node, engine::Params::default());
    r.server.page.set_repair_pass(true, true);
    client(&mut r, &mut node, &mut now, &Request::Identity);
    assert_eq!(read_all(&mut r, &mut node, &mut now, 400), 2_000, "the repair did not read every row");
    let rep = r.server.page.repair_report();
    println!("repair after: missing {}, put back {}", rep.missing, rep.put_back);
    assert_eq!((rep.missing, rep.put_back, rep.given_up), (lost.len() as u64, lost.len() as u64, 0), "the repair after the check: {rep:?}");
    assert!(lost.iter().all(|id| node.contracts.contains_key(id)), "a lost block is not back after the repair");
}
