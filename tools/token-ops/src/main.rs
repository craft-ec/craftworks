//! TOKEN OPERATIONS on a node (owner 10-10: "handle the first token via Wren, and manage the beacon via B").
//!
//!   token-ops beacon <beacon.wasm> <ws> <seed file> [tick secs]
//!       THE BEACON, kept ticking: its params from the seed file (made once: the same beacon after every restart), its
//!       state read back from the node and extended — a step every tick, its final outputs into the outputs log. Run
//!       as a service on the node's own box.
//!   token-ops make-token <token.wasm> <idlog.wasm> <ws> <beacon id hex> <words file> <device seed file> <name> <supply>
//!       A TOKEN made by the account the words hold: its definition (the account its creator, the whole supply its
//!       genesis), its authority chain and freeze list, and the account's chain begun — written by a device of the
//!       account this tool keeps (its seed file; credentialed by the words' owner key). Prints the token's id.
use anyhow::{bail, Context, Result};
use craftworks_beacon_contract as B;
use craftworks_token_contract as T;
use ed25519_dalek::{Signer as _, SigningKey};
use freenet_stdlib::prelude::*;
use futures::{SinkExt, StreamExt};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

fn container(code: &[u8], params: Vec<u8>) -> ContractContainer {
    ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(std::sync::Arc::new(ContractCode::from(code.to_vec())), Parameters::from(params))))
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn unhex32(s: &str) -> Result<[u8; 32]> {
    let v = (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16)).collect::<Result<Vec<_>, _>>()?;
    v.try_into().map_err(|_| anyhow::anyhow!("32 bytes of hex"))
}
/// 32 bytes from a file holding them as hex — made fresh (and kept, 0600) when it does not exist.
fn seed_file(path: &str) -> Result<[u8; 32]> {
    if let Ok(s) = std::fs::read_to_string(path) {
        return unhex32(s.trim());
    }
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).map_err(|e| anyhow::anyhow!("randomness: {e}"))?;
    std::fs::write(path, hex(&b) + "\n")?;
    #[cfg(unix)]
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(b)
}

async fn put(ws: &str, c: ContractContainer, state: Vec<u8>) -> Result<String> {
    let (mut tx, mut rx) = tokio_tungstenite::connect_async(ws).await.context("connecting")?.0.split();
    for f in wire::frame_put(c, WrappedState::new(state), 2).map_err(|e| anyhow::anyhow!(e))? {
        tx.send(Message::Binary(f.into())).await?;
    }
    let mut r = wire::Reassembler::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        match tokio::time::timeout_at(deadline, rx.next()).await {
            Err(_) => return Ok("NO ANSWER in 90 s".into()),
            Ok(None) => bail!("closed"),
            Ok(Some(Err(e))) => bail!("{e}"),
            Ok(Some(Ok(Message::Binary(b)))) => match wire::unframe(&mut r, &b) {
                wire::Incoming::Ack(wire::AckKind::Put(_)) => return Ok("ACCEPTED".into()),
                wire::Incoming::PutFailed { said, .. } | wire::Incoming::PutFailedByText { said, .. } => return Ok(format!("REFUSED: {said}")),
                wire::Incoming::Refused(r) => return Ok(format!("REFUSED: {r:?}")),
                _ => {}
            },
            Ok(Some(Ok(_))) => {}
        }
    }
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

/// Fresh mix for a step (the beacon's own rule takes any: its output is the VDF's).
fn mix() -> [u8; 32] {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).expect("randomness");
    b
}

/// The beacon's params — the same shape the token's live runs use — from its seed.
fn beacon_params(seed: [u8; 32]) -> B::Params {
    B::Params { k: 1 << 20, segs: 256, sample: 8, window: 2, keep: 600, seed }
}

async fn beacon(code: &[u8], ws: &str, seed: [u8; 32], tick: Duration) -> Result<()> {
    let p = beacon_params(seed);
    let c = || container(code, p.encode());
    let id: [u8; 32] = c().key().id().as_bytes().try_into().unwrap();
    // Where it is: read back (a restart continues it), else begun.
    let mut s = match get(ws, id).await?.and_then(|b| B::wire::decode(&b)) {
        Some(s) => s,
        None => B::State::default().extend(&p, mix()),
    };
    println!("beacon {} — height {} (tick {:?})", hex(&id), s.height(), tick);
    let mut last_report = Instant::now();
    loop {
        s = s.extend(&p, mix());
        let said = put(ws, c(), B::wire::encode(&s)).await.unwrap_or_else(|e| format!("ERROR {e}"));
        if said != "ACCEPTED" {
            // The node holds another branch or a newer step: take the node's and go on from there.
            println!("height {}: {said} — reading the node's", s.height());
            if let Some(n) = get(ws, id).await.ok().flatten().and_then(|b| B::wire::decode(&b)) {
                s = n;
            }
        }
        let fin = s.height().saturating_sub(p.window as u64);
        for idx in [fin.saturating_sub(1) / B::log::SPAN, fin / B::log::SPAN] {
            let lp = B::log::Params { beacon: id, index: idx };
            let rec = B::log::record(&lp, &s);
            if !rec.entries.is_empty() && (idx == fin / B::log::SPAN || fin % B::log::SPAN < 2) {
                let _ = put(ws, container(code, lp.encode()), rec.encode()).await;
            }
        }
        if last_report.elapsed() > Duration::from_secs(600) {
            println!("height {}", s.height());
            last_report = Instant::now();
        }
        tokio::time::sleep(tick).await;
    }
}

/// The tool's DEVICE of an account: its key, and the credential the words' owner key gave it.
struct Dev {
    key: SigningKey,
    cred: Vec<u8>,
    seq: u32,
}
impl craftworks_wallet::Device for Dev {
    fn public(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }
    fn sign(&self, m: &[u8]) -> [u8; 64] {
        self.key.sign(m).to_bytes()
    }
    fn credential(&self) -> Vec<u8> {
        self.cred.clone()
    }
    fn key_seq(&self) -> u32 {
        self.seq
    }
}

#[allow(clippy::too_many_arguments)]
async fn make_token(code: &[u8], idlog_code: &[u8], ws: &str, beacon: [u8; 32], words: &str, device_seed: [u8; 32], name: &str, supply: u128) -> Result<()> {
    let entropy = craftworks_account::entropy(words).context("the words do not read")?;
    let ch = *blake3::hash(code).as_bytes();
    let ih = *blake3::hash(idlog_code).as_bytes();
    // The account: its DID from its key log (the words' owner key is one of its keys — rotated or not).
    let did_text = std::env::var("DID").context("DID: the account's did:craftec:… (the words may be rotated into it)")?;
    let did = craftworks_account::did_bytes(&did_text).context("not a DID")?;
    let log = get(ws, T::id(&ih, &did)).await?.and_then(|b| craftworks_idlog_contract::read(&did, &b)).context("the account's key log is not there")?;
    let (seq, owner_seed) = (0..8u32)
        .filter_map(|j| craftworks_account::owner_seed_at(&entropy, j))
        .find_map(|s| {
            let pk = SigningKey::from_bytes(&s).verifying_key().to_bytes();
            log.events.iter().position(|e| e.key == pk).map(|i| (i as u32, s))
        })
        .context("these words hold none of the account's keys")?;
    let key = SigningKey::from_bytes(&device_seed);
    let cred = craftworks_identity::credential(&did, &key.verifying_key().to_bytes(), &[], &owner_seed);
    let dev = Dev { key, cred, seq };
    let mut nonce = [0u8; 16];
    getrandom::getrandom(&mut nonce).map_err(|e| anyhow::anyhow!("randomness: {e}"))?;
    let d = T::Definition {
        code_hash: ch, decimals: 0, min_send: 1, genesis_owner: did, genesis_amount: supply, name: name.as_bytes().to_vec(), nonce, beacon, margin: 2, idlog: ih,
        mint: None, freeze: None, ids: T::Ids::None, royalty: None, cap: 0, item: [0; 32], version: [0; 32], soulbound: false, transfer_fee: None, sale: None,
    };
    let id = T::id(&ch, &d.encode());
    let a = d.authority();
    let f = T::freeze::Params { code_hash: ch, definition: id, beacon };
    for (what, params, state) in [("definition", d.encode(), d.encode()), ("authority chain", a.encode(), T::authority::Chain::default().encode()), ("freeze list", f.encode(), T::freeze::List::default().encode())] {
        println!("{what}: {}", put(ws, container(code, params), state).await?);
    }
    let now = get(ws, beacon).await?.and_then(|b| B::wire::decode(&b)).map(|s| s.height()).context("the beacon is not there")?;
    let params = T::ChainParams { code_hash: ch, definition: id, owner: did, beacon, idlog: ih, item: [0; 32] };
    let held = craftworks_wallet::Held { params: params.clone(), definition: d.clone(), state: T::State::default(), blocks: Default::default() };
    let w = craftworks_wallet::Wallet { device: &dev, keys: &log };
    let entries = w.genesis(&held, now).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut s = T::State::default();
    s.entries.extend(entries);
    println!("the account's chain ({} of {name}): {}", s.balance(), put(ws, container(code, params.encode()), s.encode()).await?);
    println!("TOKEN {} — {name}, {supply}, made by {did_text} (its chain {})", hex(&id), hex(&params.id()));
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("beacon") => {
            let code = std::fs::read(&a[2])?;
            let tick = Duration::from_secs(a.get(5).map_or(Ok(5), |t| t.parse())?);
            beacon(&code, &a[3], seed_file(&a[4])?, tick).await
        }
        Some("make-token") => {
            let (code, idlog) = (std::fs::read(&a[2])?, std::fs::read(&a[3])?);
            let words = std::fs::read_to_string(&a[6])?;
            make_token(&code, &idlog, &a[4], unhex32(&a[5])?, words.trim(), seed_file(&a[7])?, &a[8], a[9].parse()?).await
        }
        _ => bail!("token-ops beacon <beacon.wasm> <ws> <seed file> [tick secs] | make-token <token.wasm> <idlog.wasm> <ws> <beacon hex> <words file> <device seed file> <name> <supply>"),
    }
}
