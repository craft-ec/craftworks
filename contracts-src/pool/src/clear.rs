//! BATCH CLEARING (TOKEN §5): a pool's epoch of orders cleared at ONE uniform price — a pure function, so anyone
//! computes the same result and order within the batch does not matter (nobody can front-run).
//!
//! Orders are LIMIT orders on the pair (A, B), prices in B per A, fixed point ([`PRICE_ONE`] = 1):
//! - a SELL gives up to `amount` of A for B, at no less than `limit`;
//! - a BUY gives up to `amount` of B for A, at no more than `limit`.
//!
//! A liquidity provider's curve is a ladder of such orders (built by its client): one primitive here.
//!
//! The price: among every order's limit, the one that matches the most A — ties to the smaller imbalance, then the
//! lower price. At it, the short side fills whole, the long side pro rata (rounded down; what rounding leaves goes to
//! the earliest orders, by their place in the batch's canonical order). Everything in integers, the same everywhere.

/// Price fixed point: a price of `PRICE_ONE` is 1 B per A.
pub const PRICE_ONE: u128 = 1_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    Sell,
    Buy,
}

/// One order: its side, its amount (A for a sell, B for a buy), its limit price.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Order {
    pub side: Side,
    pub amount: u128,
    pub limit: u128,
}

/// What an order got: `paid` of what it gave (A for a sell, B for a buy), `got` of the other; the rest refunded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Fill {
    pub paid: u128,
    pub got: u128,
}

/// The batch's result: the price (`None`: nothing crosses) and each order's fill, in the orders' order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cleared {
    pub price: Option<u128>,
    pub fills: Vec<Fill>,
}

/// A of `b` B at price `p` (rounded down).
fn a_of(b: u128, p: u128) -> u128 {
    b.saturating_mul(PRICE_ONE) / p
}
/// B of `a` A at price `p` (rounded down).
fn b_of(a: u128, p: u128) -> u128 {
    a.saturating_mul(p) / PRICE_ONE
}

/// At price `p`: the A offered (sells whose limit is at most `p`) and the A wanted (buys whose limit is at least `p`).
fn sides(orders: &[Order], p: u128) -> (u128, u128) {
    let mut supply = 0u128;
    let mut demand = 0u128;
    for o in orders {
        match o.side {
            Side::Sell if o.limit <= p => supply = supply.saturating_add(o.amount),
            Side::Buy if o.limit >= p => demand = demand.saturating_add(a_of(o.amount, p)),
            _ => {}
        }
    }
    (supply, demand)
}

/// Shares of `total` among `weights` pro rata, rounded down; what is left over, one each to the earliest.
fn shares(weights: &[u128], total: u128) -> Vec<u128> {
    let sum: u128 = weights.iter().fold(0u128, |s, w| s.saturating_add(*w));
    if sum == 0 {
        return vec![0; weights.len()];
    }
    let mut out: Vec<u128> = weights.iter().map(|w| mul_div(*w, total, sum)).collect();
    let mut left = total.saturating_sub(out.iter().sum());
    for (i, w) in weights.iter().enumerate() {
        if left == 0 {
            break;
        }
        if out[i] < *w {
            out[i] += 1;
            left -= 1;
        }
    }
    out
}

/// `a × b / c` without overflowing in between (exact while the result fits).
fn mul_div(a: u128, b: u128, c: u128) -> u128 {
    if c == 0 {
        return 0;
    }
    match a.checked_mul(b) {
        Some(x) => x / c,
        None => {
            // Split a = q·c + r: a·b/c = q·b + r·b/c.
            let (q, r) = (a / c, a % c);
            q.saturating_mul(b).saturating_add(r.checked_mul(b).map_or_else(|| mul_div(b, r, c), |x| x / c))
        }
    }
}

/// CLEAR one batch.
pub fn clear(orders: &[Order]) -> Cleared {
    let none = Cleared { price: None, fills: vec![Fill::default(); orders.len()] };
    let mut prices: Vec<u128> = orders.iter().filter(|o| o.limit > 0 && o.amount > 0).map(|o| o.limit).collect();
    prices.sort();
    prices.dedup();
    // The price matching the most A; ties: the smaller imbalance, then the lower price.
    let mut best: Option<(u128, u128, u128)> = None; // (volume, imbalance, price)
    for &p in &prices {
        let (s, d) = sides(orders, p);
        let v = s.min(d);
        if v == 0 {
            continue;
        }
        let imb = s.abs_diff(d);
        let better = match best {
            None => true,
            Some((bv, bi, _)) => v > bv || (v == bv && imb < bi),
        };
        if better {
            best = Some((v, imb, p));
        }
    }
    let Some((volume, _, p)) = best else { return none };
    // Each side's share of the volume (in A): the short side whole, the long side pro rata.
    let sell_in: Vec<usize> = (0..orders.len()).filter(|&i| orders[i].side == Side::Sell && orders[i].limit <= p && orders[i].amount > 0).collect();
    let buy_in: Vec<usize> = (0..orders.len()).filter(|&i| orders[i].side == Side::Buy && orders[i].limit >= p && orders[i].amount > 0).collect();
    let sell_a = shares(&sell_in.iter().map(|&i| orders[i].amount).collect::<Vec<_>>(), volume);
    let buy_a = shares(&buy_in.iter().map(|&i| a_of(orders[i].amount, p)).collect::<Vec<_>>(), volume);
    let mut fills = vec![Fill::default(); orders.len()];
    for (k, &i) in sell_in.iter().enumerate() {
        fills[i] = Fill { paid: sell_a[k], got: b_of(sell_a[k], p) };
    }
    for (k, &i) in buy_in.iter().enumerate() {
        // A buy pays the B its A costs at the price (rounded up, never past what it gave).
        let cost = buy_a[k].saturating_mul(p).div_ceil(PRICE_ONE).min(orders[i].amount);
        fills[i] = Fill { paid: cost, got: buy_a[k] };
    }
    Cleared { price: Some(p), fills }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: u128 = PRICE_ONE;
    fn sell(amount: u128, limit: u128) -> Order {
        Order { side: Side::Sell, amount, limit }
    }
    fn buy(amount: u128, limit: u128) -> Order {
        Order { side: Side::Buy, amount, limit }
    }

    #[test]
    fn one_price_clears_where_the_most_trades() {
        // Sellers: 100 A at ≥ 1, 100 A at ≥ 2. Buyers: 300 B at ≤ 3, 100 B at ≤ 1.5.
        let o = vec![sell(100, ONE), sell(100, 2 * ONE), buy(300, 3 * ONE), buy(100, 3 * ONE / 2)];
        let c = clear(&o);
        // At 1.5: supply 100, demand 200+66 → 100 A. At 2: supply 200, demand 150 → 150 A (the most). At 3: 200 vs 100.
        assert_eq!(c.price, Some(2 * ONE));
        assert_eq!(c.fills[0], Fill { paid: 75, got: 150 }, "sells share 150 A pro rata: 75 each, at 2");
        assert_eq!(c.fills[1], Fill { paid: 75, got: 150 });
        assert_eq!(c.fills[2], Fill { paid: 300, got: 150 }, "the buy at ≤ 3 takes all 150 A for 300 B");
        assert_eq!(c.fills[3], Fill::default(), "the buy at ≤ 1.5 is below the price: nothing");
        // A and B both balance: what sellers get is what buyers pay; what buyers get, sellers paid.
        let (paid_a, got_b): (u128, u128) = (c.fills[0].paid + c.fills[1].paid, c.fills[0].got + c.fills[1].got);
        assert_eq!((paid_a, got_b), (c.fills[2].got, c.fills[2].paid));
    }

    #[test]
    fn ties_go_to_the_smaller_imbalance_then_the_lower_price() {
        // 100 A at ≥ 1 against 200 B at ≤ 2: 100 A trade at 1 (demand 200, imbalance 100) and at 2 (demand 100,
        // imbalance 0) — the price that leaves nobody over: 2.
        assert_eq!(clear(&[sell(100, ONE), buy(200, 2 * ONE)]).price, Some(2 * ONE));
        // Equal volume AND imbalance at 1 and at 2 (10 A, 20 over at each): the lower.
        let o = [sell(10, ONE), sell(20, 2 * ONE), buy(10, ONE), buy(20, 2 * ONE)];
        assert_eq!(clear(&o).price, Some(ONE));
    }

    #[test]
    fn nothing_crosses_nothing_clears() {
        assert_eq!(clear(&[sell(100, 3 * ONE), buy(100, 2 * ONE)]).price, None);
        assert_eq!(clear(&[sell(100, ONE)]).price, None);
        assert_eq!(clear(&[]).price, None);
        assert_eq!(clear(&[sell(0, ONE), buy(100, 2 * ONE)]).price, None);
    }

    #[test]
    fn the_result_does_not_depend_on_who_came_first_except_rounding() {
        let o = vec![sell(70, ONE), sell(30, ONE), buy(50, ONE)];
        let c = clear(&o);
        assert_eq!(c.price, Some(ONE));
        // 50 A among sells of 70 and 30: 35 and 15.
        assert_eq!((c.fills[0].paid, c.fills[1].paid), (35, 15));
        let r: Vec<Order> = o.iter().rev().cloned().collect();
        let cr = clear(&r);
        assert_eq!(cr.price, c.price);
        assert_eq!((cr.fills[2].paid, cr.fills[1].paid), (35, 15));
        // Rounding: 10 A among three equal sells: 4, 3, 3 — the earliest gets the rest.
        let t = clear(&[sell(5, ONE), sell(5, ONE), sell(5, ONE), buy(10, ONE)]);
        assert_eq!(t.fills.iter().take(3).map(|f| f.paid).collect::<Vec<_>>(), vec![4, 3, 3]);
    }

    #[test]
    fn nothing_is_created_and_no_one_trades_past_their_limit() {
        // A grid of orders: every fill within its order, at a price within its limit, and A and B conserved.
        let mut o = Vec::new();
        for i in 1..=7u128 {
            o.push(sell(13 * i, i * ONE / 3));
            o.push(buy(17 * i, (8 - i) * ONE / 3));
        }
        let c = clear(&o);
        let p = c.price.unwrap();
        let (mut a_sold, mut a_bought, mut b_paid, mut b_got) = (0, 0, 0, 0);
        for (x, f) in o.iter().zip(&c.fills) {
            assert!(f.paid <= x.amount);
            match x.side {
                Side::Sell => {
                    assert!(f.paid == 0 || x.limit <= p);
                    a_sold += f.paid;
                    b_got += f.got;
                }
                Side::Buy => {
                    assert!(f.paid == 0 || x.limit >= p);
                    a_bought += f.got;
                    b_paid += f.paid;
                }
            }
        }
        assert_eq!(a_sold, a_bought, "A: every unit sold is bought");
        assert!(b_got <= b_paid, "B: sellers get no more than buyers pay (rounding favours the batch)");
        assert!(b_paid - b_got <= o.len() as u128, "…by at most a unit an order");
    }

    #[test]
    fn mul_div_does_not_overflow() {
        assert_eq!(mul_div(u128::MAX / 3, 3, 3), u128::MAX / 3);
        assert_eq!(mul_div(1 << 100, 1 << 27, 1 << 27), 1 << 100);
    }
}
