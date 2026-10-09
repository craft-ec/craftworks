//! sdk#294: parity IS written and `ParityComplete` IS delivered, yet no row
//! said "backed up", so the builder's "saved + backed up" chip could never
//! appear (engineer1's measurement at ccff2d8, one tab and two: 120 Published,
//! 120 ParityComplete, owed 0, scan Done -- the same from t+0 s to t+39 s).
//! The row state is DERIVED from the one owner, `Server::key_state` (R-b):
//! with no parity owed over a tree known in full, a saved row says
//! `BACKED_UP`. Asserted here with one tab and with two, where it was only
//! measured before. Values of 1,500 B, stored by REFERENCE, so each write
//! owes parity (a 400 B value is inline and owes none: "backed up" would hold
//! from the first moment, and the control below would see it).

use craftworks_sdk::store::{Edit, Reads, RowState, Store};

fn run(two: bool) -> (usize, usize, usize) {
    let node = testkit::PageNode::new();
    let (mut a, conn_a, _k) = testkit::page_store(&node);
    let other = if two { Some(testkit::page_store(&node)) } else { None };
    let mut now = 1_000u64;
    let keys: Vec<Vec<u8>> = (0..120u32).map(|i| format!("n/{i:04}").into_bytes()).collect();
    for (i, k) in keys.iter().enumerate() {
        a.apply_batch(&[(k.clone(), Edit::Put(vec![(i % 251) as u8; 1500]))]).expect("taken");
        if i % 10 == 9 && i + 1 < keys.len() {
            now += 1_000;
            conn_a.clone().tick_at(now);
            a.sync();
            if let Some((_, cb, _)) = other.as_ref() {
                cb.clone().tick_at(now);
            }
        }
    }
    // THE CONTROL: a row whose blocks the node has NOT acknowledged does not
    // claim to be backed up -- or `BACKED_UP` would be a constant. Since race
    // put (COMMIT-LIFE §P) the parity is acked in the commit's own round, so
    // "before the tick" is no longer that moment; holding the node's answers is.
    a.sync();
    let held_key = b"n/held".to_vec();
    conn_a.clone().hold_answers();
    a.apply_batch(&[(held_key.clone(), Edit::Put(vec![9u8; 1500]))]).expect("taken");
    a.sync();
    let early = usize::from(a.row_state(&held_key) == RowState::BackedUp);
    conn_a.clone().stop_holding();
    while conn_a.held() > 0 {
        let _ = conn_a.clone().release_one();
    }
    let keys: Vec<Vec<u8>> = keys.into_iter().chain(std::iter::once(held_key)).collect();
    let mut saving = 0;
    for _ in 0..40 {
        now += 1_000;
        conn_a.clone().tick_at(now);
        a.sync();
        if let Some((_, cb, _)) = other.as_ref() {
            cb.clone().tick_at(now);
        }
    }
    let mut backed_up = 0;
    for k in &keys {
        match a.row_state(k) {
            RowState::BackedUp => backed_up += 1,
            RowState::Pending | RowState::Queued => saving += 1,
            _ => {}
        }
    }
    (backed_up, saving, early)
}

#[test]
fn a_saved_row_whose_parity_is_on_the_network_says_backed_up_with_one_tab_and_two() {
    for two in [false, true] {
        let (backed_up, saving, early) = run(two);
        println!("  two tabs={two}: {backed_up} of 121 rows BACKED_UP, {saving} still saving; held row backed up early: {early}");
        assert_eq!(early, 0, "two={two}: a row whose blocks the node never acknowledged claimed backed up: the state is a constant");
        assert_eq!(saving, 0, "two={two}: rows still saving after 40 s of ticks");
        assert_eq!(backed_up, 121, "two={two}: rows whose parity was written do not say backed up");
    }
}
