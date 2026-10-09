//! The probed fixtures: the only supported way to build the thing under test.
//!
//! # Why this crate exists
//!
//! Every test that built the system itself built it a little differently, and
//! the differences decided what the tests could SEE.
//!
//! - **A clock that never advanced, written out eleven times.**
//!   `CachedStore::new(Box::new(|| 0))` appeared in six files (the outbox,
//!   gone with R-b: the engine owns the write queue). Nothing in the
//!   SDK's tests could observe time passing, so a whole class of behaviour was
//!   untestable by construction rather than by decision. [`Clock`] is one a
//!   test can drive.
//! - **The node, hand-rolled per file.** A tab after the switch-over is the
//!   in-page engine's `page::Server` over a node; [`page_node`] is that, with
//!   the REAL signer and the REAL Register merge on the far side, shared by
//!   every tab on it. (The delegate-era cross-call `Node` and `FullNode`, which
//!   rebuilt the Shell from its context on every call, went with the Shell.)

pub mod mem_store;
pub mod model;
pub mod page_node;
pub use mem_store::MemStore;
pub use page_node::{PageConn, PageNode};

use std::cell::RefCell;
use std::rc::Rc;

/// A clock a test can DRIVE.
///
/// The thing it replaces is `Box::new(|| 0)` — a closure returning zero,
/// written by hand in six files. It is not that those tests chose a frozen
/// clock; it is that constructing one that moves was extra work in every
/// single test, so nobody did, and "does this behaviour need time?" became a
/// question the suite could not ask.
#[derive(Clone, Default)]
pub struct Clock(Rc<RefCell<u64>>);

impl Clock {
    pub fn new(start_ms: u64) -> Clock {
        Clock(Rc::new(RefCell::new(start_ms)))
    }
    /// Move time forward. The whole point of the type.
    pub fn advance(&self, ms: u64) {
        *self.0.borrow_mut() += ms;
    }
    pub fn now_ms(&self) -> u64 {
        *self.0.borrow()
    }
    /// The closure the SDK's constructors take.
    pub fn as_fn(&self) -> Box<dyn Fn() -> u64> {
        let c = self.0.clone();
        Box::new(move || *c.borrow())
    }
}

/// A tab's STORE (READ-STATE, design B): a `PageStore` lent a new tab on
/// `node`, started (its `Identity` sent and answered), with the tab handed
/// back so a test can drive its clock and look at its server, and the clock.
pub fn page_store(node: &PageNode) -> (craftworks_sdk::PageStore<PageConn>, PageConn, Clock) {
    let clock = Clock::new(0);
    let conn = node.connect();
    let mut store = craftworks_sdk::PageStore::new(clock.as_fn(), clock.as_fn());
    store.writes.client.send(&protocol::Request::Identity);
    store.set_host(conn.clone());
    (store, conn, clock)
}

/// A `PageStore` over a host the caller built (a test's own rig), on a clock
/// that can be driven, the clock handed back.
pub fn page_store_over<H: page::server::Host>(host: H) -> (craftworks_sdk::PageStore<H>, Clock) {
    let clock = Clock::new(0);
    let mut store = craftworks_sdk::PageStore::new(clock.as_fn(), clock.as_fn());
    store.set_host(host);
    (store, clock)
}
