//! WHAT A FREENET 0.2.138 NODE SAYS to a delegate request for a delegate it never registered (sdk#439,
//! freenet-core #5729): the bytes are the NODE'S, captured -- never built by this test.
//!
//! Captured 2026-09-26 from a private node (freenet 0.2.138, 5fb1aa93e15c; explicit dirs; ws on a free port):
//! realnet visitor V's own signer request (wire-V.jsonl's first frame, an ApplicationMessages to the signer
//! delegate Eu9Z7D7QRhgqfhjj8KDTosvSe7bzsyM15fstynnVNYJJ), sent once, then twice more 10 and 60 ms after its answer.
//! The answers, in order: `MISSING` (+2 ms), `THROTTLED` (+38 ms, the node's delegate backoff armed by the Missing),
//! `MISSING` again (+90 ms, the backoff over). The same 80 `MISSING` bytes are what V received live in realnet.
use freenet_stdlib::client_api::{ClientError, DelegateError, ErrorKind, HostResponse, RequestError};
use wire::Incoming;

const SIGNER: &str = "Eu9Z7D7QRhgqfhjj8KDTosvSe7bzsyM15fstynnVNYJJ";
const MISSING: &str = "01000000080000000100000002000000ce83cdbd19994a6d16cf0f8459bcb29dc6902d4bbe2ce858d9c1ef240036dfaf840adca4f234e5d42ae92265bab472486ef7fe1c81ace9830929b472d44f6271";
const THROTTLED: &str = "010000000800000001000000010000006d0000000000000064656c6567617465204575395a374437515268677166686a6a384b44546f7376536537627a73794d3135667374796e6e564e594a4a2069732072617465206c696d69746564206166746572207265706561746564206661696c757265733b20726574727920696e203437206d73";

fn bytes(h: &str) -> Vec<u8> {
    core_types::hex::decode(h).expect("hex")
}

/// "No such delegate here" NAMES the delegate: it is decoded with its key, never flattened into a keyless refusal
/// (which a page cannot attribute to its signer request).
#[test]
fn a_0_2_138_nodes_missing_delegate_is_decoded_with_its_key() {
    assert_eq!(wire::_test::classify_for_test(&bytes(MISSING)), Incoming::DelegateMissing { key: SIGNER.into() });
}

/// The node's delegate BACKOFF is its own answer -- not a refusal, not a Missing.
#[test]
fn a_0_2_138_nodes_delegate_backoff_is_decoded_as_throttled() {
    match wire::_test::classify_for_test(&bytes(THROTTLED)) {
        Incoming::DelegateThrottled { said } => {
            assert!(said.contains(wire::DELEGATE_THROTTLED) && said.contains(SIGNER), "the node's words were not kept: {said}")
        }
        other => panic!("the node's backoff was read as {other:?}"),
    }
}

/// THE CONTROLS: another ExecutionError (no backoff words) and another delegate error stay what they were -- a
/// keyless refusal -- so the two readings above are keyed on what the node said, not on "a delegate error".
#[test]
fn control_other_delegate_errors_stay_refusals() {
    for e in [DelegateError::ExecutionError("the delegate panicked".into()), DelegateError::RegisterError(wire::delegate_from_code(b"any delegate").1)] {
        let b = bincode::serialize(&Err::<HostResponse, ClientError>(ErrorKind::RequestError(RequestError::DelegateError(e.clone())).into())).expect("encodes");
        assert!(matches!(wire::_test::classify_for_test(&b), Incoming::Refused(_)), "{e:?} was not a refusal");
    }
}

/// The node's bytes ARE the stdlib's encoding of those errors: a test that builds a Missing or a backoff answer
/// with freenet-stdlib gets exactly these bytes (so page-io's forced tests, which must name their own signer,
/// build what the node sends).
#[test]
fn the_captured_bytes_are_stdlibs_own_encoding() {
    for h in [MISSING, THROTTLED] {
        let e: Result<HostResponse, ClientError> = bincode::deserialize(&bytes(h)).expect("the node's bytes decode");
        assert_eq!(bincode::serialize(&e).expect("encodes"), bytes(h), "re-encoding the node's answer changed its bytes");
    }
}
