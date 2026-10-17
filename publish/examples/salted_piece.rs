//! A SALTED piece is served as its `piece` file (the publisher's placement, 10-18): one piece put through one node in
//! a container with a salt, then fetched over HTTP through ANOTHER node at `/v1/contract/web/<address>/piece` — the
//! bytes must be the piece's. Control: the salt-0 container of the same bytes is at a different address.
//!   cargo run -p craftworks-publish --example salted_piece -- <put ws> <get http base> <webapp.wasm>
use anyhow::{bail, Context, Result};
use freenet_stdlib::prelude::*;
use futures::{SinkExt, StreamExt};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

#[tokio::main]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let (ws, http, code) = (&a[1], &a[2], std::fs::read(&a[3]).context("the webapp code")?);
    let bytes: Vec<u8> = (0..4096u32).map(|i| (i * 7 + std::process::id()) as u8).collect();
    let plain = wire::webapp::piece_container(&bytes).map_err(|e| anyhow::anyhow!(e))?;
    let salted = wire::webapp::app_container(&[(wire::webapp::PIECE_FILE, &bytes), ("salt", b"1")]).map_err(|e| anyhow::anyhow!(e))?;
    let (pa, sa) = (wire::webapp::address(&code, &plain), wire::webapp::address(&code, &salted));
    println!("salt 0 at {pa}\nsalt 1 at {sa}");
    if pa == sa {
        bail!("the salt did not move the address");
    }
    let contract = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(code.clone())),
        Parameters::from(wire::webapp::params(&salted).to_vec()),
    )));
    let (mut c, _) = tokio_tungstenite::connect_async(ws.as_str()).await?;
    for f in wire::frame_put(contract, WrappedState::new(salted.clone()), 1).map_err(|e| anyhow::anyhow!(e))? {
        c.send(Message::Binary(f.into())).await?;
    }
    let mut r = wire::Reassembler::new();
    let acked = tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(Ok(m)) = c.next().await {
            if let Message::Binary(b) = m {
                if let wire::Incoming::Ack(wire::AckKind::Put(k)) = wire::unframe(&mut r, &b) {
                    return Some(k);
                }
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    println!("put: {acked:?}");
    for i in 0..12 {
        let out = std::process::Command::new("curl").args(["-s", "-m", "60", "-o", "/tmp/salted.piece", "-w", "%{http_code}", &format!("{http}/v1/contract/web/{sa}/piece")]).output()?;
        let code = String::from_utf8_lossy(&out.stdout).to_string();
        let got = std::fs::read("/tmp/salted.piece").unwrap_or_default();
        println!("try {i}: HTTP {code}, {} bytes, the piece's: {}", got.len(), got == bytes);
        if got == bytes {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
    bail!("the salted piece was not served as its piece file")
}
