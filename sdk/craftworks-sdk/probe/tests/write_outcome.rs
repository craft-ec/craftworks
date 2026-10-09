//! The live-page-writes wait, on demand: a write ends at `Published` or at
//! any TERMINAL state, and a terminal one without `Published` says why.

use probe::verdict::write_outcome;
use protocol::{Reply, WriteState};

fn state(write_id: u64, state: WriteState) -> Reply {
    Reply::SessionWriteState { session: 4, write_id, state }
}

#[test]
fn a_reads_less_write_refused_unread_ends_at_once_and_names_the_key() {
    // What the stale probe got and waited 120 s on (sdk#283).
    let got = [Reply::Unread { session: 4, write_id: 1, key: b"r/00000".to_vec() }, state(1, WriteState::Unread)];
    let Some(Err(why)) = write_outcome(1, &got) else { panic!("an Unread write must end, as a failure: {:?}", write_outcome(1, &got)) };
    assert!(why.contains("Unread") && why.contains("r/00000"), "{why}");
}

#[test]
fn every_terminal_state_ends_the_wait() {
    let terminal = [
        WriteState::Busy,
        WriteState::Failed,
        WriteState::FailedWhy { why: protocol::FailWhy::SilentRelease },
        WriteState::Lost,
        WriteState::Conflict,
        WriteState::Unread,
        WriteState::Unknown,
        WriteState::OutOfOrder { expected: 1 },
        WriteState::QueueFull { bytes: 1, limit: 1 },
        WriteState::TooLarge { bound: protocol::WriteBound::WriteBytes, limit: 1, got: 2 },
    ];
    for s in terminal {
        assert!(matches!(write_outcome(1, &[state(1, s)]), Some(Err(_))), "{s:?} must end the wait as a failure");
    }
}

#[test]
fn published_is_the_success_and_what_comes_before_it_waits() {
    assert_eq!(write_outcome(1, &[state(1, WriteState::Accepted)]), None);
    assert_eq!(write_outcome(1, &[state(1, WriteState::Stalled)]), None);
    assert_eq!(write_outcome(1, &[state(1, WriteState::Accepted), state(1, WriteState::Published)]), Some(Ok(())));
}

#[test]
fn another_writes_answer_is_not_this_ones() {
    assert_eq!(write_outcome(2, &[state(1, WriteState::Published)]), None);
    assert_eq!(write_outcome(2, &[state(1, WriteState::Unread)]), None);
}
