//! The recorded v1 session: the file that must keep replaying when v2 exists.

use protocol::session::{replay, Line, Session};
use protocol::*;

/// The frozen session, compiled INTO this test binary.
///
/// Not read from a path at run time. `CARGO_MANIFEST_DIR` is baked in when the
/// binary is built, so a test binary served from a shared `CARGO_TARGET_DIR`
/// carries whichever worktree built it — and once that worktree is gone the
/// gate fails on a missing file having checked nothing. Embedding the bytes
/// binds the fixture to the build instead of to the filesystem.
const FIXTURE: &[u8] = include_bytes!("fixtures/v1-session.bin");

/// Where the recorder WRITES. Only the `#[ignore]`d recorder uses this, and it
/// is run deliberately, from the worktree it is meant to update.
fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1-session.bin")
}

/// Build the conversation that gets frozen.
///
/// Every v1 request and a representative reply for each, so the fixture
/// covers the protocol rather than the one message a happy path uses. If a
/// future version drops or changes any of these, THIS file is what notices.
fn v1_session() -> Session {
    let mut lines = Vec::new();
    let mut send = |r: Request| lines.push(Line::Sent(encode_request(1, &r).expect("encodes")));

    send(Request::Identity);
    send(Request::Write {
        write_id: 1,
        ops: vec![
            Op::Put(b"k/one".to_vec(), vec![0x5Au8; 200]),
            Op::Delete(b"k/gone".to_vec()),
        ],
    });
    send(Request::AskWrite { write_id: 1 });
    send(Request::Get {
        req_id: 10,
        key: b"k/one".to_vec(),
    });
    send(Request::Range {
        req_id: 11,
        lo: Bound::Included(b"k/".to_vec()),
        hi: Bound::Excluded(b"k0".to_vec()),
        reverse: false,
        after: None,
        max_entries: 25,
    });
    send(Request::Range {
        req_id: 12,
        lo: Bound::Unbounded,
        hi: Bound::Unbounded,
        reverse: true,
        after: Some(b"k/one".to_vec()),
        max_entries: 25,
    });
    send(Request::Preload {
        roots: vec![[7u8; 32], [9u8; 32]],
    });
    send(Request::Subscribe {
        write_ids: vec![1, 2, 3],
    });
    send(Request::Tick { now: 1_726_000_000 });
    send(Request::Flush);

    let mut recv = |r: Reply| lines.push(Line::Received(encode_reply(&r).expect("encodes")));
    recv(Reply::Identity {
        engine: "craftworks-engine/0.1".into(),
        key_source: "Provisioned(Test)".into(),
        head_seq: 2,
        head_root: [0xAB; 32],
        head_writable: true,
        head_id: [0xCD; 32],
    });
    recv(Reply::WriteState {
        write_id: 1,
        state: WriteState::Accepted,
    });
    recv(Reply::WriteState {
        write_id: 1,
        state: WriteState::Published,
    });
    recv(Reply::WriteState {
        write_id: 1,
        state: WriteState::ParityComplete,
    });
    recv(Reply::WriteState {
        write_id: 2,
        state: WriteState::Busy,
    });
    recv(Reply::Value {
        req_id: 10,
        value: Some(vec![0x5Au8; 200]),
    });
    recv(Reply::Value {
        req_id: 10,
        value: None,
    });
    recv(Reply::Page {
        req_id: 11,
        entries: vec![
            (b"k/one".to_vec(), vec![1u8; 8]),
            (b"k/two".to_vec(), vec![2u8; 8]),
        ],
        cursor: Some(b"k/two".to_vec()),
        max_entries: 25,
        at: protocol::At {
            seq: 3,
            root: [0x3C; 32],
        },
    });
    recv(Reply::Unavailable {
        req_id: 12,
        blocked_on: [0xCD; 32],
    });
    recv(Reply::Unsupported {
        got: 99,
        known: KNOWN.to_vec(),
    });
    recv(Reply::Dropped {
        reason: Dropped::TrailingBytes,
    });

    Session { lines }
}

/// Write the fixture. Run by hand; never a gate.
///
/// ```text
/// cargo test -p protocol --test session -- --ignored record
/// ```
#[test]
#[ignore = "records the v1 session fixture"]
fn record_the_v1_session() {
    let s = v1_session();
    let path = fixture_path();
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&path, s.encode()).expect("write");
    println!("  recorded {} lines to {}", s.lines.len(), path.display());
}

/// The recorded v1 session still replays.
///
/// This is the file that must keep passing when v2 exists. It holds BYTES,
/// not messages: a fixture built by the same encoder it checks would be
/// rewritten by any change to the encoding, and the test would go green
/// having proved nothing.
#[test]
fn the_recorded_v1_session_replays() {
    // The bytes this gate checks. Empty would be a gate with nothing to
    // check, which is not a pass.
    assert!(!FIXTURE.is_empty(), "the recorded v1 session is empty");
    let bytes = FIXTURE.to_vec();
    let s = Session::decode(&bytes).expect("the fixture must decode");
    let r = replay(&s);

    assert!(
        r.unreadable.is_empty(),
        "this build cannot read line(s) {:?} of a session it recorded at v1. \
         Every published version keeps working (§19), and this file is what \
         says so.",
        r.unreadable
    );
    assert!(
        r.unsupported.is_empty(),
        "this build no longer serves version(s) {:?} that the fixture uses",
        r.unsupported
    );
    // Floors, so a fixture that lost its tail cannot pass as a short one.
    assert!(
        r.requests >= 10,
        "the fixture replayed only {} request(s); it was recorded with more, \
         so it has been truncated",
        r.requests
    );
    assert!(
        r.replies >= 11,
        "the fixture replayed only {} repl(ies)",
        r.replies
    );
    println!(
        "  v1 session: {} requests and {} replies replayed, {} bytes on the wire",
        r.requests,
        r.replies,
        bytes.len()
    );
}

/// The control the acceptance asks for: one byte of the VERSION field
/// changed, and the line becomes `Unsupported`.
///
/// Not a corrupted message — a message from a version this build does not
/// serve. The difference is the whole reason the version is on the front.
#[test]
fn one_byte_of_the_version_makes_a_recorded_line_unsupported() {
    let bytes = FIXTURE.to_vec();
    let s = Session::decode(&bytes).expect("decodes");

    let mut damaged = s.clone();
    let mut changed = 0usize;
    for line in damaged.lines.iter_mut() {
        if let Line::Sent(b) = line {
            // The version is the first field, little-endian.
            b[0] = 99;
            changed += 1;
        }
    }
    assert!(changed >= 10, "only {changed} request line(s) were changed");

    let r = replay(&damaged);
    assert_eq!(
        r.unsupported.len(),
        changed,
        "{} of {changed} version-changed lines were reported Unsupported; the \
         rest were read as something else, which means a client at an \
         unknown version would be answered as though it were known",
        r.unsupported.len()
    );
    assert!(
        r.unsupported.iter().all(|v| *v == 99),
        "the reported version is not the one on the wire"
    );
    assert_eq!(
        r.requests, 0,
        "{} version-changed line(s) still decoded as v1 requests",
        r.requests
    );
    println!("  {changed} lines at v99: all Unsupported, none mistaken for v1");
}
