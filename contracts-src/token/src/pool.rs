//! POOLS (TOKEN §5): a pool is a DEFINITION of its own ([`Pool`]: its two tokens, its epoch length, its grace) — a
//! contract whose state is its params — and, per epoch, an ORDER REGISTRY ([`Params`] → [`Registry`]): pointers to the
//! orders of that batch (each an `Order` entry in its trader's chain). The registry takes no pointer from the batch's
//! close plus its grace on, so every clearer clears the same set; the clearing itself is `clear::clear`.

use crate::{Tok, Why};

pub const POOL: u8 = 8;
pub const ORDERS: u8 = 9;
/// The most orders one batch holds.
pub const ORDERS_MAX: usize = 4096;

/// A POOL: tokens A and B (prices are B per A), batches of `epoch` beacon steps, settled `grace` steps after each closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pool {
    pub code_hash: [u8; 32],
    pub a: Tok,
    pub b: Tok,
    pub epoch: u64,
    pub grace: u64,
    pub beacon: [u8; 32],
}

impl Pool {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = vec![POOL];
        v.extend_from_slice(&self.code_hash);
        self.a.put(&mut v);
        self.b.put(&mut v);
        v.extend_from_slice(&self.epoch.to_le_bytes());
        v.extend_from_slice(&self.grace.to_le_bytes());
        v.extend_from_slice(&self.beacon);
        v
    }
    pub fn decode(b: &[u8]) -> Option<Pool> {
        let mut r = crate::R(b);
        (r.u8()? == POOL).then_some(())?;
        let p = Pool { code_hash: r.arr()?, a: Tok::read(&mut r)?, b: Tok::read(&mut r)?, epoch: r.u64()?, grace: r.u64()?, beacon: r.arr()? };
        (r.done() && p.a != p.b && p.epoch > 0).then_some(p)
    }
    pub fn id(&self) -> [u8; 32] {
        crate::id(&self.code_hash, &self.encode())
    }
    /// Batch `e`: (its close — orders stamped before it —, its settle height — close plus grace).
    pub fn window(&self, e: u64) -> (u64, u64) {
        let close = (e + 1).saturating_mul(self.epoch);
        (close, close.saturating_add(self.grace))
    }
    /// The batch an order stamped `stamp` is in.
    pub fn batch(&self, stamp: u64) -> u64 {
        stamp / self.epoch
    }
}

/// A batch's REGISTRY: its pool, its epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    pub code_hash: [u8; 32],
    pub pool: [u8; 32],
    pub beacon: [u8; 32],
    pub epoch: u64,
    /// The height from which it takes nothing: the batch's settle height (from the pool).
    pub frozen_at: u64,
}

impl Params {
    pub fn of(p: &Pool, e: u64) -> Params {
        Params { code_hash: p.code_hash, pool: p.id(), beacon: p.beacon, epoch: e, frozen_at: p.window(e).1 }
    }
    pub fn encode(&self) -> Vec<u8> {
        [&[ORDERS][..], &self.code_hash, &self.pool, &self.beacon, &self.epoch.to_le_bytes(), &self.frozen_at.to_le_bytes()].concat()
    }
    pub fn decode(b: &[u8]) -> Option<Params> {
        let mut r = crate::R(b);
        (r.u8()? == ORDERS).then_some(())?;
        let p = Params { code_hash: r.arr()?, pool: r.arr()?, beacon: r.arr()?, epoch: r.u64()?, frozen_at: r.u64()? };
        r.done().then_some(p)
    }
    pub fn id(&self) -> [u8; 32] {
        crate::id(&self.code_hash, &self.encode())
    }
}

/// An order's POINTER: the token of the chain it is in, its owner, its nonce.
pub type OrderRef = (Tok, [u8; 32], [u8; 16]);

/// A registry: the batch's order pointers, sorted (the batch's CANONICAL ORDER — what clearing's rounding follows).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registry {
    pub orders: Vec<OrderRef>,
}

impl Registry {
    pub fn encode(&self) -> Vec<u8> {
        let mut v = b"CWOR".to_vec();
        v.extend_from_slice(&(self.orders.len() as u32).to_le_bytes());
        for (t, o, n) in &self.orders {
            t.put(&mut v);
            v.extend_from_slice(o);
            v.extend_from_slice(n);
        }
        v
    }
    pub fn decode(b: &[u8]) -> Option<Registry> {
        let mut r = crate::R(b.strip_prefix(b"CWOR")?);
        let n = r.u32()? as usize;
        (n <= ORDERS_MAX).then_some(())?;
        let orders = (0..n).map(|_| Some((Tok::read(&mut r)?, r.arr()?, r.arr()?))).collect::<Option<Vec<OrderRef>>>()?;
        (r.done() && orders.windows(2).all(|w| w[0] < w[1])).then_some(Registry { orders })
    }
}

/// The union, sorted.
pub fn merge(a: &Registry, b: &Registry) -> Registry {
    let mut v: Vec<OrderRef> = a.orders.iter().chain(&b.orders).copied().collect();
    v.sort();
    v.dedup();
    Registry { orders: v }
}

/// THE FREEZE: `held` takes `given` only while the beacon (`now`) is below its batch's settle height; after, it keeps
/// what it holds (a pointer arriving late is ignored — copies keep syncing). Orders are stamped before the close, so
/// their pointers have the whole grace to spread.
pub fn take(p: &Params, held: &Registry, given: &Registry, now: u64) -> Result<Registry, Why> {
    if now >= p.frozen_at {
        return Ok(held.clone());
    }
    let m = merge(held, given);
    if m.orders.len() > ORDERS_MAX {
        return Err("more orders than a batch holds");
    }
    Ok(m)
}

