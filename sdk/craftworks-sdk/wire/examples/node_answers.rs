//! What a node would SAY about a contract PUT, and what a page's frames ARE —
//! in the stdlib's own encoding, for the JavaScript test that drives the real
//! `Session` (`tests/js/contract-put.test.mjs`). Bytes a test built by hand
//! would agree with the test's own idea of the format; these are the node's.
//!
//! `node_answers <code> <params> <state> <cause> [frame…]`, every argument
//! but `cause` in hex. Prints one JSON object:
//! `{"key", "ack", "refusal", "got", "frames": [{"op", …}]}` — the PUT's ack
//! and refusal and a GET answer, hex-encoded as the node sends them, and each
//! frame decoded.

use freenet_stdlib::client_api::{ClientError, ClientRequest, ContractError, ContractRequest, ContractResponse, ErrorKind, HostResponse, RequestError};

fn unhex(s: &str) -> Vec<u8> {
    core_types::hex::decode(s).expect("hex")
}

use core_types::hex::encode as hex;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    assert!(a.len() >= 4, "usage: node_answers <code> <params> <state> <cause> [frame…]");
    let (key, c, _) = wire::puts::contract(&unhex(&a[0]), &unhex(&a[1]), &unhex(&a[2]));
    let ack = bincode::serialize(&Ok::<HostResponse, ClientError>(HostResponse::ContractResponse(
        ContractResponse::PutResponse { key: c.key() },
    )))
    .expect("encodes");
    let refusal: ClientError = ErrorKind::RequestError(RequestError::ContractError(ContractError::Put {
        key: c.key(),
        cause: a[3].clone().into(),
    }))
    .into();
    let refusal = bincode::serialize(&Err::<HostResponse, ClientError>(refusal)).expect("encodes");
    // The KEYLESS form the node sends for a PUT its executor refused (sdk#433): the pinned `PUT_ERROR_FORMAT`.
    let (prefix, sep) = wire::PUT_ERROR_FORMAT;
    let keyless: ClientError = ErrorKind::OperationError { cause: format!("{prefix}{}{sep}{}", c.key(), a[3]).into() }.into();
    let keyless = bincode::serialize(&Err::<HostResponse, ClientError>(keyless)).expect("encodes");
    // A GET answer for this contract: what a page that never asked for it
    // must leave to whoever did.
    let got = bincode::serialize(&Ok::<HostResponse, ClientError>(HostResponse::ContractResponse(
        ContractResponse::GetResponse { key: c.key(), contract: None, state: freenet_stdlib::prelude::WrappedState::new(unhex(&a[2])) },
    )))
    .expect("encodes");
    let frames: Vec<String> = a[4..]
        .iter()
        .map(|f| match bincode::deserialize::<ClientRequest>(&unhex(f)) {
            Ok(ClientRequest::ContractOp(ContractRequest::Put { contract, state, .. })) => format!(
                r#"{{"op":"put","key":"{}","state":"{}"}}"#,
                contract.key().id().encode(),
                hex(state.as_ref())
            ),
            Ok(ClientRequest::ContractOp(ContractRequest::Get { key, subscribe, .. })) => {
                format!(r#"{{"op":"get","key":"{}","subscribe":{subscribe}}}"#, key.encode())
            }
            Ok(ClientRequest::ContractOp(ContractRequest::Update { key, .. })) => {
                format!(r#"{{"op":"update","key":"{}"}}"#, key.id().encode())
            }
            // A SIGNER request inside a delegate message: named, with its id.
            Ok(ClientRequest::DelegateOp(freenet_stdlib::client_api::DelegateRequest::ApplicationMessages { inbound, .. })) => {
                let reqs: Vec<String> = inbound
                    .iter()
                    .filter_map(|m| match m {
                        freenet_stdlib::prelude::InboundDelegateMsg::ApplicationMessage(am) => signer_proto::decode_request(&am.payload)
                            .map(|(id, r)| format!(r#"{{"req":"{}","id":{id}}}"#, format!("{r:?}").split([' ', '{', '(']).next().unwrap_or("?"))),
                        _ => None,
                    })
                    .collect();
                format!(r#"{{"op":"delegate","signer":[{}]}}"#, reqs.join(","))
            }
            Ok(ClientRequest::DelegateOp(_)) => r#"{"op":"delegate"}"#.to_string(),
            Ok(other) => format!(r#"{{"op":"{}"}}"#, format!("{other:?}").split(['(', ' ', '{']).next().unwrap_or("?")),
            Err(_) => r#"{"op":"undecodable"}"#.to_string(),
        })
        .collect();
    // SIGNER_ANSWER=register:<id>:<params hex | none> — the signer's answer to
    // "which Register?", as the node delivers it (a delegate response).
    let signer_answer = std::env::var("SIGNER_ANSWER").ok().map(|spec| {
        let mut it = spec.splitn(3, ':');
        let (kind, id, arg) = (it.next().expect("kind"), it.next().expect("id").parse::<u32>().expect("id"), it.next().unwrap_or(""));
        // register:<id>:<params hex | none>, or refused:<id>: (the signer's
        // "a different key is already provisioned").
        let answer = match kind {
            "refused" => signer_proto::Answer::Refused(signer_proto::Why::KeyAlreadyProvisioned),
            _ => signer_proto::Answer::Register { params: (arg != "none").then(|| unhex(arg)) },
        };
        let payload = signer_proto::encode_answer(id, &answer);
        let (_, dkey) = wire::delegate_from_code(b"any: the page does not check which delegate answered");
        hex(&bincode::serialize(&Ok::<HostResponse, ClientError>(HostResponse::DelegateResponse {
            key: dkey,
            values: vec![freenet_stdlib::prelude::OutboundDelegateMsg::ApplicationMessage(
                freenet_stdlib::prelude::ApplicationMessage::new(payload),
            )],
        }))
        .expect("encodes"))
    });
    // SIGNER_CODE=<hex> — the node's answer to the SIGNER's registration, as
    // measured on 0.2.136 (sdk#260's frame log): a DelegateResponse naming
    // that delegate and carrying NO message.
    let registered = std::env::var("SIGNER_CODE").ok().map(|code| {
        let (_, dkey) = wire::delegate_from_code(&unhex(&code));
        hex(&bincode::serialize(&Ok::<HostResponse, ClientError>(HostResponse::DelegateResponse { key: dkey, values: vec![] })).expect("encodes"))
    });
    println!(
        r#"{{"key":"{key}","ack":"{}","refusal":"{}","keyless":"{}","got":"{}","signer_answer":"{}","registered":"{}","frames":[{}]}}"#,
        hex(&ack),
        hex(&refusal),
        hex(&keyless),
        hex(&got),
        signer_answer.unwrap_or_default(),
        registered.unwrap_or_default(),
        frames.join(",")
    );
}
