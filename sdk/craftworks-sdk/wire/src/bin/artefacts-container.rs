//! Build the SDK's ARTEFACTS web container (craftworks-builder#104).
//!
//! usage: artefacts-container <pkg/web> <webapp.wasm> <out> [--tar-only]
//!
//! Packs the four artefacts `artefacts.json` names into a deterministic tar,
//! compresses it with the `xz` CLI (`-6 -T1`, one thread so the output does not
//! depend on the machine's core count), frames it as the node's web container,
//! and writes it to <out>. Prints one JSON object on stdout for build.sh's
//! manifest: the address, the container's sha256, its bytes, and the xz version
//! that made them — the tool is part of what made these bytes.
use std::io::Write;
use std::process::{Command, Stdio};

const FILES: [&str; 4] = ["craftworks_sdk_bg.wasm", "signer.wasm", "block.wasm", "register.wasm"];

fn main() -> Result<(), String> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: artefacts-container <pkg/web> <webapp.wasm> <out> [--tar-only]";
    let (dir, code, out) = (a.first().ok_or(usage)?, a.get(1).ok_or(usage)?, a.get(2).ok_or(usage)?);
    let mut files = Vec::new();
    for f in FILES {
        let p = format!("{dir}/{f}");
        files.push((f, std::fs::read(&p).map_err(|e| format!("{p}: {e}"))?));
    }
    let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    let tar = wire::webapp::tar(&refs)?;
    if a.get(3).map(String::as_str) == Some("--tar-only") {
        std::fs::write(out, &tar).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let mut xz = Command::new("xz")
        .args(["-6", "-T1", "--check=crc64", "-c"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("xz: {e} — the xz CLI is required"))?;
    // FROM A THREAD: writing all of stdin before reading stdout deadlocks as
    // soon as xz's output fills the pipe (it did: a build that hung).
    let mut stdin = xz.stdin.take().expect("piped");
    let feed = std::thread::spawn(move || stdin.write_all(&tar).map(|_| tar));
    let done = xz.wait_with_output().map_err(|e| e.to_string())?;
    let tar = feed.join().map_err(|_| "the xz feeder panicked".to_string())?.map_err(|e| e.to_string())?;
    if !done.status.success() {
        return Err(format!("xz failed: {}", done.status));
    }
    let version = String::from_utf8_lossy(&Command::new("xz").arg("--version").output().map_err(|e| e.to_string())?.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    let meta = br#"{"craftworks":"artefacts"}"#;
    let state = wire::webapp::container(meta, &done.stdout);
    let code = std::fs::read(code).map_err(|e| format!("{code}: {e}"))?;
    std::fs::write(out, &state).map_err(|e| format!("{out}: {e}"))?;
    let sha = {
        use std::fmt::Write as _;
        let d = Command::new("shasum").args(["-a", "256", out]).output().map_err(|e| e.to_string())?;
        let mut s = String::new();
        write!(s, "{}", String::from_utf8_lossy(&d.stdout).split_whitespace().next().unwrap_or("")).unwrap();
        s
    };
    println!(
        r#"{{ "address": "{}", "sha256": "{sha}", "bytes": {}, "tar_bytes": {}, "xz": "{}" }}"#,
        wire::webapp::address(&code, &state),
        state.len(),
        tar.len(),
        version.replace('"', "'")
    );
    Ok(())
}
