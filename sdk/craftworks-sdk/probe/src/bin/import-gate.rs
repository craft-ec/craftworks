//! Refuse a delegate wasm that imports a host function the pinned node does
//! not define.
//!
//! An artefact gate, run on every delegate build. It WILL fire when
//! freenet-stdlib is bumped and the new version calls something the pinned
//! node has not got — that is its job, not a false alarm.
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| {
        eprintln!("usage: import-gate <delegate.wasm>");
        std::process::exit(2);
    });
    let wasm = std::fs::read(&path)?;
    // ONE call. Both halves of the gate are inside it, because they are not
    // independent: the subset check is vacuous over a module with no imports,
    // so a caller running only that half reports a clean pass over an unwired
    // delegate.
    match probe::check(&wasm) {
        Ok(c) => {
            println!(
                "{path}: {} import(s), {} from freenet_delegate*",
                c.total,
                c.delegate.len()
            );
            for (m, f) in &c.delegate {
                println!("  {m}::{f}");
            }
            println!(
                "ok: every delegate import is one of the {} the node defines",
                probe::DEFINED_BY_NODE.len()
            );
            Ok(())
        }
        Err(e) => {
            println!("REFUSED {path}: {e}");
            anyhow::bail!("{path}: {e}")
        }
    }
}
