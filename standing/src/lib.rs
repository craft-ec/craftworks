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

/// "Peer `peer` is at ring position `ring`, and I am at `at`" — said in epoch `epoch` by node key `by`, which is
/// connected to it (so a ring NEIGHBOUR of it: what its arc is made from) and computed `ring` from the address it sees
/// ([`ring_of_addr`]); `at` is its own place as its node says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attest {
    pub by: [u8; 32],
    pub peer: String,
    pub ring: u64,
    pub at: u64,
    pub epoch: u64,
    pub sig: [u8; 64],
}

impl Attest {
    pub fn message(by: &[u8; 32], peer: &str, ring: u64, at: u64, epoch: u64) -> Vec<u8> {
        let mut v = [craftworks_identity::STANDING_DOMAIN, b"attest"].concat();
        v.extend_from_slice(by);
        put_peer(&mut v, peer);
        v.extend_from_slice(&ring.to_le_bytes());
        v.extend_from_slice(&at.to_le_bytes());
        v.extend_from_slice(&epoch.to_le_bytes());
        v
    }
    pub fn new(key: &SigningKey, peer: &str, addr: &SocketAddr, at: u64, epoch: u64) -> Attest {
        let by = key.verifying_key().to_bytes();
        let ring = ring_of_addr(addr);
        Attest { by, peer: peer.into(), ring, at, epoch, sig: sign(key, &Self::message(&by, peer, ring, at, epoch)) }
    }
    pub fn holds(&self) -> bool {
        peer_ok(&self.peer) && sig_ok(&self.by, &Self::message(&self.by, &self.peer, self.ring, self.at, self.epoch), &self.sig)
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
        let mut v = [craftworks_identity::STANDING_DOMAIN, b"bind"].concat();
        v.extend_from_slice(key);
        put_peer(&mut v, peer);
        v.extend_from_slice(&epoch.to_le_bytes());
        v
    }
    pub fn new(key: &SigningKey, peer: &str, epoch: u64) -> Bind {
        let mut b = Self::draft(key.verifying_key().to_bytes(), peer, epoch);
        b.sig = sign(key, &b.signed());
        b
    }
    /// UNSIGNED, for a key held elsewhere (a node's member key, in its identity delegate): sign [`Bind::signed`], set `sig`.
    pub fn draft(key: [u8; 32], peer: &str, epoch: u64) -> Bind {
        Bind { key, peer: peer.into(), epoch, sig: [0; 64] }
    }
    /// The message its key signs.
    pub fn signed(&self) -> Vec<u8> {
        Self::message(&self.key, &self.peer, self.epoch)
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

/// Where peer `peer` is in `epoch` and its ARC: its [`located`] place, and the arc between it and the places its
/// attesters (its connected peers: its ring neighbours) say they are. `None` short of a quorum.
pub fn place(peer: &str, epoch: u64, attests: &[Attest], own_keys: &[[u8; 32]], quorum: usize) -> Option<(u64, (u64, u64))> {
    let ring = located(peer, epoch, attests, own_keys, quorum)?;
    let neighbours: Vec<u64> = attests.iter().filter(|a| a.peer == peer && a.epoch == epoch && a.ring == ring && !own_keys.contains(&a.by) && a.holds()).map(|a| a.at).collect();
    Some((ring, arc(ring, &neighbours)?))
}

/// One height in [`SAMPLE`] is a peer's to answer (picked by the beacon's output there and the peer): the answers kept
/// (an answer is [`PICKS`] proofs — ~9 KB — so ~8 an epoch).
pub const SAMPLE: u64 = 128;
pub fn sampled(out: &[u8; 32], peer: &str) -> bool {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-09 standing sample");
    h.update(out);
    h.update(peer.as_bytes());
    u64::from_le_bytes(h.finalize().as_bytes()[..8].try_into().unwrap()) % SAMPLE == 0
}

// ---- holdings: a FIXED LIST's positional proof — the SDK's one (`freenet_prolly::list`: a prolly tree is for keyed
// maps; a fixed list sampled by position needs a ~32 B/level path, its prolly proof measured 20–40× larger) ----

pub use freenet_prolly::list::{path, root, verify};

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
        let mut v = [craftworks_identity::STANDING_DOMAIN, b"holdings"].concat();
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
        let (mut h, ids) = Self::draft(key.verifying_key().to_bytes(), peer, epoch, hosted, arc_);
        h.sig = sign(key, &h.signed());
        (h, ids)
    }
    /// UNSIGNED (the key held elsewhere): sign [`Holdings::signed`], set `sig`.
    pub fn draft(key: [u8; 32], peer: &str, epoch: u64, hosted: &[([u8; 32], u64)], arc_: (u64, u64)) -> (Holdings, Vec<[u8; 32]>) {
        let mut mine: Vec<([u8; 32], u64)> = hosted.iter().copied().filter(|(id, _)| in_arc(ring_of_contract(id), arc_)).collect();
        mine.sort();
        mine.dedup_by_key(|x| x.0);
        let ids: Vec<[u8; 32]> = mine.iter().map(|x| x.0).collect();
        let (r, count, bytes) = (root(&ids), ids.len() as u32, mine.iter().map(|x| x.1).sum());
        (Holdings { key, peer: peer.into(), epoch, root: r, count, bytes, sig: [0; 64] }, ids)
    }
    /// The message its key signs.
    pub fn signed(&self) -> Vec<u8> {
        Self::message(&self.key, &self.peer, self.epoch, &self.root, self.count, self.bytes)
    }
    pub fn holds(&self) -> bool {
        peer_ok(&self.peer) && sig_ok(&self.key, &Self::message(&self.key, &self.peer, self.epoch, &self.root, self.count, self.bytes), &self.sig)
    }
}

/// The SEAL's work: hash rounds per 32-byte block of a piece, each block's chain continuing the last (sequential —
/// no block can be made before the one before it). ~2-4 s for a 256 KiB piece on one core.
pub const ROUNDS: u32 = 512;

/// A SEALED REPLICA of contract `id`'s state `piece`, unique to `key` (the holdings key): the piece XOR a keystream
/// from a sequential hash chain over (key, id). The same call unseals.
pub fn seal(key: &[u8; 32], id: &[u8; 32], piece: &[u8]) -> Vec<u8> {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-09 standing seal");
    h.update(key);
    h.update(id);
    let mut st = *h.finalize().as_bytes();
    let mut out = piece.to_vec();
    for block in out.chunks_mut(32) {
        for _ in 0..ROUNDS {
            st = *blake3::hash(&st).as_bytes();
        }
        for (b, k) in block.iter_mut().zip(st) {
            *b ^= k;
        }
    }
    out
}

/// A pick's ANSWER HASH from its piece: sealed for `key` ([`seal`]: the slow part), then hashed with the beacon's
/// output and the peer ([`answer_hash`]).
pub fn sealed_hash(key: &[u8; 32], id: &[u8; 32], piece: &[u8], out: &[u8; 32], peer: &str) -> [u8; 32] {
    answer_hash(&seal(key, id, piece), out, peer)
}

/// Pieces a sampled height challenges at once: re-sealing them all takes far longer than the beacon step.
pub const PICKS: usize = 16;

/// The `PICKS` entries of `count` holdings the beacon's output `out` asks `peer` for (with repeats on a small set).
pub fn challenge(out: &[u8; 32], peer: &str, count: u32) -> Option<Vec<usize>> {
    if count == 0 {
        return None;
    }
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-09 standing challenge");
    h.update(out);
    h.update(peer.as_bytes());
    let mut x = h.finalize_xof();
    Some((0..PICKS).map(|_| {
        let mut b = [0u8; 8];
        x.fill(&mut b);
        (u64::from_le_bytes(b) % count as u64) as usize
    }).collect())
}

/// The answer for one pick: the SEALED REPLICA, bound to that beacon output and this peer.
pub fn answer_hash(replica: &[u8], out: &[u8; 32], peer: &str) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-09 standing answer");
    h.update(out);
    h.update(peer.as_bytes());
    h.update(replica);
    *h.finalize().as_bytes()
}

/// One challenged entry, answered: its place in the holdings, its id, the path proving it, the replica's hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub index: u32,
    pub id: [u8; 32],
    pub path: Vec<Option<[u8; 32]>>,
    pub hash: [u8; 32],
}

/// A node's ANSWER at beacon height `height`: each challenged entry of its holdings, answered from its replicas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub key: [u8; 32],
    pub peer: String,
    pub height: u64,
    pub picks: Vec<Pick>,
    pub sig: [u8; 64],
}

impl Answer {
    pub fn message(&self) -> Vec<u8> {
        let mut v = [craftworks_identity::STANDING_DOMAIN, b"answer"].concat();
        v.extend_from_slice(&self.key);
        put_peer(&mut v, &self.peer);
        v.extend_from_slice(&self.height.to_le_bytes());
        for p in &self.picks {
            v.extend_from_slice(&p.index.to_le_bytes());
            v.extend_from_slice(&p.id);
            v.push(p.path.len() as u8);
            for s in &p.path {
                match s {
                    None => v.push(0),
                    Some(s) => {
                        v.push(1);
                        v.extend_from_slice(s);
                    }
                }
            }
            v.extend_from_slice(&p.hash);
        }
        v
    }
    /// The answer to the beacon's `out` at `height`, from the node's REPLICAS (`replica`: a sealed copy by id): `None` if
    /// nothing is held, or a challenged replica is not here.
    pub fn new(key: &SigningKey, h: &Holdings, ids: &[[u8; 32]], height: u64, out: &[u8; 32], replica: &dyn Fn(&[u8; 32]) -> Option<Vec<u8>>) -> Option<Answer> {
        let mut a = Self::draft(h, ids, height, out, replica)?;
        a.sig = sign(key, &a.message());
        Some(a)
    }
    /// UNSIGNED (the key held elsewhere: `h`'s): sign [`Answer::message`], set `sig`.
    pub fn draft(h: &Holdings, ids: &[[u8; 32]], height: u64, out: &[u8; 32], replica: &dyn Fn(&[u8; 32]) -> Option<Vec<u8>>) -> Option<Answer> {
        Self::draft_hashed(h, ids, height, out, &|id| replica(id).map(|r| answer_hash(&r, out, &h.peer)))
    }
    /// [`Answer::draft`] from each asked piece's REPLICA HASH (`hash_of`: [`sealed_hash`], computed elsewhere — a page
    /// seals in a worker, off its thread).
    pub fn draft_hashed(h: &Holdings, ids: &[[u8; 32]], height: u64, out: &[u8; 32], hash_of: &dyn Fn(&[u8; 32]) -> Option<[u8; 32]>) -> Option<Answer> {
        let picks = challenge(out, &h.peer, h.count)?
            .into_iter()
            .map(|i| {
                let id = *ids.get(i)?;
                Some(Pick { index: i as u32, id, path: path(ids, i), hash: hash_of(&id)? })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Answer { key: h.key, peer: h.peer.clone(), height, picks, sig: [0; 64] })
    }
    /// The pieces an answer at `out` asks for (its picks' ids) — what to read before drafting it.
    pub fn asked(h: &Holdings, ids: &[[u8; 32]], out: &[u8; 32]) -> Vec<[u8; 32]> {
        challenge(out, &h.peer, h.count).map_or(vec![], |is| is.into_iter().filter_map(|i| ids.get(i).copied()).collect())
    }
}

/// CHECK an answer against the holdings it answers for and the beacon's output at its height: signed, every pick the
/// one the beacon asked for and under the committed root, in the node's arc if one is given (location: a soft rule) —
/// and, given one pick's PIECE (`piece`: the pick and the contract's state, fetched from the network by whoever
/// checks), its replica's hash: the verifier seals it itself. `piece: None` checks everything but that hash.
pub fn check(h: &Holdings, a: &Answer, out: &[u8; 32], arc_: Option<(u64, u64)>, piece: Option<(usize, &[u8])>) -> Result<(), Why> {
    if !h.holds() {
        return Err("holdings not signed by their key");
    }
    if a.key != h.key || a.peer != h.peer || epoch(a.height) != h.epoch {
        return Err("an answer for other holdings");
    }
    if !sig_ok(&a.key, &a.message(), &a.sig) {
        return Err("an answer not signed by its key");
    }
    let asked = challenge(out, &h.peer, h.count).ok_or("an answer for empty holdings")?;
    if a.picks.len() != asked.len() || a.picks.iter().zip(&asked).any(|(p, i)| p.index as usize != *i) {
        return Err("an answer to another challenge");
    }
    for p in &a.picks {
        if !verify(&h.root, h.count, p.index as usize, &p.id, &p.path) {
            return Err("an answer not under its holdings");
        }
        if arc_.is_some_and(|arc_| !in_arc(ring_of_contract(&p.id), arc_)) {
            return Err("a holding outside the node's arc");
        }
    }
    if let Some((j, state)) = piece {
        let p = a.picks.get(j).ok_or("no such pick")?;
        if answer_hash(&seal(&h.key, &p.id, state), out, &h.peer) != p.hash {
            return Err("an answer that is not the piece's replica");
        }
    }
    Ok(())
}

pub mod record;
pub mod codec;

#[cfg(test)]
mod tests;
