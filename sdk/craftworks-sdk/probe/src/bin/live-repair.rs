//! LIVE: REPAIR ON A REAL NETWORK (sdk#479). What the network LACKS is put back by reading the tree.
//!
//! A private two-node network of this probe's own ([`probe::silent::SilentNet`]: gateway "gw" and "peer", each in its
//! own dirs and web cache, never the owner's ports). A WRITER page on gw publishes a tree of rows THROUGH ws-drop
//! ([`probe::drop::DropEvery`], in process): every `N`th Block PUT never reaches gw and is answered by the proxy, so
//! those blocks were NEVER STORED on the network. The writer is then closed. On peer:
//!  1. a COLD reader reads every row through the normal read (REPAIR: a race finds a block NotFound, rebuilds it from
//!     any `k` of its group -- a lost parity re-encoded -- and PUTs it back). Its report must count as MISSING at
//!     least every dropped block (the loss is real and the reads saw it: the non-vacuous half), and put back exactly
//!     what it found missing, with none refused, none given up;
//!  2. every dropped contract is then read RAW from peer: it is on the network again;
//!  3. a FRESH reader reads every row and finds NOTHING missing.
//!
//! usage: PAGE_PORT=<base> PAGE_TMP=<dir> live-repair <signer.wasm> <block.wasm> <register.wasm> [--every N]
//! (the nodes take base, base+1, base+10, base+11; the proxy base+4). Prints each dropped block, each phase's report, and a VERDICT.

use anyhow::{bail, Context, Result};
use freenet_stdlib::client_api::{ClientRequest, ContractRequest, ContractResponse, HostResponse, WebApi};
use freenet_stdlib::prelude::ContractInstanceId;
use page::{Ms, PutPath};
use page_io::PageIo;
use probe::live::{next_frame, now_ms, Sock};
use probe::silent::SilentNet;
use protocol::{Reply, Request};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SESSION: u64 = 11;
const WRITES: u64 = 3;
const ROWS_PER_WRITE: u32 = 400;

struct Io {
    sock: Sock,
    io: PageIo,
    t0: Instant,
    replies: Vec<Reply>,
}

impl Io {
    /// Drive until `done`, or fail at `budget` naming what the page still waits on.
    async fn drive(&mut self, budget: Duration, mut done: impl FnMut(&mut PageIo, &[Reply]) -> bool) -> Result<()> {
        let start = Instant::now();
        loop {
            probe::live::flush(&mut self.sock, &mut self.io, &mut self.replies).await?;
            if done(&mut self.io, &self.replies) {
                return Ok(());
            }
            if start.elapsed() > budget {
                bail!("not within {budget:?}: not answering {:?}; unusable {:?}", self.io.not_answering(), self.io.unusable());
            }
            if let Some(b) = next_frame(&mut self.sock, &mut self.io, self.t0).await? {
                self.io.inbound(&b, Ms(now_ms(self.t0)));
            }
        }
    }

    fn ask(&mut self, r: &Request) {
        self.io.client(&protocol::encode_session_request(4, SESSION, r).expect("encodes"));
    }

    async fn identity(&mut self) -> Result<()> {
        self.ask(&Request::Identity);
        self.drive(Duration::from_secs(120), |_, r| r.iter().any(|x| matches!(x, Reply::Identity { .. }))).await.context("identity")
    }

    /// Every row of the tree, through the normal read: a full range, page by page.
    async fn read_all(&mut self, base: u64) -> Result<usize> {
        let (mut rows, mut after) = (0usize, None);
        for n in 1u64.. {
            let req_id = base + n;
            self.ask(&Request::Range { req_id, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: after.clone(), max_entries: 500 });
            self.drive(Duration::from_secs(900), |_, r| r.iter().any(|x| matches!(x, Reply::Page { req_id: q, .. } if *q == req_id))).await.with_context(|| format!("scan page {n}"))?;
            let (got, cursor) = self.replies.iter().find_map(|x| if let Reply::Page { req_id: q, entries, cursor, .. } = x { (*q == req_id).then(|| (entries.len(), cursor.clone())) } else { None }).expect("matched");
            rows += got;
            match cursor {
                Some(c) => after = Some(c),
                None => return Ok(rows),
            }
        }
        unreachable!()
    }
}

fn reader(block_code: &[u8], register_id: [u8; 32]) -> PageIo {
    PageIo::reader(page::server::Server::new(page::Page::unstarted(engine::Params::default(), PutPath::Page, Ms(0)), page::server::SignerFacts::default()), block_code.to_vec(), register_id, 1)
}

/// A contract's state as `peer` serves it to a raw GET: `None` when not answered within 30 s.
async fn raw_get(c: &mut WebApi, id: ContractInstanceId) -> Result<Option<Vec<u8>>> {
    c.send(ClientRequest::ContractOp(ContractRequest::Get { key: id, return_contract_code: false, subscribe: false, blocking_subscribe: false })).await?;
    let by = Instant::now() + Duration::from_secs(30);
    while Instant::now() < by {
        match tokio::time::timeout(Duration::from_secs(30), c.recv()).await {
            Ok(Ok(HostResponse::ContractResponse(ContractResponse::GetResponse { key, state, .. }))) if *key.id() == id => return Ok(Some(state.as_ref().to_vec())),
            Ok(Ok(HostResponse::ContractResponse(ContractResponse::NotFound { instance_id }))) if instance_id == id => return Ok(None),
            Ok(Ok(_)) => continue,
            Ok(Err(e)) => bail!("peer's socket, reading {id}: {e}"),
            Err(_) => return Ok(None),
        }
    }
    Ok(None)
}

async fn connect(url: &str) -> Result<Sock> {
    Ok(tokio_tungstenite::connect_async(url).await.with_context(|| format!("connecting to {url}"))?.0)
}

async fn run(net: &SilentNet, base: u16, every: usize, signer_wasm: &[u8], block_code: &[u8], register_code: &[u8]) -> Result<String> {
    let t0 = Instant::now();
    let mut peer = probe::signer::connect(&net.b.ws()).await?;
    println!("peer connected to {:?}", net.wait_connected(&mut peer).await?);

    // ws-drop in front of gw, in process: its lines kept for the checks and printed.
    let drops: Arc<Mutex<Vec<ContractInstanceId>>> = Arc::new(Mutex::new(Vec::new()));
    let keep = drops.clone();
    let log = probe::proxy::Log::to(move |l| {
        println!("ws-drop: {l}");
        if let Some(id) = l.get("dropped").and_then(|v| v.as_str()) {
            if let Ok(id) = id.parse::<ContractInstanceId>() {
                keep.lock().unwrap().push(id);
            }
        }
    });
    let gw_url = net.a_ws();
    let gw_port = probe::node::allowed_port(&gw_url)?;
    let proxy_port = base + 4;
    probe::node::allowed_ports(&[proxy_port])?;
    tokio::spawn(probe::proxy::serve(proxy_port, gw_port, serde_json::json!({ "dropping": format!("every {every}th Block PUT") }), Arc::new(probe::drop::DropEvery::with_log(every, log))));
    tokio::time::sleep(Duration::from_millis(300)).await;
    let through = gw_url.replacen(&format!(":{gw_port}"), &format!(":{proxy_port}"), 1);

    // THE WRITER, on gw through ws-drop.
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
    let io = probe::page::for_key(&sk, signer_wasm, block_code.to_vec(), register_code.to_vec());
    let register_id = io.register_id();
    let mut w = Io { sock: connect(&through).await?, io, t0, replies: Vec::new() };
    w.drive(Duration::from_secs(120), |io, _| io.provisioned() && io.register_params_known()).await.context("provisioning")?;
    w.identity().await?;
    for n in 1..=WRITES {
        let lo = (n as u32 - 1) * ROWS_PER_WRITE;
        let ops = (lo..lo + ROWS_PER_WRITE).map(|i| protocol::Op::Put(format!("r/{i:05}").into_bytes(), vec![(i % 251) as u8; 90])).collect();
        w.ask(&Request::forced_write(n, ops));
        if let Err(e) = w.drive(Duration::from_secs(300), |_, r| probe::verdict::write_outcome(n, r).is_some()).await {
            let said: Vec<String> = w.replies.iter().filter_map(|r| match r {
                Reply::WriteState { write_id, state } | Reply::SessionWriteState { write_id, state, .. } if *write_id == n => Some(format!("{state:?}")),
                _ => None,
            }).collect();
            bail!("write {n}: {e:#}; its states {said:?}; page waiting {}; ops on the wire {}; unusable {:?}", w.io.waiting(), w.io.server.page.ops_on_wire(), w.io.unusable());
        }
        match probe::verdict::write_outcome(n, &w.replies) {
            Some(Ok(())) => {}
            other => bail!("write {n} ended {other:?}"),
        }
    }
    // Every PUT answered (the dropped ones by the proxy): nothing more of the tree is on its way.
    w.drive(Duration::from_secs(300), |io, _| !io.waiting()).await.context("the writer settling")?;
    // The CURRENT tree's blocks, walked from the published root in the writer's own store: a block ws-drop kept off
    // the network from a SUPERSEDED head is no read's business.
    let root = w.io.server.page.published().1;
    let mut tree = std::collections::BTreeSet::from([root]);
    let mut at = vec![root];
    while let Some(id) = at.pop() {
        let bytes = freenet_prolly::store::Blocks::get(w.io.server.page.blocks(), &id).context("the writer holds its tree")?.to_vec();
        let n = freenet_prolly::node::Node::parse(&bytes).map_err(|e| anyhow::anyhow!("a tree node: {e:?}"))?;
        tree.extend(n.parity());
        for (_, members) in freenet_prolly::parity::group_members(&n) {
            tree.extend(members.iter().copied());
        }
        if !n.is_leaf() {
            at.extend((0..n.len()).map(|i| n.child(i).0));
        }
    }
    let current: std::collections::HashSet<ContractInstanceId> = tree.iter().map(|c| ContractInstanceId::new(wire::block::contract_for(block_code, c))).collect();
    drop(w);
    let all_dropped: Vec<ContractInstanceId> = drops.lock().unwrap().clone();
    let dropped: Vec<ContractInstanceId> = all_dropped.iter().copied().filter(|id| current.contains(id)).collect();
    let rows_written = (WRITES as usize) * ROWS_PER_WRITE as usize;
    println!("writer on gw: {rows_written} rows published; ws-drop dropped {} blocks (never stored), {} of them in the CURRENT tree of {} blocks; writer closed at t+{} s", all_dropped.len(), dropped.len(), current.len(), t0.elapsed().as_secs());
    if dropped.is_empty() {
        bail!("SETUP: ws-drop dropped no block of the current tree: the run proves nothing");
    }

    // 1. REPAIR: a cold reader on peer reads the whole tree.
    let mut r1 = Io { sock: connect(&net.b.ws()).await?, io: reader(block_code, register_id), t0, replies: Vec::new() };
    r1.io.server.page.set_repair_pass(true, true); // as Session::scan_all does for repairAll
    r1.identity().await?;
    let t1 = Instant::now();
    let rows1 = r1.read_all(100).await?;
    r1.drive(Duration::from_secs(600), |io, _| io.server.page.repair_report().pending == 0).await.context("the put-backs' answers")?;
    let rep = r1.io.server.page.repair_report();
    println!("REPAIR (cold reader on peer, {} s): rows {rows1}; missing {}, put back {} (acked), reput {}, rejected {}, given up {}, parity mismatched {}, why {:?}", t1.elapsed().as_secs(), rep.missing, rep.put_back, rep.reput, rep.rejected, rep.given_up, rep.parity_mismatched, rep.why);
    if rows1 != rows_written {
        bail!("the repairing read read {rows1} rows of {rows_written}");
    }
    if (rep.missing as usize) < dropped.len() {
        bail!("the repairing read found {} missing, fewer than the {} current-tree blocks ws-drop kept off the network: it did not see the loss", rep.missing, dropped.len());
    }
    if rep.put_back != rep.missing || rep.rejected != 0 || rep.given_up != 0 || rep.parity_mismatched != 0 {
        bail!("the repair did not put back exactly what it found missing: {rep:?}");
    }
    drop(r1);

    // 2. Every dropped block is on the network again, read RAW from peer.
    let mut absent = Vec::new();
    for id in &dropped {
        if raw_get(&mut peer, *id).await?.is_none() {
            absent.push(id.to_string());
        }
    }
    if !absent.is_empty() {
        bail!("{} of {} dropped blocks are still not served by peer: {absent:?}", absent.len(), dropped.len());
    }
    println!("peer serves all {} dropped blocks again", dropped.len());

    // 3. A FRESH reader on peer finds nothing missing.
    let mut r2 = Io { sock: connect(&net.b.ws()).await?, io: reader(block_code, register_id), t0, replies: Vec::new() };
    r2.identity().await?;
    let rows2 = r2.read_all(100).await?;
    let rep2 = r2.io.server.page.repair_report();
    println!("FRESH reader on peer: rows {rows2}; missing {}", rep2.missing);
    if rows2 != rows_written || rep2.missing != 0 {
        bail!("the fresh reader read {rows2} rows and found {} missing", rep2.missing);
    }
    Ok(format!("GREEN: {} current-tree blocks never stored (ws-drop every {every}th Block PUT) -> repair found {} missing, put back {} -> peer serves every one -> a fresh reader reads {rows2} rows with 0 missing", dropped.len(), rep.missing, rep.put_back))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    println!("freenet: {}", probe::node::freenet_version()?);
    let base: u16 = std::env::var("PAGE_PORT").ok().and_then(|p| p.parse().ok()).context("PAGE_PORT=<base port> is required; there is no default")?;
    let tmp = std::env::var("PAGE_TMP").context("PAGE_TMP=<dir> is required: the nodes' dirs go under it")?;
    let a: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: PAGE_PORT=<base> PAGE_TMP=<dir> live-repair <signer.wasm> <block.wasm> <register.wasm> [--every N]";
    let (files, every) = match a.as_slice() {
        [s, b, r] => ([s, b, r], 7),
        [s, b, r, e, n] if e == "--every" => ([s, b, r], n.parse::<usize>().context(usage)?),
        _ => bail!(usage),
    };
    let signer_wasm = std::fs::read(files[0])?;
    probe::check(&signer_wasm).map_err(|e| anyhow::anyhow!("the signer is refused by the import gate: {e}"))?;
    let block_code = std::fs::read(files[1])?;
    let register_code = std::fs::read(files[2])?;
    let root = std::path::PathBuf::from(&tmp).join(format!("live-repair-{}", std::process::id()));
    let net = SilentNet::start(&root, base, 1)?;
    let got = run(&net, base, every, &signer_wasm, &block_code, &register_code).await;
    net.finish()?;
    let _ = std::fs::remove_dir_all(&root);
    match got {
        Ok(line) => {
            println!("{line}");
            println!("VERDICT: GREEN");
            Ok(())
        }
        Err(e) => {
            println!("RED: {e:#}");
            println!("VERDICT: RED");
            Err(e)
        }
    }
}
