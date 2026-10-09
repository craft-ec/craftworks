//! THE CONTROL for the Register's format (sdk#364; the owner: "control so that things won't happen"): a Register
//! state or params are READ only by the Register crate (`craftec-register-contract`: `read`, `wire::record_head`),
//! the code every node validates with. The RG01 magic or a hand-written piece of its layout anywhere else in the
//! workspace's non-test sources fails this test, so a new copy cannot merge (the copy it replaces refused a state
//! with fork evidence, F56). ONE writer is excepted, counted exactly and printed: `contract_keys::register_params` (re-exported by `wire`) (the crate
//! parses params but has no encoder; `wire/tests/register_params.rs` round-trips it through the crate's parser).
//!
//! Test sources are not scanned: they build hostile and foreign states ON PURPOSE (a forged record, known-answer
//! vectors, the artefact gate's independent conformance check).
//!
//! And the crate's FEATURES: a crate's NORMAL dependency on it takes the LIBRARY (`default-features = false`, from
//! the one pin in the workspace root), never the contract (`freenet-main-contract`), which brings freenet-stdlib --
//! except `testkit`, the test fixtures, already excused by probe's client-API allowlist.

use std::path::{Path, PathBuf};

/// The one writer, and the exact number of its lines this scan matches.
const EXCEPT: &[(&str, usize)] = &[("contract-keys/src/lib.rs", 1)];

/// A line that names the Register's magic or lays out its format by hand (a TRIPWIRE for the shapes known so far,
/// each with a control line below).
fn hand_written(l: &str) -> bool {
    let magic = l.contains("\"RG01") || l.contains("b'R', b'G', b'0', b'1'") || l.contains("[82, 71, 48, 49]");
    let crate_magic = l.contains("wire::MAGIC") || l.contains("RECORD_MAGIC");
    let flags = l.contains("FLAG_RECORD") || l.contains("FLAG_EVIDENCE");
    let sig_domain = l.contains("RG01-sig") || l.contains("SIG_DOMAIN");
    magic || crate_magic || flags || sig_domain
}

fn copies(text: &str) -> Vec<String> {
    text.lines().filter(|l| !l.trim_start().starts_with("//")).filter(|l| hand_written(l)).map(str::to_string).collect()
}

/// Non-test Rust sources: every `src/`, `examples/` and `build.rs` of the workspace.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if !matches!(name, "target" | "node_modules" | "pkg" | "tests" | "benches") && !name.starts_with('.') {
                sources(&p, out);
            }
        } else if name.ends_with(".rs") {
            out.push(p);
        }
    }
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the workspace").to_path_buf()
}

#[test]
fn the_register_format_is_read_only_by_the_register_crate() {
    let root = workspace();
    let mut files = Vec::new();
    sources(&root, &mut files);
    assert!(files.len() > 60, "the scan found {} sources: it is not looking at the workspace", files.len());
    let mut found = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(&root).unwrap_or(f).to_string_lossy().to_string();
        let hits = copies(&std::fs::read_to_string(f).unwrap_or_default());
        match EXCEPT.iter().find(|(p, _)| *p == rel) {
            Some((_, n)) => {
                println!("excepted: {rel}: {} line(s), {n} allowed (the one params writer)", hits.len());
                if hits.len() != *n {
                    found.push(format!("{rel}: {} lines where {n} are excepted:\n    {}", hits.len(), hits.join("\n    ")));
                }
            }
            None => found.extend(hits.into_iter().map(|l| format!("{rel}: {}", l.trim()))),
        }
    }
    assert!(found.is_empty(), "the Register's format written outside the Register crate (read it with craftec_register_contract::read / wire::record_head):\n{}", found.join("\n"));
}

#[test]
fn the_detector_catches_each_copy_shape() {
    for caught in [
        r#"    let rest = state.strip_prefix(b"RG01")?;"#,
        r#"    let mut p = Vec::from(*b"RG01");"#,
        r#"pub const RECORD_MAGIC: &[u8; 4] = b"RG01";"#,
        r#"    if flags & FLAG_RECORD == 0 {"#,
        r#"const SIG_DOMAIN: &[u8; 8] = b"RG01-sig";"#,
        r#"    let mut p = Vec::from(*craftec_register_contract::wire::MAGIC);"#,
        r#"    if s.starts_with(&[82, 71, 48, 49]) {"#,
    ] {
        assert_eq!(copies(caught).len(), 1, "a copy shape was not caught: {caught}");
    }
    for clean in [
        "    let (_t, seq, value) = craftec_register_contract::wire::record_head(state)?;",
        "    let (_, s) = craftec_register_contract::read(params, state)?;",
        r#"    // b"RG01" in a comment is not code"#,
    ] {
        assert!(copies(clean).is_empty(), "the call it points to was flagged: {clean}");
    }
}

/// Section-aware: the `[dependencies]` lines naming the register crate in each member's manifest.
fn normal_dep_lines(manifest: &str) -> Vec<String> {
    let mut section = String::new();
    let mut out = Vec::new();
    for l in manifest.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            section = t.to_string();
        } else if section == "[dependencies]" && t.starts_with("craftec-register-contract") {
            out.push(t.to_string());
        }
    }
    out
}

#[test]
fn a_normal_dependency_on_the_register_crate_is_the_library_never_the_contract() {
    let root = workspace();
    let top = std::fs::read_to_string(root.join("Cargo.toml")).expect("the workspace manifest");
    let pin: Vec<&str> = top.lines().filter(|l| l.trim_start().starts_with("craftec-register-contract")).collect();
    assert_eq!(pin.len(), 1, "the register crate is not pinned ONCE in the workspace root: {pin:?}");
    assert!(pin[0].contains("default-features = false"), "the one pin takes the contract, not the library: {}", pin[0]);
    let mut members = 0;
    let mut bad = Vec::new();
    for e in std::fs::read_dir(&root).expect("the workspace").flatten() {
        let m = e.path().join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&m) else { continue };
        members += 1;
        let name = e.file_name().to_string_lossy().to_string();
        for l in normal_dep_lines(&text) {
            if !l.contains("workspace = true") {
                bad.push(format!("{name}: a second pin, not the workspace's: {l}"));
            } else if l.contains("freenet-main-contract") && name != "testkit" {
                bad.push(format!("{name}: a NORMAL dependency on the CONTRACT (freenet-stdlib): {l}"));
            }
        }
    }
    assert!(members > 10, "the scan read {members} member manifests");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn the_manifest_check_catches_the_contract_as_a_normal_dependency() {
    let bad = "[dependencies]\ncraftec-register-contract = { workspace = true, features = [\"freenet-main-contract\"] }\n";
    assert_eq!(normal_dep_lines(bad).len(), 1);
    let dev = "[dev-dependencies]\ncraftec-register-contract = { workspace = true, features = [\"freenet-main-contract\"] }\n";
    assert!(normal_dep_lines(dev).is_empty(), "a dev-dependency was read as a normal one");
}
