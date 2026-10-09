//! IS A CLIENT'S GET IN PROGRESS OFF THE NODE'S ONE CLIENT QUEUE? (sdk#447, the architect's ruling (1))
//!
//! The page counts the ops on the wire ahead of a first send as places in the node's one queue for this client (F61,
//! sdk#390). sdk#447 leaves a SILENT GET (the node is still fetching it) out of that count, on the claim that the node
//! took it off its client queue once it dispatched it. V20 cannot show that: its client was the node's HTTP web path,
//! one transient client per request. This measures it on ONE websocket, as the page has one.
//!
//! TWO private nodes on loopback (never 7509/7609): A an isolated gateway with a throwaway transport key, B joined to
//! A only. B holds a block (PUT and answered first). Then A is PAUSED (SIGSTOP, by the handle it was spawned with): a
//! SILENT PEER, so a GET B cannot answer locally goes out to A and gets no answer (F64: up to its attempts × 60 s).
//! On B's one socket: that GET FIRST, then a GET of the block B holds, then a PUT. Every answer is timed and named by
//! the key it carries. The held GET answered while the first is unanswered = a GET in progress does not hold this
//! client's queue. A is resumed and both nodes stopped on every exit.
//!
//! Every line is JSON; the verdict line is last.
//!
//! `--silent-peers N` (default 1): N-1 more private nodes join A, and b's own connections are listed before any is
//! paused; ALL N are paused. The node's retry fallback asks one silent peer at most twice (F64), so N distinct silent
//! peers are what reach its full attempt bound.
//!
//! usage: get-in-flight <block.wasm> <dir> [--base PORT] [--watch SECS] [--silent-peers N]
use anyhow::{bail, Context, Result};
use freenet_stdlib::client_api::{ClientRequest, ContractRequest, ContractResponse, HostResponse};
use freenet_stdlib::prelude::*;
use probe::silent::arg;
use probe::signer::{container, contract_op, raw_block};
use std::time::{Duration, Instant};
use tokio::time::timeout;

fn random_body() -> Vec<u8> {
    let mut body = vec![0u8; 700];
    getrandom::getrandom(&mut body).expect("random");
    body
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let usage = "usage: get-in-flight <block.wasm> <dir> [--base PORT] [--watch SECS] [--silent-peers N]";
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (Some(bw), Some(dir)) = (a.first(), a.get(1)) else { bail!("{usage}") };
    let watch = Duration::from_secs(arg(&a, "--watch").map(|v| v.parse()).transpose()?.unwrap_or(90));
    let bcode = std::fs::read(bw).with_context(|| format!("reading {bw}"))?;
    let (_tree, mut net, mut c, peers) = probe::silent::open(&a, dir, 47_500, None).await?;
    let n_peers = peers.len();
    println!("{}", serde_json::json!({ "b": net.b.ws(), "b_connected_to": peers }));

    // SETUP: a block B HOLDS (PUT through B, answered, while A still answers).
    let (held_id, held_state) = raw_block(random_body());
    let held = container(&bcode, &held_id);
    let r = contract_op(&mut c, ContractRequest::Put { contract: held.clone(), state: WrappedState::new(held_state), related_contracts: Default::default(), subscribe: false, blocking_subscribe: false }).await?;
    println!("{}", serde_json::json!({ "setup_put_on_b": r }));

    // Every peer goes SILENT.
    let paused = net.pause()?;
    println!("{}", serde_json::json!({ "paused": paused, "as": "SIGSTOP: silent peers" }));

    let (never_id, _) = raw_block(random_body());
    let never = container(&bcode, &never_id).key();
    let (put_id, put_state) = raw_block(random_body());
    let put = container(&bcode, &put_id);
    let names = [(*never.id(), "silent-get"), (*held.key().id(), "held-get"), (*put.key().id(), "later-put")];
    let name = |id: &ContractInstanceId| names.iter().find(|(k, _)| k == id).map_or("?", |(_, n)| *n);
    let get = |id: ContractInstanceId| ClientRequest::ContractOp(ContractRequest::Get { key: id, return_contract_code: false, subscribe: false, blocking_subscribe: false });

    let t0 = Instant::now();
    let mut answered: Vec<(&str, u128, String)> = Vec::new();
    c.send(get(*never.id())).await?;
    println!("{}", serde_json::json!({ "sent": "silent-get", "ms": t0.elapsed().as_millis() }));
    // Anything the node says in the next 3 s is heard (and timed) before the later ops go out.
    let mut plan: Vec<(u128, &str)> = vec![(3_000, "held-get"), (3_000, "later-put")];
    let end = tokio::time::Instant::now() + watch;
    while tokio::time::Instant::now() < end && answered.len() < 3 {
        while let Some((at, what)) = plan.first().copied() {
            if t0.elapsed().as_millis() < at {
                break;
            }
            plan.remove(0);
            match what {
                "held-get" => c.send(get(*held.key().id())).await?,
                _ => c.send(ClientRequest::ContractOp(ContractRequest::Put { contract: put.clone(), state: WrappedState::new(put_state.clone()), related_contracts: Default::default(), subscribe: false, blocking_subscribe: false })).await?,
            }
            println!("{}", serde_json::json!({ "sent": what, "ms": t0.elapsed().as_millis() }));
        }
        let got = match timeout(Duration::from_millis(100), c.recv()).await {
            Err(_) => continue,
            Ok(r) => r,
        };
        let ms = t0.elapsed().as_millis();
        let (who, what) = match got {
            Ok(HostResponse::ContractResponse(ContractResponse::GetResponse { key, state, .. })) => (name(key.id()), format!("state {} bytes", state.size())),
            Ok(HostResponse::ContractResponse(ContractResponse::NotFound { instance_id })) => (name(&instance_id), "not found".into()),
            Ok(HostResponse::ContractResponse(ContractResponse::PutResponse { key })) => (name(key.id()), "put".into()),
            Ok(other) => ("?", format!("{other:?}").chars().take(160).collect()),
            Err(e) => ("?", format!("error: {e}").chars().take(200).collect()),
        };
        println!("{}", serde_json::json!({ "answer": who, "what": what, "ms": ms }));
        answered.push((who, ms, what));
    }
    let at = |n: &str| answered.iter().find(|(w, _, _)| *w == n).map(|(_, ms, _)| *ms);
    let silent = at("silent-get");
    let before = |l: Option<u128>| match (l, silent) {
        (Some(l), Some(s)) => l < s,
        (Some(_), None) => true,
        _ => false,
    };
    println!("{}", serde_json::json!({ "verdict": {
        "silent_get_ms": silent, "held_get_ms": at("held-get"), "later_put_ms": at("later-put"), "watched_s": watch.as_secs(), "silent_peers": n_peers,
        "held_get_answered_while_the_silent_get_was_unanswered": before(at("held-get")),
        "later_put_answered_while_the_silent_get_was_unanswered": before(at("later-put")),
    }}));
    drop(c);
    net.finish()?;
    Ok(())
}
