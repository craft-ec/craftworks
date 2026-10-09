//! A freenet node this process starts, and will stop.
//!
//! Shaped after `freenet-harness`'s `kill9.rs`, deliberately: that file
//! already worked out how to spawn a private node and reap it, and
//! rediscovering it would have been a second answer to a settled question.
//!
//! It REFUSES the ports other people's nodes are on. A probe that measured
//! someone else's node would produce numbers that look fine and mean nothing.

use anyhow::{bail, Context, Result};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Ports this probe must never touch: the user's own nodes.
pub const RESERVED: &[u16] = &[7509, 7609];

/// The port a node URL names, REFUSED when it is one of [`RESERVED`]: parsed
/// from the URL, never matched as text (a text match on ":7509/" passed
/// "ws://127.0.0.1:7509" straight through). A URL whose port cannot be read
/// is refused too: an unreadable target is never assumed to be someone else's.
pub fn allowed_port(url: &str) -> Result<u16> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let port = authority.rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok()).with_context(|| format!("{url}: no port to check against the owner's nodes"))?;
    if RESERVED.contains(&port) {
        bail!("{url} is the owner's node (port {port}): never touched by a probe");
    }
    Ok(port)
}

/// The `freenet` on PATH, as it names itself (`freenet --version`'s first line): what every live probe prints first, so
/// its result names the release it ran against.
pub fn freenet_version() -> Result<String> {
    let v = Command::new("freenet").arg("--version").output().context("freenet --version")?;
    Ok(String::from_utf8_lossy(&v.stdout).lines().next().unwrap_or_default().to_string())
}

/// EVERY port a run derives, refused BEFORE anything starts if one is someone else's (the architect's review of
/// sdk#444: a base a few below 7509 derives 7509). Each probe keeps its own layout (its offsets, and their overflow);
/// this is the one rule, through [`allowed_port`], so a refusal names the port the same way everywhere.
pub fn allowed_ports(ports: &[u16]) -> Result<()> {
    for port in ports {
        allowed_port(&format!("ws://127.0.0.1:{port}"))?;
    }
    Ok(())
}

const BOOT: Duration = Duration::from_secs(45);

/// How the node is run.
///
/// Not a detail: a delegate-originated PUT is silently dropped in `Local` and
/// works in `Network`, measured both ways on the same binary. Anything that
/// exercises a delegate WRITE must run on a `Network` node, which also means
/// a local-mode node cannot stand in for one in an acceptance run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `freenet local local`.
    Local,
    /// `freenet network --is-gateway --skip-load-from-network`, bound to
    /// loopback.
    ///
    /// `--is-gateway` runs ISOLATED under `--skip-load-from-network`: no
    /// remote gateway index, no on-disk gateway cache, and no `--gateway`
    /// entries, so it connects to nothing. Its network port is 127.0.0.1, so
    /// nothing it does leaves this machine. It is network MODE without being
    /// on the network, which is the only way to ask a delegate-write question
    /// without joining anything.
    IsolatedNetwork { network_port: u16 },
}

/// FLAGS EVERY PROBE NODE RUNS WITH, whatever its mode.
///
/// `--disable-auto-update`: our test nodes are pinned to the release the
/// WORKAROUNDS register is gated on. A bare `freenet network`/`local` run polls
/// for releases, and the day a newer one is published it EXITS with code 42
/// mid-test whenever the poll is not rate-limited (measured, 2026-09-21: every
/// L3 run whose node logged "Update 0.2.136 was detected" was red, and every
/// run whose poll was rate-limited was not). A version bump is the register's
/// process, never a test node's own.
pub const NODE_FLAGS: &[&str] = &["--disable-auto-update"];

/// The command line a probe node is started with. A function, so a test can
/// read it without spawning anything.
pub fn node_args(port: u16, dir: &Path, mode: Mode) -> Vec<String> {
    let mut args: Vec<String> = match mode {
        Mode::Local => vec!["local".into(), "local".into()],
        Mode::IsolatedNetwork { network_port } => {
            vec![
                "network".into(),
                "--is-gateway".into(),
                "--skip-load-from-network".into(),
                // Loopback on both the listen and the advertised address:
                // a gateway that advertises 127.0.0.1 is reachable only
                // from this machine, and with no gateway entries and no
                // index fetch it dials nothing.
                "--network-address".into(),
                "127.0.0.1".into(),
                "--network-port".into(),
                network_port.to_string(),
                "--public-network-address".into(),
                "127.0.0.1".into(),
                "--public-network-port".into(),
                network_port.to_string(),
                "--ws-api-address".into(),
                "127.0.0.1".into(),
            ]
        }
    };
    args.extend([
        "--ws-api-port".to_string(),
        port.to_string(),
        "--data-dir".into(),
        dir.join("data").to_string_lossy().into_owned(),
        "--config-dir".into(),
        dir.join("config").to_string_lossy().into_owned(),
        "--log-dir".into(),
        dir.join("log").to_string_lossy().into_owned(),
    ]);
    if let Ok(level) = std::env::var("PROBE_LOG_LEVEL") {
        args.push("--log-level".into());
        args.push(level);
    }
    args.extend(NODE_FLAGS.iter().map(|f| f.to_string()));
    args
}

/// The command line of a PRIVATE network-mode node that is not the lone
/// gateway `node_args` makes -- live-cold-read's two nodes, one joined to the
/// other through `extra`. Built HERE so it carries the same three dirs and
/// the same NODE_FLAGS: a probe that assembled its own could forget one.
pub fn private_network_args(ws: u16, net: u16, dir: &Path, extra: &[String]) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "network".into(),
        "--skip-load-from-network".into(),
        "--network-address".into(),
        "127.0.0.1".into(),
        "--network-port".into(),
        net.to_string(),
        "--ws-api-address".into(),
        "127.0.0.1".into(),
        "--ws-api-port".into(),
        ws.to_string(),
        "--data-dir".into(),
        dir.join("data").to_string_lossy().into_owned(),
        "--config-dir".into(),
        dir.join("config").to_string_lossy().into_owned(),
        "--log-dir".into(),
        dir.join("log").to_string_lossy().into_owned(),
    ];
    args.extend(extra.iter().cloned());
    args.extend(NODE_FLAGS.iter().map(|f| f.to_string()));
    args
}

/// The node's ENVIRONMENT: its own web-container cache, inside its dir.
///
/// A node serves web containers from ONE per-user cache directory unless
/// `FREENET_WEBAPP_CACHE_DIR` says otherwise (freenet-core 0.2.136,
/// `config.rs` `default_webapp_cache_dir`), and that cache's locks and
/// eviction guards are per-PROCESS. A test node without it unpacks into, and
/// sweeps, the OWNER's node's web cache on this machine: an unpack there made
/// the SDK's artefacts container 404 on two nodes at once, the failure the
/// owner hit. So it is set with the three dirs, by the one door that starts
/// a node.
pub fn node_env(dir: &Path) -> Vec<(&'static str, PathBuf)> {
    vec![("FREENET_WEBAPP_CACHE_DIR", dir.join("webapp_cache"))]
}

pub struct Node {
    child: Option<Child>,
    pub port: u16,
    pub mode: Mode,
    dir: PathBuf,
    /// The command line it was started with -- what `restart` starts again.
    args: Vec<String>,
}

/// THE refusal that keeps every probe off the owner's nodes (sdk#199): a port
/// in `RESERVED`, or one something already listens on, is refused by name.
/// Every door that starts a node asks it, for every port, BEFORE anything is
/// created.
fn refuse(port: u16, what: &str) -> Result<()> {
    if RESERVED.contains(&port) {
        bail!("{what} {port} belongs to someone else's node; refusing to use it");
    }
    if TcpStream::connect(("127.0.0.1", port)).is_ok() {
        bail!("something is already listening on {port}; refusing to share it");
    }
    Ok(())
}

/// THE PRIVATE-NODE MARK: a file every data dir this crate creates carries (holding its spawner's nonce), and the
/// only dirs a store-editing tool (`node-forget`) opens. An ALLOWLIST: the owner's node -- or any node, stopped --
/// passes any list of paths to refuse; it never carries this file.
pub const PRIVATE_MARKER: &str = ".craftworks-private-node";

/// Mark `data_dir` as a private node's: `PRIVATE_MARKER`, holding a nonce (this process, this instant).
pub fn mark_private(data_dir: &Path) -> Result<()> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    std::fs::write(data_dir.join(PRIVATE_MARKER), format!("{}-{nanos}\n", std::process::id()))
        .with_context(|| format!("marking {} private", data_dir.display()))
}

/// Refuse, BY NAME, any `data_dir` that is not a private node's: first the owner's places (a path under /Library, the
/// freenet default dirs) as a second guard, then -- the rule -- a dir without `PRIVATE_MARKER`. Called before
/// anything under it is opened.
pub fn require_private(data_dir: &Path) -> Result<()> {
    let s = data_dir.to_string_lossy();
    if s.contains("/Library/") || s.ends_with("/.local/share/freenet") || s.ends_with("/.cache/freenet") {
        bail!("{s}: not a private node's data dir (the owner's node is never touched)");
    }
    let marker = data_dir.join(PRIVATE_MARKER);
    match std::fs::read_to_string(&marker) {
        Ok(nonce) if !nonce.trim().is_empty() => Ok(()),
        Ok(_) => bail!("{s}: its {PRIVATE_MARKER} is empty -- not a mark this crate wrote; refused"),
        Err(_) => bail!("{s}: no {PRIVATE_MARKER} -- not a data dir a private node was spawned with; refused"),
    }
}

impl Node {
    pub fn spawn(port: u16, dir: &Path) -> Result<Self> {
        Self::spawn_in(port, dir, Mode::Local)
    }

    pub fn spawn_in(port: u16, dir: &Path, mode: Mode) -> Result<Self> {
        refuse(port, "port")?;
        // The network port too — BEFORE anything is created or spawned. It was
        // checked after the directories were made, so a refused network port
        // left a tree behind.
        if let Mode::IsolatedNetwork { network_port } = mode {
            refuse(network_port, "network port")?;
        }
        // The node takes `--log-level`, NOT `RUST_LOG`: setting RUST_LOG in
        // this process changes nothing there, which is how an earlier attempt
        // to read the node's own account of a delegate PUT came back empty
        // and looked like "the node said nothing".
        Self::start(port, dir, mode, node_args(port, dir, mode))
    }

    /// A PRIVATE network-mode node that is not the lone gateway `spawn_in`
    /// makes: live-cold-read's two nodes, one joined to the other through
    /// `extra`. Through the SAME door: both ports refused by `refuse` before
    /// anything is created, the command line from `private_network_args` (its
    /// dirs, NODE_FLAGS). A probe's own copy of the refusal was the line that
    /// keeps probes off the owner's node, untested (sdk#208 review).
    pub fn spawn_private_network(ws: u16, net: u16, dir: &Path, extra: &[String]) -> Result<Self> {
        refuse(ws, "port")?;
        refuse(net, "network port")?;
        Self::start(
            ws,
            dir,
            Mode::IsolatedNetwork { network_port: net },
            private_network_args(ws, net, dir, extra),
        )
    }

    /// Create the node's tree and start it with `args`; ready or an error.
    /// Callers have refused their ports already.
    fn start(port: u16, dir: &Path, mode: Mode, args: Vec<String>) -> Result<Self> {
        for sub in ["data", "config", "log", "webapp_cache"] {
            std::fs::create_dir_all(dir.join(sub))?;
        }
        mark_private(&dir.join("data"))?;
        let child = Command::new("freenet")
            .args(&args)
            .envs(node_env(dir))
            // Captured to the temp tree rather than discarded: the node's
            // FILE log carries no DEBUG lines even at `--log-level debug`, so
            // the only place its own account of a delegate PUT can be is the
            // console. Discarding it is why an earlier attempt concluded "the
            // node said nothing" when nobody had looked where it speaks.
            .stdout(Stdio::from(std::fs::File::create(
                dir.join("log/console.out"),
            )?))
            .stderr(Stdio::from(std::fs::File::create(
                dir.join("log/console.err"),
            )?))
            .spawn()
            .context("spawning `freenet` — is the binary on PATH?")?;
        let mut n = Node {
            child: Some(child),
            port,
            mode,
            dir: dir.to_path_buf(),
            args,
        };
        n.wait_ready()?;
        Ok(n)
    }

    fn wait_ready(&mut self) -> Result<()> {
        // Never PROBE the owner's port either. `refuse` keeps every door off
        // it; this keeps a door that lost its refusal -- a mutant under test,
        // or a future edit -- from connecting to the owner's node to ask
        // whether "it" is ready (a mutant run did, sdk#208: TCP only, nothing
        // sent). Refused by the same words, so a test sees the same reason.
        if RESERVED.contains(&self.port) {
            bail!(
                "port {} belongs to someone else's node; refusing to probe it",
                self.port
            );
        }
        let deadline = Instant::now() + BOOT;
        while Instant::now() < deadline {
            if let Some(c) = self.child.as_mut() {
                if let Ok(Some(status)) = c.try_wait() {
                    bail!("the node exited before it was ready ({status})");
                }
            }
            if TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                std::thread::sleep(Duration::from_millis(300));
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        bail!("the node did not accept connections within {BOOT:?}")
    }

    /// SIGKILL then REAP: killing without waiting leaves a zombie holding the
    /// port against the next spawn.
    pub fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Stop and start again on the same data, in the SAME mode.
    ///
    /// The mode is carried deliberately. Restarting through `Node::spawn`
    /// would come back as `Local` whatever it was before, and a network node
    /// that quietly became a local one mid-run is a probe whose later answers
    /// are about a different system from its earlier ones — with nothing in
    /// the output to say so.
    pub fn restart(&mut self) -> Result<()> {
        let dir = self.dir.clone();
        let port = self.port;
        let mode = self.mode;
        self.stop();
        // Its OWN command line, so a private node joined through `extra`
        // comes back joined, not as the lone gateway its mode also describes.
        let mut fresh = Node::start(port, &dir, mode, self.args.clone())?;
        self.child = fresh.child.take();
        Ok(())
    }

    /// The node process's id WHILE IT IS OURS AND ALIVE, else `None` (after `stop`, or once it exited): a probe that
    /// pauses its OWN node (SIGSTOP, a silent peer) names it by this handle, never by a search. While this handle holds
    /// the child unreaped the pid cannot be reused by another process, so a signal sent by it reaches this node only;
    /// `try_wait` reaps a child that exited, and from then on there is no pid to signal.
    pub fn pid(&mut self) -> Option<u32> {
        let c = self.child.as_mut()?;
        matches!(c.try_wait(), Ok(None)).then(|| c.id())
    }

    pub fn ws(&self) -> String {
        format!(
            "ws://127.0.0.1:{}/v1/contract/command?encodingProtocol=native",
            self.port
        )
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Removes its directory whatever happens, including on an early return.
pub struct TempTree(pub PathBuf);

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod private_mark {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("probe-private-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The allowlist: a dir outside /Library and the default dirs, WITHOUT the mark, is refused by name; marked, it
    /// passes. The /Library refusal holds even for a marked dir (the second guard).
    #[test]
    fn only_a_marked_dir_is_a_private_nodes() {
        let d = scratch("unmarked");
        let e = require_private(&d).unwrap_err().to_string();
        assert!(e.contains(PRIVATE_MARKER), "refused, but not by the mark's name: {e}");
        mark_private(&d).unwrap();
        require_private(&d).expect("a marked dir is a private node's");
        std::fs::write(d.join(PRIVATE_MARKER), "").unwrap();
        assert!(require_private(&d).is_err(), "an empty mark passed");
        let lib = Path::new("/Users/x/Library/Application Support/freenet");
        assert!(require_private(lib).unwrap_err().to_string().contains("owner's node"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
