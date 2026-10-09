//! KEEPER §11 (a) and (b), LIVE: what an `AskHeld` (the signer's `Held` read, the page's presence query) costs, and
//! whether it searches.
//!
//! Source, freenet v0.2.136 `crates/core/src/wasm_runtime/native_api.rs:1172` (`local_contract_state`): the code
//! hash by instance id, then `state_store_db.get_state_sync`: the node's own store, never a GET. This measures it.
//!
//! On node A (the asker; its signer is registered here, `Held` needs no provisioning), three ARMS of `--n` blocks:
//! * `held`: PUT through A;
//! * `elsewhere`: PUT through B only (it exists on the network; A is asked whether IT holds it);
//! * `never`: random ids no one ever PUT.
//!
//! Each block is asked ONCE PER OP (as the page asks), `--repeats` times, and then all of an arm in ONE op (up to
//! MAX_HELD), to show the per-op cost. After the Held asks (a GET makes the node hold what it fetches, F6), one
//! plain GET per `elsewhere` and `never` id, for contrast: what a SEARCH costs.
//!
//! (b) `--ids <file>` (one hex BLOCK id per line) or `--contracts <file>` (one contract instance id per line, base58,
//! as `classify-frames` prints a PUT's contract): AskHeld each, one per op, then batched; the total and per-block
//! cost of a full pass over those blocks.
//!
//! (c) `--put N --out FILE`: PUT N random blocks through A, write their block ids to FILE (one hex per line), and ask
//! Held for them once at once. A later `--ids FILE` run on the same node says how many it still holds: how long a node
//! keeps a block nobody asks for.
//!
//! Every line is JSON; the verdict line is last. Refuses 7509/7609.
//!
//! usage: live-held <A ws-url> <B ws-url | -> <signer.wasm> <block.wasm> [--n N] [--repeats R] [--ids FILE | --contracts FILE | --put N --out FILE]
use anyhow::{bail, Context, Result};
use freenet_stdlib::client_api::{ContractRequest, HostResponse, WebApi, ContractResponse, ClientRequest};
use freenet_stdlib::prelude::*;
use probe::signer::{ask, connect, container, contract_op_within, raw_block, register_delegate};
use signer::{Answer, Request};
use std::time::{Duration, Instant};
use tokio::time::timeout;

/// A setup PUT is waited on longer than a step: the real network's PUT tail (~60 s) is not what this measures.
const PUT_WAIT: Duration = Duration::from_secs(180);

fn random_block() -> ([u8; 32], Vec<u8>) {
    let mut body = vec![0u8; 700];
    getrandom::getrandom(&mut body).expect("random");
    raw_block(body)
}

fn random_id() -> [u8; 32] {
    let mut id = [0u8; 32];
    getrandom::getrandom(&mut id).expect("random");
    id
}

/// One Held ask about `contracts`; its wall time in µs and the answer's `present` list.
async fn held(c: &mut WebApi, key: &DelegateKey, contracts: Vec<[u8; 32]>) -> Result<(u128, Vec<bool>)> {
    let t = Instant::now();
    let a = ask(c, key, &Request::Held { contracts }).await?;
    let us = t.elapsed().as_micros();
    match a {
        Answer::Held { present } => Ok((us, present)),
        other => bail!("Held was answered {other:?}"),
    }
}

/// One plain GET of a block's contract through `c`: ms to an answer, or "not within" the deadline.
async fn get(c: &mut WebApi, key: ContractKey, within: Duration) -> Result<serde_json::Value> {
    let t = Instant::now();
    c.send(ClientRequest::ContractOp(ContractRequest::Get {
        key: *key.id(),
        return_contract_code: false,
        subscribe: false,
        blocking_subscribe: false,
    }))
    .await?;
    let end = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < end {
        match timeout(Duration::from_millis(250), c.recv()).await {
            // What the answer CARRIED: a GetResponse's state size (0 is not a block), or NotFound, never taken for
            // "found" by its kind alone.
            Ok(Ok(HostResponse::ContractResponse(ContractResponse::GetResponse { state, .. }))) => {
                return Ok(serde_json::json!({ "answer": "state", "bytes": state.size(), "ms": t.elapsed().as_millis() }));
            }
            Ok(Ok(HostResponse::ContractResponse(ContractResponse::NotFound { .. }))) => {
                return Ok(serde_json::json!({ "answer": "not found", "ms": t.elapsed().as_millis() }));
            }
            Ok(Ok(_)) => {}
            Ok(Err(e)) => return Ok(serde_json::json!({ "answer": format!("error: {e}"), "ms": t.elapsed().as_millis() })),
            Err(_) => {}
        }
    }
    Ok(serde_json::json!({ "answer": format!("not within {} s", within.as_secs()) }))
}

fn arg(a: &[String], name: &str) -> Option<String> {
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1)).cloned()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let usage = "usage: live-held <A ws-url> <B ws-url | -> <signer.wasm> <block.wasm> [--n N] [--repeats R] [--ids FILE | --contracts FILE]";
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (Some(wa), Some(wb), Some(sw), Some(bw)) = (a.first(), a.get(1), a.get(2), a.get(3)) else { bail!("{usage}") };
    let n: usize = arg(&a, "--n").map(|v| v.parse()).transpose()?.unwrap_or(5);
    let repeats: usize = arg(&a, "--repeats").map(|v| v.parse()).transpose()?.unwrap_or(3);
    let signer_wasm = std::fs::read(sw).with_context(|| format!("reading {sw}"))?;
    let bcode = std::fs::read(bw).with_context(|| format!("reading {bw}"))?;
    let mut ca = connect(wa).await?;
    let key = register_delegate(&mut ca, &signer_wasm).await?;
    let contract_of = |id: &[u8; 32]| wire::block::contract_for(&bcode, id);

    // (c): PUT a set, write its ids, ask once now.
    if let Some(k) = arg(&a, "--put") {
        let k: usize = k.parse()?;
        let out = arg(&a, "--out").context("--put needs --out FILE")?;
        let mut ids = Vec::new();
        for i in 0..k {
            let (id, st) = random_block();
            let r = contract_op_within(&mut ca, ContractRequest::Put { contract: container(&bcode, &id), state: WrappedState::new(st), related_contracts: Default::default(), subscribe: false, blocking_subscribe: false }, PUT_WAIT).await?;
            println!("{}", serde_json::json!({ "put": "A", "i": i, "answer": r }));
            ids.push(id);
        }
        std::fs::write(&out, ids.iter().map(|id| core_types::hex::encode(id)).collect::<Vec<_>>().join("\n") + "\n")?;
        let (us, p) = held(&mut ca, &key, ids.iter().map(contract_of).collect()).await?;
        println!("{}", serde_json::json!({ "put_set": { "blocks": k, "file": out, "present_now": p.iter().filter(|x| **x).count(), "ask_us": us } }));
        return Ok(());
    }

    // (b): a full pass over the given block ids, on A alone.
    let pass: Option<Vec<[u8; 32]>> = if let Some(f) = arg(&a, "--ids") {
        Some(std::fs::read_to_string(&f)?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| core_types::hex::decode(l.trim()).and_then(|b| b.try_into().ok()).map(|id| contract_of(&id)).context(format!("not a 32-byte hex id: {l}")))
            .collect::<Result<_>>()?)
    } else if let Some(f) = arg(&a, "--contracts") {
        Some(std::fs::read_to_string(&f)?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let c: ContractInstanceId = l.trim().parse().map_err(|e| anyhow::anyhow!("not a contract instance id: {l}: {e:?}"))?;
                <[u8; 32]>::try_from(c.as_bytes()).map_err(|_| anyhow::anyhow!("not 32 bytes: {l}"))
            })
            .collect::<Result<_>>()?)
    } else {
        None
    };
    if let Some(ids) = pass {
        let t = Instant::now();
        let mut present = 0;
        let mut per = Vec::new();
        for id in &ids {
            let (us, p) = held(&mut ca, &key, vec![*id]).await?;
            per.push(us);
            present += p.iter().filter(|x| **x).count();
        }
        let one_per_op_ms = t.elapsed().as_millis();
        let t = Instant::now();
        let mut batched_present = 0;
        let mut batches = 0;
        for chunk in ids.chunks(signer::MAX_HELD) {
            let (_, p) = held(&mut ca, &key, chunk.to_vec()).await?;
            batched_present += p.iter().filter(|x| **x).count();
            batches += 1;
        }
        let batched_ms = t.elapsed().as_millis();
        per.sort();
        println!("{}", serde_json::json!({
            "pass": { "blocks": ids.len(), "present": present,
                      "one_per_op": { "ops": ids.len(), "total_ms": one_per_op_ms, "per_block_us": { "min": per.first(), "median": per.get(per.len() / 2), "max": per.last() } },
                      "batched": { "ops": batches, "max_per_op": signer::MAX_HELD, "total_ms": batched_ms, "present": batched_present } }
        }));
        return Ok(());
    }

    // (a): the three arms.
    if wb == "-" {
        bail!("(a) needs B's ws url: the `elsewhere` arm is PUT through B");
    }
    let mut cb = connect(wb).await?;
    let mut arms: Vec<(&str, Vec<[u8; 32]>)> = vec![("held", vec![]), ("elsewhere", vec![]), ("never", vec![])];
    for i in 0..n {
        let (id, st) = random_block();
        let r = contract_op_within(&mut ca, ContractRequest::Put { contract: container(&bcode, &id), state: WrappedState::new(st), related_contracts: Default::default(), subscribe: false, blocking_subscribe: false }, PUT_WAIT).await?;
        println!("{}", serde_json::json!({ "put": "A", "i": i, "answer": r }));
        arms[0].1.push(id);
        let (id, st) = random_block();
        let r = contract_op_within(&mut cb, ContractRequest::Put { contract: container(&bcode, &id), state: WrappedState::new(st), related_contracts: Default::default(), subscribe: false, blocking_subscribe: false }, PUT_WAIT).await?;
        println!("{}", serde_json::json!({ "put": "B", "i": i, "answer": r }));
        arms[1].1.push(id);
        arms[2].1.push(random_id());
    }
    let mut table = serde_json::Map::new();
    for (arm, ids) in &arms {
        let mut per_repeat = Vec::new();
        for r in 0..repeats {
            let mut us = Vec::new();
            let mut present = 0;
            for id in ids {
                let (t, p) = held(&mut ca, &key, vec![contract_of(id)]).await?;
                us.push(t);
                present += p.iter().filter(|x| **x).count();
            }
            us.sort();
            let median = us[us.len() / 2];
            println!("{}", serde_json::json!({ "arm": arm, "repeat": r, "one_per_op_us": us, "present": present }));
            per_repeat.push(median);
        }
        let (_, batch_present) = held(&mut ca, &key, ids.iter().map(contract_of).collect()).await?;
        let t = Instant::now();
        let _ = held(&mut ca, &key, ids.iter().map(contract_of).collect()).await?;
        let batch_us = t.elapsed().as_micros();
        let lo = *per_repeat.iter().min().unwrap();
        let hi = *per_repeat.iter().max().unwrap();
        table.insert(arm.to_string(), serde_json::json!({ "median_us_per_repeat": per_repeat, "spread_us": hi - lo, "present_of": [batch_present.iter().filter(|x| **x).count(), ids.len()], "one_op_for_all_us": batch_us }));
    }
    // Contrast: a SEARCH (a plain GET) for what A does not hold -- after every Held ask, since a GET makes A hold it.
    let mut gets = serde_json::Map::new();
    for (arm, ids) in arms.iter().skip(1) {
        let mut out = Vec::new();
        for id in ids {
            out.push(get(&mut ca, crate_key(&bcode, id), Duration::from_secs(30)).await?);
        }
        gets.insert(arm.to_string(), serde_json::Value::Array(out));
    }
    println!("{}", serde_json::json!({ "verdict": { "held": table, "get_after": gets, "n": n, "repeats": repeats } }));
    Ok(())
}

fn crate_key(bcode: &[u8], id: &[u8; 32]) -> ContractKey {
    container(bcode, id).key()
}
