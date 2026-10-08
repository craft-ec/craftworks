//! TOKEN, LIVE on two nodes, with a beacon ticking: the core (step 3), allowances (5a), swaps with the append rules
//! (5b).
use anyhow::{bail, Context, Result};
use craftworks_beacon_contract as B;
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

/// The world of one run: the token code, the two nodes, the beacon (kept ticking by a task), a salt for fresh ids.
struct W {
    code: Vec<u8>,
    ch: [u8; 32],
    x: String,
    y: String,
    beacon: [u8; 32],
    salt: [u8; 16],
}

impl W {
    async fn now(&self) -> Result<u64> {
        Ok(get(&self.x, self.beacon).await?.and_then(|b| B::wire::decode(&b)).map_or(0, |s| s.height()))
    }
    /// Wait until the beacon reaches `h`.
    async fn until(&self, h: u64) -> Result<u64> {
        let t0 = Instant::now();
        loop {
            let n = self.now().await?;
            if n >= h {
                return Ok(n);
            }
            if t0.elapsed() > Duration::from_secs(300) {
                bail!("the beacon did not reach {h} (at {n})");
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    fn n(&self, k: u8) -> [u8; 16] {
        let mut v = self.salt;
        v[15] ^= k;
        v[14] ^= 0x5a;
        v
    }
    fn def(&self, genesis: u8, name: &[u8], k: u8) -> T::Definition {
        T::Definition { code_hash: self.ch, decimals: 8, min_send: 10, genesis_owner: pk(genesis), genesis_amount: 1000, name: name.to_vec(), nonce: self.n(k), beacon: self.beacon, margin: 2 }
    }
    fn chain(&self, token: [u8; 32], o: u8) -> T::ChainParams {
        T::ChainParams { code_hash: self.ch, definition: token, owner: pk(o), beacon: self.beacon }
    }
    async fn put_def(&self, d: &T::Definition) -> Result<[u8; 32]> {
        put(&self.x, container(&self.code, d.encode()), d.encode()).await?;
        Ok(T::id(&self.ch, &d.encode()))
    }
    async fn put_chain(&self, ws: &str, token: [u8; 32], o: u8, s: &T::State) -> Result<(String, u128)> {
        put(ws, container(&self.code, self.chain(token, o).encode()), s.encode()).await
    }
    async fn get_chain(&self, ws: &str, token: [u8; 32], o: u8) -> Result<Option<T::State>> {
        Ok(get(ws, self.chain(token, o).id()).await?.and_then(|b| T::State::decode(&b)))
    }
}

/// Append an entry at `stamp`: signed by `by`, a spender (`spender`), or with work.
fn push(s: &mut T::State, body: T::Body, n: [u8; 16], by: Option<u8>, stamp: u64) {
    push_as(s, body, n, by.map(Signer_::Owner).unwrap_or(Signer_::Work), stamp)
}
enum Signer_ {
    Owner(u8),
    Spender(u8),
    Work,
}
fn push_as(s: &mut T::State, body: T::Body, n: [u8; 16], who: Signer_, stamp: u64) {
    let bal = s.balance();
    let balance = match &body {
        T::Body::Genesis { amount } => *amount,
        T::Body::Send { amount, .. } | T::Body::Burn { amount } | T::Body::Spend { amount, .. } | T::Body::Lock { amount, .. } | T::Body::Fill { amount, .. } => bal - amount,
        T::Body::Receive { amount, .. } | T::Body::Claim { amount, .. } => bal + amount,
        T::Body::Allow { .. } | T::Body::Offer { .. } => bal,
        T::Body::Refund { lock } => bal + s.entries.iter().find_map(|x| match &x.body { T::Body::Lock { amount, .. } if &x.nonce == lock => Some(*amount), _ => None }).unwrap_or(0),
    };
    let mut e = T::Entry { seq: s.entries.len() as u64, prev: s.head(), nonce: n, stamp, body, balance, auth: T::Auth::Work(0) };
    let m = e.message();
    e.auth = match who {
        Signer_::Owner(k) => T::Auth::Sig(key(k).sign(&m).to_bytes()),
        Signer_::Spender(k) => T::Auth::Delegated { by: pk(k), sig: key(k).sign(&m).to_bytes() },
        Signer_::Work => T::Auth::Work((0u64..).find(|w| T::work_ok(&m, *w)).unwrap()),
    };
    s.entries.push(e);
}
fn stamp_of(s: &T::State, n: [u8; 16]) -> u64 {
    s.entries.iter().find(|e| e.nonce == n).map_or(0, |e| e.stamp)
}

/// A beacon on X, extended in the background (a small step: ~4 a second at most; one a second here).
async fn start_beacon(code: &[u8], x: &str, salt: [u8; 16]) -> Result<[u8; 32]> {
    let p = B::Params { k: 1 << 20, segs: 256, sample: 8, window: 2, keep: 600, seed: *blake3::hash(&salt).as_bytes() };
    let c = || container(code, p.encode());
    let id: [u8; 32] = c().key().id().as_bytes().try_into().unwrap();
    let first = B::State::default().extend(&p);
    put(x, c(), B::wire::encode(&first)).await?;
    let (code, x) = (code.to_vec(), x.to_string());
    tokio::spawn(async move {
        let mut s = first;
        loop {
            tokio::time::sleep(Duration::from_millis(1000)).await;
            s = s.extend(&p);
            let _ = put(&x, container(&code, p.encode()), B::wire::encode(&s)).await;
        }
    });
    Ok(id)
}

#[tokio::main]
async fn main() -> Result<()> {
    // token-live <token.wasm> <beacon.wasm> <ws X> <ws Y> [basic|allow|swap]
    let a: Vec<String> = std::env::args().collect();
    let code = std::fs::read(&a[1])?;
    let bcode = std::fs::read(&a[2])?;
    let (x, y) = (a[3].clone(), a[4].clone());
    let salt: [u8; 16] = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_nanos().to_le_bytes();
    let beacon = start_beacon(&bcode, &x, salt).await?;
    let w = W { ch: *blake3::hash(&code).as_bytes(), code, x, y, beacon, salt };
    w.until(2).await?;
    println!("beacon {} on X, ticking; height {}", hex(&beacon[..4]), w.now().await?);
    for mode in a.get(5).map_or(vec!["basic", "allow", "swap"], |m| vec![m.as_str()]) {
        match mode {
            "basic" => basic(&w).await?,
            "allow" => allow(&w).await?,
            "swap" => swap(&w).await?,
            "wallet" => wallet(&w).await?,
            m => bail!("no mode {m}"),
        }
    }
    Ok(())
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A payment; a double receive; a receive of a send that does not exist; two writer sends at one seq.
async fn basic(w: &W) -> Result<()> {
    println!("— basic");
    let ta = w.put_def(&w.def(1, b"Basic", 1)).await?;
    let h = w.now().await?;
    let mut ca = T::State::default();
    push(&mut ca, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h);
    push(&mut ca, T::Body::Send { to: pk(2), amount: 300 }, w.n(2), Some(1), h);
    println!("1  A: genesis, send 300 to B, on X: {}", w.put_chain(&w.x, ta, 1, &ca).await?.0);
    let r = w.until(h + 2).await?;
    let mut cb = T::State::default();
    push(&mut cb, T::Body::Receive { from: pk(1), nonce: w.n(2), amount: 300, cited: h }, w.n(2), None, r);
    println!("2  B receives it a margin later, on Y: {}", w.put_chain(&w.y, ta, 2, &cb).await?.0);
    let r = w.now().await?;
    let mut twice = cb.clone();
    push(&mut twice, T::Body::Receive { from: pk(1), nonce: w.n(2), amount: 300, cited: h }, w.n(3), None, r);
    let (s, _) = w.put_chain(&w.y, ta, 2, &twice).await?;
    println!("3  B receives it AGAIN, on Y: {s} — Y holds balance {}", w.get_chain(&w.y, ta, 2).await?.map_or(0, |s| s.balance()));
    let mut cc = T::State::default();
    push(&mut cc, T::Body::Receive { from: pk(1), nonce: w.n(9), amount: 50, cited: h }, w.n(9), None, r);
    println!("4  C receives a send that does not exist, on Y: {}", w.put_chain(&w.y, ta, 3, &cc).await?.0);
    let r = w.now().await?;
    let (mut ax, mut ay) = (ca.clone(), ca.clone());
    push(&mut ax, T::Body::Send { to: pk(2), amount: 600 }, w.n(4), Some(1), r);
    push(&mut ay, T::Body::Send { to: pk(3), amount: 600 }, w.n(5), Some(1), r);
    let (s1, s2) = tokio::join!(w.put_chain(&w.x, ta, 1, &ax), w.put_chain(&w.y, ta, 1, &ay));
    println!("5  A signs two sends at one seq: via X {}, via Y {}", s1?.0, s2?.0);
    tokio::time::sleep(Duration::from_secs(5)).await;
    let (gx, gy) = (w.get_chain(&w.x, ta, 1).await?, w.get_chain(&w.y, ta, 1).await?);
    println!("   after ~5 s: X fork {}, Y fork {}, same {}", gx.as_ref().is_some_and(|s| s.fork.is_some()), gy.as_ref().is_some_and(|s| s.fork.is_some()), gx.is_some() && gx == gy);
    Ok(())
}

/// S spends A's within an allowance; past it, refused.
async fn allow(w: &W) -> Result<()> {
    println!("— allow");
    let ta = w.put_def(&w.def(1, b"Allow", 2)).await?;
    let h = w.now().await?;
    let mut ca = T::State::default();
    push(&mut ca, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h);
    push(&mut ca, T::Body::Allow { spender: pk(5), limit: 200 }, w.n(2), Some(1), h);
    println!("1  A: genesis, allow S 200, on X: {}", w.put_chain(&w.x, ta, 1, &ca).await?.0);
    let r = w.now().await?;
    let mut cs = ca.clone();
    push_as(&mut cs, T::Body::Spend { to: pk(3), amount: 150 }, w.n(3), Signer_::Spender(5), r);
    println!("2  S spends 150 to C, through Y: {}", w.put_chain(&w.y, ta, 1, &cs).await?.0);
    let r2 = w.until(r + 2).await?;
    let mut cc = T::State::default();
    push(&mut cc, T::Body::Receive { from: pk(1), nonce: w.n(3), amount: 150, cited: r }, w.n(3), None, r2);
    println!("3  C receives it, through X: {}", w.put_chain(&w.x, ta, 3, &cc).await?.0);
    let r3 = w.now().await?;
    let mut over = cs.clone();
    push_as(&mut over, T::Body::Spend { to: pk(3), amount: 51 }, w.n(4), Signer_::Spender(5), r3);
    println!("4  S spends 51 more (50 left), through Y: {}", w.put_chain(&w.y, ta, 1, &over).await?.0);
    Ok(())
}

/// THE SWAP (step 5b): M offers 2 A per 1 B; T locks 100 B; once final, the fill through Y; T receives; anyone claims
/// for M. And what the append rules refuse: a fill before its lock is final, a refund before its deadline, a refund
/// of a filled lock.
async fn swap(w: &W) -> Result<()> {
    println!("— swap");
    let (m, t) = (1u8, 2u8);
    let ta = w.put_def(&w.def(m, b"SwapA", 3)).await?;
    let tb = w.put_def(&w.def(t, b"SwapB", 4)).await?;
    let h = w.now().await?;
    let mut ma = T::State::default();
    push(&mut ma, T::Body::Genesis { amount: 1000 }, w.n(1), Some(m), h);
    w.put_chain(&w.x, ta, m, &ma).await?;
    let h = w.now().await?;
    push(&mut ma, T::Body::Offer { get: tb, give: 2, per: 1, max: 400, until: h + 1000 }, w.n(10), Some(m), h);
    println!("1  M offers 2 A per 1 B (an append, on X): {}", w.put_chain(&w.x, ta, m, &ma).await?.0);
    let mut tbc = T::State::default();
    push(&mut tbc, T::Body::Genesis { amount: 1000 }, w.n(1), Some(t), h);
    w.put_chain(&w.y, tb, t, &tbc).await?;
    let h = w.now().await?;
    let deadline = h + 40;
    push(&mut tbc, T::Body::Lock { to: pk(m), amount: 100, until: deadline, token: ta, offer: w.n(10) }, w.n(20), Some(t), h);
    println!("2  T locks 100 B for it, deadline {deadline} (now {h}), on Y: {}", w.put_chain(&w.y, tb, t, &tbc).await?.0);
    let lock_at = stamp_of(&tbc, w.n(20));
    // Too soon: the lock not final yet.
    let r = w.now().await?;
    let mut early = ma.clone();
    push(&mut early, T::Body::Fill { offer: w.n(10), taker: pk(t), lock: w.n(20), amount: 200, cited: lock_at, until: deadline }, w.n(30), None, r);
    if r < lock_at + 2 {
        println!("3  the fill BEFORE the lock is final (now {r}), through Y: {}", w.put_chain(&w.y, ta, m, &early).await?.0);
    }
    let r = w.until(lock_at + 2).await?;
    let mut filled = ma.clone();
    push(&mut filled, T::Body::Fill { offer: w.n(10), taker: pk(t), lock: w.n(20), amount: 200, cited: lock_at, until: deadline }, w.n(30), None, r);
    println!("4  the fill once final (now {r}), through Y: {}", w.put_chain(&w.y, ta, m, &filled).await?.0);
    let fill_at = r;
    let r = w.until(fill_at + 2).await?;
    let mut tac = T::State::default();
    push(&mut tac, T::Body::Receive { from: pk(m), nonce: w.n(30), amount: 200, cited: fill_at }, w.n(30), None, r);
    println!("5  T receives the 200 A, through X: {}", w.put_chain(&w.x, ta, t, &tac).await?.0);
    let mut mbc = T::State::default();
    push(&mut mbc, T::Body::Claim { taker: pk(t), lock: w.n(20), amount: 100, cited: fill_at, token: ta }, w.n(40), None, r);
    println!("6  anyone claims the 100 B for M, through X: {}", w.put_chain(&w.x, tb, m, &mbc).await?.0);
    // A refund of the FILLED lock.
    let mut bad = tbc.clone();
    let r = w.now().await?;
    push(&mut bad, T::Body::Refund { lock: w.n(20) }, w.n(21), None, r);
    println!("7  a refund of the filled lock, through Y: {}", w.put_chain(&w.y, tb, t, &bad).await?.0);
    // A second lock nobody fills: refund too soon, then on time.
    let r = w.now().await?;
    let d2 = r + 6;
    push(&mut tbc, T::Body::Lock { to: pk(m), amount: 50, until: d2, token: ta, offer: w.n(10) }, w.n(22), Some(t), r);
    w.put_chain(&w.y, tb, t, &tbc).await?;
    let r = w.now().await?;
    let mut soon = tbc.clone();
    push(&mut soon, T::Body::Refund { lock: w.n(22) }, w.n(23), None, r);
    if r < d2 {
        println!("8  a refund BEFORE its deadline {d2} (now {r}), through Y: {}", w.put_chain(&w.y, tb, t, &soon).await?.0);
    }
    let r = w.until(d2).await?;
    let mut ontime = tbc.clone();
    push(&mut ontime, T::Body::Refund { lock: w.n(22) }, w.n(23), None, r);
    println!("9  the refund at its deadline (now {r}), through Y: {}", w.put_chain(&w.y, tb, t, &ontime).await?.0);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let bal = |s: Option<T::State>| s.map_or(0, |s| s.balance());
    println!(
        "   balances: M A {} · T A {} · T B {} · M B {}",
        bal(w.get_chain(&w.x, ta, m).await?),
        bal(w.get_chain(&w.x, ta, t).await?),
        bal(w.get_chain(&w.y, tb, t).await?),
        bal(w.get_chain(&w.x, tb, m).await?)
    );
    Ok(())
}

/// STEP 6, LIVE: the minting-by-burying attack accepted by the contracts on the nodes — and refused by a WALLET reading
/// the same chains from a node; an honest payment verified.
async fn wallet(w: &W) -> Result<()> {
    use craftworks_token_contract::wallet::{Source, Wallet};
    println!("— wallet");
    let d = w.def(1, b"Wallet", 5);
    let ta = w.put_def(&d).await?;
    let h = w.now().await?;
    let mut c1 = T::State::default();
    push(&mut c1, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h);
    for k in 0..T::RECENT as u8 {
        push(&mut c1, T::Body::Send { to: pk(2), amount: 10 }, w.n(20 + k), Some(1), h);
    }
    push(&mut c1, T::Body::Send { to: pk(4), amount: 50 }, w.n(30), Some(1), h);
    println!("1  1: genesis, {} real sends of 10 to 2, 50 to 4, on X: {}", T::RECENT, w.put_chain(&w.x, ta, 1, &c1).await?.0);
    let r = w.until(h + 2).await?;
    let mut c2 = T::State::default();
    push(&mut c2, T::Body::Receive { from: pk(1), nonce: w.n(9), amount: 5000, cited: h }, w.n(9), None, r);
    for k in 0..T::RECENT as u8 {
        push(&mut c2, T::Body::Receive { from: pk(1), nonce: w.n(20 + k), amount: 10, cited: h }, w.n(20 + k), None, r);
    }
    push(&mut c2, T::Body::Send { to: pk(3), amount: 3000 }, w.n(40), Some(2), r);
    println!("2  2: a FAKE receive of 5000 buried under {} real ones, then 3000 to 3 — written fresh on Y: {}", T::RECENT, w.put_chain(&w.y, ta, 2, &c2).await?.0);
    let r2 = w.until(r + 2).await?;
    let mut c3 = T::State::default();
    push(&mut c3, T::Body::Receive { from: pk(2), nonce: w.n(40), amount: 3000, cited: r }, w.n(40), None, r2);
    println!("3  3 receives the 3000, on Y: {}", w.put_chain(&w.y, ta, 3, &c3).await?.0);
    let mut c4 = T::State::default();
    push(&mut c4, T::Body::Receive { from: pk(1), nonce: w.n(30), amount: 50, cited: h }, w.n(30), None, r2);
    println!("4  4 receives an honest 50 from 1, on Y: {}", w.put_chain(&w.y, ta, 4, &c4).await?.0);
    // A wallet reading the chains back from the node (through blocking reads of a runtime of its own).
    let (y, code) = (w.y.clone(), w.code.clone());
    let ch = w.ch;
    let beacon = w.beacon;
    let verdicts = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let _ = code;
        let chain = |p: &T::ChainParams| rt.block_on(get(&y, p.id())).ok().flatten().and_then(|b| T::State::decode(&b));
        let defs = |t: &[u8; 32]| if *t == ta { Some(d.clone()) } else { None };
        let src = Source { chain: &chain, definition: &defs };
        let p = |o: u8| T::ChainParams { code_hash: ch, definition: ta, owner: pk(o), beacon };
        let mut wallet3 = Wallet::new(pk(3));
        let v3 = wallet3.verify(&src, &p(3), 0);
        let mut wallet4 = Wallet::new(pk(4));
        let v4 = wallet4.verify(&src, &p(4), 0);
        (v3.map_err(|f| (f.chain.owner == pk(2), f.seq, f.why)), wallet3.reads.get(), v4.map_err(|f| f.why), wallet4.reads.get())
    })
    .await?;
    println!("5  3's wallet verifies its 3000 (reading from Y): {:?} after {} chain reads", verdicts.0.map_err(|(is2, seq, why)| format!("REFUSED — chain {} seq {seq}: {why}", if is2 { "2" } else { "?" })), verdicts.1);
    println!("6  4's wallet verifies its honest 50: {:?} after {} chain reads", verdicts.2, verdicts.3);
    Ok(())
}
