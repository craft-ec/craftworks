//! `PACK`: a set of blocks carried in one PUT.
//!
//! A relay forwards the whole contract container — the wasm included — at every
//! hop, so the number of PUTs on the commit path is the lever, not the bytes of
//! any one block. A pack is one PUT carrying several blocks, unpacked off the
//! commit path by anyone who holds it.
//!
//! **A pack is not a layout.** No pointer anywhere names `(pack, index)`. Root
//! hashes, dedup, proofs and parity are untouched: a pack is a transport, and
//! the blocks inside it are the same blocks with the same ids.
//!
//! ```text
//! body   = "PK01" ‖ count:u16 ‖ member*
//! member = kind:u8 ‖ len:u32 ‖ bytes
//! ```
//!
//! Members are strictly ascending **by their own block id**, which states "a
//! set of blocks" directly: the id IS a block's identity, so ordering by it
//! forbids duplicates and gives one encoding per set. Two writers packing the
//! same commit therefore produce the same pack, with the same id.
//!
//! # Why the format lives here and not in the contract
//!
//! It was spelled twice — authoritatively in the Block contract, which is what
//! actually refuses a malformed pack, and again in the engine, which must BUILD
//! packs and cannot depend on the contracts repo. Two spellings of one format
//! drift, and the drift does not show up in either repo's tests: it shows up as
//! a host refusing a commit, in production, with nothing to point at. This
//! crate is the one place both already trust.
//!
//! # What is NOT here
//!
//! **Which kinds may ride in a pack is the CONTRACT's policy, not the
//! format's**, and so is what makes a body a valid block of its kind, and so is
//! each kind's ceiling. A library that decided those would be deciding
//! something the contract is answerable for — and a later change to what a
//! `TREE_NODE` may be would have to be made twice again. They arrive through
//! [`Policy`].
//!
//! [`MAX_PACK`] is the FORMAT's ceiling. How large a pack a writer chooses to
//! build is policy and belongs to the engine.

use crate::{block_id, Cid};

/// Magic for the pack body.
pub const MAGIC: &[u8; 4] = b"PK01";

/// The ceiling on a pack's body. A ceiling, not a target.
pub const MAX_PACK: usize = 1024 * 1024;

/// The smallest a member can be: a kind byte, a length, and an empty body.
/// Every early refusal is measured against this.
pub const MIN_MEMBER: usize = 1 + 4;

/// `MAGIC ‖ count:u16`.
pub const HEADER: usize = 4 + 2;

/// What the CONTRACT decides about a member, which the format does not.
///
/// The three questions are separate because they are answered at three
/// different points in the walk, and the ORDER is a security property: a
/// stranger who sends a megabyte must not be able to buy a megabyte of hashing
/// with it. `packable` and `max_body` are decided from a kind byte and a length
/// FIELD, before any byte of the member is read; `well_formed` costs real work
/// and runs last, on a member an honest pack would pay for anyway.
pub trait Policy {
    /// May this kind ride in a pack? Decided from the kind byte alone.
    fn packable(&self, kind: u8) -> bool;
    /// The largest body this kind may have. Compared against the length FIELD.
    fn max_body(&self, kind: u8) -> usize;
    /// Is this a well-formed block of that kind? The expensive question.
    fn well_formed(&self, kind: u8, body: &[u8]) -> bool;
}

/// The format and nothing else: every kind rides, every body is well formed,
/// and the only ceiling is the format's own.
///
/// This is what [`members`] walks with. It is not a permissive Policy anyone
/// should validate a pack with — it is the absence of one, which is why the
/// doors that use it say so in their own documentation.
pub struct FormatOnly;

impl Policy for FormatOnly {
    fn packable(&self, _kind: u8) -> bool {
        true
    }
    fn max_body(&self, _kind: u8) -> usize {
        MAX_PACK
    }
    fn well_formed(&self, _kind: u8, _body: &[u8]) -> bool {
        true
    }
}

/// A member, as it sits in the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Member<'a> {
    pub kind: u8,
    pub body: &'a [u8],
    /// `BLAKE3(kind ‖ body)` — computed during the walk, because the order the
    /// format requires is over exactly this and an unpacker needs it anyway.
    pub id: Cid,
}

/// Why a body is not a pack.
///
/// Named rather than a bare `false`, because "the host refused the commit" with
/// nothing to point at is the failure this module exists to stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackError {
    /// Shorter than the header, or the magic is not `PK01`.
    NotAPack,
    /// A pack of nothing is not a pack, and the empty body has one spelling.
    Empty,
    /// The declared count cannot fit in what follows, at `MIN_MEMBER` each.
    CountDoesNotFit { count: usize, have: usize },
    /// A member ran off the end of the body.
    Truncated { member: usize },
    /// The policy refuses this kind in a pack.
    KindNotPackable { member: usize, kind: u8 },
    /// The length field is above what the policy allows that kind.
    MemberTooLong {
        member: usize,
        len: usize,
        max: usize,
    },
    /// Members must strictly ascend by block id: this one did not.
    OutOfOrder { member: usize },
    /// The policy refuses this member as a block of its kind.
    MemberNotWellFormed { member: usize, kind: u8 },
    /// The lengths did not tile the body: trailing bytes are a second
    /// encoding of the same set.
    TrailingBytes { left: usize },
}

/// One walk, used by every door, so two doors cannot disagree about the format.
///
/// `collect` is where members land when a caller wants them. It is `Option` and
/// not a second function because the CHECK ORDER is the load-bearing part: a
/// parser that hashed before looking at the kind byte would give a stranger the
/// hashing this order denies them, and two implementations of one order is the
/// duplication this module exists to remove.
fn walk<'a, P: Policy>(
    body: &'a [u8],
    policy: &P,
    mut collect: Option<&mut Vec<Member<'a>>>,
) -> Result<(), PackError> {
    let Some((head, mut rest)) = body.split_at_checked(HEADER) else {
        return Err(PackError::NotAPack);
    };
    if &head[..4] != MAGIC {
        return Err(PackError::NotAPack);
    }
    let count = u16::from_le_bytes([head[4], head[5]]) as usize;
    if count == 0 {
        return Err(PackError::Empty);
    }
    // The declared count must fit in what follows, at the smallest a member can
    // be. This is what stops `count = 65535` buying 65,535 iterations of
    // anything — including, for a collecting caller, a 65,535-element
    // allocation out of a ten-byte body. It is decided from two lengths,
    // before a single byte is hashed.
    match count.checked_mul(MIN_MEMBER) {
        Some(need) if need <= rest.len() => {}
        _ => {
            return Err(PackError::CountDoesNotFit {
                count,
                have: rest.len(),
            })
        }
    }
    if let Some(out) = collect.as_deref_mut() {
        out.reserve(count);
    }

    let mut prev: Option<Cid> = None;
    for i in 0..count {
        let Some((h, tail)) = rest.split_at_checked(MIN_MEMBER) else {
            return Err(PackError::Truncated { member: i });
        };
        let kind = h[0];
        // Refused from the kind byte alone: an unknown or unpackable kind costs
        // nothing, however large the pack claiming it is.
        if !policy.packable(kind) {
            return Err(PackError::KindNotPackable { member: i, kind });
        }
        let len = u32::from_le_bytes([h[1], h[2], h[3], h[4]]) as usize;
        // Checked against the length FIELD, before anything is read.
        let max = policy.max_body(kind);
        if len > max {
            return Err(PackError::MemberTooLong {
                member: i,
                len,
                max,
            });
        }
        let Some((bytes, tail)) = tail.split_at_checked(len) else {
            return Err(PackError::Truncated { member: i });
        };
        rest = tail;

        // From here a member costs real work, and an honest pack pays the same.
        // The id comes first because it is one hash, where a body check can be
        // a full node parse.
        #[cfg(any(test, feature = "testing"))]
        work::tick_id();
        let id = block_id(kind, bytes);
        if prev.is_some_and(|p| p >= id) {
            return Err(PackError::OutOfOrder { member: i });
        }
        prev = Some(id);

        #[cfg(any(test, feature = "testing"))]
        work::tick_body();
        if !policy.well_formed(kind, bytes) {
            return Err(PackError::MemberNotWellFormed { member: i, kind });
        }
        if let Some(out) = collect.as_deref_mut() {
            out.push(Member {
                kind,
                body: bytes,
                id,
            });
        }
    }
    if !rest.is_empty() {
        return Err(PackError::TrailingBytes { left: rest.len() });
    }
    Ok(())
}

/// Is `body` a well-formed pack under `policy`?
///
/// The order of the checks is the point: everything decidable from lengths and
/// kind bytes runs first, so a hostile pack is refused for the price of reading
/// its header.
pub fn well_formed<P: Policy>(body: &[u8], policy: &P) -> bool {
    walk(body, policy, None).is_ok()
}

/// The same walk, with the reason it refused.
pub fn check<P: Policy>(body: &[u8], policy: &P) -> Result<(), PackError> {
    walk(body, policy, None)
}

/// Every member of `body`, in order, or why the body is not a pack.
///
/// This walks the FORMAT: structure, the strict ascending order, and the exact
/// tiling of the body. It does NOT ask whether a kind may ride in a pack or
/// whether a member is a valid block of its kind — those are the contract's,
/// and a caller that needs them passes its [`Policy`] to [`check`] first. An
/// unpacker that only needs to take a pack apart needs neither: every member it
/// yields is one the pack really contains, with the id it is really stored
/// under.
///
/// Bounded before it allocates: the declared count is checked against the bytes
/// that follow, so a ten-byte body cannot ask for a 65,535-element vector.
pub fn members(body: &[u8]) -> Result<Vec<Member<'_>>, PackError> {
    let mut out = Vec::new();
    walk(body, &FormatOnly, Some(&mut out))?;
    Ok(out)
}

/// Build a pack body from members, ordering them as the format requires.
///
/// A pack is a SET, so the same block offered twice is one member: sorting
/// without de-duplicating would hand the caller bytes the contract refuses.
/// De-duplication is by block id, which is what the order is over and what "the
/// same block" means here — two members with the same id ARE the same bytes.
///
/// It does not otherwise check the members: [`check`] decides, and a builder
/// that refused early would hide the cases the tests exist to reach.
pub fn build(members: &[(u8, Vec<u8>)]) -> Result<Vec<u8>, BuildError> {
    let mut ordered: Vec<&(u8, Vec<u8>)> = members.iter().collect();
    ordered.sort_by_key(|(k, b)| block_id(*k, b));
    ordered.dedup_by_key(|(k, b)| block_id(*k, b));
    if ordered.is_empty() {
        return Err(BuildError::Empty);
    }
    // `count` is a u16 on the wire, so more members than that cannot be
    // expressed — and casting would have written a small count in front of a
    // long body, which the contract refuses for a reason the caller could not
    // see from its own input.
    if ordered.len() > u16::MAX as usize {
        return Err(BuildError::TooManyMembers(ordered.len()));
    }
    let mut out = Vec::from(&MAGIC[..]);
    out.extend_from_slice(&(ordered.len() as u16).to_le_bytes());
    for (k, b) in ordered {
        // A member's length is a u32 on the wire. Refused rather than cast, for
        // the same reason as the count: a truncated length writes a short
        // member in front of a long body.
        let Ok(len) = u32::try_from(b.len()) else {
            return Err(BuildError::MemberTooLong(b.len()));
        };
        out.push(*k);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(b);
    }
    // Checked at the end rather than accumulated, because the members are not
    // known to be within their own kinds' caps either; the contract's policy
    // decides that, and this only promises that what comes back is a pack the
    // contract could accept on size.
    if out.len() > MAX_PACK {
        return Err(BuildError::TooLarge(out.len()));
    }
    Ok(out)
}

/// Why a set of members cannot be made into a pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// A pack of nothing is not a pack.
    Empty,
    /// More members than the u16 count can express.
    TooManyMembers(usize),
    /// A member longer than the u32 length field can express.
    MemberTooLong(usize),
    /// The body would exceed [`MAX_PACK`].
    TooLarge(usize),
}

/// What walking a pack COST, counted so a test can assert on work rather than
/// on a clock.
///
/// Per-thread, and that is the whole point: the harness runs a binary's tests on
/// many threads, so a process-global counter reports every thread's work to
/// every reader, and `assert_eq!(ids(), 0)` after a refused pack then passes
/// when the other tests happen to be elsewhere and fails when they are not —
/// with nothing in a green run to say which it was. The proof counters in this
/// crate lied exactly that way once (#43).
#[cfg(any(test, feature = "testing"))]
pub mod work {
    use core::cell::Cell;

    thread_local! {
        static IDS: Cell<usize> = const { Cell::new(0) };
        static BODIES: Cell<usize> = const { Cell::new(0) };
    }

    /// Member ids hashed on this thread since [`reset`].
    pub fn ids() -> usize {
        IDS.with(|n| n.get())
    }

    /// Member bodies handed to the policy since [`reset`] — the other half of
    /// the cost, and the one a length check avoids.
    pub fn bodies() -> usize {
        BODIES.with(|n| n.get())
    }

    /// Both, because a test that measures one and forgets the other reads a
    /// number left over from whatever it did before.
    pub fn reset() {
        IDS.with(|n| n.set(0));
        BODIES.with(|n| n.set(0));
    }

    pub(super) fn tick_id() {
        IDS.with(|n| n.set(n.get() + 1));
    }

    pub(super) fn tick_body() {
        BODIES.with(|n| n.set(n.get() + 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind;

    /// A policy standing in for the Block contract's: `RAW` and `TREE_NODE`
    /// ride, a body is capped, and a body is "well formed" when it does not
    /// start with the refusal byte. The last one is a stand-in on purpose —
    /// what makes a TREE_NODE valid is the contract's, and a test here that
    /// encoded it would be this module deciding it after all.
    struct Contractish {
        max: usize,
        refuse: bool,
    }

    impl Default for Contractish {
        fn default() -> Self {
            Contractish {
                max: 256 * 1024 + 64,
                refuse: true,
            }
        }
    }

    impl Policy for Contractish {
        fn packable(&self, kind: u8) -> bool {
            kind == kind::RAW || kind == kind::TREE_NODE
        }
        fn max_body(&self, _kind: u8) -> usize {
            self.max
        }
        fn well_formed(&self, _kind: u8, body: &[u8]) -> bool {
            !(self.refuse && body.first() == Some(&0xBA))
        }
    }

    fn member(seed: u8, len: usize) -> (u8, Vec<u8>) {
        (
            kind::RAW,
            (0..len)
                .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
                .collect(),
        )
    }

    fn pack_of(n: u8) -> Vec<u8> {
        let members: Vec<_> = (0..n).map(|i| member(i, 1 + i as usize * 3)).collect();
        build(&members).expect("builds")
    }

    // ---- the format ------------------------------------------------------

    #[test]
    fn a_built_pack_is_accepted_by_the_walk_that_refuses_things() {
        // The control for every refusal below: they must fail for their own
        // reason and not because `check` refuses everything.
        for n in 1..=8u8 {
            let body = pack_of(n);
            assert_eq!(check(&body, &Contractish::default()), Ok(()), "n={n}");
            assert_eq!(members(&body).map(|m| m.len()), Ok(n as usize));
        }
    }

    #[test]
    fn a_pack_is_a_set_so_the_same_block_twice_is_one_member() {
        let a = member(1, 10);
        let body = build(&[a.clone(), a.clone(), a]).expect("builds");
        assert_eq!(members(&body).map(|m| m.len()), Ok(1));
    }

    #[test]
    fn the_order_is_by_block_id_so_two_writers_agree() {
        let ms = vec![member(3, 40), member(1, 9), member(2, 77)];
        let mut reversed = ms.clone();
        reversed.reverse();
        assert_eq!(build(&ms), build(&reversed));
    }

    #[test]
    fn a_pack_of_nothing_is_not_a_pack() {
        assert_eq!(build(&[]), Err(BuildError::Empty));
        let mut empty = Vec::from(&MAGIC[..]);
        empty.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(
            check(&empty, &Contractish::default()),
            Err(PackError::Empty)
        );
    }

    #[test]
    fn trailing_bytes_are_a_second_encoding_of_the_same_set() {
        let mut body = pack_of(2);
        body.push(0);
        assert_eq!(
            check(&body, &Contractish::default()),
            Err(PackError::TrailingBytes { left: 1 })
        );
    }

    #[test]
    fn members_out_of_order_are_refused() {
        // Built, then the two members swapped by hand: the bytes are otherwise
        // a pack, so only the order can be what refuses this.
        let ms = vec![member(1, 4), member(2, 4)];
        let body = build(&ms).expect("builds");
        let cut = HEADER + MIN_MEMBER + 4;
        let mut swapped = Vec::from(&body[..HEADER]);
        swapped.extend_from_slice(&body[cut..]);
        swapped.extend_from_slice(&body[HEADER..cut]);
        assert_eq!(swapped.len(), body.len());
        assert_eq!(
            check(&swapped, &Contractish::default()),
            Err(PackError::OutOfOrder { member: 1 })
        );
        // ... and the same bytes in the order build chose are fine, so the
        // swap is what this test measured.
        assert_eq!(check(&body, &Contractish::default()), Ok(()));
    }

    /// `build` de-duplicates, so the only way to reach this is by hand — and
    /// "strictly ascending" is two rules, not one: no duplicates AND no
    /// descent. A walk that only refused descent would accept two copies of
    /// one block, which is a second encoding of the same set.
    #[test]
    fn the_same_member_twice_is_refused() {
        let (k, b) = member(4, 6);
        let mut body = Vec::from(&MAGIC[..]);
        body.extend_from_slice(&2u16.to_le_bytes());
        for _ in 0..2 {
            body.push(k);
            body.extend_from_slice(&(b.len() as u32).to_le_bytes());
            body.extend_from_slice(&b);
        }
        assert_eq!(
            check(&body, &Contractish::default()),
            Err(PackError::OutOfOrder { member: 1 })
        );
        // The control: one copy of the same member is a pack, so the refusal
        // above is the repetition and not these bytes.
        let one = build(&[(k, b)]).expect("builds");
        assert_eq!(check(&one, &Contractish::default()), Ok(()));
    }

    #[test]
    fn a_count_that_cannot_fit_is_refused_from_two_lengths() {
        let mut body = Vec::from(&MAGIC[..]);
        body.extend_from_slice(&u16::MAX.to_le_bytes());
        body.extend_from_slice(&[0u8; 9]);
        assert_eq!(
            check(&body, &Contractish::default()),
            Err(PackError::CountDoesNotFit {
                count: 65535,
                have: 9
            })
        );
    }

    #[test]
    fn a_body_that_is_not_a_pack_is_refused_before_anything_else() {
        assert_eq!(
            check(b"", &Contractish::default()),
            Err(PackError::NotAPack)
        );
        assert_eq!(
            check(b"PK0", &Contractish::default()),
            Err(PackError::NotAPack)
        );
        assert_eq!(
            check(b"XX01\x01\x00", &Contractish::default()),
            Err(PackError::NotAPack)
        );
    }

    // ---- the policy, each part biting separately -------------------------

    #[test]
    fn the_policy_refuses_a_kind_it_does_not_want_in_a_pack() {
        let body = build(&[(9, vec![1, 2, 3])]).expect("builds");
        assert_eq!(
            check(&body, &Contractish::default()),
            Err(PackError::KindNotPackable { member: 0, kind: 9 })
        );
        // The control: the FORMAT has no opinion about kinds, so the same
        // bytes walk cleanly without a policy. Without this, the assertion
        // above would also pass if the format itself had refused kind 9.
        assert_eq!(members(&body).map(|m| m.len()), Ok(1));
    }

    #[test]
    fn the_policy_caps_a_member_from_its_length_field() {
        let body = build(&[member(1, 100)]).expect("builds");
        let tight = Contractish {
            max: 99,
            ..Default::default()
        };
        assert_eq!(
            check(&body, &tight),
            Err(PackError::MemberTooLong {
                member: 0,
                len: 100,
                max: 99
            })
        );
        // Exactly at the cap is accepted: a bound that refuses its own limit
        // is a different bound from the one documented.
        let at = Contractish {
            max: 100,
            ..Default::default()
        };
        assert_eq!(check(&body, &at), Ok(()));
    }

    #[test]
    fn the_policy_refuses_a_member_that_is_not_a_block_of_its_kind() {
        let body = build(&[(kind::RAW, vec![0xBA, 1, 2])]).expect("builds");
        assert_eq!(
            check(&body, &Contractish::default()),
            Err(PackError::MemberNotWellFormed {
                member: 0,
                kind: kind::RAW
            })
        );
        // The control: with that rule switched off the same bytes pass, so
        // the refusal above is the body check and not the length or the kind.
        let permissive = Contractish {
            refuse: false,
            ..Default::default()
        };
        assert_eq!(check(&body, &permissive), Ok(()));
    }

    // ---- what refusing COSTS ---------------------------------------------

    #[test]
    fn a_hostile_count_is_refused_without_hashing_anything() {
        work::reset();
        let mut body = Vec::from(&MAGIC[..]);
        body.extend_from_slice(&u16::MAX.to_le_bytes());
        body.extend_from_slice(&vec![0u8; 1024]);
        assert!(check(&body, &Contractish::default()).is_err());
        assert_eq!(work::ids(), 0, "a declared count bought hashing");
        assert_eq!(work::bodies(), 0);
    }

    #[test]
    fn an_unpackable_kind_is_refused_without_hashing_however_big_the_pack() {
        // One honest member, then a megabyte under a kind the policy refuses.
        // The honest one is hashed; the hostile one must cost nothing.
        let big = vec![7u8; 900 * 1024];
        let body = build(&[member(1, 8), (9, big)]).expect("builds");
        work::reset();
        assert!(matches!(
            check(&body, &Contractish::default()),
            Err(PackError::KindNotPackable { .. })
        ));
        let ids = work::ids();
        assert!(ids <= 1, "{ids} ids hashed to refuse a kind byte");
        // The control: the same pack under a policy that accepts kind 9 pays
        // for both, so the number above is the refusal saving work and not the
        // counter being broken.
        struct Wide;
        impl Policy for Wide {
            fn packable(&self, _k: u8) -> bool {
                true
            }
            fn max_body(&self, _k: u8) -> usize {
                MAX_PACK
            }
            fn well_formed(&self, _k: u8, _b: &[u8]) -> bool {
                true
            }
        }
        work::reset();
        assert_eq!(check(&body, &Wide), Ok(()));
        assert_eq!(work::ids(), 2);
        assert_eq!(work::bodies(), 2);
    }

    #[test]
    fn an_over_long_member_is_refused_from_the_length_field_alone() {
        let body = build(&[(kind::RAW, vec![3u8; 600 * 1024])]).expect("builds");
        work::reset();
        assert!(matches!(
            check(&body, &Contractish::default()),
            Err(PackError::MemberTooLong { .. })
        ));
        assert_eq!(work::ids(), 0, "a length field bought a hash");
        assert_eq!(work::bodies(), 0);
    }

    // ---- the two doors agree ---------------------------------------------

    #[test]
    fn what_the_format_accepts_and_what_it_yields_are_the_same_walk() {
        struct Wide;
        impl Policy for Wide {
            fn packable(&self, _k: u8) -> bool {
                true
            }
            fn max_body(&self, _k: u8) -> usize {
                MAX_PACK
            }
            fn well_formed(&self, _k: u8, _b: &[u8]) -> bool {
                true
            }
        }
        // Every prefix of a real pack, plus the pack itself: `check` under the
        // format-only policy and `members` must agree on every one of them,
        // whether they accept or refuse.
        let body = pack_of(6);
        for cut in 0..=body.len() {
            let slice = &body[..cut];
            assert_eq!(
                check(slice, &Wide).is_ok(),
                members(slice).is_ok(),
                "the two doors disagree at {cut} bytes"
            );
        }
        // And it is not vacuous: some prefixes are accepted and some are not.
        assert!(members(&body).is_ok());
        assert!(members(&body[..body.len() - 1]).is_err());
    }

    // ---- nothing panics ---------------------------------------------------

    #[test]
    fn no_input_panics() {
        let good = pack_of(5);
        let mut inputs: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"P".to_vec(),
            b"PK01".to_vec(),
            b"PK01\xff\xff".to_vec(),
            vec![0u8; HEADER + MIN_MEMBER],
            vec![0xff; 1024],
        ];
        // Every prefix, and every single-byte corruption at a few offsets: a
        // parser is handed bytes by strangers, and "it did not panic on the
        // inputs I thought of" is not the claim being made here.
        for cut in 0..=good.len() {
            inputs.push(good[..cut].to_vec());
        }
        for at in 0..good.len().min(64) {
            let mut bad = good.clone();
            bad[at] ^= 0xff;
            inputs.push(bad);
        }
        for i in &inputs {
            let _ = check(i, &Contractish::default());
            let _ = members(i);
            let _ = well_formed(i, &Contractish::default());
        }
    }

    // ---- build's own bounds ----------------------------------------------

    #[test]
    fn build_refuses_a_body_it_could_not_express() {
        // Over MAX_PACK: the caller learns from its own input rather than from
        // a host refusing the commit later.
        let big = vec![0u8; MAX_PACK];
        assert!(matches!(
            build(&[(kind::RAW, big)]),
            Err(BuildError::TooLarge(_))
        ));
    }

    #[test]
    fn build_and_the_walk_agree_about_max_pack() {
        // One byte under the ceiling, with the header and member overhead
        // accounted for, builds and walks; the size boundary is a place two
        // implementations of one format drift.
        let body_len = MAX_PACK - HEADER - MIN_MEMBER;
        let body = build(&[(kind::RAW, vec![1u8; body_len])]).expect("builds at the ceiling");
        assert_eq!(body.len(), MAX_PACK);
        struct Huge;
        impl Policy for Huge {
            fn packable(&self, _k: u8) -> bool {
                true
            }
            fn max_body(&self, _k: u8) -> usize {
                MAX_PACK
            }
            fn well_formed(&self, _k: u8, _b: &[u8]) -> bool {
                true
            }
        }
        assert_eq!(check(&body, &Huge), Ok(()));
    }
}
