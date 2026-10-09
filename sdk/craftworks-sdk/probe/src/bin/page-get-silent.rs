//! sdk#447's ACCEPTANCE, LIVE: the PAGE sends exactly ONE node GET per key while the node is silent about it, for
//! longer than the node's own GET bound B (`page::rto::NODE_GET_BOUND_MS`).
//!
//! A SIGSTOPped peer makes silence deterministic (the real network cannot force it). Private loopback nodes, never
//! 7509/7609: gateway a, `--silent-peers N`-1 more joined to a, and b joined to a; b is connected to every one of them
//! before ALL are paused. The real `page::Page` then runs on ONE websocket to b: it is told a head whose root is a
//! block nobody holds, and a client read makes its engine fetch that root. Every GET frame the page sends is counted
//! by key and timed, every answer b gives is fed back to the page, and nothing else the page sends goes out.
//!
//! The verdict is the property, measured from both ends:
//! - the page: never a GET of a key while its previous GET of that key is UNANSWERED (the node still fetching),
//!   and never a re-send on the RTO -- a second GET only after an answer, or once B + an RTO has passed;
//! - node b's own log: its client GETs of that key, the transactions it spawned.
//!
//! b's log dir and console out/err are copied out to `<keep>` before the temp tree goes.
//!
//! usage: page-get-silent <block.wasm> <dir> <keep> [--base PORT] [--watch SECS] [--silent-peers N]
use anyhow::{bail, Context, Result};
use freenet_stdlib::client_api::{ClientRequest, ContractRequest, ContractResponse, HostResponse};
use freenet_stdlib::prelude::*;
use page::{Answer, Ms, Op, Page, PutPath};
use probe::silent::arg;
use probe::signer::container;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tokio::time::timeout;

fn hex(b: &[u8]) -> String {
    core_types::hex::encode(b)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let usage = "usage: page-get-silent <block.wasm> <dir> <keep> [--base PORT] [--watch SECS] [--silent-peers N]";
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (Some(bw), Some(dir), Some(keep)) = (a.first(), a.get(1), a.get(2)) else { bail!("{usage}") };
    let bound = page::rto::NODE_GET_BOUND_MS as u64;
    let watch = Duration::from_secs(arg(&a, "--watch").map(|v| v.parse()).transpose()?.unwrap_or(bound / 1000 + 90));
    let bcode = std::fs::read(bw).with_context(|| format!("reading {bw}"))?;
    let (_tree, mut net, mut c, peers) = probe::silent::open(&a, dir, 47_600, Some(keep.as_str())).await?;
    let n_peers = peers.len();
    println!("{}", serde_json::json!({ "b": net.b.ws(), "b_connected_to": peers, "B_ms": bound }));
    println!("{}", serde_json::json!({ "paused": net.pause()? }));

    // THE PAGE: a head whose root nobody holds, and a read that needs it.
    let t0 = Instant::now();
    let now = || Ms(1_000 + t0.elapsed().as_millis() as u64);
    let mut p = Page::unstarted(engine::Params::default(), PutPath::Page, now());
    let mut missing = [0u8; 32];
    getrandom::getrandom(&mut missing).expect("random");
    p.event(engine::Event::HeadRead { epoch: engine::Epoch(0), seq: 1, root: missing });
    p.event(engine::Event::Get { client: engine::ClientId(1), req_id: engine::read::ReqId(1), key: b"k".to_vec() });

    let mut by_contract: std::collections::HashMap<ContractInstanceId, [u8; 32]> = std::collections::HashMap::new();
    // Per key: every send (ms) and every answer (ms, what).
    let mut sends: BTreeMap<[u8; 32], Vec<u64>> = BTreeMap::new();
    let mut answers: BTreeMap<[u8; 32], Vec<(u64, String)>> = BTreeMap::new();
    let mut other_ops = 0usize;
    // The property, checked at each send: the key's previous GET was answered, or B + an RTO passed since it.
    let mut violations: Vec<String> = Vec::new();
    let end = tokio::time::Instant::now() + watch;
    while tokio::time::Instant::now() < end {
        p.tick(now());
        for op in p.take_ops() {
            let Op::Get { id } = op else {
                other_ops += 1;
                continue;
            };
            let at = now().0;
            let last_send = sends.get(&id).and_then(|s| s.last().copied());
            let answered_since = last_send.is_some_and(|l| answers.get(&id).is_some_and(|a| a.iter().any(|(t, _)| *t >= l)));
            if let Some(l) = last_send {
                if !answered_since && at < l + bound {
                    violations.push(format!("{} sent again at {at} ms, {} ms after an UNANSWERED GET (inside B)", hex(&id[..4]), at - l));
                }
            }
            sends.entry(id).or_default().push(at);
            let key = *container(&bcode, &id).key().id();
            by_contract.insert(key, id);
            c.send(ClientRequest::ContractOp(ContractRequest::Get { key, return_contract_code: false, subscribe: false, blocking_subscribe: false })).await?;
            println!("{}", serde_json::json!({ "sent": hex(&id[..4]), "ms": at }));
        }
        let got = match timeout(Duration::from_millis(50), c.recv()).await {
            Err(_) => continue,
            Ok(r) => r,
        };
        let at = now();
        let (id, answer, what) = match got {
            Ok(HostResponse::ContractResponse(ContractResponse::NotFound { instance_id })) => match by_contract.get(&instance_id) {
                Some(id) => (*id, Answer::GetMissed(*id), "not found".to_string()),
                None => continue,
            },
            Ok(HostResponse::ContractResponse(ContractResponse::GetResponse { key, state, .. })) => match by_contract.get(key.id()) {
                Some(id) => (*id, Answer::Got { id: *id, bytes: state.as_ref().to_vec() }, format!("state {} bytes", state.size())),
                None => continue,
            },
            Ok(other) => {
                println!("{}", serde_json::json!({ "other": format!("{other:?}").chars().take(160).collect::<String>(), "ms": at.0 }));
                continue;
            }
            Err(e) => {
                println!("{}", serde_json::json!({ "error": format!("{e}").chars().take(200).collect::<String>(), "ms": at.0 }));
                continue;
            }
        };
        answers.entry(id).or_default().push((at.0, what.clone()));
        println!("{}", serde_json::json!({ "answer": hex(&id[..4]), "what": what, "ms": at.0 }));
        p.answer(answer, at);
    }
    let per_key: BTreeMap<String, serde_json::Value> = sends
        .iter()
        .map(|(id, s)| (hex(&id[..4]), serde_json::json!({ "sent_ms": s, "answers": answers.get(id).cloned().unwrap_or_default() })))
        .collect();
    println!("{}", serde_json::json!({ "verdict": {
        "watched_s": watch.as_secs(), "B_ms": bound, "silent_peers": n_peers, "per_key": per_key,
        "other_ops_not_sent": other_ops, "violations": violations, "one_node_get_per_key_while_silent": violations.is_empty() && !sends.is_empty(),
        "b_logs_kept_at": keep,
    }}));
    drop(c);
    net.finish()?;
    Ok(())
}
