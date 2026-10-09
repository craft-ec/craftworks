//! A PUT of a contract the app names (builder#104): its key as the node names
//! it, the node's answers matched to it BY THAT KEY, and nothing else's.

use freenet_stdlib::client_api::{ClientError, ContractError, ContractResponse, ErrorKind, HostResponse, RequestError};
use wire::puts::contract;
use wire::{unframe, AckKind, Incoming, Reassembler};

const CODE: &[u8] = b"any code: the key hashes whatever it is given";

fn put_ack(key: freenet_stdlib::prelude::ContractKey) -> Vec<u8> {
    let r: Result<HostResponse, ClientError> = Ok(HostResponse::ContractResponse(ContractResponse::PutResponse { key }));
    bincode::serialize(&r).expect("encodes")
}

fn put_refusal(key: freenet_stdlib::prelude::ContractKey, cause: &str) -> Vec<u8> {
    let e: ClientError =
        ErrorKind::RequestError(RequestError::ContractError(ContractError::Put { key, cause: cause.to_string().into() })).into();
    bincode::serialize(&Err::<HostResponse, ClientError>(e)).expect("encodes")
}

/// The key `contract` returns is the one the node's ack NAMES — or no ack
/// could ever be matched — and, for the webapp contract, the address the
/// node serves the container under.
#[test]
fn the_key_is_the_one_the_nodes_ack_names_and_the_webapp_address() {
    let state = b"a container".to_vec();
    let params = wire::webapp::params(&state);
    let (key, c, s) = contract(CODE, &params, &state);
    assert_eq!(s.as_ref(), state.as_slice());
    assert_eq!(unframe(&mut Reassembler::new(), &put_ack(c.key())), Incoming::Ack(AckKind::Put(key.clone())));
    assert_eq!(key, wire::webapp::address(CODE, &state));
    // Params are part of the key: a different one is a different contract.
    assert_ne!(contract(CODE, b"other", &state).0, key);
}

/// A PUT the node refuses NAMING the contract is `PutFailed` with that key and
/// the node's cause. A GET refusal is still `GetFailed`, and one naming
/// nothing is still `Refused`.
#[test]
fn a_put_refusal_naming_its_contract_is_put_failed_with_its_key() {
    let (key, c, _) = contract(CODE, b"p", b"s");
    assert_eq!(
        unframe(&mut Reassembler::new(), &put_refusal(c.key(), "invalid put")),
        Incoming::PutFailed { key, said: "invalid put".into() }
    );
    let get: ClientError =
        ErrorKind::RequestError(RequestError::ContractError(ContractError::Get { key: c.key(), cause: "x".into() })).into();
    let get = bincode::serialize(&Err::<HostResponse, ClientError>(get)).expect("encodes");
    assert!(matches!(unframe(&mut Reassembler::new(), &get), Incoming::GetFailed { .. }));
    let bare = bincode::serialize(&Err::<HostResponse, ClientError>(wire::_test::client_error("no"))).expect("encodes");
    assert!(matches!(unframe(&mut Reassembler::new(), &bare), Incoming::Refused(_)));
}
