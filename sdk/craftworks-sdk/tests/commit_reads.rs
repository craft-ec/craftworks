//! M2 (sdk#148's SDK half): every `Db` write says what it READ, the page's
//! Server hands the reads to its engine, and the engine refuses a write whose
//! read no longer holds. Two tabs on one node, each its own page and engine;
//! each reads by WALKING its own engine's tree (READ-STATE, design B), so a
//! tab that has not adopted the other's head reads — and writes from — its
//! older view.

use craftworks_sdk::store::{Reads, RowState};
use craftworks_sdk::*;
use serde_json::json;
use testkit::page_node::PageNode;

mod support;

struct At(u32);
impl Env for At {
    fn now_ms(&mut self) -> u64 {
        1_750_000_000_000
    }
    fn rand32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        self.0
    }
}

type Tab = support::page_tab::Tab<At>;

/// A tab of its own on `node`: its own page, engine and session.
fn tab(node: &PageNode, seed: u32) -> Tab {
    Tab::open(node, At(seed), [seed as u8, 2, 3, 4])
}

fn schema() -> Schema {
    serde_json::from_value(json!({ "type": "Task", "fields": [{ "name": "title", "kind": "text", "required": true }] })).unwrap()
}

fn fields(t: &str) -> serde_json::Map<String, serde_json::Value> {
    json!({ "title": t }).as_object().unwrap().clone()
}

fn published(v: &[protocol::WriteState]) -> bool {
    v.contains(&protocol::WriteState::Published)
}

/// B read the record, A changed it, B's update from its stale read is REFUSED:
/// nothing applied, rolled back, and A's value stands for a fresh reader. The
/// control (no A in between) publishes.
///
/// It goes ONCE (R-b): B's engine takes it on its own older view, and when
/// its commit meets A's head -- a foreign move -- the ENGINE re-applies its
/// queue there and re-judges B's reads: `Conflict` + `Conflicted` naming the
/// key (M2). The client never sends it again.
#[test]
fn a_stale_update_is_refused_and_rolled_back_however_often_it_goes() {
    for interfere in [false, true] {
        let node = PageNode::new();
        let mut a = tab(&node, 1);
        a.call(|d| d.define("tasks", &schema())).expect("define");
        a.pump();
        a.seconds(3);
        let rec = a.call(|d| d.put("tasks", &fields("first"))).expect("put");
        a.pump();
        a.seconds(3);
        let loc = id::loc_from_hex(&rec.id).expect("an id");
        let key = craftworks_sdk::db::record_key("tasks", loc);
        let mut b = tab(&node, 2);
        if interfere {
            a.call(|d| d.update("tasks", loc, &fields("A's edit"))).expect("A's update");
            a.pump();
            a.seconds(3);
            let wa = a.db.store().writes.next_write_id() - 1;
            assert!(published(&a.told(wa)), "A's update did not publish");
        }
        let w = b.db.store().writes.next_write_id();
        b.call(|d| d.update("tasks", loc, &fields("B's edit"))).expect("B's update is made");
        b.pump();
        b.seconds(10);
        // ONCE (R-b): re-judging after a foreign move is the engine's, and
        // a re-run is a NEW write (`Db`'s), never this one again.
        let sent = b.sends(w).unwrap_or(0);
        assert_eq!(sent, 1, "interfere {interfere}: B's write went on the wire {sent} times");
        // `Conflicted` names the key whose read no longer held: at most one,
        // for the one judgement that refused it.
        assert!(b.conflicted_replies() <= 1, "`Conflicted` arrived {} times for one write", b.conflicted_replies());
        if interfere {
            // Refused where its read is judged (`Failed`), or — when B's
            // engine judged it on its own older view — lost to A's newer head
            // (`Lost`, R0: re-judged if ever re-sent). Either way NOT applied.
            assert!(
                matches!(b.told(w).last(), Some(protocol::WriteState::Failed | protocol::WriteState::FailedWhy { .. } | protocol::WriteState::Lost | protocol::WriteState::Conflict)),
                "a stale update was not refused: {:?}",
                b.told(w)
            );
            assert!(!published(&b.told(w)), "a stale update published over A's edit");
            assert_eq!(Reads::row_state(b.db.store(), &key), RowState::RolledBack);
            let mut fresh = tab(&node, 3);
            let got = fresh.call(|d| d.get("tasks", loc)).expect("read").expect("present");
            assert_eq!(got.fields["title"], json!("A's edit"), "A's edit did not stand");
        } else {
            assert!(published(&b.told(w)), "an update from a true read did not publish: {:?}", b.told(w));
        }
    }
}

/// Two sessions `create_at` ONE slot, each having read it absent: exactly ONE
/// record is made; the other is refused (sdk#149's stated residual, closed).
#[test]
fn two_create_ats_of_one_slot_make_one_record() {
    let node = PageNode::new();
    let mut a = tab(&node, 1);
    a.call(|d| d.define("tasks", &schema())).expect("define");
    a.pump();
    a.seconds(3);
    let mut b = tab(&node, 2);
    let slot = slot_from(1_749_999_000_000, "ns", "source-1");
    let wa = a.db.store().writes.next_write_id();
    let wb = b.db.store().writes.next_write_id();
    assert!(matches!(a.call(|d| d.create_at("tasks", slot, &fields("from A"))).expect("A"), CreateAt::Created(_)));
    assert!(matches!(b.call(|d| d.create_at("tasks", slot, &fields("from B"))).expect("B"), CreateAt::Created(_)), "B read the slot absent too");
    a.pump();
    b.pump();
    a.seconds(5);
    b.seconds(5);
    let made = [published(&a.told(wa)), published(&b.told(wb))];
    assert_eq!(made.iter().filter(|m| **m).count(), 1, "A {:?}, B {:?}", a.told(wa), b.told(wb));
}

/// Two sessions define ONE schema from one absent base: BOTH succeed (a write
/// already there is not a conflict — main's ruling). The builder's two tabs do
/// this on every mount.
#[test]
fn two_sessions_defining_one_schema_both_succeed() {
    let node = PageNode::new();
    let mut a = tab(&node, 1);
    let mut b = tab(&node, 2);
    let (wa, wb) = (a.db.store().writes.next_write_id(), b.db.store().writes.next_write_id());
    a.call(|d| d.define("tasks", &schema())).expect("A defines");
    b.call(|d| d.define("tasks", &schema())).expect("B defines");
    a.pump();
    b.pump();
    a.seconds(5);
    b.seconds(5);
    assert!(published(&a.told(wa)), "A: {:?}", a.told(wa));
    assert!(published(&b.told(wb)), "B's identical define was refused: {:?}", b.told(wb));
}

