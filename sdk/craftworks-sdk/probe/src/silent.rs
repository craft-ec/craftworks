//! A PRIVATE NETWORK WITH SILENT PEERS (sdk#447): gateway a, `silent_peers - 1` more nodes joined to a, and b joined
//! to a -- all on loopback, all this process's own. b is checked CONNECTED to every peer, then every peer is
//! SIGSTOPped: a GET b cannot answer locally then gets no answer, deterministically (the real network cannot force
//! silence). The probes `get-in-flight` and `page-get-silent` measure on b's one socket.
//!
//! Its safety is here, once: every port the run derives is refused BEFORE the first spawn if it is someone else's; a
//! peer is signalled only by the handle it was spawned with ([`Node::pid`]); and [`SilentNet::finish`] resumes, kills
//! and reaps every peer and CHECKS it gone -- a paused node never outlives the probe (dropping it without `finish`
//! still resumes and kills them).
use crate::node::{allowed_ports, Node};
use anyhow::{bail, Context, Result};
use freenet_stdlib::client_api::{ClientRequest, HostResponse, NodeQuery, QueryResponse, WebApi};
use std::path::Path;
use std::time::Duration;
use tokio::time::timeout;

/// A probe's `--name value` argument.
pub fn arg(a: &[String], name: &str) -> Option<String> {
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1)).cloned()
}

/// Every port a run on `base` with `silent_peers` uses: a's ws and network, b's, and each other peer's.
pub fn ports(base: u16, silent_peers: u16) -> Result<Vec<u16>> {
    let at = |off: u32| u16::try_from(u32::from(base) + off).context("a derived port is past 65535");
    let mut out = vec![at(0)?, at(1)?, at(10)?, at(11)?];
    for i in 1..u32::from(silent_peers) {
        out.extend([at(20 + 10 * i)?, at(21 + 10 * i)?]);
    }
    Ok(out)
}

fn signal(pid: u32, sig: &str) -> Result<()> {
    if !std::process::Command::new("kill").args([sig, &pid.to_string()]).status()?.success() {
        bail!("kill {sig} {pid} failed");
    }
    Ok(())
}

/// The run's directory: removed on every exit, after b's own dir (its logs, console out/err) is copied to `keep` if
/// one was given.
pub struct Tree {
    root: std::path::PathBuf,
    keep_b: Option<std::path::PathBuf>,
}

impl Drop for Tree {
    fn drop(&mut self) {
        if let Some(keep) = &self.keep_b {
            let _ = std::process::Command::new("cp").arg("-R").arg(self.root.join("b")).arg(keep).status();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A probe's common setup from its arguments (`--base`, default `default_base`; `--silent-peers`, default 1): the
/// run's tree under `dir`, the nodes, and a socket to b, with b connected to every peer (the addresses). Bind as
/// `let (_tree, mut net, mut c, peers) = ...`: dropped in reverse, so the nodes stop before the tree goes.
pub async fn open(a: &[String], dir: &str, default_base: u16, keep_b: Option<&str>) -> Result<(Tree, SilentNet, WebApi, Vec<String>)> {
    let base: u16 = arg(a, "--base").map(|v| v.parse()).transpose()?.unwrap_or(default_base);
    let silent_peers: u16 = arg(a, "--silent-peers").map(|v| v.parse()).transpose()?.unwrap_or(1).max(1);
    let tree = Tree { root: std::path::PathBuf::from(dir), keep_b: keep_b.map(std::path::PathBuf::from) };
    let net = SilentNet::start(&tree.root, base, silent_peers)?;
    let mut c = crate::signer::connect(&net.b.ws()).await?;
    let peers = net.wait_connected(&mut c).await?;
    Ok((tree, net, c, peers))
}

pub struct SilentNet {
    peers: Vec<Node>,
    pub b: Node,
    paused: Vec<u32>,
}

impl SilentNet {
    /// Start the nodes under `root` (each in its own dir). Every derived port is refused first.
    pub fn start(root: &Path, base: u16, silent_peers: u16) -> Result<SilentNet> {
        let ports = ports(base, silent_peers.max(1))?;
        allowed_ports(&ports)?;
        let mut secret = [0u8; 32];
        getrandom::getrandom(&mut secret).expect("random");
        let public = curve25519_dalek::montgomery::MontgomeryPoint::mul_base_clamped(secret).to_bytes();
        std::fs::create_dir_all(root.join("a"))?;
        std::fs::write(root.join("a/transport.key"), core_types::hex::encode(&secret))?;
        let a_extra = [
            "--is-gateway".to_string(),
            "--transport-keypair".into(),
            root.join("a/transport.key").to_string_lossy().into_owned(),
            "--public-network-address".into(),
            "127.0.0.1".into(),
            "--public-network-port".into(),
            ports[1].to_string(),
        ];
        let mut peers = vec![Node::spawn_private_network(ports[0], ports[1], &root.join("a"), &a_extra)?];
        let joined = ["--gateway".to_string(), format!("127.0.0.1:{},{}", ports[1], core_types::hex::encode(&public))];
        for (i, pair) in ports[4..].chunks(2).enumerate() {
            peers.push(Node::spawn_private_network(pair[0], pair[1], &root.join(format!("c{}", i + 1)), &joined)?);
        }
        let b = Node::spawn_private_network(ports[2], ports[3], &root.join("b"), &joined)?;
        Ok(SilentNet { peers, b, paused: Vec::new() })
    }

    /// The GATEWAY's ws URL: a page publishes through it while `b` reads (site_probe: what a page on one node
    /// publishes, read on ANOTHER node).
    pub fn a_ws(&self) -> String {
        self.peers[0].ws()
    }

    /// Wait (up to 90 s) until b, through `c`, is connected to every peer; the addresses it names. Refused if it is
    /// not: a run that paused fewer peers than it says measures something else.
    pub async fn wait_connected(&self, c: &mut WebApi) -> Result<Vec<String>> {
        let wanted = self.peers.len();
        let by = tokio::time::Instant::now() + Duration::from_secs(90);
        let mut peers: Vec<String> = Vec::new();
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            c.send(ClientRequest::NodeQueries(NodeQuery::ConnectedPeers)).await?;
            if let Ok(Ok(HostResponse::QueryResponse(QueryResponse::ConnectedPeers { peers: p }))) = timeout(Duration::from_secs(5), c.recv()).await {
                peers = p.iter().map(|(_, addr)| addr.to_string()).collect();
            }
            if peers.len() >= wanted || tokio::time::Instant::now() > by {
                break;
            }
        }
        if peers.len() < wanted {
            bail!("SETUP: b is connected to {} peers, not the {wanted} this run pauses", peers.len());
        }
        Ok(peers)
    }

    /// SIGSTOP every peer, by its own handle. How many.
    pub fn pause(&mut self) -> Result<usize> {
        for n in &mut self.peers {
            let pid = n.pid().context("a peer is not running")?;
            signal(pid, "-STOP")?;
            self.paused.push(pid);
        }
        Ok(self.paused.len())
    }

    /// Resume, kill and reap every node, and CHECK each paused peer gone. The pids still alive (none, or an error).
    pub fn finish(mut self) -> Result<()> {
        let paused = std::mem::take(&mut self.paused);
        for pid in &paused {
            let _ = signal(*pid, "-CONT");
        }
        self.peers.clear();
        self.b.stop();
        let alive: Vec<u32> = paused.iter().copied().filter(|pid| std::process::Command::new("kill").args(["-0", &pid.to_string()]).status().is_ok_and(|s| s.success())).collect();
        println!("{}", serde_json::json!({ "paused_peers_gone": alive.is_empty(), "still_alive": alive }));
        if !alive.is_empty() {
            bail!("a paused peer outlived the probe: {alive:?}");
        }
        Ok(())
    }
}

impl Drop for SilentNet {
    /// An early exit still resumes every paused peer, so the nodes' own drops kill and reap them.
    fn drop(&mut self) {
        for pid in &self.paused {
            let _ = signal(*pid, "-CONT");
        }
    }
}
