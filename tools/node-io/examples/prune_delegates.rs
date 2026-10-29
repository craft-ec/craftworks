//! PRUNE a TEST node's delegates: every delegate it holds but the ones the given wasm make (the current build's
//! signer and identity) is UNREGISTERED — 10-29, fx/fy: 60+ old builds' identity delegates, each woken every minute,
//! spent the node's whole wake-up budget and starved the current build's rounds. Test nodes only (never a live one:
//! a member who skipped builds is handed over by an earlier build's delegate).
//! NOT freed by this: the APP's background slots (64 per app, freenet `MAX_RECORDS_PER_APP`) — the node drops a
//! delegate's record only on an unregister from the app's own connection (a page), never a tool's (measured 10-29,
//! fx: after pruning, "This app already has the maximum number of background-capable delegates").
//!   cargo run --example prune_delegates -- <ws url> <node data dir>/delegates <keep.wasm>…
use anyhow::{Context, Result};
use freenet_stdlib::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (ws, dir, keep_wasm) = (a.first().context("ws url")?, a.get(1).context("delegates dir")?, &a[2..]);
    anyhow::ensure!(!keep_wasm.is_empty(), "the wasm to keep (the current signer and identity) must be given");
    let keep: Vec<String> = keep_wasm.iter().map(|p| std::fs::read(p).map(|w| wire::delegate_from_code(&w).1.encode())).collect::<Result<_, _>>()?;
    println!("keeping {}", keep.iter().map(|k| &k[..12]).collect::<Vec<_>>().join(", "));
    let (mut gone, mut kept, mut left) = (0, 0, 0);
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.extension().and_then(|x| x.to_str()) != Some("reg") {
            continue;
        }
        let name = p.file_stem().and_then(|s| s.to_str()).context("a .reg name")?.to_string();
        if keep.contains(&name) {
            kept += 1;
            continue;
        }
        let reg = std::fs::read(&p)?;
        let key: [u8; 32] = bs58::decode(&name).into_vec()?.try_into().map_err(|_| anyhow::anyhow!("{name}: not 32 bytes"))?;
        let code: [u8; 32] = reg.get(1..33).context("a .reg holds its code hash")?.try_into()?;
        let answered = node_io::unregister_delegate(ws, key, code).await?;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        if p.exists() { left += 1; println!("{}: still registered (answered: {answered})", &name[..12]); } else { gone += 1; }
    }
    println!("unregistered {gone}, kept {kept}, still registered {left}");
    Ok(())
}
