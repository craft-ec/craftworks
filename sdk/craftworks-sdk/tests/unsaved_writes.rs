//! What a closing tab would lose: every write not yet PUBLISHED, queued ones
//! included (craftworks-sdk#163).
//!
//! The page arms its unsaved-changes guard, and says "saving N…", from
//! `PageStore::unsaved_writes`: this session's writes in the ENGINE's queue,
//! pulled from the page's Server (R-b). A write waiting its turn behind the
//! commit in flight is in the queue and counted -- never "saved" because
//! nothing has answered it yet.

use craftworks_sdk::Store as _;

/// **100 writes behind a commit that cannot land: all 100 are unsaved; once
/// the node answers and every one is published, none is.**
#[test]
fn a_hundred_writes_are_unsaved_until_published_queued_ones_included() {
    let node = testkit::PageNode::new();
    let (mut s, mut conn, _clock) = testkit::page_store(&node);
    s.sync();
    conn.hold_answers();
    for i in 0..100u32 {
        s.put(format!("d/notes/{i:04}").as_bytes(), b"row").expect("the store took the write");
    }
    assert_eq!(s.unsaved_writes(), 100, "writes queued behind a commit in flight were counted as saved");
    let (queued, _) = s.queue_load();
    assert_eq!(queued, 100, "the engine does not hold all 100: {queued}");

    conn.stop_holding();
    for step in 0.. {
        assert!(step < 100_000, "the node was still answering after 100,000 releases");
        if conn.held() == 0 {
            break;
        }
        let _ = conn.release_one();
    }
    s.sync();
    assert_eq!(s.unsaved_writes(), 0, "writes still unsaved after the node answered them all");
    let (_, root) = node.head().expect("published");
    assert_eq!(node.tree(&root).expect("whole").len(), 100, "not every write is in the published tree");
}

/// THE CONTROL: a write the engine has taken and not published is still
/// unsaved (sdk#163's ruling): a tab closed now loses it.
#[test]
fn a_taken_write_is_still_unsaved() {
    let node = testkit::PageNode::new();
    let (mut s, mut conn, _clock) = testkit::page_store(&node);
    s.sync();
    conn.hold_answers();
    s.put(b"d/notes/0001", b"row").expect("the store took the write");
    assert_eq!(s.unsaved_writes(), 1, "a taken, unpublished write was counted as saved");
}
