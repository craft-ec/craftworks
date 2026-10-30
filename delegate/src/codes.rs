//! THE CONTRACTS' CODE upkeep writes and reads: what a page hands over (`UpkeepCodes`, kept in the identity's secrets)
//! or, until one has, THIS BUILD's OWN — the same `contracts/*.wasm` the site's `*-wasm` packages are published from
//! (`publish`). 10-30, k87-three: a page logged in and closed before handing the codes over, and the node's account
//! round stopped ("the contracts' code is not held here yet") with nothing to wait for.
use craftworks_identity::{self as identity, Host};

const BAG: &[u8] = include_bytes!("../../contracts/bag.wasm");
const TAIL: &[u8] = include_bytes!("../../contracts/tail.wasm");
const SEALED: &[u8] = include_bytes!("../../contracts/sealed.wasm");
const PIECE: &[u8] = include_bytes!("../../contracts/piece.wasm");
const IDLOG: &[u8] = include_bytes!("../../contracts/idlog.wasm");
const BLOCK: &[u8] = include_bytes!("../../contracts/block.wasm");
const DEVICES: &[u8] = include_bytes!("../../contracts/devices.wasm");

/// The secret `key` (one of `identity::UPKEEP_*`'s codes): handed over, else this build's own (a code, or — idlog,
/// block, devices — its hash, as the hand-over keeps them).
pub fn get<H: Host>(h: &H, key: &[u8]) -> Option<Vec<u8>> {
    if let Some(v) = h.get_secret(key).filter(|v| !v.is_empty()) {
        return Some(v);
    }
    let hash = |b: &[u8]| contract_keys::code_hash(b).to_vec();
    Some(match key {
        k if k == identity::UPKEEP_BAG => BAG.to_vec(),
        k if k == identity::UPKEEP_TAIL => TAIL.to_vec(),
        k if k == identity::UPKEEP_SEALED => SEALED.to_vec(),
        k if k == identity::UPKEEP_PIECE => PIECE.to_vec(),
        k if k == identity::UPKEEP_IDLOG => hash(IDLOG),
        k if k == identity::UPKEEP_BLOCK => hash(BLOCK),
        k if k == identity::UPKEEP_DEVICES => hash(DEVICES),
        _ => return None,
    })
}
