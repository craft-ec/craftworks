//! Driving the SIGNER delegate over the client API, for the live probes (live-signer, live-held): register it,
//! send it a request under a fresh id, and take the answer to THAT request (never another's). Plus the two
//! contract ops the probes need. Moved here from live-signer so each probe is not its own copy.
use anyhow::{bail, Context, Result};
use freenet_stdlib::client_api::{ClientRequest, ContractRequest, ContractResponse, DelegateRequest, HostResponse, WebApi};
use freenet_stdlib::prelude::*;
use signer::{Answer, Request};
use std::time::Duration;
use tokio::time::timeout;

/// Each step's wait for an answer.
pub const STEP: Duration = Duration::from_secs(20);

/// Connect to a node's client API. REFUSES the owner's nodes (7509/7609) BEFORE any socket opens: the guard is
/// here, once, so every probe that connects through this is safe by construction.
pub async fn connect(ws: &str) -> Result<WebApi> {
    crate::node::allowed_port(ws)?;
    let (stream, _) = tokio_tungstenite::connect_async(ws).await.context("connecting")?;
    Ok(WebApi::start(stream))
}

pub async fn register_delegate(c: &mut WebApi, wasm: &[u8]) -> Result<DelegateKey> {
    let (delegate, key) = wire::delegate_from_code(wasm);
    timeout(
        STEP,
        c.send(ClientRequest::DelegateOp(
            DelegateRequest::RegisterDelegate {
                delegate,
                cipher: [0u8; 32],
                nonce: [0u8; 24],
            },
        )),
    )
    .await
    .map_err(|_| anyhow::anyhow!("registering timed out"))??;
    let _ = timeout(Duration::from_secs(3), c.recv()).await;
    Ok(key)
}

/// Every signer request of this run gets its own id (SG02), from 1: `0` is UNATTRIBUTED.
static NEXT_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// Send `r` under a fresh id, and return the id its answer will carry.
pub async fn send_signer(c: &mut WebApi, key: &DelegateKey, r: &Request) -> Result<u32> {
    let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    timeout(
        STEP,
        c.send(ClientRequest::DelegateOp(
            DelegateRequest::ApplicationMessages {
                key: key.clone(),
                params: vec![].into(),
                inbound: vec![InboundDelegateMsg::ApplicationMessage(
                    ApplicationMessage::new(signer::encode_request(id, r)),
                )],
            },
        )),
    )
    .await
    .map_err(|_| anyhow::anyhow!("send blocked"))??;
    Ok(id)
}

/// The answer to request `id`, attributed by its id alone. Answers under any other id -- a `Put` (UNATTRIBUTED), or
/// another request's -- are counted into `others`, never taken for this one.
pub async fn answer_to(c: &mut WebApi, id: u32, others: &mut Vec<(u32, Answer)>) -> Result<Answer> {
    let end = tokio::time::Instant::now() + STEP;
    while tokio::time::Instant::now() < end {
        match timeout(Duration::from_millis(500), c.recv()).await {
            Ok(Ok(HostResponse::DelegateResponse { values, .. })) => {
                for v in values {
                    if let OutboundDelegateMsg::ApplicationMessage(m) = v {
                        match signer::decode_answer(&m.payload) {
                            Some((got, a)) if got == id => return Ok(a),
                            Some(other) => others.push(other),
                            None => {}
                        }
                    }
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => bail!("node error: {e}"),
            Err(_) => {}
        }
    }
    bail!("no answer to signer request {id} within {STEP:?}")
}

/// Ask, and take the answer to THIS request; an answer to any other is a finding, not this one's.
pub async fn ask(c: &mut WebApi, key: &DelegateKey, r: &Request) -> Result<Answer> {
    let id = send_signer(c, key, r).await?;
    let mut others = Vec::new();
    let a = answer_to(c, id, &mut others).await?;
    if others.iter().any(|(i, _)| *i != signer::UNATTRIBUTED) {
        bail!("answers to requests not in flight arrived before request {id}'s: {others:?}");
    }
    Ok(a)
}

pub fn container(code: &[u8], params: &[u8]) -> ContractContainer {
    ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(code.to_vec())),
        Parameters::from(params.to_vec()),
    )))
}

pub async fn contract_op(c: &mut WebApi, r: ContractRequest<'static>) -> Result<String> {
    contract_op_within(c, r, STEP).await
}

/// `contract_op` waiting at most `within` for the answer (a real-network PUT's tail runs past a step).
pub async fn contract_op_within(c: &mut WebApi, r: ContractRequest<'static>, within: Duration) -> Result<String> {
    timeout(STEP, c.send(ClientRequest::ContractOp(r)))
        .await
        .map_err(|_| anyhow::anyhow!("send blocked"))??;
    let end = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < end {
        match timeout(Duration::from_millis(500), c.recv()).await {
            Ok(Ok(HostResponse::ContractResponse(ContractResponse::PutResponse { .. }))) => {
                return Ok("put".into())
            }
            Ok(Ok(HostResponse::ContractResponse(ContractResponse::UpdateResponse { .. }))) => {
                return Ok("update".into())
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => bail!("node error: {e}"),
            Err(_) => {}
        }
    }
    bail!("no answer to a contract op within {within:?}")
}


/// A raw block a probe PUTs (`kind::RAW` ‖ `body`) and its id: the state and the id the node will derive.
pub fn raw_block(body: Vec<u8>) -> ([u8; 32], Vec<u8>) {
    let id = freenet_prolly::block_id(freenet_prolly::kind::RAW, &body);
    let mut st = vec![freenet_prolly::kind::RAW];
    st.extend_from_slice(&body);
    (id, st)
}

#[cfg(test)]
mod tests {
    /// The guard runs before any socket: the owner's port is refused with the GUARD's words (a network error would
    /// say "connecting"), and a free, closed port -- the control -- fails as a connection, so the guard is not
    /// refusing everything.
    #[tokio::test]
    async fn connect_refuses_the_owners_nodes_before_opening_a_socket() {
        for port in [7509, 7609] {
            let e = super::connect(&format!("ws://127.0.0.1:{port}/v1/contract/command")).await.err().expect("refused");
            assert!(format!("{e:#}").contains("the owner's node"), "not refused by the guard: {e:#}");
        }
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let e = super::connect(&format!("ws://127.0.0.1:{free}/v1/contract/command")).await.err().expect("nothing listens there");
        assert!(format!("{e:#}").contains("connecting"), "CONTROL: a closed port did not fail as a connection: {e:#}");
    }
}
