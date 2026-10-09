//! Bake into the wasm what this build IS.
//!
//! The builder copies a built SDK into its own tree and records the rev it
//! asked for beside it. Nothing can currently check that those two agree,
//! because the rev is not inside the wasm — so a stale copy compares equal to
//! a fresh one. This file is what puts it in.
//!
//! The rev must come from the SOURCE, never from the caller. The builder
//! fetches the SDK with `git archive <rev> | tar -x`, so passing the rev in as
//! an environment variable would make the check circular: both sides of the
//! comparison would come from `SDK_REV` and a stale wasm would still match.
//! `git archive` substitutes `$Format:…$` in files marked `export-subst`, so
//! `src/build_rev.txt` carries the real commit inside an archive and the
//! literal placeholder in an ordinary checkout — where `git rev-parse` answers
//! instead.

use std::{path::Path, process::Command};

#[path = "build_support.rs"]
mod build_support;

fn main() {
    // WHEN TO REGENERATE is the hand-written half of a generated value, and
    // where staleness collects (sdk#134). Once any `rerun-if-*` is printed,
    // cargo watches ONLY what is declared — so every input this script reads
    // is declared, the environment variable included. Without it, pointing
    // `CRAFTWORKS_CONTRACTS` at a checkout (or at a different one) left the
    // wasm reporting the hashes of the build before, `unknown` included.
    println!("cargo:rerun-if-env-changed=CRAFTWORKS_CONTRACTS");
    println!("cargo:rerun-if-changed=src/build_rev.txt");
    println!("cargo:rerun-if-changed=Cargo.lock");
    // The rev is read from git, so git's record of HEAD is an input too.
    // Without it a new COMMIT re-ran nothing, and the wasm went on naming the
    // commit it was first built at — measured: three builds at three HEADs
    // all reported the first one, with no `-dirty` to say so.
    watch_head();
    println!("cargo:rustc-env=SDK_BUILD_REV={}", rev());
    println!("cargo:rustc-env=SDK_PROLLY_REV={}", prolly_rev());
    // The contract hashes this build provisions with, COPIED from the
    // contracts build — never re-hashed here. A consumer that re-hashes the
    // files matches only while its algorithm and its notion of the canonical
    // wasm stay byte-identical to the one script that builds them, and shows
    // an authoritative-looking number that matches nothing when they drift.
    // `hashes.toml` says so itself, and this obeys it.
    let (block, register, webapp, rev) = contract_hashes();
    println!("cargo:rustc-env=SDK_BLOCK_HASH={block}");
    println!("cargo:rustc-env=SDK_WEBAPP_HASH={webapp}");
    println!("cargo:rustc-env=SDK_REGISTER_HASH={register}");
    println!("cargo:rustc-env=SDK_CONTRACTS_REV={rev}");
}

/// `(block, register, rev)` as the contracts build recorded them — or the
/// build FAILS, naming CRAFTWORKS_CONTRACTS and the path it looked at
/// (craftworks-sdk#252; `build_support::contracts_repo`). It used to bake
/// `unknown` and exit 0.
fn contract_hashes() -> (String, String, String, String) {
    let beside = Path::new(env!("CARGO_MANIFEST_DIR")).join("../freenet-contracts");
    let env = std::env::var("CRAFTWORKS_CONTRACTS").ok();
    let repo = match build_support::contracts_repo(env.as_deref(), &beside, |p| p.is_file()) {
        Ok(r) => r,
        Err(why) => {
            // Watch what was looked at, so the build re-runs once it exists.
            watch(&beside.join("build/hashes.toml"));
            panic!("{why}");
        }
    };
    let path = repo.join("build/hashes.toml");
    watch(&path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let field = |name: &str| {
        text.lines()
            .find_map(|l| {
                let (k, v) = l.split_once('=')?;
                (k.trim() == name).then(|| v.trim().trim_matches('"').to_string())
            })
            .unwrap_or_else(|| panic!("{} names no `{name}`", path.display()))
    };
    (field("block"), field("register"), field("webapp"), field("rev"))
}

/// Watch the file this script read. Named explicitly (`CRAFTWORKS_CONTRACTS`)
/// and absent, it is watched anyway: the caller said where the contracts are,
/// so re-running until they appear there is what was asked for.
fn watch(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
}

/// Watch where git keeps HEAD, so a commit re-runs this script.
///
/// Three places, because a commit can land in any of them: this worktree's
/// `HEAD` (a checkout of another branch or a detached commit), the common
/// `refs/heads` DIRECTORY (a commit rewrites the branch's loose ref, or
/// creates it when the branch was packed — cargo scans a directory for any
/// change), and `packed-refs`. Only paths that EXIST are declared: cargo
/// re-runs every build for a declared path that does not exist, which would
/// trade a stale rev for a script that never stops running.
///
/// What this does NOT cover: an UNSTAGED edit changes none of these, so
/// `-dirty` reflects the tree as it was when the script last ran. A rev is a
/// convenience label; the artefact's identity is the sha256 of its bytes,
/// which `artefacts.json` carries and which cannot go stale.
fn watch_head() {
    let Some(head) = git(&["rev-parse", "--path-format=absolute", "--git-path", "HEAD"]) else {
        return; // not a checkout (an archive): `build_rev.txt` names the commit
    };
    let mut paths = vec![std::path::PathBuf::from(head)];
    if let Some(common) = git(&["rev-parse", "--path-format=absolute", "--git-common-dir"]) {
        let common = std::path::PathBuf::from(common);
        paths.push(common.join("refs/heads"));
        paths.push(common.join("packed-refs"));
    }
    for p in paths.iter().filter(|p| p.exists()) {
        watch(p);
    }
}

/// The commit this build came from, or `unknown` when nothing can say.
///
/// `unknown` is a deliberate value, not a fallback that hides a problem: the
/// builder treats it as a mismatch, because a build that cannot name itself is
/// exactly the case the version panel exists to surface.
fn rev() -> String {
    let stamped = std::fs::read_to_string("src/build_rev.txt").unwrap_or_default();
    let stamped = stamped.trim();
    // Inside a `git archive` this is a 40-hex commit. In a checkout it is still
    // the literal placeholder, which starts with `$Format:`.
    if stamped.len() == 40 && stamped.chars().all(|c| c.is_ascii_hexdigit()) {
        return stamped[..7].to_string();
    }
    let Some(head) = git(&["rev-parse", "--short=7", "HEAD"]) else {
        return "unknown".to_string();
    };
    // A rev that names a commit whose code is not what compiled is the
    // confidently-wrong value this whole mechanism exists to prevent.
    match git(&["status", "--porcelain"]) {
        Some(s) if !s.is_empty() => format!("{head}-dirty"),
        Some(_) => head,
        None => format!("{head}-dirty"),
    }
}

/// The `freenet-prolly` rev this build LINKS, read from the lock file.
///
/// From `Cargo.lock` and not `Cargo.toml`: the manifest says what was asked
/// for, the lock says what was resolved, and only one of those was compiled.
fn prolly_rev() -> String {
    let Ok(lock) = std::fs::read_to_string(lock_path()) else {
        return "unknown".to_string();
    };
    let mut in_pkg = false;
    for line in lock.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            in_pkg = false;
        } else if line == "name = \"freenet-prolly\"" {
            in_pkg = true;
        } else if in_pkg {
            if let Some(src) = line.strip_prefix("source = \"") {
                // git+https://…?rev=afcada3#afcada31b3b396d3264455175aef913d5a1ef067
                if let Some(full) = src.rsplit('#').next() {
                    let full = full.trim_end_matches('"');
                    if full.len() >= 7 && full.chars().all(|c| c.is_ascii_hexdigit()) {
                        return full[..7].to_string();
                    }
                }
                return "unknown".to_string();
            }
        }
    }
    "unknown".to_string()
}

/// The lock beside this crate, or the workspace's if this is a member.
fn lock_path() -> std::path::PathBuf {
    let here = Path::new("Cargo.lock");
    if here.exists() {
        return here.to_path_buf();
    }
    Path::new("..").join("Cargo.lock")
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
