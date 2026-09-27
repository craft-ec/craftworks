//! PUBLISH the Craftworks app on freenet, through a node's signer (the identity it already holds, or, for a rehearsal on a
//! private node, a key given by seed).
//!
//! What goes up:
//! - every PACKAGE as a PIECE SET: k data + 8 parity pieces (the SDK's `pieces::cut`), each its own immutable web
//!   container (served at `/v1/contract/web/<address>/piece`); any k rebuild it. The manifest names each piece by
//!   address and sha256, and the package by its own sha256. The loader races them: the first k to arrive win;
//! - the LOADER as its own SITE `loader` (loader.js, timeline.js, and the SDK's racing parts): signed, versioned;
//! - the APP as its SITE `craftworks` (the wrapper's index.html + boot.js, and manifest.json: its layout — what fills
//!   the header and footer — its pages, and its packages).
//!
//! Sites go through page-io's one site publication (`publish_site`: read, sign the next version, PUT, read back).
//!
//! usage: publish-craftworks <ws_url> <craftworks_root>
//!   env PUBLISH_KEY_SEED=<text>  rehearsal only: provision the node's signer with a key from this seed
use anyhow::{bail, Context, Result};
use freenet_stdlib::prelude::*;
use futures::SinkExt;
use page::server::{Server, SignerFacts};
use page::{Ms, Page, Publication, PutPath};
use page_io::{Artefacts, PageIo};
use probe::live::{next_frame, now_ms};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

fn read(p: &Path) -> Result<Vec<u8>> {
    std::fs::read(p).with_context(|| format!("reading {}", p.display()))
}

fn sha256_hex(b: &[u8]) -> String {
    use sha2::Digest;
    core_types::hex::encode(&sha2::Sha256::digest(b))
}

/// PARITY for every package: m = 8, whatever its size (the owner's). A package is cut into k data pieces of at most
/// PAYLOAD bytes plus M parity pieces; any k of them rebuild it. A package smaller than one piece is k = 1: its 9
/// pieces are 9 ways to the same bytes.
const M: usize = 8;
/// The builder's own piece size.
const PAYLOAD: usize = 96 * 1024;

/// One piece: its container, its state and the address the node names it by.
struct Piece {
    address: String,
    contract: ContractContainer,
    state: Vec<u8>,
}

/// A package as its PIECE SET (`pieces::bundle` then `pieces::cut`, the SDK's one implementation). Nothing is sent
/// here. Returns the pieces and the manifest entry's fields after `kind`.
fn cut_package(webapp_code: &[u8], file: &str, bytes: &[u8]) -> Result<(Vec<Piece>, String)> {
    let bundle = pieces::bundle(&[(file, bytes)]).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let cut = pieces::cut(&bundle, PAYLOAD, M).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut out = Vec::new();
    let mut lines = Vec::new();
    for p in &cut.pieces {
        let state = wire::webapp::piece_container(p).map_err(|e| anyhow::anyhow!(e))?;
        let address = wire::webapp::address(webapp_code, &state);
        let contract = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
            std::sync::Arc::new(ContractCode::from(webapp_code.to_vec())),
            Parameters::from(wire::webapp::params(&state).to_vec()),
        )));
        lines.push(format!(r#"{{ "address": "{address}", "sha256": "{}" }}"#, sha256_hex(p)));
        out.push(Piece { address, contract, state });
    }
    let fields = format!(
        r#""file": "{file}", "sha256": "{}", "k": {}, "m": {}, "payload": {}, "bundle_len": {}, "pieces": [{}]"#,
        sha256_hex(bytes),
        cut.k,
        cut.m,
        cut.payload,
        cut.bundle_len,
        lines.join(", ")
    );
    Ok((out, fields))
}

/// Put every piece, all at once: the node queues them. The socket is SPLIT, so answers are read while PUTs are still
/// going out (a client that sends everything before reading deadlocks: the node stops reading while its answers sit
/// unread). Framing and reading are the SDK's (`wire::frame_put`, `wire::unframe`); answers are matched by the key
/// each names, never by order. A PUT of the same container is the same PUT, so one the node leaves unanswered for
/// 60 s after the last send is sent again, up to TRIES times. Returns the pieces still unanswered: the caller decides
/// whether each package has enough (k of its k + m).
async fn put_all(ws: &str, pieces: &[&Piece]) -> Result<std::collections::HashSet<String>> {
    use futures::StreamExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    const TRIES: u32 = 3;
    let (mut tx, mut rx) = tokio_tungstenite::connect_async(ws).await.context("the pieces' connection")?.0.split();
    // THE SENDER, its own task: frames go out whole and in order (a chunked PUT is several frames), while this task
    // reads answers. It stamps each send, so "unanswered" is counted from the last frame actually sent.
    let t0 = Instant::now();
    let last_sent = std::sync::Arc::new(AtomicU64::new(0));
    let (queue, mut frames) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let stamp = last_sent.clone();
    let sender = tokio::spawn(async move {
        while let Some(f) = frames.recv().await {
            tx.send(Message::Binary(f.into())).await?;
            stamp.store(t0.elapsed().as_millis() as u64, Ordering::Relaxed);
        }
        anyhow::Ok(())
    });
    let mut r = wire::Reassembler::new();
    let mut owed: std::collections::HashMap<String, &Piece> = pieces.iter().map(|p| (p.address.clone(), *p)).collect();
    let mut stream = 1u32;
    let quiet = Duration::from_secs(60);
    for attempt in 1..=TRIES {
        for p in owed.values() {
            stream += 1;
            for f in wire::frame_put(p.contract.clone(), WrappedState::new(p.state.clone()), stream).map_err(|e| anyhow::anyhow!(e))? {
                queue.send(f).context("the sender stopped")?;
            }
        }
        let asked = t0.elapsed();
        while !owed.is_empty() {
            let since = Duration::from_millis(last_sent.load(Ordering::Relaxed)).max(asked);
            let Some(left) = (since + quiet).checked_sub(t0.elapsed()) else { break };
            match tokio::time::timeout(left.min(Duration::from_secs(1)), rx.next()).await {
                Err(_) => continue,
                Ok(None) => bail!("the node closed the pieces' connection"),
                Ok(Some(Err(e))) => bail!("the pieces' connection: {e}"),
                Ok(Some(Ok(Message::Binary(b)))) => match wire::unframe(&mut r, &b) {
                    wire::Incoming::Ack(wire::AckKind::Put(key)) => {
                        owed.remove(&key);
                    }
                    wire::Incoming::PutFailed { key, said } | wire::Incoming::PutFailedByText { key, said } if owed.contains_key(&key) => {
                        bail!("the node refused piece {key}: {said}")
                    }
                    _ => {}
                },
                Ok(Some(Ok(_))) => {}
            }
        }
        if owed.is_empty() {
            break;
        }
        eprintln!("{} piece(s) unanswered 60 s after the last send (try {attempt} of {TRIES})", owed.len());
    }
    drop(queue);
    sender.abort();
    Ok(owed.into_keys().collect())
}

struct Driver {
    sock: probe::live::Sock,
    io: PageIo,
    t0: Instant,
}

impl Driver {
    async fn drive(&mut self, what: &str, budget: Duration, mut done: impl FnMut(&PageIo) -> bool) -> Result<()> {
        let start = Instant::now();
        loop {
            for f in self.io.take_frames() {
                self.sock.send(Message::Binary(f.into())).await.context("send")?;
            }
            let _ = self.io.take_replies();
            if done(&self.io) {
                return Ok(());
            }
            if start.elapsed() > budget {
                bail!("{what}: not within {budget:?} (unusable: {:?})", self.io.unusable());
            }
            if let Some(b) = next_frame(&mut self.sock, &mut self.io, self.t0).await? {
                self.io.inbound(&b, Ms(now_ms(self.t0)));
            }
        }
    }

    async fn publish(&mut self, app: &str, site_code: &[u8], web: Vec<u8>) -> Result<u64> {
        let t = Instant::now();
        self.io.publish_site(app, site_code, web, Ms(now_ms(self.t0))).map_err(|e| anyhow::anyhow!("{app}: {e}"))?;
        self.drive(app, Duration::from_secs(120), |io| !matches!(io.publication(app), Some(Publication::Publishing { .. }) | None)).await?;
        match self.io.publication(app) {
            Some(Publication::Published { version }) => {
                println!("site {app}: published version {version} in {} ms", t.elapsed().as_millis());
                Ok(version)
            }
            other => bail!("site {app} ended {other:?}"),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [ws, root] = a.as_slice() else { bail!("usage: publish-craftworks <ws_url> <craftworks_root>") };
    probe::node::allowed_port(ws)?;
    let app = PathBuf::from(root);
    let contracts = app.join("contracts");
    // The contract code and the frozen signer, exactly the bytes contracts/SHA256SUMS names: a different signer is a
    // different identity, a different contract a different address.
    for line in std::fs::read_to_string(contracts.join("SHA256SUMS"))?.lines() {
        let (want, file) = line.split_once("  ").context("SHA256SUMS: a line that is not '<sha256>  <file>'")?;
        let got = sha256_hex(&read(&contracts.join(file))?);
        if got != want {
            bail!("contracts/{file} hashes to {got}, SHA256SUMS says {want}");
        }
    }
    let signer_wasm = read(&contracts.join("signer.wasm"))?;
    let (webapp_code, site_code) = (read(&contracts.join("webapp.wasm"))?, read(&contracts.join("site.wasm"))?);
    let (block_code, register_code) = (read(&contracts.join("block.wasm"))?, read(&contracts.join("register.wasm"))?);

    // 1. The identity: the node's signer's, or (rehearsal) a key from a seed.
    let t0 = Instant::now();
    let (sock, _) = tokio_tungstenite::connect_async(ws.as_str()).await.context("connecting")?;
    let io = match std::env::var("PUBLISH_KEY_SEED") {
        Ok(seed) => {
            let sk = ed25519_dalek::SigningKey::from_bytes(blake3::hash(seed.as_bytes()).as_bytes());
            probe::page::for_key(&sk, &signer_wasm, block_code, register_code)
        }
        Err(_) => {
            let (container, signer) = wire::delegate_from_code(&signer_wasm);
            let mut io = PageIo::new(
                Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), SignerFacts::default()),
                Artefacts { block_code, register_code, register_params: Vec::new(), signer },
            );
            io.begin(container);
            io
        }
    };
    let mut d = Driver { sock, io, t0 };
    d.drive("the signer", Duration::from_secs(60), |io| io.provisioned() && io.register_params_known()).await?;
    let loader_site = d.io.site_link(&site_code, "loader").context("no loader site for this identity")?;
    let app_site = d.io.site_link(&site_code, "craftworks").context("no craftworks site for this identity")?;
    println!("identity: the signer's; loader site {loader_site}, craftworks site {app_site}");

    // 2. Packages, immutable.
    let built = app.join("packages/build");
    let packages: [(&str, &str, PathBuf); 11] = [
        ("header", "module", app.join("packages/header.js")),
        ("footer", "module", app.join("packages/footer.js")),
        ("home", "module", app.join("packages/home.js")),
        ("account", "module", app.join("packages/account.js")),
        ("node", "service", app.join("packages/node.js")),
        ("identity", "service", app.join("packages/identity.js")),
        ("auth", "service", app.join("packages/auth.js")),
        ("core-glue", "module", built.join("craftworks_core.js")),
        ("core-wasm", "bytes", built.join("craftworks_core_bg.wasm")),
        // The identity delegate's code (the node needs it to run it) and the Register's (a first login puts two).
        ("identity-wasm", "bytes", built.join("identity.wasm")),
        ("register-wasm", "bytes", contracts.join("register.wasm")),
    ];
    let mut entries = Vec::new();
    // WHAT IS ALREADY UP: the piece addresses the live manifest names (PUBLISHED_MANIFEST, the app site's own
    // manifest.json as the node serves it). A package whose pieces are all there is not sent again.
    let published: std::collections::HashSet<String> = match std::env::var("PUBLISHED_MANIFEST") {
        Ok(f) => match std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
            Some(m) => m["packages"]
                .as_object()
                .map(|p| {
                    p.values()
                        .flat_map(|v| v["pieces"].as_array().cloned().unwrap_or_default())
                        .filter_map(|x| x["address"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            None => Default::default(),
        },
        Err(_) => Default::default(),
    };
    let mut all = Vec::new();
    // Per package sent: its name, its k, and its pieces' addresses.
    let mut sent: Vec<(String, usize, Vec<String>)> = Vec::new();
    for (name, kind, path) in &packages {
        let bytes = read(path)?;
        let file = path.file_name().and_then(|n| n.to_str()).context("a package file name")?;
        let (pieces, fields) = cut_package(&webapp_code, file, &bytes)?;
        let up = pieces.iter().all(|p| published.contains(&p.address));
        println!("package {name}: {} B as {} pieces{}", bytes.len(), pieces.len(), if up { " (already published)" } else { "" });
        entries.push(format!(r#"    "{name}": {{ "kind": "{kind}", {fields} }}"#));
        if !up {
            let k = pieces.len() - M;
            sent.push((name.to_string(), k, pieces.iter().map(|p| p.address.clone()).collect()));
            all.extend(pieces);
        }
    }
    let t = Instant::now();
    let missing = put_all(&ws, &all.iter().collect::<Vec<_>>()).await?;
    println!("pieces: {} of {} accepted in {} ms", all.len() - missing.len(), all.len(), t.elapsed().as_millis());
    // RACING needs any k of a package's k + m pieces: a package with fewer accepted cannot be rebuilt, so nothing that
    // names it is published. One with k or more is fine; the rest are said.
    for (name, k, addresses) in &sent {
        let have = addresses.iter().filter(|a| !missing.contains(*a)).count();
        if have < *k {
            bail!("package {name}: only {have} of its {} pieces accepted, fewer than the {k} that rebuild it", addresses.len());
        }
        if have < addresses.len() {
            println!("package {name}: {have} of {} pieces accepted (any {k} rebuild it)", addresses.len());
        }
    }
    let manifest = format!(
        "{{ \"app\": \"Craftworks\",\n  \"layout\": {{ \"header\": [\"header\"], \"footer\": [\"footer\"] }},\n  \"pages\": {{ \"/\": [\"home\"], \"/account\": [\"account\"] }},\n  \"packages\": {{\n{}\n  }} }}\n",
        entries.join(",\n")
    );

    // 3. The loader's site, then the app's (whose wrapper names the loader's site).
    let sdk = app.join("loader/sdk");
    let loader_web = wire::webapp::app_web(&[
        ("loader.js", &read(&app.join("packages/loader.js"))?),
        ("timeline.js", &read(&app.join("packages/timeline.js"))?),
        // The racing parts, the SDK's own (tools/sdk-racing-parts.sh).
        ("served.js", &read(&sdk.join("served.js"))?),
        ("rto.js", &read(&sdk.join("rto.js"))?),
        ("instrument-vocab.js", &read(&sdk.join("instrument-vocab.js"))?),
        ("pieces.js", &read(&sdk.join("pieces.js"))?),
        ("decoder.wasm", &read(&sdk.join("decoder.wasm"))?),
    ])
    .map_err(|e| anyhow::anyhow!(e))?;
    d.publish("loader", &site_code, loader_web).await?;
    let boot = String::from_utf8(read(&app.join("wrapper/boot.js"))?)?;
    if !boot.contains("__LOADER_SITE__") {
        bail!("wrapper/boot.js has no __LOADER_SITE__ to fill in");
    }
    let boot = boot.replace("__LOADER_SITE__", &loader_site);
    let app_web = wire::webapp::app_web(&[
        ("index.html", &read(&app.join("wrapper/index.html"))?),
        ("boot.js", boot.as_bytes()),
        ("manifest.json", manifest.as_bytes()),
    ])
    .map_err(|e| anyhow::anyhow!(e))?;
    d.publish("craftworks", &site_code, app_web).await?;
    println!("OPEN: /v1/contract/web/{app_site}/");
    println!("SITE {app_site}");
    Ok(())
}
