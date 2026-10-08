//! TOKEN step 3, LIVE: the standard's contract on two nodes.
use anyhow::{bail, Context, Result};
use craftworks_token_contract as T;
use ed25519_dalek::{Signer, SigningKey};
use freenet_stdlib::prelude::*;
use futures::{SinkExt, StreamExt};
use std::time::{Duration, Instant, SystemTime};
use tokio_tungstenite::tungstenite::Message;

fn container(code: &[u8], params: Vec<u8>) -> ContractContainer {
    ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(code.to_vec())),
        Parameters::from(params),
    )))
}

async fn put(ws: &str, c: ContractContainer, state: Vec<u8>) -> Result<(String, u128)> {
    let (mut tx, mut rx) = tokio_tungstenite::connect_async(ws).await.context("connecting")?.0.split();
    let t = Instant::now();
    for f in wire::frame_put(c, WrappedState::new(state), 2).map_err(|e| anyhow::anyhow!(e))? {
        tx.send(Message::Binary(f.into())).await?;
    }
    let mut r = wire::Reassembler::new();
    // ONE deadline for the whole PUT (a per-message timeout restarts on every unrelated message).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let mut seen: Vec<String> = Vec::new();
    let said = loop {
        match tokio::time::timeout_at(deadline, rx.next()).await {
            Err(_) => break format!("NO ANSWER in 90 s (other messages: {})", seen.join(", ")),
            Ok(None) => bail!("closed"),
            Ok(Some(Err(e))) => bail!("{e}"),
            Ok(Some(Ok(Message::Binary(b)))) => match wire::unframe(&mut r, &b) {
                wire::Incoming::Ack(wire::AckKind::Put(_)) => break "ACCEPTED".to_string(),
                wire::Incoming::PutFailed { said, .. } | wire::Incoming::PutFailedByText { said, .. } => break format!("REFUSED: {said}"),
                wire::Incoming::Refused(r) => break format!("REFUSED: {r:?}"),
                other => seen.push(format!("{:.60}", format!("{other:?}"))),
            },
            Ok(Some(Ok(m))) => seen.push(format!("{:.20}", format!("{m:?}"))),
        }
    };
    Ok((said, t.elapsed().as_millis()))
}


async fn get(ws: &str, id: [u8; 32]) -> Result<Option<Vec<u8>>> {
    let (mut tx, mut rx) = tokio_tungstenite::connect_async(ws).await.context("connecting")?.0.split();
    for f in wire::frame_get(wire::contract_id(id), false, 3).map_err(|e| anyhow::anyhow!(e))? {
        tx.send(Message::Binary(f.into())).await?;
    }
    let mut r = wire::Reassembler::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        match tokio::time::timeout_at(deadline, rx.next()).await {
            Err(_) => return Ok(None),
            Ok(None) => bail!("closed"),
            Ok(Some(Err(e))) => bail!("{e}"),
            Ok(Some(Ok(Message::Binary(b)))) => match wire::unframe(&mut r, &b) {
                wire::Incoming::Got { state, .. } => return Ok(Some(state.to_vec())),
                wire::Incoming::GetFailed { .. } => return Ok(None),
                _ => {}
            },
            Ok(Some(Ok(_))) => {}
        }
    }
}

fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}
fn pk(n: u8) -> [u8; 32] {
    key(n).verifying_key().to_bytes()
}
fn push(s: &mut T::State, body: T::Body, n: [u8; 16], by: Option<u8>) {
    let bal = s.balance();
    let balance = match &body {
        T::Body::Genesis { amount } => *amount,
        T::Body::Send { amount, .. } | T::Body::Burn { amount } => bal - amount,
        T::Body::Receive { amount, .. } => bal + amount,
    };
    let mut e = T::Entry { seq: s.entries.len() as u64, prev: s.head(), nonce: n, body, balance, auth: T::Auth::Work(0) };
    e.auth = match by {
        Some(k) => T::Auth::Sig(key(k).sign(&e.message()).to_bytes()),
        None => {
            let m = e.message();
            T::Auth::Work((0u64..).find(|w| T::work_ok(&m, *w)).unwrap())
        }
    };
    s.entries.push(e);
}

#[tokio::main]
async fn main() -> Result<()> {
    // token-live <token.wasm> <ws X> <ws Y> [net <rounds>]
    let a: Vec<String> = std::env::args().collect();
    let code = std::fs::read(&a[1])?;
    let (x, y) = (a[2].as_str(), a[3].as_str());
    if a.get(4).map(String::as_str) == Some("net") {
        return net(&code, x, y, a.get(5).and_then(|n| n.parse().ok()).unwrap_or(3)).await;
    }
    let ch = *blake3::hash(&code).as_bytes();
    let salt: [u8; 16] = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_nanos().to_le_bytes();
    let def = T::Definition { code_hash: ch, decimals: 8, min_send: 10, genesis_owner: pk(1), genesis_amount: 1000, name: b"Live".to_vec(), nonce: salt };
    let did = T::id(&ch, &def.encode());
    let chain = |o: u8| T::ChainParams { code_hash: ch, definition: did, owner: pk(o) };
    let cp = |o: u8| container(&code, chain(o).encode());
    let n = |k: u8| { let mut v = salt; v[15] ^= k; v };

    let (s, ms) = put(x, container(&code, def.encode()), def.encode()).await?;
    println!("1  definition on X: {s} ({ms} ms)");

    let mut ca = T::State::default();
    push(&mut ca, T::Body::Genesis { amount: 1000 }, n(1), Some(1));
    push(&mut ca, T::Body::Send { to: pk(2), amount: 300 }, n(2), Some(1));
    let (s, ms) = put(x, cp(1), ca.encode()).await?;
    println!("2  A: genesis 1000, send 300 to B, on X: {s} ({ms} ms)");

    let mut cb = T::State::default();
    push(&mut cb, T::Body::Receive { from: pk(1), nonce: n(2), amount: 300 }, n(2), None);
    let (s, ms) = put(y, cp(2), cb.encode()).await?;
    println!("3  B: receive it, on Y (fetches the definition and A): {s} ({ms} ms)");

    let mut twice = cb.clone();
    push(&mut twice, T::Body::Receive { from: pk(1), nonce: n(2), amount: 300 }, n(3), None);
    let (s, ms) = put(y, cp(2), twice.encode()).await?;
    let held = get(y, chain(2).id()).await?.and_then(|b| T::State::decode(&b));
    println!("4  B: receive it AGAIN, on Y: {s} ({ms} ms) — Y holds B with {} entr(ies), balance {}", held.as_ref().map_or(0, |h| h.entries.len()), held.as_ref().map_or(0, |h| h.balance()));

    let mut cc = T::State::default();
    push(&mut cc, T::Body::Receive { from: pk(1), nonce: n(9), amount: 50 }, n(9), None);
    let (s, ms) = put(y, cp(3), cc.encode()).await?;
    println!("5  C: receive a send that does not exist, on Y: {s} ({ms} ms)");

    let mut ax = ca.clone();
    push(&mut ax, T::Body::Send { to: pk(2), amount: 600 }, n(4), Some(1));
    let mut ay = ca.clone();
    push(&mut ay, T::Body::Send { to: pk(3), amount: 600 }, n(5), Some(1));
    let (s1, _) = put(x, cp(1), ax.encode()).await?;
    let (s2, _) = put(y, cp(1), ay.encode()).await?;
    println!("6  A signs two sends at seq 2: via X {s1}, via Y {s2}");
    for t in [5u64, 15, 30] {
        tokio::time::sleep(Duration::from_secs(if t == 5 { 5 } else { 10 })).await;
        let gx = get(x, chain(1).id()).await?.and_then(|b| T::State::decode(&b));
        let gy = get(y, chain(1).id()).await?.and_then(|b| T::State::decode(&b));
        let desc = |g: &Option<T::State>| g.as_ref().map_or("none".to_string(), |s| format!("{} entries, fork {}", s.entries.len(), if s.fork.is_some() { "YES" } else { "no" }));
        println!("   after ~{t} s: X holds {} · Y holds {} · same: {}", desc(&gx), desc(&gy), gx.is_some() && gx == gy);
    }
    Ok(())
}

/// THE REAL NETWORK (TOKEN step 4): per round, a fresh token — (1) A's chain on X, B's receive on Y: how long Y takes
/// to fetch A across the network and accept; (2) A signs two sends at one seq, one via X and one via Y at once: how
/// long until both nodes hold the same frozen chain (the finality margin's raw number).
async fn net(code: &[u8], x: &str, y: &str, rounds: u32) -> Result<()> {
    let ch = *blake3::hash(code).as_bytes();
    for r in 0..rounds {
        let salt: [u8; 16] = (SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_nanos() ^ r as u128).to_le_bytes();
        let def = T::Definition { code_hash: ch, decimals: 8, min_send: 10, genesis_owner: pk(1), genesis_amount: 1000, name: b"Net".to_vec(), nonce: salt };
        let did = T::id(&ch, &def.encode());
        let chain = |o: u8| T::ChainParams { code_hash: ch, definition: did, owner: pk(o) };
        let cp = |o: u8| container(code, chain(o).encode());
        let n = |k: u8| { let mut v = salt; v[15] ^= k; v };
        let (d, _) = put(x, container(code, def.encode()), def.encode()).await?;
        let mut ca = T::State::default();
        push(&mut ca, T::Body::Genesis { amount: 1000 }, n(1), Some(1));
        push(&mut ca, T::Body::Send { to: pk(2), amount: 300 }, n(2), Some(1));
        let (sa, msa) = put(x, cp(1), ca.encode()).await?;
        let mut cb = T::State::default();
        push(&mut cb, T::Body::Receive { from: pk(1), nonce: n(2), amount: 300 }, n(2), None);
        let (sb, msb) = put(y, cp(2), cb.encode()).await?;
        println!("round {r}: definition on X {d}; A on X {sa} ({msa} ms); B's receive on Y {sb} in {msb} ms (Y fetching the definition and A across the network)");
        // Two writer sends at seq 2, at once, through the two nodes.
        let mut ax = ca.clone();
        push(&mut ax, T::Body::Send { to: pk(2), amount: 600 }, n(4), Some(1));
        let mut ay = ca.clone();
        push(&mut ay, T::Body::Send { to: pk(3), amount: 600 }, n(5), Some(1));
        let t0 = Instant::now();
        let (rx, ry) = tokio::join!(put(x, cp(1), ax.encode()), put(y, cp(1), ay.encode()));
        let (rx, ry) = (rx?.0, ry?.0);
        let mut agreed = None;
        while t0.elapsed() < Duration::from_secs(240) {
            let (gx, gy) = tokio::join!(get(x, chain(1).id()), get(y, chain(1).id()));
            let (gx, gy) = (gx?.and_then(|b| T::State::decode(&b)), gy?.and_then(|b| T::State::decode(&b)));
            if gx.as_ref().is_some_and(|s| s.fork.is_some()) && gx == gy {
                agreed = Some(t0.elapsed().as_millis());
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        match agreed {
            Some(ms) => println!("         two sends at seq 2 (via X {rx}, via Y {ry}): both nodes the SAME frozen chain after {ms} ms"),
            None => println!("         two sends at seq 2 (via X {rx}, via Y {ry}): NOT agreed within 240 s"),
        }
    }
    Ok(())
}
