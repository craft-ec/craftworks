//! build.rs FAILS, by name, when it cannot find the contracts build
//! (craftworks-sdk#252) — the decision it makes, driven directly.
#[path = "../build_support.rs"]
mod build_support;
use build_support::contracts_repo;
use std::path::Path;

#[test]
fn no_contracts_build_is_an_error_naming_the_variable_and_the_path() {
    let e = contracts_repo(None, Path::new("/nowhere/freenet-contracts"), |_| false).unwrap_err();
    assert!(e.contains("CRAFTWORKS_CONTRACTS is not set") && e.contains("/nowhere/freenet-contracts/build/hashes.toml"), "{e}");
    let e = contracts_repo(Some("/set/but/unbuilt"), Path::new("/x"), |_| false).unwrap_err();
    assert!(e.contains("/set/but/unbuilt/build/hashes.toml") && e.contains("CRAFTWORKS_CONTRACTS"), "{e}");
}

#[test]
fn control_a_built_checkout_is_found_the_named_one_first() {
    assert_eq!(contracts_repo(Some("/named"), Path::new("/beside"), |_| true).unwrap(), Path::new("/named"));
    assert_eq!(contracts_repo(None, Path::new("/beside"), |p| p.starts_with("/beside")).unwrap(), Path::new("/beside"));
}
