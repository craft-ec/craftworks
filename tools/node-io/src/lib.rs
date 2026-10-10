//! The tools' ONE way to a node's client API (Craftworks rule 1: one door) — a contract's container, a PUT and its
//! answer, a GET — and the beacon's standard params. Every tool (token-live, token-ops, beacon-live, health-watch) uses
//! these; none keeps its own copy.
use anyhow::{bail, Context, Result};
use craftworks_beacon_contract as B;
use freenet_stdlib::prelude::*;
use futures::{SinkExt, StreamExt};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

/// The beacon's STANDARD params (the shape every live beacon uses), from its seed.
pub fn beacon_params(seed: [u8; 32]) -> B::Params {
    B::Params { k: 1 << 20, segs: 256, sample: 8, window: 2, keep: 600, seed }
}

/// 32 bytes from a file holding them as hex — made fresh (and kept, 0600) when it does not exist.
pub fn seed_file(path: &str) -> Result<[u8; 32]> {
    if let Ok(s) = std::fs::read_to_string(path) {
        let v: Vec<u8> = (0..s.trim().len() / 2).map(|i| u8::from_str_radix(&s.trim()[2 * i..2 * i + 2], 16)).collect::<Result<_, _>>().context("a seed file holds hex")?;
        return v.try_into().map_err(|_| anyhow::anyhow!("a seed is 32 bytes"));
    }
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).map_err(|e| anyhow::anyhow!("randomness: {e}"))?;
    std::fs::write(path, b.iter().map(|x| format!("{x:02x}")).collect::<String>() + "\n")?;
    #[cfg(unix)]
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(b)
}

pub fn container(code: &[u8], params: Vec<u8>) -> ContractContainer {
    ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(code.to_vec())),
        Parameters::from(params),
    )))
}

pub async fn put(ws: &str, c: ContractContainer, state: Vec<u8>) -> Result<(String, u128)> {
    let (mut tx, mut rx) = tokio_tungstenite::connect_async(ws).await.context("connecting")?.0.split();
    let t = Instant::now();
    for f in wire::frame_put(c, WrappedState::new(state), 2).map_err(|e| anyhow::anyhow!(e))? {
        tx.send(Message::Binary(f.into())).await?;
    }
    let mut r = wire::Reassembler::new();
    // ONE deadline for the whole PUT (a per-message timeout restarts on every unrelated message).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let mut seen: Vec<String> = Vec::new();
    let said = loop {
        match tokio::time::timeout_at(deadline, rx.next()).await {
            Err(_) => break format!("NO ANSWER in 90 s (other messages: {})", seen.join(", ")),
            Ok(None) => bail!("closed"),
            Ok(Some(Err(e))) => bail!("{e}"),
            Ok(Some(Ok(Message::Binary(b)))) => match wire::unframe(&mut r, &b) {
                wire::Incoming::Ack(wire::AckKind::Put(_)) => break "ACCEPTED".to_string(),
                wire::Incoming::PutFailed { said, .. } | wire::Incoming::PutFailedByText { said, .. } => break format!("REFUSED: {said}"),
                wire::Incoming::Refused(r) => break format!("REFUSED: {r:?}"),
                other => seen.push(format!("{:.60}", format!("{other:?}"))),
            },
            Ok(Some(Ok(m))) => seen.push(format!("{:.20}", format!("{m:?}"))),
        }
    };
    Ok((said, t.elapsed().as_millis()))
}


pub async fn get(ws: &str, id: [u8; 32]) -> Result<Option<Vec<u8>>> {
    let (mut tx, mut rx) = tokio_tungstenite::connect_async(ws).await.context("connecting")?.0.split();
    for f in wire::frame_get(wire::contract_id(id), false, 3).map_err(|e| anyhow::anyhow!(e))? {
        tx.send(Message::Binary(f.into())).await?;
    }
    let mut r = wire::Reassembler::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        match tokio::time::timeout_at(deadline, rx.next()).await {
            Err(_) => return Ok(None),
            Ok(None) => bail!("closed"),
            Ok(Some(Err(e))) => bail!("{e}"),
            Ok(Some(Ok(Message::Binary(b)))) => match wire::unframe(&mut r, &b) {
                wire::Incoming::Got { state, .. } => return Ok(Some(state.to_vec())),
                wire::Incoming::GetFailed { .. } => return Ok(None),
                _ => {}
            },
            Ok(Some(Ok(_))) => {}
        }
    }
}

