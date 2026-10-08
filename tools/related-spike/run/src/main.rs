//! SPIKE runner: PUT a LOCK on node X, then a FILL citing it on node Y — does Y's validation fetch the lock (a
//! RELATED contract) from the network, how long does it take, and are a missing lock and a wrong lock refused?
//!   related-spike-run <wasm> <ws X> <ws Y>
use anyhow::{bail, Context, Result};
use freenet_stdlib::prelude::*;
use futures::{SinkExt, StreamExt};
use std::time::{Duration, Instant, SystemTime};
use tokio_tungstenite::tungstenite::Message;

fn container(code: &[u8], params: Vec<u8>) -> ContractContainer {
    ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(code.to_vec())),
        Parameters::from(params),
    )))
}

async fn put(ws: &str, c: ContractContainer, state: Vec<u8>) -> Result<(String, u128)> {
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

#[tokio::main]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let code = std::fs::read(&a[1])?;
    // SINGLE STEPS (a lock on one node while the other is down; its fill later): `lock <ws> <tag>` / `fill <ws> <lock id>`.
    if a[2] == "lock" {
        let lc = container(&code, [&[0u8][..], a[4].as_bytes()].concat());
        let id = *lc.key().id();
        let (s, ms) = put(&a[3], lc, b"LOCK solo".to_vec()).await?;
        println!("lock {}: {s} ({ms} ms)", id.encode());
        return Ok(());
    }
    if a[2] == "fill" {
        let lock = ContractInstanceId::from_bytes(&a[4]).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let (s, ms) = put(&a[3], container(&code, [&[1u8][..], lock.as_bytes()].concat()), b"fill solo".to_vec()).await?;
        println!("fill citing {}: {s} ({ms} ms)", a[4]);
        return Ok(());
    }
    let (x, y) = (a[2].as_str(), a[3].as_str());
    let n = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_nanos().to_le_bytes();
    let id_of = |c: &ContractContainer| *c.key().id();
    let fill = |lock: ContractInstanceId| container(&code, [&[1u8][..], lock.as_bytes()].concat());

    // A: a fill citing a lock nobody made.
    let ghost = id_of(&container(&code, [&[0u8][..], &n, b"ghost"].concat()));
    let (s, ms) = put(y, fill(ghost), b"fill A".to_vec()).await?;
    println!("A  fill citing a MISSING lock, on Y: {s} ({ms} ms)");

    // B: lock LOCK… on X, then the fill on Y at once.
    let lc = container(&code, [&[0u8][..], &n, b"B"].concat());
    let lock = id_of(&lc);
    let (s, ms) = put(x, lc, b"LOCK b".to_vec()).await?;
    println!("B1 lock LOCK on X: {s} ({ms} ms)");
    let (s, ms) = put(y, fill(lock), b"fill B".to_vec()).await?;
    println!("B2 fill citing it, on Y: {s} ({ms} ms)");

    // C: lock NOPE… on X, fill on Y.
    let lc = container(&code, [&[0u8][..], &n, b"C"].concat());
    let lock = id_of(&lc);
    let (s, ms) = put(x, lc, b"NOPE c".to_vec()).await?;
    println!("C1 lock NOPE on X: {s} ({ms} ms)");
    let (s, ms) = put(y, fill(lock), b"fill C".to_vec()).await?;
    println!("C2 fill citing it, on Y: {s} ({ms} ms)");

    // D: lock and fill both on Y (the lock in Y's own store).
    let lc = container(&code, [&[0u8][..], &n, b"D"].concat());
    let lock = id_of(&lc);
    let (s, ms) = put(y, lc, b"LOCK d".to_vec()).await?;
    println!("D1 lock LOCK on Y: {s} ({ms} ms)");
    let (s, ms) = put(y, fill(lock), b"fill D".to_vec()).await?;
    println!("D2 fill citing it, on Y: {s} ({ms} ms)");
    Ok(())
}
