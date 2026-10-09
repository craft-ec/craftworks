//! The retry clock of EVERY node call on the page path (main's B2 ruling): an
//! RFC 6298 retransmission timeout over completed calls, and — for block GETs
//! — a congestion window. Nothing here is fixed; both move with what the node
//! does (the owner: per call, adaptive, moving with the network).
//!
//! The same rules as the cold reader (`craftworks_sdk::cold`, sdk#212), kept
//! here because the page crate does not depend on the SDK crate:
//! * `RTO = SRTT + 4·RTTVAR`, a [`RTO_FLOOR_MS`] floor, [`RTO_INITIAL_MS`]
//!   before any sample, [`RTO_MAX_MS`] ceiling;
//! * KARN'S RULE — a call that was re-sent is never a sample: which copy was
//!   answered is unknowable;
//! * §5.5 BACK-OFF — a timeout doubles the RTO (once per tick, however many
//!   timed out in it), until a call answers on its first send;
//! * the WINDOW — start [`WINDOW_INITIAL`], +1 per answer below the slow-start
//!   threshold, +1 per window above it, halved on a LOSS EPISODE (the page
//!   decides what one is: a GET's first timeout, sent after the last halving;
//!   sdk#345), the threshold set there, never below [`WINDOW_FLOOR`].

/// Before any sample (RFC 6298 §2.1).
pub const RTO_INITIAL_MS: f64 = 1_000.0;
/// A loopback node answers in a few ms; a timeout that close re-sends on
/// scheduling jitter alone.
pub const RTO_FLOOR_MS: f64 = 100.0;
/// RFC 6298 §2.5 allows a ceiling of at least 60 s.
pub const RTO_MAX_MS: f64 = 60_000.0;
/// THE NODE'S OWN BOUNDS on a GET (freenet 0.2.138; FREENET-CONSTRAINTS F64, re-read at every bump): what a node
/// does while a GET is still fetching, so a caller re-asks the NODE, not the network, only when that GET is over.
///
/// The node's web path waits this long for a GET, then answers 503 (the "transient contract-fetch error" retry page)
/// while the GET GOES ON: it only disconnects its own transient client (`server/path_handlers.rs:1500`,
/// `ensure_contract_cached`). Measured: V20 `latency=30002 ms` (the stall review, 2026-09-26).
pub const NODE_WEB_BOUND_MS: f64 = 30_000.0;
/// The node's attempts at one GET: the first and `MAX_RETRIES = 3` more (`operations/get/op_ctx_task.rs:2305`); a
/// stream failure spends the same budget (1455-1457).
pub const GET_ATTEMPTS: f64 = 4.0;
/// How long the node waits on one attempt: `OPERATION_TTL`, 60 s (`config.rs:59`; `operations/op_ctx.rs:466-468`,
/// applied at 781).
pub const GET_ATTEMPT_MS: f64 = 60_000.0;
/// B: the longest one node GET runs against SILENT peers (the architect's ruling on sdk#447). The stream claim runs
/// only after a header (`op_ctx_task.rs:1495-1503`, 2151), so it does not add to this; a trickling stream has NO
/// bound (`transport/peer_connection/streaming.rs:55`, reset per fragment at 399-401). Nothing caps the op
/// (`node/op_state_manager.rs:2536-2541`). Measured (probe `get-in-flight`, F64): with one and with two silent peers the
/// node answered NotFound at ~120 s, inside B; B stays the source's upper bound (main), since an earlier answer
/// re-arms the page at once and B governs only a node still silent.
pub const NODE_GET_BOUND_MS: f64 = GET_ATTEMPTS * GET_ATTEMPT_MS;
/// A non-200 that took at least this long is the web bound's 503: the node's GET is still in flight. DERIVED, one
/// RTO floor below the bound: the node answers at its own deadline, and a loopback answer arrives within a floor of it
/// (V20: 30,002 ms for a 30,000 ms bound); anything faster did not wait the bound out (the join window's immediate 503,
/// a refusal), so no GET of the node's is left running.
pub const STILL_FETCHING_AFTER_MS: f64 = NODE_WEB_BOUND_MS - RTO_FLOOR_MS;
/// WHEN A SOURCE WHOSE NODE IS STILL FETCHING IS ASKED AGAIN (main's re-rule of sdk#447, after batch 4's realnet):
/// each wait counts from that source's LAST slow answer, the n-th wait the n-th entry, the last repeating. The first is
/// one RTO floor: a re-ask of a node that has meanwhile FINISHED reads its local store and answers at once (it looks
/// there before it routes), so the piece is taken the moment the node has it, not up to B later. While the node is
/// still fetching, that re-ask costs one duplicate node GET, and the waits then double from the web bound up to B, so
/// the duplicates over one B are few (three) rather than one per RTO. The page can't ask the node's store directly at
/// this point: the loader races its bundles before the SDK's wasm, its session and its signer exist.
pub fn still_fetching_reask_ms() -> Vec<f64> {
    let mut waits = vec![RTO_FLOOR_MS];
    let mut w = NODE_WEB_BOUND_MS;
    while w < NODE_GET_BOUND_MS {
        waits.push(w);
        w *= 2.0;
    }
    waits.push(NODE_GET_BOUND_MS);
    waits
}
/// Block GETs in flight to start with.
pub const WINDOW_INITIAL: f64 = 4.0;
/// Block GETs the window always allows (sdk#345): one silent block never
/// holds the whole pipe, so the reads behind it keep moving.
pub const WINDOW_FLOOR: f64 = 2.0;

#[derive(Debug, Clone)]
pub struct Rto {
    srtt: Option<f64>,
    rttvar: f64,
    backoff: u32,
}

impl Default for Rto {
    fn default() -> Rto {
        Rto { srtt: None, rttvar: 0.0, backoff: 0 }
    }
}

impl Rto {
    /// The timeout now, in ms.
    pub fn rto_ms(&self) -> u64 {
        let base = match self.srtt {
            None => RTO_INITIAL_MS,
            Some(s) => (s + 4.0 * self.rttvar).max(RTO_FLOOR_MS),
        };
        (base * f64::from(1u32 << self.backoff.min(10))).min(RTO_MAX_MS) as u64
    }

    /// A call answered on its FIRST send took `r` ms: a sample, and the
    /// back-off ends (RFC 6298 §2.2–2.3, §5.7).
    pub fn sample(&mut self, r: u64) {
        let r = r as f64;
        match self.srtt {
            None => {
                self.srtt = Some(r);
                self.rttvar = r / 2.0;
            }
            Some(s) => {
                self.rttvar = 0.75 * self.rttvar + 0.25 * (s - r).abs();
                self.srtt = Some(0.875 * s + 0.125 * r);
            }
        }
        self.backoff = 0;
    }

    /// A call answered on its FIRST send, but QUEUED behind others (sdk#390): its time is its wait in the node's
    /// queue too, so it is no sample -- but the path answered a call sent once, and the back-off ends (§5.7).
    pub fn answered_queued(&mut self) {
        self.backoff = 0;
    }

    /// Something timed out this tick: back the timer off (§5.5).
    pub fn timed_out(&mut self) {
        self.backoff = (self.backoff + 1).min(10);
    }

    pub fn srtt_ms(&self) -> Option<f64> {
        self.srtt
    }
}

#[derive(Debug, Clone)]
pub struct Window {
    w: f64,
    ssthresh: Option<f64>,
    /// When the current loss episode began (the last halving): a GET sent
    /// before it that is lost is part of that episode (sdk#345).
    loss_since: u64,
}

impl Default for Window {
    fn default() -> Window {
        Window { w: WINDOW_INITIAL, ssthresh: None, loss_since: 0 }
    }
}

impl Window {
    /// How many GETs may be in flight now.
    pub fn size(&self) -> usize {
        self.w.max(WINDOW_FLOOR).floor() as usize
    }

    /// A GET was answered.
    pub fn opened(&mut self) {
        let w = self.w.max(WINDOW_FLOOR);
        self.w = if self.ssthresh.is_some_and(|t| w >= t) { w + 1.0 / w } else { w + 1.0 };
    }

    /// A GET sent at `sent_at` was lost at `now`, on its `attempt`-th send.
    /// The window halves once per LOSS EPISODE (sdk#345, QUIC's rule): only a
    /// GET's FIRST timeout, and only of a GET sent after the last halving. A
    /// silent block timing out again backs off its own RTO and nothing else;
    /// halving on every re-timeout shrank a window 9 -> 1 behind ONE block.
    pub fn lost(&mut self, sent_at: u64, attempt: u32, now: u64) {
        if attempt == 1 && sent_at >= self.loss_since {
            self.halved();
            self.loss_since = now;
        }
    }

    /// Halve (never below the floor), and remember where.
    pub fn halved(&mut self) {
        let half = (self.w.max(WINDOW_FLOOR) / 2.0).max(WINDOW_FLOOR);
        self.w = half;
        self.ssthresh = Some(half);
    }
}
