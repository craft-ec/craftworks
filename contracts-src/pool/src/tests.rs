use super::*;
use craftworks_token_contract::clear::PRICE_ONE as ONE;

fn tok(n: u8) -> Tok {
    Tok { definition: [n; 32], item: [0; 32] }
}
fn batch() -> Batch {
    Batch { a: tok(1), b: tok(2), epoch_len: 10, grace: 4, beacon: [9; 32], epoch: 0 }
}
fn order(t: u8, owner: u8, n: u8, amount: u128, side: Side, limit: u128) -> Order {
    Order { escrow: (tok(t), [owner; 32], [n; 16]), amount, side, limit }
}
/// 1 sells 300 A at ≥ 2; 2 buys A with 500 B at ≤ 3.
fn two() -> Vec<Order> {
    let mut v = vec![order(1, 1, 7, 300, Side::Sell, 2 * ONE), order(2, 2, 8, 500, Side::Buy, 3 * ONE)];
    v.sort_by_key(|x| x.escrow);
    v
}
fn open(orders: Vec<Order>) -> State {
    State { section: Section { escrows: listed(&orders), ..Default::default() }, orders }
}

#[test]
fn a_batch_settles_to_exactly_its_clearing() {
    let p = batch();
    assert_eq!(Batch::decode(&p.encode()), Some(p));
    assert_eq!(p.window(), (10, 14));
    let orders = two();
    let v = verdict(&p, &orders);
    // Cleared at 2: 250 A for 500 B. 1: 50 A back, 500 B paid out; 2: nothing back, 250 A paid out.
    let sell = orders.iter().find(|o| o.side == Side::Sell).unwrap().escrow;
    let buy = orders.iter().find(|o| o.side == Side::Buy).unwrap().escrow;
    assert_eq!(v.refund_of(&sell), Some(50));
    assert_eq!(v.refund_of(&buy), Some(0));
    assert_eq!(v.pay(&pay_key(&sell)), Some((tok(2), [1; 32], 500)));
    assert_eq!(v.pay(&pay_key(&buy)), Some((tok(1), [2; 32], 250)));
    assert_eq!(v.holds(), Ok(()), "nothing paid out past what was escrowed");
    let settled = State { section: v, orders: orders.clone() };
    assert_eq!(State::decode(&settled.encode()), Some(settled.clone()));
    assert_eq!(check(&p, &settled, 14), Ok(()));
    assert_eq!(check(&p, &settled, 13), Err("a verdict before the batch settles"));
    let mut wrong = settled.clone();
    wrong.section.pays[0].3 += 1;
    assert_eq!(check(&p, &wrong, 14), Err("a verdict that is not the batch's clearing"));
    // Before it settles: its orders, no verdict.
    assert_eq!(check(&p, &open(orders.clone()), 3), Ok(()));
    let mut lied = open(orders.clone());
    lied.section.escrows[0].2 = escrow::terms_hash(b"other terms");
    assert_eq!(check(&p, &lied, 3), Err("a section that is not its orders"), "listed on terms not its own");
    // An order in the wrong token for its side; out of order.
    assert_eq!(check(&p, &open(vec![order(2, 1, 7, 300, Side::Sell, ONE)]), 3), Err("an order in the wrong token, or of nothing"));
    let mut rev = orders.clone();
    rev.reverse();
    assert_eq!(check(&p, &open(rev), 3), Err("orders not in their order"));
    // Nothing crosses: everything back, nothing paid out.
    let none = verdict(&p, &[order(1, 1, 7, 300, Side::Sell, 4 * ONE), order(2, 2, 8, 500, Side::Buy, 3 * ONE)]);
    assert!(none.pays.is_empty());
    assert_eq!(none.refunds.iter().map(|r| r.1).collect::<Vec<_>>(), vec![300, 500]);
}

#[test]
fn a_batch_takes_orders_until_it_settles_and_nothing_after() {
    let p = batch();
    let orders = two();
    let (one, other) = (open(vec![orders[0]]), open(vec![orders[1]]));
    // Before: the union, whichever order.
    assert_eq!(merge(&p, &one, &other, 13), open(orders.clone()));
    assert_eq!(merge(&p, &other, &one, 13), open(orders.clone()));
    // After: nothing new.
    assert_eq!(merge(&p, &one, &other, 14), one);
    // A settled copy over an unsettled one — only from the settle height.
    let settled = State { section: verdict(&p, &orders), orders: orders.clone() };
    assert_eq!(merge(&p, &open(orders.clone()), &settled, 13), open(orders.clone()), "not before it settles");
    assert_eq!(merge(&p, &open(orders.clone()), &settled, 14), settled);
    assert_eq!(merge(&p, &settled, &open(orders.clone()), 14), settled);
    // Of two settled on different sets: the one with more orders, either way round.
    let small = State { section: verdict(&p, &orders[..1]), orders: orders[..1].to_vec() };
    assert_eq!(merge(&p, &small, &settled, 20), settled);
    assert_eq!(merge(&p, &settled, &small, 20), settled);
}
