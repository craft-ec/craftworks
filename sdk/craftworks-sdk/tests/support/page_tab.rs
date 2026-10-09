//! A TAB as an app has one (READ-STATE, design B): `Db` over the page's store
//! (`PageStore`), whose reads WALK the in-page engine's tree and whose writes
//! reach its `page::Server` in the same call — over the page path's scripted
//! node (`testkit::PageNode`).
//!
//! The one thing added to `testkit::page_store` is a WATCH on the tab's
//! traffic: every write frame the store sends (a re-send is visible as a
//! second send) and every reply it is handed. A test that counts sends or
//! reads verdicts reads them here, never by taking the store's frames itself
//! — the store is what delivers them.

#![allow(dead_code)]

use craftworks_sdk::{Db, DbError, Ended, Env, Outcome, PageStore};
use page::server::{Host, Server};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use testkit::{Clock, PageConn, PageNode};

/// What crossed between the store and its tab.
#[derive(Default)]
pub struct Log {
    /// Per write id, how many times it went on the wire (a `Commit` or a `Write`).
    pub sends: BTreeMap<u64, usize>,
    /// Every reply the store was handed, decoded, in order.
    pub replies: Vec<protocol::Reply>,
}

/// The tab, watched.
pub struct Watched {
    pub conn: PageConn,
    pub log: Rc<RefCell<Log>>,
}

impl Host for Watched {
    fn with_server<R>(&mut self, f: impl FnOnce(&mut Server) -> R) -> R {
        Host::with_server(&mut self.conn, f)
    }
    fn peek<R>(&self, f: impl FnOnce(&Server) -> R) -> R {
        Host::peek(&self.conn, f)
    }
    fn client(&mut self, frame: &[u8]) {
        if let protocol::Incoming::Ok(env) = protocol::decode_request(frame) {
            if let protocol::Request::Write { write_id, .. } | protocol::Request::Commit { write_id, .. } = env.body {
                *self.log.borrow_mut().sends.entry(write_id).or_default() += 1;
            }
        }
        Host::client(&mut self.conn, frame);
    }
    fn take_replies(&mut self) -> Vec<Vec<u8>> {
        let out = Host::take_replies(&mut self.conn);
        let mut log = self.log.borrow_mut();
        log.replies.extend(out.iter().filter_map(|r| protocol::decode_reply(r).ok()));
        out
    }
}

/// One tab: its `Db`, the tab it runs on, its clock and its log.
pub struct Tab<E: Env> {
    pub db: Db<PageStore<Watched>, E>,
    pub conn: PageConn,
    pub clock: Clock,
    pub log: Rc<RefCell<Log>>,
}

impl<E: Env> Tab<E> {
    /// A new tab on `node`, started (its `Identity` sent and answered), as
    /// `testkit::page_store` starts one.
    pub fn open(node: &PageNode, env: E, device: [u8; 4]) -> Tab<E> {
        Tab::open_with(node, engine::Params::default(), env, device)
    }

    /// The same, its page's engine on these parameters (a small queue bound).
    pub fn open_with(node: &PageNode, params: engine::Params, env: E, device: [u8; 4]) -> Tab<E> {
        let clock = Clock::new(0);
        let conn = node.connect_with(params);
        let log = Rc::new(RefCell::new(Log::default()));
        let mut store = PageStore::new(clock.as_fn(), clock.as_fn());
        store.writes.client.send(&protocol::Request::Identity);
        store.set_host(Watched { conn: conn.clone(), log: log.clone() });
        Tab { db: Db::new(store, env, device), conn, clock, log }
    }

    /// A `Db` call made the way a page makes it (`engine-db.js`'s `once`):
    /// a refusal for want of blocks is WAITED on — its ticket, until it ends
    /// — then RESUMED and asked again, at most `engine-db.js`'s `MAX_HOPS`
    /// times. What `PageStore::decide` decides, not a copy of it.
    pub fn call<T>(&mut self, mut f: impl FnMut(&mut Db<PageStore<Watched>, E>) -> Result<T, DbError>) -> Result<T, DbError> {
        const MAX_HOPS: usize = 8;
        let mut last = None;
        for _ in 0..MAX_HOPS {
            let r = f(&mut self.db);
            match self.db.store_mut().decide(r) {
                Outcome::Done(v) => return Ok(v),
                Outcome::Told(e) => return Err(e),
                Outcome::Wait(e, t) => {
                    let mut how = None;
                    for _ in 0..50 {
                        if let Some((_, h)) = self.db.store_mut().take_ended().into_iter().find(|(id, _)| *id == t) {
                            how = Some(h);
                            break;
                        }
                        self.seconds(1);
                    }
                    match how {
                        Some(Ended::Loaded) => self.db.store_mut().resume(t),
                        _ => return Err(e),
                    }
                    last = Some(e);
                }
            }
        }
        Err(last.expect("a hop was made"))
    }

    /// Carry what is waiting both ways, as the page's pump does.
    pub fn pump(&mut self) {
        self.db.store_mut().sync();
    }

    /// `n` seconds pass: the ask after an applying write, then a pump.
    /// Nothing times a write or a read out (R-b; rules 7, 8).
    pub fn seconds(&mut self, n: u64) {
        for _ in 0..n {
            self.clock.advance(1000);
            let _ = self.db.store_mut().ask_after_applying();
            self.pump();
        }
    }

    /// The states THIS tab's writes were told, per write id, in order.
    pub fn verdicts(&self) -> BTreeMap<u64, Vec<protocol::WriteState>> {
        let session = self.db.store().writes.client.session();
        let mut out: BTreeMap<u64, Vec<protocol::WriteState>> = BTreeMap::new();
        for r in &self.log.borrow().replies {
            if let protocol::Reply::SessionWriteState { session: s, write_id, state } = r {
                if Some(*s) == session {
                    out.entry(*write_id).or_default().push(*state);
                }
            }
        }
        out
    }

    /// The states write `id` was told, in order.
    pub fn told(&self, id: u64) -> Vec<protocol::WriteState> {
        self.verdicts().get(&id).cloned().unwrap_or_default()
    }

    /// How many times write `id` went on the wire.
    pub fn sends(&self, id: u64) -> Option<usize> {
        self.log.borrow().sends.get(&id).copied()
    }

    /// How many `Conflicted` replies this tab was handed.
    pub fn conflicted_replies(&self) -> usize {
        self.log.borrow().replies.iter().filter(|r| matches!(r, protocol::Reply::Conflicted { .. })).count()
    }
}
