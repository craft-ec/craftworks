//! A live PACKAGE rebuilt from its pieces (a hotfix keeps every other package byte-identical to the live site):
//!   join_package <entry.json> <pieces dir: piece-<i> files, any k of them> <out file>
use anyhow::{Context, Result};
fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let e: serde_json::Value = serde_json::from_slice(&std::fs::read(&a[1])?)?;
    let (k, m, payload, len) = (e["k"].as_u64().context("k")? as usize, e["m"].as_u64().context("m")? as usize, e["payload"].as_u64().context("payload")? as usize, e["bundle_len"].as_u64().context("bundle_len")? as usize);
    let have: Vec<Option<Vec<u8>>> = (0..k + m).map(|i| std::fs::read(format!("{}/piece-{i}", a[2])).ok()).collect();
    let bundle = pieces::join(k, m, payload, len, &have).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let files = pieces::unbundle(&bundle).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let (name, bytes) = files.into_iter().next().context("an empty bundle")?;
    std::fs::write(&a[3], &bytes)?;
    println!("{name}: {} B", bytes.len());
    Ok(())
}
