//! PUBLISH the Craftworks app on freenet, through a node's signer (the identity it already holds, or, for a rehearsal on a
//! private node, a key given by seed).
//!
//! What goes up:
//! - every PACKAGE as a PIECE SET: k data + 8 parity pieces (the SDK's `pieces::cut`), each its own immutable web
//!   container (served at `/v1/contract/web/<address>/piece`); any k rebuild it. The manifest names each piece by
//!   address and sha256, and the package by its own sha256. The loader races them: the first k to arrive win;
//! - the LOADER as its own SITE `loader` (loader.js, trace.js, and the SDK's racing parts): signed, versioned;
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

/// A delegate build as the page addresses it: `<key>:<code hash>`, both base58.
fn delegate_id(wasm: &[u8]) -> String {
    let (_, key) = wire::delegate_from_code(wasm);
    format!("{}:{}", key.encode(), key.code_hash().encode())
}

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
/// 60 s after the last send is sent again, up to TRIES times, unless `enough` says the pieces still unanswered no
/// longer matter. Returns the pieces still unanswered.
async fn put_all(
    ws: &str,
    pieces: &[&Piece],
    enough: impl Fn(&std::collections::HashSet<String>) -> bool,
) -> Result<std::collections::HashSet<String>> {
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
    // REFUSED pieces: a peer on the way would not store one (a full peer's disk budget, say). A piece is one of a
    // package's n, any k of which rebuild it, so a refusal is a missing piece like an unanswered one: tried again
    // next round, and the package is judged by the k it needs, never by one peer.
    let mut refused: std::collections::HashMap<String, &Piece> = Default::default();
    for attempt in 1..=TRIES {
        owed.extend(refused.drain());
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
                        eprintln!("piece {key} refused: {said}");
                        if let Some(p) = owed.remove(&key) {
                            refused.insert(key, p);
                        }
                    }
                    _ => {}
                },
                Ok(Some(Ok(_))) => {}
            }
        }
        if owed.is_empty() && refused.is_empty() {
            break;
        }
        eprintln!("{} piece(s) unanswered 60 s after the last send, {} refused (try {attempt} of {TRIES})", owed.len(), refused.len());
        // Every package can be rebuilt already: the rest are spares, not worth another minute.
        if enough(&owed.keys().chain(refused.keys()).cloned().collect()) {
            break;
        }
    }
    drop(queue);
    sender.abort();
    Ok(owed.into_keys().chain(refused.into_keys()).collect())
}

async fn follow(ws: &str, sites: &[String]) -> Result<()> {
    use futures::{SinkExt, StreamExt};
    let (mut tx, mut rx) = tokio_tungstenite::connect_async(ws).await.context("connecting")?.0.split();
    let mut r = wire::Reassembler::new();
    for (i, site) in sites.iter().enumerate() {
        let id: [u8; 32] = *ContractInstanceId::from_base58(site).map_err(|e| anyhow::anyhow!("{site}: not a site id: {e}"))?;
        let t = Instant::now();
        for f in wire::frame_get(wire::contract_id(id), true, i as u32 + 2).map_err(|e| anyhow::anyhow!(e))? {
            tx.send(Message::Binary(f.into())).await?;
        }
        let said = loop {
            match tokio::time::timeout(Duration::from_secs(120), rx.next()).await {
                Err(_) => break "no answer in 120 s".to_string(),
                Ok(None) => bail!("the node closed the connection"),
                Ok(Some(Err(e))) => bail!("the connection: {e}"),
                Ok(Some(Ok(Message::Binary(b)))) => match wire::unframe(&mut r, &b) {
                    wire::Incoming::Got { state, .. } => break format!("got {} B", state.len()),
                    wire::Incoming::GetFailed { why, .. } => break format!("get failed: {why:?}"),
                    _ => {}
                },
                Ok(Some(Ok(_))) => {}
            }
        };
        println!("follow {site}: {said} in {} ms", t.elapsed().as_millis());
    }
    Ok(())
}

async fn relay(from: &str, to: &str, sites: &[String]) -> Result<()> {
    use freenet_stdlib::client_api::{ClientRequest, ContractRequest, ContractResponse, HostResponse};
    use futures::{SinkExt, StreamExt};
    let (mut ftx, mut frx) = tokio_tungstenite::connect_async(from).await.context("connecting to the source")?.0.split();
    let (mut ttx, mut trx) = tokio_tungstenite::connect_async(to).await.context("connecting to the target")?.0.split();
    for (i, site) in sites.iter().enumerate() {
        let key = ContractInstanceId::from_base58(site).map_err(|e| anyhow::anyhow!("{site}: not a site id: {e}"))?;
        let req = ClientRequest::ContractOp(ContractRequest::Get { key, return_contract_code: true, subscribe: false, blocking_subscribe: false });
        ftx.send(Message::Binary(bincode::serialize(&req)?.into())).await?;
        let mut r = wire::Reassembler::new();
        let (contract, state) = loop {
            let m = tokio::time::timeout(Duration::from_secs(120), frx.next()).await.context("the source did not answer in 120 s")?;
            let Some(Ok(Message::Binary(b))) = m else { continue };
            let resp = match wire::Reassembler::decode(&b) {
                Ok(HostResponse::StreamChunk { stream_id, index, total, data }) => match r.chunk(stream_id, index, total, data.to_vec()) {
                    Ok(Some(whole)) => wire::Reassembler::decode(&whole).map_err(|e| anyhow::anyhow!("{e:?}"))?,
                    Ok(None) => continue,
                    Err(e) => bail!("a broken stream from the source: {e:?}"),
                },
                Ok(x) => x,
                Err(e) => bail!("the source said: {e:?}"),
            };
            if let HostResponse::ContractResponse(ContractResponse::GetResponse { contract: Some(c), state, .. }) = resp {
                break (c, state);
            }
        };
        let bytes = state.as_ref().len();
        for f in wire::frame_put(contract, state, i as u32 + 2).map_err(|e| anyhow::anyhow!(e))? {
            ttx.send(Message::Binary(f.into())).await?;
        }
        let mut r = wire::Reassembler::new();
        let said = loop {
            match tokio::time::timeout(Duration::from_secs(120), trx.next()).await {
                Err(_) => break "no answer in 120 s".to_string(),
                Ok(None) => bail!("the target closed the connection"),
                Ok(Some(Err(e))) => bail!("the target: {e}"),
                Ok(Some(Ok(Message::Binary(b)))) => match wire::unframe(&mut r, &b) {
                    wire::Incoming::Ack(wire::AckKind::Put(_)) => break "taken".to_string(),
                    wire::Incoming::PutFailed { said, .. } | wire::Incoming::PutFailedByText { said, .. } => break format!("refused: {said}"),
                    _ => {}
                },
                Ok(Some(Ok(_))) => {}
            }
        };
        println!("relay {site}: {bytes} B, {said}");
    }
    Ok(())
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
    if let [flag, wasm] = a.as_slice() {
        if flag == "--delegate-key" {
            println!("{}", delegate_id(&read(Path::new(wasm))?));
            return Ok(());
        }
    }
    // FOLLOW: a node that holds a site answers a plain GET from its copy; a GET that SUBSCRIBES has it ask the site's
    // peers and keep following it. Only GETs are sent — nothing is written — so this is the one mode allowed on the
    // owner's node, when the owner asks for it (a stale copy of their site).
    // RELAY: a site's current version, as `from` has it (its contract and signed state), PUT into `to` — for a node
    // holding an old copy that following did not bring up to date. The site contract checks the state's signature, so
    // only the real publication is taken.
    if a.first().map(String::as_str) == Some("relay") {
        let [_, from, to, sites @ ..] = a.as_slice() else { bail!("usage: publish-craftworks relay <from_ws> <to_ws> <site id>…") };
        return relay(from, to, sites).await;
    }
    if a.first().map(String::as_str) == Some("follow") {
        let [_, ws, sites @ ..] = a.as_slice() else { bail!("usage: publish-craftworks follow <ws_url> <site id>…") };
        return follow(ws, sites).await;
    }
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
    // SITE_NAME: publish the same app at another address too (a second front end over the same account data); the
    // default is the app's own site.
    let site_name = std::env::var("SITE_NAME").unwrap_or_else(|_| "craftworks".into());
    let app_site = d.io.site_link(&site_code, &site_name).context("no site of that name for this identity")?;
    println!("identity: the signer's; loader site {loader_site}, craftworks site {app_site}");

    // 2. Packages, immutable.
    let built = app.join("packages/build");
    let packages: [(&str, &str, PathBuf); 79] = [
        // The look: design tokens and base styles, applied by the loader before anything mounts.
        ("theme", "service", app.join("packages/theme.js")),
        ("header", "module", app.join("packages/header.js")),
        ("footer", "module", app.join("packages/footer.js")),
        ("home", "module", app.join("packages/home.js")),
        ("account", "module", app.join("packages/account.js")),
        ("node", "service", app.join("packages/node.js")),
        ("identity", "service", app.join("packages/identity.js")),
        ("auth", "service", app.join("packages/auth.js")),
        ("space", "service", app.join("packages/space.js")),
        ("login", "service", app.join("packages/login.js")),
        ("access", "service", app.join("packages/access.js")),
        // The account's MLS group on this node: the source of every table key.
        // One agreed order of entries per object (several types: tail now; log, witnessed later).
        ("ordering", "service", app.join("packages/ordering.js")),
        ("keys", "service", app.join("packages/keys.js")),
        // Who belongs: your account's nodes (its MLS group's members), and later spaces' members.
        ("membership", "service", app.join("packages/membership.js")),
        // MLS: its own wasm package, loaded only by `keys`.
        ("mls-glue", "module", built.join("craftworks_mls.js")),
        ("mls-wasm", "bytes", built.join("craftworks_mls_bg.wasm")),
        ("feed-glue", "module", built.join("craftworks_feed.js")),
        ("feed-wasm", "bytes", built.join("craftworks_feed_bg.wasm")),
        ("core-glue", "module", built.join("craftworks_core.js")),
        ("core-wasm", "bytes", built.join("craftworks_core_bg.wasm")),
        // The identity delegate's code (the node needs it to run it).
        ("identity-wasm", "bytes", built.join("identity.wasm")),
        // The account's key event log (its DID names it) and the Register (a set of words' whoami).
        ("idlog-wasm", "bytes", contracts.join("idlog.wasm")),
        ("register-wasm", "bytes", contracts.join("register.wasm")),
        // The data: each node's rows for this app, as its own tail.
        // Tree blocks: the one door (fetch raced against parity, put).
        ("blocks", "service", app.join("packages/blocks.js")),
        ("storage", "service", app.join("packages/storage.js")),
        // EDGE (pins, labels) and its two components.
        ("edge", "service", app.join("packages/edge.js")),
        ("pin-button", "service", app.join("packages/pin-button.js")),
        ("label-menu", "service", app.join("packages/label-menu.js")),
        ("notes", "module", app.join("packages/notes.js")),
        ("content", "service", app.join("packages/content.js")),
        ("directory", "service", app.join("packages/directory.js")),
        ("index", "service", app.join("packages/index.js")),
        ("conversation", "service", app.join("packages/conversation.js")),
        ("roles", "service", app.join("packages/roles.js")),
        ("moderation", "service", app.join("packages/moderation.js")),
        ("server-settings", "service", app.join("packages/server-settings.js")),
        ("person", "service", app.join("packages/person.js")),
        ("people-list", "service", app.join("packages/people-list.js")),
        ("activity", "service", app.join("packages/activity.js")),
        ("recovery", "service", app.join("packages/recovery.js")),
        ("contacts", "module", app.join("packages/contacts.js")),
        ("items", "service", app.join("packages/items.js")),
        ("board", "module", app.join("packages/board.js")),
        ("rail", "module", app.join("packages/rail.js")),
        ("upkeep", "service", app.join("packages/upkeep.js")),
        ("app-icons", "service", app.join("packages/app-icons.js")),
        ("join-button", "service", app.join("packages/join-button.js")),
        ("files", "service", app.join("packages/files.js")),
        ("file-keys", "service", app.join("packages/file-keys.js")),
        ("keep", "service", app.join("packages/keep.js")),
        ("attachments", "service", app.join("packages/attachments.js")),
        ("markdown", "service", app.join("packages/markdown.js")),
        ("md-editor", "service", app.join("packages/md-editor.js")),
        ("media-view", "service", app.join("packages/media-view.js")),
        ("comments", "service", app.join("packages/comments.js")),
        ("drive-store", "service", app.join("packages/drive-store.js")),
        ("drive", "module", app.join("packages/drive.js")),
        ("kinds", "service", app.join("packages/kinds.js")),
        ("audience", "service", app.join("packages/audience.js")),
        ("image-studio", "service", app.join("packages/image-studio.js")),
        ("feed-bar", "service", app.join("packages/feed-bar.js")),
        ("video-player", "service", app.join("packages/video-player.js")),
        ("video-studio", "service", app.join("packages/video-studio.js")),
        // One page for every media app (Videos, Audio): its route chooses the domain.
        ("media", "module", app.join("packages/media.js")),
        ("subtitle-store", "service", app.join("packages/subtitle-store.js")),
        ("subtitles", "module", app.join("packages/subtitles.js")),
        ("app-settings", "service", app.join("packages/app-settings.js")),
        ("space-home", "module", app.join("packages/space-home.js")),
        ("chat", "module", app.join("packages/chat.js")),
        ("messages", "module", app.join("packages/messages.js")),
        ("mail", "module", app.join("packages/mail.js")),
        ("room", "service", app.join("packages/room.js")),
        ("tail-wasm", "bytes", contracts.join("tail.wasm")),
        // The Block contract: a table's tree blocks, after a flush.
        ("block-wasm", "bytes", contracts.join("block.wasm")),
        ("sealed-wasm", "bytes", contracts.join("sealed.wasm")),
        // The Piece contract: a file's pieces since burning.
        ("piece-wasm", "bytes", contracts.join("piece.wasm")),
        // mp4box.js (vendor/mp4box): run by `video-player` to stream an MP4 by range.
        ("mp4box", "bytes", app.join("vendor/mp4box/mp4box.all.min.js")),
        // Mediabunny (vendor/mediabunny, MPL-2.0): run by `video-studio` to make a video's renditions.
        ("mediabunny", "bytes", app.join("vendor/mediabunny/mediabunny.min.mjs")),
        ("bag-wasm", "bytes", contracts.join("bag.wasm")),
    ];
    let mut entries = Vec::new();
    // WHAT IS ALREADY UP: the piece addresses the live manifest names (PUBLISHED_MANIFEST, the app site's own
    // manifest.json as the node serves it). A package whose pieces are all there is not sent again.
    let published: std::collections::HashSet<String> = match std::env::var("PUBLISHED_MANIFEST") {
        Ok(f) => match std::fs::read(&f).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
            // By the package's own hash: its pieces are a function of its bytes, so a live package with the same hash
            // has every one of them up already.
            Some(m) => m["packages"]
                .as_object()
                .map(|p| p.values().filter_map(|v| v["sha256"].as_str().map(String::from)).collect())
                .unwrap_or_default(),
            None => Default::default(),
        },
        Err(_) => Default::default(),
    };
    let mut all = Vec::new();
    // Per package sent: its name, its k, and its pieces' addresses.
    let mut sent: Vec<(String, usize, Vec<String>)> = Vec::new();
    // Each package's ENTRY (its pieces), as its own file of the site named by its hash: a page reads only the ones it
    // needs. The manifest names each by kind and hash alone.
    let mut entry_files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut sources: Vec<(String, String)> = Vec::new();
    for (name, kind, path) in &packages {
        let bytes = read(path)?;
        let file = path.file_name().and_then(|n| n.to_str()).context("a package file name")?;
        let (pieces, fields) = cut_package(&webapp_code, file, &bytes)?;
        let sha = sha256_hex(&bytes);
        let up = published.contains(&sha);
        println!("package {name}: {} B as {} pieces{}", bytes.len(), pieces.len(), if up { " (already published)" } else { "" });
        entries.push(format!(r#"    "{name}": {{ "kind": "{kind}", "sha256": "{sha}" }}"#));
        entry_files.push((format!("p/{}.json", &sha[..16]), format!(r#"{{ "kind": "{kind}", {fields} }}"#).into_bytes()));
        if *kind != "bytes" {
            sources.push((name.to_string(), String::from_utf8_lossy(&bytes).into_owned()));
        }
        if !up {
            let k = pieces.len() - M;
            sent.push((name.to_string(), k, pieces.iter().map(|p| p.address.clone()).collect()));
            all.extend(pieces);
        }
    }
    let t = Instant::now();
    let rebuildable = |missing: &std::collections::HashSet<String>| {
        sent.iter().all(|(_, k, a)| a.iter().filter(|x| !missing.contains(*x)).count() >= *k)
    };
    let missing = put_all(&ws, &all.iter().collect::<Vec<_>>(), rebuildable).await?;
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
    // EARLIER IDENTITY BUILDS: each build is a new delegate, and the members on a node stay in the build that made
    // them. The page asks these (newest first) to hand a member over on a PIN its own build does not know. The list
    // is contracts/identity-history, one build per line, appended here when this build is new.
    let history_file = contracts.join("identity-history");
    let this_build = delegate_id(&read(&built.join("identity.wasm"))?);
    let mut history: Vec<String> = std::fs::read_to_string(&history_file).unwrap_or_default().lines().map(str::to_string).filter(|l| !l.is_empty()).collect();
    if history.last() != Some(&this_build) {
        history.push(this_build.clone());
        std::fs::write(&history_file, history.join("\n") + "\n").context("writing contracts/identity-history")?;
    }
    let prior: Vec<String> = history.iter().rev().filter(|l| **l != this_build).take(8).map(|l| format!("\"{l}\"")).collect();
    // Each page's NEEDS: every package its code could ask for — its own, the layout's and the theme, and, through
    // them, every package name any of them mentions (a name in a string: over-counting costs a few small reads; missing
    // one would only cost a read later). The loader asks for them all at once when the page opens.
    let names: Vec<&str> = packages.iter().map(|(n, _, _)| *n).collect();
    let mentions = |src: &str| -> Vec<&str> { names.iter().copied().filter(|n| src.contains(&format!("\"{n}\""))).collect() };
    // The app's PAGES (route → its package): the manifest's `pages`, and where each page's needs start.
    let pages: [(&str, &str); 13] = [("/subtitles", "subtitles"), ("/videos", "media"), ("/audio", "media"), ("/space", "space-home"), ("/", "home"), ("/account", "account"), ("/notes", "notes"), ("/chat", "chat"), ("/messages", "messages"), ("/mail", "mail"), ("/contacts", "contacts"), ("/board", "board"), ("/drive", "drive")];
    let mut needs = Vec::new();
    for (route, page) in pages {
        let mut have: Vec<&str> = vec!["theme", "header", "rail", "footer", page];
        let mut i = 0;
        while i < have.len() {
            if let Some((_, src)) = sources.iter().find(|(n, _)| n == have[i]) {
                for m in mentions(src) {
                    if !have.contains(&m) {
                        have.push(m);
                    }
                }
            }
            i += 1;
        }
        needs.push(format!("\"{route}\": [{}]", have.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ")));
    }
    let manifest = format!(
        "{{ \"app\": \"Craftworks\",\n  \"theme\": \"theme\",\n  \"layout\": {{ \"header\": [\"header\"], \"side\": [\"rail\"], \"footer\": [\"footer\"] }},\n  \"pages\": {{ {} }},\n  \"apps\": [ {{ \"name\": \"Notes\", \"views\": [\"personal\", \"shared\"], \"icon\": \"📝\", \"route\": \"/notes\", \"about\": \"Notes, tagged and pinned: yours, or a space's, kept together.\" }}, {{ \"name\": \"Messages\", \"views\": [\"personal\"], \"icon\": \"✉️\", \"route\": \"/messages\", \"counts\": \"messages\", \"about\": \"Private conversations with one person, sealed end to end.\" }}, {{ \"name\": \"Chat\", \"views\": [\"shared\", \"public\"], \"icon\": \"💬\", \"route\": \"/chat\", \"counts\": \"chat\", \"about\": \"A space's channels, Discord-style.\" }}, {{ \"name\": \"Mail\", \"views\": [\"personal\"], \"icon\": \"📮\", \"route\": \"/mail\", \"about\": \"Mail to anyone by their id: signed by your account, sealed to theirs.\" }}, {{ \"name\": \"Contacts\", \"views\": [\"personal\", \"public\"], \"icon\": \"👤\", \"route\": \"/contacts\", \"about\": \"The people you know: friends, following, requests. Find anyone by their id.\" }}, {{ \"name\": \"Board\", \"views\": [\"personal\", \"shared\", \"public\"], \"icon\": \"📋\", \"route\": \"/board\", \"about\": \"Posts, comments and votes, Reddit-style: a space's, or your own profile.\" }}, {{ \"name\": \"Drive\", \"views\": [\"personal\", \"shared\"], \"icon\": \"🗂️\", \"route\": \"/drive\", \"about\": \"Every file you upload or attach, in folders: yours, or a space's.\" }}, {{ \"name\": \"Videos\", \"views\": [\"personal\", \"shared\", \"public\"], \"icon\": \"▶️\", \"route\": \"/videos\", \"about\": \"Your channel and those you follow: videos, movies, episodes, shorts.\" }}, {{ \"name\": \"Audio\", \"views\": [\"personal\", \"shared\", \"public\"], \"icon\": \"🎧\", \"route\": \"/audio\", \"about\": \"Music, podcasts and audiobooks: yours and those you follow, with lyrics and transcripts.\" }}, {{ \"name\": \"Subtitles\", \"views\": [\"personal\", \"shared\"], \"icon\": \"🔤\", \"route\": \"/subtitles\", \"about\": \"Subtitle tracks as data of their own: yours on any video, edited, exported as WebVTT or SRT.\" }} ],\n  \"uses\": [\"notes\", \"pins\", \"tags\", \"spaces\", \"mailbox\", \"spacekeys\", \"reads\", \"people\", \"posts\", \"journal\", \"asks\", \"files\", \"uploads\", \"drive\", \"keypacks\", \"encodes\", \"keep\"],\n  \"identity_prior\": [{}],\n  \"needs\": {{ {} }},\n  \"packages\": {{\n{}\n  }} }}\n",
        pages.iter().map(|(r, p)| format!("\"{r}\": [\"{p}\"]")).collect::<Vec<_>>().join(", "),
        prior.join(", "),
        needs.join(", "),
        entries.join(",\n")
    );

    // 3. The loader's site, then the app's (whose wrapper names the loader's site).
    let sdk = app.join("loader/sdk");
    let loader_web = wire::webapp::app_web(&[
        ("loader.js", &read(&app.join("packages/loader.js"))?),
        ("trace.js", &read(&app.join("packages/trace.js"))?),
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
    let index_html = read(&app.join("wrapper/index.html"))?;
    let mut files: Vec<(&str, &[u8])> = vec![("index.html", &index_html), ("boot.js", boot.as_bytes()), ("manifest.json", manifest.as_bytes())];
    files.extend(entry_files.iter().map(|(p, b)| (p.as_str(), b.as_slice())));
    let app_web = wire::webapp::app_web(&files).map_err(|e| anyhow::anyhow!(e))?;
    println!("manifest: {} B ({} package entries in their own files)", manifest.len(), entry_files.len());
    d.publish(&site_name, &site_code, app_web).await?;
    println!("OPEN: /v1/contract/web/{app_site}/");
    println!("SITE {app_site}");
    Ok(())
}
