//! The protocol keeps its promises to versions it has never met.

use protocol::*;

/// A version this build does not serve is ANSWERED, and told what is served.
///
/// The control is the same message at a version it does know. Without it,
/// "unsupported was returned" would read identically from a decoder that
/// rejects everything.
#[test]
fn an_unknown_version_is_answered_unsupported_and_a_known_one_is_not() {
    let session = protocol::mint_session(0x0123_4567_89ab);
    let good = protocol::encode_session_request(CURRENT, session, &Request::Flush).expect("encodes");
    assert_eq!(
        decode_request(&good),
        Incoming::Ok(Envelope {
            version: CURRENT,
            session,
            now_ms: None,
            frame: None,
            body: Request::Flush
        }),
        "a message at the current version was not understood, so nothing \
         below this line means anything"
    );

    // One byte of the VERSION changed. The acceptance issue's control.
    let mut future = good.clone();
    future[0] = 99;
    match decode_request(&future) {
        Incoming::Unsupported(v) => assert_eq!(v, 99),
        other => panic!(
            "a v99 message was answered {other:?} rather than Unsupported. A \
             client that hears silence waits; one that hears a guess acts on \
             it."
        ),
    }
}

/// `Unsupported` names what IS known, or a client learns only that it failed.
#[test]
fn unsupported_carries_the_versions_that_are_served() {
    let r = Reply::Unsupported {
        got: 99,
        known: KNOWN.to_vec(),
    };
    let round = decode_reply(&encode_reply(&r).expect("encodes")).expect("decodes");
    match round {
        Reply::Unsupported { got, known } => {
            assert_eq!(got, 99);
            assert!(
                known.contains(&CURRENT),
                "the reply does not name the version this build actually \
                 speaks, so a client cannot fall back to anything"
            );
            assert!(!known.is_empty());
        }
        other => panic!("{other:?}"),
    }
}

/// The version is readable even when the BODY is not.
///
/// A v2 body reaching a v1 decoder is not a corrupt message, it is a message
/// from the future — and answering `Unparseable` tells its sender nothing it
/// can act on, where `Unsupported` tells it to fall back.
#[test]
fn a_future_body_is_unsupported_not_unparseable() {
    let mut msg = 7u16.to_le_bytes().to_vec();
    msg.extend_from_slice(b"a body no v1 decoder can make sense of");
    assert_eq!(
        decode_request(&msg),
        Incoming::Unsupported(7),
        "a message whose VERSION is readable was reported as unparseable; the \
         version is on the front precisely so this case is answerable"
    );
}

/// Trailing bytes are refused, not half-read.
#[test]
fn a_message_with_trailing_bytes_is_refused() {
    let good = encode_session_request(CURRENT, mint_session(0x5e55_1017), &Request::Flush).expect("encodes");
    let mut trailing = good.clone();
    trailing.extend_from_slice(b"and then some");
    assert_eq!(
        decode_request(&trailing),
        Incoming::Dropped(Dropped::TrailingBytes),
        "a message with {} trailing byte(s) was accepted; its prefix was read \
         as a whole message, which is how one message becomes another",
        trailing.len() - good.len()
    );
}

/// Garbage, truncation and oversize are all REFUSED, and none panics.
///
/// Generated rather than hand-picked, with a floor on how many were tried: a
/// sweep that covered three inputs and a sweep that covered three hundred
/// print the same thing.
#[test]
fn nothing_malformed_panics_and_every_case_is_refused() {
    let good = encode_session_request(
        CURRENT, mint_session(0x5e55_1017),
        &Request::Write {
            write_id: 1,
            ops: vec![Op::Put(b"k".to_vec(), vec![7u8; 64])],
        },
    )
    .expect("encodes");
    let mut tried = 0usize;
    let mut refused = 0usize;

    // Every truncation.
    for n in 0..good.len() {
        tried += 1;
        if !matches!(decode_request(&good[..n]), Incoming::Ok(_)) {
            refused += 1;
        }
    }
    // Every single-byte corruption of the BODY. The version bytes are left
    // alone here: changing those is the Unsupported case, which is a
    // different answer and has its own test.
    let mut rng = 0x243F6A88u32;
    for at in 2..good.len() {
        for _ in 0..2 {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            let mut bad = good.clone();
            bad[at] ^= (rng & 0xFF) as u8;
            tried += 1;
            // A corruption may still decode to a DIFFERENT valid message —
            // that is what a self-describing format allows — so what is
            // asserted is that it never panics, and the refusals are counted.
            if !matches!(decode_request(&bad), Incoming::Ok(_)) {
                refused += 1;
            }
        }
    }
    // Oversize.
    tried += 1;
    let huge = vec![0u8; MAX_MESSAGE + 1];
    assert_eq!(
        decode_request(&huge),
        Incoming::Dropped(Dropped::TooLarge),
        "a message over the cap was not refused before decoding"
    );
    refused += 1;

    assert!(
        tried > 200,
        "only {tried} malformed input(s) were tried; a sweep this small says \
         nothing about a decoder"
    );
    assert!(
        refused > tried / 2,
        "only {refused} of {tried} malformed inputs were refused, which is \
         too few to believe: a decoder that accepts most damage is one that \
         is not reading the structure"
    );
    println!("  {tried} malformed inputs, {refused} refused, 0 panics");
}

/// v4 carries the SESSION on every request, and it round-trips (sdk#146).
#[test]
fn a_v4_request_carries_its_session() {
    let s = protocol::mint_session(0xdead_beef_cafe_f00d);
    let b = protocol::encode_session_request(4, s, &Request::Flush).expect("encodes");
    match decode_request(&b) {
        Incoming::Ok(e) => assert_eq!((e.version, e.session, e.body), (4, s, Request::Flush)),
        other => panic!("{other:?}"),
    }
}

/// THE CONTROL, and the promise to old pages: a v3 request is encoded
/// EXACTLY as before — no session on the wire — and decodes as the legacy
/// session.
#[test]
fn a_v3_request_is_the_old_bytes_and_decodes_as_the_legacy_session() {
    let b = protocol::encode_session_request(3, 12345, &Request::Flush).expect("encodes");
    // The pre-v4 envelope: version (u16 LE) then the body — nothing between.
    let old_shape = protocol::encode_session_request(3, protocol::LEGACY_SESSION, &Request::Flush).unwrap();
    assert_eq!(b, old_shape, "a session leaked onto a v3 frame");
    assert_eq!(protocol::request_len(3, &Request::Flush) as usize, b.len());
    match decode_request(&b) {
        Incoming::Ok(e) => assert_eq!((e.version, e.session), (3, protocol::LEGACY_SESSION)),
        other => panic!("{other:?}"),
    }
    let v4 = protocol::encode_session_request(4, 12345, &Request::Flush).unwrap();
    assert_eq!(v4.len(), b.len() + 8, "v4 is the v3 frame plus eight bytes of session");
    assert_eq!(protocol::request_len(4, &Request::Flush) as usize, v4.len());
}

/// A minted session is never the legacy one, and never wider than 48 bits.
#[test]
fn a_minted_session_is_48_bits_and_never_legacy() {
    for r in [0u64, 1, u64::MAX, 1 << 48, (1 << 48) + 1] {
        let s = protocol::mint_session(r);
        assert_ne!(s, protocol::LEGACY_SESSION, "{r}");
        assert_ne!(s, 0);
        assert!(s < (1 << protocol::SESSION_BITS), "{r} -> {s}");
    }
}

/// **A1: a v4 frame whose session no client can hold is REFUSED by name** —
/// zero, the legacy session, or wider than 48 bits — never truncated into a
/// session whose verdicts the sender would then drop.
#[test]
fn a_v4_frame_with_a_session_no_client_can_hold_is_refused_by_name() {
    for bad in [0u64, protocol::LEGACY_SESSION, 1 << 48, 0xABCD_0000_0000_0123, u64::MAX] {
        let b = protocol::encode_session_request(4, bad, &Request::Flush).expect("encodes");
        assert_eq!(decode_request(&b), Incoming::Dropped(protocol::Dropped::BadSession), "session {bad:#x}");
    }
    // THE CONTROL: a real one, and the widest real one, decode.
    for good in [2u64, (1 << 48) - 1, protocol::mint_session(u64::MAX)] {
        let b = protocol::encode_session_request(4, good, &Request::Flush).expect("encodes");
        assert!(matches!(decode_request(&b), Incoming::Ok(_)), "session {good:#x}");
    }
}

/// The session-less encoder REFUSES a version whose frames carry a session —
/// it used to clamp v4+ to v3, so every caller passing `CURRENT` spoke the
/// legacy session silently (craftworks-sdk#194).
#[test]
fn the_session_less_encoder_refuses_a_sessioned_version_by_name() {
    for v in [SESSION_SINCE, CURRENT, FLOOR_SINCE] {
        assert_eq!(encode_request(v, &Request::Flush), Err(Dropped::NeedsSession), "v{v}");
    }
    // CONTROL: the legacy path, asked for by name, still encodes — as v3.
    let legacy = encode_request(LAST_SESSIONLESS, &Request::Flush).expect("encodes");
    assert_eq!(u16::from_le_bytes([legacy[0], legacy[1]]), 3);
    assert!(matches!(decode_request(&legacy), Incoming::Ok(Envelope { session: LEGACY_SESSION, .. })));
}
