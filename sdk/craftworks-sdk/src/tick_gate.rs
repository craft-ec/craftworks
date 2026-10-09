//! AT MOST ONE UNANSWERED TICK PER SESSION, ALWAYS (craftworks-sdk#174).
//!
//! Every frame a page sends is one delegate call, and the node runs a
//! delegate's calls one at a time behind a queue of 8 (the 9th is refused;
//! F50). A delegate parked on the network for 30 s used to collect a tick a
//! second from every tab — thirty stale times, each costing a queue slot, and
//! the queue filled (sdk#173). A tick says only "it is now T"; a second one
//! sent before the first is answered says nothing the first will not, and
//! the first call AFTER the wait should carry the time as it is then.
//!
//! So a tick goes out only when this session has none unanswered. "Answered"
//! is read off the per-call report: the delegate emits exactly one
//! `Reply::Call { saw: Client, .. }` for every client frame it runs, on the
//! connection that sent it, in the order they ran. Counting frames sent
//! against those reports says whether the tick's frame has run.
//!
//! A frame the node refuses (a full queue) or a connection that drops
//! mid-send is never answered, so `answered` falls behind `sent`. What undoes
//! that is the forgetting: a tick unanswered for [`FORGET_MS`] is dropped AND
//! the lag behind it is written off (counted answered), and the next one
//! goes. Both halves are needed, and both are measured
//! (`testkit/tests/one_tick_at_a_time.rs`): without the forgetting one lost
//! reply stops the delegate's clock for good; without the write-off it
//! slows it to one tick per [`FORGET_MS`] for good, 6 ticks in 60 s instead
//! of 51, because every later tick waits on an answer that already went to
//! the frame before it.
//!
//! The write-off's price, stated: if the forgotten frame was only SLOW and is
//! answered later, that answer is counted twice, and the gate opens one
//! frame early once. So "at most one unanswered" holds except after a silence
//! of [`FORGET_MS`], where it is at most two.
//!
//! The same shape bounds `AskWrite` (the continuation of a parked write): at
//! most one unanswered at a time, forgotten on the same terms.

/// How long an unanswered tick (or ask) holds the gate shut.
///
/// Sized from how long a queued frame can LEGITIMATELY wait, which the node
/// states: a parked delegate works for at most `PARK_WORK_BUDGET` = 75 s and
/// is dropped at `PARK_TTL` = 90 s (F50, freenet-core's delegate park, read
/// at the pinned version). An answer later than 100 s is one the node never
/// ran. Shorter would forget ticks that are merely queued: at 10 s a 30 s
/// park held 4 ticks per tab (measured), and under the 75 s budget every
/// long park would double each tab's ticks.
///
/// The cost, stated: one lost tick reply freezes this session's share of the
/// engine's clock for up to 100 s. (v5 replaces this timer with a fact — a
/// per-session frame sequence echoed in every `Reply::Call`; WRITE-PATH.md.)
pub const FORGET_MS: u64 = 100_000;

/// Frames this session sent against the per-call reports that answered them.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Frames {
    sent: u64,
    answered: u64,
}

impl Frames {
    /// A client frame was queued to go out.
    pub fn sent(&mut self) {
        self.sent += 1;
    }

    /// A `Reply::Call { saw: Client, .. }` arrived: one of our frames ran.
    pub fn answered(&mut self) {
        self.answered += 1;
    }

    /// The position of the frame most recently sent.
    fn last(&self) -> u64 {
        self.sent
    }

    /// Has the frame at `pos` run?
    fn has_run(&self, pos: u64) -> bool {
        self.answered >= pos
    }
}

/// One outstanding frame of one kind: sent at `pos`, at `at_ms`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OneAtATime {
    out: Option<(u64, u64)>,
    /// Refused because one was unanswered.
    pub refused: u64,
    /// Given up on after [`FORGET_MS`] with no answer.
    pub forgotten: u64,
}

impl OneAtATime {
    /// May one more go now?
    ///
    /// Yes when none is outstanding, when the outstanding one has run, or when
    /// it has waited [`FORGET_MS`] — then it is forgotten. A wall clock that
    /// stepped BACK re-dates the outstanding one to now rather than holding
    /// the gate shut until the clock catches up.
    pub fn may(&mut self, now_ms: u64, frames: &mut Frames) -> bool {
        let Some((pos, at)) = self.out else {
            return true;
        };
        if frames.has_run(pos) {
            self.out = None;
            return true;
        }
        if now_ms < at {
            self.out = Some((pos, now_ms));
        } else if now_ms - at >= FORGET_MS {
            self.out = None;
            // Written off: this frame and any unanswered before it.
            frames.answered = frames.answered.max(pos);
            self.forgotten += 1;
            return true;
        }
        self.refused += 1;
        false
    }

    /// The frame just sent (the last one counted in `frames`) is this kind's.
    pub fn went(&mut self, now_ms: u64, frames: &Frames) {
        self.out = Some((frames.last(), now_ms));
    }

    /// Is one outstanding? Asked of the frames, so an answer that arrived
    /// since the last `may` counts.
    pub fn outstanding(&self, frames: &Frames) -> bool {
        self.out.is_some_and(|(pos, _)| !frames.has_run(pos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Send one frame of this kind, as the client does: count it, then mark it.
    fn send(g: &mut OneAtATime, f: &mut Frames, now: u64) -> bool {
        if !g.may(now, &mut *f) {
            return false;
        }
        f.sent();
        g.went(now, f);
        true
    }

    #[test]
    fn a_second_tick_waits_for_the_first_to_be_answered() {
        let (mut g, mut f) = (OneAtATime::default(), Frames::default());
        assert!(send(&mut g, &mut f, 0));
        for t in 1..10 {
            assert!(!send(&mut g, &mut f, t * 1000), "a second tick at {t} s");
        }
        assert_eq!(g.refused, 9);
        f.answered();
        assert!(send(&mut g, &mut f, 9_500), "answered, and still refused");
    }

    /// Other frames interleave: the tick is answered by ITS report, not the
    /// first one to arrive.
    #[test]
    fn an_earlier_frame_s_answer_does_not_answer_the_tick() {
        let (mut g, mut f) = (OneAtATime::default(), Frames::default());
        f.sent(); // a write, before the tick
        assert!(send(&mut g, &mut f, 0));
        f.answered(); // the write's report
        assert!(
            g.outstanding(&f),
            "the write's report was read as the tick's"
        );
        assert!(!send(&mut g, &mut f, 1000));
        f.answered(); // the tick's
        assert!(!g.outstanding(&f));
        assert!(send(&mut g, &mut f, 2000));
    }

    /// THE FORGETTING. One lost reply must not stop the clock for ever: the
    /// mutant without it is red here.
    #[test]
    fn an_unanswered_tick_is_forgotten_after_forget_ms() {
        let (mut g, mut f) = (OneAtATime::default(), Frames::default());
        assert!(send(&mut g, &mut f, 0));
        assert!(!send(&mut g, &mut f, FORGET_MS - 1));
        assert!(
            send(&mut g, &mut f, FORGET_MS),
            "a lost reply stopped the clock"
        );
        assert_eq!(g.forgotten, 1);
        // THE WRITE-OFF: the lost frame is no longer waited on, so the NEXT
        // tick's own answer opens the gate. Without it the count stays one
        // behind and every later tick waits the full FORGET_MS.
        f.answered();
        assert!(
            !g.outstanding(&f),
            "the lost frame's lag was not written off"
        );
        assert!(send(&mut g, &mut f, FORGET_MS + 1000));
    }

    /// A frame the node REFUSED is answered by the refusal. Two refused
    /// frames used to shut the gate until FORGET_MS; counted, the next tick
    /// goes at once (the architect's sdk#196 gate-lag probe).
    #[test]
    fn a_refused_frame_is_answered_by_its_refusal() {
        for refused in [0u64, 1, 2, 8] {
            let (mut g, mut f) = (OneAtATime::default(), Frames::default());
            for _ in 0..refused {
                f.sent(); // a write burst the node refused ...
                f.answered(); // ... and said so
            }
            let mut sent = 0;
            for s in 0..=120u64 {
                if send(&mut g, &mut f, s * 1000) {
                    sent += 1;
                    f.answered();
                }
            }
            assert_eq!(sent, 121, "{refused} refused frame(s) held the gate shut");
        }
    }

    /// CONTROL: the same refusals NOT counted as answers shut the gate until
    /// FORGET_MS -- so the test above measures the counting, not a gate that
    /// cannot shut.
    #[test]
    fn control_uncounted_refusals_shut_the_gate() {
        let (mut g, mut f) = (OneAtATime::default(), Frames::default());
        f.sent();
        f.sent();
        let mut sent = 0;
        for s in 0..=120u64 {
            if send(&mut g, &mut f, s * 1000) {
                sent += 1;
                f.answered();
            }
        }
        assert!(
            sent < 30,
            "two unanswered frames did not shut the gate: {sent} ticks"
        );
    }

    #[test]
    fn a_clock_stepping_back_re_dates_rather_than_shuts_the_gate() {
        let (mut g, mut f) = (OneAtATime::default(), Frames::default());
        assert!(send(&mut g, &mut f, 1_000_000));
        assert!(!send(&mut g, &mut f, 5_000)); // an hour back
        assert!(send(&mut g, &mut f, 5_000 + FORGET_MS));
    }
}
