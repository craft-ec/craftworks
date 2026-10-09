//! PRE-PROVISION a node's signer with a GIVEN key: the realnet harness's
//! same-key convergence step (#117). Two private nodes provisioned with ONE
//! throwaway key sign for ONE Register, so a page on either opens the same
//! tree — what the page does on a node whose signer already holds a key
//! (`Session::provision_page` asks, and mints only when there is none).
//!
//! It proves same-key convergence across two nodes; it is NOT how a person
//! adds a second device (that is the keyset, Phase 6).
//!
//! Through the page's own path, never a second encoder (`probe::page::for_key`,
//! the one construction live-page-writes also uses). The key is the only
//! difference: given here (hex seed), random there.
//!
//! usage: provision-signer <ws-url> <seed-hex-32> <signer.wasm> <block.wasm> <register.wasm>
//! Prints `register <hex>` once the signer answers Provisioned (or names the
//! Register it already holds for this key); refuses the owner's 7509/7609.
use anyhow::{bail, Context, Result};
use futures::{SinkExt, StreamExt};
use page::Ms;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let usage = "usage: provision-signer <ws-url> <seed-hex-32> <signer.wasm> <block.wasm> <register.wasm>";
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [ws, seed, signer, block, register] = a.as_slice() else { bail!("{usage}") };
    probe::node::allowed_port(ws)?;
    let seed: [u8; 32] = core_types::hex::decode(seed).context("seed: hex")?.try_into().map_err(|_| anyhow::anyhow!("seed: 32 bytes"))?;
    let signer_wasm = std::fs::read(signer).with_context(|| format!("reading {signer}"))?;
    probe::check(&signer_wasm).map_err(|e| anyhow::anyhow!("the signer is refused by the import gate: {e}"))?;
    let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
    let mut io = probe::page::for_key(
        &sk,
        &signer_wasm,
        std::fs::read(block).with_context(|| format!("reading {block}"))?,
        std::fs::read(register).with_context(|| format!("reading {register}"))?,
    );

    let (mut sock, _) = tokio_tungstenite::connect_async(ws.as_str()).await.with_context(|| format!("connecting to {ws}"))?;
    let t0 = Instant::now();
    let now = || Ms(1_000 + t0.elapsed().as_millis() as u64);
    // A deadline on the whole exchange: a node that never answers is said, never waited on.
    let budget = Duration::from_secs(60);
    loop {
        for f in io.take_frames() {
            sock.send(Message::Binary(f.into())).await.context("send")?;
        }
        if io.provisioned() {
            println!("register {}", core_types::hex::encode(&io.register_id()));
            return Ok(());
        }
        if !io.unusable().is_empty() {
            bail!("the signer did not take the key: {:?}", io.unusable());
        }
        if t0.elapsed() > budget {
            bail!("not provisioned within {budget:?}");
        }
        match tokio::time::timeout(Duration::from_millis(50), sock.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => {
                io.inbound(&b, now());
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(e))) => bail!("the socket: {e}"),
            Ok(None) => bail!("the node closed the socket"),
            Err(_) => io.tick(now()),
        }
    }
}
