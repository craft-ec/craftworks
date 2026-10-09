//! A NODE THAT LOST BLOCKS (the Assets demo, main 2026-09-27: publish, THEN the node loses data -- decision (a)).
//!
//!   node-forget forget <node-data-dir> <block.wasm> --root <hex> --lose m|m+1 [--groups N] [--domain D]
//!       On a STOPPED private node: read every Block-contract state it stores (freenet 0.2.138's redb store,
//!       `<data-dir>/db/db`, table `state`, keyed by the contract instance id), find the tree's groups
//!       (`probe::forget`), and REMOVE the chosen blocks' states (and their hosting metadata) -- `N` groups (default 1)
//!       each losing `m` (repairable) or `m + 1` (the control). A running node holds the store's lock: refused.
//!   node-forget check <node-ws-url> <block.wasm> <block-id-hex>...
//!       On the RESTARTED node: GET each block, and say what the node answers -- `found` or `not_found` -- so a run
//!       shows the loss is the node's (a forgotten block answers NotFound, a kept one still answers).
//!
//! Private nodes only: the data dir must carry `probe::node::PRIVATE_MARKER` (written by whatever spawned the node;
//! an allowlist -- a path under ~/Library or the freenet default is refused too, as a second guard), and the ws URL's port must pass `probe::node::allowed_port`. One JSON line per fact, on stdout.
use anyhow::{bail, Context, Result};
use freenet_stdlib::client_api::{ClientRequest, ContractRequest, ContractResponse, HostResponse};
use freenet_stdlib::prelude::{ContractCode, ContractInstanceId};
use probe::forget::{choose, groups_of, lose_count, reachable};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

// freenet-core v0.2.138, crates/core/src/contract/storages/redb.rs: the tables a contract's state lives in.
const STATE_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("state");
const HOSTING_METADATA_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("hosting_metadata");

const USAGE: &str = "usage: node-forget forget <node-data-dir> <block.wasm> --root <tree-root-hex> --lose m|m+1 [--groups N] [--domain D]\n       node-forget check <node-ws-url> <block.wasm> <block-id-hex>...\n       node-forget keys <node-data-dir> <block.wasm> --root <tree-root-hex>";

fn code_hash(block_wasm: &Path) -> Result<[u8; 32]> {
    let code = std::fs::read(block_wasm).with_context(|| format!("reading {}", block_wasm.display()))?;
    let hash = *ContractCode::from(code).hash();
    let bytes: &[u8] = hash.as_ref();
    bytes.try_into().context("a code hash that is not 32 bytes")
}

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// The node's store, found under its data dir; refused for anything that is not a private node's own dir.
fn store_of(data_dir: &Path) -> Result<PathBuf> {
    // The allowlist, before anything is opened: only a dir a private node was spawned with (its PRIVATE_MARKER).
    probe::node::require_private(data_dir)?;
    let s = data_dir.to_string_lossy();
    // freenet-core v0.2.138 redb.rs `new(data_dir)`: the store is `<data-dir>/db` (the dir) joined with "db" (the file);
    // a LOCAL node (`freenet local`) keeps it one level down, `<data-dir>/db/local/db` (seen 2026-09-27).
    for p in [data_dir.join("db").join("db"), data_dir.join("db").join("local").join("db"), data_dir.join("data").join("db").join("db")] {
        if p.exists() {
            return Ok(p);
        }
    }
    bail!("no db/db (the redb store) under {s}: not a freenet 0.2.138 node's data dir")
}

fn forget(args: &[String]) -> Result<()> {
    let [data_dir, block_wasm, ..] = args else { bail!("{USAGE}") };
    let n = lose_count(&opt(args, "--lose").context("--lose m|m+1 is required")?)?;
    let take: usize = opt(args, "--groups").map_or(Ok(1), |g| g.parse()).context("--groups is a count")?;
    // The head's CURRENT tree: `node:<hex>` (the SDK's db.root()) or bare hex. The store keeps every older version's
    // nodes too, and a group of an old node is not one the head reads.
    let root_arg = opt(args, "--root").context("--root <tree root> is required (the SDK's db.root())")?;
    let root: [u8; 32] = core_types::hex::decode(root_arg.trim_start_matches("node:"))
        .and_then(|b| b.try_into().ok())
        .with_context(|| format!("{root_arg}: not a 64-hex tree root"))?;
    let domain = opt(args, "--domain").map(|d| craftworks_sdk::db::record_prefix(&d));
    let code = code_hash(Path::new(block_wasm))?;
    let path = store_of(Path::new(data_dir))?;
    let db = Database::open(&path).with_context(|| format!("opening {} (a RUNNING node holds its lock: stop it first)", path.display()))?;

    // Every stored Block-contract state: its id computed from its bytes, and kept only when the key it is stored
    // under IS that block's contract (instance(block code, id)) -- never a guess from the bytes alone.
    let mut blocks: BTreeMap<[u8; 32], (Vec<u8>, Vec<u8>)> = BTreeMap::new(); // cid -> (key, state)
    let mut states = 0usize;
    {
        let txn = db.begin_read()?;
        let table = txn.open_table(STATE_TABLE)?;
        for row in table.iter()? {
            let (k, v) = row?;
            states += 1;
            let (key, state) = (k.value().to_vec(), v.value().to_vec());
            if let Some((cid, _)) = wire::block::block_of_state(&state) {
                if contract_keys::instance(&code, &cid).as_slice() == key.as_slice() {
                    blocks.insert(cid, (key, state));
                }
            }
        }
    }
    let tree = reachable(root, |c| blocks.get(c).map(|(_, s)| s.as_slice()));
    // Only a group the node holds WHOLE: a slot already missing would make `m` lost read as more.
    let all: Vec<_> = tree.iter().flat_map(|state| groups_of(state, domain.as_deref())).collect();
    let groups: Vec<_> = all.iter().filter(|(m, p)| m.iter().chain(p).all(|s| blocks.contains_key(s))).cloned().collect();
    let chosen = choose(&groups, take, n);
    println!("{}", serde_json::json!({ "states": states, "blocks": blocks.len(), "tree_nodes": tree.len(), "groups": all.len(), "groups_whole": groups.len(), "lose": n, "groups_chosen": chosen.len() }));
    if chosen.len() < take {
        bail!("only {} group(s) of k >= 2 to choose from (asked {take}): nothing forgotten", chosen.len());
    }
    let txn = db.begin_write()?;
    let mut forgot = 0usize;
    {
        let mut state = txn.open_table(STATE_TABLE)?;
        let mut hosting = txn.open_table(HOSTING_METADATA_TABLE)?;
        for (i, c) in chosen.iter().enumerate() {
            let lost: Vec<String> = c.lost.iter().map(|id| core_types::hex::encode(id)).collect();
            let held: Vec<bool> = c.slots.iter().map(|s| blocks.contains_key(s)).collect();
            println!("{}", serde_json::json!({ "group": i, "k": c.k, "slots": c.slots.len(), "slots_stored": held.iter().filter(|h| **h).count(), "lost": lost }));
            for id in &c.lost {
                let Some((key, st)) = blocks.get(id) else {
                    println!("{}", serde_json::json!({ "not_stored": core_types::hex::encode(id) }));
                    continue;
                };
                let removed = state.remove(key.as_slice())?.is_some();
                hosting.remove(key.as_slice())?;
                if removed {
                    forgot += 1;
                    println!("{}", serde_json::json!({ "forgot": core_types::hex::encode(id), "bytes": st.len() }));
                }
            }
        }
    }
    txn.commit()?;
    println!("{}", serde_json::json!({ "forgot_total": forgot }));
    Ok(())
}

/// `keys <data-dir> <block.wasm> --root <hex>`: every LEAF entry of the head's tree, one JSON line each (the key as
/// lossy text), so a run can say what a tree's rows are.
fn keys(args: &[String]) -> Result<()> {
    let [data_dir, block_wasm, ..] = args else { bail!("{USAGE}") };
    let root_arg = opt(args, "--root").context("--root <tree root> is required")?;
    let root: [u8; 32] = core_types::hex::decode(root_arg.trim_start_matches("node:")).and_then(|b| b.try_into().ok()).with_context(|| format!("{root_arg}: not a 64-hex tree root"))?;
    let code = code_hash(Path::new(block_wasm))?;
    let db = Database::open(store_of(Path::new(data_dir))?)?;
    let txn = db.begin_read()?;
    let table = txn.open_table(STATE_TABLE)?;
    let mut blocks: BTreeMap<[u8; 32], Vec<u8>> = BTreeMap::new();
    for row in table.iter()? {
        let (k, v) = row?;
        let state = v.value().to_vec();
        if let Some((cid, _)) = wire::block::block_of_state(&state) {
            if contract_keys::instance(&code, &cid).as_slice() == k.value() {
                blocks.insert(cid, state);
            }
        }
    }
    for state in reachable(root, |c| blocks.get(c).map(Vec::as_slice)) {
        let node = freenet_prolly::node::Node::parse(&state[1..]).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        if node.is_leaf() {
            for i in 0..node.len() {
                println!("{}", serde_json::json!({ "key": String::from_utf8_lossy(&node.key(i)) }));
            }
        }
    }
    Ok(())
}

async fn check(args: &[String]) -> Result<()> {
    let [ws, block_wasm, ids @ ..] = args else { bail!("{USAGE}") };
    if ids.is_empty() {
        bail!("{USAGE}");
    }
    probe::node::allowed_port(ws)?;
    let code = code_hash(Path::new(block_wasm))?;
    let mut c = probe::signer::connect(ws).await?;
    for hex in ids {
        let cid: [u8; 32] = core_types::hex::decode(hex).and_then(|b| b.try_into().ok()).with_context(|| format!("{hex}: not a 64-hex block id"))?;
        let key = ContractInstanceId::new(contract_keys::instance(&code, &cid));
        c.send(ClientRequest::ContractOp(ContractRequest::Get { key, return_contract_code: false, subscribe: false, blocking_subscribe: false })).await?;
        let answer = match tokio::time::timeout(Duration::from_secs(20), c.recv()).await {
            Ok(Ok(HostResponse::ContractResponse(ContractResponse::GetResponse { state, .. }))) => {
                if wire::block::block_of_state(state.as_ref()).is_some_and(|(id, _)| id == cid) { "found" } else { "wrong_bytes" }
            }
            Ok(Ok(HostResponse::ContractResponse(ContractResponse::NotFound { .. }))) => "not_found",
            Ok(Ok(other)) => {
                println!("{}", serde_json::json!({ "block": hex, "answer": "other", "said": format!("{other:?}").chars().take(200).collect::<String>() }));
                continue;
            }
            Ok(Err(e)) => {
                let said = e.to_string();
                if said.to_lowercase().contains("not found") { "not_found" } else {
                    println!("{}", serde_json::json!({ "block": hex, "answer": "error", "said": said.chars().take(200).collect::<String>() }));
                    continue;
                }
            }
            Err(_) => "silent_20s",
        };
        println!("{}", serde_json::json!({ "block": hex, "answer": answer }));
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("forget") => forget(&args[1..]),
        Some("check") => check(&args[1..]).await,
        Some("keys") => keys(&args[1..]),
        _ => bail!("{USAGE}"),
    }
}
