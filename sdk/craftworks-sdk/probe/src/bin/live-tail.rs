//! LIVE: does a real freenet node run the Tail as designed (ARCHITECTURE §1, Tail)?
//!
//! On a PRIVATE isolated node (never the owner's): create a tail, subscribe from a second connection, send signed
//! deltas (writes, a flush into a local prolly tree, more writes), try an old and a skipping delta, and read the tail
//! back from a fresh connection. Every check prints PASS or FAIL; the exit code is non-zero if any fails.
//!
//! The flush's tree blocks are built locally and NOT put on the network: this run is about the tail contract.
//!
//! usage: live-tail <tail.wasm> [ws_port]
use anyhow::{bail, Context, Result};
use craftec_register_contract::wire::{Params, Signed};
use ed25519_dalek::{Signer, SigningKey};
use freenet_prolly::store::MemBlocks;
use freenet_stdlib::client_api::{
    ClientRequest, ContractRequest, ContractResponse, HostResponse, WebApi,
};
use freenet_stdlib::prelude::*;
use probe::node::{Mode, Node, TempTree};
use std::time::{Duration, Instant};
use tail::{flush_into, Op, Unsigned, Writer};
use tokio::time::timeout;

const STEP: Duration = Duration::from_secs(20);

async fn connect(ws: &str) -> Result<WebApi> {
    let (stream, _) = tokio_tungstenite::connect_async(ws)
        .await
        .context("connecting to the node")?;
    Ok(WebApi::start(stream))
}

/// Wait for the first answer `pick` accepts; other messages are skipped.
async fn wait<T>(
    c: &mut WebApi,
    what: &str,
    mut pick: impl FnMut(HostResponse) -> Option<T>,
) -> Result<T> {
    let by = Instant::now() + STEP;
    while Instant::now() < by {
        match timeout(by - Instant::now(), c.recv()).await {
            Ok(Ok(r)) => {
                if let Some(t) = pick(r) {
                    return Ok(t);
                }
            }
            Ok(Err(e)) => bail!("{what}: node error: {e}"),
            Err(_) => break,
        }
    }
    bail!("{what}: no answer within {STEP:?}")
}

async fn get(c: &mut WebApi, id: ContractInstanceId) -> Result<Vec<u8>> {
    c.send(ClientRequest::ContractOp(ContractRequest::Get {
        key: id,
        return_contract_code: false,
        subscribe: false,
        blocking_subscribe: false,
    }))
    .await?;
    wait(c, "GET", |r| match r {
        HostResponse::ContractResponse(ContractResponse::GetResponse { key, state, .. })
            if *key.id() == id =>
        {
            Some(state.as_ref().to_vec())
        }
        _ => None,
    })
    .await
}

/// Send one delta; `Ok(ms)` when the node answers the UPDATE.
async fn update(c: &mut WebApi, key: &ContractKey, delta: Vec<u8>) -> Result<u64> {
    let t = Instant::now();
    c.send(ClientRequest::ContractOp(ContractRequest::Update {
        key: *key,
        data: UpdateData::Delta(StateDelta::from(delta)),
    }))
    .await?;
    let id = *key.id();
    wait(c, "UPDATE", |r| match r {
        HostResponse::ContractResponse(ContractResponse::UpdateResponse { key, .. })
            if *key.id() == id =>
        {
            Some(())
        }
        _ => None,
    })
    .await?;
    Ok(t.elapsed().as_millis() as u64)
}

fn sign(sk: &SigningKey, u: &Unsigned) -> Signed {
    Signed {
        terminal: false,
        seq: u.seq,
        value_hash: u.body.hash(),
        bitmap: 0,
        sigs: vec![sk.sign(&u.message).to_bytes()],
    }
}

fn step(sk: &SigningKey, w: &mut Writer, ops: Vec<Op>) -> Result<Vec<u8>> {
    let u = w
        .prepare(ops)
        .context("the contract would refuse this step")?;
    let s = sign(sk, &u);
    w.commit(u, s)
        .context("the writer refused its own signature")
}

fn set(k: &str, v: &str) -> Op {
    Op::Set {
        key: k.as_bytes().to_vec(),
        value: v.as_bytes().to_vec(),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let wasm_path = std::env::args()
        .nth(1)
        .context("usage: live-tail <tail.wasm> [ws_port]")?;
    let port: u16 = std::env::args()
        .nth(2)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(17561);
    probe::node::allowed_ports(&[port, port + 20000])?;
    let code = std::fs::read(&wasm_path).with_context(|| format!("reading {wasm_path}"))?;
    println!("freenet: {}", probe::node::freenet_version()?);
    println!(
        "tail.wasm: {} bytes, blake3 {}",
        code.len(),
        blake3::hash(&code).to_hex()
    );

    let dir = std::env::temp_dir().join(format!("live-tail-{}", std::process::id()));
    let _tree = TempTree(dir.clone());
    let node = Node::spawn_in(
        port,
        &dir,
        Mode::IsolatedNetwork {
            network_port: port + 20000,
        },
    )?;
    println!("node: private, isolated network mode, ws 127.0.0.1:{port}");

    // The writer's key and the tail's params: the Register's params, single key.
    let sk = SigningKey::from_bytes(blake3::hash(b"live-tail writer key").as_bytes());
    let params = wire::register_params(&sk.verifying_key().to_bytes(), b"tail");
    let p = Params::parse(&params).context("params")?;
    let mut w = Writer::new(&params).context("writer")?;

    let container = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(code.clone())),
        Parameters::from(params.clone()),
    )));
    let key = container.key();
    let id = *key.id();
    println!("tail: {id}");

    let mut fails = 0u32;
    let mut check = |ok: bool, what: &str| {
        println!("{} {what}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            fails += 1;
        }
    };

    // 1. Create the tail, empty.
    let mut a = connect(&node.ws()).await?;
    let t = Instant::now();
    a.send(ClientRequest::ContractOp(ContractRequest::Put {
        contract: container,
        state: WrappedState::new(Vec::new()),
        related_contracts: RelatedContracts::default(),
        subscribe: false,
        blocking_subscribe: false,
    }))
    .await?;
    wait(&mut a, "PUT", |r| {
        matches!(
            r,
            HostResponse::ContractResponse(ContractResponse::PutResponse { .. })
        )
        .then_some(())
    })
    .await?;
    println!("put: empty tail created in {} ms", t.elapsed().as_millis());

    // 2. A subscriber on another connection.
    let mut s = connect(&node.ws()).await?;
    s.send(ClientRequest::ContractOp(ContractRequest::Subscribe {
        key: id,
        summary: None,
    }))
    .await?;
    wait(&mut s, "SUBSCRIBE", |r| {
        matches!(
            r,
            HostResponse::ContractResponse(ContractResponse::SubscribeResponse { .. })
        )
        .then_some(())
    })
    .await?;
    println!("subscribe: second connection subscribed");
    let mut seen: Option<tail::Tail> = None;
    let mut notes = 0u32;

    // 3. Writes, a flush, more writes. The subscriber folds every notification through the contract's own merge.
    let mut blocks = MemBlocks::default();
    let mut sent = Vec::new();
    let plan: Vec<Vec<Op>> = vec![
        vec![set("a", "1")],
        vec![set("b", "2"), set("c", "3")],
        vec![Op::Delete { key: b"a".to_vec() }],
        vec![], // flush
        vec![set("d", "4")],
        vec![set("b", "22")],
    ];
    for ops in plan {
        let ops = if ops.is_empty() {
            let (applied, op) = flush_into(&mut blocks, &w.body(), w.seq())
                .map_err(|e| anyhow::anyhow!("flush: {e:?}"))?;
            println!(
                "flush: tree root {} (local blocks only)",
                hex(&applied.root)
            );
            vec![op]
        } else {
            ops
        };
        let d = step(&sk, &mut w, ops)?;
        let ms = update(&mut a, &key, d.clone()).await?;
        sent.push(d);
        // The notification for this step.
        let got = timeout(Duration::from_secs(10), async {
            loop {
                match s.recv().await {
                    Ok(HostResponse::ContractResponse(ContractResponse::UpdateNotification {
                        update,
                        ..
                    })) => return Some(update),
                    Ok(_) => continue,
                    Err(_) => return None,
                }
            }
        })
        .await
        .ok()
        .flatten();
        let what = match &got {
            Some(u) => {
                notes += 1;
                let bytes: Vec<Vec<u8>> = match u {
                    UpdateData::State(x) => vec![x.as_ref().to_vec()],
                    UpdateData::Delta(x) => vec![x.as_ref().to_vec()],
                    UpdateData::StateAndDelta { state, delta } => {
                        vec![state.as_ref().to_vec(), delta.as_ref().to_vec()]
                    }
                    _ => vec![],
                };
                let kind = match u {
                    UpdateData::State(_) => "state",
                    UpdateData::Delta(_) => "delta",
                    UpdateData::StateAndDelta { .. } => "state+delta",
                    _ => "other",
                };
                for b in bytes {
                    seen = craftec_tail_contract::absorb(seen.take(), &b, &p);
                }
                format!("notified ({kind}, {} B)", u.size())
            }
            None => "NO notification within 10 s".into(),
        };
        println!(
            "seq {}: update answered in {ms} ms, delta {} B; subscriber {what}",
            w.seq(),
            sent.last().unwrap().len()
        );
    }
    check(
        notes as usize == sent.len(),
        &format!(
            "subscriber was notified of every delta ({notes}/{})",
            sent.len()
        ),
    );
    check(
        seen.as_ref().map(|t| t.encode(&p.authority)) == Some(w.state()),
        "subscriber's copy equals the writer's tail",
    );

    // 4. What the node holds, read by a fresh connection.
    let mut c = connect(&node.ws()).await?;
    let held = get(&mut c, id).await?;
    check(
        held == w.state(),
        &format!(
            "node serves exactly the writer's tail (seq {}, {} B)",
            w.seq(),
            held.len()
        ),
    );
    let body = craftec_tail_contract::read(&params, &held)
        .and_then(|(_, t)| t)
        .map(|t| t.body);
    check(
        body.as_ref()
            .is_some_and(|b| b.root.is_some() && b.get(b"a").is_none()),
        "after the flush, row a left the tail (it is in the tree)",
    );
    check(
        body.as_ref()
            .is_some_and(|b| tail::get(&blocks, b, b"b") == Ok(Some(b"22".to_vec()))),
        "row b reads its newest value (tail over tree)",
    );
    check(
        body.as_ref()
            .is_some_and(|b| tail::get(&blocks, b, b"c") == Ok(Some(b"3".to_vec()))),
        "row c reads from the tree",
    );

    // 5. An old delta and a skipping one change nothing.
    let _ = update(&mut a, &key, sent[1].clone()).await;
    let mut ahead = w.clone();
    let _skipped = step(&sk, &mut ahead, vec![set("x", "1")])?;
    let skip = step(&sk, &mut ahead, vec![set("y", "2")])?;
    let _ = update(&mut a, &key, skip).await;
    let after = get(&mut c, id).await?;
    check(
        after == w.state(),
        "an old delta and a delta skipping a seq left the tail unchanged",
    );

    drop(node);
    if fails > 0 {
        bail!("{fails} check(s) failed");
    }
    println!("RESULT passes");
    Ok(())
}

fn hex(b: &[u8]) -> String {
    core_types::hex::encode(&b[..b.len().min(8)])
}
