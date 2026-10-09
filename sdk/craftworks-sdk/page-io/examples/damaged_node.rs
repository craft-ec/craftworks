//! A NODE THAT HAS LOST A TREE'S ROOT GROUP, for the JavaScript test that reads through the REAL web `Session`
//! (`tests/js/session-damaged.test.mjs`, sdk#524): its Register holds a real signed head naming a root and the root's
//! parity (a group of ONE, k = 1, sdk#335), and every block -- the root and each of its parity -- is answered the
//! node's own `NotFound`. So a reader's read of that tree waits on a group with none of its slots left: DAMAGED.
//! Bytes are the stdlib's own encoding, never a test's idea of it.
//!
//! `damaged_node <register code hex> [frame hex…]`. Prints one JSON object: `{"register": <the Register's instance id,
//! hex>, "root": <root id hex>, "answers": [<hex>…], "asked": [{"op", "key"}…]}` -- an answer for each GET frame: the
//! head for the Register, NotFound for anything else.

use core_types::hex::{decode as unhex, encode as hex};
use freenet_stdlib::client_api::{ClientError, ClientRequest, ContractRequest, ContractResponse, HostResponse};
use freenet_stdlib::prelude::WrappedState;

fn ok(r: HostResponse) -> Vec<u8> {
    bincode::serialize(&Ok::<HostResponse, ClientError>(r)).expect("encodes")
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    assert!(!a.is_empty(), "usage: damaged_node <register code hex> [frame hex…]");
    let register_code = unhex(&a[0]).expect("hex");
    // The owner: a fixed key, so every run names the same Register and head.
    let sk = ed25519_dalek::SigningKey::from_bytes(&[42u8; 32]);
    let params = wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME);
    // The root and its parity, coded as the engine codes a root (never served: the node lost them all).
    let root_bytes = b"a root this node no longer holds".to_vec();
    let root = freenet_prolly::block_id(freenet_prolly::kind::TREE_NODE, &root_bytes);
    let parity: Vec<[u8; 32]> = engine::repair::root_parity(&root_bytes).expect("a root's parity").into_iter().map(|(id, _)| id).collect();
    let ledger = signer_proto::head::Ledger { parity: Some(parity.concat()), ..Default::default() };
    let value = signer_proto::head::value(&root, &ledger);
    let head = contract_keys::register::head_state(&params, &sk.to_bytes(), 1, &value).expect("a signed head");
    let (_, register, _) = wire::puts::contract(&register_code, &params, &head);
    let register_id = *register.key().id();

    let mut answers = Vec::new();
    let mut asked = Vec::new();
    for f in &a[1..] {
        let Ok(ClientRequest::ContractOp(ContractRequest::Get { key, .. })) = bincode::deserialize::<ClientRequest>(&unhex(f).expect("hex")) else {
            continue;
        };
        let is_head = key == register_id;
        asked.push(format!(r#"{{"op":"get","key":"{}","head":{is_head}}}"#, key.encode()));
        answers.push(hex(&if is_head {
            ok(HostResponse::ContractResponse(ContractResponse::GetResponse { key: register.key(), contract: None, state: WrappedState::new(head.clone()) }))
        } else {
            ok(HostResponse::ContractResponse(ContractResponse::NotFound { instance_id: key }))
        }));
    }
    println!(
        r#"{{"register":"{}","root":"{}","answers":[{}],"asked":[{}]}}"#,
        hex(register_id.as_bytes()),
        hex(&root),
        answers.iter().map(|x| format!("\"{x}\"")).collect::<Vec<_>>().join(","),
        asked.join(",")
    );
}
