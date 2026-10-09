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
/// Person n's DEVICE key (its node key: what signs its chains).
fn pk(n: u8) -> [u8; 32] {
    key(n).verifying_key().to_bytes()
}
/// Person n's ACCOUNT: its key log (an inception by its owner key, committing to the next), its DID, its device's
/// credential (the account's own format).
/// This RUN's people: their accounts are new each run (a key log rotated by one run stays rotated on the network).
static RUN: std::sync::OnceLock<[u8; 16]> = std::sync::OnceLock::new();
fn seed(what: &str, n: u8) -> [u8; 32] {
    *blake3::hash(&[what.as_bytes(), &RUN.get().copied().unwrap_or_default()[..], &[n]].concat()).as_bytes()
}
fn owner_seed(n: u8) -> [u8; 32] {
    seed("owner", n)
}
fn next_seed(n: u8) -> [u8; 32] {
    seed("next", n)
}
fn log(n: u8) -> craftworks_idlog_contract::Log {
    let next = SigningKey::from_bytes(&next_seed(n)).verifying_key().to_bytes();
    let e = craftworks_idlog_contract::Event::signed(0, [0; 32], &owner_seed(n), craftworks_idlog_contract::commit(&next), [n; 32], [n; 32], vec![]).unwrap();
    craftworks_idlog_contract::Log { events: vec![e] }
}
/// The log ROTATED once: the owner key now the pre-committed next one.
fn rotated(n: u8) -> craftworks_idlog_contract::Log {
    let mut l = log(n);
    let after = SigningKey::from_bytes(&seed("after", n)).verifying_key().to_bytes();
    l.events.push(craftworks_idlog_contract::Event::signed(1, l.events[0].id(), &next_seed(n), craftworks_idlog_contract::commit(&after), [n; 32], [n; 32], vec![]).unwrap());
    l
}
fn did(n: u8) -> [u8; 32] {
    thread_local!(static DIDS: std::cell::RefCell<std::collections::HashMap<u8, [u8; 32]>> = Default::default());
    DIDS.with(|m| *m.borrow_mut().entry(n).or_insert_with(|| log(n).events[0].id()))
}
fn keys_of(d: &[u8; 32]) -> Option<craftworks_idlog_contract::Log> {
    (0..=40u8).find(|n| did(*n) == *d).map(log)
}

/// The world of one run: the token code, the two nodes, the beacon (kept ticking by a task), a salt for fresh ids.
struct W {
    code: Vec<u8>,
    ch: [u8; 32],
    x: String,
    y: String,
    beacon: [u8; 32],
    salt: [u8; 16],
    /// The key log's code hash (every owner a DID) and its code (to publish people's logs).
    idlog: [u8; 32],
    idlog_code: Vec<u8>,
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
        T::Definition { code_hash: self.ch, decimals: 8, min_send: 10, genesis_owner: did(genesis), genesis_amount: 1000, name: name.to_vec(), nonce: self.n(k), beacon: self.beacon, margin: 2, idlog: self.idlog, mint: None, freeze: None }
    }
    fn chain(&self, token: [u8; 32], o: u8) -> T::ChainParams {
        T::ChainParams { code_hash: self.ch, definition: token, owner: did(o), beacon: self.beacon, idlog: self.idlog }
    }
    /// Person n's KEY LOG, published (its contract: the key log's code, the DID its params).
    async fn put_log(&self, ws: &str, l: &craftworks_idlog_contract::Log) -> Result<(String, u128)> {
        put(ws, container(&self.idlog_code, l.events[0].id().to_vec()), l.encode()).await
    }
    /// A token: its definition, then its AUTHORITY CHAIN and FREEZE LIST (every token has both, made with it — a
    /// debit's update reads the freeze list).
    async fn put_def(&self, d: &T::Definition) -> Result<[u8; 32]> {
        let id = T::id(&self.ch, &d.encode());
        let a = d.authority();
        let f = T::freeze::Params { code_hash: self.ch, definition: id, beacon: self.beacon };
        for (params, state) in [(d.encode(), d.encode()), (a.encode(), T::authority::Chain::default().encode()), (f.encode(), T::freeze::List::default().encode())] {
            let (said, _) = put(&self.x, container(&self.code, params), state).await?;
            if said != "ACCEPTED" {
                bail!("a token's definition, authority chain or freeze list: {said}");
            }
        }
        Ok(id)
    }
    async fn put_chain(&self, ws: &str, token: [u8; 32], o: u8, s: &T::State) -> Result<(String, u128)> {
        put(ws, container(&self.code, self.chain(token, o).encode()), s.encode()).await
    }
    async fn put_ws(&self, ws: &str, c: ContractContainer, state: Vec<u8>) -> Result<(String, u128)> {
        put(ws, c, state).await
    }
    async fn get_chain(&self, ws: &str, token: [u8; 32], o: u8) -> Result<Option<T::State>> {
        Ok(get(ws, self.chain(token, o).id()).await?.and_then(|b| T::State::decode(&b)))
    }
}

/// Append an entry at `stamp`: signed by `by`, a spender (`spender`), or with work.
fn push(s: &mut T::State, body: T::Body, n: [u8; 16], by: Option<u8>, stamp: u64) {
    push_as(s, body, n, by.map(Signer_::Owner).unwrap_or(Signer_::Work), stamp)
}
#[derive(Clone, Copy)]
enum Signer_ {
    Owner(u8),
    Spender(u8),
    Work,
}
fn push_as(s: &mut T::State, body: T::Body, n: [u8; 16], who: Signer_, stamp: u64) {
    // Before a person first signs a chain: its WRITER entry (its device, credentialed on key seq 0).
    if let Signer_::Owner(k) = who {
        if !matches!(body, T::Body::Writer { .. }) && T::writer_at(s, s.entries.len()).is_none() {
            let cred = craftworks_identity::credential(&did(k), &pk(k), &[], &owner_seed(k));
            push_as(s, T::Body::Writer { cred, key_seq: 0 }, [0xee; 16], Signer_::Owner(k), stamp);
        }
    }
    let bal = s.balance();
    let balance = match &body {
        T::Body::Genesis { amount } => *amount,
        T::Body::Send { amount, .. } | T::Body::Burn { amount } | T::Body::Spend { amount, .. } | T::Body::Lock { amount, .. } | T::Body::Fill { amount, .. } => bal - amount,
        T::Body::Receive { amount, .. } | T::Body::Claim { amount, .. } | T::Body::Mint { amount, .. } => bal + amount,
        T::Body::Allow { .. } | T::Body::Offer { .. } | T::Body::Cut { .. } | T::Body::Writer { .. } | T::Body::Members { .. } => bal,
        T::Body::Refund { lock, .. } => bal + s.entries.iter().find_map(|x| match &x.body { T::Body::Lock { amount, .. } if &x.nonce == lock => Some(*amount), _ => None }).unwrap_or(0),
    };
    let mut e = T::Entry { seq: s.entries.last().map_or(0, |e| e.seq + 1), prev: s.head(), nonce: n, stamp, body, balance, auth: T::Auth::Work(0) };
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
    RUN.set(salt).ok();
    let beacon = start_beacon(&bcode, &x, salt).await?;
    let idlog_code = std::fs::read(std::path::Path::new(&a[1]).with_file_name("idlog.wasm"))?;
    let w = W { ch: *blake3::hash(&code).as_bytes(), code, x, y, beacon, salt, idlog: *blake3::hash(&idlog_code).as_bytes(), idlog_code };
    // Everyone's KEY LOG on the network (a chain's validation reads its owner's).
    let t = Instant::now();
    for n in 1..=9u8 {
        let l = log(n);
        let (sx, sy) = tokio::join!(w.put_log(&w.x, &l), w.put_log(&w.y, &l));
        if sx?.0 != "ACCEPTED" || sy?.0 != "ACCEPTED" {
            bail!("person {n}'s key log was not taken");
        }
    }
    println!("key logs of 9 people on X and Y in {} ms", t.elapsed().as_millis());
    w.until(2).await?;
    println!("beacon {} on X, ticking; height {}", hex(&beacon[..4]), w.now().await?);
    for mode in a.get(5).map_or(vec!["basic", "allow", "swap"], |m| vec![m.as_str()]) {
        match mode {
            "basic" => basic(&w).await?,
            "allow" => allow(&w).await?,
            "swap" => swap(&w).await?,
            "wallet" => wallet(&w).await?,
            "did" => did_mode(&w).await?,
            "mint" => mint_mode(&w).await?,
            "multisig" => multisig(&w).await?,
            "cut" => cut(&w, &std::fs::read(std::path::Path::new(&a[1]).with_file_name("block.wasm"))?).await?,
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
    push(&mut ca, T::Body::Send { to: did(2), amount: 300 }, w.n(2), Some(1), h);
    println!("1  A: genesis, send 300 to B, on X: {}", w.put_chain(&w.x, ta, 1, &ca).await?.0);
    let r = w.until(h + 2).await?;
    let mut cb = T::State::default();
    push(&mut cb, T::Body::Receive { from: did(1), nonce: w.n(2), amount: 300, cited: h, ev: T::Evidence::default() }, w.n(2), None, r);
    println!("2  B receives it a margin later, on Y: {}", w.put_chain(&w.y, ta, 2, &cb).await?.0);
    let r = w.now().await?;
    let mut twice = cb.clone();
    push(&mut twice, T::Body::Receive { from: did(1), nonce: w.n(2), amount: 300, cited: h, ev: T::Evidence::default() }, w.n(3), None, r);
    let (s, _) = w.put_chain(&w.y, ta, 2, &twice).await?;
    println!("3  B receives it AGAIN, on Y: {s} — Y holds balance {}", w.get_chain(&w.y, ta, 2).await?.map_or(0, |s| s.balance()));
    let mut cc = T::State::default();
    push(&mut cc, T::Body::Receive { from: did(1), nonce: w.n(9), amount: 50, cited: h, ev: T::Evidence::default() }, w.n(9), None, r);
    println!("4  C receives a send that does not exist, on Y: {}", w.put_chain(&w.y, ta, 3, &cc).await?.0);
    let r = w.now().await?;
    let (mut ax, mut ay) = (ca.clone(), ca.clone());
    push(&mut ax, T::Body::Send { to: did(2), amount: 600 }, w.n(4), Some(1), r);
    push(&mut ay, T::Body::Send { to: did(3), amount: 600 }, w.n(5), Some(1), r);
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
    push_as(&mut cs, T::Body::Spend { to: did(3), amount: 150 }, w.n(3), Signer_::Spender(5), r);
    println!("2  S spends 150 to C, through Y: {}", w.put_chain(&w.y, ta, 1, &cs).await?.0);
    let r2 = w.until(r + 2).await?;
    let mut cc = T::State::default();
    push(&mut cc, T::Body::Receive { from: did(1), nonce: w.n(3), amount: 150, cited: r, ev: T::Evidence::default() }, w.n(3), None, r2);
    println!("3  C receives it, through X: {}", w.put_chain(&w.x, ta, 3, &cc).await?.0);
    let r3 = w.now().await?;
    let mut over = cs.clone();
    push_as(&mut over, T::Body::Spend { to: did(3), amount: 51 }, w.n(4), Signer_::Spender(5), r3);
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
    push(&mut tbc, T::Body::Lock { to: did(m), amount: 100, until: deadline, token: ta, offer: w.n(10) }, w.n(20), Some(t), h);
    println!("2  T locks 100 B for it, deadline {deadline} (now {h}), on Y: {}", w.put_chain(&w.y, tb, t, &tbc).await?.0);
    let lock_at = stamp_of(&tbc, w.n(20));
    // Too soon: the lock not final yet.
    let r = w.now().await?;
    let mut early = ma.clone();
    push(&mut early, T::Body::Fill { offer: w.n(10), taker: did(t), lock: w.n(20), amount: 200, cited: lock_at, until: deadline, ev: T::Evidence::default() }, w.n(30), None, r);
    if r < lock_at + 2 {
        println!("3  the fill BEFORE the lock is final (now {r}), through Y: {}", w.put_chain(&w.y, ta, m, &early).await?.0);
    }
    let r = w.until(lock_at + 2).await?;
    let mut filled = ma.clone();
    push(&mut filled, T::Body::Fill { offer: w.n(10), taker: did(t), lock: w.n(20), amount: 200, cited: lock_at, until: deadline, ev: T::Evidence::default() }, w.n(30), None, r);
    println!("4  the fill once final (now {r}), through Y: {}", w.put_chain(&w.y, ta, m, &filled).await?.0);
    let fill_at = r;
    let r = w.until(fill_at + 2).await?;
    let mut tac = T::State::default();
    push(&mut tac, T::Body::Receive { from: did(m), nonce: w.n(30), amount: 200, cited: fill_at, ev: T::Evidence::default() }, w.n(30), None, r);
    println!("5  T receives the 200 A, through X: {}", w.put_chain(&w.x, ta, t, &tac).await?.0);
    let mut mbc = T::State::default();
    push(&mut mbc, T::Body::Claim { taker: did(t), lock: w.n(20), amount: 100, cited: fill_at, token: ta, ev: T::Evidence::default() }, w.n(40), None, r);
    println!("6  anyone claims the 100 B for M, through X: {}", w.put_chain(&w.x, tb, m, &mbc).await?.0);
    // A refund of the FILLED lock.
    let mut bad = tbc.clone();
    let r = w.now().await?;
    push(&mut bad, T::Body::Refund { lock: w.n(20), ev: T::Evidence::default() }, w.n(21), None, r);
    println!("7  a refund of the filled lock, through Y: {}", w.put_chain(&w.y, tb, t, &bad).await?.0);
    // A second lock nobody fills: refund too soon, then on time.
    let r = w.now().await?;
    let d2 = r + 6;
    push(&mut tbc, T::Body::Lock { to: did(m), amount: 50, until: d2, token: ta, offer: w.n(10) }, w.n(22), Some(t), r);
    w.put_chain(&w.y, tb, t, &tbc).await?;
    let r = w.now().await?;
    let mut soon = tbc.clone();
    push(&mut soon, T::Body::Refund { lock: w.n(22), ev: T::Evidence::default() }, w.n(23), None, r);
    if r < d2 {
        println!("8  a refund BEFORE its deadline {d2} (now {r}), through Y: {}", w.put_chain(&w.y, tb, t, &soon).await?.0);
    }
    let r = w.until(d2).await?;
    let mut ontime = tbc.clone();
    push(&mut ontime, T::Body::Refund { lock: w.n(22), ev: T::Evidence::default() }, w.n(23), None, r);
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
    use craftworks_token_contract::wallet::{Fail, Source, Wallet};
    println!("— wallet");
    let d = w.def(1, b"Wallet", 5);
    let ta = w.put_def(&d).await?;
    let h = w.now().await?;
    let mut c1 = T::State::default();
    push(&mut c1, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h);
    for k in 0..T::RECENT as u8 {
        push(&mut c1, T::Body::Send { to: did(2), amount: 10 }, w.n(20 + k), Some(1), h);
    }
    push(&mut c1, T::Body::Send { to: did(4), amount: 50 }, w.n(30), Some(1), h);
    println!("1  1: genesis, {} real sends of 10 to 2, 50 to 4, on X: {}", T::RECENT, w.put_chain(&w.x, ta, 1, &c1).await?.0);
    let r = w.until(h + 2).await?;
    let mut c2 = T::State::default();
    push(&mut c2, T::Body::Receive { from: did(1), nonce: w.n(9), amount: 5000, cited: h, ev: T::Evidence::default() }, w.n(9), None, r);
    for k in 0..T::RECENT as u8 {
        push(&mut c2, T::Body::Receive { from: did(1), nonce: w.n(20 + k), amount: 10, cited: h, ev: T::Evidence::default() }, w.n(20 + k), None, r);
    }
    push(&mut c2, T::Body::Send { to: did(3), amount: 3000 }, w.n(40), Some(2), r);
    println!("2  2: a FAKE receive of 5000 buried under {} real ones, then 3000 to 3 — written fresh on Y: {}", T::RECENT, w.put_chain(&w.y, ta, 2, &c2).await?.0);
    let r2 = w.until(r + 2).await?;
    let mut c3 = T::State::default();
    push(&mut c3, T::Body::Receive { from: did(2), nonce: w.n(40), amount: 3000, cited: r, ev: T::Evidence::default() }, w.n(40), None, r2);
    println!("3  3 receives the 3000, on Y: {}", w.put_chain(&w.y, ta, 3, &c3).await?.0);
    let mut c4 = T::State::default();
    push(&mut c4, T::Body::Receive { from: did(1), nonce: w.n(30), amount: 50, cited: h, ev: T::Evidence::default() }, w.n(30), None, r2);
    println!("4  4 receives an honest 50 from 1, on Y: {}", w.put_chain(&w.y, ta, 4, &c4).await?.0);
    // A wallet reading the chains back from the node (through blocking reads of a runtime of its own).
    let (y, code) = (w.y.clone(), w.code.clone());
    let ch = w.ch;
    let beacon = w.beacon;
    let idlog = w.idlog;
    let verdicts = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let _ = code;
        let chain = |p: &T::ChainParams| rt.block_on(get(&y, p.id())).ok().flatten().and_then(|b| T::State::decode(&b));
        let defs = |t: &[u8; 32]| if *t == ta { Some(d.clone()) } else { None };
        let src = Source { chain: &chain, definition: &defs, block: &|_| None, keys: &keys_of, supply: &|_| None, authority: &|_| None };
        let p = |o: u8| T::ChainParams { code_hash: ch, definition: ta, owner: did(o), beacon, idlog };
        let fraud = |r: Result<(), Fail>| r.map_err(|f| match f {
            Fail::Fraud(f) => f,
            Fail::Need(_) => unreachable!("no cuts in this run"),
        });
        let mut wallet3 = Wallet::new(pk(3));
        let v3 = fraud(wallet3.verify(&src, &p(3), 0));
        let mut wallet4 = Wallet::new(pk(4));
        let v4 = fraud(wallet4.verify(&src, &p(4), 0));
        (v3.map_err(|f| (f.chain.owner == did(2), f.seq, f.why)), wallet3.reads.get(), v4.map_err(|f| f.why), wallet4.reads.get())
    })
    .await?;
    println!("5  3's wallet verifies its 3000 (reading from Y): {:?} after {} chain reads", verdicts.0.map_err(|(is2, seq, why)| format!("REFUSED — chain {} seq {seq}: {why}", if is2 { "2" } else { "?" })), verdicts.1);
    println!("6  4's wallet verifies its honest 50: {:?} after {} chain reads", verdicts.2, verdicts.3);
    Ok(())
}

/// FOUNDATION 1, LIVE: the PRUNE CUT. A holder checks a cut against what it summarises and keeps the chain from it; a
/// send received before a cut is not received again; a send behind its sender's cut is received with the proof (and
/// refused without it); a wallet walks across the cut from the history's blocks, published as Block contracts.
async fn cut(w: &W, block_code: &[u8]) -> Result<()> {
    use craftworks_token_contract::{cut, wallet::{Fail, Source, Wallet}};
    println!("— cut");
    let why = |e: &'static str| anyhow::anyhow!(e);
    let d = w.def(1, b"Cut", 6);
    let ta = w.put_def(&d).await?;
    let h = w.now().await?;
    let mut ca = T::State::default();
    push(&mut ca, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h);
    push(&mut ca, T::Body::Send { to: did(2), amount: 300 }, w.n(2), Some(1), h);
    push(&mut ca, T::Body::Send { to: did(3), amount: 100 }, w.n(4), Some(1), h);
    println!("1  A: genesis, 300 to B, 100 to C, on X: {}", w.put_chain(&w.x, ta, 1, &ca).await?.0);
    let r = w.until(h + 2).await?;
    let mut cb = T::State::default();
    push(&mut cb, T::Body::Receive { from: did(1), nonce: w.n(2), amount: 300, cited: h, ev: T::Evidence::default() }, w.n(2), None, r);
    println!("2  B receives the 300, on Y: {}", w.put_chain(&w.y, ta, 2, &cb).await?.0);
    // B CUTS: the holder (Y, B whole) is sent the chain from the cut alone.
    let r = w.now().await?;
    let mut cb = cb;
    push(&mut cb, T::Body::Writer { cred: craftworks_identity::credential(&did(2), &pk(2), &[], &owner_seed(2)), key_seq: 0 }, [0xee; 16], Some(2), w.now().await?);
    let (body, bblocks) = cut::make(&w.chain(ta, 2), &d, &cb, Some(&log(2))).map_err(why)?;
    let mut wb = cb.clone();
    push(&mut wb, body, w.n(50), Some(2), r);
    let lb = T::prune(&wb);
    let (said, ms) = w.put_chain(&w.y, ta, 2, &lb).await?;
    let held = w.get_chain(&w.y, ta, 2).await?;
    println!("3  B cuts (Y sent only the cut): {said} in {ms} ms — Y holds {} entr(ies) from seq {}, balance {}", held.as_ref().map_or(0, |s| s.entries.len()), held.as_ref().and_then(|s| s.entries.first()).map_or(0, |e| e.seq), held.as_ref().map_or(0, |s| s.balance()));
    // A cut that is not what came before it, to a holder of what it summarises: refused there.
    let mut c6 = T::State::default();
    push(&mut c6, T::Body::Allow { spender: pk(7), limit: 50 }, w.n(60), Some(6), r);
    w.put_chain(&w.y, ta, 6, &c6).await?;
    let (mut body6, _) = cut::make(&w.chain(ta, 6), &d, &c6, Some(&log(6))).map_err(why)?;
    if let T::Body::Cut { allowances, .. } = &mut body6 {
        allowances.clear();
    }
    let mut w6 = c6.clone();
    push(&mut w6, body6, w.n(61), Some(6), r);
    println!("4  6 cuts dropping its allowance (Y holds what it summarises): {}", w.put_chain(&w.y, ta, 6, &T::prune(&w6)).await?.0);
    // B receives the 300 AGAIN: the only proof it can make is of presence.
    let rb = w.until(r + 2).await?;
    let T::Body::Cut { received, .. } = &lb.entries[0].body else { bail!("no cut") };
    let present = cut::proof(&bblocks, received, &T::received_key(T::RECEIVED, &did(1), &w.n(2))).map_err(why)?;
    let mut again = lb.clone();
    push(&mut again, T::Body::Receive { from: did(1), nonce: w.n(2), amount: 300, cited: h, ev: T::Evidence { mine: present, theirs: vec![] } }, w.n(51), None, rb);
    println!("5  B receives the 300 again with its presence proof, on Y: {}", w.put_chain(&w.y, ta, 2, &again).await?.0);
    let mut bare = lb.clone();
    push(&mut bare, T::Body::Receive { from: did(1), nonce: w.n(2), amount: 300, cited: h, ev: T::Evidence::default() }, w.n(52), None, rb);
    println!("6  … and with no proof, on Y: {}", w.put_chain(&w.y, ta, 2, &bare).await?.0);
    // A CUTS (the send to C behind it); its history's blocks published as Block contracts.
    let r = w.now().await?;
    let (abody, ablocks) = cut::make(&w.chain(ta, 1), &d, &ca, Some(&log(1))).map_err(why)?;
    let mut wa = ca.clone();
    push(&mut wa, abody, w.n(53), Some(1), r);
    let la = T::prune(&wa);
    println!("7  A cuts, on X: {}", w.put_chain(&w.x, ta, 1, &la).await?.0);
    let t = Instant::now();
    let mut ok = 0;
    for (cid, body) in &ablocks {
        let st = wire::block::block_state(cid, body).context("a block of no kind")?;
        if w.put_ws(&w.x, wire::block::block_contract(block_code, cid), st).await?.0 == "ACCEPTED" {
            ok += 1;
        }
    }
    println!("8  A's history: {ok} of {} block(s) published on X in {} ms", ablocks.len(), t.elapsed().as_millis());
    // C RECEIVES the 100 (A's send behind A's cut) — on a chain Y already holds (so the update's rules run).
    w.put_chain(&w.y, ta, 3, &T::State::default()).await?;
    let rc = w.until(r + 2).await?;
    let T::Body::Cut { history, .. } = &la.entries[0].body else { bail!("no cut") };
    let send = ca.entries.iter().find(|e| e.nonce == w.n(4)).context("A's send to C")?;
    let theirs = vec![(w.chain(ta, 1).id(), send.seq, cut::proof(&ablocks, history, &T::history_key(send.seq)).map_err(why)?, send.encode())];
    let mut nop = T::State::default();
    push(&mut nop, T::Body::Receive { from: did(1), nonce: w.n(4), amount: 100, cited: h, ev: T::Evidence::default() }, w.n(4), None, rc);
    println!("9  C receives A's 100 with NO evidence (the send is behind A's cut), on Y: {}", w.put_chain(&w.y, ta, 3, &nop).await?.0);
    let mut cc = T::State::default();
    push(&mut cc, T::Body::Receive { from: did(1), nonce: w.n(4), amount: 100, cited: h, ev: T::Evidence { mine: vec![], theirs } }, w.n(4), None, rc);
    println!("10 C receives it with the proof that A's history holds the send, on Y: {}", w.put_chain(&w.y, ta, 3, &cc).await?.0);
    // C's WALLET: across A's cut, from the blocks on the network.
    let (x, y, ch, beacon, bc, idlog) = (w.x.clone(), w.y.clone(), w.ch, w.beacon, block_code.to_vec(), w.idlog);
    let v = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let chain = |p: &T::ChainParams| rt.block_on(get(&y, p.id())).ok().flatten().and_then(|b| T::State::decode(&b));
        let defs = |t: &[u8; 32]| if *t == ta { Some(d.clone()) } else { None };
        let p = |o: u8| T::ChainParams { code_hash: ch, definition: ta, owner: did(o), beacon, idlog };
        // The wallet reads only what is HELD; what it NEEDS is fetched (the blocks the tree names — in a page, through
        // the blocks door: raced against their groups, rebuilt from parity) and it runs again.
        let held: std::cell::RefCell<std::collections::HashMap<[u8; 32], Vec<u8>>> = Default::default();
        let t = Instant::now();
        let (mut rounds, mut fetched) = (0, 0);
        let r = loop {
            rounds += 1;
            let block = |c: &[u8; 32]| held.borrow().get(c).cloned();
            let src = Source { chain: &chain, definition: &defs, block: &block, keys: &keys_of, supply: &|_| None, authority: &|_| None };
            match Wallet::new(pk(3)).verify(&src, &p(3), 0) {
                Ok(()) => break Ok(()),
                Err(Fail::Fraud(f)) => break Err(format!("REFUSED — seq {}: {}", f.seq, f.why)),
                Err(Fail::Need(n)) if rounds <= 20 => {
                    for c in n {
                        let b = rt.block_on(get(&x, wire::block::contract_for(&bc, &c))).ok().flatten().and_then(|st| wire::block::block_of_state(&st).filter(|(id, _)| *id == c).map(|(_, b)| b.to_vec()));
                        if let Some(b) = b {
                            held.borrow_mut().insert(c, b);
                            fetched += 1;
                        }
                    }
                }
                Err(Fail::Need(n)) => break Err(format!("still needs {} block(s)", n.len())),
            }
        };
        (r, rounds, fetched, t.elapsed().as_millis())
    })
    .await?;
    println!("11 C's wallet verifies its 100 across A's cut: {:?} — {} run(s), {} history block(s) fetched as needed, {} ms", v.0, v.1, v.2, v.3);
    Ok(())
}

/// FOUNDATION 2, LIVE: the owner is a DID. A ROTATION of its key revokes the old device for new writes (its holder
/// refuses them); the recovered account names a new device writer on the new key; a stolen device's send at the same
/// seq loses to the recovery on both nodes — nothing frozen.
async fn did_mode(w: &W) -> Result<()> {
    println!("— did");
    let d = w.def(1, b"Did", 7);
    let ta = w.put_def(&d).await?;
    let h = w.now().await?;
    let mut ca = T::State::default();
    push(&mut ca, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h);
    push(&mut ca, T::Body::Send { to: did(2), amount: 100 }, w.n(2), Some(1), h);
    println!("1  A (a DID; device 1 its writer on key 0): writer, genesis, 100 to B, on X: {}", w.put_chain(&w.x, ta, 1, &ca).await?.0);
    // A's account ROTATES its key (its words: the pre-committed next key).
    let r1 = rotated(1);
    let (rx, ry) = tokio::join!(w.put_log(&w.x, &r1), w.put_log(&w.y, &r1));
    println!("2  A's key log rotated (key seq 1), on X and Y: {} / {}", rx?.0, ry?.0);
    // The OLD device (stolen, say) spends — through X, which holds A's chain.
    let r = w.now().await?;
    let mut thief = ca.clone();
    push(&mut thief, T::Body::Send { to: did(9), amount: 800 }, w.n(3), Some(1), r);
    println!("3  the old device sends 800 to 9, through X (holds A's chain): {}", w.put_chain(&w.x, ta, 1, &thief).await?.0);
    // The RECOVERED account names device 31 its writer, on key seq 1 — at the same seq.
    let mut rescue = ca.clone();
    let cred = craftworks_identity::credential(&did(1), &pk(31), &[], &next_seed(1));
    push(&mut rescue, T::Body::Writer { cred, key_seq: 1 }, w.n(4), Some(31), r);
    println!("4  the recovered account names device 31 writer (key 1), through X: {}", w.put_chain(&w.x, ta, 1, &rescue).await?.0);
    // The thief's copy offered to Y (a node that may not hold A's chain yet: from scratch it is well-formed).
    println!("5  the old device's send offered to Y: {}", w.put_chain(&w.y, ta, 1, &thief).await?.0);
    let mut more = rescue.clone();
    push(&mut more, T::Body::Burn { amount: 1 }, w.n(5), Some(31), w.now().await?);
    println!("6  device 31 writes on (a burn), through Y: {}", w.put_chain(&w.y, ta, 1, &more).await?.0);
    tokio::time::sleep(Duration::from_secs(6)).await;
    let (gx, gy) = (w.get_chain(&w.x, ta, 1).await?, w.get_chain(&w.y, ta, 1).await?);
    let at3 = |s: &Option<T::State>| s.as_ref().and_then(|s| s.entries.get(3)).map(|e| match &e.body {
        T::Body::Writer { key_seq, .. } => format!("Writer(key {key_seq})"),
        T::Body::Send { amount, .. } => format!("Send {amount}"),
        _ => "other".into(),
    });
    println!(
        "7  after ~6 s: X seq 3 = {:?}, Y seq 3 = {:?}; frozen X {} Y {}; same {}; A's balance {}",
        at3(&gx),
        at3(&gy),
        gx.as_ref().is_some_and(|s| s.fork.is_some()),
        gy.as_ref().is_some_and(|s| s.fork.is_some()),
        gx.is_some() && gx == gy,
        gx.as_ref().map_or(0, |s| s.balance())
    );
    Ok(())
}

/// k-of-1 set of key `n`.
fn one(n: u8) -> T::authority::KeySet {
    T::authority::KeySet { k: 1, keys: vec![pk(n)] }
}
fn a_mint(def: &[u8; 32], to: u8, amount: u128, nonce: [u8; 16], stamp: u64, by: u8) -> T::supply::Mint {
    let mut m = T::supply::Mint { to: did(to), amount, nonce, stamp, sigs: vec![] };
    m.sigs = vec![(0, key(by).sign(&m.message(def)).to_bytes())];
    m
}
fn a_word(def: &[u8; 32], owner: u8, frozen: bool, seq: u32, stamp: u64, by: u8) -> T::freeze::List {
    let mut r = T::freeze::Record { owner: did(owner), frozen, seq, stamp, sigs: vec![] };
    r.sigs = vec![(0, key(by).sign(&r.message(def)).to_bytes())];
    T::freeze::List { records: vec![r] }
}

/// FOUNDATION 3: a token with a MINT authority (key 60) and a FREEZE authority (key 61). Mints listed in the epoch's
/// supply shard, credited to B; a mint not listed refused; the mint set moved (sealing the supply); the old key's
/// mint after it and into its past refused; B frozen — its send refused, a credit taken — then thawed.
async fn mint_mode(w: &W) -> Result<()> {
    println!("— mint");
    let d = T::Definition { mint: Some(one(60)), freeze: Some(one(61)), ..w.def(1, b"Mint", 8) };
    let ta = w.put_def(&d).await?;
    let ap = d.authority();
    let fp = T::freeze::Params { code_hash: w.ch, definition: ta, beacon: w.beacon };
    let shard = |e: u64| T::supply::Params { code_hash: w.ch, definition: ta, beacon: w.beacon, epoch: e };
    println!("1  definition (mint 60, freeze 61), its authority chain and freeze list, on X: ACCEPTED");
    let h = w.now().await?;
    let e = T::supply::epoch(h);
    let m1 = a_mint(&ta, 2, 500, w.n(1), h, 60);
    let sh = T::supply::Shard { mints: vec![m1.clone()] };
    println!("2  mint 500 to B (key 60) into shard {e}, on X: {}", put(&w.x, container(&w.code, shard(e).encode()), sh.encode()).await?.0);
    let forged = T::supply::Shard { mints: vec![m1.clone(), a_mint(&ta, 9, 900, w.n(2), h, 62)] };
    println!("3  a shard with a mint signed by key 62 (not the authority), on Y: {}", put(&w.y, container(&w.code, shard(e).encode()), forged.encode()).await?.0);
    let r = w.now().await?;
    let mut cb = T::State::default();
    push(&mut cb, T::Body::Mint { nonce: w.n(1), amount: 500, minted: h, ev: T::Evidence::default() }, w.n(1), None, r);
    println!("4  B's chain credits the mint, on Y: {} — balance {}", w.put_chain(&w.y, ta, 2, &cb).await?.0, w.get_chain(&w.y, ta, 2).await?.map_or(0, |s| s.balance()));
    let mut cc = T::State::default();
    push(&mut cc, T::Body::Mint { nonce: w.n(3), amount: 700, minted: h, ev: T::Evidence::default() }, w.n(3), None, r);
    println!("5  C's chain credits a mint not in the shard, on Y: {}", w.put_chain(&w.y, ta, 3, &cc).await?.0);
    // The mint set moves 60 → 63, sealing every epoch so far as it holds.
    let r = w.now().await?;
    let closed: Vec<(u64, [u8; 32])> = (0..=T::supply::epoch(r)).filter_map(|x| (x == e).then(|| T::supply::seal(&sh, r)).flatten().map(|h| (x, h))).collect();
    let mut c = T::authority::Change { seq: 0, role: T::authority::Role::Mint, next: Some(one(63)), stamp: r, closed, cap: 0, sigs: vec![] };
    c.sigs = vec![(0, key(60).sign(&c.message(&ta)).to_bytes())];
    let moved = T::authority::Chain { changes: vec![c] };
    println!("6  the mint set moves 60 → 63 at {r} (sealing {:?}), on X: {}", moved.changes[0].closed.iter().map(|c| c.0).collect::<Vec<_>>(), put(&w.x, container(&w.code, ap.encode()), moved.encode()).await?.0);
    let r2 = w.until(r + 1).await?;
    let e2 = T::supply::epoch(r2);
    let mut late = sh.clone();
    if e2 != e {
        late = T::supply::Shard::default();
    }
    let old = T::supply::merge(&late, &T::supply::Shard { mints: vec![a_mint(&ta, 2, 50, w.n(4), r2, 60)] }).unwrap();
    println!("7  key 60 mints after the move, on X: {}", put(&w.x, container(&w.code, shard(e2).encode()), old.encode()).await?.0);
    w.until(h + 2 * d.margin + 1).await?;
    let back = T::supply::merge(&sh, &T::supply::Shard { mints: vec![a_mint(&ta, 2, 50, w.n(5), h, 60)] }).unwrap();
    println!("8  key 60 mints into its past (stamp {h}, now {}), on X: {}", w.now().await?, put(&w.x, container(&w.code, shard(e).encode()), back.encode()).await?.0);
    let r3 = w.now().await?;
    let e3 = T::supply::epoch(r3);
    let base = if e3 == e { sh.clone() } else { T::supply::Shard::default() };
    let new = T::supply::merge(&base, &T::supply::Shard { mints: vec![a_mint(&ta, 2, 50, w.n(6), r3, 63)] }).unwrap();
    println!("9  key 63 mints, on X: {}", put(&w.x, container(&w.code, shard(e3).encode()), new.encode()).await?.0);
    // A send BEFORE any freeze (the control: B's debits append).
    let mut pre = cb.clone();
    push(&mut pre, T::Body::Send { to: did(3), amount: 10 }, w.n(9), Some(2), w.now().await?);
    println!("10a B sends 10 to C, nothing frozen, on Y: {}", w.put_chain(&w.y, ta, 2, &pre).await?.0);
    let cb = pre;
    // FREEZE B.
    let r = w.now().await?;
    println!("10 B frozen (key 61), on Y: {}", put(&w.y, container(&w.code, fp.encode()), a_word(&ta, 2, true, 0, r, 61).encode()).await?.0);
    let mut sent = cb.clone();
    push(&mut sent, T::Body::Send { to: did(3), amount: 100 }, w.n(7), Some(2), w.now().await?);
    println!("11 B (frozen) sends 100 to C, on Y: {}", w.put_chain(&w.y, ta, 2, &sent).await?.0);
    let r = w.now().await?;
    println!("12 B thawed (key 61), on Y: {}", put(&w.y, container(&w.code, fp.encode()), a_word(&ta, 2, false, 1, r, 61).encode()).await?.0);
    let mut sent = cb.clone();
    push(&mut sent, T::Body::Send { to: did(3), amount: 100 }, w.n(8), Some(2), w.now().await?);
    println!("13 B (thawed) sends 100 to C, on Y: {} — balance {}", w.put_chain(&w.y, ta, 2, &sent).await?.0, w.get_chain(&w.y, ta, 2).await?.map_or(0, |s| s.balance()));
    let mut shards: Vec<T::supply::Shard> = Vec::new();
    let mut es = vec![e, e2, e3];
    es.sort();
    es.dedup();
    for x in es {
        shards.extend(get(&w.x, shard(x).id()).await?.and_then(|b| T::supply::Shard::decode(&b)));
    }
    println!("14 totalSupply (mints in the shards, from X): {:?}", T::supply::minted(&shards));
    Ok(())
}

/// Append `body` at `stamp`, signed by the keys at `by` of `seeds` (a key set's k signatures).
fn push_multi(s: &mut T::State, body: T::Body, n: [u8; 16], seeds: &[u8], by: &[u8], stamp: u64) {
    push(s, body, n, None, stamp);
    let e = s.entries.last_mut().unwrap();
    let m = e.message();
    e.auth = T::Auth::Multi(by.iter().map(|i| (*i, key(seeds[*i as usize]).sign(&m).to_bytes())).collect());
}
fn set_of(k: u8, seeds: &[u8]) -> T::authority::KeySet {
    T::authority::KeySet { k, keys: seeds.iter().map(|n| pk(*n)).collect() }
}

/// FOUNDATION 4: a chain owned by a 2-of-3 KEY SET (keys 70, 71, 72) — no key log anywhere. A pays it; one signature
/// refused, two taken; the set changes to 2-of-2 (73, 74) by two of the old; the old set refused, the new taken.
async fn multisig(w: &W) -> Result<()> {
    println!("— multisig");
    const TRIO: [u8; 3] = [70, 71, 72];
    const NEXT: [u8; 2] = [73, 74];
    let ta = w.put_def(&w.def(1, b"Multi", 9)).await?;
    let owner = T::set_id(&set_of(2, &TRIO));
    let sp = T::ChainParams { owner, ..w.chain(ta, 1) };
    let put_set = |s: &T::State| put(&w.y, container(&w.code, sp.encode()), s.encode());
    let h = w.now().await?;
    let mut ca = T::State::default();
    push(&mut ca, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h);
    push(&mut ca, T::Body::Send { to: owner, amount: 300 }, w.n(2), Some(1), h);
    println!("1  A: genesis, 300 to the 2-of-3 set, on X: {}", w.put_chain(&w.x, ta, 1, &ca).await?.0);
    let r = w.until(h + 2).await?;
    let mut cs = T::State::default();
    push_multi(&mut cs, T::Body::Members { set: set_of(2, &TRIO), set_seq: 0 }, w.n(40), &TRIO, &[0, 2], r);
    push(&mut cs, T::Body::Receive { from: did(1), nonce: w.n(2), amount: 300, cited: h, ev: T::Evidence::default() }, w.n(2), None, r);
    println!("2  the set's chain: its members (2 of 3 sign), the receive, on Y: {}", put_set(&cs).await?.0);
    let r = w.now().await?;
    let mut one = cs.clone();
    push_multi(&mut one, T::Body::Send { to: did(3), amount: 100 }, w.n(3), &TRIO, &[1], r);
    println!("3  a send signed by 1 of 3, on Y: {}", put_set(&one).await?.0);
    let mut two = cs.clone();
    push_multi(&mut two, T::Body::Send { to: did(3), amount: 100 }, w.n(4), &TRIO, &[1, 2], r);
    println!("4  a send signed by 2 of 3, on Y: {}", put_set(&two).await?.0);
    let r = w.now().await?;
    let mut moved = two.clone();
    push_multi(&mut moved, T::Body::Members { set: set_of(2, &NEXT), set_seq: 1 }, w.n(41), &TRIO, &[0, 1], r);
    println!("5  the members change to 2-of-2 (73, 74), signed by 2 of the old, on Y: {}", put_set(&moved).await?.0);
    let r = w.now().await?;
    let mut old = moved.clone();
    push_multi(&mut old, T::Body::Send { to: did(3), amount: 50 }, w.n(5), &TRIO, &[0, 2], r);
    println!("6  the OLD set sends, on Y: {}", put_set(&old).await?.0);
    let mut new = moved.clone();
    push_multi(&mut new, T::Body::Send { to: did(3), amount: 50 }, w.n(6), &NEXT, &[0, 1], r);
    let said = put_set(&new).await?.0;
    let held = get(&w.y, sp.id()).await?.and_then(|b| T::State::decode(&b));
    println!("7  the NEW set sends, on Y: {said} — balance {}", held.map_or(0, |s| s.balance()));
    Ok(())
}
