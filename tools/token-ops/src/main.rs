//! TOKEN OPERATIONS on a node (owner 10-10: "handle the first token via Wren, and manage the beacon via B").
//!
//!   token-ops beacon <beacon.wasm> <ws> <seed file> [tick secs]
//!       THE BEACON, kept ticking: its params from the seed file (made once: the same beacon after every restart), its
//!       state read back from the node and extended — a step every tick, its final outputs into the outputs log. Run
//!       as a service on the node's own box.
//!   token-ops witness <token.wasm> <idlog.wasm> <bag.wasm> <ws> <beacon hex> <pool hex> <key seed file> [tick secs]
//!       A WITNESS kept running: signs every queued entry it is seated for, by induction (TOKEN §7a).
//!   token-ops make-token <token.wasm> <idlog.wasm> <ws> <beacon id hex> <words file> <device seed file> <name> <supply>
//!       (`supply` in whole tokens: made with the standard 9 decimals, `T::DECIMALS`)
//!       A TOKEN made by the account the words hold: its definition (the account its creator, the whole supply its
//!       genesis), its authority chain and freeze list, and the account's chain begun — written by a device of the
//!       account this tool keeps (its seed file; credentialed by the words' owner key). Prints the token's id.
use anyhow::{bail, Context, Result};
use craftworks_beacon_contract as B;
use craftworks_token_contract as T;
use ed25519_dalek::{Signer as _, SigningKey};
use freenet_stdlib::prelude::*;
use std::time::{Duration, Instant};
use node_io::{seed_file, container, get};

/// A PUT's answer (`node_io::put`), its time dropped.
async fn put(ws: &str, c: ContractContainer, state: Vec<u8>) -> Result<String> {
    Ok(node_io::put(ws, c, state).await?.0)
}


fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn unhex32(s: &str) -> Result<[u8; 32]> {
    let v = (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16)).collect::<Result<Vec<_>, _>>()?;
    v.try_into().map_err(|_| anyhow::anyhow!("32 bytes of hex"))
}




/// Fresh mix for a step (the beacon's own rule takes any: its output is the VDF's).
fn mix() -> [u8; 32] {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).expect("randomness");
    b
}

/// The beacon's params — the same shape the token's live runs use — from its seed.


async fn beacon(code: &[u8], ws: &str, seed: [u8; 32], tick: Duration) -> Result<()> {
    let p = node_io::beacon_params(seed);
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
        code_hash: ch, decimals: T::DECIMALS, min_send: 1, genesis_owner: did, genesis_amount: supply.checked_mul(T::UNIT).context("a supply too large")?, name: name.as_bytes().to_vec(), nonce, beacon, margin: 2, idlog: ih,
        mint: None, freeze: None, ids: T::Ids::None, royalty: None, cap: 0, item: [0; 32], version: [0; 32], soulbound: false, transfer_fee: None, sale: None, keepers: None,
    };
    let id = T::id(&ch, &d.encode());
    for (what, (params, state)) in ["definition", "authority chain", "freeze list"].into_iter().zip(d.contracts(&ch)) {
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
        Some("registry") => {
            // token-ops registry <bag.wasm> <ws> <epoch>: the signed vouches in that epoch's vouch registry.
            let code = std::fs::read(&a[2])?;
            let epoch: u64 = a[4].parse()?;
            let mut items = Vec::new();
            for k in 0..64 {
                let address = craftworks_reputation::item::registry_address(epoch, k);
                let id: [u8; 32] = container(&code, address.to_vec()).key().id().as_bytes().try_into()?;
                let these = get(&a[3], id).await?.and_then(|b| craftworks_bag_contract::read(&address, &b)).unwrap_or_default();
                let full = craftworks_bag_contract::chained::full(these.iter().map(|i| craftworks_bag_contract::payload(i).len()));
                items.extend(these);
                if !full {
                    break;
                }
            }
            for i in &items {
                match craftworks_reputation::item::Item::decode(craftworks_bag_contract::payload(i)) {
                    Some(v) => println!("{} → {} at {} {}", hex(&v.from[..4]), hex(&v.to[..4]), v.at, if v.on { "vouch" } else { "withdraw" }),
                    None => println!("(not a vouch item)"),
                }
            }
            println!("{} item(s)", items.len());
            Ok(())
        }
        Some("statement") => {
            // token-ops statement <bag.wasm> <ws> <epoch>: the roots posted for that epoch's statement — and each
            // root's entries as stored (key, value bytes) when block.wasm lies beside bag.wasm: what two differing
            // roots differ in.
            let code = std::fs::read(&a[2])?;
            let block_code = std::fs::read(std::path::Path::new(&a[2]).with_file_name("block.wasm")).ok();
            let address = craftworks_reputation::statement::bag_address(a[4].parse()?);
            let id: [u8; 32] = container(&code, address.to_vec()).key().id().as_bytes().try_into()?;
            let items = get(&a[3], id).await?.and_then(|b| craftworks_bag_contract::read(&address, &b)).unwrap_or_default();
            for i in &items {
                let Some(p) = craftworks_reputation::statement::Posted::decode(craftworks_bag_contract::payload(i)) else {
                    println!("(not a posted statement)");
                    continue;
                };
                println!("epoch {} root {} ({} parity)", p.epoch, hex(&p.root), p.parity.len());
                let Some(bc) = &block_code else { continue };
                let mut held: std::collections::HashMap<[u8; 32], Vec<u8>> = Default::default();
                for _ in 0..64 {
                    match craftworks_reputation::statement::entries(&p.root, &|c| held.get(c).cloned()) {
                        Ok(es) => {
                            for (k, v) in es {
                                println!("  {} = {}", hex(&k), v.map_or("(not inline)".into(), |v| format!("{} B {}", v.len(), hex(&v))));
                            }
                            break;
                        }
                        Err(want) if want.is_empty() => break,
                        Err(want) => {
                            for c in want {
                                match get(&a[3], contract_keys::block::contract_for(bc, &c)).await?.filter(|s| s.len() > 1) {
                                    Some(st) => held.insert(c, st[1..].to_vec()),
                                    None => {
                                        println!("  block {} not on the network", hex(&c));
                                        held.insert(c, Vec::new())
                                    }
                                };
                            }
                        }
                    }
                }
            }
            println!("{} posted", items.len());
            Ok(())
        }
        Some("chains") => {
            // token-ops chains <token.wasm> <idlog.wasm> <bag.wasm> <ws> <beacon hex> <epoch>…
            // (beacon.wasm, standing.wasm beside token.wasm): every chain queued for its witnesses in those standing
            // epochs, and whether the network still answers it (its entries) — a diagnosis for "not seen".
            let net = token_net::Net {
                ws: a[5].clone(),
                code_hash: *blake3::hash(&std::fs::read(&a[2])?).as_bytes(),
                idlog_hash: *blake3::hash(&std::fs::read(&a[3])?).as_bytes(),
                bag_code: std::fs::read(&a[4])?,
                beacon_hash: *blake3::hash(&std::fs::read(std::path::Path::new(&a[2]).with_file_name("beacon.wasm"))?).as_bytes(),
                beacon: unhex32(&a[6])?,
                pool: [0; 32],
                standing_hash: *blake3::hash(&std::fs::read(std::path::Path::new(&a[2]).with_file_name("standing.wasm"))?).as_bytes(),
                block_code: std::fs::read(std::path::Path::new(&a[2]).with_file_name("block.wasm"))?,
            };
            let epochs: Vec<u64> = a[7..].iter().map(|e| e.parse()).collect::<Result<_, _>>()?;
            tokio::task::spawn_blocking(move || -> Result<()> {
                let r = token_net::Reader::new(&net)?;
                let mut seen = std::collections::HashSet::new();
                for e in epochs {
                    for q in r.queue(e) {
                        if !seen.insert((q.token.definition, q.owner)) {
                            continue;
                        }
                        let id = r.params(&q.token, &q.owner).map(|p| hex(&p.id()));
                        let st = r.chain(&q.token, &q.owner);
                        println!("epoch {e} token {} owner {} chain {}: {}", hex(&q.token.definition[..4]), hex(&q.owner[..4]), id.unwrap_or("? (no definition)".into()), st.map_or("NOT FOUND".into(), |s| format!("{} entries", s.entries.len())));
                    }
                }
                Ok(())
            })
            .await?
        }
        Some("keepers") => {
            // token-ops keepers <token.wasm> <idlog.wasm> <bag.wasm> <ws> <beacon hex> <epoch> (beacon.wasm, standing.wasm,
            // block.wasm beside token.wasm): each keeper listed that epoch and the heights its record answered, by epoch.
            let net = token_net::Net {
                ws: a[5].clone(),
                code_hash: *blake3::hash(&std::fs::read(&a[2])?).as_bytes(),
                idlog_hash: *blake3::hash(&std::fs::read(&a[3])?).as_bytes(),
                bag_code: std::fs::read(&a[4])?,
                beacon_hash: *blake3::hash(&std::fs::read(std::path::Path::new(&a[2]).with_file_name("beacon.wasm"))?).as_bytes(),
                beacon: unhex32(&a[6])?,
                pool: [0; 32],
                standing_hash: *blake3::hash(&std::fs::read(std::path::Path::new(&a[2]).with_file_name("standing.wasm"))?).as_bytes(),
                block_code: std::fs::read(std::path::Path::new(&a[2]).with_file_name("block.wasm"))?,
            };
            let epoch: u64 = a[7].parse()?;
            tokio::task::spawn_blocking(move || -> Result<()> {
                let r = token_net::Reader::new(&net)?;
                for (did, key, rec) in r.keepers(epoch) {
                    match rec {
                        None => println!("{} key {}: no record", hex(&did[..4]), hex(&key[..4])),
                        Some(eps) => {
                            for (e, hs) in eps {
                                println!("{} key {} epoch {e}: {} answer(s) {:?}", hex(&did[..4]), hex(&key[..4]), hs.len(), hs);
                            }
                        }
                    }
                }
                Ok(())
            })
            .await?
        }
        Some("accepted") => {
            // token-ops accepted <token.wasm> <bag.wasm> <ws> <beacon hex> <epoch> <root hex>
            // (beacon.wasm beside token.wasm): an EPOCH STATEMENT root's acceptance by the rules' own check
            // (`tokens::statement_accepted`, $CRAFT the pool), and what it read — the place's signatures and the claims.
            let (code_hash, beacon) = (*blake3::hash(&std::fs::read(&a[2])?).as_bytes(), unhex32(&a[5])?);
            let idlog_hash = *blake3::hash(&std::fs::read(std::path::Path::new(&a[2]).with_file_name("idlog.wasm"))?).as_bytes();
            let pool = T::id(&code_hash, &craftworks_wallet::craft::definition(code_hash, beacon, idlog_hash).encode());
            let bag_code = std::fs::read(&a[3])?;
            let lot = craftworks_rules::tokens::Lot { pool, bag: *blake3::hash(&bag_code).as_bytes(), beacon_code: *blake3::hash(&std::fs::read(std::path::Path::new(&a[2]).with_file_name("beacon.wasm"))?).as_bytes() };
            let (epoch, root): (u64, [u8; 32]) = (a[6].parse()?, unhex32(&a[7])?);
            let mut got: std::collections::HashMap<[u8; 32], Vec<u8>> = Default::default();
            for _ in 0..16 {
                let g = got.clone();
                match craftworks_rules::tokens::statement_accepted(&lot, &|x| g.get(x).cloned(), code_hash, beacon, epoch, &root).map_err(|e| anyhow::anyhow!(e))? {
                    craftworks_rules::tokens::Step::Want(w) => {
                        for id in w {
                            got.insert(id, get(&a[4], id).await?.unwrap_or_default());
                        }
                    }
                    craftworks_rules::tokens::Step::Done(r) => {
                        println!("epoch {epoch} root {}: {r:?}", hex(&root[..6]));
                        let place = craftworks_reputation::statement::witness_place(epoch);
                        let addr = T::wallet::witness_bag(&place, 0);
                        let bag = got.get(&T::id(&lot.bag, &addr)).and_then(|b| craftworks_bag_contract::read(&addr, b)).unwrap_or_default();
                        for i in &bag {
                            if let Some(w) = T::wallet::bytes::read_witness_sig(craftworks_bag_contract::payload(i)) {
                                println!("  sig on {} by a key: {}", hex(&w.entry[..6]), if w.entry == root { "this root" } else { "another root" });
                            }
                        }
                        let h = craftworks_reputation::statement::lot_height(epoch);
                        let reg = T::claims::Params { code_hash, pool, beacon, epoch: h / T::supply::EPOCH }.id();
                        for c in got.get(&reg).and_then(|b| T::claims::Registry::decode(b)).map_or(vec![], |r| r.claims) {
                            let signed = bag.iter().filter_map(|i| T::wallet::bytes::read_witness_sig(craftworks_bag_contract::payload(i))).any(|w| w.entry == root && w.signed_by(&c.key));
                            let pd = got.get(&pool).and_then(|b| T::Definition::decode(b));
                            let stake = pd.as_ref().and_then(|pd| {
                                let cp = T::ChainParams { code_hash, definition: pool, owner: c.owner, beacon: pd.beacon, idlog: pd.idlog, item: [0; 32] };
                                got.get(&cp.id()).and_then(|b| T::State::decode(b)).map(|st| T::wallet::stake_at(&cp, pd, &st, (h / T::supply::EPOCH) * T::supply::EPOCH))
                            });
                            println!("  claim {} amount {} key {} signed {signed} stake at the epoch's start: {stake:?}", hex(&c.owner[..4]), c.amount, hex(&c.key[..4]));
                        }
                        return Ok(());
                    }
                    craftworks_rules::tokens::Step::Sign(_) => anyhow::bail!("a status never signs"),
                }
            }
            anyhow::bail!("too many reads")
        }
        Some("witness") => {
            // token-ops witness <token.wasm> <idlog.wasm> <bag.wasm> <ws> <beacon hex> <pool hex | craft> <key seed file> [tick secs]
            // (the beacon's and the standing contract's code: beacon.wasm, standing.wasm beside token.wasm):
            // A WITNESS (TOKEN §7a), kept running: every tick it reads the witness queue of this standing epoch and the
            // last, and signs each entry it is SEATED for (drawn by lot, or the baseline's seat while the network is
            // small) once the rule takes it (`wallet::may_witness`, by induction — an entry refused now, its past not yet
            // accepted, is tried again until it is too old) into the entry's witness bag.
            let (code_hash, idlog_hash, beacon) = (*blake3::hash(&std::fs::read(&a[2])?).as_bytes(), *blake3::hash(&std::fs::read(&a[3])?).as_bytes(), unhex32(&a[6])?);
            // The stake pool: `craft` — $CRAFT, keepers' earned stake (its one definition for this network).
            let pool = if a[7] == "craft" { T::id(&code_hash, &craftworks_wallet::craft::definition(code_hash, beacon, idlog_hash).encode()) } else { unhex32(&a[7])? };
            let net = token_net::Net {
                ws: a[5].clone(),
                code_hash,
                idlog_hash,
                bag_code: std::fs::read(&a[4])?,
                beacon_hash: *blake3::hash(&std::fs::read(std::path::Path::new(&a[2]).with_file_name("beacon.wasm"))?).as_bytes(),
                beacon,
                pool,
                standing_hash: *blake3::hash(&std::fs::read(std::path::Path::new(&a[2]).with_file_name("standing.wasm"))?).as_bytes(),
                block_code: std::fs::read(std::path::Path::new(&a[2]).with_file_name("block.wasm"))?,
            };
            let key = ed25519_dalek::SigningKey::from_bytes(&seed_file(&a[8])?);
            let tick = Duration::from_secs(a.get(9).map_or(Ok(10), |t| t.parse())?);
            println!("witness {} — pool {}, tick {:?}", hex(&key.verifying_key().to_bytes()[..6]), hex(&net.pool[..6]), tick);
            tokio::task::spawn_blocking(move || -> Result<()> {
                let r = token_net::Reader::new(&net)?;
                let mut signed: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
                let mut said: std::collections::HashMap<Vec<u8>, String> = std::collections::HashMap::new();
                let mut statements: std::collections::HashSet<u64> = std::collections::HashSet::new();
                loop {
                    // The epoch STATEMENT (TOKEN §7d): the posted root this witness computes too, signed once.
                    let h = r.height();
                    let se = h.saturating_sub(craftworks_reputation::statement::SETTLE) / craftworks_reputation::EPOCH;
                    if !statements.contains(&se) {
                        match r.witness_statement(se, &key) {
                            Ok(Some(root)) => {
                                println!("signed  the statement of epoch {se}: root {}", hex(&root[..6]));
                                statements.insert(se);
                            }
                            Ok(None) => {}
                            Err(why) => {
                                let k = format!("statement {se}").into_bytes();
                                if said.get(&k) != Some(&why) {
                                    println!("refused the statement of epoch {se}: {why}");
                                    said.insert(k, why);
                                }
                            }
                        }
                    }
                    // The queue is per STANDING epoch (`wallet::queue::address`), as pages write it.
                    let epoch = h / craftworks_reputation::EPOCH;
                    for e in epoch.saturating_sub(1)..=epoch {
                        for q in r.queue(e) {
                            let k = q.encode();
                            if signed.contains(&k) {
                                continue;
                            }
                            let place = format!("{}…/{} seq {}", hex(&q.token.definition[..4]), hex(&q.owner[..4]), q.seq);
                            match r.witness(&q, &key) {
                                Ok(Some(_)) => {
                                    println!("signed  {place}");
                                    signed.insert(k);
                                }
                                Ok(None) => {}
                                Err(why) => {
                                    if said.get(&k) != Some(&why) {
                                        println!("refused {place}: {why}");
                                        said.insert(k, why);
                                    }
                                }
                            }
                        }
                    }
                    std::thread::sleep(tick);
                }
            })
            .await?
        }
        Some("key") => {
            // token-ops key <seed file>: the ed25519 PUBLIC key of a seed file (hex) — never the seed.
            println!("{}", hex(&ed25519_dalek::SigningKey::from_bytes(&seed_file(&a[2])?).verifying_key().to_bytes()));
            Ok(())
        }
        Some("height") => {
            // token-ops height <ws> <beacon id hex>: its height now (0: not there).
            let h = get(&a[2], unhex32(&a[3])?).await?.and_then(|b| B::wire::decode(&b)).map_or(0, |s| s.height());
            println!("{h}");
            Ok(())
        }
        Some("params") => {
            // token-ops params <beacon.wasm> <seed file>: the beacon's params (hex — a page's `beacon-config`) and its id.
            let code = std::fs::read(&a[2])?;
            let p = node_io::beacon_params(seed_file(&a[3])?);
            let id = container(&code, p.encode()).key().id().as_bytes().to_vec();
            println!("{{\"params\":\"{}\"}}", hex(&p.encode()));
            eprintln!("beacon {}", hex(&id));
            Ok(())
        }
        Some("make-token") => {
            let (code, idlog) = (std::fs::read(&a[2])?, std::fs::read(&a[3])?);
            let words = std::fs::read_to_string(&a[6])?;
            make_token(&code, &idlog, &a[4], unhex32(&a[5])?, words.trim(), seed_file(&a[7])?, &a[8], a[9].parse()?).await
        }
        _ => bail!("token-ops beacon <beacon.wasm> <ws> <seed file> [tick secs] | make-token <token.wasm> <idlog.wasm> <ws> <beacon hex> <words file> <device seed file> <name> <supply>"),
    }
}
