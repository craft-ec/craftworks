//! What happens when the two sides of the wire disagree about what exists.
//!
//! v2 adds `Reply::CallBytes`. Both directions are tested, because they fail
//! differently and only one of them is loud.
//!
//! The failure that matters is not a crash. It is a reader that decodes
//! something into the WRONG THING and carries on — the same silent
//! re-interpretation as a data file whose meaning changed while every field
//! stayed where it was.

use protocol::{decode_reply, decode_request, encode_reply, encode_request, Dropped, Incoming};
use protocol::{Reply, Request};

/// NEWER WRITER, OLDER READER: a message from the future must FAIL to decode,
/// not decode into something else.
///
/// This is the whole reason `CallBytes` is a new VARIANT and not new fields on
/// `Reply::Call`. Appending a field changes an existing message: a reader that
/// does not know about it does not error, it reads a different message.
/// Appending a variant gives it a new wire tag, and an unknown tag cannot be
/// mistaken for a known one.
#[test]
fn a_reply_from_a_newer_build_is_refused_rather_than_misread() {
    let mut bytes = encode_reply(&Reply::CallBytes {
        put_bytes: 101_641,
        puts: 2,
        gets: 1,
    })
    .expect("encodes");
    // bincode writes the variant index at the front. Push it past every
    // variant this build has — which is exactly what an older reader sees.
    bytes[0] = bytes[0].wrapping_add(50);

    match decode_reply(&bytes) {
        Err(Dropped::Unparseable) | Err(Dropped::TrailingBytes) => {}
        Err(other) => panic!("refused, but for the wrong reason: {other:?}"),
        Ok(r) => panic!(
            "a message from a newer build decoded as an EXISTING variant. \
             That is the silent misreading the variant scheme exists to \
             prevent: {r:?}"
        ),
    }
}

/// OLDER WRITER, NEWER READER: every message that existed before still decodes
/// to exactly what it was.
///
/// The half that would break silently if someone INSERTED a variant rather
/// than appending one — nothing about the new message would look wrong, and
/// every older one would quietly become its neighbour.
#[test]
fn every_older_reply_still_decodes_to_itself() {
    let olds = [
        Reply::Unsupported {
            got: 99,
            known: protocol::KNOWN.to_vec(),
        },
        Reply::Dropped {
            reason: Dropped::TooLarge,
        },
    ];
    for r in olds {
        let round = decode_reply(&encode_reply(&r).expect("encodes"))
            .expect("an older reply still decodes");
        assert_eq!(round, r, "a variant was renumbered by the append");
    }
}

/// A v1 REQUEST is still served, and is answered at the version it asked in.
///
/// Bumping CURRENT must not strand a client that has not been updated: v1 is
/// still in KNOWN, so a v1 request is understood rather than refused.
#[test]
fn a_v1_request_is_still_understood_after_the_bump() {
    assert!(protocol::KNOWN.contains(&1), "v1 must remain served");
    assert!(protocol::KNOWN.contains(&2), "v2 must remain served");
    assert!(protocol::KNOWN.contains(&3), "v3 must remain served");
    assert_eq!(protocol::CURRENT, 4);

    let v1 = encode_request(1, &Request::Flush).expect("encodes");
    match decode_request(&v1) {
        Incoming::Ok(env) => {
            assert_eq!(env.version, 1, "the version is READ, not assumed");
            assert_eq!(env.body, Request::Flush);
        }
        other => panic!("a v1 request was not understood: {other:?}"),
    }
}

/// And a version nobody serves is still answered rather than guessed at.
#[test]
fn an_unserved_version_is_still_refused_with_the_list() {
    let mut future = protocol::encode_session_request(protocol::CURRENT, protocol::mint_session(0x5e55_1017), &Request::Flush).expect("encodes");
    future[0] = 99;
    match decode_request(&future) {
        Incoming::Unsupported(v) => assert_eq!(v, 99),
        other => panic!("a v99 request was answered {other:?} rather than Unsupported"),
    }
}

/// The new message round-trips, so the tests above are not passing because
/// nothing was added.
#[test]
fn the_new_reply_round_trips() {
    let r = Reply::CallBytes {
        put_bytes: 101_641,
        puts: 2,
        gets: 1,
    };
    assert_eq!(
        decode_reply(&encode_reply(&r).expect("encodes")).expect("decodes"),
        r
    );
}

// ---------------------------------------------------------------------------
// v3: `WriteState::TooLarge` (craftworks-sdk#136), and whatever joins it.
//
// Written over `WriteState::NEWER_THAN_V2`, so a state added later (sdk#143's
// `Conflict`) is covered by listing it there: the decoder test, the downgrade
// test and the delegate's one conversion point all iterate or call the same
// two functions, `since` and `for_client`.
// ---------------------------------------------------------------------------

/// A v2 build's `WriteState`, FROZEN: the variants it knew, in wire order.
/// What an old page's decoder is. Never edit it to match the current enum --
/// that would test the current build against itself.
#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
enum WriteStateV2 {
    Accepted,
    Stalled,
    Published,
    ParityComplete,
    Busy,
    Failed,
    Lost,
}

#[test]
fn every_state_newer_than_v2_is_refused_by_a_v2_decoder_not_misread() {
    use bincode::Options;
    let v2 = || bincode::DefaultOptions::new().with_fixint_encoding();
    assert!(
        !protocol::WriteState::NEWER_THAN_V2.is_empty(),
        "nothing listed: this test checks nothing"
    );
    for state in protocol::WriteState::NEWER_THAN_V2 {
        let bytes = v2().serialize(state).expect("a state encodes");
        match v2().deserialize::<WriteStateV2>(&bytes) {
            Err(_) => {}
            Ok(read) => panic!("a v2 decoder read {state:?} as {read:?} -- the silent misreading"),
        }
    }
    // CONTROL: the same frozen decoder reads every v2 state it knows, so the
    // refusal above is about the new variant, not a broken decoder.
    for (s, name) in [
        (protocol::WriteState::Failed, "Failed"),
        (protocol::WriteState::Busy, "Busy"),
    ] {
        let bytes = v2().serialize(&s).expect("encodes");
        let read = v2()
            .deserialize::<WriteStateV2>(&bytes)
            .expect("a v2 state decodes");
        assert_eq!(format!("{read:?}"), name);
    }
}

#[test]
fn an_older_client_is_told_what_it_can_read_and_a_v3_client_the_state_itself() {
    // The order rule's verdicts are never sent below v5 — their downgrade is a
    // debug assertion, tested on its own in tests/v5.rs.
    for state in protocol::WriteState::NEWER_THAN_V2.iter().filter(|s| !protocol::WriteState::ORDER_RULE.contains(s)) {
        assert!(
            state.since() > 2,
            "{state:?} is listed as newer than v2 but says since {}",
            state.since()
        );
        for old in 1..state.since() {
            let told = state.for_client(old);
            // `Unknown` (R-b, COMMIT-LIFE ⁵: may have landed, check) has NO
            // older state that means the same. It is told `Stalled`, which
            // claims nothing: an older client neither rolls it back (`Lost`,
            // `Failed`) nor sends it again -- the one safe wrong answer. Named
            // here, not by loosening the rule below for every state.
            if *state == protocol::WriteState::Unknown {
                assert_eq!(told, protocol::WriteState::Stalled, "a v{old} client is told {told:?} for Unknown");
                continue;
            }
            assert!(
                told.since() <= old,
                "a v{old} client would be sent {told:?}, which it cannot read"
            );
            assert!(
                told.terminal() == state.terminal()
                    && told.should_resubmit() == state.should_resubmit(),
                "a v{old} client is told {told:?}, which does not MEAN what {state:?} means"
            );
        }
        assert_eq!(state.for_client(protocol::CURRENT), *state);
    }
    assert!(protocol::KNOWN.contains(&protocol::CURRENT));
}

#[test]
fn counts_saturate_rather_than_wrap() {
    assert_eq!(protocol::saturating_u32(7), 7);
    assert_eq!(protocol::saturating_u32(u32::MAX as usize), u32::MAX);
    #[cfg(target_pointer_width = "64")]
    {
        assert_eq!(
            protocol::saturating_u32((1usize << 32) + 5),
            u32::MAX,
            "2^32 + 5 wrapped to 5 would read as under a limit of 128"
        );
        // Through the constructor the delegate uses -- the one place the
        // engine's counts are narrowed onto the wire.
        assert_eq!(
            protocol::WriteState::too_large(
                protocol::WriteBound::CommitBlocks,
                128,
                (1usize << 32) + 5
            ),
            protocol::WriteState::TooLarge {
                bound: protocol::WriteBound::CommitBlocks,
                limit: 128,
                got: u32::MAX
            }
        );
    }
}

#[test]
fn a_request_over_the_limit_is_an_error_not_an_empty_frame() {
    let huge = Request::Write {
        write_id: 1,
        ops: vec![protocol::Op::Put(
            b"k".to_vec(),
            vec![0u8; protocol::MAX_MESSAGE],
        )],
    };
    assert_eq!(
        protocol::encode_session_request(protocol::CURRENT, protocol::mint_session(0x5e55_1017), &huge),
        Err(Dropped::TooLarge)
    );
    assert!(protocol::request_len(protocol::CURRENT, &huge) > protocol::MAX_MESSAGE as u64);
    // CONTROL: one that fits encodes, and `request_len` agrees with it.
    let small = Request::Write {
        write_id: 1,
        ops: vec![protocol::Op::Put(b"k".to_vec(), vec![0u8; 10])],
    };
    // At CURRENT a frame carries a session (sdk#146): the length is of the
    // frame a client WITH one sends.
    let bytes = protocol::encode_session_request(protocol::CURRENT, protocol::mint_session(7), &small).expect("fits");
    assert_eq!(
        bytes.len() as u64,
        protocol::request_len(protocol::CURRENT, &small)
    );
}

/// What an unencodable reply is replaced with must itself always encode.
#[test]
fn a_dropped_reply_always_encodes() {
    for reason in [
        Dropped::TooLarge,
        Dropped::Unparseable,
        Dropped::TrailingBytes,
        Dropped::Unexpected,
        Dropped::NotForUs,
    ] {
        let bytes = encode_reply(&Reply::Dropped { reason }).expect("a Dropped reply encodes");
        assert!(bytes.len() < 64);
    }
}
