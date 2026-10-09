//! LIVE: writes through the PAGE PATH (ruling B) on a private node.
//!
//! The browser's shape exactly: one websocket to the node, `page-io`'s frames
//! sent byte for byte, every binary message fed back to it. The engine runs
//! here (in `page::Server`), the SIGNER delegate signs, blocks are page-PUT
//! with their code, the head Register is created by the first head's PUT.
//! Nothing of the engine delegate is involved.
//!
//! Publishes `ROWS` rows (default 100, then 300: main's switch-over gate) in
//! writes of `PER_WRITE`, each waited on until `Published`, then reads the
//! whole range back through the same page and checks every row.
//!
//! SETTLE=backed: each write waits for the last to be BACKED_UP (a save on an idle node), not back to back.
//!
//! usage: PAGE_PORT=<port> PAGE_TMP=<dir> live-page-writes <signer.wasm> <block.wasm> <register.wasm>
//! The node's data, config and log dirs are all under PAGE_TMP (explicit, no
//! default); the node runs with --disable-auto-update (probe::node).

use anyhow::{bail, Context, Result};
use futures::{SinkExt, StreamExt};
use page::server::{Server, SignerFacts};
use page::{Ms, Page, PutPath};
use page_io::{Artefacts, PageIo};
use probe::live::now_ms;
use probe::node::{Mode, Node, TempTree};
use protocol::{Reply, Request};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

const PER_WRITE: usize = 10;

thread_local! {
    /// THE OP ORDER (#378): every frame sent, one letter each -- `P` a contract PUT, `U` an UPDATE, `G` a GET,
    /// `S` a delegate op (during a write: the Sign), `c` a chunk of a larger request.
    static OPS: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    /// The writes told `ParityComplete` (BACKED_UP): every parity PUT still sent and acked.
    static BACKED: std::cell::RefCell<std::collections::BTreeSet<u64>> = const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
}

fn note_backed(r: &Reply) {
    let r = match r {
        Reply::Acked { body, .. } => body,
        r => r,
    };
    if let Reply::SessionWriteState { write_id, state: protocol::WriteState::ParityComplete, .. } | Reply::WriteState { write_id, state: protocol::WriteState::ParityComplete } = r {
        BACKED.with(|b| b.borrow_mut().insert(*write_id));
    }
}

fn op_letter(frame: &[u8]) -> char {
    use freenet_stdlib::client_api::{ClientRequest, ContractRequest};
    match bincode::deserialize::<ClientRequest>(frame) {
        Ok(ClientRequest::ContractOp(ContractRequest::Put { .. })) => 'P',
        Ok(ClientRequest::ContractOp(ContractRequest::Update { .. })) => 'U',
        Ok(ClientRequest::ContractOp(ContractRequest::Get { .. })) => 'G',
        Ok(ClientRequest::DelegateOp(_)) => 'S',
        _ => 'c',
    }
}
const SESSION: u64 = 11;


#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let v = std::process::Command::new("freenet").arg("--version").output().context("freenet --version")?;
    println!("freenet: {}", String::from_utf8_lossy(&v.stdout).lines().next().unwrap_or_default());
    let port: u16 = std::env::var("PAGE_PORT").ok().and_then(|p| p.parse().ok()).context("PAGE_PORT=<port> is required; there is no default")?;
    if probe::node::RESERVED.contains(&port) {
        bail!("port {port} is the owner's node");
    }
    let tmp = std::env::var("PAGE_TMP").context("PAGE_TMP=<dir> is required: the node's three dirs go under it")?;
    let usage = "usage: live-page-writes <signer.wasm> <block.wasm> <register.wasm>";
    let mut a = std::env::args().skip(1);
    let signer_wasm = std::fs::read(a.next().context(usage)?)?;
    probe::check(&signer_wasm).map_err(|e| anyhow::anyhow!("the signer is refused by the import gate: {e}"))?;
    let block_code = std::fs::read(a.next().context(usage)?)?;
    let register_code = std::fs::read(a.next().context(usage)?)?;
    let rows: Vec<usize> = std::env::var("ROWS").ok().map(|r| r.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_else(|| vec![100, 300]);

    let mut verdict = Ok(());
    for (run, n) in rows.iter().enumerate() {
        // A FRESH node per run: one signer per node holds one key, and a
        // second key is refused (`KeyAlreadyProvisioned`), as it must be.
        let dir = std::path::PathBuf::from(&tmp).join(format!("live-page-writes-{}-{run}", std::process::id()));
        let _tree = TempTree(dir.clone());
        let node = Node::spawn_in(port, &dir, Mode::IsolatedNetwork { network_port: port + 1 })?;
        println!("node {run}: ws {}, data/config/log under {}", node.ws(), dir.display());
        match one_run(&node.ws(), &signer_wasm, &block_code, &register_code, *n, run as u64).await {
            Ok(line) => println!("{line}"),
            Err(e) => {
                println!("RED: {n} rows: {e:#}");
                verdict = Err(e);
            }
        }
        drop(node);
    }
    println!("VERDICT: {}", if verdict.is_ok() { "GREEN" } else { "RED" });
    verdict
}

/// One fresh key (so one fresh Register), `n` rows published and read back.
async fn one_run(ws: &str, signer_wasm: &[u8], block_code: &[u8], register_code: &[u8], n: usize, run: u64) -> Result<String> {
    let (mut sock, _) = tokio_tungstenite::connect_async(ws).await.context("connecting")?;
    let t0 = Instant::now();
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    seed[0] ^= run as u8;
    let sk = ed25519_dalek::SigningKey::from_bytes(&seed);
    let mut io = probe::page::for_key(&sk, signer_wasm, block_code.to_vec(), register_code.to_vec());

    // Drive until `done` says so, or the budget runs out. Every frame out as
    // it is, every message in as it came; the page's timers on its own clock.
    async fn drive(
        sock: &mut (impl SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
        io: &mut PageIo,
        t0: Instant,
        budget: Duration,
        drop_heads: &mut usize,
        mut done: impl FnMut(&[Reply], &PageIo) -> bool,
    ) -> Result<Vec<Reply>> {
        let start = Instant::now();
        let mut replies = Vec::new();
        // The probe's OWN reassembly, to see a whole message before deciding
        // to lose it: a GET answer may arrive as several chunks (0.2.136
        // streams them), and page-io must get all of a message or none.
        let mut seen = wire::Reassembler::default();
        let mut held: Vec<Vec<u8>> = Vec::new();
        loop {
            for f in io.take_frames() {
                OPS.with(|o| o.borrow_mut().push(op_letter(&f)));
                sock.send(Message::Binary(f.into())).await.context("send")?;
            }
            for r in io.take_replies() {
                let r = protocol::decode_reply(&r).map_err(|d| anyhow::anyhow!("a reply that does not decode: {d:?}"))?;
                note_backed(&r);
                replies.push(r);
            }
            if done(&replies, io) {
                return Ok(replies);
            }
            if start.elapsed() > budget {
                bail!("not within {budget:?}; unusable: {:?}", io.unusable());
            }
            let wait = io.next_due().map_or(50, |d| d.0.saturating_sub(now_ms(t0)).clamp(1, 50));
            match tokio::time::timeout(Duration::from_millis(wait), sock.next()).await {
                Ok(Some(Ok(Message::Binary(b)))) => {
                    // The LOST-HEAD-READ case (sdk#175): a whole answer to the
                    // head Register's GET is discarded, as a lost read's.
                    held.push(b.to_vec());
                    match wire::unframe(&mut seen, &b) {
                        wire::Incoming::Partial => continue,
                        wire::Incoming::Got { id, .. } | wire::Incoming::GetFailed { id, .. } if *drop_heads > 0 && id == io.register_id() => {
                            *drop_heads -= 1;
                            held.clear();
                        }
                        _ => {
                            for f in held.drain(..) {
                                io.inbound(&f, Ms(now_ms(t0)));
                            }
                        }
                    }
                }
                Ok(Some(Ok(_))) => {}
                Ok(Some(Err(e))) => bail!("the socket: {e}"),
                Ok(None) => bail!("the node closed the socket"),
                Err(_) => io.tick(Ms(now_ms(t0))),
            }
        }
    }

    drive(&mut sock, &mut io, t0, Duration::from_secs(60), &mut 0, |_, io| io.provisioned()).await.context("provisioning the signer")?;
    let provisioned_ms = t0.elapsed().as_millis();
    io.client(&protocol::encode_session_request(4, SESSION, &Request::Identity).expect("encodes"));
    drive(&mut sock, &mut io, t0, Duration::from_secs(60), &mut 0, |r, _| r.iter().any(|x| matches!(x, Reply::Identity { .. }))).await.context("identity")?;

    BACKED.with(|b| b.borrow_mut().clear()); // per run: the writes' ids restart at 1
    let mut write_ms = Vec::new();
    // Per write: the PUTs sent before its Sign, and after it (while the write is waited on).
    let mut sign_at: Vec<(usize, usize)> = Vec::new();
    // SETTLE=backed: each write waits for the one before it to be BACKED_UP, so it is saved on an idle node (the
    // one-row save a person makes); unset, the writes go back to back (throughput: each queues behind the last one's
    // parity).
    let settle = std::env::var("SETTLE").is_ok_and(|v| v == "backed");
    for (w, chunk) in (0..n).collect::<Vec<_>>().chunks(PER_WRITE).enumerate() {
        let id = w as u64 + 1;
        if settle && id > 1 {
            drive(&mut sock, &mut io, t0, Duration::from_secs(60), &mut 0, |_, _| BACKED.with(|b| b.borrow().contains(&(id - 1))))
                .await
                .with_context(|| format!("write {} never BACKED_UP", id - 1))?;
        }
        let ops = chunk.iter().map(|i| protocol::Op::Put(format!("r/{i:05}").into_bytes(), format!("value {i} of run {run}").into_bytes())).collect();
        let t = Instant::now();
        OPS.with(|o| o.borrow_mut().clear());
        // A write that does not read what it changes is sent FORCED
        // (sdk#283): a reads-less `Request::Write` is refused `Unread`.
        io.client(&protocol::encode_session_request(4, SESSION, &Request::forced_write(id, ops)).expect("encodes"));
        let got = drive(&mut sock, &mut io, t0, Duration::from_secs(120), &mut 0, |r, _| probe::verdict::write_outcome(id, r).is_some())
            .await
            .with_context(|| format!("write {id}"))?;
        // Published is what counts (ParityComplete follows it); anything
        // terminal without it is the failure, said with its reason.
        if let Some(Err(why)) = probe::verdict::write_outcome(id, &got) {
            bail!("{why}");
        }
        write_ms.push(t.elapsed().as_millis());
        let ops = OPS.with(|o| o.borrow().clone());
        if let Some(at) = ops.find('S') {
            sign_at.push((ops[..at].matches('P').count(), ops[at..].matches('P').count()));
        }
        if w < 2 {
            println!("write {id} op order: {ops}");
        }
    }

    // BACKED_UP: every write, its follow-up parity included, acked -- waited on here, and the PUTs it took.
    let t = Instant::now();
    OPS.with(|o| o.borrow_mut().clear());
    let writes = write_ms.len() as u64;
    drive(&mut sock, &mut io, t0, Duration::from_secs(60), &mut 0, |_, _| BACKED.with(|b| (1..=writes).all(|w| b.borrow().contains(&w))))
        .await
        .with_context(|| format!("BACKED_UP: {} of {writes} writes; ops while waiting: {}", BACKED.with(|b| b.borrow().len()), OPS.with(|o| o.borrow().clone())))?;
    let backed_line = format!("BACKED_UP: {writes} of {writes} writes, the last {} ms after the last Published, {} PUT(s) after it", t.elapsed().as_millis(), OPS.with(|o| o.borrow().matches('P').count()));

    // READ BACK: the whole range, through the same page.
    let req = Request::Range { req_id: 900, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: None, max_entries: 256 };
    let mut rows_read = 0usize;
    let mut after = None;
    loop {
        let r = match after.take() {
            None => req.clone(),
            Some(a) => Request::Range { req_id: 901 + rows_read as u64, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: Some(a), max_entries: 256 },
        };
        let rid = match &r {
            Request::Range { req_id, .. } => *req_id,
            _ => unreachable!(),
        };
        io.client(&protocol::encode_session_request(4, SESSION, &r).expect("encodes"));
        let got = drive(&mut sock, &mut io, t0, Duration::from_secs(60), &mut 0, |rs, _| rs.iter().any(|x| matches!(x, Reply::Page { req_id, .. } if *req_id == rid))).await.context("read back")?;
        let Some(Reply::Page { entries, cursor, .. }) = got.into_iter().find(|x| matches!(x, Reply::Page { req_id, .. } if *req_id == rid)) else { bail!("no page") };
        for (i, (k, v)) in entries.iter().enumerate() {
            let want = format!("r/{:05}", rows_read + i);
            if k != want.as_bytes() || !v.starts_with(format!("value {} of run {run}", rows_read + i).as_bytes()) {
                bail!("row {} read back as {:?}", rows_read + i, String::from_utf8_lossy(k));
            }
        }
        rows_read += entries.len();
        match cursor {
            Some(c) if !entries.is_empty() => after = Some(c),
            _ => break,
        }
    }
    if rows_read != n {
        bail!("read back {rows_read} of {n} rows");
    }
    // REOPEN, WITH THE HEAD READ LOST (sdk#175): a second page on the same
    // node and key — its signer provisioned already — whose first THREE head
    // reads are lost. It must ask again and read every row, never an empty tree.
    let (mut sock2, _) = tokio_tungstenite::connect_async(ws).await.context("connecting again")?;
    let (container2, signer2) = wire::delegate_from_code(signer_wasm);
    let mut io2 = PageIo::new(
        Server::new(Page::unstarted(engine::Params::default(), PutPath::Page, Ms(now_ms(t0))), SignerFacts::default()),
        Artefacts {
            block_code: block_code.to_vec(),
            register_code: register_code.to_vec(),
            register_params: wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME),
            signer: signer2,
        },
    );
    // Registered again (the same delegate), provisioned again with the SAME
    // key (accepted: a key is never replaced, only re-stated).
    io2.provision(container2, sk.to_bytes().to_vec());
    drive(&mut sock2, &mut io2, t0, Duration::from_secs(60), &mut 0, |_, io| io.provisioned()).await.context("reopen: provisioning")?;
    let mut lose = 3usize;
    io2.client(&protocol::encode_session_request(4, SESSION + 1, &Request::Identity).expect("encodes"));
    drive(&mut sock2, &mut io2, t0, Duration::from_secs(60), &mut lose, |r, _| r.iter().any(|x| matches!(x, Reply::Identity { .. }))).await.context("reopen: identity")?;
    let rr = Request::Range { req_id: 7000, lo: protocol::Bound::Unbounded, hi: protocol::Bound::Unbounded, reverse: false, after: None, max_entries: 256 };
    io2.client(&protocol::encode_session_request(4, SESSION + 1, &rr).expect("encodes"));
    let got = drive(&mut sock2, &mut io2, t0, Duration::from_secs(90), &mut lose, |rs, _| rs.iter().any(|x| matches!(x, Reply::Page { req_id: 7000, .. }))).await.context("reopen: read")?;
    let first = got.iter().find_map(|x| if let Reply::Page { req_id: 7000, entries, .. } = x { Some(entries.len()) } else { None }).unwrap_or(0);
    if lose != 0 {
        bail!("reopen: the lost head reads were never asked again ({lose} left)");
    }
    if first != n.min(256) {
        bail!("reopen: the first page read {first} rows after 3 lost head reads (want {}): an empty tree?", n.min(256));
    }
    let (rto, srtt, window) = io.server.page.clock();
    let mut before: Vec<usize> = sign_at.iter().map(|x| x.0).collect();
    let mut after: Vec<usize> = sign_at.iter().map(|x| x.1).collect();
    before.sort();
    after.sort();
    let med = |v: &[usize]| v.get(v.len() / 2).copied().unwrap_or(0);
    let mut sorted = write_ms.clone();
    sorted.sort();
    println!("{backed_line}");
    Ok(format!(
        "GREEN: {n} rows in {} writes: every write Published and every row read back; a reopened page with 3 lost head reads read {first} rows. provisioned in {provisioned_ms} ms; per write median {} ms, max {} ms; the Sign after a median {} PUT(s) of the write, {} after it ({} writes); total {} ms; page clock at the end: RTO {rto} ms, SRTT {:.1?} ms, GET window {window}; unusable {:?}",
        write_ms.len(),
        sorted[sorted.len() / 2],
        sorted.last().copied().unwrap_or(0),
        med(&before),
        med(&after),
        sign_at.len(),
        t0.elapsed().as_millis(),
        srtt,
        io.unusable()
    ))
}
