//! `unframe` as a trust boundary: nothing a stranger sends may panic, allocate
//! what it chose, or complete a message it did not send.
//!
//! A gateway is not the user's node. Everything here is input somebody else
//! picked, and this crate compiles to wasm with `panic = abort`, where a panic
//! is a dead instance rather than an error an app can catch.

use freenet_stdlib::client_api::streaming::MAX_CONCURRENT_STREAMS;
use wire::{unframe, Incoming, Reassembler, Unusable};

fn r() -> Reassembler {
    Reassembler::new()
}

/// Garbage, truncation and emptiness are ANSWERS.
#[test]
fn nothing_malformed_panics_and_every_case_is_said() {
    let mut rs = r();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", vec![]),
        ("one byte", vec![0]),
        ("all ones", vec![0xFF; 64]),
        ("text", b"this is not a host response".to_vec()),
        ("a long run of zeroes", vec![0u8; 4096]),
        // A plausible bincode prefix, then nothing. The dangerous shape: a
        // decoder that accepts a prefix turns one message into another.
        ("a truncated variant tag", vec![0, 0, 0, 0]),
    ];
    for (what, bytes) in cases {
        let got = unframe(&mut rs, &bytes);
        assert!(
            matches!(got, Incoming::Unusable(_) | Incoming::Refused(_)),
            "{what}: expected an answer, got {got:?}"
        );
    }
}

/// A frame larger than this build will decode is refused BEFORE it is decoded.
#[test]
fn an_oversized_frame_is_refused_before_it_is_read() {
    let mut rs = r();
    let huge = vec![0u8; wire::MAX_FRAME + 1];
    assert_eq!(
        unframe(&mut rs, &huge),
        Incoming::Unusable(Unusable::TooLarge)
    );
}

/// An absurd `total` is refused at the FIRST chunk, before the slots for it
/// are allocated. The sender picks that number.
#[test]
fn an_absurd_total_is_refused_at_the_first_chunk() {
    let mut rs = r();
    for total in [0u32, 257, 100_000, u32::MAX] {
        assert_eq!(
            rs.chunk(1, 0, total, vec![1, 2, 3]),
            Err(Unusable::BadStream),
            "total {total} was accepted"
        );
    }
    assert_eq!(rs.in_flight(), 0, "a refused stream was still recorded");
    assert_eq!(rs.bytes(), 0, "a refused stream held bytes");
}

/// An index outside its own total is refused.
#[test]
fn an_index_past_its_own_total_is_refused() {
    let mut rs = r();
    assert_eq!(rs.chunk(1, 5, 3, vec![1]), Err(Unusable::BadStream));
    assert_eq!(rs.chunk(1, 3, 3, vec![1]), Err(Unusable::BadStream));
    assert_eq!(
        rs.chunk(1, 2, 3, vec![1]),
        Ok(None),
        "a valid index must work"
    );
}

/// A sender cannot re-shape a stream in flight by changing its total.
#[test]
fn a_second_total_for_one_stream_is_refused() {
    let mut rs = r();
    assert_eq!(rs.chunk(7, 0, 3, vec![1]), Ok(None));
    assert_eq!(
        rs.chunk(7, 1, 9, vec![2]),
        Err(Unusable::BadStream),
        "a chunk claiming a different total was taken as part of the stream"
    );
}

/// Interleaved streams complete independently and do not mix.
#[test]
fn interleaved_streams_do_not_contaminate_each_other() {
    let mut rs = r();
    assert_eq!(rs.chunk(1, 0, 2, b"aa".to_vec()), Ok(None));
    assert_eq!(rs.chunk(2, 0, 2, b"bb".to_vec()), Ok(None));
    assert_eq!(
        rs.chunk(2, 1, 2, b"BB".to_vec()),
        Ok(Some(b"bbBB".to_vec()))
    );
    assert_eq!(
        rs.chunk(1, 1, 2, b"AA".to_vec()),
        Ok(Some(b"aaAA".to_vec()))
    );
    assert_eq!(rs.in_flight(), 0);
}

/// A duplicate chunk REPLACES rather than accumulating.
///
/// Otherwise a sender grows a stream past its own declared size simply by
/// re-sending, which is an allocation it chose after the bound was checked.
#[test]
fn a_duplicate_chunk_replaces_and_does_not_grow_the_stream() {
    let mut rs = r();
    assert_eq!(rs.chunk(1, 0, 2, vec![b'x'; 100]), Ok(None));
    let after_one = rs.bytes();
    for _ in 0..50 {
        assert_eq!(rs.chunk(1, 0, 2, vec![b'x'; 100]), Ok(None));
    }
    assert_eq!(
        rs.bytes(),
        after_one,
        "fifty duplicates of one chunk grew the stream"
    );
    assert_eq!(
        rs.chunk(1, 1, 2, vec![b'y'; 100]),
        Ok(Some([vec![b'x'; 100], vec![b'y'; 100]].concat()))
    );
}

/// A message that never completes does not wedge reassembly.
///
/// stdlib's own buffer evicts stale streams — but `evict_stale` is
/// `#[cfg(not(target_family = "wasm"))]`, so in a BROWSER nothing ages one
/// out, and eight abandoned streams would fill the cap permanently. That is
/// something a node could do on purpose and a flaky connection could do by
/// accident.
#[test]
fn abandoned_streams_never_wedge_reassembly() {
    let mut rs = r();
    // Nine streams started, none finished — one more than the cap.
    for id in 0..9u32 {
        assert_eq!(rs.chunk(id, 0, 2, vec![id as u8; 10]), Ok(None));
    }
    // THE COUNT IS BOUNDED, and this is the assertion the test was missing.
    //
    // It used to check only that a fresh stream could still complete — which
    // is true whether or not anything bounds the count, because nothing
    // REFUSES a new stream; they would simply accumulate. Mutation found it:
    // disabling the slot eviction left the test green. What the guard
    // actually does is keep the number of half-finished streams from growing
    // without limit, so that is what is asserted.
    assert!(
        rs.in_flight() <= MAX_CONCURRENT_STREAMS,
        "nine abandoned streams left {} in flight against a cap of {}",
        rs.in_flight(),
        MAX_CONCURRENT_STREAMS
    );

    // And a new stream still works, which is what the bound is FOR: without
    // it the oldest would never go and a browser would accumulate them.
    assert_eq!(rs.chunk(100, 0, 2, b"aa".to_vec()), Ok(None));
    assert_eq!(
        rs.chunk(100, 1, 2, b"AA".to_vec()),
        Ok(Some(b"aaAA".to_vec())),
        "a fresh stream could not complete because abandoned ones held every \
         slot — replies would simply stop arriving"
    );
}

/// The bytes held in flight are bounded too, because a count is not a byte
/// budget.
#[test]
fn the_bytes_held_in_flight_are_bounded() {
    let mut rs = r();
    rs.max_bytes = 1000;
    for id in 0..4u32 {
        let _ = rs.chunk(id, 0, 4, vec![b'x'; 400]);
    }
    assert!(
        rs.bytes() <= 1000,
        "held {} B against a cap of 1000",
        rs.bytes()
    );
}

/// A reconnect clears what was in flight.
///
/// Stream ids restart at zero on a new connection — stdlib's `next_stream_id`
/// lives on the client — so a half-finished stream from the old connection
/// would be reassembled with chunks of a DIFFERENT message under the same id.
/// Silently, and into something that then fails to decode.
#[test]
fn a_reconnect_clears_what_was_in_flight() {
    let mut rs = r();
    assert_eq!(rs.chunk(0, 0, 2, b"old".to_vec()), Ok(None));
    rs.reset();
    assert_eq!(rs.in_flight(), 0);
    assert_eq!(rs.bytes(), 0);
    // The same id, a new connection, a different message: it starts clean.
    assert_eq!(rs.chunk(0, 0, 2, b"nw".to_vec()), Ok(None));
    assert_eq!(
        rs.chunk(0, 1, 2, b"ok".to_vec()),
        Ok(Some(b"nwok".to_vec()))
    );
}

/// A refusal may drive a bounded retry and may NEVER ask for a credential.
///
/// The node chooses the reason, so it steers client control flow. "Refused:
/// bad key → ask the person to re-enter their passphrase" is a phishing
/// channel delivered through the protocol. This is a function rather than a
/// comment so a future caller finds an answer instead of inventing one.
#[test]
fn no_refusal_may_ever_lead_to_a_credential_prompt() {
    for said in [
        "bad key",
        "unauthorized",
        "your session expired, sign in again",
        "please re-enter your passphrase",
    ] {
        let r = wire::Refused { said: said.into(), code: "unhandled" };
        assert!(
            !r.may_prompt_for_credentials(),
            "a node-supplied reason ({said}) was allowed to prompt for a credential"
        );
    }
}

/// The URL carries `encodingProtocol=native`, and that is not cosmetic.
///
/// The node falls back to Flatbuffers when the parameter is absent
/// (`client_events/websocket.rs:904` at `ea1ff5f`) while everything here
/// speaks bincode. Built in Rust because it is a protocol fact — the page's
/// first socket omitted it, and the failure would have been a connection that
/// opens and then never understands anything.
#[test]
fn the_url_asks_for_the_encoding_this_crate_actually_speaks() {
    let url = wire::ws_url("127.0.0.1", 7711).expect("loopback is allowed");
    assert!(
        url.contains("encodingProtocol=native"),
        "the URL does not ask for native encoding: {url}"
    );
    assert!(url.starts_with("ws://127.0.0.1:7711/"), "{url}");
}

/// A cleartext URL is only ever built for loopback.
///
/// `ws://` to another machine would put every request, every reply and the
/// shape of a whole session on the wire in the clear. Whether we ever do that,
/// and over what, is a decision nobody has taken — and until they have,
/// BUILDING the URL is how it happens by accident.
#[test]
fn a_cleartext_url_is_refused_for_anything_but_loopback() {
    for host in ["example.com", "10.0.0.5", "203.0.113.1", "node.local"] {
        let got = wire::ws_url(host, 7711);
        assert!(
            got.is_err(),
            "built a cleartext ws:// URL for `{host}`: {got:?}"
        );
    }
    // The control: loopback still works, in each spelling a person uses.
    for host in ["127.0.0.1", "localhost", "::1"] {
        assert!(
            wire::ws_url(host, 7711).is_ok(),
            "refused loopback host `{host}`, which is the only one anything \
             can currently reach"
        );
    }
}

/// The same node error means the same thing at BOTH doors.
///
/// It arrives unchunked, or reassembled from chunks. The first version
/// decoded those on different arms, so one fact got two answers depending on
/// a transport detail the SENDER picks — and a sender that wanted the softer
/// one only had to chunk.
#[test]
fn a_node_error_means_the_same_thing_chunked_or_not() {
    // A `HostResponse` that is the node's own error: `Err` inside the result.
    let err: Result<wire::_test::Host, wire::_test::Err> = Err(wire::_test::client_error("no"));
    let bytes = bincode::serialize(&err).expect("encodes");

    // Door one: straight in.
    let mut a = r();
    let direct = unframe(&mut a, &bytes);
    assert!(
        matches!(direct, Incoming::Refused(_)),
        "unchunked, a node error came back {direct:?}"
    );

    // Door two: the same bytes, arriving as one complete chunk.
    let mut b = r();
    let whole = b
        .chunk(1, 0, 1, bytes.clone())
        .expect("a single-chunk stream")
        .expect("completes at once");
    let reassembled = wire::_test::classify_for_test(&whole);
    assert!(
        matches!(reassembled, Incoming::Refused(_)),
        "reassembled, the same node error came back {reassembled:?} — one \
         fact must not get two answers because of how it was carried"
    );
}

/// ONE stream may not hold what it likes.
///
/// **This is the case the first byte-cap test missed, and it missed it for a
/// reason worth writing down.** That test used FOUR streams, so it exercised
/// the eviction path — and eviction was guarded by `streams.len() > 1`, which
/// is precisely the condition a single stream never meets. The bound looked
/// tested and a lone sender was unbounded: chunks were bounded by `MAX_FRAME`
/// (4 MiB) rather than by stdlib's own `CHUNK_SIZE` (256 KiB), so one stream
/// could hold 256 × 4 MiB ≈ 1 GiB in a browser tab — for a message `decode`
/// would then refuse as `TooLarge` anyway.
#[test]
fn one_stream_alone_is_bounded_too() {
    let mut rs = r();
    rs.max_bytes = 16 * 1024 * 1024;

    // A chunk bigger than the sender's own chunk size is not a chunk.
    assert_eq!(
        rs.chunk(1, 0, 8, vec![0u8; wire::MAX_CHUNK + 1]),
        Err(Unusable::BadStream),
        "a chunk larger than CHUNK_SIZE was accepted, which is how one stream \
         holds gigabytes"
    );

    // And a lone stream feeding legal-sized chunks is still bounded.
    let mut sent = 0usize;
    for i in 0..200u32 {
        match rs.chunk(2, i, 200, vec![0u8; wire::MAX_CHUNK]) {
            Ok(_) => sent += 1,
            Err(_) => break,
        }
        assert!(
            rs.bytes() <= rs.max_bytes,
            "ONE stream held {} B against a cap of {} B after {} chunk(s)",
            rs.bytes(),
            rs.max_bytes,
            sent
        );
    }
    assert!(
        rs.bytes() <= rs.max_bytes,
        "one stream ended holding {} B against a cap of {} B",
        rs.bytes(),
        rs.max_bytes
    );
}

/// A message whose DECLARED size cannot be legitimate is refused at chunk 0,
/// before a single byte of it is held.
#[test]
fn an_impossible_total_is_refused_before_any_of_it_arrives() {
    let mut rs = r();
    // 256 chunks is stdlib's cap and ~64 MiB; nothing this client asks for
    // comes back that large, so it is refused on the declaration alone.
    let too_many = (wire::MAX_REASSEMBLED / wire::MAX_CHUNK + 2) as u32;
    assert_eq!(
        rs.chunk(1, 0, too_many, vec![0u8; 16]),
        Err(Unusable::BadStream),
        "a total declaring more than this client will ever reassemble was \
         accepted, so the bytes arrive before anything objects"
    );
    assert_eq!(rs.bytes(), 0);
    assert_eq!(rs.in_flight(), 0);
}

/// EVICTION still has something to do, and this is the test that reaches it.
///
/// **Found by re-running the mutants after adding the per-stream bound.**
/// "Never drop the oldest" SURVIVED: the new declaration and per-stream caps
/// catch a single greedy stream long before the buffer's own byte cap is
/// reached, so nothing in the suite exercised eviction any more. A guard that
/// no test can reach is a guard that can be deleted with the suite green.
///
/// The case eviction is actually for is MANY streams, each individually
/// legitimate, together over the buffer's cap — a node answering several
/// requests at once while one of them stalls.
#[test]
fn many_legitimate_streams_together_still_force_eviction() {
    let mut rs = r();
    rs.max_bytes = 1024 * 1024; // 1 MiB

    // Eight streams — stdlib's concurrent cap — each sending ONE legal chunk
    // of 256 KiB. Individually fine; together 2 MiB, twice the buffer's cap.
    for id in 0..8u32 {
        let _ = rs.chunk(id, 0, 4, vec![b'x'; wire::MAX_CHUNK]);
        assert!(
            rs.bytes() <= rs.max_bytes,
            "after stream {id} the buffer held {} B against a cap of {} B — \
             eviction did not fire, and every one of these chunks is legal on \
             its own, so nothing else would stop them",
            rs.bytes(),
            rs.max_bytes
        );
    }
    assert!(
        rs.in_flight() < 8,
        "all eight streams survived a 1 MiB cap holding 2 MiB, so nothing was \
         evicted"
    );
}
