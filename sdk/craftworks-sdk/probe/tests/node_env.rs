//! Every node a probe starts gets its OWN web-container cache
//! (`FREENET_WEBAPP_CACHE_DIR` inside its dir), never the per-user one the
//! owner's node serves from on this machine. Checked on the environment the
//! spawned PROCESS actually received: a STUB `freenet` first on PATH records
//! it and exits, so nothing real starts.

use probe::node::{node_env, Node};

fn stub_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("node-env-stub-{}", std::process::id()))
}

fn seen() -> std::path::PathBuf {
    stub_dir().join("seen")
}

fn stub_freenet() {
    let bin = stub_dir();
    std::fs::create_dir_all(&bin).expect("stub dir");
    let f = bin.join("freenet");
    std::fs::write(
        &f,
        format!(
            "#!/bin/sh\nprintf '%s' \"${{FREENET_WEBAPP_CACHE_DIR-UNSET}}\" > '{}'\nexit 1\n",
            seen().display()
        ),
    )
    .expect("stub");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{path}", bin.display()));
}

/// A port nothing listens on right now (the OS picks it).
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port()
}

#[test]
fn a_spawned_node_gets_its_own_web_cache_inside_its_dir() {
    stub_freenet();
    let dir = std::env::temp_dir().join(format!("node-env-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(seen());
    // The stub exits at once, so the spawn itself fails: what it was GIVEN is the point.
    let r = Node::spawn(free_port(), &dir);
    assert!(r.is_err(), "the stub cannot serve; a spawn that succeeded ran something else");
    let got = std::fs::read_to_string(seen()).expect("the stub never ran: nothing was spawned");
    assert_ne!(got, "UNSET", "the node was started with no FREENET_WEBAPP_CACHE_DIR: it shares the per-user web cache");
    assert_eq!(std::path::PathBuf::from(&got), dir.join("webapp_cache"), "the node's web cache is not inside its own dir");
    assert!(dir.join("webapp_cache").is_dir(), "the node's web cache dir was not created");
    assert_eq!(node_env(&dir), vec![("FREENET_WEBAPP_CACHE_DIR", dir.join("webapp_cache"))]);
    let _ = std::fs::remove_dir_all(&dir);
}
