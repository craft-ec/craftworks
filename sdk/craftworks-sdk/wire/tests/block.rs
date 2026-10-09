//! A block as the network holds it, named the same way by the page and the
//! delegate; and the two client-API answers the page reads (ENGINE-SHAPE §2, §6).

use freenet_stdlib::client_api::{ContractResponse, HostResponse};
use freenet_stdlib::prelude::*;
use wire::block::{block_contract, block_of_state, block_state, contract_deriver, contract_for};
use wire::{unframe, AckKind, Incoming, Reassembler};

const CODE: &[u8] = b"not a real contract: the derivation hashes whatever it is given";

fn cid(n: u8) -> [u8; 32] {
    [n; 32]
}

/// ONE derivation: the page's copy and the shell's must name the same
/// contract for the same block, or a page-put block is a block the delegate
/// cannot find. Pinned until the shell's copy is deleted.
#[test]
fn the_page_names_a_blocks_contract_as_the_delegate_does() {
    for n in [0u8, 1, 7, 255] {
        assert_eq!(contract_for(CODE, &cid(n)), contract_keys::block::contract_for(CODE, &cid(n)));
        assert_eq!(contract_deriver(CODE)(&cid(n)), contract_for(CODE, &cid(n)), "the fast deriver drifted from the node's");
    }
}

/// The kind is RECOVERED from the id — exact, because the id hashes the kind
/// byte — for every kind a block can be; and bytes no kind hashes to are
/// refused rather than put under a guessed one.
#[test]
fn a_blocks_state_is_its_kind_and_body_and_its_inverse_names_the_same_block() {
    let body = b"some bytes".to_vec();
    for k in [freenet_prolly::kind::TREE_NODE, freenet_prolly::kind::RAW, freenet_prolly::kind::PARITY, 6u8] {
        let id = freenet_prolly::block_id(k, &body);
        let state = block_state(&id, &body).expect("the id hashes these bytes under one kind");
        assert_eq!(state[0], k);
        let (back, b) = block_of_state(&state).expect("a state of a kind byte and a body");
        assert_eq!((back, b), (id, body.as_slice()));
    }
    assert_eq!(block_state(&cid(9), &body), None, "bytes that hash to no kind were given one");
    assert_eq!(block_of_state(&[]), None);
}

/// A GET answer is `Got` with the contract's instance id and its state — the
/// default build used to file it as Unusable(UnknownKind).
#[test]
fn a_get_answer_is_got_with_its_instance_id_and_state() {
    let c = block_contract(CODE, &cid(3));
    let key = c.key();
    let reply: Result<HostResponse, wire::_test::Err> = Ok(HostResponse::ContractResponse(ContractResponse::GetResponse {
        key,
        contract: None,
        state: WrappedState::new(b"kind and body".to_vec()),
    }));
    let bytes = bincode::serialize(&reply).expect("encodes");
    let mut id = [0u8; 32];
    id.copy_from_slice(&key.id().as_bytes()[..32]);
    assert_eq!(unframe(&mut Reassembler::new(), &bytes), Incoming::Got { id, state: b"kind and body".to_vec() });
}

/// An UPDATE answer is an Ack that NAMES the contract and says nothing about
/// which record the register kept (F56).
#[test]
fn an_update_answer_is_an_ack_naming_the_contract() {
    let key = block_contract(CODE, &cid(4)).key();
    let reply: Result<HostResponse, wire::_test::Err> = Ok(HostResponse::ContractResponse(ContractResponse::UpdateResponse {
        key,
        summary: StateSummary::from(Vec::new()),
    }));
    let bytes = bincode::serialize(&reply).expect("encodes");
    assert_eq!(unframe(&mut Reassembler::new(), &bytes), Incoming::Ack(AckKind::Updated(key.to_string())));
}

/// GET and UPDATE frame to the requests they name, whole.
#[test]
fn get_and_update_frame_to_the_requests_they_name() {
    use freenet_stdlib::client_api::{ClientRequest, ContractRequest};
    let c = block_contract(CODE, &cid(5));
    let id = *c.key().id();
    let frames = wire::frame_get(id, true, 1).expect("frames");
    assert_eq!(frames.len(), 1, "a GET is small: one frame");
    match bincode::deserialize::<ClientRequest>(&frames[0]).expect("decodes") {
        ClientRequest::ContractOp(ContractRequest::Get { key, return_contract_code, subscribe, .. }) => {
            assert_eq!((key, return_contract_code, subscribe), (id, false, true));
        }
        other => panic!("{other:?}"),
    }
    let frames = wire::frame_update(c.key(), b"record".to_vec(), 2).expect("frames");
    match bincode::deserialize::<ClientRequest>(&frames[0]).expect("decodes") {
        ClientRequest::ContractOp(ContractRequest::Update { key, data: UpdateData::State(s) }) => {
            assert_eq!((key, s.as_ref()), (c.key(), b"record".as_slice()));
        }
        other => panic!("{other:?}"),
    }
    // A DELTA goes as `UpdateData::Delta`: a node checks an incoming `State` as a whole state and refuses a delta sent
    // as one ("invalid put").
    let frames = wire::frame_update_delta(c.key(), b"step".to_vec(), 3).expect("frames");
    match bincode::deserialize::<ClientRequest>(&frames[0]).expect("decodes") {
        ClientRequest::ContractOp(ContractRequest::Update { key, data: UpdateData::Delta(d) }) => {
            assert_eq!((key, d.as_ref()), (c.key(), b"step".as_slice()));
        }
        other => panic!("{other:?}"),
    }
}

/// A LOCAL node's "missing contract" (`freenet local` answers the GET of a contract it never stored with
/// `ContractError::MissingContract`) is the explicit NotFound, with the contract's id -- the fact a network node's
/// `ContractResponse::NotFound` states. Control: a GET refusal of the same contract stays `Refused`.
#[test]
fn a_local_nodes_missing_contract_is_not_found_with_its_id() {
    use freenet_stdlib::client_api::{ClientError, ContractError, ErrorKind, RequestError};
    let key = block_contract(CODE, &cid(7)).key();
    let missing: ClientError = ErrorKind::RequestError(RequestError::ContractError(ContractError::MissingContract { key: *key.id() })).into();
    let bytes = bincode::serialize(&Err::<HostResponse, ClientError>(missing)).expect("encodes");
    let mut id = [0u8; 32];
    id.copy_from_slice(&key.id().as_bytes()[..32]);
    assert_eq!(unframe(&mut Reassembler::new(), &bytes), Incoming::GetFailed { id, why: wire::GetFail::NotFound });
    let refused: ClientError = ErrorKind::RequestError(RequestError::ContractError(ContractError::Get { key, cause: "busy".into() })).into();
    let bytes = bincode::serialize(&Err::<HostResponse, ClientError>(refused)).expect("encodes");
    assert_eq!(unframe(&mut Reassembler::new(), &bytes), Incoming::GetFailed { id, why: wire::GetFail::Refused("busy".into()) });
}

/// A GET the node refuses, NAMING the contract, is `GetFailed` with its
/// instance id — how a cold read learns its root is this node's own (F55). A
/// refusal that names nothing is still `Refused`.
#[test]
fn a_get_refusal_naming_its_contract_is_get_failed_with_its_id() {
    use freenet_stdlib::client_api::{ClientError, ContractError, ErrorKind, RequestError};
    let key = block_contract(CODE, &cid(6)).key();
    let named: ClientError = ErrorKind::RequestError(RequestError::ContractError(ContractError::Get {
        key,
        cause: "not found".into(),
    }))
    .into();
    let bytes = bincode::serialize(&Err::<HostResponse, ClientError>(named)).expect("encodes");
    let mut id = [0u8; 32];
    id.copy_from_slice(&key.id().as_bytes()[..32]);
    assert_eq!(unframe(&mut Reassembler::new(), &bytes), Incoming::GetFailed { id, why: wire::GetFail::Refused("not found".into()) });
    let bare = bincode::serialize(&Err::<HostResponse, ClientError>(wire::_test::client_error("no"))).expect("encodes");
    assert!(matches!(unframe(&mut Reassembler::new(), &bare), Incoming::Refused(_)));
}

/// 0.2.136's explicit NotFound for a GET is `GetFailed` naming the contract —
/// and SAYS it is NotFound: only it means absent; a refusal (above) is told
/// apart by its `why` (#332 review).
#[test]
fn a_not_found_answer_is_a_get_failed_naming_the_contract() {
    let key = block_contract(CODE, &cid(9)).key();
    let reply: Result<HostResponse, wire::_test::Err> =
        Ok(HostResponse::ContractResponse(ContractResponse::NotFound { instance_id: *key.id() }));
    let bytes = bincode::serialize(&reply).expect("encodes");
    let mut id = [0u8; 32];
    id.copy_from_slice(&key.id().as_bytes()[..32]);
    assert_eq!(unframe(&mut Reassembler::new(), &bytes), Incoming::GetFailed { id, why: wire::GetFail::NotFound });
}
