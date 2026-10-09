//! A READ OVER 4 MiB IS READ (the builder's 5.9 MB site, engineer1 from the page's own lines): the node chunks a large
//! answer (stdlib's `chunk_response`, its own chunker), the page reassembles it, and it is the answer -- not
//! `TooLarge`, discarded and re-asked for ever.

use freenet_stdlib::client_api::streaming::chunk_response;
use freenet_stdlib::client_api::{ClientError, ContractResponse, HostResponse};
use freenet_stdlib::prelude::WrappedState;
use wire::{unframe, GetFail, Incoming, Reassembler};

/// `state` answered to a GET of a site contract, framed as the node frames it: chunked, each chunk its own frame.
fn chunked_get(state: &[u8]) -> ([u8; 32], Vec<Vec<u8>>) {
    let (_, contract, _) = wire::puts::contract(b"site code", b"site params", &[]);
    let key = contract.key();
    let whole = bincode::serialize(&Ok::<HostResponse, ClientError>(HostResponse::ContractResponse(ContractResponse::GetResponse {
        key,
        contract: None,
        state: WrappedState::new(state.to_vec()),
    })))
    .unwrap();
    let frames = chunk_response(whole, 1).into_iter().map(|c| bincode::serialize(&Ok::<HostResponse, ClientError>(c)).unwrap()).collect();
    (key.id().as_bytes().try_into().expect("32 bytes"), frames)
}

#[test]
fn a_site_read_over_four_mib_arrives_as_its_state() {
    let state: Vec<u8> = (0..5_900_000u32).map(|i| (i % 251) as u8).collect();
    let (id, frames) = chunked_get(&state);
    assert!(frames.len() > 1, "THE SETUP: the answer was not chunked");
    let mut r = Reassembler::new();
    let mut last = None;
    for f in &frames {
        last = Some(unframe(&mut r, f));
    }
    match last {
        Some(Incoming::Got { id: got, state: s }) => assert!(got == id && s == state, "the site read arrived as another answer"),
        other => panic!("a site read of 5.9 MB was not read: the last frame came out as {other:?}"),
    }
}

fn get_answer(state: Vec<u8>) -> ([u8; 32], Vec<u8>) {
    let (_, contract, _) = wire::puts::contract(b"site code", b"site params", &[]);
    let key = contract.key();
    let bytes = bincode::serialize(&Ok::<HostResponse, ClientError>(HostResponse::ContractResponse(ContractResponse::GetResponse { key, contract: None, state: WrappedState::new(state) }))).unwrap();
    (key.id().as_bytes().try_into().unwrap(), bytes)
}

/// THE KEY FROM THE HEAD (`get_key_prefix`): a real GET answer names its contract, its state never decoded -- and
/// nothing else names one: a PUT's answer, a message cut short, zeros, garbage. Pins the mirror types against the
/// stdlib's own encoding.
#[test]
fn a_get_answer_names_its_contract_from_its_head() {
    let (id, bytes) = get_answer(vec![9u8; 100_000]);
    assert_eq!(wire::get_key_prefix(&bytes), Some(id));
    assert_eq!(wire::get_key_prefix(&bytes[..200]), None, "a message cut short named a GET (its state does not fill it)");
    assert_eq!(wire::get_key_prefix(&vec![0u8; 5_000_000]), None, "a message of zeros named a GET");
    let (_, contract, _) = wire::puts::contract(b"site code", b"site params", &[]);
    let put = bincode::serialize(&Ok::<HostResponse, ClientError>(HostResponse::ContractResponse(ContractResponse::PutResponse { key: contract.key() }))).unwrap();
    assert_eq!(wire::get_key_prefix(&put), None, "a PUT's answer was taken for a GET's");
    assert_eq!(wire::get_key_prefix(b"garbage"), None);
}

// The bounds' arithmetic, checked when this compiles: the read state and its envelope ARE the one bound, a site's
// web fits inside the read state, and the bound covers freenet's 50 MiB MAX_STATE_SIZE.
const _: () = assert!(wire::MAX_READ_STATE + wire::READ_ENVELOPE == wire::MAX_REASSEMBLED);
const _: () = assert!(wire::MAX_SITE_WEB < wire::MAX_READ_STATE && wire::MAX_REASSEMBLED >= 50 * 1024 * 1024);

/// THE ONE BOUND'S ENVELOPE: a GET answer's encoding adds at most `READ_ENVELOPE` to its state, so a state of
/// `MAX_READ_STATE` is a message of at most `MAX_REASSEMBLED` -- one this page reads.
#[test]
fn a_state_of_max_read_state_is_a_message_this_page_reads() {
    let (_, bytes) = get_answer(vec![0u8; 4096]);
    let envelope = bytes.len() - 4096;
    assert!(envelope <= wire::READ_ENVELOPE, "a GET answer's envelope is {envelope} bytes, over READ_ENVELOPE");
}

/// A GET ANSWERED TOO LARGE IS NAMED (rule 8): one frame over `MAX_FRAME` (a node chunks anything larger, so this is
/// a node that did not) is `GetFailed { TooLarge }` for the GET its head names -- an answer the page can end, never a
/// silence.
#[test]
fn a_get_answered_in_one_frame_too_large_is_named_for_its_contract() {
    let (id, bytes) = get_answer(vec![3u8; wire::MAX_FRAME + 1]);
    match unframe(&mut Reassembler::new(), &bytes) {
        Incoming::GetFailed { id: got, why: GetFail::TooLarge { bytes: n } } => assert_eq!((got, n), (id, bytes.len())),
        other => panic!("a too-large GET answer came out as {other:?}"),
    }
}
