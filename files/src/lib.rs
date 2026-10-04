//! FILES (ARCHITECTURE §6): a file as encrypted chunks, RLNC-coded over GF(2⁸) in generations of 16, every fragment and
//! index piece a `sealed` contract state at an address only the file's key gives, the index a tree whose root's hash
//! the reference carries. Pure: no I/O, no clock, no randomness (a file's key comes from its content: deduplication,
//! and an upload that resumes makes the same fragments at the same addresses).
//!
//!   let plan = Plan::of(size);
//!   let key = content_key(&content_hash, salt);                       // salt: the space's, or None (public)
//!   for g in 0..plan.gens { encode(&key, &plan, g, plain_of(g), EXTRA) }  // (j, Piece) per fragment
//!   let (pieces, root) = index(&key, &plan, &stored);                 // the index tree, the root's hash
//!   Root::open(&key, &root, state) → Leaf::open(..) → Decoder / read_chunk

mod gf;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

/// Chunks: the file's size ÷ 16 within these bounds (so a file codes about 16 chunks), a multiple of 1 KiB.
pub const MIN_CHUNK: usize = 16 * 1024;
pub const MAX_CHUNK: usize = 256 * 1024;
/// Chunks per generation: any 16 independent fragments rebuild one.
pub const GEN: usize = 16;
/// Fragments sent beyond a generation's chunks at first (more are minted whenever one is not stored).
pub const EXTRA: usize = 8;
/// The most fragments an index lists for one generation.
pub const MAX_LISTED: usize = 32;
/// Files this small are not coded: they ride inline in their item.
pub const INLINE_MAX: usize = 64 * 1024;
const TAG: usize = 16;
/// A stored state: `FORMAT ‖ kind ‖ …`; the `sealed` contract checks only the length.
const FORMAT: u8 = 1;
const KIND_FRAGMENT: u8 = 3;
/// An index piece ADDRESSED BY ITS HASH (its nonce carried in it): two indexes of one file (two uploads that stored
/// different fragments) never share an address — at a shared one the network keeps the first, and the second reads as
/// forged.
const KIND_INDEX_H: u8 = 5;
const MAGIC: &[u8; 4] = b"CWF1";
/// The codec an index names: RLNC over GF(2⁸), fragments checked by the index's hash list.
pub const CODEC: u8 = 1;
/// Generations one leaf of the index lists, and children one inner piece lists (both keep a piece under 256 KiB).
pub const GENS_PER_LEAF: usize = 192;
pub const FANOUT: usize = 7000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A piece that is not what its hash (or its key) says.
    Forged,
    /// Bytes that do not read as what they claim to be.
    Malformed,
    /// A fragment whose coefficients add nothing (already spanned).
    Redundant,
}

/// How a file of `size` bytes is cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub size: u64,
    pub chunk: usize,
    pub chunks: u64,
    pub gens: u64,
}

impl Plan {
    pub fn of(size: u64) -> Plan {
        let per = size.div_ceil(GEN as u64).div_ceil(1024) * 1024;
        let chunk = (per as usize).clamp(MIN_CHUNK, MAX_CHUNK);
        let chunks = size.div_ceil(chunk as u64).max(1);
        Plan {
            size,
            chunk,
            chunks,
            gens: chunks.div_ceil(GEN as u64),
        }
    }
    /// Chunks in generation `g` (16, the last maybe fewer).
    pub fn k(&self, g: u64) -> usize {
        (self.chunks - g * GEN as u64).min(GEN as u64) as usize
    }
    /// The byte range of generation `g`.
    pub fn range(&self, g: u64) -> (u64, u64) {
        let start = g * (GEN * self.chunk) as u64;
        (start, (start + (GEN * self.chunk) as u64).min(self.size))
    }
    /// A fragment's payload: a chunk's ciphertext (chunk + tag), zero-padded to this.
    fn symbol(&self) -> usize {
        self.chunk + TAG
    }
    /// The plaintext length of chunk `i`.
    fn chunk_len(&self, i: u64) -> usize {
        (self.size - i * self.chunk as u64).min(self.chunk as u64) as usize
    }
}

/// A piece to PUT: a `sealed` contract's params (the address) and state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub address: [u8; 32],
    pub state: Vec<u8>,
}

impl Piece {
    pub fn hash(&self) -> [u8; 32] {
        *blake3::hash(&self.state).as_bytes()
    }
}

/// A file's KEY, from its content (its BLAKE3 hash): a public file's from the content alone (the whole network dedups
/// it), any other's with its space's dedup salt (members dedup; nobody outside can test for a file).
pub fn content_key(content: &[u8; 32], salt: Option<&[u8; 32]>) -> [u8; 32] {
    match salt {
        None => blake3::derive_key("craftworks files public key", content),
        Some(s) => blake3::keyed_hash(
            &blake3::derive_key("craftworks files space key", s),
            content,
        )
        .into(),
    }
}

fn sub(key: &[u8; 32], what: &str) -> [u8; 32] {
    blake3::derive_key(what, key)
}
fn nonce(key: &[u8; 32], what: &[u8]) -> [u8; 24] {
    let mut n = [0u8; 24];
    n.copy_from_slice(
        &blake3::keyed_hash(&sub(key, "craftworks files nonce"), what).as_bytes()[..24],
    );
    n
}

/// Where fragment `j` of generation `g` lives.
pub fn fragment_address(key: &[u8; 32], g: u64, j: u8) -> [u8; 32] {
    let place = [&b"f"[..], &g.to_be_bytes(), &[j]].concat();
    blake3::keyed_hash(&sub(key, "craftworks files address"), &place).into()
}
/// Where an index piece (the root too) lives: by its hash, which its parent (the reference, for the root) carries —
/// two uploads of one file that stored other fragments never meet at one address.
pub fn hashed_address(key: &[u8; 32], hash: &[u8; 32]) -> [u8; 32] {
    blake3::keyed_hash(
        &sub(key, "craftworks files address"),
        &[&b"h"[..], hash].concat(),
    )
    .into()
}

fn aead(key: &[u8; 32], what: &str) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new((&sub(key, what)).into())
}

/// Chunk `i`'s ciphertext, zero-padded to the plan's symbol.
fn seal_chunk(key: &[u8; 32], plan: &Plan, i: u64, plain: &[u8]) -> Vec<u8> {
    let n = nonce(key, &[&b"c"[..], &i.to_be_bytes()].concat());
    let mut ct = aead(key, "craftworks files chunk key")
        .encrypt(
            XNonce::from_slice(&n),
            Payload {
                msg: plain,
                aad: &plan.size.to_be_bytes(),
            },
        )
        .expect("sealing cannot fail");
    ct.resize(plan.symbol(), 0);
    ct
}
fn open_chunk(key: &[u8; 32], plan: &Plan, i: u64, symbol: &[u8]) -> Result<Vec<u8>, Error> {
    let len = plan.chunk_len(i) + TAG;
    let n = nonce(key, &[&b"c"[..], &i.to_be_bytes()].concat());
    aead(key, "craftworks files chunk key")
        .decrypt(
            XNonce::from_slice(&n),
            Payload {
                msg: symbol.get(..len).ok_or(Error::Malformed)?,
                aad: &plan.size.to_be_bytes(),
            },
        )
        .map_err(|_| Error::Forged)
}

/// Fragment `j`'s coefficients in a generation of `k` chunks: the unit vector for `j < k` (systematic), else drawn
/// from the key (never all zero).
fn coefficients(key: &[u8; 32], g: u64, j: u8, k: usize) -> Vec<u8> {
    if (j as usize) < k {
        let mut c = vec![0u8; k];
        c[j as usize] = 1;
        return c;
    }
    let mut out = vec![0u8; k];
    let mut h = blake3::Hasher::new_keyed(&sub(key, "craftworks files coefficients"));
    h.update(&g.to_be_bytes()).update(&[j]);
    h.finalize_xof().fill(&mut out);
    if out.iter().all(|c| *c == 0) {
        out[0] = 1;
    }
    out
}

fn fragment_state(k: usize, coeffs: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut s = Vec::with_capacity(3 + k + payload.len());
    s.extend_from_slice(&[FORMAT, KIND_FRAGMENT, k as u8]);
    s.extend_from_slice(coeffs);
    s.extend_from_slice(payload);
    s
}
fn parse_fragment<'a>(
    plan: &Plan,
    k: usize,
    state: &'a [u8],
) -> Result<(&'a [u8], &'a [u8]), Error> {
    if state.len() != 3 + k + plan.symbol()
        || state[..2] != [FORMAT, KIND_FRAGMENT]
        || state[2] as usize != k
    {
        return Err(Error::Malformed);
    }
    Ok((&state[3..3 + k], &state[3 + k..]))
}

/// GENERATION `g` coded: `plain` its bytes (the plan's range), fragments `0 .. k + extra` (the first `k` its chunks).
pub fn encode(key: &[u8; 32], plan: &Plan, g: u64, plain: &[u8], extra: usize) -> Vec<(u8, Piece)> {
    let symbols = seal_generation(key, plan, g, plain);
    let k = symbols.len();
    (0..(k + extra).min(MAX_LISTED) as u8)
        .map(|j| (j, fragment(key, plan, g, j, &symbols)))
        .collect()
}
/// One more fragment `j` of generation `g` (minted: a slow or refused one replaced), from its chunks' ciphertext.
pub fn mint(key: &[u8; 32], plan: &Plan, g: u64, plain: &[u8], j: u8) -> Piece {
    fragment(key, plan, g, j, &seal_generation(key, plan, g, plain))
}
fn seal_generation(key: &[u8; 32], plan: &Plan, g: u64, plain: &[u8]) -> Vec<Vec<u8>> {
    let k = plan.k(g);
    let (start, end) = plan.range(g);
    assert_eq!(
        plain.len() as u64,
        end - start,
        "generation {g} is {} bytes",
        end - start
    );
    (0..k)
        .map(|i| {
            let from = i * plan.chunk;
            seal_chunk(
                key,
                plan,
                g * GEN as u64 + i as u64,
                &plain[from..(from + plan.chunk).min(plain.len())],
            )
        })
        .collect()
}
fn fragment(key: &[u8; 32], plan: &Plan, g: u64, j: u8, symbols: &[Vec<u8>]) -> Piece {
    let k = symbols.len();
    let c = coefficients(key, g, j, k);
    let mut payload = vec![0u8; plan.symbol()];
    for (ci, s) in c.iter().zip(symbols) {
        gf::axpy(&mut payload, s, *ci);
    }
    Piece {
        address: fragment_address(key, g, j),
        state: fragment_state(k, &c, &payload),
    }
}

/// What the index lists for one generation: the fragments stored, each `(j, hash of its state)`.
pub type Listed = Vec<(u8, [u8; 32])>;

/// An index piece: its nonce from its place and its content (deterministic), carried in it.
fn seal_index_h(key: &[u8; 32], place: &[u8], plain: &[u8]) -> Piece {
    let n = nonce(
        key,
        &[&b"h"[..], place, blake3::hash(plain).as_bytes()].concat(),
    );
    let ct = aead(key, "craftworks files index key")
        .encrypt(XNonce::from_slice(&n), plain)
        .expect("sealing cannot fail");
    let mut state = vec![FORMAT, KIND_INDEX_H];
    state.extend_from_slice(&n);
    state.extend_from_slice(&ct);
    Piece {
        address: hashed_address(key, blake3::hash(&state).as_bytes()),
        state,
    }
}
/// An index piece opened: checked against its hash, then opened with the nonce it carries.
fn open_index(key: &[u8; 32], hash: &[u8; 32], state: &[u8]) -> Result<Vec<u8>, Error> {
    if blake3::hash(state).as_bytes() != hash {
        return Err(Error::Forged);
    }
    if state.get(..2) != Some(&[FORMAT, KIND_INDEX_H][..]) {
        return Err(Error::Malformed);
    }
    let n = state.get(2..26).ok_or(Error::Malformed)?;
    aead(key, "craftworks files index key")
        .decrypt(XNonce::from_slice(n), &state[26..])
        .map_err(|_| Error::Forged)
}

/// THE INDEX TREE for a file whose fragments `stored[g]` are stored: its pieces (leaves, inner levels, the root) and
/// the root's hash — the reference's. Each piece at `hashed_address` of its hash. Put it LAST: a file reads once its
/// index is there.
pub fn index_hashed(key: &[u8; 32], plan: &Plan, stored: &[Listed]) -> (Vec<Piece>, [u8; 32]) {
    let seal = |level: u8, n: u64, b: &[u8]| seal_index_h(key, &[&[level][..], &n.to_be_bytes()].concat(), b);
    assert_eq!(stored.len() as u64, plan.gens);
    let mut pieces = Vec::new();
    // Leaves: GENS_PER_LEAF generations each, `n ‖ (j ‖ hash)*n` per generation.
    let mut level: Vec<[u8; 32]> = stored
        .chunks(GENS_PER_LEAF)
        .enumerate()
        .map(|(n, gens)| {
            let mut b = Vec::new();
            for listed in gens {
                assert!(!listed.is_empty() && listed.len() <= MAX_LISTED);
                b.push(listed.len() as u8);
                for (j, h) in listed {
                    b.push(*j);
                    b.extend_from_slice(h);
                }
            }
            let p = seal(0, n as u64, &b);
            let h = p.hash();
            pieces.push(p);
            h
        })
        .collect();
    // Inner levels until the root lists at most FANOUT.
    let mut depth = 0u8;
    while level.len() > FANOUT {
        depth += 1;
        level = level
            .chunks(FANOUT)
            .enumerate()
            .map(|(n, hs)| {
                let p = seal(depth, n as u64, &hs.concat());
                let h = p.hash();
                pieces.push(p);
                h
            })
            .collect();
    }
    let mut root = Vec::new();
    root.extend_from_slice(MAGIC);
    root.push(CODEC);
    root.extend_from_slice(&plan.size.to_be_bytes());
    root.extend_from_slice(&(plan.chunk as u32).to_be_bytes());
    root.push(depth);
    for h in &level {
        root.extend_from_slice(h);
    }
    let p = seal(u8::MAX, 0, &root);
    let h = p.hash();
    pieces.push(p);
    (pieces, h)
}

/// A file's ROOT, opened: its plan, and the hashes of the pieces one level down.
#[derive(Debug, Clone)]
pub struct Root {
    pub plan: Plan,
    pub depth: u8,
    pub children: Vec<[u8; 32]>,
}

impl Root {
    pub fn open(key: &[u8; 32], root_hash: &[u8; 32], state: &[u8]) -> Result<Root, Error> {
        let b = open_index(key, root_hash, state)?;
        if b.len() < 18 || &b[..4] != MAGIC || b[4] != CODEC || (b.len() - 18) % 32 != 0 {
            return Err(Error::Malformed);
        }
        let size = u64::from_be_bytes(b[5..13].try_into().unwrap());
        let chunk = u32::from_be_bytes(b[13..17].try_into().unwrap()) as usize;
        let plan = Plan::of(size);
        if plan.chunk != chunk {
            return Err(Error::Malformed);
        }
        Ok(Root {
            plan,
            depth: b[17],
            children: b[18..].chunks(32).map(|c| c.try_into().unwrap()).collect(),
        })
    }
    /// The leaf that lists generation `g`: its number.
    pub fn leaf_of(g: u64) -> u64 {
        g / GENS_PER_LEAF as u64
    }
    /// The path from the root to leaf `leaf`: `(level, piece number)` per level below the root, top first; the hash of
    /// each comes from its parent (`Inner::open`), the first from `children`.
    pub fn path(&self, leaf: u64) -> Vec<(u8, u64)> {
        (0..=self.depth)
            .rev()
            .map(|l| (l, leaf / (FANOUT as u64).pow(l as u32)))
            .collect()
    }
}

/// An inner piece of the index (levels above the leaves): the hashes of its children.
pub fn open_inner(key: &[u8; 32], hash: &[u8; 32], state: &[u8]) -> Result<Vec<[u8; 32]>, Error> {
    let b = open_index(key, hash, state)?;
    if b.is_empty() || b.len() % 32 != 0 {
        return Err(Error::Malformed);
    }
    Ok(b.chunks(32).map(|c| c.try_into().unwrap()).collect())
}

/// A LEAF of the index: per generation it covers, the fragments stored.
pub fn open_leaf(
    key: &[u8; 32],
    plan: &Plan,
    n: u64,
    hash: &[u8; 32],
    state: &[u8],
) -> Result<Vec<Listed>, Error> {
    let b = open_index(key, hash, state)?;
    let first = n * GENS_PER_LEAF as u64;
    let count = (plan.gens - first).min(GENS_PER_LEAF as u64) as usize;
    let mut out = Vec::with_capacity(count);
    let mut at = 0;
    for _ in 0..count {
        let m = *b.get(at).ok_or(Error::Malformed)? as usize;
        at += 1;
        let mut listed = Vec::with_capacity(m);
        for _ in 0..m {
            let e = b.get(at..at + 33).ok_or(Error::Malformed)?;
            listed.push((e[0], e[1..].try_into().unwrap()));
            at += 33;
        }
        out.push(listed);
    }
    if at != b.len() {
        return Err(Error::Malformed);
    }
    Ok(out)
}

/// ONE CHUNK read alone (a seek): chunk `i` of the file from its systematic fragment, checked against the index.
pub fn read_chunk(
    key: &[u8; 32],
    plan: &Plan,
    listed: &Listed,
    i: u64,
    state: &[u8],
) -> Result<Vec<u8>, Error> {
    let g = i / GEN as u64;
    let j = (i % GEN as u64) as u8;
    let hash = listed
        .iter()
        .find(|(x, _)| *x == j)
        .map(|(_, h)| h)
        .ok_or(Error::Malformed)?;
    if blake3::hash(state).as_bytes() != hash {
        return Err(Error::Forged);
    }
    let (_, payload) = parse_fragment(plan, plan.k(g), state)?;
    open_chunk(key, plan, i, payload)
}

/// A GENERATION being rebuilt: fragments added as they arrive (each checked against the index), done on `k` independent.
pub struct Decoder {
    key: [u8; 32],
    plan: Plan,
    g: u64,
    k: usize,
    listed: Listed,
    /// Rows in reduced echelon form: `(pivot, coefficients, payload)`.
    rows: Vec<(usize, Vec<u8>, Vec<u8>)>,
}

impl Decoder {
    pub fn new(key: &[u8; 32], plan: &Plan, g: u64, listed: Listed) -> Decoder {
        Decoder {
            key: *key,
            plan: *plan,
            g,
            k: plan.k(g),
            listed,
            rows: Vec::new(),
        }
    }
    pub fn done(&self) -> bool {
        self.rows.len() == self.k
    }
    pub fn rank(&self) -> usize {
        self.rows.len()
    }
    /// Fragment `j` arrived: checked (its hash in the index), then eliminated against what is held.
    pub fn add(&mut self, j: u8, state: &[u8]) -> Result<(), Error> {
        let hash = self
            .listed
            .iter()
            .find(|(x, _)| *x == j)
            .map(|(_, h)| *h)
            .ok_or(Error::Malformed)?;
        if *blake3::hash(state).as_bytes() != hash {
            return Err(Error::Forged);
        }
        if self.done() {
            return Err(Error::Redundant);
        }
        let (c, p) = parse_fragment(&self.plan, self.k, state)?;
        let (mut c, mut p) = (c.to_vec(), p.to_vec());
        for (pivot, rc, rp) in &self.rows {
            let f = c[*pivot];
            if f != 0 {
                gf::axpy(&mut c, rc, f);
                gf::axpy(&mut p, rp, f);
            }
        }
        let Some(pivot) = c.iter().position(|x| *x != 0) else {
            return Err(Error::Redundant);
        };
        let inv = gf::inv(c[pivot]);
        gf::scale(&mut c, inv);
        gf::scale(&mut p, inv);
        // Keep the form reduced: clear the new pivot from every row held.
        for (_, rc, rp) in self.rows.iter_mut() {
            let f = rc[pivot];
            if f != 0 {
                gf::axpy(rc, &c, f);
                gf::axpy(rp, &p, f);
            }
        }
        self.rows.push((pivot, c, p));
        Ok(())
    }
    /// The generation's bytes, once done.
    pub fn plain(&self) -> Result<Vec<u8>, Error> {
        if !self.done() {
            return Err(Error::Malformed);
        }
        let mut rows: Vec<&(usize, Vec<u8>, Vec<u8>)> = self.rows.iter().collect();
        rows.sort_by_key(|r| r.0);
        let mut out = Vec::new();
        for (i, (_, _, symbol)) in rows.into_iter().enumerate() {
            out.extend(open_chunk(
                &self.key,
                &self.plan,
                self.g * GEN as u64 + i as u64,
                symbol,
            )?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
