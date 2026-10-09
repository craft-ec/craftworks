//! Does the engine build the bytes the Block contract defines?
//!
//! `engine/src/pack.rs` is a second implementation of a format whose authority
//! lives in `freenet-contracts/block/src/pack.rs`. Two spellings of one format
//! drift, and the drift does not show up here — it shows up as a host refusing
//! a commit, in production, with nothing to point at.
//!
//! So this is a differential, not a self-test: the vectors were produced BY
//! the contract's own `build`, and the engine has to reproduce them byte for
//! byte. A test that checked the engine against its own output would pass
//! forever no matter how far the two drifted.

use engine::pack;

fn member(kind: u8, seed: u8, len: usize) -> (u8, Vec<u8>) {
    (
        kind,
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect(),
    )
}

/// A named set of members: what one vector covers.
type Case = (&'static str, Vec<(u8, Vec<u8>)>);

/// The same cases the emitter in freenet-contracts uses, in the same order.
fn cases() -> Vec<Case> {
    let raw = freenet_prolly::kind::RAW;
    vec![
        ("one-empty", vec![member(raw, 0, 0)]),
        ("one-small", vec![member(raw, 7, 13)]),
        (
            "three-unordered",
            vec![member(raw, 3, 40), member(raw, 1, 9), member(raw, 2, 77)],
        ),
        (
            "duplicates",
            vec![member(raw, 5, 20), member(raw, 5, 20), member(raw, 6, 4)],
        ),
        (
            "many",
            (0..12u8)
                .map(|i| member(raw, i, 1 + i as usize * 3))
                .collect(),
        ),
    ]
}

use core_types::hex::encode as hex;

#[test]
fn the_engine_builds_the_bytes_the_contract_defines() {
    let text = include_str!("pack_vectors.txt");
    let want: Vec<(&str, &str)> = text
        .lines()
        .filter_map(|l| l.strip_prefix("VEC "))
        .filter_map(|l| l.split_once(' '))
        .collect();
    assert!(
        !want.is_empty(),
        "no vectors were read; this test would pass over nothing"
    );

    let cases = cases();
    assert_eq!(
        want.len(),
        cases.len(),
        "the vector file has {} case(s) and this test has {} — one of them \
         has been changed without the other",
        want.len(),
        cases.len()
    );

    for ((name, members), (vname, vhex)) in cases.iter().zip(&want) {
        assert_eq!(name, vname, "the cases are not in the same order");
        let body = pack::build(members).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(
            hex(&body),
            *vhex,
            "{name}: the engine's pack does not match the bytes the Block \
             contract produces for the same members"
        );
    }
    println!(
        "  {} pack vector(s) match the contract byte for byte",
        want.len()
    );
}

/// The manifest round-trips, and a truncated or over-declared one is refused.
///
/// Recovery reads this after a restart with nothing else to go on, so a
/// manifest that decodes to something plausible but wrong is a commit replayed
/// against the wrong head.
#[test]
fn a_commit_manifest_round_trips_and_refuses_a_malformed_one() {
    let m = pack::Manifest {
        prev_seq: 7,
        prev_root: [9u8; 32],
        seq: 8,
        root: [4u8; 32],
    };
    let bytes = m.encode();
    assert_eq!(pack::Manifest::decode(&bytes).as_ref(), Some(&m));

    for cut in [0, 1, 4, 40, bytes.len() - 1] {
        assert!(
            pack::Manifest::decode(&bytes[..cut]).is_none(),
            "a manifest truncated to {cut} byte(s) decoded anyway"
        );
    }
    // Trailing bytes are a second encoding of the same commit.
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(
        pack::Manifest::decode(&extra).is_none(),
        "a manifest with trailing bytes decoded anyway"
    );
    // FIXED WIDTH now that the owed set is gone (craftworks-sdk#117), so the
    // length itself is the check: a manifest is exactly this long.
    assert_eq!(bytes.len(), 4 + 8 + 32 + 8 + 32, "the manifest is fixed width");
}

/// THE CHECK HAS TO FIRE.
///
/// Before craftworks-sdk#117 `PackError::TooLarge` was not merely unfired but
/// UNFIREABLE: `max_body` appeared nowhere in the SDK, so nothing compared a
/// member to its kind's limit and an oversized pack was built in silence and
/// refused at the NODE — remotely, where a node can log nothing at the moment
/// it refuses.
///
/// A size check that no test fires is that same defect one level up, so this
/// drives the boundary from both sides for every kind that has its own limit.
/// The limit is per KIND, not per pack: packed members are `RAW`, so the
/// ceiling that bites one is 262,208 and not the pack's 1 MiB.
#[test]
fn an_oversized_member_is_refused_here_rather_than_at_the_node() {
    use core_types::kind::BlockKind;
    for bk in [BlockKind::Raw, BlockKind::Parity] {
        let (kind, limit) = (bk.byte(), pack::max_body(bk));

        // Exactly at the limit is ACCEPTED. Without this the check could be
        // off by one in the safe direction and refuse what the network takes.
        pack::build(&[member(kind, 1, limit)])
            .unwrap_or_else(|e| panic!("kind {kind} at its limit of {limit} B must build: {e:?}"));

        // One byte over is refused, and the error says WHY — it is rendered
        // into a panic message by the only caller, so what it carries is the
        // whole diagnosis available at the moment it fires.
        match pack::build(&[member(kind, 1, limit + 1)]) {
            Err(pack::PackError::TooLarge { kind: k, len, limit: l }) => {
                assert_eq!((k, len, l), (kind, limit + 1, limit));
            }
            other => panic!("kind {kind} one byte over {limit} must be refused, got {other:?}"),
        }
    }
}

/// A PACK-kind member at ITS limit (1 MiB) cannot ride in a pack: the pack
/// around it is over the format's `MAX_PACK`, which the contract applies to
/// every block's state (`MAX_STATE = 1 + MAX_PACK`). Before the format came
/// from `freenet_prolly::pack` (sdk#305) the engine checked members only, and
/// built this pack in silence for the node to refuse. Both sides of the
/// member's own limit are refused here, each naming its reason.
#[test]
fn a_pack_sized_member_is_refused_by_the_pack_around_it() {
    let limit = pack::max_body(core_types::kind::BlockKind::Pack);
    match pack::build(&[member(pack::PACK_KIND, 1, limit)]) {
        Err(pack::PackError::TooLarge { kind, len, limit: l }) => {
            assert_eq!((kind, l), (pack::PACK_KIND, pack::MAX_PACK));
            assert!(len > pack::MAX_PACK, "the reason is the whole pack: {len}");
        }
        other => panic!("a pack carrying a {limit} B member must be refused, got {other:?}"),
    }
    match pack::build(&[member(pack::PACK_KIND, 1, limit + 1)]) {
        Err(pack::PackError::TooLarge { kind, len, limit: l }) => {
            assert_eq!((kind, len, l), (pack::PACK_KIND, limit + 1, limit));
        }
        other => panic!("one byte over the member limit must be refused, got {other:?}"),
    }
}

/// The limits are the contract's, mirrored rather than imported, so a test has
/// to hold them to the numbers rather than to the constant's own definition —
/// which would agree with itself whatever it drifted to.
#[test]
fn the_mirrored_limits_are_the_contracts_numbers() {
    assert_eq!(pack::MAX_BODY, 262_208);
    assert_eq!(pack::MAX_PACK, 1_048_576);
    assert_eq!(pack::MAX_PARITY, 262_213);
    assert_eq!(pack::max_body(core_types::kind::BlockKind::Raw), 262_208);
    assert_eq!(pack::max_body(core_types::kind::BlockKind::Pack), 1_048_576);

    // And the KINDS the dispatch turns on. `max_body` selects by kind, so the
    // limits being right is worth nothing if the engine and the contract
    // disagree about which byte means PARITY — the mirror would then be
    // correct and still applied to the wrong member. The contract spells these
    // in its own `kind` module; the engine reads freenet-prolly's.
    assert_eq!(freenet_prolly::kind::RAW, 0);
    assert_eq!(freenet_prolly::kind::PARITY, 4);
    assert_eq!(pack::PACK_KIND, 6);
}

/// ONE FORMAT (sdk#305, the owner's rule 3): `pack.rs` keeps the engine's
/// POLICY (the PACK kind, each kind's ceiling, the CM01 manifest) and takes the
/// pack FORMAT from `freenet_prolly::pack`. It was a second implementation of
/// the same bytes once; this reads its source so that it cannot quietly become
/// one again.
#[test]
fn the_pack_format_is_freenet_prollys_not_a_second_copy() {
    let src = include_str!("../src/pack.rs");
    let body = |sig: &str| -> &str {
        let at = src.find(sig).unwrap_or_else(|| panic!("`{sig}` is not in pack.rs"));
        let rest = &src[at..];
        &rest[..rest.find("\n}\n").expect("the function ends")]
    };
    // The CONTROL: the reader finds the real bodies, so an empty read cannot
    // pass the checks below.
    let (build, members) = (body("pub fn build("), body("pub fn members("));
    assert!(build.contains("max_body") && members.contains("Vec<(Cid, Vec<u8>)>"), "the reader did not find the real bodies");

    // The magic has no name here at all (sdk#305: nothing read one), so the
    // literal is what a second copy would have to spell.
    assert!(!src.contains("b\"PK01\""), "pack.rs spells the pack magic itself");
    for (name, want) in [
        ("pub const PACK_HEADER", "format::HEADER"),
        ("pub const MAX_PACK", "format::MAX_PACK"),
    ] {
        let line = src.lines().find(|l| l.starts_with(name)).unwrap_or_else(|| panic!("no `{name}`"));
        assert!(line.contains(want), "`{name}` is defined locally, not from `{want}`: {line}");
    }
    assert!(build.contains("format::build("), "pack::build writes the bytes itself");
    assert!(members.contains("format::members("), "pack::members walks the bytes itself");
    assert!(!src.contains("split_at_checked"), "pack.rs walks a pack body itself");

    // And the bytes agree: the engine's pack IS the library's pack.
    let ms = [member(freenet_prolly::kind::RAW, 3, 900), member(freenet_prolly::kind::RAW, 9, 40)];
    let ours = pack::build(&ms).expect("builds");
    assert_eq!(ours, freenet_prolly::pack::build(&ms).expect("builds"));
    assert_eq!(pack::members(&ours).len(), 2);
}
