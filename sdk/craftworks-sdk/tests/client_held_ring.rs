//! The delegate's per-call report, recorded where it can be read.
//!
//! `Reply::Call` exists because a delegate prints nothing and the node says
//! nothing about it, so five different breaks in the write path all presented
//! as one symptom — a write that stops at `Accepted`. Until now the engine
//! emitted it and nothing read it, so a page had no answer to "why did my
//! write stop".
//!
//! This recording ships: it is the production support tool, the same bounded
//! buffer in a user's app as in a test, and "report a problem" exports it. So
//! the tests here are about what it must NOT contain as much as what it must.

use craftworks_sdk::engine_client::Client;
use instrument::{vocab::Key, Record as _};

/// A report with a `note` that carries something it must never leak.
///
/// `Reply::Call.note` is FREE TEXT, and free text is the one field on this
/// message that can walk user content into a support bundle. The recorder does
/// not read it; these tests prove that rather than assume it.
fn a_real_call_report(dropped: u32) -> Vec<u8> {
    protocol::encode_reply(&protocol::Reply::Call {
        saw: protocol::Saw::PutResponse,
        effects: 3,
        ops: 2,
        awaiting: 1,
        read_back: 1,
        stranded: 0,
        dropped,
        head_put: 0,
        head_update: 0,
        note: "domain=medical row=patient-notes alice@example.com".into(),
    })
    .expect("encodes")
}

/// A REAL per-call report leaves no key, no value and no domain name.
///
/// Byte runs, not whole strings: a count is not content, but `medical` is —
/// however small and however well typed, and no grep for a key format finds a
/// category.
#[test]
fn a_recorded_call_report_contains_no_user_content() {
    // Every one of these is IN the report's `note` field, which the recorder
    // must not read. A test whose secrets were never near the data would pass
    // whatever the code did.
    let secrets = ["medical", "alice@example.com", "patient-notes", "domain="];

    let mut c = Client::new();
    c.record_into(256);
    c.on_inbound(&a_real_call_report(0));

    let dump = c.dump("a delegate call");
    for s in secrets {
        assert!(!dump.contains(s), "the dump contains {s} verbatim:\n{dump}");
        for w in 6..=s.len() {
            let mut i = 0;
            while i + w <= s.len() {
                assert!(
                    !dump.contains(&s[i..i + w]),
                    "the dump contains a {w}-byte run of user content: {}",
                    &s[i..i + w]
                );
                i += 1;
            }
        }
    }

    // And the report IS there — a recording that leaks nothing by being empty
    // would pass the test above and be worthless.
    let r = c.recording().expect("a recording was attached");
    assert_eq!(r.total(Key::Effects), 3, "{dump}");
    assert_eq!(r.total(Key::Ops), 2, "{dump}");
    assert_eq!(r.total(Key::Stranded), 0, "{dump}");
}

/// The ring DROPS when full. It never grows and never panics.
///
/// An unbounded buffer is a leak in a long-lived page; a probe that panics
/// turns a working app into a broken one, which is the worst kind of behaviour
/// change. Dropping and SAYING SO is the only honest third option.
#[test]
fn the_ring_drops_rather_than_growing() {
    let mut c = Client::new();
    c.record_into(8);
    for _ in 0..200 {
        c.on_inbound(&a_real_call_report(0));
    }
    let r = c.recording().expect("a recording");
    assert!(
        r.dropped() > 0,
        "200 reports into a ring of 8 must have dropped events"
    );
    assert!(
        r.events().len() <= 8,
        "the ring grew past its capacity: {} events",
        r.events().len()
    );
    assert!(
        c.dump("full").contains("dropped"),
        "and the dump SAYS the beginning is gone:\n{}",
        c.dump("full")
    );
}

/// Calls that arrive with no ring attached are MARKED as a gap.
///
/// A bundle that omitted them would look continuous across a hole, which is
/// worse than one that admits it: the reader has no way to know the story is
/// missing its middle.
#[test]
fn calls_with_no_recorder_are_a_counted_gap_not_a_silent_one() {
    let mut c = Client::new();
    // No ring yet — three calls happen anyway.
    for _ in 0..3 {
        c.on_inbound(&a_real_call_report(0));
    }
    assert_eq!(c.unrecorded_calls(), 3);
    assert!(
        c.dump("before attach").contains("unrecorded"),
        "with no ring, the dump says how many were missed:\n{}",
        c.dump("before attach")
    );

    // Attach one. The earlier three are still absent, and still admitted.
    c.record_into(64);
    c.on_inbound(&a_real_call_report(0));
    let dump = c.dump("after attach");
    assert!(
        dump.contains("NOT in this recording"),
        "the gap is stated beside the recording, not silently omitted:\n{dump}"
    );
    assert_eq!(c.unrecorded_calls(), 3, "the gap does not shrink");
}

/// A dropped message is recorded by REASON, as a vocabulary value.
#[test]
fn a_dropped_message_is_recorded_by_reason() {
    let mut c = Client::new();
    c.record_into(64);
    // Not a message this build can read at all.
    c.on_inbound(&[0xff, 0xff, 0xff, 0xff]);
    let r = c.recording().expect("a recording");
    assert!(
        r.total(Key::DroppedMsgs) < 8,
        "a reason is a small stable code, never a length or a hash"
    );
    // The ring is its one record now (the client's own drop list had no reader, sdk#482): ONE DroppedMsgs entry.
    let drops = r.events().iter().filter(|e| matches!(e, instrument::Event::Counter { entry, .. } if entry.key == Key::DroppedMsgs)).count();
    assert_eq!(drops, 1, "the drop was not recorded once");
}
