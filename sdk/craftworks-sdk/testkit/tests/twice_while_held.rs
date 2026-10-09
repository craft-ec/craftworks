//! A FIELD EDITED TWICE while a commit is in flight (R-b): the first write's
//! commit is held, so the second and the third -- both to ONE key -- wait in
//! the ENGINE's queue, applied to the warm root in the order made. A read of
//! that key shows the LAST of them, by `get` and by `scan`: the value the
//! person typed last. (Before R-b they were told `Busy` and read through an
//! interim overlay.)

use craftworks_sdk::store::{Edit, Reads, Store};

#[test]
fn a_key_written_twice_while_held_reads_back_the_second() {
    let node = testkit::PageNode::new();
    let (mut s, conn, _clock) = testkit::page_store(&node);
    let mut conn = conn;
    conn.hold_answers();
    let put = |s: &mut craftworks_sdk::PageStore<testkit::PageConn>, k: &[u8], v: &[u8]| {
        s.apply_commit(&[(k.to_vec(), protocol::Expect::Any)], &[(k.to_vec(), Edit::Put(v.to_vec()))]).expect("taken by the store");
    };
    put(&mut s, b"other", b"first commit");
    put(&mut s, b"k", b"one");
    put(&mut s, b"k", b"two");
    // All three wait in the engine's queue, behind the held commit.
    assert_eq!(s.queue_load().0, 3, "the setup did not hold both edits of k");
    assert_eq!(s.get(b"k").expect("answers"), Some(b"two".to_vec()), "get shows an older edit, not the last");
    let rows = s.scan(b"k", b"l", false, usize::MAX).expect("answers");
    assert_eq!(rows, vec![(b"k".to_vec(), b"two".to_vec())], "scan shows an older edit, not the last");
    conn.stop_holding();
}
