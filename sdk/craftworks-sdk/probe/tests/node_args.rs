//! A test node's command line: every one carries its own three dirs and the
//! NODE_FLAGS, whichever builder made it (the owner's rule after a node logged
//! into the owner's directory; `--disable-auto-update` after test nodes exited
//! with code 42 the day 0.2.136 was released).
use probe::node::{node_args, private_network_args, Mode, NODE_FLAGS};
use std::path::Path;

fn check(what: &str, args: &[String], dir: &Path) {
    for (flag, sub) in [
        ("--data-dir", "data"),
        ("--config-dir", "config"),
        ("--log-dir", "log"),
    ] {
        let at = args
            .iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("{what}: no {flag} in {args:?}"));
        assert_eq!(
            args.get(at + 1).map(String::as_str),
            Some(dir.join(sub).to_string_lossy().as_ref()),
            "{what}: {flag} does not point inside the node's own tree"
        );
    }
    assert!(
        args.iter().any(|a| a == "--disable-auto-update"),
        "{what}: a test node without --disable-auto-update exits with code 42 the day a release appears: {args:?}"
    );
    for f in NODE_FLAGS {
        assert!(
            args.iter().any(|a| a == f),
            "{what}: NODE_FLAGS' {f} is missing"
        );
    }
}

#[test]
fn every_test_node_command_line_has_its_dirs_and_the_node_flags() {
    let dir = Path::new("/nonexistent/probe-node");
    check("local", &node_args(17_000, dir, Mode::Local), dir);
    check(
        "isolated network",
        &node_args(
            17_000,
            dir,
            Mode::IsolatedNetwork {
                network_port: 17_001,
            },
        ),
        dir,
    );
    check(
        "private network, joined",
        &private_network_args(
            17_010,
            17_011,
            dir,
            &["--gateway".into(), "127.0.0.1:17001,ab".into()],
        ),
        dir,
    );
}
