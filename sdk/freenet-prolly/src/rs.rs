//! The Reed–Solomon code itself: GF(2⁸), a systematic Cauchy generator, and
//! the recovery of erased symbols.
//!
//! **Everything here is pinned to the byte, and that is the point.** "Parity is
//! a pure function of the children" is what makes it deduplicate, verifiable by
//! plain hash, and repairable by anyone without a key — and it is only true if
//! two independent implementations produce identical bytes. Two libraries that
//! both call themselves systematic RS(k+m, k) routinely disagree on the field
//! polynomial, on how the generator matrix is built, and on which axis the
//! symbols run along. So each of those is fixed here and frozen by vectors.
//!
//! - Field GF(2⁸), reducing polynomial **0x11D**, generator **2**.
//! - Generator matrix `[I ; C]`, systematic, with `C` an m × k Cauchy matrix:
//!   `C[r][c] = 1 / (x_r ⊕ y_c)`. The number of parity rows `m` is a PARAMETER
//!   of each call (`1 ≤ m ≤ MAX_M = 8`), and each row is fixed by its own `x`:
//!   - DATA COLUMNS `y_c = 3 + c`, `c ∈ 0..k`, `k ≤ MAX_K = 36` (y runs 3..=38):
//!     frozen from the three-row code;
//!   - PARITY ROWS `x_r = r` for `r ∈ 0..3`, frozen, so a group's first three
//!     parity symbols are byte-identical to what the three-row code made (the
//!     frozen vectors say so, not just this sentence);
//!   - `x_r = 255 − (r − 3)` for `r ∈ 3..8` (255..=251): the added rows sit at the
//!     TOP of the field, so `MAX_K` can later grow up to 248 without moving a
//!     shipped row.
//!
//!   The x and y sets are disjoint (asserted at compile time), every square
//!   submatrix of a Cauchy matrix is invertible, a group of `k` is literally the
//!   first `k` columns and `m` rows the first `m` rows — so ONE definition covers
//!   every `k` and every `m`, `k = 1` included.
//! - The TREE's own parity per group is `parity::PARITY` and its largest group is
//!   `parity::MAX_GROUP`: the node format's numbers, owned there, which this codec
//!   does not decide. Every user of the codec (the tree, the SDK's load pieces,
//!   sdk#347) passes its own `m` and `k`.
//! - Layout: byte `i` of every data symbol is one codeword. Parity `p` byte `i`
//!   is `⊕_c C[p][c] · data_c[i]`.

/// The reducing polynomial, x⁸ + x⁴ + x³ + x² + 1.
const POLY: u16 = 0x11D;
/// The most parity symbols the codec makes for one group.
pub const MAX_M: usize = 8;
/// The largest group the codec can code. NOT the tree's grouping bound
/// (`parity::MAX_GROUP`), which is a node-format number of its own.
pub const MAX_K: usize = 36;
/// The rows of the original three-row code, whose `x_r = r` is frozen.
const FROZEN_ROWS: usize = 3;
/// `y_c = Y0 + c`, frozen from the three-row code.
const Y0: usize = 3;

/// `x_r` of parity row `r`: the three original rows at `r`, the rest from the
/// top of the field down (255, 254, …).
pub const fn x_of(r: usize) -> u8 {
    if r < FROZEN_ROWS {
        r as u8
    } else {
        (255 - (r - FROZEN_ROWS)) as u8
    }
}

// The one condition the Cauchy construction needs: x and y never meet. Checked,
// not remembered.
const _: () = assert!(FROZEN_ROWS <= Y0);
const _: () = assert!(x_of(MAX_M - 1) as usize > Y0 + MAX_K - 1);

/// `exp[i] = 2^i` and `log[2^i] = i` in GF(2⁸), built once from the polynomial
/// rather than pasted in: a table nobody can check against its own definition
/// is a place for a typo to live for ever.
struct Tables {
    exp: [u8; 512],
    log: [u8; 256],
}

impl Tables {
    const fn new() -> Tables {
        let mut t = Tables {
            exp: [0; 512],
            log: [0; 256],
        };
        let mut x: u16 = 1;
        let mut i = 0;
        while i < 255 {
            t.exp[i] = x as u8;
            t.log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= POLY;
            }
            i += 1;
        }
        // The second half repeats the first, so a product's exponent needs no
        // modulo: `exp[log a + log b]` is always in range.
        let mut j = 0;
        while j < 255 {
            t.exp[255 + j] = t.exp[j];
            j += 1;
        }
        t
    }
}

static T: Tables = Tables::new();

/// Multiplication in GF(2⁸). Zero is handled before the logarithm, which has
/// no value at zero.
fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    T.exp[T.log[a as usize] as usize + T.log[b as usize] as usize]
}

/// Multiplication, for the incremental update in [`crate::parity`].
pub fn mul_pub(a: u8, b: u8) -> u8 {
    mul(a, b)
}

/// Division, for constructing a cancelling fixture in the tests.
pub fn div_pub(a: u8, b: u8) -> u8 {
    div(a, b)
}

/// Division in GF(2⁸). `b` must not be zero; every call here divides by a
/// Cauchy denominator `x_r ⊕ y_c`, which is non-zero because the index sets are
/// disjoint.
fn div(a: u8, b: u8) -> u8 {
    debug_assert!(b != 0, "division by zero in GF(2^8)");
    if a == 0 {
        return 0;
    }
    let d = T.log[a as usize] as i16 - T.log[b as usize] as i16;
    T.exp[(d.rem_euclid(255)) as usize]
}

/// `C[r][c]` of the Cauchy generator, for a group of `k` data symbols.
///
/// `x_r` from [`x_of`], `y_c = 3 + c`. The sets are disjoint, so the
/// denominator is never zero and the matrix is defined for every `k` —
/// including `k = 1`, where the parity symbols are different scalar multiples of
/// the single data symbol.
pub fn coeff(r: usize, c: usize) -> u8 {
    debug_assert!(r < MAX_M && c < MAX_K);
    div(1, x_of(r) ^ ((Y0 + c) as u8))
}

/// The `m` parity symbols of a group (the tree calls it with `parity::PARITY`).
///
/// **There is no padding in the rule.** A symbol is its bytes followed by
/// infinitely many zeros, and parity is defined per byte index — so a member's
/// length is not part of the group's definition and changing one member cannot
/// change what the others contribute. The result is stored with its trailing
/// zeros trimmed, which makes a parity block's length a function of its own
/// bytes alone. Nothing may read that length as the group's width.
pub fn encode(data: &[Vec<u8>], m: usize) -> Result<Vec<Vec<u8>>, RsError> {
    let k = data.len();
    if k == 0 || k > MAX_K {
        return Err(RsError::GroupSize(k));
    }
    if m == 0 || m > MAX_M {
        return Err(RsError::ParityCount(m));
    }
    let width = data.iter().map(|d| d.len()).max().unwrap_or(0);
    let mut out = vec![vec![0u8; width]; m];
    for (c, d) in data.iter().enumerate() {
        for (r, p) in out.iter_mut().enumerate() {
            let f = coeff(r, c);
            if f == 0 {
                continue;
            }
            for (o, s) in p.iter_mut().zip(d.iter()) {
                *o ^= mul(f, *s);
            }
        }
    }
    for p in out.iter_mut() {
        trim(p);
    }
    Ok(out)
}

/// Byte columns solved. What a HOSTILE group costs a repairer is a property in
/// its own right: the answer is "refused" either way, and only a counter can
/// see the difference between refusing after four columns and after four
/// billion.
#[cfg(any(test, feature = "testing"))]
pub mod work {
    use core::cell::Cell;
    thread_local! { static N: Cell<usize> = const { Cell::new(0) }; }
    pub fn columns() -> usize {
        N.with(|n| n.get())
    }
    pub fn reset() {
        N.with(|n| n.set(0));
    }
    pub(super) fn tick() {
        N.with(|n| n.set(n.get() + 1));
    }
}

/// Drop trailing zero bytes. The canonical stored form of a parity block.
pub fn trim(v: &mut Vec<u8>) {
    while v.last() == Some(&0) {
        v.pop();
    }
}

/// Byte `i` of a block, which is zero past its end — the whole of what makes
/// trimming lossless.
pub fn byte_at(b: &[u8], i: usize) -> u8 {
    b.get(i).copied().unwrap_or(0)
}

/// Rebuild every data symbol from any `k` of the `k + m` blocks.
///
/// `have[j]` is `Some` for a present block: `0..k` the data symbols in group
/// order, `k..k+m` the parity. Blocks are ragged — a data symbol ends after its
/// own bytes, a parity block after its last non-zero one — and every index past
/// an end reads as zero.
///
/// The solve is per byte index against one inverted matrix. Indices 0..4 give
/// each data symbol's length prefix; the last index any of them needs is then
/// known exactly, so no index is ever revisited and the answer equals what an
/// untrimmed repair would have produced.
pub fn repair(
    k: usize,
    m: usize,
    have: &[Option<Vec<u8>>],
    max_len: usize,
) -> Result<Vec<Vec<u8>>, RsError> {
    if k == 0 || k > MAX_K {
        return Err(RsError::GroupSize(k));
    }
    if m == 0 || m > MAX_M {
        return Err(RsError::ParityCount(m));
    }
    if have.len() != k + m {
        return Err(RsError::Ragged);
    }
    // The first k present blocks, and the rows of [I;C] they stand for.
    let mut rows: Vec<[u8; MAX_K]> = Vec::with_capacity(k);
    let mut vals: Vec<&[u8]> = Vec::with_capacity(k);
    for (j, b) in have.iter().enumerate() {
        if rows.len() == k {
            break;
        }
        let Some(b) = b else { continue };
        let mut row = [0u8; MAX_K];
        if j < k {
            row[j] = 1;
        } else {
            for (c, slot) in row.iter_mut().enumerate().take(k) {
                *slot = coeff(j - k, c);
            }
        }
        rows.push(row);
        vals.push(b);
    }
    if rows.len() < k {
        return Err(RsError::NotEnough(rows.len()));
    }
    let inv = invert(&mut rows, k)?;

    // One column of the solve: the data bytes at index `i`.
    let solve = |i: usize| -> Vec<u8> {
        #[cfg(any(test, feature = "testing"))]
        work::tick();
        (0..k)
            .map(|r| {
                let mut acc = 0u8;
                for (c, v) in vals.iter().enumerate() {
                    acc ^= mul(inv[r][c], byte_at(v, i));
                }
                acc
            })
            .collect()
    };

    // The length prefixes first: four indices, and they bound everything else.
    let mut out: Vec<Vec<u8>> = vec![Vec::new(); k];
    for i in 0..4 {
        for (r, b) in solve(i).into_iter().enumerate() {
            out[r].push(b);
        }
    }
    let lens: Vec<usize> = out
        .iter()
        .map(|p| u32::from_le_bytes([p[0], p[1], p[2], p[3]]) as usize)
        .collect();
    // THE REBUILT LENGTH IS A STRANGER'S NUMBER. Parity ids are not checkable
    // by a host, so a writer can list ids of blocks whose bytes it chose; a
    // keeper repairing that tree would otherwise solve up to 4 GiB of byte
    // columns before the caller ever gets a block to hash. The caller knows
    // what kind of member this group holds, so it supplies the ceiling, and it
    // is checked BEFORE any index past the prefixes is solved.
    if let Some(&bad) = lens.iter().find(|&&l| l > max_len) {
        return Err(RsError::MemberTooLong(bad));
    }
    let end = lens.iter().map(|l| 4 + l).max().unwrap_or(4);
    for i in 4..end {
        for (r, b) in solve(i).into_iter().enumerate() {
            out[r].push(b);
        }
    }
    // Each symbol ends where its own prefix says.
    for (r, o) in out.iter_mut().enumerate() {
        o.truncate(4 + lens[r]);
    }
    Ok(out)
}

/// Invert the k x k matrix in place, returning the inverse. Any k rows of a
/// systematic Cauchy `[I;C]` are independent, so a pivot always exists.
fn invert(rows: &mut [[u8; MAX_K]], k: usize) -> Result<Vec<[u8; MAX_K]>, RsError> {
    let mut inv: Vec<[u8; MAX_K]> = (0..k)
        .map(|r| {
            let mut e = [0u8; MAX_K];
            e[r] = 1;
            e
        })
        .collect();
    for col in 0..k {
        let p = (col..k)
            .find(|&r| rows[r][col] != 0)
            .ok_or(RsError::Singular)?;
        rows.swap(col, p);
        inv.swap(col, p);
        let f = div(1, rows[col][col]);
        for v in rows[col].iter_mut().take(k) {
            *v = mul(*v, f);
        }
        for v in inv[col].iter_mut().take(k) {
            *v = mul(*v, f);
        }
        for r in 0..k {
            if r == col || rows[r][col] == 0 {
                continue;
            }
            let f = rows[r][col];
            let (pr, pi) = (rows[col], inv[col]);
            for (c, v) in rows[r].iter_mut().enumerate().take(k) {
                *v ^= mul(f, pr[c]);
            }
            for (c, v) in inv[r].iter_mut().enumerate().take(k) {
                *v ^= mul(f, pi[c]);
            }
        }
    }
    Ok(inv)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RsError {
    /// A group of 0, or of more than [`MAX_K`].
    GroupSize(usize),
    /// A parity count of 0, or of more than [`MAX_M`].
    ParityCount(usize),
    /// Symbols of different lengths, or the wrong number of slots.
    Ragged,
    /// Fewer than `k` blocks present.
    NotEnough(usize),
    /// Should be unreachable: any k rows of a Cauchy `[I;C]` are independent.
    Singular,
    /// A rebuilt member claims to be longer than its kind allows. The prefix
    /// came out of a solve over blocks a stranger may have chosen, so it is a
    /// claim and is bounded before it costs anything.
    MemberTooLong(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The field is a field: the tables are inverses of each other, and every
    /// non-zero element has a multiplicative inverse. Built from the polynomial
    /// rather than pasted, so this is what says the polynomial is right.
    #[test]
    fn the_field_tables_are_consistent() {
        for x in 1..=255u8 {
            assert_eq!(
                T.exp[T.log[x as usize] as usize], x,
                "log/exp disagree at {x}"
            );
            assert_eq!(mul(x, div(1, x)), 1, "{x} has no inverse");
        }
        assert_eq!(mul(0, 7), 0);
        assert_eq!(mul(7, 0), 0);
        // 0x11D is what makes 2 a generator: the powers of 2 must visit every
        // non-zero element exactly once before returning to 1.
        let mut seen = [false; 256];
        let mut x = 1u8;
        for _ in 0..255 {
            assert!(
                !seen[x as usize],
                "2 is not a generator under this polynomial"
            );
            seen[x as usize] = true;
            x = mul(x, 2);
        }
        assert_eq!(x, 1);
    }

    /// Systematic: the data symbols are not touched, only `m` are added. And
    /// the code is a pure function of its input, at every group size and `m`.
    #[test]
    fn encoding_is_deterministic_at_every_group_size() {
        for m in 1..=MAX_M {
            for k in 1..=MAX_K {
                // Non-zero: an all-zero data symbol codes to all-zero parity,
                // which is correct and makes the distinctness check below vacuous.
                let data: Vec<Vec<u8>> = (0..k).map(|i| sym(&[i as u8 + 1; 16])).collect();
                let a = encode(&data, m).expect("a codeable group");
                let b = encode(&data, m).expect("a codeable group");
                assert_eq!(a, b, "k = {k}, m = {m}: not deterministic");
                assert_eq!(a.len(), m);
                // Trimmed, so a parity block is at most the longest symbol and may
                // be shorter — never a fixed width.
                assert!(a.iter().all(|p| p.len() <= 20));
                // k = 1 is the degenerate case worth naming: DIFFERENT scalar
                // multiples of one symbol, not copies of it.
                if k == 1 {
                    for r in 1..m {
                        assert_ne!(
                            a[r - 1],
                            a[r],
                            "k = 1, m = {m}: parity rows {} and {r} equal",
                            r - 1
                        );
                    }
                }
            }
        }
    }

    /// A symbol as the format defines it: a length prefix, then that many
    /// bytes. `repair` reads the prefix to know where a rebuilt block ends, so
    /// a fixture of raw padded vectors would be testing a shape the format
    /// never produces.
    fn sym(payload: &[u8]) -> Vec<u8> {
        let mut v = (payload.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(payload);
        v
    }

    /// A deterministic RAGGED group: member i is a length-prefixed payload of
    /// (5 + 7i) mod 61 + 1 bytes, LCG-filled. The fixture the frozen vectors hash.
    fn fixture(k: usize) -> Vec<Vec<u8>> {
        let mut s: u32 = 0x1234_5678;
        (0..k)
            .map(|i| {
                let n = (5 + 7 * i) % 61 + 1;
                let mut v = (n as u32).to_le_bytes().to_vec();
                for _ in 0..n {
                    s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    v.push((s >> 16) as u8);
                }
                v
            })
            .collect()
    }

    fn digests(rows: &[Vec<u8>]) -> Vec<String> {
        rows.iter()
            .map(|r| blake3::hash(r).to_hex()[..16].to_string())
            .collect()
    }

    /// FROZEN BEFORE THIS CODEC TOOK `m`: the three rows of the three-row code,
    /// k = 1..=12, hashed from the code as it was at freenet-prolly d8716d3 (run
    /// there, on the unchanged `rs.rs`, and pasted). Rows 0–2 of every tree ever
    /// written are these bytes; if a row moves, every parity block on the network
    /// stops validating. `m = 8` must give the same first three rows too.
    const M3_ROWS: [[&str; 3]; 12] = [
        ["c3833a016daaf230", "042116bd34406c36", "7a728b14f64a56ed"],
        ["6b593b07b9161e44", "bf4218b88a1fb4d9", "fcaaa7c50a7d2104"],
        ["3b7e54b395fb6af0", "fa5f6b8dc6eb36f2", "102746ebfcf9cdd1"],
        ["6a80e45b830b1476", "8ec88dab2467127c", "b1a252d3039233f8"],
        ["86d12c7abcc608d1", "4ae4459df8d5fc2f", "84019914ee3b2d45"],
        ["01e1a51ad552ab75", "70f3a26273c3a8a9", "573cf2039c53d896"],
        ["1d0bcd95a8c0800d", "1dfa230f790d6fc6", "c5278c0d6e84e5b9"],
        ["668c8fc211c5edd0", "e130f7873793b949", "69b962a4faf3386b"],
        ["62161483c08c5c06", "efe57ab79229f240", "ffa3caf80d9fb1f1"],
        ["485c95dc6d2a6cd2", "a30fe73469b64bf9", "7332f8e00c204151"],
        ["5ee68021e8031810", "311b34c72fe38a12", "d20811919e17a52a"],
        ["bbec068ca8b4bed7", "35b643f5fc5e0c1c", "34afe3c9d3cc2be7"],
    ];

    #[test]
    fn rows_0_to_2_are_byte_identical_to_the_three_row_code() {
        for k in 1..=12 {
            let want: Vec<String> = M3_ROWS[k - 1].iter().map(|s| s.to_string()).collect();
            assert_eq!(
                digests(&encode(&fixture(k), 3).unwrap()),
                want,
                "k = {k}, m = 3"
            );
            for m in 3..=MAX_M {
                assert_eq!(
                    digests(&encode(&fixture(k), m).unwrap()[..3]),
                    want,
                    "k = {k}, m = {m}: rows 0-2 moved"
                );
            }
        }
        // Past the three-row code's k (12), a larger m still starts with the m = 3 rows.
        for k in 13..=MAX_K {
            let three = encode(&fixture(k), 3).unwrap();
            assert_eq!(
                &encode(&fixture(k), MAX_M).unwrap()[..3],
                &three[..],
                "k = {k}"
            );
        }
    }

    /// FROZEN HERE, the rows the codec adds (x = 255..251), at the group sizes
    /// that matter: 1, the tree's 12, the SDK load pieces' ~21 and the most, 36.
    const M8_ROWS: [(usize, [&str; 8]); 4] = [
        (
            1,
            [
                "c3833a016daaf230",
                "042116bd34406c36",
                "7a728b14f64a56ed",
                "cf14335ab272a022",
                "5e689115757fa9a5",
                "085548bd87c0ac73",
                "57722fd54b1f468d",
                "2495c5fe498e17b6",
            ],
        ),
        (
            12,
            [
                "bbec068ca8b4bed7",
                "35b643f5fc5e0c1c",
                "34afe3c9d3cc2be7",
                "4fddd45c91d08f6d",
                "9520d12b67e3050c",
                "123bdc1f1d2e5197",
                "6765b512b721a52a",
                "1c17c6118965d7e8",
            ],
        ),
        (
            21,
            [
                "bc5ad567c307646b",
                "d8601c58e7572896",
                "638454b8082a043e",
                "f70876df5054655e",
                "b1107f81fab33d42",
                "c48d12a843633b27",
                "7734f52f39d5b530",
                "acb12358e9025c7d",
            ],
        ),
        (
            36,
            [
                "a936e42e9948a536",
                "106d7007a1af3279",
                "d9fbd52bba0a0dee",
                "0befbf03fa8a9ae2",
                "391cc2308eefd782",
                "f8d4f3baecc2f8ab",
                "9639de87e3f5170e",
                "1d373d6339333c7c",
            ],
        ),
    ];

    #[test]
    fn the_m8_rows_are_frozen() {
        let mut wrong = Vec::new();
        for (k, want) in M8_ROWS {
            let got = digests(&encode(&fixture(k), MAX_M).unwrap());
            if got != want.iter().map(|s| s.to_string()).collect::<Vec<_>>() {
                wrong.push(format!("k = {k}: {got:?}"));
            }
        }
        assert!(
            wrong.is_empty(),
            "the m = 8 rows moved:\n{}",
            wrong.join("\n")
        );
    }

    /// WHERE the added rows are, spelled out: x = 255, 254, 253, 252, 251. A
    /// codec that put them right after the data columns (x = 39..) would pass
    /// every round trip today and pin a row that MAX_K could not grow past.
    #[test]
    fn the_added_rows_sit_at_the_top_of_the_field() {
        assert_eq!(
            (0..MAX_M).map(x_of).collect::<Vec<u8>>(),
            vec![0, 1, 2, 255, 254, 253, 252, 251]
        );
        // And the coefficient is that x's: C[3][0] = 1 / (255 ⊕ 3).
        assert_eq!(coeff(3, 0), div(1, 255 ^ 3));
        assert_eq!(coeff(7, MAX_K - 1), div(1, 251 ^ (3 + MAX_K as u8 - 1)));
    }

    /// Any three losses, at every group size, with symbols of UNEQUAL length —
    /// which is the case trimming exists for. m = 3, exhaustively.
    #[test]
    fn any_three_losses_are_repairable_at_every_group_size() {
        for k in 1..=MAX_K {
            let data = fixture(k);
            let parity = encode(&data, 3).expect("a codeable group");
            let all: Vec<Vec<u8>> = data.iter().chain(parity.iter()).cloned().collect();
            let n = k + 3;
            for a in 0..n {
                for b in a + 1..n {
                    for c in b + 1..n {
                        let have: Vec<Option<Vec<u8>>> = (0..n)
                            .map(|j| (j != a && j != b && j != c).then(|| all[j].clone()))
                            .collect();
                        let got = repair(k, 3, &have, usize::MAX).expect("k of k+3 present");
                        assert_eq!(got, data, "k = {k}: lost {a},{b},{c}");
                    }
                }
            }
        }
    }

    /// Any m losses for every m ≤ 8 and every k ≤ 36: all the extreme patterns
    /// (the first m, the last m, every data symbol when k ≤ m, every parity
    /// symbol) and 40 seeded random ones per (k, m).
    #[test]
    fn any_m_losses_are_repairable_for_every_m() {
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut rnd = |n: usize| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s % n as u64) as usize
        };
        let mut cases = 0usize;
        for m in 1..=MAX_M {
            for k in 1..=MAX_K {
                let data = fixture(k);
                let parity = encode(&data, m).expect("a codeable group");
                let all: Vec<Vec<u8>> = data.iter().chain(parity.iter()).cloned().collect();
                let n = k + m;
                let mut patterns: Vec<Vec<usize>> =
                    vec![(0..m).collect(), (n - m..n).collect(), (k..n).collect()];
                if k <= m {
                    patterns.push((0..k).collect());
                }
                for _ in 0..40 {
                    let mut lost: Vec<usize> = Vec::new();
                    while lost.len() < m {
                        let j = rnd(n);
                        if !lost.contains(&j) {
                            lost.push(j);
                        }
                    }
                    patterns.push(lost);
                }
                for lost in patterns {
                    let have: Vec<Option<Vec<u8>>> = (0..n)
                        .map(|j| (!lost.contains(&j)).then(|| all[j].clone()))
                        .collect();
                    let got = repair(k, m, &have, usize::MAX).expect("k of k+m present");
                    assert_eq!(got, data, "k = {k}, m = {m}: lost {lost:?}");
                    cases += 1;
                }
            }
        }
        println!("  {cases} erasure patterns repaired (m = 1..=8, k = 1..=36)");
        assert!(
            cases > 8 * 36 * 40,
            "the sweep ran fewer cases than it names"
        );
    }

    /// Fewer than k blocks cannot be repaired, and says so rather than
    /// returning something plausible; m out of range is refused by name.
    #[test]
    fn too_few_blocks_is_refused() {
        let k = 8;
        let data: Vec<Vec<u8>> = (0..k).map(|i| sym(&[i as u8 + 1; 8])).collect();
        let parity = encode(&data, 3).unwrap();
        let all: Vec<Vec<u8>> = data.iter().chain(parity.iter()).cloned().collect();
        // Four losses out of k + 3 = 11 leaves 7 < 8.
        let have: Vec<Option<Vec<u8>>> = (0..k + 3)
            .map(|j| (j >= 4).then(|| all[j].clone()))
            .collect();
        assert_eq!(
            repair(k, 3, &have, usize::MAX),
            Err(RsError::NotEnough(k - 1))
        );
        // m + 1 losses at m = 8.
        let p8 = encode(&fixture(20), 8).unwrap();
        let all8: Vec<Vec<u8>> = fixture(20).into_iter().chain(p8).collect();
        let have8: Vec<Option<Vec<u8>>> =
            (0..28).map(|j| (j >= 9).then(|| all8[j].clone())).collect();
        assert_eq!(
            repair(20, 8, &have8, usize::MAX),
            Err(RsError::NotEnough(19))
        );
        assert_eq!(encode(&[], 3), Err(RsError::GroupSize(0)));
        assert!(matches!(
            encode(&vec![vec![0u8; 4]; MAX_K + 1], 3),
            Err(RsError::GroupSize(_))
        ));
        assert_eq!(encode(&fixture(4), 0), Err(RsError::ParityCount(0)));
        assert_eq!(
            encode(&fixture(4), MAX_M + 1),
            Err(RsError::ParityCount(MAX_M + 1))
        );
        assert_eq!(repair(4, 0, &[], usize::MAX), Err(RsError::ParityCount(0)));
        assert_eq!(
            repair(4, 3, &vec![None; 8], usize::MAX),
            Err(RsError::Ragged),
            "k + m slots, not k + 3 by habit"
        );
        // Ragged input is NOT an error: a symbol is zero past its end, which is
        // exactly what makes trimming lossless.
        assert!(encode(&[sym(&[1u8; 4]), sym(&[2u8; 5])], 3).is_ok());
    }

    /// The Cauchy coefficients, spelled out for every row and column. If the
    /// construction or the index sets ever move, this fails before any vector
    /// does and says which cell changed.
    #[test]
    fn the_cauchy_coefficients_are_what_the_rule_says() {
        let xs = [0u8, 1, 2, 255, 254, 253, 252, 251];
        for (r, &x) in xs.iter().enumerate() {
            for c in 0..MAX_K {
                let y = 3 + c as u8;
                assert_ne!(x, y, "x_{r} = y_{c}: a zero denominator");
                assert_eq!(coeff(r, c), div(1, x ^ y), "C[{r}][{c}]");
                assert_ne!(coeff(r, c), 0, "C[{r}][{c}] must be invertible");
            }
        }
    }
}
