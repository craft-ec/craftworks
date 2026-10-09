//! Wire v5 — the write path's wire (WRITE-PATH.md revision 3, craftworks-sdk#183).
//! Types only: nothing here decides anything. What is pinned is the BYTES, so
//! the engine (build step 2) and the client (step 3) are written against a
//! wire that cannot drift under them.

use protocol::*;

const S: u64 = 0x0000_1234_5678_9abc;

fn hex(s: &str) -> Vec<u8> {
    core_types::hex::decode(s).expect("hex")
}

fn ack() -> Ack {
    Ack { session: S, published_through: 5, taken_through: 6, parity: Some(Parity { owed_groups: 2, at_seq: 11 }), frames_seen: 8, answering: Some(8) }
}

// ------------------------------------------------------------ frozen vectors
//
// FROZEN: written from this build once, and never regenerated to match a
// later one — a vector that follows the code tests the code against itself.

/// v5 write: version 5, session, `now` 1_790_000_000_123 ms, frame 7,
/// `WriteFrom { write_id: 9, floor: 4, ops: [Put k=v, Delete g] }`.
const V5_WRITE_FROM: &str = "0500bc9a7856341200007b6c50c4a001000007000000000000000e0000000900000000000000040000000000000002000000000000000000000001000000000000006b01000000000000007601000000010000000000000067";
/// v5 tick: frame 8, `Tick { now: 1_790_000_000 }`.
const V5_TICK: &str = "0500bc9a7856341200007b6c50c4a0010000080000000000000008000000803bb16a00000000";
/// A v4 `Flush`: no `now`, no frame on the wire.
const V4_FLUSH: &str = "0400bc9a78563412000007000000";
/// `Acked { ack, SessionWriteState { w4: Duplicate } }`.
const ACKED_DUPLICATE: &str = "10000000bc9a785634120000050000000000000006000000000000000102000000000000000b0000000000000008000000000000000108000000000000000f000000bc9a785634120000040000000000000008000000";
/// `Acked { ack, SessionWriteState { w9: OutOfOrder { expected: 7 } } }`.
const ACKED_OUT_OF_ORDER: &str = "10000000bc9a785634120000050000000000000006000000000000000102000000000000000b0000000000000008000000000000000108000000000000000f000000bc9a7856341200000900000000000000090000000700000000000000";

fn write_from() -> Request {
    Request::WriteFrom { write_id: 9, floor: 4, ops: vec![Op::Put(b"k".to_vec(), b"v".to_vec()), Op::Delete(b"g".to_vec())] }
}

#[test]
fn a_v5_write_encodes_to_its_frozen_bytes_and_back() {
    let bytes = encode_v5_request(S, 1_790_000_000_123, 7, &write_from()).expect("encodes");
    assert_eq!(bytes, hex(V5_WRITE_FROM), "the v5 write's bytes moved");
    assert_eq!(
        decode_request(&hex(V5_WRITE_FROM)),
        Incoming::Ok(Envelope { version: 5, session: S, now_ms: Some(1_790_000_000_123), frame: Some(7), body: write_from() })
    );
    let tick = encode_v5_request(S, 1_790_000_000_123, 8, &Request::Tick { now: 1_790_000_000 }).expect("encodes");
    assert_eq!(tick, hex(V5_TICK), "the v5 envelope's bytes moved");
}

/// A v4 frame decodes with NO `now` and NO frame — `None`, never 0.
#[test]
fn a_v4_frame_carries_no_now_and_no_frame() {
    assert_eq!(encode_session_request(4, S, &Request::Flush).expect("encodes"), hex(V4_FLUSH), "the v4 frame's bytes moved");
    match decode_request(&hex(V4_FLUSH)) {
        Incoming::Ok(e) => {
            assert_eq!((e.version, e.session), (4, S));
            assert_eq!(e.now_ms, None, "a v4 frame was given a clock it does not carry");
            assert_eq!(e.frame, None, "a v4 frame was given a sequence it does not carry");
            assert_eq!(e.body, Request::Flush);
        }
        other => panic!("{other:?}"),
    }
    // And the older encoder REFUSES v5 by name — never clamps it to v4.
    assert_eq!(encode_session_request(5, S, &Request::Flush), Err(Dropped::NeedsV5Envelope));
    assert_eq!(encode_session_request(6, S, &Request::Flush), Err(Dropped::NeedsV5Envelope));
}

#[test]
fn every_reply_to_a_v5_client_can_carry_its_ack_and_the_new_verdicts() {
    for (frozen, write_id, state) in [
        (ACKED_DUPLICATE, 4, WriteState::Duplicate),
        (ACKED_OUT_OF_ORDER, 9, WriteState::OutOfOrder { expected: 7 }),
    ] {
        let r = Reply::Acked { ack: ack(), body: Box::new(Reply::SessionWriteState { session: S, write_id, state }) };
        assert_eq!(encode_reply(&r).expect("encodes"), hex(frozen), "{state:?}: the Acked bytes moved");
        assert_eq!(decode_reply(&hex(frozen)).expect("decodes"), r);
    }
}

// ------------------------------------------------------------ refused by name

#[test]
fn a_v5_frame_with_frame_zero_is_refused_bad_frame() {
    let bytes = encode_v5_request(S, 1, 0, &Request::Flush).expect("a bad frame encodes, so it can be sent");
    assert_eq!(decode_request(&bytes), Incoming::Dropped(Dropped::BadFrame));
    // CONTROL: frame 1 is fine.
    assert!(matches!(decode_request(&encode_v5_request(S, 1, 1, &Request::Flush).expect("encodes")), Incoming::Ok(_)));
}

#[test]
fn a_v5_frame_with_an_invalid_session_is_refused_bad_session() {
    for bad in [0, LEGACY_SESSION, 1u64 << SESSION_BITS] {
        let bytes = encode_v5_request(bad, 1, 1, &Request::Flush).expect("encodes");
        assert_eq!(decode_request(&bytes), Incoming::Dropped(Dropped::BadSession), "session {bad:#x}");
    }
}

/// One write shape per version: a v5 write says its floor; a floor means
/// nothing below v5.
#[test]
fn the_wrong_write_shape_for_the_version_is_refused_by_name() {
    let plain = Request::Write { write_id: 9, ops: vec![Op::Delete(b"g".to_vec())] };
    let v5_plain = encode_v5_request(S, 1, 1, &plain).expect("encodes");
    assert_eq!(decode_request(&v5_plain), Incoming::Dropped(Dropped::WrongWriteShape), "a v5 write without a floor");
    let v4_floor = encode_session_request(4, S, &write_from()).expect("encodes");
    assert_eq!(decode_request(&v4_floor), Incoming::Dropped(Dropped::WrongWriteShape), "a floor below v5");
    // CONTROLS: each shape at its own version decodes.
    assert!(matches!(decode_request(&encode_session_request(4, S, &plain).expect("encodes")), Incoming::Ok(_)));
    assert!(matches!(decode_request(&encode_v5_request(S, 1, 1, &write_from()).expect("encodes")), Incoming::Ok(_)));
}

#[test]
fn an_ack_inside_an_ack_is_refused_nested_ack() {
    let once = Reply::Acked { ack: ack(), body: Box::new(Reply::Unsupported { got: 9, known: KNOWN.to_vec() }) };
    let twice = Reply::Acked { ack: ack(), body: Box::new(once.clone()) };
    assert_eq!(decode_reply(&encode_reply(&twice).expect("encodes")), Err(Dropped::NestedAck));
    // CONTROL: one wrapper decodes.
    assert_eq!(decode_reply(&encode_reply(&once).expect("encodes")), Ok(once));
}

/// One reply is about ONE session: an Ack for A around B's verdict is refused.
#[test]
fn an_ack_for_one_session_around_anothers_verdict_is_refused() {
    let other = S + 1;
    let crossed = Reply::Acked { ack: ack(), body: Box::new(Reply::SessionWriteState { session: other, write_id: 1, state: WriteState::Published }) };
    assert_eq!(decode_reply(&encode_reply(&crossed).expect("encodes")), Err(Dropped::AckSessionMismatch));
    // CONTROL: the same reply naming the Ack's own session decodes.
    let own = Reply::Acked { ack: ack(), body: Box::new(Reply::SessionWriteState { session: S, write_id: 1, state: WriteState::Published }) };
    assert_eq!(decode_reply(&encode_reply(&own).expect("encodes")), Ok(own));
}

/// Parity UNKNOWN is not parity DONE: `None` round-trips as `None`, never as
/// "0 owed".
#[test]
fn parity_unknown_is_not_zero_owed() {
    let unknown = Reply::Acked { ack: Ack { parity: None, answering: None, ..ack() }, body: Box::new(Reply::Unsupported { got: 1, known: vec![] }) };
    match decode_reply(&encode_reply(&unknown).expect("encodes")).expect("decodes") {
        Reply::Acked { ack, .. } => {
            assert_eq!(ack.parity, None, "an engine that does not know was read as knowing");
            assert_eq!(ack.answering, None);
        }
        other => panic!("{other:?}"),
    }
}

// ------------------------------------------------------------ older readers

/// A v4 build's `WriteState`, FROZEN: what an old page's decoder is.
#[derive(serde::Deserialize, Debug)]
#[allow(dead_code)]
enum WriteStateV4 {
    Accepted,
    Stalled,
    Published,
    ParityComplete,
    Busy,
    Failed,
    Lost,
    TooLarge { bound: u32, limit: u32, got: u32 },
}

/// The new verdicts are REFUSED by a v4 decoder, never misread as another state.
#[test]
fn a_v4_decoder_refuses_the_order_rule_verdicts_and_reads_every_v4_state() {
    use bincode::Options;
    let o = || bincode::DefaultOptions::new().with_fixint_encoding();
    for s in WriteState::ORDER_RULE {
        let bytes = o().serialize(s).expect("encodes");
        if let Ok(read) = o().deserialize::<WriteStateV4>(&bytes) {
            panic!("a v4 decoder read {s:?} as {read:?}");
        }
    }
    // CONTROL: the same frozen decoder reads the last v4 state.
    let bytes = o().serialize(&WriteState::Failed).expect("encodes");
    assert_eq!(format!("{:?}", o().deserialize::<WriteStateV4>(&bytes).expect("decodes")), "Failed");
}

/// `Acked` is APPENDED: its tag is the one after the last v4 reply's, so every
/// older decoder meets an unknown tag rather than a different message.
#[test]
fn acked_is_the_last_reply_tag() {
    let tag = |r: &Reply| u32::from_le_bytes(encode_reply(r).expect("encodes")[..4].try_into().expect("4 bytes"));
    let last_v4 = tag(&Reply::SessionWriteState { session: S, write_id: 1, state: WriteState::Accepted });
    let acked = tag(&Reply::Acked { ack: ack(), body: Box::new(Reply::Unsupported { got: 1, known: vec![] }) });
    assert_eq!(acked, last_v4 + 1, "Acked is not appended after SessionWriteState");
}

// ------------------------------------------------------------ the downgrade

/// The order rule's verdicts are v5-only. If one ever reached an older client
/// it would be told the TRUE v4 equivalent — `OutOfOrder` as `Busy` (nothing
/// applied, may go again), `Duplicate` as `Stalled` (claims nothing false) —
/// never `Failed`, a false rollback. In a debug build reaching it at all is a
/// bug, and says so.
#[test]
fn the_order_rule_maps_down_to_its_true_v4_meaning_in_every_profile() {
    // `mapped_down` is the VALUE, pure: tested here in debug and release
    // alike, because a delegate is a release build and that is what ships.
    for (state, want) in [(WriteState::OutOfOrder { expected: 7 }, WriteState::Busy), (WriteState::Duplicate, WriteState::Stalled)] {
        assert_eq!(state.mapped_down(5), state, "a v5 client is told the verdict itself");
        for old in 1..5 {
            let told = state.mapped_down(old);
            assert_eq!(told, want, "{state:?} told to a v{old} client");
            assert_ne!(told, WriteState::Failed, "a false rollback");
            assert_eq!((told.terminal(), told.should_resubmit()), (state.terminal(), state.should_resubmit()));
        }
    }
    assert_eq!(WriteState::OutOfOrder { expected: 1 }.since(), FLOOR_SINCE);
    assert_eq!(WriteState::Duplicate.since(), FLOOR_SINCE);
}

/// And REACHING a pre-v5 client with one is a bug a debug build names, at
/// the call (`for_client`).
#[test]
fn telling_a_pre_v5_client_an_order_rule_verdict_asserts_in_debug() {
    for state in WriteState::ORDER_RULE {
        assert_eq!(state.for_client(5), *state);
        let told = std::panic::catch_unwind(|| state.for_client(4));
        if cfg!(debug_assertions) {
            assert!(told.is_err(), "{state:?} reached a v4 client and nothing said so");
        } else {
            assert_eq!(told.expect("release: no panic in a delegate"), state.mapped_down(4));
        }
    }
}

/// v5 is KNOWN (decodable) and not yet CURRENT.
#[test]
fn v5_is_known_and_not_yet_current() {
    assert!(KNOWN.contains(&5));
    assert_eq!(CURRENT, 4, "v5 becomes current when a client AND an engine speak it");
    assert_eq!(FLOOR_SINCE, 5);
}
