//! THE SDK'S LOAD PIECES (sdk#347): what a published app fetches after its tiny starter container, as `k + m`
//! Reed–Solomon pieces raced from the first `k` (the owner's rule 11, applied to loading the SDK itself).
//!
//! Two steps, both here and nowhere else (one owner; the decoder wasm, the SDK wasm and the tools all link THIS):
//!
//! - **The bundle.** Named files as one byte string: `MAGIC`, a `u32` count, then per file (sorted by name) a
//!   `u16` name length, the name, a `u32` length; then every file's bytes in the same order. Deterministic: the
//!   same files give the same bytes, so the same pieces and the same addresses. [`unbundle`] is TOTAL over hostile
//!   bytes (a refusal, never a panic, never an allocation a length field chose).
//! - **The pieces.** The bundle cut into `k` data pieces of at most `payload` bytes each, and `m` parity pieces
//!   from `freenet_prolly::rs` (the tree's own codec, `m` a parameter). A data piece is a length-prefixed symbol
//!   (`u32` LE length, then the bytes), the form `rs::repair` reads its ends from; a parity piece is stored
//!   trimmed, as `rs::encode` makes it. ANY `k` of the `k + m` rebuild the bundle ([`join`]).
//!
//! Whether a piece is the right one is decided by the caller, by its sha256 in the manifest, before it is handed
//! here: this crate has no hash, so the decoder wasm stays small.

use freenet_prolly::rs;
/// The codec's refusal, re-exported so a caller names it without depending on the tree library itself.
pub use freenet_prolly::rs::RsError;

/// The bundle's first four bytes.
pub const MAGIC: &[u8; 4] = b"CWB1";
/// The most files one bundle names.
pub const MAX_FILES: usize = 1024;
/// The longest file name.
pub const MAX_NAME: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not a bundle: wrong magic, a length past the end, a name that is not UTF-8, too many files, trailing bytes.
    NotABundle(&'static str),
    /// Two files with one name.
    DuplicateName(String),
    /// The bundle needs more data pieces than the codec can code, or the payload is 0.
    TooManyPieces { k: usize, max: usize },
    /// A piece count that does not match `k + m`, or an `m` out of range.
    Shape(&'static str),
    /// Fewer than `k` pieces, or the codec refused what it was given.
    Rs(rs::RsError),
}

impl From<rs::RsError> for Error {
    fn from(e: rs::RsError) -> Error {
        Error::Rs(e)
    }
}

/// `files` as one bundle, sorted by name whatever order they come in.
pub fn bundle(files: &[(&str, &[u8])]) -> Result<Vec<u8>, Error> {
    if files.len() > MAX_FILES {
        return Err(Error::NotABundle("too many files"));
    }
    let mut sorted: Vec<&(&str, &[u8])> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for w in sorted.windows(2) {
        if w[0].0 == w[1].0 {
            return Err(Error::DuplicateName(w[0].0.to_string()));
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(sorted.len() as u32).to_le_bytes());
    for (name, bytes) in &sorted {
        if name.is_empty() || name.len() > MAX_NAME {
            return Err(Error::NotABundle("a name of 0 or more than 255 bytes"));
        }
        if bytes.len() > u32::MAX as usize {
            return Err(Error::NotABundle("a file over 4 GiB"));
        }
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    }
    for (_, bytes) in &sorted {
        out.extend_from_slice(bytes);
    }
    Ok(out)
}

/// A bundle's files, in its order. TOTAL: every length is checked against what is actually there before anything
/// is allocated for it, and trailing bytes are refused (one bundle has one reading).
pub fn unbundle(b: &[u8]) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let mut at = 0usize;
    let take = |at: &mut usize, n: usize| -> Result<&[u8], Error> {
        let end = at
            .checked_add(n)
            .filter(|&e| e <= b.len())
            .ok_or(Error::NotABundle("a length past the end"))?;
        let s = &b[*at..end];
        *at = end;
        Ok(s)
    };
    if take(&mut at, 4)? != MAGIC {
        return Err(Error::NotABundle("wrong magic"));
    }
    let count = u32::from_le_bytes(take(&mut at, 4)?.try_into().expect("4 bytes")) as usize;
    if count > MAX_FILES {
        return Err(Error::NotABundle("too many files"));
    }
    let mut index: Vec<(String, usize)> = Vec::with_capacity(count.min(64));
    for _ in 0..count {
        let n = u16::from_le_bytes(take(&mut at, 2)?.try_into().expect("2 bytes")) as usize;
        if n == 0 || n > MAX_NAME {
            return Err(Error::NotABundle("a name of 0 or more than 255 bytes"));
        }
        let name = core::str::from_utf8(take(&mut at, n)?)
            .map_err(|_| Error::NotABundle("a name that is not UTF-8"))?;
        let len = u32::from_le_bytes(take(&mut at, 4)?.try_into().expect("4 bytes")) as usize;
        if let Some((prev, _)) = index.last() {
            if prev.as_str() >= name {
                return Err(if prev == name {
                    Error::DuplicateName(name.to_string())
                } else {
                    Error::NotABundle("names out of order")
                });
            }
        }
        index.push((name.to_string(), len));
    }
    let mut out = Vec::with_capacity(index.len());
    for (name, len) in index {
        out.push((name, take(&mut at, len)?.to_vec()));
    }
    if at != b.len() {
        return Err(Error::NotABundle("trailing bytes"));
    }
    Ok(out)
}

/// A bundle cut into its pieces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cut {
    /// Data pieces.
    pub k: usize,
    /// Parity pieces.
    pub m: usize,
    /// The most bundle bytes one data piece carries.
    pub payload: usize,
    /// The bundle's length: [`join`] cuts the rebuilt bytes back to it.
    pub bundle_len: usize,
    /// `k` data pieces then `m` parity pieces, each exactly as it is PUT.
    pub pieces: Vec<Vec<u8>>,
}

/// How many data pieces a bundle of `len` bytes takes at `payload` bytes each (at least one).
pub fn data_pieces(len: usize, payload: usize) -> usize {
    len.div_ceil(payload.max(1)).max(1)
}

/// Cut `bundle` into `k = ⌈len / payload⌉` data pieces and `m` parity pieces.
pub fn cut(bundle: &[u8], payload: usize, m: usize) -> Result<Cut, Error> {
    if payload == 0 {
        return Err(Error::TooManyPieces {
            k: usize::MAX,
            max: rs::MAX_K,
        });
    }
    if m == 0 || m > rs::MAX_M {
        return Err(Error::Shape("m out of range"));
    }
    let k = data_pieces(bundle.len(), payload);
    if k > rs::MAX_K {
        return Err(Error::TooManyPieces { k, max: rs::MAX_K });
    }
    let data: Vec<Vec<u8>> = (0..k).map(|i| symbol(chunk(bundle, payload, i))).collect();
    let parity = rs::encode(&data, m)?;
    let mut pieces = data;
    pieces.extend(parity);
    Ok(Cut {
        k,
        m,
        payload,
        bundle_len: bundle.len(),
        pieces,
    })
}

/// Piece `index` of `bundle`'s cut, alone: what a loader that rebuilt the bundle PUTs back for a piece it saw
/// missing (the post-load repair). Equal to `cut(..).pieces[index]` by construction.
pub fn piece(bundle: &[u8], payload: usize, m: usize, index: usize) -> Result<Vec<u8>, Error> {
    let c = cut(bundle, payload, m)?;
    c.pieces
        .get(index)
        .cloned()
        .ok_or(Error::Shape("no such piece"))
}

/// The bundle from any `k` of its `k + m` pieces (`have[i]` is `Some` for piece `i` present, already checked
/// against its sha256 by the caller), cut back to `bundle_len`.
pub fn join(
    k: usize,
    m: usize,
    payload: usize,
    bundle_len: usize,
    have: &[Option<Vec<u8>>],
) -> Result<Vec<u8>, Error> {
    if have.len() != k + m {
        return Err(Error::Shape("the piece list is not k + m long"));
    }
    if k != data_pieces(bundle_len, payload) {
        return Err(Error::Shape(
            "k does not match the bundle length and payload",
        ));
    }
    // The data pieces are the bundle's own bytes: when all k are here, nothing needs solving.
    let data: Vec<Vec<u8>> = if have[..k].iter().all(Option::is_some) {
        have[..k]
            .iter()
            .map(|p| p.clone().expect("present"))
            .collect()
    } else {
        rs::repair(k, m, have, payload)?
    };
    let mut out = Vec::with_capacity(bundle_len);
    for (i, s) in data.iter().enumerate() {
        let want = chunk_len(bundle_len, payload, i);
        let body = s
            .get(4..)
            .ok_or(Error::Shape("a data piece without its length"))?;
        let len = u32::from_le_bytes(s[..4].try_into().expect("4 bytes")) as usize;
        if len != want || body.len() != want {
            return Err(Error::Shape("a data piece of the wrong length"));
        }
        out.extend_from_slice(body);
    }
    Ok(out)
}

fn chunk(b: &[u8], payload: usize, i: usize) -> &[u8] {
    let start = (i * payload).min(b.len());
    &b[start..(start + payload).min(b.len())]
}

fn chunk_len(len: usize, payload: usize, i: usize) -> usize {
    len.saturating_sub(i * payload).min(payload)
}

fn symbol(bytes: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + bytes.len());
    v.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    v.extend_from_slice(bytes);
    v
}


/// The FROZEN load-piece vectors (sdk#347): `tests/js/pieces-vectors.json` is [`vectors::json`]'s output. This
/// crate's test checks the committed file still IS that output; `tests/js/decoder.test.mjs` decodes it through the
/// BUILT decoder wasm. Regenerate (`cargo run -p pieces --example vectors`) only with a format change.
pub mod vectors {
    /// Bundle bytes per data piece in the vectors.
    pub const PAYLOAD: usize = 2048;
    /// Parity pieces in the vectors.
    pub const M: usize = 8;

    /// The vectors' files: deterministic, named as the real bundle's are.
    pub fn files() -> Vec<(String, Vec<u8>)> {
        let mut s: u32 = 0xC0FF_EE11;
        let mut noise = |n: usize| -> Vec<u8> {
            (0..n)
                .map(|_| {
                    s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    (s >> 16) as u8
                })
                .collect()
        };
        vec![
            ("craftworks_sdk_bg.wasm".to_string(), noise(14_000)),
            ("index.js".to_string(), b"export const loaded = true;\n".to_vec()),
            ("runtime.js".to_string(), noise(3_001)),
        ]
    }

    /// The vectors file, as text.
    pub fn json() -> String {
        let files = files();
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        let b = crate::bundle(&refs).expect("a bundle");
        let c = crate::cut(&b, PAYLOAD, M).expect("a cut");
        let hex = core_types::hex::encode;
        let mut o = String::new();
        o.push_str("{\n");
        o.push_str("  \"note\": \"FROZEN sdk#347 load-piece vectors; regenerate only with a format change (pieces/examples/vectors.rs)\",\n");
        o.push_str(&format!("  \"k\": {}, \"m\": {}, \"payload\": {}, \"bundle_len\": {},\n", c.k, c.m, c.payload, c.bundle_len));
        o.push_str("  \"files\": [\n");
        for (i, (n, f)) in files.iter().enumerate() {
            o.push_str(&format!("    {{ \"name\": \"{n}\", \"hex\": \"{}\" }}{}\n", hex(f), if i + 1 < files.len() { "," } else { "" }));
        }
        o.push_str("  ],\n  \"pieces\": [\n");
        for (i, p) in c.pieces.iter().enumerate() {
            o.push_str(&format!("    \"{}\"{}\n", hex(p), if i + 1 < c.pieces.len() { "," } else { "" }));
        }
        o.push_str("  ]\n}\n");
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u32) -> Vec<u8> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (s >> 16) as u8
            })
            .collect()
    }

    /// The committed vectors are what this code cuts: a change to the format or the code shows up here first, not
    /// as a page that cannot load.
    #[test]
    fn the_committed_vectors_are_what_this_code_cuts() {
        let committed = include_str!("../../tests/js/pieces-vectors.json");
        assert!(committed == vectors::json(), "tests/js/pieces-vectors.json is not what pieces::vectors::json() renders now");
    }

    #[test]
    fn a_bundle_round_trips_and_is_deterministic() {
        let a = noise(1000, 1);
        let b = noise(3, 2);
        let one = bundle(&[("sdk.wasm", &a), ("index.js", &b), ("empty", &[])]).unwrap();
        let two = bundle(&[("index.js", &b), ("empty", &[]), ("sdk.wasm", &a)]).unwrap();
        assert_eq!(one, two, "the order files are given in changed the bytes");
        let back = unbundle(&one).unwrap();
        assert_eq!(
            back,
            vec![
                ("empty".to_string(), vec![]),
                ("index.js".to_string(), b),
                ("sdk.wasm".to_string(), a)
            ]
        );
        assert!(matches!(
            bundle(&[("x", &[1]), ("x", &[2])]),
            Err(Error::DuplicateName(_))
        ));
    }

    /// TOTAL over hostile bytes: every prefix of a real bundle, every byte flipped in its header, and a few
    /// hand-made liars are refused (or, for a data byte, read as other bytes), never a panic.
    #[test]
    fn unbundle_refuses_what_is_not_a_bundle() {
        let good = bundle(&[("a.js", b"alpha"), ("b.wasm", &noise(40, 3))]).unwrap();
        let header = 4 + 4 + 2 * (2 + 4) + 4 + 6;
        for n in 0..good.len() {
            assert!(
                unbundle(&good[..n]).is_err(),
                "a {n}-byte prefix was read as a bundle"
            );
        }
        for i in 0..header {
            let mut bad = good.clone();
            bad[i] ^= 0x5A;
            let _ = unbundle(&bad);
        }
        let mut trailing = good.clone();
        trailing.push(0);
        assert_eq!(
            unbundle(&trailing),
            Err(Error::NotABundle("trailing bytes"))
        );
        // A count that promises 4 billion files: refused before anything is allocated for it.
        let mut many = MAGIC.to_vec();
        many.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(unbundle(&many), Err(Error::NotABundle("too many files")));
        // A file length past the end.
        let mut long = MAGIC.to_vec();
        long.extend_from_slice(&1u32.to_le_bytes());
        long.extend_from_slice(&1u16.to_le_bytes());
        long.push(b'x');
        long.extend_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            unbundle(&long),
            Err(Error::NotABundle("a length past the end"))
        );
    }

    /// The bundle comes back from ANY k of its k + m pieces, at the sizes the SDK uses (a ~1.4 MB bundle in 64 KiB
    /// pieces is k = 23) and at the edges (one piece; a bundle an exact multiple of the payload).
    #[test]
    fn any_k_pieces_rebuild_the_bundle() {
        let mut s: u64 = 0x2545_F491_4F6C_DD1D;
        let mut rnd = |n: usize| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % n as u64) as usize
        };
        for (len, payload, m) in [
            (1_430_000, 65_536, 8),
            (65_536 * 4, 65_536, 8),
            (10, 65_536, 8),
            (300_000, 32_768, 3),
            (0, 1024, 1),
        ] {
            let b = noise(len, len as u32 + 7);
            let c = cut(&b, payload, m).unwrap();
            assert_eq!(c.k, data_pieces(len, payload));
            assert_eq!(c.pieces.len(), c.k + m);
            // Every data piece present: the fast path, no solve.
            let all: Vec<Option<Vec<u8>>> = c.pieces.iter().cloned().map(Some).collect();
            assert_eq!(join(c.k, m, payload, len, &all).unwrap(), b);
            // m losses, several ways.
            for trial in 0..6 {
                let mut lost = Vec::new();
                while lost.len() < m.min(c.k + m - c.k) {
                    let j = if trial == 0 { lost.len() } else { rnd(c.k + m) };
                    if !lost.contains(&j) {
                        lost.push(j);
                    }
                }
                let have: Vec<Option<Vec<u8>>> = (0..c.k + m)
                    .map(|j| (!lost.contains(&j)).then(|| c.pieces[j].clone()))
                    .collect();
                assert_eq!(
                    join(c.k, m, payload, len, &have).unwrap(),
                    b,
                    "len {len}, lost {lost:?}"
                );
            }
            // m + 1 losses cannot be rebuilt, and say so.
            if c.k > 1 {
                let have: Vec<Option<Vec<u8>>> = (0..c.k + m)
                    .map(|j| (j > m).then(|| c.pieces[j].clone()))
                    .collect();
                assert!(matches!(
                    join(c.k, m, payload, len, &have),
                    Err(Error::Rs(rs::RsError::NotEnough(_)))
                ));
            }
            // One piece alone is the same bytes as the cut's: what a repair PUTs.
            for i in [0, c.k - 1, c.k + m - 1] {
                assert_eq!(piece(&b, payload, m, i).unwrap(), c.pieces[i]);
            }
        }
    }

    #[test]
    fn the_shape_is_checked() {
        let b = noise(200_000, 9);
        assert!(matches!(
            cut(&b, 1024, 8),
            Err(Error::TooManyPieces { k: 196, .. })
        ));
        assert!(matches!(cut(&b, 0, 8), Err(Error::TooManyPieces { .. })));
        assert_eq!(cut(&b, 65_536, 0), Err(Error::Shape("m out of range")));
        assert_eq!(
            cut(&b, 65_536, rs::MAX_M + 1),
            Err(Error::Shape("m out of range"))
        );
        let c = cut(&b, 65_536, 8).unwrap();
        let all: Vec<Option<Vec<u8>>> = c.pieces.iter().cloned().map(Some).collect();
        assert_eq!(
            join(c.k, 8, 65_536, b.len(), &all[..c.k + 7]),
            Err(Error::Shape("the piece list is not k + m long"))
        );
        assert_eq!(
            join(
                c.k + 1,
                8,
                65_536,
                b.len(),
                &[all.clone(), vec![None]].concat()
            ),
            Err(Error::Shape(
                "k does not match the bundle length and payload"
            ))
        );
        // A data piece that is present but the wrong length (a caller that skipped the hash check).
        let mut bad = all.clone();
        bad[0] = Some(vec![0, 0, 0, 0]);
        assert_eq!(
            join(c.k, 8, 65_536, b.len(), &bad),
            Err(Error::Shape("a data piece of the wrong length"))
        );
    }
}

/// A PIECE SET as a published app names it (sdk#516's APP-PUBLISH, sdk#493's KEEPER §5 "Walk sources"): `k` data and
/// `m` parity pieces, each by the ADDRESS its web container is served at and the sha256 of its bytes. ANY `k` of the
/// `k + m` rebuild the bundle ([`join`]), so a set is REBUILDABLE once `k` of its pieces are on the network. The ONE
/// type the publisher and the keeper read. Plain data: this crate stays hash-free, so the sha256 is the caller's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PieceSet {
    /// Its name in the app's starter (`core`, `provisioning`).
    pub name: String,
    pub k: usize,
    pub m: usize,
    pub pieces: Vec<NamedPiece>,
}

/// One piece of a [`PieceSet`]: where it is served and what its bytes hash to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedPiece {
    pub address: String,
    pub sha256: [u8; 32],
}

impl PieceSet {
    /// A set names exactly `k + m` pieces, with `k` at least 1: anything else is not a set a bundle rebuilds from.
    pub fn check(&self) -> Result<(), String> {
        if self.k == 0 || self.pieces.len() != self.k + self.m {
            return Err(format!("piece set {}: {} pieces for k {} + m {}", self.name, self.pieces.len(), self.k, self.m));
        }
        Ok(())
    }
}
