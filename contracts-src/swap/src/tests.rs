use super::*;

fn tok(n: u8) -> Tok {
    Tok { definition: [n; 32], item: [0; 32] }
}
/// X (1) gives 300 of token 1; Y (2) gives 500 of token 2; by 20 (cutoff 24).
fn deal() -> Deal {
    Deal { x: tok(1), party_x: [1; 32], amount_x: 300, y: tok(2), party_y: [2; 32], amount_y: 500, deadline: 20, beacon: [9; 32] }
}
fn ex() -> EscrowRef {
    (tok(1), [1; 32], [7; 16])
}
fn ey() -> EscrowRef {
    (tok(2), [2; 32], [8; 16])
}
fn alone(r: EscrowRef, amount: u128) -> Section {
    Section { escrows: vec![(r, amount, escrow::terms_hash(TERMS))], ..Default::default() }
}

#[test]
fn a_deal_crosses_when_both_escrow_or_refunds_after_its_deadline() {
    let d = deal();
    assert_eq!(Deal::decode(&d.encode()), Some(d));
    let c = crossed(&d, &ex(), &ey());
    assert_eq!(c.holds(), Ok(()));
    assert_eq!(c.pay(&pay_key(&ex())), Some((tok(2), [1; 32], 500)), "X gets Y's 500");
    assert_eq!(c.pay(&pay_key(&ey())), Some((tok(1), [2; 32], 300)), "Y gets X's 300");
    assert_eq!((c.refund_of(&ex()), c.refund_of(&ey())), (Some(0), Some(0)));
    assert_eq!(check(&d, &c, 0), Ok(()));
    assert_eq!(check(&d, &c, 1000), Ok(()), "valid then, valid after");
    let r = refunded(&d, &ex(), true);
    assert_eq!(check(&d, &r, 24), Ok(()));
    assert_eq!(check(&d, &r, 23), Err("a refund before the deal's deadline"));
    assert_eq!(check(&d, &alone(ex(), 300), 0), Ok(()));
    assert_eq!(check(&d, &alone(ex(), 299), 0), Err("an escrow that is not this deal's"), "another amount");
    assert_eq!(check(&d, &alone((tok(1), [3; 32], [7; 16]), 300), 0), Err("an escrow that is not this deal's"), "another party");
    let mut terms = alone(ex(), 300);
    terms.escrows[0].2 = escrow::terms_hash(b"other");
    assert_eq!(check(&d, &terms, 0), Err("an escrow that is not this deal's"));
    let mut two = alone(ex(), 300);
    two.escrows.push(((tok(1), [1; 32], [9; 16]), 300, escrow::terms_hash(TERMS)));
    assert_eq!(check(&d, &two, 0), Err("two escrows of one side"));
    // Both listed, no verdict, out of their canonical order: not a state of the deal.
    let h = escrow::terms_hash(TERMS);
    let unsorted = Section { escrows: vec![(ey(), 500, h), (ex(), 300, h)], ..Default::default() };
    assert_eq!(check(&d, &unsorted, 0), Err("a state that is not its escrows"));
    let mut lie = c.clone();
    lie.pays[0].3 = 600;
    assert_eq!(check(&d, &lie, 0), Err("a verdict that is not the deal crossed"));
    let mut lie = r.clone();
    lie.refunds[0].1 = 1;
    assert_eq!(check(&d, &lie, 30), Err("a verdict that is not the refund"));
}

#[test]
fn a_verdict_once_taken_is_never_replaced() {
    let d = deal();
    let (x, y) = (alone(ex(), 300), alone(ey(), 500));
    // Both meet before the cutoff: crossed, whichever came first.
    assert_eq!(merge(&d, &x, &y, 10), crossed(&d, &ex(), &ey()));
    assert_eq!(merge(&d, &y, &x, 23), crossed(&d, &ex(), &ey()));
    // After it: not crossed — the union only, refunded by someone.
    assert_eq!(merge(&d, &x, &y, 24).pays, vec![]);
    let r = refunded(&d, &ex(), true);
    assert_eq!(merge(&d, &x, &r, 24), r);
    assert_eq!(merge(&d, &x, &r, 23), x, "a refund before the cutoff is not taken");
    let c = crossed(&d, &ex(), &ey());
    assert_eq!(merge(&d, &x, &c, 24), x, "a crossing after the cutoff is not taken");
    // A verdict held: kept — a crossing never replaces a refund (paid out already), nor the other way.
    assert_eq!(merge(&d, &r, &c, 10), r);
    assert_eq!(merge(&d, &c, &r, 30), c);
}
