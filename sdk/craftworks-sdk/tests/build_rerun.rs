//! The build script re-runs when, and ONLY when, an input it read changed
//! (craftworks-sdk#134).
//!
//! `build.rs` bakes the contract hashes into the wasm (`buildInfo()`), and a
//! generated value is only as current as the hand-written rule deciding when
//! to regenerate it. This drives THE REAL `build.rs` — copied into a
//! throwaway crate that prints what it baked — through cargo, the way a
//! developer's builds go, and reads `cargo -v` to see whether it re-ran.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Probe {
    root: PathBuf,
}

impl Probe {
    /// `<tmp>/sdk` (the crate, with the real build.rs) beside
    /// `<tmp>/freenet-contracts` (created only when a case wants it).
    fn new(tag: &str) -> Probe {
        Probe::printing(tag, "SDK_BLOCK_HASH")
    }

    /// A probe whose program prints the baked value of `var`.
    fn printing(tag: &str, var: &str) -> Probe {
        let root = std::env::temp_dir().join(format!("sdk134-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let krate = root.join("sdk");
        std::fs::create_dir_all(krate.join("src")).unwrap();
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        std::fs::copy(here.join("build.rs"), krate.join("build.rs")).unwrap();
        std::fs::copy(here.join("build_support.rs"), krate.join("build_support.rs")).unwrap();
        std::fs::copy(here.join("src/build_rev.txt"), krate.join("src/build_rev.txt")).unwrap();
        std::fs::write(
            krate.join("Cargo.toml"),
            "[package]\nname = \"sdk134-probe\"\nversion = \"0.0.0\"\nedition = \"2021\"\nbuild = \"build.rs\"\n[workspace]\n",
        )
        .unwrap();
        std::fs::write(krate.join("src/main.rs"), format!("fn main() {{ println!(\"{{}}\", env!(\"{var}\")); }}\n")).unwrap();
        Probe { root }
    }

    fn contracts(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn write_hashes(&self, dir: &Path, block: &str) {
        std::fs::create_dir_all(dir.join("build")).unwrap();
        // A different mtime from any earlier write, whatever the filesystem's
        // resolution: cargo compares mtimes.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(dir.join("build/hashes.toml"), format!("block = \"{block}\"\nregister = \"r\"\nwebapp = \"w\"\nrev = \"v\"\n")).unwrap();
    }

    /// A build that must FAIL — no contracts to name (craftworks-sdk#252):
    /// what it said.
    fn build_refused(&self, contracts_env: Option<&Path>) -> String {
        let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        c.args(["run", "-v"])
            .current_dir(self.root.join("sdk"))
            .env("CARGO_TARGET_DIR", self.root.join("target"))
            .env_remove("CRAFTWORKS_CONTRACTS");
        if let Some(p) = contracts_env {
            c.env("CRAFTWORKS_CONTRACTS", p);
        }
        let out = c.output().expect("cargo runs");
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(!out.status.success(), "a build with no contracts to name SUCCEEDED: {stderr}");
        stderr
    }

    /// Build and run: (what was baked, whether the build script ran).
    fn build(&self, contracts_env: Option<&Path>) -> (String, bool) {
        let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        c.args(["run", "-v"])
            .current_dir(self.root.join("sdk"))
            .env("CARGO_TARGET_DIR", self.root.join("target"))
            .env_remove("CRAFTWORKS_CONTRACTS");
        if let Some(p) = contracts_env {
            c.env("CRAFTWORKS_CONTRACTS", p);
        }
        let out = c.output().expect("cargo runs");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "the probe did not build: {stderr}");
        let ran = stderr.lines().any(|l| l.contains("Running `") && l.contains("build-script-build"));
        // The program prints one line: what was baked.
        let baked = String::from_utf8_lossy(&out.stdout).lines().last().unwrap_or("").trim().to_string();
        (baked, ran)
    }
}

impl Probe {
    /// Run git in the probe crate, as a throwaway repository with its own
    /// identity (no global config is read or needed).
    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "user.name=probe", "-c", "user.email=probe@invalid", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(self.root.join("sdk"))
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// **Case 1: a build with no contracts FAILS, naming the variable and the
/// path (sdk#252 — it baked `unknown` and exited 0); then the contracts named,
/// it bakes their hash.**
#[test]
fn naming_the_contracts_after_a_build_without_them_bakes_their_hash() {
    let p = Probe::new("named");
    let said = p.build_refused(None);
    assert!(said.contains("CRAFTWORKS_CONTRACTS") && said.contains("freenet-contracts/build/hashes.toml"), "{said}");
    let c = p.contracts("contracts");
    p.write_hashes(&c, "sha256:aaa");
    assert_eq!(p.build(Some(&c)).0, "sha256:aaa", "the build kept the hash of a build that had no contracts");
}

/// **Pointing at a DIFFERENT contracts checkout bakes ITS hashes.** RED before.
#[test]
fn pointing_at_another_contracts_checkout_bakes_its_hash() {
    let p = Probe::new("elsewhere");
    let (a, b) = (p.contracts("a"), p.contracts("b"));
    p.write_hashes(&a, "sha256:aaa");
    p.write_hashes(&b, "sha256:bbb");
    assert_eq!(p.build(Some(&a)).0, "sha256:aaa");
    assert_eq!(p.build(Some(&b)).0, "sha256:bbb", "still reporting the other checkout's contracts");
}

/// **Rebuilt contracts are the NEW hash** — the silent, well-formed-but-false
/// case, with the contracts named and with them found beside the crate.
#[test]
fn rebuilt_contracts_are_the_new_hash() {
    let p = Probe::new("rebuilt");
    let c = p.contracts("contracts");
    p.write_hashes(&c, "sha256:old");
    assert_eq!(p.build(Some(&c)).0, "sha256:old");
    p.write_hashes(&c, "sha256:new");
    assert_eq!(p.build(Some(&c)).0, "sha256:new");

    let q = Probe::new("rebuilt-beside");
    let beside = q.contracts("freenet-contracts");
    q.write_hashes(&beside, "sha256:old");
    assert_eq!(q.build(None).0, "sha256:old");
    q.write_hashes(&beside, "sha256:new");
    assert_eq!(q.build(None).0, "sha256:new");
}

/// **The checkout beside the crate BUILDING its contracts after the SDK
/// built** is picked up — the checkout existed, its `build/` did not.
#[test]
fn a_contracts_build_appearing_beside_the_crate_is_picked_up() {
    let p = Probe::new("appears");
    let beside = p.contracts("freenet-contracts");
    std::fs::create_dir_all(&beside).unwrap();
    assert!(p.build_refused(None).contains("no contracts build at"), "an unbuilt checkout beside was taken for a build");
    p.write_hashes(&beside, "sha256:built");
    assert_eq!(p.build(None).0, "sha256:built");
}

/// **THE CONTROLS: with nothing changed, the script does NOT re-run** — with
/// contracts, and without any at all. Or the fix above would simply have made
/// every build re-run (and recompile), which passes every case and costs
/// every build.
#[test]
fn an_untouched_build_does_not_rerun_the_script() {
    let p = Probe::new("still");
    let c = p.contracts("contracts");
    p.write_hashes(&c, "sha256:aaa");
    assert!(p.build(Some(&c)).1, "the first build did not run the script, so the checks below prove nothing");
    assert!(!p.build(Some(&c)).1, "re-ran with nothing changed (contracts named)");

    // No contracts anywhere, or an unbuilt checkout beside: FAILS every time,
    // naming it — never a cached "success" (sdk#252).
    let q = Probe::new("still-none");
    for _ in 0..2 {
        assert!(q.build_refused(None).contains("CRAFTWORKS_CONTRACTS is not set"));
    }
    let r = Probe::new("still-beside-unbuilt");
    std::fs::create_dir_all(r.contracts("freenet-contracts")).unwrap();
    for _ in 0..2 {
        assert!(r.build_refused(None).contains("no contracts build at"));
    }
}

/// **A new COMMIT moves the rev the wasm names** (the label that evidence is
/// filed under). Before this, `build.rs` re-ran only on `build_rev.txt`,
/// `Cargo.lock` or the contracts: three builds at three HEADs all reported
/// the first HEAD, clean, with nothing to say it was stale.
#[test]
fn a_new_commit_moves_the_baked_rev() {
    let p = Probe::printing("rev", "SDK_BUILD_REV");
    let c = p.contracts("contracts");
    p.write_hashes(&c, "sha256:aaa");
    // The crate as a repository of its own; the target dir is outside it.
    p.git(&["init", "-q", "-b", "main"]);
    // The lock file is tracked, as it is in the real repository — otherwise
    // the first build creates it and the tree reads as dirty.
    let lock = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["generate-lockfile"])
        .current_dir(p.root.join("sdk"))
        .output()
        .expect("cargo runs");
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    p.git(&["add", "-A"]);
    p.git(&["commit", "-q", "-m", "one"]);
    let first = p.git(&["rev-parse", "--short=7", "HEAD"]);
    assert_eq!(p.build(Some(&c)).0, first, "THE CONTROL: the first build names its HEAD");

    // An empty commit: HEAD moves, no file the compiler reads does.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    p.git(&["commit", "-q", "--allow-empty", "-m", "two"]);
    let second = p.git(&["rev-parse", "--short=7", "HEAD"]);
    let (baked, ran) = p.build(Some(&c));
    assert!(ran, "a new commit did not re-run the build script");
    assert_eq!(baked, second, "the wasm still names {first} after a commit to {second}");

    // And a branch switch to an older commit moves it back.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    p.git(&["checkout", "-q", "--detach", &first]);
    assert_eq!(p.build(Some(&c)).0, first, "a checkout of an older commit still names the newer one");

    // THE CONTROL that this is not "re-run always": nothing changed, no re-run.
    assert!(!p.build(Some(&c)).1, "the script re-ran with HEAD unchanged");
}
