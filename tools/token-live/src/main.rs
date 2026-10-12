//! TOKEN, LIVE on two nodes, with a beacon ticking: the core (step 3), allowances (5a), swaps with the append rules
//! (5b).
use anyhow::{bail, Context, Result};
use craftworks_beacon_contract as B;
use craftworks_token_contract as T;
use ed25519_dalek::{Signer, SigningKey};
use freenet_stdlib::prelude::*;
use std::time::{Duration, Instant, SystemTime};
use node_io::{container, get, put};









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
        T::Definition { code_hash: self.ch, decimals: 8, min_send: 10, genesis_owner: did(genesis), genesis_amount: 1000, name: name.to_vec(), nonce: self.n(k), beacon: self.beacon, margin: 2, idlog: self.idlog, mint: None, freeze: None, ids: T::Ids::None, royalty: None, cap: 0, item: [0; 32], version: [0; 32], soulbound: false, transfer_fee: None, sale: None, keepers: None }
    }
    fn chain(&self, token: [u8; 32], o: u8) -> T::ChainParams {
        T::ChainParams { code_hash: self.ch, definition: token, owner: did(o), beacon: self.beacon, idlog: self.idlog, item: [0; 32] }
    }
    /// Person n's KEY LOG, published (its contract: the key log's code, the DID its params).
    async fn put_log(&self, ws: &str, l: &craftworks_idlog_contract::Log) -> Result<(String, u128)> {
        put(ws, container(&self.idlog_code, l.events[0].id().to_vec()), l.encode()).await
    }
    /// A token: its definition, then its AUTHORITY CHAIN and FREEZE LIST (every token has both, made with it — a
    /// debit's update reads the freeze list).
    async fn put_def(&self, d: &T::Definition) -> Result<[u8; 32]> {
        let id = T::id(&self.ch, &d.encode());
        for (params, state) in d.contracts(&self.ch) {
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
        T::Body::Send { amount, .. } | T::Body::Burn { amount } | T::Body::Spend { amount, .. } => bal - amount,
        T::Body::Receive { amount, .. } | T::Body::Mint { amount, .. } | T::Body::CappedMint { amount, .. } | T::Body::FeeClaim { amount, .. } | T::Body::Vested { amount, .. } => bal + amount,
        T::Body::Vest { schedule, .. } => bal - schedule.iter().map(|x| x.1).sum::<u128>(),
        T::Body::Escrow { amount, .. } => bal - amount,
        T::Body::Unescrow { refund, .. } => bal + refund,
        T::Body::Reclaim { escrow } => bal + s.entries.iter().find_map(|x| match &x.body { T::Body::Escrow { amount, .. } if &x.nonce == escrow => Some(*amount), _ => None }).unwrap_or(0),
        T::Body::Payout { amount, .. } | T::Body::HolderPayout { amount, .. } => bal + amount,
        T::Body::Stake { amount } => bal - amount,
        T::Body::Unstake { .. } | T::Body::Claim { .. } | T::Body::Slash { .. } | T::Body::Misdeed { .. } | T::Body::FalseVerdict { .. } | T::Body::FalseMint { .. } | T::Body::KeeperMint { .. } => bal,
        T::Body::Release { unstake } => bal + s.entries.iter().find_map(|x| match &x.body { T::Body::Unstake { amount } if &x.nonce == unstake => Some(*amount), _ => None }).unwrap_or(0),
        T::Body::Allow { .. } | T::Body::Cut { .. } | T::Body::Writer { .. } | T::Body::Members { .. } => bal,
        // The balance before `from` (TOKEN §3d).
        T::Body::Supersede { from } => s.entries.iter().rfind(|x| x.seq < *from).map_or(0, |x| x.balance),
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

/// Every output this run's ticker computed (height → output): the truth the outputs log is checked against.
static OUTS: std::sync::Mutex<Vec<(u64, [u8; 32])>> = std::sync::Mutex::new(Vec::new());

/// FAST-FORWARD: the beacon extended as fast as its steps compute (to reach an epoch's start), not once a second.
static FAST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A beacon on X, extended in the background (a small step: ~4 a second at most; one a second here).
async fn start_beacon(code: &[u8], x: &str, salt: [u8; 16]) -> Result<[u8; 32]> {
    let p = node_io::beacon_params(*blake3::hash(&salt).as_bytes());
    let c = || container(code, p.encode());
    let id: [u8; 32] = c().key().id().as_bytes().try_into().unwrap();
    // BEACON_START (a test's choice): the first step at that height instead of 1 — a fresh test beacon placed just before
    // an epoch boundary, so a run does not spend its time ticking towards it. (A first step above 1 links to nothing:
    // the beacon takes it as its start, as it takes a state whose oldest steps were dropped.)
    let at = std::env::var("BEACON_START").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(1);
    let first = if at <= 1 { B::State::default().extend(&p, fresh_mix()) } else { B::normalize(&p, B::State { steps: vec![B::Step::compute(&p, p.genesis(), at, fresh_mix())], alts: vec![], frauds: vec![] }) };
    OUTS.lock().unwrap().push((first.height(), first.out(&p)));
    put(x, c(), B::wire::encode(&first)).await?;
    let (code, x) = (code.to_vec(), x.to_string());
    tokio::spawn(async move {
        let mut s = first;
        loop {
            if !FAST.load(std::sync::atomic::Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
            s = s.extend(&p, fresh_mix());
            OUTS.lock().unwrap().push((s.height(), s.out(&p)));
            let _ = put(&x, container(&code, p.encode()), B::wire::encode(&s)).await;
            // Its FINAL outputs into the outputs log (the span of the newest final height, and the one before it at a
            // span's start), so a lot drawn long ago stays checkable.
            let fin = s.height().saturating_sub(p.window as u64);
            for idx in [fin.saturating_sub(1) / B::log::SPAN, fin / B::log::SPAN] {
                let lp = B::log::Params { beacon: id, index: idx };
                let rec = B::log::record(&lp, &s);
                if !rec.entries.is_empty() && (idx == fin / B::log::SPAN || fin % B::log::SPAN < 2) {
                    let _ = put(&x, container(&code, lp.encode()), rec.encode()).await;
                }
            }
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
    // BEACON_SEED (a seed file): an existing beacon — the one the pages extend (`serve`: the page's tokens on the page's
    // clock); otherwise a fresh one, ticked by this run.
    let beacon = match std::env::var("BEACON_SEED") {
        Ok(f) => container(&bcode, node_io::beacon_params(node_io::seed_file(&f)?).encode()).key().id().as_bytes().try_into().unwrap(),
        Err(_) => start_beacon(&bcode, &x, salt).await?,
    };
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
    let who = if std::env::var("BEACON_SEED").is_ok() { "the pages' (they extend it)" } else { "ticked by this run" };
    println!("beacon {} on X, {who}; height {}", hex(&beacon[..4]), w.now().await?);
    for mode in a.get(5).map_or(vec!["basic", "allow", "swap"], |m| vec![m.as_str()]) {
        match mode {
            "basic" => basic(&w).await?,
            "allow" => allow(&w).await?,
            "did" => did_mode(&w).await?,
            "mint" => mint_mode(&w).await?,
            "multisig" => multisig(&w).await?,
            "nft" => nft(&w, &std::fs::read(std::path::Path::new(&a[1]).with_file_name("swap.wasm"))?).await?,
            "editions" => editions(&w, &std::fs::read(std::path::Path::new(&a[1]).with_file_name("swap.wasm"))?).await?,
            "soulbound" => soulbound(&w).await?,
            "fees" => fees(&w).await?,
            "pause" => pause(&w).await?,
            "wallet" => wallet_mode(&w).await?,
            "ondemand" => ondemand(&w).await?,
            "serve" => serve(&w).await?,
            "reclaim" => reclaim(&w, &std::fs::read(std::path::Path::new(&a[1]).with_file_name("swap.wasm"))?).await?,
            "holders" => holders(&w, &std::fs::read(std::path::Path::new(&a[1]).with_file_name("swap.wasm"))?).await?,
            "vest" => vest(&w).await?,
            "operators" => operators_mode(&w).await?,
            "misdeed" => misdeed_mode(&w).await?,
            "outputs" => outputs_mode(&w, &bcode).await?,
            "deal" => deal_mode(&w, &std::fs::read(std::path::Path::new(&a[1]).with_file_name("swap.wasm"))?).await?,
            "pool" => pool_mode(&w, &std::fs::read(std::path::Path::new(&a[1]).with_file_name("pool.wasm"))?).await?,
            "standing" => standing_mode(&w, &bcode, &std::fs::read(std::path::Path::new(&a[1]).with_file_name("standing.wasm"))?).await?,
            "witness" => witness_mode(&w).await?,
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


/// FOUNDATION 1, LIVE: the PRUNE CUT. A holder checks a cut against what it summarises and keeps the chain from it; a
/// send received before a cut is not received again; a send behind its sender's cut is received with the proof (and
/// refused without it); A's chain read back WHOLE across its cut from the history's blocks, published as Block contracts.
async fn cut(w: &W, block_code: &[u8]) -> Result<()> {
    use craftworks_token_contract::cut;
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
    // A's chain read back WHOLE across its cut, from the blocks on the network.
    let (x, y, ch, beacon, bc, idlog) = (w.x.clone(), w.y.clone(), w.ch, w.beacon, block_code.to_vec(), w.idlog);
    let v = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let p = T::ChainParams { code_hash: ch, definition: ta, owner: did(1), beacon, idlog, item: [0; 32] };
        let live = rt.block_on(get(&y, p.id())).ok().flatten().and_then(|b| T::State::decode(&b));
        // Only what is HELD is read; what is NEEDED is fetched (the blocks the tree names — in a page, through the
        // blocks door: raced against their groups, rebuilt from parity) and the read runs again.
        let held: std::cell::RefCell<std::collections::HashMap<[u8; 32], Vec<u8>>> = Default::default();
        let t = Instant::now();
        let (mut rounds, mut fetched) = (0, 0);
        let r = loop {
            rounds += 1;
            let Some(live) = &live else { break Err("A's chain not on Y".to_string()) };
            let block = |c: &[u8; 32]| held.borrow().get(c).cloned();
            match cut::whole(live, &block) {
                Ok(s) => break Ok(s.entries.len()),
                Err(cut::Read::Refused(why)) => break Err(format!("REFUSED: {why}")),
                Err(cut::Read::Need(n)) if rounds <= 20 => {
                    for c in n {
                        let b = rt.block_on(get(&x, wire::block::contract_for(&bc, &c))).ok().flatten().and_then(|st| wire::block::block_of_state(&st).filter(|(id, _)| *id == c).map(|(_, b)| b.to_vec()));
                        if let Some(b) = b {
                            held.borrow_mut().insert(c, b);
                            fetched += 1;
                        }
                    }
                }
                Err(cut::Read::Need(n)) => break Err(format!("still needs {} block(s)", n.len())),
            }
        };
        (r, rounds, fetched, t.elapsed().as_millis())
    })
    .await?;
    println!("11 A's chain read back whole across its cut: {:?} entries — {} run(s), {} history block(s) fetched as needed, {} ms", v.0, v.1, v.2, v.3);
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
    let mut m = T::supply::Mint { to: did(to), item: [0; 32], amount, nonce, stamp, sigs: vec![], paid: None };
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
    push(&mut cb, T::Body::Mint { nonce: w.n(1), amount: 500, minted: h, mint: m1.clone(), ev: T::Evidence::default() }, w.n(1), None, r);
    println!("4  B's chain credits the mint, on Y: {} — balance {}", w.put_chain(&w.y, ta, 2, &cb).await?.0, w.get_chain(&w.y, ta, 2).await?.map_or(0, |s| s.balance()));
    let mut cc = T::State::default();
    push(&mut cc, T::Body::Mint { nonce: w.n(3), amount: 700, minted: h, mint: a_mint(&ta, 3, 700, w.n(3), h, 60), ev: T::Evidence::default() }, w.n(3), None, r);
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
    println!("14 totalSupply (mints in the shards, from X): {:?}", T::supply::minted(&d, &shards));
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

async fn nft(w: &W, scode: &[u8]) -> Result<()> {
    use craftworks_swap_contract as S;
    println!("— nft (an item sold through a deal: its creator's royalty paid)");
    let (m, t, c) = (1u8, 2u8, 7u8);
    let du = T::Definition { ids: T::Ids::Unique, mint: Some(one(60)), royalty: Some((500, did(c))), min_send: 1, ..w.def(m, b"Nft", 11) };
    let tu = w.put_def(&du).await?;
    let tb = w.put_def(&w.def(t, b"Pay", 12)).await?;
    let h = w.now().await?;
    let e = T::supply::epoch(h);
    let item = T::unique_item(&tu, e, &w.n(1));
    let it = T::Tok { definition: tu, item };
    let ip = |o: u8| T::ChainParams { item, ..w.chain(tu, o) };
    let shard = T::supply::Params { code_hash: w.ch, definition: tu, beacon: w.beacon, epoch: e };
    let mut mint = T::supply::Mint { to: did(m), item, amount: 1, nonce: w.n(1), stamp: h, sigs: vec![], paid: None };
    mint.sigs = vec![(0, key(60).sign(&mint.message(&tu)).to_bytes())];
    println!("1  the item minted to M (key 60), on X: {}", put(&w.x, container(&w.code, shard.encode()), T::supply::Shard { mints: vec![mint.clone()] }.encode()).await?.0);
    let mut two = T::supply::Mint { amount: 2, nonce: w.n(2), item: T::unique_item(&tu, e, &w.n(2)), ..mint.clone() };
    two.sigs = vec![(0, key(60).sign(&two.message(&tu)).to_bytes())];
    let both = T::supply::merge(&T::supply::Shard { mints: vec![mint.clone()] }, &T::supply::Shard { mints: vec![two] }).unwrap();
    println!("2  a mint of TWO of a unique item, on X: {}", put(&w.x, container(&w.code, shard.encode()), both.encode()).await?.0);
    let r = w.now().await?;
    let deal = S::Deal { x: it, party_x: did(m), amount_x: 1, y: tb.into(), party_y: did(t), amount_y: 100, deadline: r + 30, beacon: w.beacon };
    let prog = *container(scode, deal.encode()).key().id().as_bytes().first_chunk::<32>().unwrap();
    let mut mi = T::State::default();
    push(&mut mi, T::Body::Mint { nonce: w.n(1), amount: 1, minted: h, mint: mint.clone(), ev: T::Evidence::default() }, w.n(1), None, r);
    push(&mut mi, T::Body::Escrow { program: prog, terms: S::TERMS.to_vec(), amount: 1, until: 0 }, w.n(7), Some(m), r);
    println!("3  M's chain of the item: the mint, the item escrowed with the deal (for 100 B), on X: {} — holds {}", put(&w.x, container(&w.code, ip(m).encode()), mi.encode()).await?.0, mi.balance());
    let mut tbc = T::State::default();
    push(&mut tbc, T::Body::Genesis { amount: 1000 }, w.n(1), Some(t), r);
    push(&mut tbc, T::Body::Escrow { program: prog, terms: S::TERMS.to_vec(), amount: 100, until: 0 }, w.n(8), Some(t), r);
    println!("4  T escrows 100 B with the deal, on Y: {} — T's B {}", w.put_chain(&w.y, tb, t, &tbc).await?.0, tbc.balance());
    let (ex, ey) = ((it, did(m), w.n(7)), (tb.into(), did(t), w.n(8)));
    let hs = T::escrow::terms_hash(S::TERMS);
    let alone = |r: T::escrow::EscrowRef, a: u128| T::escrow::Section { escrows: vec![(r, a, hs)], ..Default::default() };
    let dc = || container(scode, deal.encode());
    println!("5  the deal crossed WITHOUT the creator's fee, on X: {}", put(&w.x, dc(), S::crossed(&deal, (None, None), &ex, &ey).encode()).await?.0);
    println!("6  the deal with M's escrow, on X: {}", put(&w.x, dc(), alone(ex, 1).encode()).await?.0);
    println!("7  … and T's (it crosses, reading the item's royalty), on X: {}", put(&w.x, dc(), alone(ey, 100).encode()).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let held = get(&w.x, prog).await?.and_then(|b| T::escrow::Section::decode_prefix(&b).map(|x| x.0)).context("the deal")?;
    let want = S::crossed(&deal, (Some((500, did(c))), None), &ex, &ey);
    println!("   the deal holds: crossed with the fee {} — M {:?}, creator {:?}", held == want, held.pay(&S::pay_key(&ex)).map(|p| p.2), held.pay(&S::fee_key(&ex)).map(|p| p.2));
    let now = w.now().await?;
    let pay = |key: [u8; 16], amount: u128, n: u8| {
        let mut s = T::State::default();
        push(&mut s, T::Body::Payout { program: prog, key, amount, verdict: held.hash(), ev: T::Evidence::default() }, w.n(n), None, now);
        s
    };
    let mut ti = T::State::default();
    push(&mut ti, T::Body::Payout { program: prog, key: S::pay_key(&ey), amount: 1, verdict: held.hash(), ev: T::Evidence::default() }, w.n(30), None, now);
    println!("8  T paid the item, on X: {}", put(&w.x, container(&w.code, ip(t).encode()), ti.encode()).await?.0);
    println!("9  M paid 95 B, through X: {}", w.put_chain(&w.x, tb, m, &pay(S::pay_key(&ex), 95, 31)).await?.0);
    println!("10 the creator paid its 5 B, through X: {}", w.put_chain(&w.x, tb, c, &pay(S::fee_key(&ex), 5, 32)).await?.0);
    println!("11 an outsider (8) claims the creator's 5, through X: {}", w.put_chain(&w.x, tb, 8, &pay(S::fee_key(&ex), 5, 33)).await?.0);
    let mut back = mi.clone();
    push(&mut back, T::Body::Unescrow { escrow: w.n(7), refund: 0, verdict: held.hash() }, w.n(34), None, now);
    println!("12 M's escrow released (nothing back), through Y: {}", put(&w.y, container(&w.code, ip(m).encode()), back.encode()).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let bal = |b: Option<Vec<u8>>| b.and_then(|b| T::State::decode(&b)).map_or(0, |s| s.balance());
    println!(
        "   item: M {} · T {} — B: M {} · creator {} · outsider {}",
        bal(get(&w.y, ip(m).id()).await?),
        bal(get(&w.x, ip(t).id()).await?),
        w.get_chain(&w.x, tb, m).await?.map_or(0, |s| s.balance()),
        w.get_chain(&w.x, tb, c).await?.map_or(0, |s| s.balance()),
        w.get_chain(&w.x, tb, 8).await?.map_or(0, |s| s.balance())
    );
    Ok(())
}

/// FOUNDATION 6, LIVE: WITNESSES BY LOT, weighted by stake. 16 stakers earn and stake 50 each in a POOL token, each
/// CLAIMING the next epoch in its own chain (`Body::Claim`, TOKEN §7a C2b). In another token A pays B; C credits itself 5000
/// from a send that does not exist, buried under real receives, and pays D out of it. Each entry's committee — drawn
/// from the claims by the beacon output after it — re-checks it by the witness rule and signs into its place's signatures
/// container (`witness::Params`). B's wallet accepts its payment from the signatures alone; D's is never witnessed. A
/// witness signing twice at one place is burned.
async fn witness_mode(w: &W) -> Result<()> {
    use T::{draw, supply, wallet};
    println!("— witness");
    let stakers: Vec<u8> = (20..36).collect();
    let t = Instant::now();
    for &n in &stakers {
        let l = log(n);
        let (sx, sy) = tokio::join!(w.put_log(&w.x, &l), w.put_log(&w.y, &l));
        if sx?.0 != "ACCEPTED" || sy?.0 != "ACCEPTED" {
            bail!("staker {n}'s key log was not taken");
        }
    }
    println!("1  key logs of {} stakers on X and Y in {} ms", stakers.len(), t.elapsed().as_millis());
    let pd = w.def(1, b"Pool", 11);
    let pool = w.put_def(&pd).await?;
    let h = w.now().await?;
    let mut g = T::State::default();
    push(&mut g, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h);
    for &n in &stakers {
        push(&mut g, T::Body::Send { to: did(n), amount: 50 }, w.n(n), Some(1), h);
    }
    println!("2  the pool token: genesis, 50 to each staker, on X: {}", w.put_chain(&w.x, pool, 1, &g).await?.0);
    let r = w.until(h + 2).await?;
    // Stake counts for an epoch as held at its start: to the next one — CLAIMED in each staker's own chain before that
    // epoch freezes (an eighth of an epoch before it starts).
    let epoch = supply::epoch(r) + 1;
    let start = epoch * supply::EPOCH;
    let mut took = 0;
    for &n in &stakers {
        let mut s = T::State::default();
        push(&mut s, T::Body::Receive { from: did(1), nonce: w.n(n), amount: 50, cited: h, ev: T::Evidence::default() }, w.n(n), None, r);
        push(&mut s, T::Body::Stake { amount: 50 }, w.n(100 + n), Some(n), r);
        push(&mut s, T::Body::Claim { from_epoch: epoch, amount: 50 }, w.n(140 + n), Some(n), r);
        took += (w.put_chain(&w.y, pool, n, &s).await?.0 == "ACCEPTED") as usize;
    }
    println!("3  each staker receives its 50, stakes it and claims epoch {epoch} (at {r}; it freezes at {}), on Y: {took} of {} ACCEPTED", T::pointers::frozen_at(epoch), stakers.len());
    let t = Instant::now();
    FAST.store(true, std::sync::atomic::Ordering::Relaxed);
    let reached = loop {
        let n = w.now().await?;
        if n >= start + 2 {
            break n;
        }
        if t.elapsed() > Duration::from_secs(40 * 60) {
            FAST.store(false, std::sync::atomic::Ordering::Relaxed);
            bail!("the beacon did not reach epoch {epoch} ({start}); at {n}");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    FAST.store(false, std::sync::atomic::Ordering::Relaxed);
    println!("5  beacon fast-forwarded to epoch {epoch} (height {reached}) in {} s", t.elapsed().as_secs());
    // A claim AFTER the freeze (staker 20 claims epoch {epoch} again): the contract refuses it.
    let late = {
        let mut s = get(&w.y, w.chain(pool, 20).id()).await?.and_then(|b| T::State::decode(&b)).unwrap_or_default();
        push(&mut s, T::Body::Claim { from_epoch: epoch, amount: 50 }, w.n(199), Some(20), reached);
        w.put_chain(&w.y, pool, 20, &s).await?.0
    };
    println!("   a claim after the freeze, on Y: {late}");
    // Token A: 2 pays 3; 4 buries a fake receive and pays 5 out of it.
    let td = w.def(2, b"Paid", 12);
    let ta = w.put_def(&td).await?;
    let h2 = w.now().await?;
    let mut a = T::State::default();
    push(&mut a, T::Body::Genesis { amount: 1000 }, w.n(2), Some(2), h2);
    push(&mut a, T::Body::Send { to: did(3), amount: 300 }, w.n(30), Some(2), h2);
    for k in 0..T::RECENT as u8 {
        push(&mut a, T::Body::Send { to: did(4), amount: 10 }, w.n(40 + k), Some(2), h2);
    }
    println!("6  2: genesis, 300 to 3, {} sends of 10 to 4, on X: {}", T::RECENT, w.put_chain(&w.x, ta, 2, &a).await?.0);
    let r2 = w.until(h2 + 2).await?;
    let mut b = T::State::default();
    push(&mut b, T::Body::Receive { from: did(2), nonce: w.n(30), amount: 300, cited: h2, ev: T::Evidence::default() }, w.n(30), None, r2);
    println!("7  3 receives the 300, on Y: {}", w.put_chain(&w.y, ta, 3, &b).await?.0);
    let mut c = T::State::default();
    push(&mut c, T::Body::Receive { from: did(2), nonce: w.n(99), amount: 5000, cited: h2, ev: T::Evidence::default() }, w.n(99), None, r2);
    for k in 0..T::RECENT as u8 {
        push(&mut c, T::Body::Receive { from: did(2), nonce: w.n(40 + k), amount: 10, cited: h2, ev: T::Evidence::default() }, w.n(40 + k), None, r2);
    }
    push(&mut c, T::Body::Send { to: did(5), amount: 3000 }, w.n(50), Some(4), r2);
    println!("8  4: a FAKE receive of 5000 buried under {} real ones, then 3000 to 5, on Y: {}", T::RECENT, w.put_chain(&w.y, ta, 4, &c).await?.0);
    let r3 = w.until(r2 + 2).await?;
    let mut d = T::State::default();
    push(&mut d, T::Body::Receive { from: did(4), nonce: w.n(50), amount: 3000, cited: r2, ev: T::Evidence::default() }, w.n(50), None, r3);
    println!("9  5 receives the 3000, on Y: {}", w.put_chain(&w.y, ta, 5, &d).await?.0);
    // Let the lot of the last entry be drawn (the beacon output two steps past its last append chance).
    w.until(r3 + td.margin + 3).await?;

    // THE WITNESSES and THE WALLETS: everything read from the nodes (blocking reads on a runtime of their own).
    let (x, y, beacon, tc, ch, bid, idlog) = (w.x.clone(), w.y.clone(), w.beacon, w.code.clone(), w.ch, w.beacon, w.idlog);
    let (pd2, td2) = (pd.clone(), td.clone());
    let lines = tokio::task::spawn_blocking(move || -> Vec<String> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let out = std::cell::RefCell::new(Vec::<String>::new());
        let params = |t: [u8; 32], o: u8| T::ChainParams { code_hash: ch, definition: t, owner: did(o), beacon: bid, idlog, item: [0; 32] };
        let read = |p: &T::ChainParams| rt.block_on(get(&y, p.id())).ok().flatten().and_then(|b| T::State::decode(&b));
        let beacon_state = std::cell::RefCell::new(None::<B::State>);
        let out_at = |hh: u64| -> Option<[u8; 32]> {
            let find = |s: &B::State| s.steps.iter().find(|st| st.height == hh).map(|st| st.out);
            if let Some(o) = beacon_state.borrow().as_ref().and_then(find) {
                return Some(o);
            }
            let s = rt.block_on(get(&x, beacon)).ok().flatten().and_then(|b| B::wire::decode(&b))?;
            let o = find(&s);
            *beacon_state.borrow_mut() = Some(s);
            o
        };
        // The stakers' SEATS of claims epoch `e`: each one's claim in its own pool chain (`wallet::seat_of`; this harness
        // seats them before any witness accepted them, as the registry did).
        let claims_of = |e: u64| (20..36u8).filter_map(|n| wallet::seat_of(&params(pool, n), &pd2, &read(&params(pool, n))?, e).map(|x| x.0)).collect::<Vec<_>>();
        // A place's signatures: its container under the lot's claims epoch (`witness::Params`), on X.
        let place_of = |cid: &[u8; 32], seq: u64, ep: u64| T::witness::Params { code_hash: ch, pool, beacon: bid, idlog, epoch: ep, chain: *cid, seq };
        let sigs_of = |cid: &[u8; 32], seq: u64, ep: u64| {
            let wp = place_of(cid, seq, ep);
            rt.block_on(get(&x, wp.id())).ok().flatten().map(|b| wp.sigs(&b)).unwrap_or_default()
        };
        let stake_reads = std::cell::Cell::new(0usize);
        let stake_of = |o: &[u8; 32], at: u64| {
            stake_reads.set(stake_reads.get() + 1);
            let n = (20..36u8).find(|n| did(*n) == *o)?;
            let p = params(pool, n);
            wallet::stake_at(&p, &pd2, &read(&p)?, at).ok()
        };
        let defs = |t: &[u8; 32]| if *t == ta { Some(td2.clone()) } else if *t == pool { Some(pd2.clone()) } else { None };
        let src = wallet::Source { pool, claims: &claims_of, sigs: &sigs_of, out: &out_at, stake_of: &stake_of, definition: &defs, program: &|_| None, freeze: &|_| None, keys: &|o: &[u8; 32]| (1..=99u8).find(|n| did(*n) == *o).map(log), history: &|_, _| None, standing: &|_| None };
        let cited = |t: &T::Tok, o: &[u8; 32]| rt.block_on(get(&y, T::ChainParams { code_hash: ch, definition: t.definition, owner: *o, beacon: bid, idlog, item: t.item }.id())).ok().flatten().and_then(|b| T::State::decode(&b));
        // Witness one entry: its committee drawn, each member re-checks it by the rule and signs into its container on X.
        let witness = |o: u8, seq: u64, who: &str| -> bool {
            let p = params(ta, o);
            let Some(s) = read(&p) else {
                out.borrow_mut().push(format!("   {who} seq {seq}: chain not on Y"));
                return false;
            };
            let i = s.entries.iter().position(|e| e.seq == seq).unwrap();
            let e = s.entries[i].clone();
            let Ok((ep, hh)) = wallet::lot_of(&e, i.checked_sub(1).map(|j| &s.entries[j]), td2.margin) else {
                out.borrow_mut().push(format!("   {who} seq {seq}: no lot (its predecessor not held)"));
                return false;
            };
            let claims = claims_of(ep);
            let Some(o_) = out_at(hh) else {
                out.borrow_mut().push(format!("   {who} seq {seq}: its lot is not drawn"));
                return false;
            };
            let members = draw::committee(&pool, &o_, &p.id(), seq, ep, &claims, wallet::COMMITTEE);
            let holds = |st: &T::State| T::check(&p, &td2, st, &cited, &|_| None, Some(&log(o)), &T::Extra::NONE);
            let now = rt.block_on(get(&x, beacon)).ok().flatten().and_then(|b| B::wire::decode(&b)).map_or(0, |b| b.height());
            match wallet::may_witness(&src, &p, &td2, &s, seq, now, &holds, &cited) {
                Err(why) => {
                    out.borrow_mut().push(format!("   {who} seq {seq}: drawn {} — REFUSED by every one: {why}", members.len()));
                    false
                }
                Ok(hash) => {
                    let wp = place_of(&p.id(), seq, ep);
                    let items: Vec<([u8; 32], [u8; 32], T::witness::WitnessSig)> = members
                        .iter()
                        .map(|&m| {
                            let n = (20..36u8).find(|n| pk(*n) == claims[m].key).unwrap(); // the member's own key
                            let sig = key(n).sign(&T::witness::WitnessSig::message(&p.id(), seq, &hash, now)).to_bytes();
                            (pk(n), did(n), T::witness::WitnessSig { chain: p.id(), seq, entry: hash, at: now, sig })
                        })
                        .collect();
                    let said = rt.block_on(put(&x, container(&tc, wp.encode()), wp.state(&items))).map(|r| r.0).unwrap_or_else(|e| e.to_string());
                    out.borrow_mut().push(format!("   {who} seq {seq}: drawn {}, each re-checked it and signed — container on X: {said}", members.len()));
                    true
                }
            }
        };
        out.borrow_mut().push("10 the committees witness, in order:".into());
        let len = |o: u8| read(&params(ta, o)).map_or(0, |s| s.entries.len() as u64);
        for seq in 0..len(2) {
            witness(2, seq, "2");
        }
        witness(3, 0, "3");
        for seq in 0..len(4) {
            if !witness(4, seq, "4") {
                break;
            }
        }
        witness(5, 0, "5");
        // THE WALLETS: an entry accepted from its signatures alone.
        let accept = |o: u8, seq: u64| {
            let p = params(ta, o);
            let s = read(&p)?;
            let i = s.entries.iter().position(|e| e.seq == seq)?;
            Some(wallet::accept(&src, &p, &s.entries[i], i.checked_sub(1).map(|j| &s.entries[j])))
        };
        let t = Instant::now();
        stake_reads.set(0);
        out.borrow_mut().push(format!("11 3's wallet: its 300 — {:?} ({} stake chains read, {} ms)", accept(3, 0), stake_reads.get(), t.elapsed().as_millis()));
        out.borrow_mut().push(format!("12 5's wallet: its 3000 — {:?}", accept(5, 0)));
        out.into_inner()
    })
    .await?;
    for l in lines {
        println!("{l}");
    }
    // AN EQUIVOCATION: staker 20 signs two entries at one place, into that place's signatures container; a WATCHER reads
    // it and the epoch's stakers, finds it (`wallet::equivocations`) and appends the burn to the staker's pool chain.
    let other = |e: u8| *blake3::hash(&[e]).as_bytes();
    let place = w.chain(ta, 2).id();
    let at = w.now().await?;
    let sg = |e: u8| T::witness::WitnessSig { chain: place, seq: 1, entry: other(e), at, sig: key(20).sign(&T::witness::WitnessSig::message(&place, 1, &other(e), at)).to_bytes() };
    let wp = T::witness::Params { code_hash: w.ch, pool, beacon: w.beacon, idlog: w.idlog, epoch, chain: place, seq: 1 };
    let said = put(&w.x, container(&w.code, wp.encode()), wp.state(&[(pk(20), did(20), sg(1)), (pk(20), did(20), sg(2))])).await?.0;
    println!("13 staker 20 signs two other entries at 2's seq 1, into its signatures container on X: {said}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let sigs: Vec<T::witness::WitnessSig> = get(&w.x, wp.id()).await?.map(|b| wp.sigs(&b)).unwrap_or_default();
    let mut registry = Vec::new();
    for &n in &stakers {
        let cp = w.chain(pool, n);
        if let Some(st) = get(&w.y, cp.id()).await?.and_then(|b| T::State::decode(&b)) {
            registry.extend(wallet::seat_of(&cp, &pd, &st, epoch).map(|x| x.0));
        }
    }
    let found = wallet::equivocations(&sigs, &registry);
    println!("   the watcher: {} signatures at that place, {} equivocator(s): {:?}", sigs.len(), found.len(), found.iter().map(|f| stakers.iter().find(|n| did(**n) == f.0).copied()).collect::<Vec<_>>());
    let (owner, burn) = found.into_iter().next().context("no equivocator found")?;
    let n = stakers.iter().copied().find(|n| did(*n) == owner).context("not a staker")?;
    let mut s20 = w.get_chain(&w.y, pool, n).await?.context("the staker's chain")?;
    let now = w.now().await?;
    push(&mut s20, burn, w.n(120), None, now);
    println!("   its burn, appended by the watcher, on Y: {}", w.put_chain(&w.y, pool, n, &s20).await?.0);
    let held = w.get_chain(&w.y, pool, 20).await?.context("staker 20's chain")?;
    let p20 = w.chain(pool, 20);
    println!("   its stake now: {:?}; as counted for epoch {epoch}: {:?}", T::wallet::stake_at(&p20, &pd, &held, u64::MAX).map(|x| x.0), T::wallet::stake_at(&p20, &pd, &held, start).map(|x| x.0));
    Ok(())
}

/// VESTING, LIVE (TOKEN §4a): 2 vests 3 a schedule of two steps; each step is credited only once the beacon reaches it,
/// once; the giver's balance carries the whole debit from the start.
async fn vest(w: &W) -> Result<()> {
    println!("— vest");
    let d = w.def(2, b"Vest", 13);
    let ta = w.put_def(&d).await?;
    let h = w.now().await?;
    let (s0, s1) = (h + 6, h + 10);
    let mut g = T::State::default();
    push(&mut g, T::Body::Genesis { amount: 1000 }, w.n(2), Some(2), h);
    push(&mut g, T::Body::Vest { to: did(3), schedule: vec![(s0, 100), (s1, 200)] }, w.n(5), Some(2), h);
    println!("1  2: genesis, vests 3 100 at {s0} and 200 at {s1} (now {h}), on X: {} — 2's balance {}", w.put_chain(&w.x, ta, 2, &g).await?.0, g.balance());
    let step = |c: &T::State, k: u32, amount: u128, at: u64, stamp: u64| {
        let mut c = c.clone();
        push(&mut c, T::Body::Vested { from: did(2), vest: w.n(5), step: k, amount, at, cited: h, ev: T::Evidence::default() }, w.n(60 + k as u8), None, stamp);
        c
    };
    let r = w.until(h + 2).await?;
    let early = step(&T::State::default(), 0, 100, s0, r);
    println!("2  3 claims step 0 before its time (now {r}), on Y: {}", w.put_chain(&w.y, ta, 3, &early).await?.0);
    let r0 = w.until(s0).await?;
    let c0 = step(&T::State::default(), 0, 100, s0, r0);
    println!("3  … at its time (now {r0}), on Y: {}", w.put_chain(&w.y, ta, 3, &c0).await?.0);
    let more = step(&c0, 0, 100, s0, w.now().await?);
    println!("4  step 0 again, on Y: {}", w.put_chain(&w.y, ta, 3, &more).await?.0);
    let wrong = step(&c0, 1, 300, s1, w.until(s1).await?);
    println!("5  step 1 claiming 300 (the schedule says 200), on Y: {}", w.put_chain(&w.y, ta, 3, &wrong).await?.0);
    let c1 = step(&c0, 1, 200, s1, w.now().await?);
    println!("6  step 1 as scheduled, on Y: {}", w.put_chain(&w.y, ta, 3, &c1).await?.0);
    let held = w.get_chain(&w.y, ta, 3).await?.context("3's chain")?;
    println!("   3's balance on Y: {}; 2's: {}", held.balance(), g.balance());
    Ok(())
}

/// OPERATORS, LIVE (TOKEN §1b): M approves operator 5 for ALL its items of a token at once (`setApprovalForAll`); 5
/// moves two different items from their own chains; an operator never approved is refused; M turns 5 off and 5 is
/// refused from then on — the spends made while on stay valid; a turn-off dated in the past is refused.
async fn operators_mode(w: &W) -> Result<()> {
    use T::operators as O;
    println!("— operators");
    let (m, t) = (1u8, 2u8);
    let du = T::Definition { ids: T::Ids::Unique, mint: Some(one(60)), min_send: 1, ..w.def(m, b"Ops", 14) };
    let tu = w.put_def(&du).await?;
    let h = w.now().await?;
    let e = T::supply::epoch(h);
    let items: Vec<[u8; 32]> = (1..=3u8).map(|k| T::unique_item(&tu, e, &w.n(k))).collect();
    let mints: Vec<T::supply::Mint> = (1..=3u8)
        .map(|k| {
            let mut mm = T::supply::Mint { to: did(m), item: items[k as usize - 1], amount: 1, nonce: w.n(k), stamp: h, sigs: vec![], paid: None };
            mm.sigs = vec![(0, key(60).sign(&mm.message(&tu)).to_bytes())];
            mm
        })
        .collect();
    let shard = T::supply::Params { code_hash: w.ch, definition: tu, beacon: w.beacon, epoch: e };
    println!("1  three items minted to M, on X: {}", put(&w.x, container(&w.code, shard.encode()), T::supply::Shard { mints: mints.clone() }.encode()).await?.0);
    let ip = |k: usize, o: u8| T::ChainParams { item: items[k], ..w.chain(tu, o) };
    let r = w.now().await?;
    let mut chains = Vec::new();
    for k in 0..3 {
        let mut c = T::State::default();
        push(&mut c, T::Body::Mint { nonce: w.n(k as u8 + 1), amount: 1, minted: h, mint: mints[k].clone(), ev: T::Evidence::default() }, w.n(k as u8 + 1), None, r);
        let said = put(&w.x, container(&w.code, ip(k, m).encode()), c.encode()).await?.0;
        println!("2  M's chain of item {}: its mint, on X: {said}", k + 1);
        chains.push(c);
    }
    let op = O::Params { code_hash: w.ch, definition: tu, owner: did(m), beacon: w.beacon, idlog: w.idlog };
    let record = |operator: u8, spans: Vec<(u64, Option<u64>)>, seq: u32| {
        let mut rc = O::Record { operator: pk(operator), spans, seq, key_seq: 0, sig: [0; 64] };
        rc.sig = SigningKey::from_bytes(&owner_seed(m)).sign(&rc.message(&op)).to_bytes();
        rc
    };
    let on_at = w.now().await?;
    let list = O::List { records: vec![record(5, vec![(on_at, None)], 1)] };
    println!("3  M approves operator 5 for ALL its items of the token (at {on_at}), on X: {}", put(&w.x, container(&w.code, op.encode()), list.encode()).await?.0);
    let spend = |c: &T::State, k: u8, by: u8, stamp: u64| {
        let mut c = c.clone();
        push_as(&mut c, T::Body::Spend { to: did(t), amount: 1 }, w.n(20 + k), Signer_::Spender(by), stamp);
        c
    };
    let now = w.now().await?;
    println!("4  operator 5 sends item 1 to T, through Y: {}", put(&w.y, container(&w.code, ip(0, m).encode()), spend(&chains[0], 1, 5, now).encode()).await?.0);
    println!("5  operator 5 sends item 2 to T, through Y: {}", put(&w.y, container(&w.code, ip(1, m).encode()), spend(&chains[1], 2, 5, now).encode()).await?.0);
    println!("6  operator 6 (never approved) sends item 3, through Y: {}", put(&w.y, container(&w.code, ip(2, m).encode()), spend(&chains[2], 3, 6, now).encode()).await?.0);
    let now = w.until(on_at + 6).await?;
    let back = O::List { records: vec![record(5, vec![(on_at, Some(on_at + 1))], 2)] };
    println!("7  M turns 5 off DATED IN THE PAST (at {}; now {now}, margin {}), on X: {}", on_at + 1, du.margin, put(&w.x, container(&w.code, op.encode()), back.encode()).await?.0);
    let off = O::List { records: vec![record(5, vec![(on_at, Some(now))], 2)] };
    println!("8  M turns 5 off now ({now}), on X: {}", put(&w.x, container(&w.code, op.encode()), off.encode()).await?.0);
    let after = w.until(now + 1).await?;
    println!("9  operator 5 sends item 3 after it, through Y: {}", put(&w.y, container(&w.code, ip(2, m).encode()), spend(&chains[2], 4, 5, after).encode()).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let held = |b: Option<Vec<u8>>| b.and_then(|b| T::State::decode(&b)).map(|s| s.balance());
    println!(
        "   M holds item 1: {:?} · item 2: {:?} · item 3: {:?} (on Y)",
        held(get(&w.y, ip(0, m).id()).await?),
        held(get(&w.y, ip(1, m).id()).await?),
        held(get(&w.y, ip(2, m).id()).await?)
    );
    Ok(())
}

/// An extender's MIX: fresh entropy for the step it adds (the clock's nanoseconds and a counter, hashed).
fn fresh_mix() -> [u8; 32] {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    *blake3::hash(&[&t.to_le_bytes()[..], &n.to_le_bytes()].concat()).as_bytes()
}

/// A WITNESS'S MISDEED, LIVE (TOKEN §7a): staker 2 (witness key device 2) signs 3's buried FAKE receive; anyone appends
/// the proof to 2's stake chain — the node reads the sender's chain, finds no such send, and the stake is burned. A
/// "misdeed" over a REAL receive is refused.
async fn misdeed_mode(w: &W) -> Result<()> {
    println!("— misdeed");
    let da = w.def(1, b"Fake", 15);
    let ta = w.put_def(&da).await?;
    let dp = w.def(2, b"Stake", 16);
    let tp = w.put_def(&dp).await?;
    let h = w.now().await?;
    let mut a1 = T::State::default();
    push(&mut a1, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h);
    for k in 0..T::RECENT as u8 {
        push(&mut a1, T::Body::Send { to: did(3), amount: 10 }, w.n(20 + k), Some(1), h);
    }
    println!("1  1: genesis, {} sends of 10 to 3, on X: {}", T::RECENT, w.put_chain(&w.x, ta, 1, &a1).await?.0);
    let mut st = T::State::default();
    push(&mut st, T::Body::Genesis { amount: 1000 }, w.n(2), Some(2), h);
    push(&mut st, T::Body::Stake { amount: 600 }, w.n(3), Some(2), h);
    println!("2  2 stakes 600 (its witness key: device 2), on Y: {}", w.put_chain(&w.y, tp, 2, &st).await?.0);
    let r = w.until(h + 2).await?;
    let mut c3 = T::State::default();
    push(&mut c3, T::Body::Receive { from: did(1), nonce: w.n(99), amount: 5000, cited: h, ev: T::Evidence::default() }, w.n(99), None, r);
    for k in 0..T::RECENT as u8 {
        push(&mut c3, T::Body::Receive { from: did(1), nonce: w.n(20 + k), amount: 10, cited: h, ev: T::Evidence::default() }, w.n(20 + k), None, r);
    }
    println!("3  3: a FAKE receive of 5000 buried under {} real ones, on Y: {}", T::RECENT, w.put_chain(&w.y, ta, 3, &c3).await?.0);
    let place = w.chain(ta, 3).id();
    let proof = |seq: usize| {
        let e = c3.entries[seq].clone();
        let sig = T::witness::WitnessSig { chain: place, seq: e.seq, entry: e.hash(), at: r, sig: key(2).sign(&T::witness::WitnessSig::message(&place, e.seq, &e.hash(), r)).to_bytes() };
        T::Body::Misdeed { key: pk(2), sig, tok: ta.into(), owner: did(3), entry: Box::new(e) }
    };
    let now = w.now().await?;
    let mut real = st.clone();
    push(&mut real, proof(1), w.n(40), None, now);
    println!("4  a 'misdeed' over 3's REAL receive (2 signed it), on Y: {}", w.put_chain(&w.y, tp, 2, &real).await?.0);
    let mut burn = st.clone();
    push(&mut burn, proof(0), w.n(41), None, now);
    println!("5  the misdeed over the FAKE receive 2 signed — appended by anyone, on Y: {}", w.put_chain(&w.y, tp, 2, &burn).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let held = w.get_chain(&w.y, tp, 2).await?.context("2's chain")?;
    let staked = T::walk(&w.chain(tp, 2), &dp, &held, &|_, _| None, &|_| None, None, 0, held.entries.len(), &T::Extra::NONE).map(|c| c.staked);
    println!("   2's stake on Y now: {:?} (was 600)", staked.map_err(|e| e.1));
    Ok(())
}

/// The beacon's output at height `h` as the network holds it: from the beacon while it keeps that step, from the
/// OUTPUTS LOG after.
async fn out_at(ws: &str, beacon: [u8; 32], bcode: &[u8], h: u64) -> Result<Option<[u8; 32]>> {
    let b = get(ws, beacon).await?.and_then(|b| B::wire::decode(&b));
    if let Some(o) = b.as_ref().and_then(|b| b.steps.iter().find(|s| s.height == h)).map(|s| s.out) {
        return Ok(Some(o));
    }
    let lp = B::log::Params::of(beacon, h);
    Ok(get(ws, *container(bcode, lp.encode()).key().id().as_bytes().first_chunk::<32>().unwrap()).await?.and_then(|l| B::log::Log::decode(&l)).and_then(|l| l.get(h)))
}

/// THE OUTPUTS LOG, LIVE (TOKEN §5a): the beacon run past what it keeps (600 steps); its oldest outputs are gone from
/// it and still read — from the log — exactly as computed; a forged output for a recorded height changes nothing; one
/// for a height the beacon never held final is refused.
async fn outputs_mode(w: &W, bcode: &[u8]) -> Result<()> {
    println!("— outputs");
    let t = Instant::now();
    FAST.store(true, std::sync::atomic::Ordering::Relaxed);
    let reached = loop {
        let n = w.now().await?;
        if n >= 700 {
            break n;
        }
        if t.elapsed() > Duration::from_secs(20 * 60) {
            FAST.store(false, std::sync::atomic::Ordering::Relaxed);
            bail!("the beacon did not reach 700 (at {n})");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    FAST.store(false, std::sync::atomic::Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let b = get(&w.x, w.beacon).await?.and_then(|b| B::wire::decode(&b)).context("the beacon")?;
    println!("1  beacon fast-forwarded to {reached} in {} s; it keeps heights {}..={}", t.elapsed().as_secs(), b.steps.first().map_or(0, |s| s.height), b.height());
    let truth: Vec<(u64, [u8; 32])> = OUTS.lock().unwrap().clone();
    let mut same = 0;
    let probes = [1u64, 2, 50, 99, 300, 650];
    for &h in &probes {
        let got = out_at(&w.y, w.beacon, bcode, h).await?;
        let want = truth.iter().find(|x| x.0 == h).map(|x| x.1);
        same += (got.is_some() && got == want) as usize;
        println!("   height {h}: {} — {}", if b.steps.iter().any(|s| s.height == h) { "in the beacon" } else { "from the log" }, if got.is_none() { "MISSING".to_string() } else if got == want { "as computed".to_string() } else { "DIFFERENT".to_string() });
    }
    println!("2  {same} of {} probed outputs read back exactly (from Y)", probes.len());
    let lp = B::log::Params::of(w.beacon, 50);
    let lid = *container(bcode, lp.encode()).key().id().as_bytes().first_chunk::<32>().unwrap();
    let forged = B::log::Log { entries: vec![(50, [7; 32])] };
    println!("3  a FORGED output for height 50, on X: {}", put(&w.x, container(bcode, lp.encode()), forged.encode()).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let after = get(&w.x, lid).await?.and_then(|l| B::log::Log::decode(&l)).and_then(|l| l.get(50));
    println!("   height 50 now: {}", if after == truth.iter().find(|x| x.0 == 50).map(|x| x.1) { "as computed" } else { "CHANGED" });
    let future = B::log::Log { entries: vec![(4000, [7; 32])] };
    println!("4  an output for height 4000 (never held final), on X: {}", put(&w.x, container(bcode, lp.encode()), future.encode()).await?.0);
    Ok(())
}

/// THE STANDING CONTRACT, LIVE (TOKEN §7a): a peer's record — three attesters place it, key 7 bound, holdings over 40
/// contracts, an answer at each beacon height of standing epoch 2 sampled for it — put on a node, which reads the beacon
/// and its OUTPUTS LOG (the oldest answers are past what the beacon keeps) and checks it from scratch. A record with one
/// answer moved to another height (re-signed) is refused. Run with BEACON_START a little before 2048.
async fn standing_mode(w: &W, bcode: &[u8], scode: &[u8]) -> Result<()> {
    use craftworks_standing as S;
    use craftworks_standing_contract as SC;
    println!("— standing");
    let e = 2u64;
    let top = e * S::EPOCH + 700;
    let t = Instant::now();
    FAST.store(true, std::sync::atomic::Ordering::Relaxed);
    loop {
        let n = w.now().await?;
        if n >= top {
            break;
        }
        if t.elapsed() > Duration::from_secs(30 * 60) {
            FAST.store(false, std::sync::atomic::Ordering::Relaxed);
            bail!("the beacon did not reach {top} (at {n})");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    FAST.store(false, std::sync::atomic::Ordering::Relaxed);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let b = get(&w.x, w.beacon).await?.and_then(|b| B::wire::decode(&b)).context("the beacon")?;
    println!("1  beacon to {} in {} s; it keeps {}..={}", b.height(), t.elapsed().as_secs(), b.steps.first().map_or(0, |s| s.height), b.height());
    let truth: Vec<(u64, [u8; 32])> = OUTS.lock().unwrap().clone();
    let out = |h: u64| truth.iter().find(|x| x.0 == h).map(|x| x.1);
    let peer = "5cCFQqQwLPNKX4zwK";
    let addr: std::net::SocketAddr = "86.38.75.1:1".parse()?;
    let ring = S::ring_of_addr(&addr);
    let at = |n: u64| ring.wrapping_add(if n.is_multiple_of(2) { u64::MAX / 4 } else { (u64::MAX / 4).wrapping_neg() });
    let attests: Vec<S::Attest> = (1..=3u8).map(|n| S::Attest::new(&key(n), peer, &addr, at(n as u64), e)).collect();
    let k = key(7);
    let binds = vec![S::Bind::new(&k, peer, e)];
    let (_, arc_) = S::place(peer, e, &attests, &[k.verifying_key().to_bytes()], 3).context("placed")?;
    let hosted: Vec<([u8; 32], u64)> = (0..40u32).map(|n| (*blake3::hash(&n.to_le_bytes()).as_bytes(), 1000)).collect();
    let (h, ids) = S::Holdings::new(&k, peer, e, &hosted, arc_);
    let kb = k.verifying_key().to_bytes();
    let replica = |i: &[u8; 32]| Some(S::seal(&kb, i, i));
    let final_top = b.height().saturating_sub(2);
    let answers: Vec<S::Answer> = (e * S::EPOCH..=final_top).filter_map(|x| out(x).filter(|o| S::sampled(o, peer)).and_then(|o| S::Answer::new(&k, &h, &ids, x, &o, &replica))).collect();
    let oldest = b.steps.first().map_or(0, |s| s.height);
    let past = answers.iter().filter(|a| a.height < oldest).count();
    let rec = S::record::State { epochs: vec![S::record::Epoch { epoch: e, attests, binds, holdings: vec![h], answers }] };
    let sp = SC::Params { beacon_code: *blake3::hash(bcode).as_bytes(), beacon: w.beacon, peer: peer.into() };
    let c = || container(scode, sp.encode());
    println!("2  peer {peer}'s record: {} answers ({past} older than the beacon keeps — checked from its outputs log)", rec.epochs[0].answers.len());
    let mut bad = rec.clone();
    if let Some(a) = bad.epochs[0].answers.last_mut() {
        a.height += 1;
        a.sig = k.sign(&a.message()).to_bytes();
    }
    println!("3  a record with one answer moved to another height (re-signed), on Y: {}", put(&w.y, c(), S::codec::encode(&bad)).await?.0);
    println!("4  the record, on Y: {}", put(&w.y, c(), S::codec::encode(&rec)).await?.0);
    Ok(())
}

/// A BATCH-AUCTION POOL, LIVE (TOKEN §5) — an APP over the token's escrow: 1 escrows 300 A with the batch's contract
/// (to sell at ≥ 2), 2 escrows 500 B (to buy at ≤ 3); the batch contract lists both; a verdict before it settles is
/// refused, after it the clearing (250 A for 500 B at 2) is taken; each escrow's release and each payout land as the
/// verdict says, and only so.
async fn pool_mode(w: &W, pcode: &[u8]) -> Result<()> {
    use craftworks_pool_contract as P;
    use P::clear::{Side, PRICE_ONE as ONE};
    println!("— pool (an app over escrow)");
    let (da, db) = (w.def(1, b"PoolA", 17), w.def(2, b"PoolB", 18));
    let (ta, tb) = (w.put_def(&da).await?, w.put_def(&db).await?);
    let h0 = w.now().await?;
    let len = 12;
    let batch = P::Batch { a: ta.into(), b: tb.into(), epoch_len: len, grace: 4, beacon: w.beacon, epoch: h0 / len };
    let (close, settle) = batch.window();
    let pc = || container(pcode, batch.encode());
    let prog: [u8; 32] = *pc().key().id().as_bytes().first_chunk::<32>().unwrap();
    let mut c1 = T::State::default();
    push(&mut c1, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h0);
    push(&mut c1, T::Body::Escrow { program: prog, terms: P::terms(Side::Sell, 2 * ONE), amount: 300, until: 0 }, w.n(7), Some(1), h0);
    println!("1  1 escrows 300 A with batch {} (to sell at ≥ 2; closes {close}, settles {settle}), on X: {} — balance {}", batch.epoch, w.put_chain(&w.x, ta, 1, &c1).await?.0, c1.balance());
    let mut c2 = T::State::default();
    push(&mut c2, T::Body::Genesis { amount: 1000 }, w.n(2), Some(2), h0);
    push(&mut c2, T::Body::Escrow { program: prog, terms: P::terms(Side::Buy, 3 * ONE), amount: 500, until: 0 }, w.n(8), Some(2), h0);
    println!("2  2 escrows 500 B (to buy A at ≤ 3), on Y: {} — balance {}", w.put_chain(&w.y, tb, 2, &c2).await?.0, c2.balance());
    let mut orders = vec![
        P::Order { escrow: (ta.into(), did(1), w.n(7)), amount: 300, side: Side::Sell, limit: 2 * ONE },
        P::Order { escrow: (tb.into(), did(2), w.n(8)), amount: 500, side: Side::Buy, limit: 3 * ONE },
    ];
    orders.sort_by_key(|o| o.escrow);
    let open = P::State { section: T::escrow::Section { escrows: orders.iter().map(|o| (o.escrow, o.amount, T::escrow::terms_hash(&P::terms(o.side, o.limit)))).collect(), ..Default::default() }, orders: orders.clone() };
    println!("3  the batch contract, both orders listed, on X: {}", put(&w.x, pc(), open.encode()).await?.0);
    let settled = P::State { section: P::verdict(&batch, (None, None), &orders), orders: orders.clone() };
    println!("4  its verdict before it settles (now {}), on X: {}", w.now().await?, put(&w.x, pc(), settled.encode()).await?.0);
    let now = w.until(settle).await?;
    println!("5  its verdict at its settle height ({now}), on X: {}", put(&w.x, pc(), settled.encode()).await?.0);
    let v = &settled.section;
    let r1 = v.refund_of(&(ta.into(), did(1), w.n(7))).unwrap();
    let r2 = v.refund_of(&(tb.into(), did(2), w.n(8))).unwrap();
    let (k1, k2) = (P::pay_key(&(ta.into(), did(1), w.n(7))), P::pay_key(&(tb.into(), did(2), w.n(8))));
    let (_, _, g1) = v.pay(&k1).unwrap();
    let (_, _, g2) = v.pay(&k2).unwrap();
    println!("   cleared: 1 back {r1} A, paid {g1} B; 2 back {r2} B, paid {g2} A");
    let wrong = {
        let mut x = c1.clone();
        push(&mut x, T::Body::Unescrow { escrow: w.n(7), refund: r1 + 10, verdict: v.hash() }, w.n(30), None, now);
        x
    };
    println!("6  1's release claiming 10 more than the verdict, on X: {}", w.put_chain(&w.x, ta, 1, &wrong).await?.0);
    let mut s1 = c1.clone();
    push(&mut s1, T::Body::Unescrow { escrow: w.n(7), refund: r1, verdict: v.hash() }, w.n(30), None, now);
    let mut p1 = T::State::default();
    push(&mut p1, T::Body::Payout { program: prog, key: k1, amount: g1, verdict: v.hash(), ev: T::Evidence::default() }, w.n(31), None, now);
    let mut s2 = c2.clone();
    push(&mut s2, T::Body::Unescrow { escrow: w.n(8), refund: r2, verdict: v.hash() }, w.n(32), None, now);
    let mut p2 = T::State::default();
    push(&mut p2, T::Body::Payout { program: prog, key: k2, amount: g2, verdict: v.hash(), ev: T::Evidence::default() }, w.n(33), None, now);
    let r = [w.put_chain(&w.x, ta, 1, &s1).await?.0, w.put_chain(&w.y, tb, 1, &p1).await?.0, w.put_chain(&w.y, tb, 2, &s2).await?.0, w.put_chain(&w.x, ta, 2, &p2).await?.0];
    println!("7  released and paid as the verdict says — 1's release {}, 1's payout {}, 2's release {}, 2's payout {}", r[0], r[1], r[2], r[3]);
    // WHY, if refused: the token contract's own check, run here on the bytes the node holds (the batch's state as read).
    if r.iter().any(|x| x != "ACCEPTED") {
        let pstate = get(&w.x, prog).await?;
        let program = |id: &[u8; 32]| (*id == prog).then(|| pstate.clone()).flatten();
        let x = T::Extra { ops: None, program: Some(&program) };
        let chains = [(ta, 1u8, &s1, &da), (tb, 1, &p1, &db), (tb, 2, &s2, &db), (ta, 2, &p2, &da)];
        for (t, o, st, d) in chains {
            let why = T::check(&w.chain(t, o), d, st, &|_, _| None, &|_| None, Some(&log(o)), &x);
            println!("   local check of {}'s chain of {}: {:?}", o, if t == ta { "A" } else { "B" }, why);
        }
        println!("   the batch's state on X: {} bytes, decodes: {}", pstate.as_ref().map_or(0, |b| b.len()), pstate.as_ref().and_then(|b| P::State::decode(b)).is_some());
    }
    let mut again = p1.clone();
    push(&mut again, T::Body::Payout { program: prog, key: k1, amount: g1, verdict: v.hash(), ev: T::Evidence::default() }, w.n(34), None, w.now().await?);
    println!("8  1's payout a second time, on Y: {}", w.put_chain(&w.y, tb, 1, &again).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let bal = |s: Option<T::State>| s.map_or(0, |s| s.balance());
    println!(
        "   1: {} A, {} B · 2: {} A, {} B",
        bal(w.get_chain(&w.x, ta, 1).await?),
        bal(w.get_chain(&w.y, tb, 1).await?),
        bal(w.get_chain(&w.x, ta, 2).await?),
        bal(w.get_chain(&w.y, tb, 2).await?)
    );
    Ok(())
}

/// A DIRECT SWAP, LIVE (TOKEN §5) — an app over escrow: deal 1, both sides escrow and it CROSSES (1 gets 500 B, 2 gets
/// 300 A); deal 2, only 1 escrows — a refund before the deadline's cutoff is refused, after it taken, 1's 300 back.
async fn deal_mode(w: &W, scode: &[u8]) -> Result<()> {
    use craftworks_swap_contract as S;
    println!("— deal (a direct swap over escrow)");
    let (da, db) = (w.def(1, b"DealA", 19), w.def(2, b"DealB", 20));
    let (ta, tb) = (w.put_def(&da).await?, w.put_def(&db).await?);
    let h0 = w.now().await?;
    let deal = |deadline: u64| S::Deal { x: ta.into(), party_x: did(1), amount_x: 300, y: tb.into(), party_y: did(2), amount_y: 500, deadline, beacon: w.beacon };
    let (d1, d2) = (deal(h0 + 30), deal(h0 + 6));
    let id = |d: &S::Deal| -> [u8; 32] { *container(scode, d.encode()).key().id().as_bytes().first_chunk::<32>().unwrap() };
    let h = T::escrow::terms_hash(S::TERMS);
    let mut c1 = T::State::default();
    push(&mut c1, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h0);
    push(&mut c1, T::Body::Escrow { program: id(&d1), terms: S::TERMS.to_vec(), amount: 300, until: 0 }, w.n(7), Some(1), h0);
    push(&mut c1, T::Body::Escrow { program: id(&d2), terms: S::TERMS.to_vec(), amount: 300, until: 0 }, w.n(9), Some(1), h0);
    println!("1  1 escrows 300 A with deal 1 and 300 A with deal 2, on X: {} — balance {}", w.put_chain(&w.x, ta, 1, &c1).await?.0, c1.balance());
    let mut c2 = T::State::default();
    push(&mut c2, T::Body::Genesis { amount: 1000 }, w.n(2), Some(2), h0);
    push(&mut c2, T::Body::Escrow { program: id(&d1), terms: S::TERMS.to_vec(), amount: 500, until: 0 }, w.n(8), Some(2), h0);
    println!("2  2 escrows 500 B with deal 1, on Y: {} — balance {}", w.put_chain(&w.y, tb, 2, &c2).await?.0, c2.balance());
    let (ex, ey, ex2) = ((ta.into(), did(1), w.n(7)), (tb.into(), did(2), w.n(8)), (ta.into(), did(1), w.n(9)));
    let alone = |r: T::escrow::EscrowRef, a: u128| T::escrow::Section { escrows: vec![(r, a, h)], ..Default::default() };
    let c = || container(scode, d1.encode());
    println!("3  deal 1 with 1's escrow, on X: {}", put(&w.x, c(), alone(ex, 300).encode()).await?.0);
    println!("4  … and 2's (the deal crosses), on X: {}", put(&w.x, c(), alone(ey, 500).encode()).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let held = get(&w.x, id(&d1)).await?.and_then(|b| T::escrow::Section::decode_prefix(&b).map(|x| x.0)).context("deal 1")?;
    println!("   deal 1 holds: {} escrows, crossed {}", held.escrows.len(), held == S::crossed(&d1, (None, None), &ex, &ey));
    let now = w.now().await?;
    let mut s1 = c1.clone();
    push(&mut s1, T::Body::Unescrow { escrow: w.n(7), refund: 0, verdict: held.hash() }, w.n(30), None, now);
    let mut p1 = T::State::default();
    push(&mut p1, T::Body::Payout { program: id(&d1), key: S::pay_key(&ex), amount: 500, verdict: held.hash(), ev: T::Evidence::default() }, w.n(31), None, now);
    let mut s2 = c2.clone();
    push(&mut s2, T::Body::Unescrow { escrow: w.n(8), refund: 0, verdict: held.hash() }, w.n(32), None, now);
    let mut p2 = T::State::default();
    push(&mut p2, T::Body::Payout { program: id(&d1), key: S::pay_key(&ey), amount: 300, verdict: held.hash(), ev: T::Evidence::default() }, w.n(33), None, now);
    let r = [w.put_chain(&w.x, ta, 1, &s1).await?.0, w.put_chain(&w.y, tb, 1, &p1).await?.0, w.put_chain(&w.y, tb, 2, &s2).await?.0, w.put_chain(&w.x, ta, 2, &p2).await?.0];
    println!("5  crossed — 1's release {}, 1 paid 500 B {}, 2's release {}, 2 paid 300 A {}", r[0], r[1], r[2], r[3]);
    // Deal 2: 1 alone.
    let c = || container(scode, d2.encode());
    println!("6  deal 2 with 1's escrow alone, on X: {}", put(&w.x, c(), alone(ex2, 300).encode()).await?.0);
    let refund = S::refunded(&d2, &ex2, true);
    println!("7  its refund before the cutoff ({}; now {}), on X: {}", d2.deadline + S::GRACE, w.now().await?, put(&w.x, c(), refund.encode()).await?.0);
    let at = w.until(d2.deadline + S::GRACE).await?;
    println!("8  its refund at the cutoff ({at}), on X: {}", put(&w.x, c(), refund.encode()).await?.0);
    let mut back = s1.clone();
    push(&mut back, T::Body::Unescrow { escrow: w.n(9), refund: 300, verdict: refund.hash() }, w.n(34), None, w.now().await?);
    println!("9  1's 300 A released from deal 2, on X: {}", w.put_chain(&w.x, ta, 1, &back).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let bal = |s: Option<T::State>| s.map_or(0, |s| s.balance());
    println!(
        "   1: {} A, {} B · 2: {} A, {} B",
        bal(w.get_chain(&w.x, ta, 1).await?),
        bal(w.get_chain(&w.y, tb, 1).await?),
        bal(w.get_chain(&w.x, ta, 2).await?),
        bal(w.get_chain(&w.y, tb, 2).await?)
    );
    Ok(())
}

/// EDITIONS (TOKEN §1c), LIVE: two editions of one Craftworks item (a Pixel Cats picture) — GOLD (numbered, cap
/// 3, 10% to its creator) and SILVER (plain copies, cap 5, 5%), and an OPEN numbered edition whose copy #1025 makes
/// its second record. Mints are kept in records of 1,024 numbers, made as copies are minted; a mint past a cap is
/// refused, one onto taken numbers kept but never admitted. A Gold copy resold through a deal pays Gold's own royalty.
async fn editions(w: &W, scode: &[u8]) -> Result<()> {
    use craftworks_swap_contract as S;
    println!("— editions (Pixel Cats: Gold numbered of 3, Silver plain of 5)");
    let (m, t, c) = (1u8, 2u8, 7u8);
    let item = *blake3::hash(b"a Craftworks item: the cat picture").as_bytes();
    let ed = |name: &[u8], k: u8, ids: T::Ids, cap: u128, bps: u16| T::Definition { ids, cap, decimals: 0, min_send: 1, genesis_amount: 0, mint: Some(one(60)), royalty: Some((bps, did(c))), item, ..w.def(c, name, k) };
    let (gd, sd) = (ed(b"Gold", 21, T::Ids::Numbered, 3, 1000), ed(b"Silver", 22, T::Ids::None, 5, 500));
    let (g, sv) = (w.put_def(&gd).await?, w.put_def(&sd).await?);
    let h = w.now().await?;
    let mint = |d: &[u8; 32], to: u8, first: u128, amount: u128| {
        let mut x = T::supply::Mint { to: did(to), item: [0; 32], amount, nonce: first.to_be_bytes(), stamp: h, sigs: vec![], paid: None };
        x.sigs = vec![(0, key(60).sign(&x.message(d)).to_bytes())];
        x
    };
    let record = |d: [u8; 32], k: u64| container(&w.code, T::supply::Params { code_hash: w.ch, definition: d, beacon: w.beacon, epoch: k }.encode());
    let gs = T::supply::Shard { mints: vec![mint(&g, m, 1, 2), mint(&g, t, 3, 1)] };
    println!("1  Gold #1–2 minted to M, #3 to T (record 0), on X: {}", put(&w.x, record(g, 0), gs.encode()).await?.0);
    let past = T::supply::merge(&gs, &T::supply::Shard { mints: vec![mint(&g, 8, 4, 1)] }).unwrap();
    println!("2  … and #4 to 8 (past the cap of 3), on X: {}", put(&w.x, record(g, 0), past.encode()).await?.0);
    let taken = T::supply::merge(&gs, &T::supply::Shard { mints: vec![mint(&g, 8, 2, 1)] }).unwrap();
    println!("2b … and #2 again to 8 (taken), on X: {} — admitted {} of {} mints", put(&w.x, record(g, 0), taken.encode()).await?.0, T::supply::admitted(&gd, &taken).len(), taken.mints.len());
    let ss = T::supply::Shard { mints: vec![mint(&sv, m, 1, 4)] };
    println!("3  Silver: 4 to M (numbers 1–4, unseen), on X: {}", put(&w.x, record(sv, 0), ss.encode()).await?.0);
    let over = T::supply::merge(&ss, &T::supply::Shard { mints: vec![mint(&sv, t, 5, 2)] }).unwrap();
    println!("3b … and 2 to T (5–6, past 5), on X: {}", put(&w.x, record(sv, 0), over.encode()).await?.0);
    let od = ed(b"Open", 24, T::Ids::Numbered, 0, 1000);
    let op = w.put_def(&od).await?;
    println!("3c an OPEN numbered edition: #1024 in record 0 {}, #1025 in record 1 {}, #1025 put in record 0 {}",
        put(&w.x, record(op, 0), T::supply::Shard { mints: vec![mint(&op, m, 1024, 1)] }.encode()).await?.0,
        put(&w.x, record(op, 1), T::supply::Shard { mints: vec![mint(&op, t, 1025, 1)] }.encode()).await?.0,
        put(&w.y, record(op, 0), T::supply::Shard { mints: vec![mint(&op, t, 1025, 1)] }.encode()).await?.0);
    let r = w.now().await?;
    let copy = |o: u8, n: u128| T::ChainParams { item: T::numbered_item(&g, n), ..w.chain(g, o) };
    let credit = |n: [u8; 16], amount: u128, carried: T::supply::Mint| {
        let mut s = T::State::default();
        push(&mut s, T::Body::CappedMint { nonce: n, amount, minted: h, record: T::supply::record_of(&gd, u128::from_be_bytes(n)).unwrap_or(0), mint: carried, ev: T::Evidence::default() }, n, None, r);
        s
    };
    println!("4  M's Gold #1 credited, on Y: {}", put(&w.y, container(&w.code, copy(m, 1).encode()), credit(1u128.to_be_bytes(), 1, mint(&g, m, 1, 2)).encode()).await?.0);
    println!("5  T's Gold #3 credited, on Y: {}", put(&w.y, container(&w.code, copy(t, 3).encode()), credit(3u128.to_be_bytes(), 1, mint(&g, t, 3, 1)).encode()).await?.0);
    println!("6  M claims T's #3, on Y: {}", put(&w.y, container(&w.code, copy(m, 3).encode()), credit(3u128.to_be_bytes(), 1, mint(&g, t, 3, 1)).encode()).await?.0);
    println!("7  8's #4 (past the cap), on Y: {}", put(&w.y, container(&w.code, copy(8, 4).encode()), credit(4u128.to_be_bytes(), 1, mint(&g, 8, 4, 1)).encode()).await?.0);
    println!("8  M's 4 Silver, on Y: {}", w.put_chain(&w.y, sv, m, &credit(1u128.to_be_bytes(), 4, mint(&sv, m, 1, 4))).await?.0);
    println!("9  T's 2 Silver (past the cap), on Y: {}", w.put_chain(&w.y, sv, t, &credit(5u128.to_be_bytes(), 2, mint(&sv, t, 5, 2))).await?.0);
    let oc = T::ChainParams { item: T::numbered_item(&op, 1025), ..w.chain(op, t) };
    println!("9b T's Open #1025 credited from record 1, on Y: {}", put(&w.y, container(&w.code, oc.encode()), credit(1025u128.to_be_bytes(), 1, mint(&op, t, 1025, 1)).encode()).await?.0);
    // M resells Gold #1 to T for 100 B through a deal: Gold's 10% to the creator.
    let tb = w.put_def(&w.def(t, b"PayEd", 23)).await?;
    let it = T::Tok { definition: g, item: T::numbered_item(&g, 1) };
    let r = w.now().await?;
    let deal = S::Deal { x: it, party_x: did(m), amount_x: 1, y: tb.into(), party_y: did(t), amount_y: 100, deadline: r + 30, beacon: w.beacon };
    let prog = *container(scode, deal.encode()).key().id().as_bytes().first_chunk::<32>().unwrap();
    let mut mc = credit(1u128.to_be_bytes(), 1, mint(&g, m, 1, 2));
    push(&mut mc, T::Body::Escrow { program: prog, terms: S::TERMS.to_vec(), amount: 1, until: 0 }, w.n(7), Some(m), r);
    println!("10 M escrows Gold #1 with a deal for 100 B, on Y: {}", put(&w.y, container(&w.code, copy(m, 1).encode()), mc.encode()).await?.0);
    let mut tbc = T::State::default();
    push(&mut tbc, T::Body::Genesis { amount: 1000 }, w.n(1), Some(t), r);
    push(&mut tbc, T::Body::Escrow { program: prog, terms: S::TERMS.to_vec(), amount: 100, until: 0 }, w.n(8), Some(t), r);
    println!("11 T escrows 100 B, on X: {}", w.put_chain(&w.x, tb, t, &tbc).await?.0);
    let (ex, ey) = ((it, did(m), w.n(7)), (tb.into(), did(t), w.n(8)));
    let hs = T::escrow::terms_hash(S::TERMS);
    let alone = |e: T::escrow::EscrowRef, a: u128| T::escrow::Section { escrows: vec![(e, a, hs)], ..Default::default() };
    put(&w.x, container(scode, deal.encode()), alone(ex, 1).encode()).await?;
    println!("12 the deal with both escrows (it crosses, reading Gold's 10%), on X: {}", put(&w.x, container(scode, deal.encode()), alone(ey, 100).encode()).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let held = get(&w.x, prog).await?.and_then(|b| T::escrow::Section::decode_prefix(&b).map(|x| x.0)).context("the deal")?;
    println!("   the deal pays: M {:?} B, the creator {:?} B, T Gold #1 {:?}", held.pay(&S::pay_key(&ex)).map(|p| p.2), held.pay(&S::fee_key(&ex)).map(|p| p.2), held.pay(&S::pay_key(&ey)).map(|p| p.2));
    let now = w.now().await?;
    let mut ti = T::State::default();
    push(&mut ti, T::Body::Payout { program: prog, key: S::pay_key(&ey), amount: 1, verdict: held.hash(), ev: T::Evidence::default() }, w.n(30), None, now);
    println!("13 T paid Gold #1, on Y: {}", put(&w.y, container(&w.code, copy(t, 1).encode()), ti.encode()).await?.0);
    Ok(())
}

/// HOLDERS' ROYALTY (TOKEN §1c), LIVE: edition H — numbered, cap 4, 10% to its HOLDERS. Copies #1 M, #2 T, #3 holder
/// 8. M sells #1 to T for 100 B through a deal: 90 B to M, the 10 B fee to the holders — each OTHER copy claims
/// 10 / 3 = 3 B, once; the sold copy none.
async fn holders(w: &W, scode: &[u8]) -> Result<()> {
    use craftworks_swap_contract as S;
    println!("— holders (an edition whose royalty its holders share)");
    let (m, t, h8) = (1u8, 2u8, 8u8);
    let hd = T::Definition { ids: T::Ids::Numbered, cap: 4, decimals: 0, min_send: 1, genesis_amount: 0, mint: Some(one(60)), royalty: Some((1000, T::HOLDERS)), ..w.def(m, b"H", 26) };
    let e = w.put_def(&hd).await?;
    let h = w.now().await?;
    let mint = |to: u8, n: u128| {
        let mut x = T::supply::Mint { to: did(to), item: [0; 32], amount: 1, nonce: n.to_be_bytes(), stamp: h, sigs: vec![], paid: None };
        x.sigs = vec![(0, key(60).sign(&x.message(&e)).to_bytes())];
        x
    };
    let rec = T::supply::Shard { mints: vec![mint(m, 1), mint(t, 2), mint(h8, 3)] };
    println!("1  H #1 to M, #2 to T, #3 to 8, on X: {}", put(&w.x, container(&w.code, T::supply::Params { code_hash: w.ch, definition: e, beacon: w.beacon, epoch: 0 }.encode()), rec.encode()).await?.0);
    let r = w.now().await?;
    let copy = |o: u8, n: u128| T::ChainParams { item: T::numbered_item(&e, n), ..w.chain(e, o) };
    let credit = |o: u8, n: u128| {
        let mut s = T::State::default();
        push(&mut s, T::Body::CappedMint { nonce: n.to_be_bytes(), amount: 1, minted: h, record: 0, mint: mint(o, n), ev: T::Evidence::default() }, n.to_be_bytes(), None, r);
        s
    };
    for (o, n) in [(m, 1u128), (t, 2), (h8, 3)] {
        println!("2  {o}'s copy #{n} credited, on Y: {}", put(&w.y, container(&w.code, copy(o, n).encode()), credit(o, n).encode()).await?.0);
    }
    let tb = w.put_def(&w.def(t, b"PayH", 27)).await?;
    let it = T::Tok { definition: e, item: T::numbered_item(&e, 1) };
    let deal = S::Deal { x: it, party_x: did(m), amount_x: 1, y: tb.into(), party_y: did(t), amount_y: 100, deadline: r + 30, beacon: w.beacon };
    let prog = *container(scode, deal.encode()).key().id().as_bytes().first_chunk::<32>().unwrap();
    let mut mc = credit(m, 1);
    push(&mut mc, T::Body::Escrow { program: prog, terms: S::TERMS.to_vec(), amount: 1, until: 0 }, w.n(7), Some(m), r);
    println!("3  M escrows #1 with a deal for 100 B, on Y: {}", put(&w.y, container(&w.code, copy(m, 1).encode()), mc.encode()).await?.0);
    let mut tbc = T::State::default();
    push(&mut tbc, T::Body::Genesis { amount: 1000 }, w.n(1), Some(t), r);
    push(&mut tbc, T::Body::Escrow { program: prog, terms: S::TERMS.to_vec(), amount: 100, until: 0 }, w.n(8), Some(t), r);
    println!("4  T escrows 100 B, on X: {}", w.put_chain(&w.x, tb, t, &tbc).await?.0);
    let (ex, ey) = ((it, did(m), w.n(7)), (tb.into(), did(t), w.n(8)));
    let hs = T::escrow::terms_hash(S::TERMS);
    let alone = |x: T::escrow::EscrowRef, a: u128| T::escrow::Section { escrows: vec![(x, a, hs)], ..Default::default() };
    put(&w.x, container(scode, deal.encode()), alone(ex, 1).encode()).await?;
    println!("5  the deal with both (it crosses, the fee to the holders), on X: {}", put(&w.x, container(scode, deal.encode()), alone(ey, 100).encode()).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let held = get(&w.x, prog).await?.and_then(|b| T::escrow::Section::decode_prefix(&b).map(|x| x.0)).context("the deal")?;
    let fee = held.pay(&S::fee_key(&ex));
    println!("   the deal pays: M {:?} B, the holders {:?}", held.pay(&S::pay_key(&ex)).map(|p| p.2), fee.map(|p| (p.1 == T::HOLDERS, p.2)));
    let now = w.now().await?;
    let share = |n: u128, amount: u128| T::Body::HolderPayout { program: prog, sold: ex, copy: n, at: r, amount, verdict: held.hash(), ev: T::Evidence::default() };
    let mut tc = tbc.clone();
    push(&mut tc, share(2, 3), w.n(30), None, now);
    println!("6  T's share for copy #2 (3 B), on X: {}", w.put_chain(&w.x, tb, t, &tc).await?.0);
    let mut c8 = T::State::default();
    push(&mut c8, share(3, 3), w.n(31), None, now);
    println!("7  8's share for copy #3 (3 B), on X: {}", w.put_chain(&w.x, tb, h8, &c8).await?.0);
    let mut again = c8.clone();
    push(&mut again, share(3, 3), w.n(32), None, now);
    println!("8  8's share again, on Y: {}", w.put_chain(&w.y, tb, h8, &again).await?.0);
    let mut mm = T::State::default();
    push(&mut mm, share(1, 3), w.n(33), None, now);
    println!("9  M's share for the copy it sold (#1), on X: {}", w.put_chain(&w.x, tb, m, &mm).await?.0);
    let mut nine = T::State::default();
    push(&mut nine, share(3, 3), w.n(34), None, now);
    println!("10 9 claims copy #3's share (not its copy), on X: {}", w.put_chain(&w.x, tb, 9, &nine).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let bal = |s: Option<T::State>| s.map_or(0, |s| s.balance());
    println!("   B: T {} (900 + 3) · 8 {} · M {} · 9 {}", bal(w.get_chain(&w.x, tb, t).await?), bal(w.get_chain(&w.x, tb, h8).await?), bal(w.get_chain(&w.x, tb, m).await?), bal(w.get_chain(&w.x, tb, 9).await?));
    Ok(())
}

/// ESCROW RECLAIM (TOKEN §4), LIVE: 1 escrows 300 A to a program that does not exist (a wrong id), reclaimable from
/// h+4 — refused before its time, taken after; and 300 A to a real deal that took it — never reclaimed.
async fn reclaim(w: &W, scode: &[u8]) -> Result<()> {
    use craftworks_swap_contract as S;
    println!("— reclaim (an escrow its program never took)");
    let ta = w.put_def(&w.def(1, b"RecA", 28)).await?;
    let tb = w.put_def(&w.def(2, b"RecB", 29)).await?;
    let h0 = w.now().await?;
    let deal = S::Deal { x: ta.into(), party_x: did(1), amount_x: 300, y: tb.into(), party_y: did(2), amount_y: 500, deadline: h0 + 30, beacon: w.beacon };
    let prog = *container(scode, deal.encode()).key().id().as_bytes().first_chunk::<32>().unwrap();
    let nowhere = *blake3::hash(b"no such program").as_bytes();
    let mut c = T::State::default();
    push(&mut c, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h0);
    push(&mut c, T::Body::Escrow { program: nowhere, terms: vec![], amount: 300, until: h0 + 4 }, w.n(7), Some(1), h0);
    push(&mut c, T::Body::Escrow { program: prog, terms: S::TERMS.to_vec(), amount: 300, until: h0 + 4 }, w.n(8), Some(1), h0);
    println!("1  1 escrows 300 A to no program and 300 A to a deal (both reclaimable from {}), on X: {} — balance {}", h0 + 4, w.put_chain(&w.x, ta, 1, &c).await?.0, c.balance());
    let hs = T::escrow::terms_hash(S::TERMS);
    let listed = T::escrow::Section { escrows: vec![((ta.into(), did(1), w.n(8)), 300, hs)], ..Default::default() };
    println!("2  the deal takes its escrow, on X: {}", put(&w.x, container(scode, deal.encode()), listed.encode()).await?.0);
    let early = w.now().await?;
    let mut e1 = c.clone();
    push(&mut e1, T::Body::Reclaim { escrow: w.n(7) }, w.n(30), None, early);
    println!("3  reclaim before its time (now {early}), on X: {}", w.put_chain(&w.x, ta, 1, &e1).await?.0);
    let at = w.until(h0 + 4).await?;
    let mut r1 = c.clone();
    push(&mut r1, T::Body::Reclaim { escrow: w.n(7) }, w.n(31), None, at);
    println!("4  reclaim from no program at its time ({at}), on X: {}", w.put_chain(&w.x, ta, 1, &r1).await?.0);
    let mut r2 = r1.clone();
    push(&mut r2, T::Body::Reclaim { escrow: w.n(8) }, w.n(32), None, at);
    println!("5  reclaim the escrow the deal took — the chain takes it (it never reads a program), on X: {}", w.put_chain(&w.x, ta, 1, &r2).await?.0);
    // ... but no WITNESS signs it: the deal (as a witness reads it) took that escrow; the one to no program, it signs.
    // (Read from the network, as a witness reads them.)
    let (deal_b, none_b) = (get(&w.y, prog).await.ok().flatten(), get(&w.y, nowhere).await.ok().flatten());
    let fetched = none_b.is_some();
    let took = |n: [u8; 16]| if n == w.n(8) { deal_b.clone() } else { none_b.clone() }.and_then(|b| T::escrow::Section::decode_prefix(&b).map(|x| x.0)).is_some_and(|sec| sec.escrows.iter().any(|(r, ..)| *r == (ta.into(), did(1), n)));
    println!("   the program for the first: fetched {fetched} (none) — a witness signs its reclaim: {}; the deal's: {}", !took(w.n(7)), !took(w.n(8)));
    tokio::time::sleep(Duration::from_secs(2)).await;
    println!("   1's A: {}", w.get_chain(&w.x, ta, 1).await?.map_or(0, |s| s.balance()));
    Ok(())
}

/// NON-TRANSFERABLE (TOKEN §1d), LIVE: a soulbound BADGE minted to 2 — credited, never sent, escrowed or vested; burned.
async fn soulbound(w: &W) -> Result<()> {
    println!("— soulbound (a token that never leaves its chain)");
    let d = T::Definition { soulbound: true, mint: Some(one(60)), genesis_amount: 0, ..w.def(1, b"Badge", 30) };
    let t = w.put_def(&d).await?;
    let h = w.now().await?;
    let mut m = T::supply::Mint { to: did(2), item: [0; 32], amount: 100, nonce: w.n(1), stamp: h, sigs: vec![], paid: None };
    m.sigs = vec![(0, key(60).sign(&m.message(&t)).to_bytes())];
    let sp = T::supply::Params { code_hash: w.ch, definition: t, beacon: w.beacon, epoch: T::supply::epoch(h) };
    println!("1  100 badge minted to 2, on X: {}", put(&w.x, container(&w.code, sp.encode()), T::supply::Shard { mints: vec![m.clone()] }.encode()).await?.0);
    let r = w.now().await?;
    let mut c = T::State::default();
    push(&mut c, T::Body::Mint { nonce: w.n(1), amount: 100, minted: h, mint: m.clone(), ev: T::Evidence::default() }, w.n(1), None, r);
    println!("2  2 credits it, on Y: {}", w.put_chain(&w.y, t, 2, &c).await?.0);
    for (i, (what, b)) in [
        ("sends 10 to 3", T::Body::Send { to: did(3), amount: 10 }),
        ("allows 3 to spend 10", T::Body::Allow { spender: pk(3), limit: 10 }),
        ("escrows 10", T::Body::Escrow { program: [7; 32], terms: vec![], amount: 10, until: 0 }),
    ]
    .into_iter()
    .enumerate()
    {
        let mut x = c.clone();
        push(&mut x, b, w.n(10 + i as u8), Some(2), r);
        println!("3  2 {what}, on Y: {}", w.put_chain(&w.y, t, 2, &x).await?.0);
    }
    let mut x = c.clone();
    push(&mut x, T::Body::Burn { amount: 10 }, w.n(20), Some(2), r);
    println!("4  2 burns 10, on Y: {} — holds {}", w.put_chain(&w.y, t, 2, &x).await?.0, x.balance());
    Ok(())
}

/// TRANSFER FEES (TOKEN §1e), LIVE: token F — 1% per transfer, at most 5, to collector 9. 1 sends 300 to 2 paying 3 on
/// top (an entry without it refused); 2 receives the whole 300; 9 claims the 3 once; 8 (not the collector) cannot.
async fn fees(w: &W) -> Result<()> {
    println!("— fees (a transfer fee on top, claimed by its collector)");
    let d = T::Definition { transfer_fee: Some((100, 5, did(9))), ..w.def(1, b"Fee", 31) };
    let t = w.put_def(&d).await?;
    let h = w.now().await?;
    // An entry whose carried balance pays `fee` on top (signed again after).
    let with_fee = |s: &mut T::State, body: T::Body, n: [u8; 16], by: Option<u8>, stamp: u64, fee: u128| {
        push(s, body, n, by, stamp);
        let mut e = s.entries.pop().unwrap();
        e.balance -= fee;
        let m = e.message();
        e.auth = match by {
            Some(k) => T::Auth::Sig(key(k).sign(&m).to_bytes()),
            None => T::Auth::Work((0u64..).find(|x| T::work_ok(&m, *x)).unwrap()),
        };
        s.entries.push(e);
    };
    let mut a = T::State::default();
    push(&mut a, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h);
    let mut bad = a.clone();
    with_fee(&mut bad, T::Body::Send { to: did(2), amount: 300 }, w.n(30), Some(1), h, 0);
    println!("1  1 sends 300 to 2 WITHOUT the fee, on X: {}", w.put_chain(&w.x, t, 1, &bad).await?.0);
    with_fee(&mut a, T::Body::Send { to: did(2), amount: 300 }, w.n(30), Some(1), h, 3);
    println!("2  1 sends 300 to 2 paying 3 on top, on X: {} — 1 holds {}", w.put_chain(&w.x, t, 1, &a).await?.0, a.balance());
    let r = w.until(h + 3).await?;
    let mut b = T::State::default();
    push(&mut b, T::Body::Receive { from: did(1), nonce: w.n(30), amount: 300, cited: h, ev: T::Evidence::default() }, w.n(30), None, r);
    println!("3  2 receives the whole 300, on Y: {}", w.put_chain(&w.y, t, 2, &b).await?.0);
    let claim = |o: u8, k: u8, amount: u128| {
        let mut c = T::State::default();
        push(&mut c, T::Body::FeeClaim { from: did(1), nonce: w.n(30), gross: 300, amount, cited: h, ev: T::Evidence::default() }, w.n(k), None, r);
        (o, c)
    };
    let (o, c) = claim(9, 40, 2);
    println!("4  9 claims 2 (the fee is 3), on Y: {}", w.put_chain(&w.y, t, o, &c).await?.0);
    let (o, c) = claim(8, 41, 3);
    println!("5  8 (not the collector) claims the 3, on Y: {}", w.put_chain(&w.y, t, o, &c).await?.0);
    let (o, mut c) = claim(9, 42, 3);
    println!("6  9 claims the 3, on Y: {}", w.put_chain(&w.y, t, o, &c).await?.0);
    push(&mut c, T::Body::FeeClaim { from: did(1), nonce: w.n(30), gross: 300, amount: 3, cited: h, ev: T::Evidence::default() }, w.n(43), None, r);
    println!("7  … and again, on Y: {}", w.put_chain(&w.y, t, 9, &c).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let bal = |s: Option<T::State>| s.map_or(0, |s| s.balance());
    println!("   1 {} · 2 {} · 9 {} (= 1000)", bal(w.get_chain(&w.x, t, 1).await?), bal(w.get_chain(&w.y, t, 2).await?), bal(w.get_chain(&w.y, t, 9).await?));
    Ok(())
}

/// PAUSE (TOKEN §2), LIVE: the freeze authority (key 61) freezes EVERY account of a token with one word: no debit of
/// anyone while paused — a credit still taken — then unpaused by the next word.
async fn pause(w: &W) -> Result<()> {
    println!("— pause (the whole token frozen at once)");
    let d = T::Definition { freeze: Some(one(61)), ..w.def(1, b"Pause", 32) };
    let t = w.put_def(&d).await?;
    let fp = T::freeze::Params { code_hash: w.ch, definition: t, beacon: w.beacon };
    let word = |frozen: bool, seq: u32, stamp: u64| {
        let mut r = T::freeze::Record { owner: T::freeze::EVERYONE, frozen, seq, stamp, sigs: vec![] };
        r.sigs = vec![(0, key(61).sign(&r.message(&t)).to_bytes())];
        T::freeze::List { records: vec![r] }
    };
    let h = w.now().await?;
    let mut a = T::State::default();
    push(&mut a, T::Body::Genesis { amount: 1000 }, w.n(1), Some(1), h);
    push(&mut a, T::Body::Send { to: did(2), amount: 300 }, w.n(2), Some(1), h);
    println!("1  1: genesis, sends 300 to 2, on Y: {}", w.put_chain(&w.y, t, 1, &a).await?.0);
    let r = w.now().await?;
    println!("2  the token PAUSED (one word, key 61), on Y: {}", put(&w.y, container(&w.code, fp.encode()), word(true, 0, r).encode()).await?.0);
    let r = w.until(h + 3).await?;
    let mut x = a.clone();
    push(&mut x, T::Body::Send { to: did(3), amount: 100 }, w.n(3), Some(1), r);
    println!("3  1 sends 100 to 3 (paused), on Y: {}", w.put_chain(&w.y, t, 1, &x).await?.0);
    let mut b = T::State::default();
    push(&mut b, T::Body::Receive { from: did(1), nonce: w.n(2), amount: 300, cited: h, ev: T::Evidence::default() }, w.n(2), None, r);
    println!("4  2 receives its 300 (a credit, paused), on Y: {}", w.put_chain(&w.y, t, 2, &b).await?.0);
    let mut y = b.clone();
    push(&mut y, T::Body::Send { to: did(3), amount: 50 }, w.n(4), Some(2), r);
    println!("5  2 sends 50 to 3 (paused), on Y: {}", w.put_chain(&w.y, t, 2, &y).await?.0);
    let r = w.now().await?;
    println!("6  UNPAUSED (the next word), on Y: {}", put(&w.y, container(&w.code, fp.encode()), word(false, 1, r).encode()).await?.0);
    let mut z = a.clone();
    push(&mut z, T::Body::Send { to: did(3), amount: 100 }, w.n(5), Some(1), w.now().await?);
    println!("7  1 sends 100 to 3, on Y: {}", w.put_chain(&w.y, t, 1, &z).await?.0);
    Ok(())
}

/// Person `n`'s DEVICE for the wallet core: its node key `key(n)`, credentialed by its account's owner key.
struct Dev(u8);
impl craftworks_wallet::Device for Dev {
    fn public(&self) -> [u8; 32] {
        pk(self.0)
    }
    fn sign(&self, m: &[u8]) -> [u8; 64] {
        key(self.0).sign(m).to_bytes()
    }
    fn credential(&self) -> Vec<u8> {
        craftworks_identity::credential(&did(self.0), &pk(self.0), &[], &owner_seed(self.0))
    }
    fn key_seq(&self) -> u32 {
        0
    }
}

/// THE WALLET CORE (TOKEN §7c), LIVE: every entry here built and signed by `craftworks_wallet` — no hand-made entry.
/// 1 starts token W and pays 2 300 and 3 100; 2 collects its receive once final and pays 3 50; 1 CUTS its chain; 3
/// collects both its payments — 1's from 1's history (the cut's blocks), with its proof. Each through the network.
async fn wallet_mode(w: &W) -> Result<()> {
    use craftworks_wallet::{Held, Sources, Wallet};
    println!("— wallet (the wallet core, end to end)");
    let d = w.def(1, b"Wal", 41);
    let t = w.put_def(&d).await?;
    let held = |o: u8, s: T::State| Held { params: w.chain(t, o), definition: d.clone(), state: s, blocks: Default::default() };
    let (l1, l2, l3) = (log(1), log(2), log(3));
    let (w1, w2, w3) = (Wallet { device: &Dev(1), keys: &l1 }, Wallet { device: &Dev(2), keys: &l2 }, Wallet { device: &Dev(3), keys: &l3 });
    let why = |e: &str| anyhow::anyhow!("{e}");
    let mut a = held(1, T::State::default());
    let h0 = w.now().await?;
    a.state.entries.extend(w1.genesis(&a, h0).map_err(why)?);
    a.state.entries.extend(w1.debit(&a, T::Body::Send { to: did(2), amount: 300 }, h0).map_err(why)?);
    a.state.entries.extend(w1.debit(&a, T::Body::Send { to: did(3), amount: 100 }, h0).map_err(why)?);
    println!("1  1: genesis, 300 to 2, 100 to 3 (its wallet), on X: {} — {}", w.put_chain(&w.x, t, 1, &a.state).await?.0, a.state.balance());
    let none = T::cut::Blocks::new();
    let chains1 = [(did(1), a.state.clone())];
    let early = w2.collect(&held(2, T::State::default()), &Sources { chains: &chains1, blocks: &none, programs: &[], shards: &[] }, w.now().await?).map_err(why)?;
    println!("2  2 collects before the margin: {} credit(s)", early.len());
    let now = w.until(h0 + 3).await?;
    let mut b = held(2, T::State::default());
    b.state.entries.extend(w2.collect(&b, &Sources { chains: &chains1, blocks: &none, programs: &[], shards: &[] }, now).map_err(why)?);
    println!("3  2 collects after it (now {now}): its receive, on Y: {} — {}", w.put_chain(&w.y, t, 2, &b.state).await?.0, b.state.balance());
    b.state.entries.extend(w2.debit(&b, T::Body::Send { to: did(3), amount: 50 }, now).map_err(why)?);
    println!("4  2 pays 3 50, on Y: {} — {}", w.put_chain(&w.y, t, 2, &b.state).await?.0, b.state.balance());
    let (cut, blocks) = w1.cut(&a, now).map_err(why)?;
    a.state.entries.extend(cut);
    a.state = T::prune(&a.state);
    a.blocks = blocks;
    println!("5  1 CUTS its chain (its sends now behind the cut), on X: {} — live {} entr(ies)", w.put_chain(&w.x, t, 1, &a.state).await?.0, a.state.entries.len());
    let now = w.until(now + 3).await?;
    let chains = [(did(1), a.state.clone()), (did(2), b.state.clone())];
    let mut c = held(3, T::State::default());
    let got = w3.collect(&c, &Sources { chains: &chains, blocks: &a.blocks, programs: &[], shards: &[] }, now).map_err(why)?;
    let shown = got.iter().filter(|e| matches!(&e.body, T::Body::Receive { ev, .. } if !ev.theirs.is_empty())).count();
    c.state.entries.extend(got);
    println!("6  3 collects both — 1's from its history ({shown} with a proof), on Y: {} — {}", w.put_chain(&w.y, t, 3, &c.state).await?.0, c.state.balance());
    let again = w3.collect(&c, &Sources { chains: &chains, blocks: &a.blocks, programs: &[], shards: &[] }, now).map_err(why)?;
    println!("7  3 collects again: {} credit(s)", again.len());
    tokio::time::sleep(Duration::from_secs(2)).await;
    let bal = |s: Option<T::State>| s.map_or(0, |s| s.balance());
    println!("   on the network: 1 {} · 2 {} · 3 {} (= 1000)", bal(w.get_chain(&w.x, t, 1).await?), bal(w.get_chain(&w.y, t, 2).await?), bal(w.get_chain(&w.y, t, 3).await?));
    Ok(())
}

/// SOLD ON DEMAND (TOKEN §1c, owner 10-10: "allow mint without owner for on demand"), LIVE: edition S — numbered, cap
/// 10, 10 P a copy to 9, NO mint authority. 1 buys with its wallet: pays 10 P (its nonce tagged for record 0), then
/// adds the paid mint to the record — held on the payment alone; its chain of the copy collects it. A mint citing no
/// payment, or one payment twice, is refused.
async fn ondemand(w: &W) -> Result<()> {
    use craftworks_wallet::{Held, Sources, Wallet};
    println!("— ondemand (an edition anyone buys, its owner offline)");
    let why = |e: &str| anyhow::anyhow!("{e}");
    let pd = w.def(1, b"P", 43);
    let p = w.put_def(&pd).await?;
    let sd = T::Definition { ids: T::Ids::Numbered, cap: 10, decimals: 0, min_send: 1, genesis_amount: 0, sale: Some((p.into(), 10, did(9))), ..w.def(9, b"S", 44) };
    let s = w.put_def(&sd).await?;
    let l1 = log(1);
    let w1 = Wallet { device: &Dev(1), keys: &l1 };
    let mut pay = Held { params: w.chain(p, 1), definition: pd.clone(), state: T::State::default(), blocks: Default::default() };
    let h0 = w.now().await?;
    pay.state.entries.extend(w1.genesis(&pay, h0).map_err(why)?);
    let empty = T::supply::Shard::default();
    let (es, mint) = w1.buy(&pay, &sd, &s, (0, &empty), 1, h0).map_err(why)?;
    pay.state.entries.extend(es);
    println!("1  1 pays 10 P to 9 for a copy, on X: {} — {}", w.put_chain(&w.x, p, 1, &pay.state).await?.0, pay.state.balance());
    tokio::time::sleep(Duration::from_secs(2)).await;
    let record = || container(&w.code, T::supply::Params { code_hash: w.ch, definition: s, beacon: w.beacon, epoch: 0 }.encode());
    let rec = T::supply::Shard { mints: vec![mint.clone()] };
    println!("2  its paid mint (copy #{}) added to record 0, no signature, on Y: {}", u128::from_be_bytes(mint.nonce), put(&w.y, record(), rec.encode()).await?.0);
    let unpaid = T::supply::Mint { nonce: 2u128.to_be_bytes(), paid: Some((pay.params.id(), { let mut x = mint.paid.unwrap().1; x[15] ^= 1; x })), ..mint.clone() };
    println!("3  a mint citing a payment never made, on X: {}", put(&w.x, record(), T::supply::merge(&rec, &T::supply::Shard { mints: vec![unpaid] }).unwrap().encode()).await?.0);
    let twice = T::supply::Mint { nonce: 3u128.to_be_bytes(), ..mint.clone() };
    println!("4  a second copy citing the same payment, on X: {}", put(&w.x, record(), T::supply::merge(&rec, &T::supply::Shard { mints: vec![twice] }).unwrap().encode()).await?.0);
    // CONTROL: a second real buy (copy #2, its own payment) is taken.
    let (es, m2) = w1.buy(&pay, &sd, &s, (0, &rec), 1, w.now().await?).map_err(why)?;
    pay.state.entries.extend(es);
    println!("4b 1 pays again, on X: {} — {}", w.put_chain(&w.x, p, 1, &pay.state).await?.0, pay.state.balance());
    tokio::time::sleep(Duration::from_secs(2)).await;
    let rec2 = T::supply::merge(&rec, &T::supply::Shard { mints: vec![m2.clone()] }).unwrap();
    println!("4c its paid mint (copy #{}), on X: {}", u128::from_be_bytes(m2.nonce), put(&w.x, record(), rec2.encode()).await?.0);
    // A RACE: 1 pays for #3, but a 5-copy buy (#3–7) fills record 0 first — #3's payment then buys #8 in record 1.
    let (es, _) = w1.buy(&pay, &sd, &s, (0, &rec2), 1, w.now().await?).map_err(why)?;
    let pa = es.last().unwrap().clone();
    pay.state.entries.extend(es);
    let (es, m5) = w1.buy(&pay, &sd, &s, (0, &rec2), 5, w.now().await?).map_err(why)?;
    pay.state.entries.extend(es);
    println!("6  1 pays for #3, then for 5 copies, on X: {} — {}", w.put_chain(&w.x, p, 1, &pay.state).await?.0, pay.state.balance());
    tokio::time::sleep(Duration::from_secs(2)).await;
    let full = T::supply::merge(&rec2, &T::supply::Shard { mints: vec![m5] }).unwrap();
    println!("7  the 5 copies (#3–7) fill record 0, on X: {}", put(&w.x, record(), full.encode()).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let (k, ms) = craftworks_wallet::remint(&pay.params, &pa, &sd, &s, (0, &full), &T::supply::Shard::default()).map_err(why)?;
    let record1 = || container(&w.code, T::supply::Params { code_hash: w.ch, definition: s, beacon: w.beacon, epoch: 1 }.encode());
    println!("8  #3's payment buys #{} in record {k} (record 0 full), on Y: {}", u128::from_be_bytes(ms.nonce), put(&w.y, record1(), T::supply::Shard { mints: vec![ms.clone()] }.encode()).await?.0);
    let reused = T::supply::Mint { nonce: 9u128.to_be_bytes(), stamp: mint.stamp, ..mint.clone() };
    let reused = T::supply::Mint { paid: mint.paid, ..reused };
    println!("9  copy #1's payment spilled into record 1 too, on X: {}", put(&w.x, record1(), T::supply::Shard { mints: vec![ms, reused] }.encode()).await?.0);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let held = get(&w.x, T::supply::Params { code_hash: w.ch, definition: s, beacon: w.beacon, epoch: 0 }.id()).await?.and_then(|b| T::supply::Shard::decode(&b)).context("the record")?;
    println!("   record 0 holds {} mint(s)", held.mints.len());
    let copy = Held { params: T::ChainParams { item: T::numbered_item(&s, 1), ..w.chain(s, 1) }, definition: sd.clone(), state: T::State::default(), blocks: Default::default() };
    let recs = [held];
    let now = w.now().await?;
    let got = w1.collect(&copy, &Sources { chains: &[], blocks: &Default::default(), programs: &[], shards: &recs }, now).map_err(why)?;
    let mut c = copy.clone();
    c.state.entries.extend(got);
    println!("5  1's chain of copy #1 collects it, on Y: {} — {}", put(&w.y, container(&w.code, c.params.encode()), c.state.encode()).await?.0, c.state.balance());
    Ok(())
}

/// SERVE the token set-up for a page test (TOKEN §1c, S5): on the beacon (BEACON_SEED: the pages', which they extend;
/// else one started here, this process the test's clock), and a PAY token P — its creator `PAY_OWNER` (hex DID: that account begins its chain with 1000 P when it
/// first buys) — published on X. Writes `{ pay }` to `TOKENS_CONFIG` (the page's `tokens-config`), then
/// ticks until killed.
async fn serve(w: &W) -> Result<()> {
    let owner = std::env::var("PAY_OWNER").context("PAY_OWNER: the pay token's creator (hex DID)")?;
    let owner: [u8; 32] = (0..32).map(|i| u8::from_str_radix(&owner[2 * i..2 * i + 2], 16)).collect::<Result<Vec<_>, _>>()?.try_into().map_err(|_| anyhow::anyhow!("a DID is 32 bytes"))?;
    let pd = T::Definition { genesis_owner: owner, decimals: 0, min_send: 1, ..w.def(1, b"P", 60) };
    let pay = w.put_def(&pd).await?;
    // ALLOWANCE=1: an allowance token (9 decimals) whose TREASURY is test account 1, its pot escrowed to the programs of
    // this standing epoch and the next two (each reclaimable after its epoch): the page builds each verdict.
    let allowance = if std::env::var("ALLOWANCE").is_ok() {
        use craftworks_allowance_contract as A;
        use craftworks_wallet::{Held, Wallet};
        let acode = std::fs::read(std::path::Path::new(&std::env::args().nth(1).unwrap()).with_file_name("allowance.wasm"))?;
        let ah = *blake3::hash(&acode).as_bytes();
        let ad = T::Definition { decimals: T::DECIMALS, min_send: 1, genesis_amount: 1_000 * T::UNIT, ..w.def(1, b"WRENT", 61) };
        let at = w.put_def(&ad).await?;
        let l1 = log(1);
        let w1 = Wallet { device: &Dev(1), keys: &l1 };
        let why = |e: &str| anyhow::anyhow!("{e}");
        let mut h = Held { params: w.chain(at, 1), definition: ad.clone(), state: T::State::default(), blocks: Default::default() };
        let now = w.now().await?;
        h.state.entries.extend(w1.genesis(&h, now).map_err(why)?);
        let epoch = now / 1024;
        for e in epoch..epoch + 3 {
            let pid = T::id(&ah, &A::Params { token: T::Tok { definition: at, item: [0; 32] }, epoch: e }.encode());
            let until = (e + 1) * 1024 + 8;
            h.state.entries.extend(w1.debit(&h, T::Body::Escrow { program: pid, terms: A::terms(e), amount: A::POT, until }, now).map_err(why)?);
        }
        println!("— allowance token {} (treasury: account 1), epochs {epoch}..{} funded: {}", hex(&at[..6]), epoch + 2, w.put_chain(&w.x, at, 1, &h.state).await?.0);
        Some(at)
    } else {
        None
    };
    let cfg = match allowance {
        Some(a) => format!("{{\"pay\":\"{}\",\"allowance\":\"{}\"}}\n", hex(&pay), hex(&a)),
        None => format!("{{\"pay\":\"{}\"}}\n", hex(&pay)),
    };
    let path = std::env::var("TOKENS_CONFIG").context("TOKENS_CONFIG: where to write the page's set-up")?;
    std::fs::write(&path, &cfg)?;
    println!("— serve: {cfg}  written to {path}; running (kill to stop)");
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        println!("   height {}", w.now().await?);
    }
}
