//! Where the contracts build is, or WHY it cannot be named — the decision
//! build.rs makes, in a file a test can include too (tests/build_contracts.rs).
//! Kept free of cargo: pure inputs, pure answer.

use std::path::{Path, PathBuf};

/// The contracts checkout: `CRAFTWORKS_CONTRACTS` if set, else the one beside
/// the crate — and it must HOLD a build (`build/hashes.toml`). Anything else is
/// an error naming the variable and the path looked at (craftworks-sdk#252):
/// a build that cannot name the contracts it provisions with used to bake
/// "unknown" and exit 0, caught — if at all — by one JS test much later.
pub fn contracts_repo(env: Option<&str>, beside: &Path, has_build: impl Fn(&Path) -> bool) -> Result<PathBuf, String> {
    let (repo, how) = match env {
        Some(p) => (PathBuf::from(p), "CRAFTWORKS_CONTRACTS"),
        None => (beside.to_path_buf(), "the checkout beside this crate (CRAFTWORKS_CONTRACTS is not set)"),
    };
    if has_build(&repo.join("build/hashes.toml")) {
        Ok(repo)
    } else {
        Err(format!(
            "no contracts build at {} — {how}. Set CRAFTWORKS_CONTRACTS to a freenet-contracts checkout that has been built.",
            repo.join("build/hashes.toml").display()
        ))
    }
}
