//! What it SHOULD have cost, what it DID cost, and the ratio between them.
//!
//! # Why a ratio and not a number
//!
//! A 1 KiB block was measured costing 240–395 KB to fetch (F46). Establishing
//! that took six rejected instruments and a night, and the reason it took that
//! long is that nothing said what it should have cost: every candidate number
//! looked plausible on its own, so each had to be argued down separately.
//!
//! Against an expectation the same run reads as **≈34×** on the first attempt.
//! A ratio is falsifiable in a way an absolute is not — 340 KB is a number, but
//! 34× the payload is a question with an answer.
//!
//! # Where the expectation comes from
//!
//! Not from a second measurement. From the DATA: node boundaries are
//! deterministic (ARCHITECTURE §5), so what an operation touches is a pure
//! function of the tree's shape, computed without running anything. That is the
//! whole point of the derive-or-instrument split — the expectation costs
//! nothing and cannot drift, because it is not a recording of a previous run.

use crate::vocab::Key;
use crate::Record;

/// What an operation was expected to cost, derived rather than measured.
///
/// Every field is a pure function of the data. Nothing here is observed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Expected {
    /// Bytes the operation's payload accounts for.
    pub bytes: u64,
    /// Blocks the operation must touch.
    pub blocks: u64,
}

/// What actually happened, read from a recording.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Actual {
    pub bytes_out: u64,
    pub bytes_in: u64,
    pub ops: u64,
}

/// The comparison, with the ratio that makes it readable at a glance.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Gap {
    pub expected: Expected,
    pub actual: Actual,
}

impl Gap {
    /// Read the actual side out of a recording. The expected side is supplied
    /// by the caller, because only the caller knows which data the operation
    /// was over.
    pub fn of(rec: &impl Record, expected: Expected) -> Gap {
        Gap {
            expected,
            actual: Actual {
                bytes_out: rec.total(Key::BytesOut),
                bytes_in: rec.total(Key::BytesIn),
                ops: rec.total(Key::Ops),
            },
        }
    }

    /// Total bytes moved, both directions.
    pub fn bytes_moved(&self) -> u64 {
        self.actual.bytes_out + self.actual.bytes_in
    }

    /// How many times the payload the operation moved.
    ///
    /// `None` when nothing was expected — a ratio against zero is not a large
    /// number, it is an undefined one, and reporting it as "infinite overhead"
    /// is how a measurement about nothing gets quoted.
    pub fn ratio(&self) -> Option<f64> {
        if self.expected.bytes == 0 {
            return None;
        }
        Some(self.bytes_moved() as f64 / self.expected.bytes as f64)
    }

    /// One line, for a report or a failing test.
    pub fn line(&self) -> String {
        match self.ratio() {
            Some(r) => format!(
                "expected {} B in {} block(s); moved {} B in {} op(s) — {r:.1}x",
                self.expected.bytes,
                self.expected.blocks,
                self.bytes_moved(),
                self.actual.ops
            ),
            None => format!(
                "expected nothing; moved {} B in {} op(s) — no ratio, because a \
                 ratio against zero is undefined rather than large",
                self.bytes_moved(),
                self.actual.ops
            ),
        }
    }
}
