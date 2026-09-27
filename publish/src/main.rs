//! PUBLISH the Craftworks app on freenet, through a node's signer (the identity it already holds, or, for a rehearsal on a
//! private node, a key given by seed).
//!
//! What goes up:
//! - every PACKAGE as an immutable web container (`wire::webapp::piece_container`, served at `/v1/contract/web/<address>/piece`),
//!   named in the app's manifest by that address and its sha256;
//! - the LOADER as its own SITE `loader` (loader.js + timeline.js): signed, versioned, one fixed address;
//! - the APP as its SITE `craftworks` (the wrapper's index.html + boot.js, and manifest.json: its layout — what fills
//!   the header and footer — its pages, and its packages).
//!
//! Sites go through page-io's one site publication (`publish_site`: read, sign the next version, PUT, read back).
//!
//! usage: publish-craftworks <ws_url> <craftworks_root>
//!   env PUBLISH_KEY_SEED=<text>  rehearsal only: provision the node's signer with a key from this seed
use anyhow::{bail, Context, Result};
use freenet_stdlib::client_api::{ClientRequest, ContractRequest, ContractResponse, HostResponse, WebApi};
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

/// Put one immutable package container; returns its address. Waits for the node's answer.
async fn put_package(c: &mut WebApi, webapp_code: &[u8], bytes: &[u8]) -> Result<String> {
    let state = wire::webapp::piece_container(bytes).map_err(|e| anyhow::anyhow!(e))?;
    let address = wire::webapp::address(webapp_code, &state);
    let container = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(webapp_code.to_vec())),
        Parameters::from(wire::webapp::params(&state).to_vec()),
    )));
    c.send(ClientRequest::ContractOp(ContractRequest::Put { contract: container, state: WrappedState::new(state), related_contracts: RelatedContracts::default(), subscribe: false, blocking_subscribe: false })).await?;
    let by = Instant::now() + Duration::from_secs(60);
    while Instant::now() < by {
        match tokio::time::timeout(Duration::from_secs(60), c.recv()).await {
            Ok(Ok(HostResponse::ContractResponse(ContractResponse::PutResponse { key }))) if key.id().encode() == address => return Ok(address),
            Ok(Ok(_)) => continue,
            Ok(Err(e)) => bail!("put {address}: node error: {e}"),
            Err(_) => break,
        }
    }
    bail!("put {address}: no answer within 60 s")
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
    let mut c = WebApi::start(tokio_tungstenite::connect_async(ws.as_str()).await.context("second connection")?.0);
    let built = app.join("packages/build");
    let packages: [(&str, &str, PathBuf); 8] = [
        ("header", "module", app.join("packages/header.js")),
        ("footer", "module", app.join("packages/footer.js")),
        ("home", "module", app.join("packages/home.js")),
        ("who", "module", app.join("packages/who.js")),
        ("node", "service", app.join("packages/node.js")),
        ("core-glue", "module", built.join("craftworks_core.js")),
        ("core-wasm", "bytes", built.join("craftworks_core_bg.wasm")),
        ("signer", "bytes", contracts.join("signer.wasm")),
    ];
    let mut entries = Vec::new();
    // WHAT IS ALREADY UP: the addresses the live manifest names (PUBLISHED_MANIFEST, the app site's own manifest.json as
    // the node serves it). A package at one of them is on the network already: not sent again.
    let published: std::collections::HashSet<String> = match std::env::var("PUBLISHED_MANIFEST") {
        Ok(f) => match std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
            Some(m) => m["packages"].as_object().map(|p| p.values().filter_map(|v| v["address"].as_str().map(String::from)).collect()).unwrap_or_default(),
            None => Default::default(),
        },
        Err(_) => Default::default(),
    };
    for (name, kind, path) in &packages {
        let bytes = read(path)?;
        let t = Instant::now();
        let state = wire::webapp::piece_container(&bytes).map_err(|e| anyhow::anyhow!(e))?;
        let address = if published.contains(&wire::webapp::address(&webapp_code, &state)) {
            let a = wire::webapp::address(&webapp_code, &state);
            println!("package {name}: {} B at {a} (already published)", bytes.len());
            a
        } else {
            let a = put_package(&mut c, &webapp_code, &bytes).await?;
            println!("package {name}: {} B at {a} ({} ms)", bytes.len(), t.elapsed().as_millis());
            a
        };
        entries.push(format!(r#"    "{name}": {{ "kind": "{kind}", "address": "{address}", "sha256": "{}" }}"#, sha256_hex(&bytes)));
    }
    let manifest = format!(
        "{{ \"app\": \"Craftworks\",\n  \"layout\": {{ \"header\": [\"header\"], \"footer\": [\"footer\"] }},\n  \"pages\": {{ \"/\": [\"home\"], \"/me\": [\"who\"] }},\n  \"packages\": {{\n{}\n  }} }}\n",
        entries.join(",\n")
    );

    // 3. The loader's site, then the app's (whose wrapper names the loader's site).
    let loader_web = wire::webapp::app_web(&[
        ("loader.js", &read(&app.join("packages/loader.js"))?),
        ("timeline.js", &read(&app.join("packages/timeline.js"))?),
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
