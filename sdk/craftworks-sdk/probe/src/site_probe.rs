//! A SITE's publication reaching ANOTHER node, on a private two-node network (batch 7a's realnet step 9, localised:
//! the builder republishes an app from the SAME page, and a reader on another node must see the new version).
//!
//! Private gateway `a` and peer `b` ([`crate::silent::SilentNet`], each in its own dirs and web cache, off the owner's
//! ports), the REAL signer and site contract. One page on `a`:
//!  1. (with data) writes a record -- a head commit -- then publishes site v1;
//!  2. `b` reads the head and the site WITH subscribe (it now holds and follows them, as a reader does);
//!  3. (with data) writes a second record -- the "structure change", head seq + 1 -- then republishes site v2;
//!  4. `b` reads again, every 500 ms, up to 60 s.
//!
//! Every frame the page sends for the site or the head is printed (each Sign with its label and seq, each site PUT
//! with its record's seq and its web part's hash, each head write), with every NEW `unusable()` line and the page's
//! published head seq. Two ARMS on two fresh networks:
//! * LIVE -- GREEN when `b` serves v2 (seq 2, v2's web) and, with data, head seq >= 2;
//! * CONTROL -- v2's site PUT is WITHHELD (never sent): `b` must still serve v1 ten seconds later. A probe whose
//!   reader could not see a missing publication would be GREEN in both arms; this arm fails it.
//!
//! The reader check is also shown non-vacuous in every run: `b` reports v1 before the republish.

use crate::silent::SilentNet;
use anyhow::{bail, Context, Result};
use freenet_stdlib::client_api::{ClientRequest, ContractRequest, ContractResponse, DelegateRequest, HostResponse, WebApi};
use freenet_stdlib::prelude::{ContractInstanceId, InboundDelegateMsg};
use crate::live::{now_ms, Sock};
use futures::SinkExt;
use page::{Ms, Publication};
use page_io::PageIo;
use protocol::{Reply, Request};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

const APP: &str = "notes";
const SESSION: u64 = 11;

/// The probe's inputs: the four contracts, and where its nodes go.
pub struct Inputs {
    pub base: u16,
    pub tmp: String,
    pub signer_wasm: Vec<u8>,
    pub block_code: Vec<u8>,
    pub register_code: Vec<u8>,
    pub site_code: Vec<u8>,
}

impl Inputs {
    /// `PAGE_PORT=<base>` (the nodes take base..base+3), `PAGE_TMP=<dir>`, then `<signer.wasm> <block.wasm>
    /// <register.wasm> <site.wasm>`; no defaults.
    pub fn from_env(name: &str) -> Result<Inputs> {
        println!("freenet: {}", crate::node::freenet_version()?);
        let base: u16 = std::env::var("PAGE_PORT").ok().and_then(|p| p.parse().ok()).context("PAGE_PORT=<base port> is required; there is no default")?;
        let tmp = std::env::var("PAGE_TMP").context("PAGE_TMP=<dir> is required: the nodes' dirs go under it")?;
        let usage = format!("usage: {name} <signer.wasm> <block.wasm> <register.wasm> <site.wasm>");
        let mut a = std::env::args().skip(1);
        let signer_wasm = std::fs::read(a.next().context(usage.clone())?)?;
        crate::check(&signer_wasm).map_err(|e| anyhow::anyhow!("the signer is refused by the import gate: {e}"))?;
        let block_code = std::fs::read(a.next().context(usage.clone())?)?;
        let register_code = std::fs::read(a.next().context(usage.clone())?)?;
        let site_code = std::fs::read(a.next().context(usage)?)?;
        Ok(Inputs { base, tmp, signer_wasm, block_code, register_code, site_code })
    }
}

/// Which arm runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arm {
    Live,
    /// v2's site PUT is withheld.
    Control,
}

/// Run both arms (each on its own fresh network); print a VERDICT; the error of the first arm that failed.
pub async fn run_both(name: &str, with_data: bool) -> Result<()> {
    let inputs = Inputs::from_env(name)?;
    let mut verdict = Ok(());
    for arm in [Arm::Live, Arm::Control] {
        let root = std::path::PathBuf::from(&inputs.tmp).join(format!("{name}-{}-{arm:?}", std::process::id()));
        let net = SilentNet::start(&root, inputs.base, 1)?;
        println!("ARM {arm:?} (with data: {with_data})");
        let got = run(&net, &inputs, with_data, arm).await;
        net.finish()?;
        let _ = std::fs::remove_dir_all(&root);
        match got {
            Ok(line) => println!("{line}"),
            Err(e) => {
                println!("RED: arm {arm:?}: {e:#}");
                if verdict.is_ok() {
                    verdict = Err(e);
                }
            }
        }
    }
    println!("VERDICT: {}", if verdict.is_ok() { "GREEN" } else { "RED" });
    verdict
}

fn hex8(b: &[u8]) -> String {
    core_types::hex::encode(&b[..b.len().min(4)])
}

/// A site's web part for version `n` (4000 bytes, distinct per version).
fn web(n: u8) -> Vec<u8> {
    (0..4_000u32).map(|i| (i as u8) ^ n.wrapping_mul(37)).collect()
}

/// What one page frame is, for the site or the head.
enum Frame {
    Signer(String),
    SitePut { seq: u64, web: String },
    Head(String),
}

fn classify(frame: &[u8], site: &ContractInstanceId, head: &[u8; 32]) -> Option<Frame> {
    let req: ClientRequest = bincode::deserialize(frame).ok()?;
    match req {
        ClientRequest::DelegateOp(DelegateRequest::ApplicationMessages { inbound, .. }) => {
            let parts: Vec<String> = inbound
                .iter()
                .filter_map(|m| match m {
                    InboundDelegateMsg::ApplicationMessage(am) => signer::decode_request(&am.payload).map(|(id, r)| format!("id {id}: {}", format!("{r:?}").chars().take(150).collect::<String>())),
                    _ => None,
                })
                .collect();
            (!parts.is_empty()).then(|| Frame::Signer(parts.join(" | ")))
        }
        ClientRequest::ContractOp(ContractRequest::Put { contract, state, .. }) if contract.key().id() == site => {
            let (meta, web) = contract_keys::site::framing(state.as_ref())?;
            Some(Frame::SitePut { seq: page::HeadRead::from_record(meta)?.seq, web: hex8(blake3::hash(web).as_bytes()) })
        }
        ClientRequest::ContractOp(ContractRequest::Update { key, .. }) if key.id().as_bytes() == head => Some(Frame::Head("UPDATE".into())),
        ClientRequest::ContractOp(ContractRequest::Put { contract, state, .. }) if contract.key().id().as_bytes() == head => {
            Some(Frame::Head(format!("PUT seq {:?}", page::HeadRead::from_record(state.as_ref()).map(|h| h.seq))))
        }
        _ => None,
    }
}

/// The page on `a`, and what the probe has printed of it.
struct Pub {
    sock: Sock,
    io: PageIo,
    t0: Instant,
    site: ContractInstanceId,
    head: [u8; 32],
    said: usize,
    seq: u64,
    replies: Vec<Reply>,
    site_code: Vec<u8>,
    /// Withhold a site PUT at this seq (the control arm), and whether one was withheld.
    withhold: Option<u64>,
    withheld: bool,
}

impl Pub {
    /// Drive until `done`, or `budget` (an error, unless `quiet`: the control's fixed window).
    async fn drive(&mut self, budget: Duration, quiet: bool, mut done: impl FnMut(&mut PageIo, &[Reply]) -> bool) -> Result<()> {
        let start = Instant::now();
        loop {
            for f in self.io.take_frames() {
                let at = self.t0.elapsed().as_millis();
                match classify(&f, &self.site, &self.head) {
                    Some(Frame::SitePut { seq, web }) if self.withhold == Some(seq) => {
                        println!("  t+{at:>6} ms  page -> a: SITE PUT seq {seq} web {web}  WITHHELD (control)");
                        self.withheld = true;
                        continue;
                    }
                    Some(Frame::SitePut { seq, web }) => println!("  t+{at:>6} ms  page -> a: SITE PUT seq {seq} web {web}"),
                    Some(Frame::Signer(s)) => println!("  t+{at:>6} ms  page -> a: signer {s}"),
                    Some(Frame::Head(s)) => println!("  t+{at:>6} ms  page -> a: HEAD {s}"),
                    None => {}
                }
                self.sock.send(Message::Binary(f.into())).await.context("send")?;
            }
            for r in self.io.take_replies() {
                self.replies.push(protocol::decode_reply(&r).map_err(|d| anyhow::anyhow!("a reply that does not decode: {d:?}"))?);
            }
            let said = self.io.unusable();
            for line in &said[self.said.min(said.len())..] {
                println!("  t+{:>6} ms  UNUSABLE: {line}", self.t0.elapsed().as_millis());
            }
            self.said = said.len();
            let seq = self.io.server.page.published().0;
            if seq != self.seq {
                println!("  t+{:>6} ms  the page's published head seq {} -> {seq}", self.t0.elapsed().as_millis(), self.seq);
                self.seq = seq;
            }
            if done(&mut self.io, &self.replies) {
                return Ok(());
            }
            if start.elapsed() > budget {
                if quiet {
                    return Ok(());
                }
                bail!("not within {budget:?}; publication {:?}", self.io.publication(APP));
            }
            if let Some(b) = crate::live::next_frame(&mut self.sock, &mut self.io, self.t0).await? {
                self.io.inbound(&b, Ms(now_ms(self.t0)));
            }
        }
    }

    /// Write one record (a head commit) and drive to its outcome.
    async fn write(&mut self, id: u64) -> Result<()> {
        let ops = vec![protocol::Op::Put(format!("r/{id}").into_bytes(), vec![7u8; 600])];
        self.io.client(&protocol::encode_session_request(4, SESSION, &Request::forced_write(id, ops)).expect("encodes"));
        self.drive(Duration::from_secs(90), false, |_, r| crate::verdict::write_outcome(id, r).is_some()).await.with_context(|| format!("write {id}"))?;
        match crate::verdict::write_outcome(id, &self.replies) {
            Some(Ok(())) => Ok(()),
            other => bail!("write {id} ended {other:?}"),
        }
    }

    /// Publish site version `n`; drive to its end (or, `quiet`, for a fixed window).
    async fn publish(&mut self, n: u8, quiet: bool) -> Result<Option<Publication>> {
        self.io.publish_site(APP, &self.site_code, web(n), Ms(now_ms(self.t0))).map_err(|e| anyhow::anyhow!(e))?;
        let window = if quiet { Duration::from_secs(10) } else { Duration::from_secs(90) };
        self.drive(window, quiet, |io, _| !matches!(io.publication(APP), Some(Publication::Publishing { .. }) | None)).await.with_context(|| format!("site v{n}"))?;
        Ok(self.io.publication(APP))
    }
}

/// A contract's state as `b` serves it to a GET. `None`: no answer within 10 s (not served YET; the caller asks
/// again). A SOCKET ERROR is not an absence: it is an error, with its reason (a could-not-check never reads as
/// "nothing served").
async fn read(c: &mut WebApi, id: ContractInstanceId, subscribe: bool) -> Result<Option<Vec<u8>>> {
    c.send(ClientRequest::ContractOp(ContractRequest::Get { key: id, return_contract_code: false, subscribe, blocking_subscribe: false })).await?;
    let by = Instant::now() + Duration::from_secs(10);
    while Instant::now() < by {
        match tokio::time::timeout(Duration::from_secs(10), c.recv()).await {
            Ok(Ok(HostResponse::ContractResponse(ContractResponse::GetResponse { key, state, .. }))) if *key.id() == id => return Ok(Some(state.as_ref().to_vec())),
            Ok(Ok(_)) => continue,
            Ok(Err(e)) => bail!("b's socket, reading {id}: {e}"),
            Err(_) => return Ok(None),
        }
    }
    Ok(None)
}

/// What `b` serves: the head's seq (`head`: None when the run makes no data commit -- there is no head to read, and
/// a GET of it would only wait out its bound), and the site's (seq, web hash).
async fn b_view(c: &mut WebApi, site: ContractInstanceId, head: Option<ContractInstanceId>, subscribe: bool) -> Result<(Option<u64>, Option<(u64, String)>)> {
    let h = match head {
        Some(head) => read(c, head, subscribe).await?.and_then(|s| page::HeadRead::from_record(&s)).map(|h| h.seq),
        None => None,
    };
    let s = read(c, site, subscribe).await?.and_then(|st| {
        let (meta, web) = contract_keys::site::framing(&st)?;
        Some((page::HeadRead::from_record(meta)?.seq, hex8(blake3::hash(web).as_bytes())))
    });
    Ok((h, s))
}

async fn run(net: &SilentNet, inputs: &Inputs, with_data: bool, arm: Arm) -> Result<String> {
    let t0 = Instant::now();
    let mut reader = crate::signer::connect(&net.b.ws()).await?;
    println!("b connected to {:?}", net.wait_connected(&mut reader).await?);
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
    let params = wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME);
    let site = *page_io::site_contract(&inputs.site_code, &params, APP).context("no site contract")?.key().id();
    let (sock, _) = tokio_tungstenite::connect_async(net.a_ws()).await.context("connecting to a")?;
    let io = crate::page::for_key(&sk, &inputs.signer_wasm, inputs.block_code.clone(), inputs.register_code.clone());
    let head = io.register_id();
    let mut p = Pub { sock, io, t0, site, head, said: 0, seq: 0, replies: Vec::new(), site_code: inputs.site_code.clone(), withhold: None, withheld: false };
    p.drive(Duration::from_secs(60), false, |io, _| io.provisioned() && io.register_params_known()).await.context("provisioning")?;
    p.io.client(&protocol::encode_session_request(4, SESSION, &Request::Identity).expect("encodes"));
    p.drive(Duration::from_secs(60), false, |_, r| r.iter().any(|x| matches!(x, Reply::Identity { .. }))).await.context("identity")?;
    if with_data {
        p.write(1).await?;
    }
    if p.publish(1, false).await? != Some(Publication::Published { version: 1 }) {
        bail!("site v1 did not publish: {:?}", p.io.publication(APP));
    }
    let head_id = with_data.then(|| ContractInstanceId::new(head));
    let mut before = (None, None);
    for _ in 0..20 {
        before = b_view(&mut reader, site, head_id, true).await?;
        if before.1.is_some() && (!with_data || before.0.is_some()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let v1 = hex8(blake3::hash(&web(1)).as_bytes());
    // NON-VACUOUS: b serves v1 before the republish (the reader can tell v1 from v2).
    if before.1.as_ref().map(|(s, w)| (*s, w.as_str())) != Some((1, v1.as_str())) {
        bail!("SETUP: b does not serve v1 before the republish: {before:?}");
    }
    println!("b before the republish: head seq {:?}, site {:?}", before.0, before.1);
    if with_data {
        p.write(2).await?;
    }
    let floor = p.io.server.page.published().0;
    p.withhold = (arm == Arm::Control).then_some(2);
    let t2 = Instant::now();
    let ended = p.publish(2, arm == Arm::Control).await?;
    println!("site v2 (its head floor {floor}): {ended:?} after {} ms", t2.elapsed().as_millis());
    let v2 = hex8(blake3::hash(&web(2)).as_bytes());
    match arm {
        Arm::Live => {
            if ended != Some(Publication::Published { version: 2 }) {
                bail!("site v2 ended {ended:?}");
            }
            let t3 = Instant::now();
            let mut last = (None, None);
            while t3.elapsed() < Duration::from_secs(60) {
                last = b_view(&mut reader, site, head_id, false).await?;
                let head_ok = !with_data || last.0.is_some_and(|h| h >= 2);
                if head_ok && last.1.as_ref().is_some_and(|(s, w)| *s == 2 && *w == v2) {
                    return Ok(format!("GREEN: arm Live: b serves head seq {:?} and site v2 {} ms after v2 ended", last.0, t3.elapsed().as_millis()));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            bail!("b serves head seq {:?}, site {:?} 60 s after v2 ended (want site (2, {v2}))", last.0, last.1)
        }
        Arm::Control => {
            if !p.withheld {
                bail!("SETUP: the control withheld no site PUT (the page never sent v2's)");
            }
            let after = b_view(&mut reader, site, head_id, false).await?;
            if after.1.as_ref().map(|(s, w)| (*s, w.as_str())) != Some((1, v1.as_str())) {
                bail!("the CONTROL failed: v2's site PUT was withheld, yet b serves {after:?}: the reader does not measure what reached b");
            }
            Ok(format!("GREEN: arm Control: v2's site PUT withheld; b still serves site (1, {v1}) and head seq {:?}", after.0))
        }
    }
}
