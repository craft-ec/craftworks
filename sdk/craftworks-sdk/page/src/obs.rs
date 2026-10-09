//! THE OBSERVATION TREE's page side (craftworks-docs OBSERVABILITY §1-§2, sdk#399): the per-(app, minute) DETAIL
//! record's key, and the page's recording split so each window's record holds THAT window's events only.
//!
//! The record's value is the ONE publish filter's output (`instrument::publish`), nothing the page adds (rule 3). The
//! page never serializes its recording any other way: a source scan (page/tests/obs_one_way_out.rs) holds it.

use instrument::{Probe, Recorder, Recording};

/// The detail record's key in the observation tree: `w/ ‖ the app's SITE contract id ‖ the window's minute (u64 BE)`.
///
/// The app is named by its site contract id (the architect on #399): the stable site key (#117), the one owner of
/// "which app", fixed-width, and what support already needs to read the app's container at that version -- never the
/// app's name, a second and mutable form. Big-endian minutes: one app's windows sort by time, and a range scan over
/// `w/ ‖ site` reads that app alone.
pub fn detail_key(site: &[u8; 32], minute: u64) -> Vec<u8> {
    let mut k = Vec::with_capacity(2 + 32 + 8);
    k.extend_from_slice(b"w/");
    k.extend_from_slice(site);
    k.extend_from_slice(&minute.to_be_bytes());
    k
}

/// The page's recording, WHOLE for the person's own dump, and beside it the CURRENT WINDOW's (sdk#399): every event
/// goes to both. A window's record is published from the window recorder alone, which is then replaced by a fresh one,
/// so the next record holds none of this one's events -- the "per window" the filter's `dropped` and ordinals rely on
/// (the architect's carry-over from instrument#18: an Exit carries no offset, so only the drain keeps an old one out).
/// The ring refuses events once full (it never overwrites), so a per-window recorder is also what keeps a long-lived
/// page publishing at all.
pub(crate) struct Rec {
    local: Recorder,
    window: std::cell::RefCell<Recorder>,
    window_capacity: usize,
}

impl Rec {
    pub(crate) fn new(capacity: usize) -> Rec {
        Rec { local: Recorder::with_capacity(capacity), window: std::cell::RefCell::new(Recorder::with_capacity(capacity)), window_capacity: capacity }
    }

    /// The whole recording (a separate read handle; the page itself only writes).
    pub(crate) fn recording(&self) -> Recording<'_> {
        self.local.recording()
    }

    /// The window now closing, taken whole; a fresh recorder takes its place.
    pub(crate) fn take_window(&self) -> Recorder {
        self.window.replace(Recorder::with_capacity(self.window_capacity))
    }
}

impl Probe for Rec {
    fn event(&self, e: instrument::Event) {
        self.local.event(e);
        // A probe never panics: a window being taken at this instant (it can't be, single-threaded) loses the event.
        if let Ok(w) = self.window.try_borrow() {
            w.event(e);
        }
    }
}
