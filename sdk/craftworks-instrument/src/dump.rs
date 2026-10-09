//! The failure dump: the consumer this vocabulary was designed for.
//!
//! The rule this crate exists to serve is "a test that fails must already have
//! recorded WHY" — running bare, failing, then re-running with one-off debug
//! added was the most repeated waste in the project, and the one-off probes
//! were thrown away each time.
//!
//! So the dump has ONE requirement, and the vocabulary was chosen to meet it:
//! **a human reading the last N events must be able to tell a wedge from
//! work.** That is why `Edge` pairs by id — the wedge that voided four harness
//! runs was "thousands of requests sent, none answered", and nothing but
//! OUTSTANDING says that.

use crate::{
    recorder::{Record, Recorder},
    vocab::Key,
    Event, STREAM_VERSION,
};

/// Print the recording when the thread is panicking, and stay silent otherwise.
///
/// Held by the probed fixture, so using the fixture gets the dump for free —
/// enforcement that relies on people wanting the better tool, rather than on
/// remembering to ask for it.
pub struct DumpOnPanic<'a> {
    recorder: &'a Recorder,
    what: &'static str,
    last: usize,
}

impl<'a> DumpOnPanic<'a> {
    pub fn new(recorder: &'a Recorder, what: &'static str) -> Self {
        DumpOnPanic {
            recorder,
            what,
            last: 40,
        }
    }

    /// How many trailing events to print. The default is enough to see a shape
    /// and short enough to read.
    pub fn last(mut self, n: usize) -> Self {
        self.last = n;
        self
    }

    /// The dump as text, so a caller can print it somewhere else — and so the
    /// test that proves the dump works does not have to capture stderr.
    pub fn render(&self) -> String {
        render(&self.recorder.recording(), self.what, self.last)
    }
}

impl Drop for DumpOnPanic<'_> {
    fn drop(&mut self) {
        // Only on the way out of a panic. A passing test prints nothing.
        if std::thread::panicking() {
            eprintln!("{}", self.render());
        }
    }
}

/// The text of a dump.
///
/// Order matters: the counts that distinguish a wedge from work come FIRST,
/// because a reader who sees `outstanding: 1,847` has the answer before
/// reading a single event.
pub fn render<R: Record + ?Sized>(rec: &R, what: &'static str, last: usize) -> String {
    use core::fmt::Write as _;
    let mut s = String::with_capacity(1024);
    let events = rec.events();
    let outstanding = rec.outstanding();
    let unfinished = rec.unfinished();

    let _ = writeln!(
        s,
        "── instrument dump: {what} (stream v{STREAM_VERSION}) ──"
    );
    let _ = writeln!(
        s,
        "   offered {} · kept {} · DROPPED {}",
        rec.offered(),
        events.len(),
        rec.dropped()
    );
    // The line that tells a wedge from work.
    let _ = writeln!(
        s,
        "   OUTSTANDING {} · unfinished spans {}",
        outstanding.len(),
        unfinished.len()
    );
    if !outstanding.is_empty() {
        let shown: Vec<String> = outstanding.iter().take(8).map(|l| l.to_string()).collect();
        let _ = writeln!(
            s,
            "   requests with no response: {}{}",
            shown.join(", "),
            if outstanding.len() > shown.len() {
                format!(" … and {} more", outstanding.len() - shown.len())
            } else {
                String::new()
            }
        );
    }
    for (site, op) in unfinished.iter().take(8) {
        let _ = writeln!(s, "   entered and never exited: {}", op_name(*site, *op));
    }
    for (o, n) in rec.outcomes() {
        let _ = writeln!(s, "   outcome {o:?}: {n}");
    }
    for key in [
        Key::Reads,
        Key::Misses,
        Key::Sent,
        Key::Received,
        Key::Attempts,
        Key::Owed,
        Key::Ambiguous,
        Key::Effects,
        Key::Ops,
        Key::Awaiting,
        Key::ReadBack,
        Key::Stranded,
        Key::DroppedMsgs,
        Key::Said,
    ] {
        let t = rec.total(key);
        if t > 0 {
            let _ = writeln!(s, "   {key:?}: {t}");
        }
    }
    if rec.dropped() > 0 {
        let _ = writeln!(
            s,
            "   NOTE: the ring dropped {} event(s); what follows is the TAIL, and the \
             beginning of this recording is gone.",
            rec.dropped()
        );
    }
    let from = events.len().saturating_sub(last);
    let _ = writeln!(
        s,
        "   last {} of {} events:",
        events.len() - from,
        events.len()
    );
    for (i, e) in events[from..].iter().enumerate() {
        let _ = writeln!(s, "   {:>5}  {}", from + i, one(e));
    }
    s
}

/// How a dump names an operation: `site#n` for a plain id (a span's, or a
/// `req#n`'s -- the form it has always had), and `site kind#n` for a label of
/// another kind, so `fetch#1` never reads as `req#1`.
fn op_name(site: crate::Site, op: crate::OpId) -> String {
    match crate::Label::of_op(op) {
        Some(l) if l.kind() != crate::Kind::Request => format!("{} {l}", site.name()),
        _ => format!("{}#{}", site.name(), op.0),
    }
}

fn one(e: &Event) -> String {
    match e {
        Event::Enter { site, op } => format!("enter  {}", op_name(*site, *op)),
        Event::Exit { site, op, outcome } => {
            format!("exit   {} {outcome:?}", op_name(*site, *op))
        }
        Event::Counter { site, op, entry } => {
            // The operation is shown only when the counter belongs to one; a
            // connection-wide total printing `#4294967295` would be noise that
            // a reader has to learn to ignore.
            if *op == crate::OpId::NONE {
                format!("count  {} {:?}={}", site.name(), entry.key, entry.value)
            } else {
                format!(
                    "count  {} {:?}={}",
                    op_name(*site, *op),
                    entry.key,
                    entry.value
                )
            }
        }
        Event::Edge { site, dir, id } => format!("edge   {} {dir:?} {id}", site.name()),
    }
}
