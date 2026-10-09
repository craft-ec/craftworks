//! STANDING (TOKEN §7a): what a node PROVES it holds where Freenet placed it — the base of a witness's weight.
//!
//! - **The ring, as the core computes it.** A node's location is freenet-core's own (`ring/location.rs`): a hash of
//!   the IPv4 /24 (IPv6 /48) its peers see — never chosen by the node. A contract's location is its id read as a
//!   fraction of the ring. Both as `u64` positions (location × 2⁶⁴), exact.
//! - **Located by its peers.** A node cannot sign its own address; the peers it is connected to SEE it, and each says
//!   "peer P is at ring position R" ([`Attest`]). A node key says "I run peer P" ([`Bind`]).
//! - **Its ARC.** What the core makes a node responsible for: from the midpoint with its nearest attested peer below
//!   to the midpoint with its nearest one above. Only contracts in that arc count — data a node keeps far from its
//!   place earns nothing.
//! - **Holdings, answered.** Each epoch a node commits to the contracts it holds in its arc ([`Holdings`]: a merkle
//!   root over their sorted ids); at each beacon height the beacon's output picks one ([`challenge`]) and the node
//!   answers `blake3(state ‖ output ‖ peer)` ([`Answer`]) — which anyone holding that contract checks.

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use std::net::{IpAddr, SocketAddr};

pub type Why = &'static str;

/// Beacon heights per epoch: a node's location, binding and holdings are each said once per epoch.
pub const EPOCH: u64 = 1024;
/// The longest peer id the node names (base58 of 12 bytes of its transport key is 16–17 characters).
pub const PEER_MAX: usize = 32;

pub fn epoch(height: u64) -> u64 {
    height / EPOCH
}

// ---- the ring, as freenet-core computes it ----

/// freenet-core's `distribute_hash` (ring/location.rs), exactly.
fn distribute_hash(x: u64) -> u64 {
    let mut h = x;
    h = h.wrapping_mul(0x517cc1b727220a95);
    h ^= h >> 32;
    h = h.wrapping_mul(0x4cf5ad432745937f);
    h ^= h >> 28;
    h = h.wrapping_mul(0x2f38a814cad5c4ed);
    h ^= h >> 31;
    h
}

/// A node's ring position from the address its peers see — freenet-core's `Location::from_address`: the IPv4 /24
/// (IPv6 /48) hashed; a loopback address also by its port (the core's simulation peers).
pub fn ring_of_addr(a: &SocketAddr) -> u64 {
    let x = match a.ip() {
        IpAddr::V4(v) => {
            let masked = (u32::from(v) & 0xFFFF_FF00) as u64;
            if v.is_loopback() {
                masked | ((a.port() as u64) << 32)
            } else {
                masked
            }
        }
        IpAddr::V6(v) => {
            let s = v.segments();
            let c = (u64::from(s[0]) << 32) | (u64::from(s[1]) << 16) | u64::from(s[2]);
            if v.is_loopback() || c == 0 {
                c | ((a.port() as u64) << 48)
            } else {
                c
            }
        }
    };
    distribute_hash(x)
}

/// A contract's ring position: its id read as a fraction of the ring (freenet-core's `from_contract_key`).
pub fn ring_of_contract(id: &[u8; 32]) -> u64 {
    u64::from_be_bytes(id[..8].try_into().unwrap())
}

/// A ring position as the node writes it, in millionths (its six decimals).
pub fn millionths(r: u64) -> u32 {
    ((r as u128 * 1_000_000 + (1u128 << 63)) >> 64) as u32
}

/// A node's ARC: `(start, length)` going up the ring — from the midpoint with its nearest peer below to the midpoint
/// with its nearest peer above. `None` with no peer elsewhere on the ring.
pub fn arc(own: u64, peers: &[u64]) -> Option<(u64, u64)> {
    let above = peers.iter().map(|p| p.wrapping_sub(own)).filter(|d| *d > 0).min()?;
    let below = peers.iter().map(|p| own.wrapping_sub(*p)).filter(|d| *d > 0).min()?;
    let (b, a) = (below / 2, above / 2);
    Some((own.wrapping_sub(b), b + a))
}

pub fn in_arc(x: u64, (start, len): (u64, u64)) -> bool {
    x.wrapping_sub(start) < len
}

// ---- signatures ----

fn sig_ok(key: &[u8; 32], message: &[u8], sig: &[u8; 64]) -> bool {
    VerifyingKey::from_bytes(key).is_ok_and(|k| k.verify_strict(message, &Signature::from_bytes(sig)).is_ok())
}
fn sign(key: &SigningKey, message: &[u8]) -> [u8; 64] {
    key.sign(message).to_bytes()
}
fn put_peer(v: &mut Vec<u8>, peer: &str) {
    v.push(peer.len() as u8);
    v.extend_from_slice(peer.as_bytes());
}
fn peer_ok(peer: &str) -> bool {
    !peer.is_empty() && peer.len() <= PEER_MAX
}

/// "Peer `peer` is at ring position `ring`" — said in epoch `epoch` by node key `by`, which is connected to it and
/// computed `ring` from the address it sees ([`ring_of_addr`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attest {
    pub by: [u8; 32],
    pub peer: String,
    pub ring: u64,
    pub epoch: u64,
    pub sig: [u8; 64],
}

impl Attest {
    pub fn message(by: &[u8; 32], peer: &str, ring: u64, epoch: u64) -> Vec<u8> {
        let mut v = b"craftworks standing attest".to_vec();
        v.extend_from_slice(by);
        put_peer(&mut v, peer);
        v.extend_from_slice(&ring.to_le_bytes());
        v.extend_from_slice(&epoch.to_le_bytes());
        v
    }
    pub fn new(key: &SigningKey, peer: &str, addr: &SocketAddr, epoch: u64) -> Attest {
        let by = key.verifying_key().to_bytes();
        let ring = ring_of_addr(addr);
        Attest { by, peer: peer.into(), ring, epoch, sig: sign(key, &Self::message(&by, peer, ring, epoch)) }
    }
    pub fn holds(&self) -> bool {
        peer_ok(&self.peer) && sig_ok(&self.by, &Self::message(&self.by, &self.peer, self.ring, self.epoch), &self.sig)
    }
}

/// "I, node key `key`, run peer `peer`" — in epoch `epoch`. (A transport key cannot sign, so this cannot be proven:
/// two keys claiming one peer share it.)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bind {
    pub key: [u8; 32],
    pub peer: String,
    pub epoch: u64,
    pub sig: [u8; 64],
}

impl Bind {
    pub fn message(key: &[u8; 32], peer: &str, epoch: u64) -> Vec<u8> {
        let mut v = b"craftworks standing bind".to_vec();
        v.extend_from_slice(key);
        put_peer(&mut v, peer);
        v.extend_from_slice(&epoch.to_le_bytes());
        v
    }
    pub fn new(key: &SigningKey, peer: &str, epoch: u64) -> Bind {
        let k = key.verifying_key().to_bytes();
        Bind { key: k, peer: peer.into(), epoch, sig: sign(key, &Self::message(&k, peer, epoch)) }
    }
    pub fn holds(&self) -> bool {
        peer_ok(&self.peer) && sig_ok(&self.key, &Self::message(&self.key, &self.peer, self.epoch), &self.sig)
    }
}

/// Where peer `peer` is, in `epoch`: the ring position at least `quorum` DISTINCT attesters agree on — none of them a
/// key bound to the peer itself (it cannot vouch for its own place). `None` short of a quorum.
pub fn located(peer: &str, epoch: u64, attests: &[Attest], own_keys: &[[u8; 32]], quorum: usize) -> Option<u64> {
    let mut by_ring: Vec<(u64, Vec<[u8; 32]>)> = Vec::new();
    for a in attests.iter().filter(|a| a.peer == peer && a.epoch == epoch && !own_keys.contains(&a.by) && a.holds()) {
        match by_ring.iter_mut().find(|(r, _)| *r == a.ring) {
            Some((_, by)) if !by.contains(&a.by) => by.push(a.by),
            Some(_) => {}
            None => by_ring.push((a.ring, vec![a.by])),
        }
    }
    by_ring.into_iter().filter(|(_, by)| by.len() >= quorum).max_by_key(|(_, by)| by.len()).map(|(r, _)| r)
}

// ---- holdings: a merkle tree over sorted ids ----

fn leaf(id: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-09 standing leaf");
    h.update(id);
    *h.finalize().as_bytes()
}
fn node(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-09 standing node");
    h.update(a);
    h.update(b);
    *h.finalize().as_bytes()
}
fn level(xs: &[[u8; 32]]) -> Vec<[u8; 32]> {
    xs.chunks(2).map(|c| if c.len() == 2 { node(&c[0], &c[1]) } else { c[0] }).collect()
}

/// The root over `ids` (sorted, distinct): what [`Holdings`] commits to.
pub fn root(ids: &[[u8; 32]]) -> [u8; 32] {
    let mut xs: Vec<[u8; 32]> = ids.iter().map(leaf).collect();
    if xs.is_empty() {
        return [0; 32];
    }
    while xs.len() > 1 {
        xs = level(&xs);
    }
    xs[0]
}

/// The path proving `ids[i]` under [`root`]: the sibling at each level (none where a level's last node has none).
pub fn path(ids: &[[u8; 32]], mut i: usize) -> Vec<Option<[u8; 32]>> {
    let mut xs: Vec<[u8; 32]> = ids.iter().map(leaf).collect();
    let mut out = Vec::new();
    while xs.len() > 1 {
        let j = i ^ 1;
        out.push(xs.get(j).copied());
        xs = level(&xs);
        i /= 2;
    }
    out
}

/// `id` is entry `i` of `count` under `root`.
pub fn verify(root_: &[u8; 32], count: u32, mut i: usize, id: &[u8; 32], path: &[Option<[u8; 32]>]) -> bool {
    if i >= count as usize {
        return false;
    }
    let mut x = leaf(id);
    let mut n = count as usize;
    for p in path {
        if n <= 1 {
            return false;
        }
        x = match (i % 2, p) {
            (0, Some(s)) => node(&x, s),
            (1, Some(s)) => node(s, &x),
            // The last node of an odd level rises alone.
            (0, None) if i == n - 1 => x,
            _ => return false,
        };
        i /= 2;
        n = n.div_ceil(2);
    }
    n == 1 && &x == root_
}

/// A node's HOLDINGS in an epoch: the contracts in its arc, committed — `root` over their sorted ids, `count` of
/// them, `bytes` their states' total — signed by its bound key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holdings {
    pub key: [u8; 32],
    pub peer: String,
    pub epoch: u64,
    pub root: [u8; 32],
    pub count: u32,
    pub bytes: u64,
    pub sig: [u8; 64],
}

impl Holdings {
    pub fn message(key: &[u8; 32], peer: &str, epoch: u64, root_: &[u8; 32], count: u32, bytes: u64) -> Vec<u8> {
        let mut v = b"craftworks standing holdings".to_vec();
        v.extend_from_slice(key);
        put_peer(&mut v, peer);
        v.extend_from_slice(&epoch.to_le_bytes());
        v.extend_from_slice(root_);
        v.extend_from_slice(&count.to_le_bytes());
        v.extend_from_slice(&bytes.to_le_bytes());
        v
    }
    /// The holdings of `hosted` (id, size) — only those in `arc`.
    pub fn new(key: &SigningKey, peer: &str, epoch: u64, hosted: &[([u8; 32], u64)], arc_: (u64, u64)) -> (Holdings, Vec<[u8; 32]>) {
        let mut mine: Vec<([u8; 32], u64)> = hosted.iter().copied().filter(|(id, _)| in_arc(ring_of_contract(id), arc_)).collect();
        mine.sort();
        mine.dedup_by_key(|x| x.0);
        let ids: Vec<[u8; 32]> = mine.iter().map(|x| x.0).collect();
        let (r, count, bytes) = (root(&ids), ids.len() as u32, mine.iter().map(|x| x.1).sum());
        let k = key.verifying_key().to_bytes();
        (Holdings { key: k, peer: peer.into(), epoch, root: r, count, bytes, sig: sign(key, &Self::message(&k, peer, epoch, &r, count, bytes)) }, ids)
    }
    pub fn holds(&self) -> bool {
        peer_ok(&self.peer) && sig_ok(&self.key, &Self::message(&self.key, &self.peer, self.epoch, &self.root, self.count, self.bytes), &self.sig)
    }
}

/// Which of `count` holdings the beacon's output `out` at a height asks `peer` for.
pub fn challenge(out: &[u8; 32], peer: &str, count: u32) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-09 standing challenge");
    h.update(out);
    h.update(peer.as_bytes());
    Some((u64::from_le_bytes(h.finalize().as_bytes()[..8].try_into().unwrap()) % count as u64) as usize)
}

/// The answer: the contract's state, bound to that beacon output and to this peer (no node's answer is another's).
pub fn answer_hash(state: &[u8], out: &[u8; 32], peer: &str) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-09 standing answer");
    h.update(out);
    h.update(peer.as_bytes());
    h.update(state);
    *h.finalize().as_bytes()
}

/// A node's ANSWER at beacon height `height`: the challenged entry of its holdings, the path proving it, its hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub key: [u8; 32],
    pub peer: String,
    pub height: u64,
    pub index: u32,
    pub id: [u8; 32],
    pub path: Vec<Option<[u8; 32]>>,
    pub hash: [u8; 32],
    pub sig: [u8; 64],
}

impl Answer {
    pub fn message(&self) -> Vec<u8> {
        let mut v = b"craftworks standing answer".to_vec();
        v.extend_from_slice(&self.key);
        put_peer(&mut v, &self.peer);
        v.extend_from_slice(&self.height.to_le_bytes());
        v.extend_from_slice(&self.index.to_le_bytes());
        v.extend_from_slice(&self.id);
        for p in &self.path {
            match p {
                None => v.push(0),
                Some(s) => {
                    v.push(1);
                    v.extend_from_slice(s);
                }
            }
        }
        v.extend_from_slice(&self.hash);
        v
    }
    /// The answer to the beacon's `out` at `height`: `None` if nothing is held, or the challenged state is not here.
    pub fn new(key: &SigningKey, h: &Holdings, ids: &[[u8; 32]], height: u64, out: &[u8; 32], state: &dyn Fn(&[u8; 32]) -> Option<Vec<u8>>) -> Option<Answer> {
        let i = challenge(out, &h.peer, h.count)?;
        let id = *ids.get(i)?;
        let s = state(&id)?;
        let mut a = Answer { key: key.verifying_key().to_bytes(), peer: h.peer.clone(), height, index: i as u32, id, path: path(ids, i), hash: answer_hash(&s, out, &h.peer), sig: [0; 64] };
        a.sig = sign(key, &a.message());
        Some(a)
    }
}

/// CHECK an answer against the holdings it answers for, the beacon's output at its height and the node's arc: signed,
/// the entry the beacon asked for, under the committed root, in the arc — and, given the contract's state (whoever
/// holds it), the right hash. `state`: `None` checks everything but the hash.
pub fn check(h: &Holdings, a: &Answer, out: &[u8; 32], arc_: (u64, u64), state: Option<&[u8]>) -> Result<(), Why> {
    if !h.holds() {
        return Err("holdings not signed by their key");
    }
    if a.key != h.key || a.peer != h.peer || epoch(a.height) != h.epoch {
        return Err("an answer for other holdings");
    }
    if !sig_ok(&a.key, &a.message(), &a.sig) {
        return Err("an answer not signed by its key");
    }
    if challenge(out, &h.peer, h.count) != Some(a.index as usize) {
        return Err("an answer to another challenge");
    }
    if !verify(&h.root, h.count, a.index as usize, &a.id, &a.path) {
        return Err("an answer not under its holdings");
    }
    if !in_arc(ring_of_contract(&a.id), arc_) {
        return Err("a holding outside the node's arc");
    }
    if state.is_some_and(|s| answer_hash(s, out, &h.peer) != a.hash) {
        return Err("an answer that is not the contract's state");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
