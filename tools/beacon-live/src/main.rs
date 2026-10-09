//! TOKEN step 4, LIVE: the beacon on two nodes.
use anyhow::{bail, Context, Result};
use craftworks_beacon_contract as B;
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


fn desc(s: &Option<B::State>) -> String {
    s.as_ref().map_or("none".into(), |s| format!("height {} ({} alt, {} proof)", s.height(), s.alts.len(), s.frauds.len()))
}

#[tokio::main]
async fn main() -> Result<()> {
    // beacon-live <beacon.wasm> <ws X> <ws Y> [real]
    let a: Vec<String> = std::env::args().collect();
    let code = std::fs::read(&a[1])?;
    let (x, y) = (a[2].as_str(), a[3].as_str());
    let seed: [u8; 32] = *blake3::hash(&SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_nanos().to_le_bytes()).as_bytes();
    if a.get(4).map(String::as_str) == Some("real") {
        let p = B::Params { k: 2048 * 524_288, segs: 2048, sample: 8, window: 2, keep: 1440, seed };
        let c = container(&code, p.encode());
        let t = Instant::now();
        let s = B::State::default().extend(&p, fresh_mix());
        let secs = t.elapsed().as_secs_f64();
        let bytes = B::wire::encode(&s);
        let (said, ms) = put(x, c, bytes.clone()).await?;
        println!("REAL step: K = {} computed in {secs:.1} s here; state {} B; PUT on X: {said} ({ms} ms, the node validating it: 8 of 2048 segments re-run)", p.k, bytes.len());
        return Ok(());
    }
    let p = B::Params { k: 1 << 24, segs: 2048, sample: 8, window: 2, keep: 1440, seed };
    let c = || container(&code, p.encode());
    let id: [u8; 32] = c().key().id().as_bytes().try_into().unwrap();
    let mut s = B::State::default();
    for (n, ws, name) in [(1, x, "X"), (2, y, "Y"), (3, x, "X")] {
        let t = Instant::now();
        s = s.extend(&p, fresh_mix());
        let secs = t.elapsed().as_secs_f64();
        let (said, ms) = put(ws, c(), B::wire::encode(&s)).await?;
        println!("{n}  step {} computed in {secs:.2} s, PUT on {name}: {said} ({ms} ms)", s.height());
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    let (gx, gy) = (get(x, id).await?.and_then(|b| B::wire::decode(&b)), get(y, id).await?.and_then(|b| B::wire::decode(&b)));
    println!("   X holds {} · Y holds {} · same: {}", desc(&gx), desc(&gy), gx.is_some() && gx == gy);

    // A FAKE step 4 that slips past the sample: its output forged, the root ground until the sample misses it.
    let honest4 = B::Step::compute(&p, s.out(&p), 4, fresh_mix());
    let mut fake = None;
    for t in 0u32.. {
        let mut st = honest4.clone();
        let pts = st.points.as_mut().unwrap();
        let j = pts.len() - 1;
        pts[j] = *blake3::hash(&t.to_le_bytes()).as_bytes();
        st.root = B::root(pts);
        st.out = pts[j];
        if !B::samples(&p, &st.root, &st.prev, 4).contains(&j) {
            fake = Some((st, t));
            break;
        }
    }
    let (fake, tries) = fake.unwrap();
    let mut fs = s.clone();
    fs.steps.push(fake.clone());
    let fs = B::normalize(&p, fs);
    let fake_ahead = fs.extend(&p, fresh_mix());
    let (said, ms) = put(y, c(), B::wire::encode(&fake_ahead)).await?;
    println!("4  FAKE step 4 (output forged, {tries} root tries to dodge the sample) + an honest step 5 on it, PUT on Y: {said} ({ms} ms)");
    tokio::time::sleep(Duration::from_secs(5)).await;
    let (gx, gy) = (get(x, id).await?.and_then(|b| B::wire::decode(&b)), get(y, id).await?.and_then(|b| B::wire::decode(&b)));
    println!("   X holds {} · Y holds {} — the cheap fake leads until proven", desc(&gx), desc(&gy));

    // A verifier recomputes step 4 and proves it; the honest step 4 comes too.
    let t = Instant::now();
    let fr = fake.find_fraud(&p).expect("found");
    let found = t.elapsed().as_secs_f64();
    let mut proof = s.clone();
    proof.frauds.push(fr);
    let mut with_honest = proof.clone();
    with_honest.steps.push(honest4);
    let with_honest = B::normalize(&p, with_honest);
    let (said, ms) = put(x, c(), B::wire::encode(&with_honest)).await?;
    println!("5  fraud proof (found in {found:.2} s) + the honest step 4, PUT on X: {said} ({ms} ms)");
    for t in [5u64, 15] {
        tokio::time::sleep(Duration::from_secs(if t == 5 { 5 } else { 10 })).await;
        let (gx, gy) = (get(x, id).await?.and_then(|b| B::wire::decode(&b)), get(y, id).await?.and_then(|b| B::wire::decode(&b)));
        let honest_top = |g: &Option<B::State>| g.as_ref().is_some_and(|s| s.steps.iter().any(|st| st.height == 4 && st.root == with_honest.steps.last().unwrap().root));
        println!("   after ~{t} s: X holds {} (honest step 4: {}) · Y holds {} (honest step 4: {}) · same: {}", desc(&gx), honest_top(&gx), desc(&gy), honest_top(&gy), gx.is_some() && gx == gy);
    }
    Ok(())
}

/// An extender's MIX: fresh entropy for the step it adds (the clock's nanoseconds and a counter, hashed).
fn fresh_mix() -> [u8; 32] {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    *blake3::hash(&[&t.to_le_bytes()[..], &n.to_le_bytes()].concat()).as_bytes()
}
