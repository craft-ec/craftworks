//! A write refused at MAKE time uses up no write id (craftworks-sdk#251).
//!
//! A make-time refusal (no session, too large to send) never reaches the
//! wire. When the id was minted before those checks, the ids a node saw had a
//! gap -- "ids on the wire [1, 3]; refused locally [2]" (the architect on
//! sdk#186) -- and anything reading a gap as a lost write read a refusal as
//! one.

use craftworks_sdk::{Edit, Refused, Writes};

/// A write half with a session, on a frozen clock.
fn writes() -> Writes {
    Writes::new(Box::new(|| 0))
}

/// A write of one new key.
fn put(w: &mut Writes, key: &[u8], value: Vec<u8>) -> Result<u64, Refused> {
    w.make(&[(key.to_vec(), protocol::Expect::Absent)], &[(key.to_vec(), Edit::Put(value))])
}

/// The write ids on the wire, in the order they were framed.
fn ids_on_the_wire(w: &mut Writes) -> Vec<u64> {
    w.take_outbound()
        .iter()
        .filter_map(|frame| match protocol::decode_request(frame) {
            protocol::Incoming::Ok(env) => match env.body {
                protocol::Request::Write { write_id, .. } | protocol::Request::Commit { write_id, .. } => Some(write_id),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn a_write_too_large_to_send_uses_up_no_id_and_the_ids_on_the_wire_are_consecutive() {
    let mut w = writes();
    assert_eq!(put(&mut w, b"a", b"1".to_vec()), Ok(1));
    let r = put(&mut w, b"huge", vec![7u8; protocol::MAX_MESSAGE + 1]);
    assert!(matches!(r, Err(Refused::TooLargeToSend { .. })), "THE SETUP: the write was not refused at make time: {r:?}");
    assert_eq!(w.next_write_id(), 2, "the refused write used up an id");
    assert_eq!(put(&mut w, b"b", b"2".to_vec()), Ok(2), "the write after the refusal did not take the next id");
    assert_eq!(ids_on_the_wire(&mut w), vec![1, 2], "the ids on the wire have a gap");
}

#[test]
fn a_write_refused_for_no_session_uses_up_no_id() {
    let mut w = writes();
    let session = std::mem::replace(&mut w.client, craftworks_sdk::engine_client::Client::from_random(None));
    let r = put(&mut w, b"a", b"1".to_vec());
    assert!(matches!(r, Err(Refused::NoSession)), "THE SETUP: the write was not refused for its session: {r:?}");
    assert_eq!(w.next_write_id(), 1, "the refused write used up an id");
    // THE CONTROL: with a session again, the next write is the first on the wire.
    w.client = session;
    assert_eq!(put(&mut w, b"a", b"1".to_vec()), Ok(1));
    assert_eq!(ids_on_the_wire(&mut w), vec![1]);
}
