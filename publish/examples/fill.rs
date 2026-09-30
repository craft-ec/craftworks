//! FILL: a measurement for the node's per-contract memory (upstream freenet-core#5647). Puts `n` immutable contracts
//! of ONE code (craftworks' `sealed`, the kind a table's tree blocks and a file's pieces are) with random `size`-byte
//! states into a node, `batch` at a time; after each batch, once every put is answered and the node has settled, it
//! prints the contracts put so far and the node process's resident memory (`ps -o rss`).
//!
//!   cargo run --release -p craftworks-publish --example fill -- ws://127.0.0.1:17801/v1/contract/command?encodingProtocol=native \
//!     contracts/sealed.wasm <node pid> <n> <batch> <size> <settle secs>
use anyhow::{bail, Context, Result};
use freenet_stdlib::prelude::*;
use futures::{SinkExt, StreamExt};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

fn rss_kib(pid: &str) -> u64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", pid]).output().expect("ps runs");
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0)
}

#[tokio::main]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let [_, ws, code, pid, n, batch, size, settle] = &a[..] else { bail!("usage: fill <ws> <code.wasm> <pid> <n> <batch> <size> <settle secs>") };
    let code = std::fs::read(code).context("the contract code")?;
    let (n, batch, size, settle): (usize, usize, usize, u64) = (n.parse()?, batch.parse()?, size.parse()?, settle.parse()?);
    let (mut tx, mut rx) = tokio_tungstenite::connect_async(ws.as_str()).await.context("connecting")?.0.split();
    let mut r = wire::Reassembler::new();
    let t0 = Instant::now();
    println!("t_s,contracts,rss_kib");
    println!("{},{},{}", t0.elapsed().as_secs(), 0, rss_kib(pid));
    let mut stream = 1u32;
    let mut done = 0;
    while done < n {
        let mut owed = std::collections::HashSet::new();
        for _ in 0..batch.min(n - done) {
            // Distinct per run and per contract (the run's start time, the count): no two runs collide.
            let seed = blake3::hash(format!("fill {:?} {done} {stream}", std::time::SystemTime::now()).as_bytes());
            let address = *seed.as_bytes();
            let mut state = vec![0u8; size];
            blake3::Hasher::new_keyed(&address).finalize_xof().fill(&mut state);
            let c = wire::block::block_contract(&code, &address);
            owed.insert(c.key().id().encode());
            stream += 1;
            for f in wire::frame_put(c, WrappedState::new(state), stream).map_err(|e| anyhow::anyhow!(e))? {
                tx.send(Message::Binary(f.into())).await?;
            }
        }
        let deadline = Instant::now() + Duration::from_secs(120);
        while !owed.is_empty() && Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(1), rx.next()).await {
                Ok(Some(Ok(Message::Binary(b)))) => match wire::unframe(&mut r, &b) {
                    wire::Incoming::Ack(wire::AckKind::Put(key)) => {
                        owed.remove(&key);
                    }
                    wire::Incoming::PutFailed { key, said } | wire::Incoming::PutFailedByText { key, said } => {
                        eprintln!("refused {key}: {said}");
                        owed.remove(&key);
                    }
                    _ => {}
                },
                Ok(None) => bail!("the node closed the connection"),
                _ => {}
            }
        }
        if !owed.is_empty() {
            eprintln!("{} put(s) unanswered after 120 s", owed.len());
        }
        done += batch.min(n - done);
        tokio::time::sleep(Duration::from_secs(settle)).await;
        println!("{},{},{}", t0.elapsed().as_secs(), done, rss_kib(pid));
    }
    Ok(())
}
