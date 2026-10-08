//! SPIKE (TOKEN prune cut): prolly absence proofs — their size, and a contract checking them.
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
    // prolly-spike-run <contract.wasm> <ws>
    use freenet_prolly::{build::build, node::Value, proof::{prove, verify_bytes, ProvenOwned}, store::MemBlocks};
    let a: Vec<String> = std::env::args().collect();
    let code = std::fs::read(&a[1])?;
    let ws = a[2].as_str();
    let salt = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_nanos().to_le_bytes();
    for n in [1_000u32, 100_000, 1_000_000] {
        // A RECEIVED SET of n names (48 bytes: sender key ‖ send nonce), as a cut would hold.
        let mut keys: Vec<Vec<u8>> = (0..n).map(|i| { let h = blake3::hash(&[&salt[..], &i.to_le_bytes()].concat()); [h.as_bytes().as_slice(), &h.as_bytes()[..16]].concat() }).collect();
        keys.sort();
        let mut blocks = MemBlocks::default();
        let t = Instant::now();
        let root = build(keys.iter().map(|k| (k.as_slice(), Value::Inline(&[]))), |cid, b| blocks.insert(cid, b)).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let built = t.elapsed().as_millis();
        let absent: Vec<u8> = { let h = blake3::hash(b"never received"); [h.as_bytes().as_slice(), &[0u8; 16]].concat() };
        let present = keys[n as usize / 2].clone();
        let pa = prove(&blocks, &root, &absent).map_err(|e| anyhow::anyhow!("{e:?}"))?.encode();
        let pp = prove(&blocks, &root, &present).map_err(|e| anyhow::anyhow!("{e:?}"))?.encode();
        let t = Instant::now();
        let mut ok = 0;
        for _ in 0..1000 {
            if matches!(verify_bytes(&root, &absent, &pa), Ok(ProvenOwned::Absent)) {
                ok += 1;
            }
        }
        let us = t.elapsed().as_micros() as f64 / 1000.0;
        println!("n={n:>9}: tree built in {built} ms ({} blocks); absence proof {} B, presence proof {} B; native verify {us:.1} µs (ok {ok}/1000)", blocks.0.len(), pa.len(), pp.len());
        let state = |key: &[u8], proof: &[u8]| [&root[..], &(key.len() as u16).to_le_bytes(), key, proof].concat();
        let mut bad = pa.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 1;
        for (name, st, tag) in [("ABSENT key, its proof", state(&absent, &pa), 1u8), ("PRESENT key, its proof", state(&present, &pp), 2), ("ABSENT key, a tampered proof", state(&absent, &bad), 3)] {
            let c = container(&code, [&salt[..], &n.to_le_bytes(), &[tag]].concat());
            let (s, ms) = put(ws, c, st).await?;
            println!("    PUT {name}: {s} ({ms} ms)");
        }
    }
    Ok(())
}
