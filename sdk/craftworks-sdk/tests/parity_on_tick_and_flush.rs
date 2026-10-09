//! THE PARITY ROWS, ON THE PAGE PATH (re-stated from the Shell's
//! `what_a_flush_causes.rs`, deleted with the Shell).
//!
//! Parity is per GROUP, and a group is a trio listed by a tree NODE, so it
//! needs a node with enough children (ARCHITECTURE §7: 7–12, mean ~9, three
//! parity blocks). Since race put (COMMIT-LIFE §P) a commit puts its groups'
//! parity in the SAME round as its data; this grows a tree with groups and
//! measures, through a real tab, that a commit carries its parity and that a
//! `Tick` or a `Flush` afterwards puts nothing.

use testkit::page_node::Served;
use testkit::{PageConn, PageNode};

/// Records big enough that a leaf fills and the tree grows a level — which is
/// what puts trios on a node. GENERATED: the point is to reach a structural
/// condition, not to pin a particular tree.
fn write(n: u64) -> protocol::Request {
    protocol::Request::forced_write(n, vec![protocol::Op::Put(format!("k/{n:06}").into_bytes(), vec![(n % 251) as u8; 512])])
}

fn puts(c: &PageConn) -> usize {
    c.served(Served::Put)
}

/// A tab over a tree of `n` records: deep enough (a branch over leaves) that
/// a commit changes a GROUP, which is what carries parity.
fn grown(n: u64) -> PageConn {
    let node = PageNode::new();
    let mut c = node.connect();
    c.client(&protocol::Request::Identity);
    for i in 1..=n {
        c.client(&write(i));
    }
    c
}

/// **PARITY RIDES THE COMMIT** (COMMIT-LIFE §P, race put): a commit that
/// changes a group puts that group's 3 parity blocks IN THE SAME ROUND as its
/// data, so there is nothing left for a Tick or a Flush to put. Before §P the
/// parity was OWED after the head and paced by Tick/Flush; these rows measured
/// that machinery, which is gone.
#[test]
fn the_parity_rows() {
    let mut c = grown(400);
    // One more commit, in a tree with groups: at least the changed path
    // (root and leaf, 2) plus the leaf group's 3 parity.
    let before = puts(&c);
    c.client(&write(401));
    let commit = puts(&c) - before;
    // With NO time, with a TICK well past any age, with a FLUSH: nothing more.
    let before = puts(&c);
    c.client(&protocol::Request::AskWrite { write_id: 401 });
    let no_time = puts(&c) - before;
    let t = c.now_ms() + 10_000_000;
    c.tick_at(t);
    let with_tick = puts(&c) - before - no_time;
    c.client(&protocol::Request::Flush);
    let with_flush = puts(&c) - before - no_time - with_tick;
    println!("\n  400 records; one commit put {commit} block(s)");
    println!("  behaviour            | no time | with tick | with flush");
    println!("  ---------------------|---------|-----------|-----------");
    println!("  puts caused          | {no_time:<7} | {with_tick:<9} | {with_flush}");
    assert!(commit >= 5, "a commit in a tree with groups put {commit} block(s): its group's parity did not ride the commit (§P)");
    assert_eq!((no_time, with_tick, with_flush), (0, 0, 0), "a put after the commit: parity is still being paid later instead of in the commit's round");
}

/// **AND IT STOPS.** A published commit whose blocks are all acked is not put
/// again by later ticks.
#[test]
fn a_settled_group_is_not_put_again() {
    let mut c = grown(400);
    c.client(&write(401));
    let t0 = c.now_ms() + 10_000_000;
    let before = puts(&c);
    c.tick_at(t0);
    c.tick_at(t0 + 10_000_000);
    c.tick_at(t0 + 20_000_000);
    assert_eq!(puts(&c) - before, 0, "an acked commit's blocks were put again by later ticks");
}

/// The control: a tree that is ONE leaf has no groups, so its commit puts its
/// ONE block and the root's own PARITY parity (sdk#335: the root is a group of
/// one) -- no group parity. This pins the extra puts above to the GROUP.
#[test]
fn nothing_owed_puts_nothing() {
    let node = PageNode::new();
    let mut c = node.connect();
    c.client(&protocol::Request::Identity);
    let before = puts(&c);
    c.client(&write(1));
    let commit = puts(&c) - before;
    let t = c.now_ms() + 10_000_000;
    c.tick_at(t);
    c.client(&protocol::Request::Flush);
    let one = 1 + engine::PARITY;
    assert_eq!(commit, one, "a one-leaf tree's commit put {commit} block(s): parity with no group to code");
    assert_eq!(puts(&c) - before, one, "a Tick and a Flush caused puts with nothing owed");
}
