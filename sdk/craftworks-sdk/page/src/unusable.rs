//! THE LINES THE SDK SAYS TO THE APP (craftworks-sdk#482; the architect's ruling): what the page, page-io and the
//! session could not use or do, as text for a person. ONE shape for all three:
//! * every line is RECORDED where it is said -- one `Key::Said` counter at its SITE, no text (the site names the line;
//!   the text can name keys or a node's refusal, so it never enters a recording);
//! * the text is kept here for the app, CAPPED at [`KEEP`]: past it the OLDEST line is dropped and COUNTED (rule 9:
//!   drop and count, never an unbounded queue) -- so it stays bounded when nothing ever reads it;
//! * it has ONE reader, which DRAINS it ([`Unusable::take`]): the session's `take_unusable`, through page-io's and the
//!   page's, to JS `unusable()`.

use std::collections::VecDeque;

/// The site a line is said at (its one recorded code): `instrument::Site`, for crates that say lines through the page.
pub use instrument::Site;

/// How many lines are kept for the app between two reads.
pub const KEEP: usize = 256;

/// The lines said and not yet read, newest last; and how many were dropped since the last read.
#[derive(Debug, Default)]
pub struct Unusable {
    lines: VecDeque<String>,
    dropped: u64,
}

impl Unusable {
    /// Keep `line` for the app (the caller has recorded it): past [`KEEP`] the oldest goes, counted.
    pub fn push(&mut self, line: String) {
        if self.lines.len() == KEEP {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.lines.push_back(line);
    }

    /// THE ONE READ: every line kept, oldest first -- led by "N earlier line(s) dropped" when any were -- and none
    /// kept after it.
    pub fn take(&mut self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.lines.len() + 1);
        if self.dropped > 0 {
            out.push(format!("{} earlier line(s) dropped: only the newest {KEEP} are kept between reads", self.dropped));
        }
        out.extend(self.lines.drain(..));
        self.dropped = 0;
        out
    }

    /// A look without taking, for TESTS only (the app's one reader is [`Unusable::take`]).
    #[doc(hidden)]
    pub fn peek(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }

    /// Lines kept now, and lines dropped since the last read.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Bounded with NO reader** (the architect): 10,000 lines said and never read leave KEEP kept and the rest
    /// counted; the one read says so first, and leaves nothing. CONTROL: fewer than KEEP lines are all kept, and no
    /// "dropped" line leads.
    #[test]
    fn ten_thousand_lines_with_no_reader_stay_bounded_and_the_drop_is_counted() {
        let mut u = Unusable::default();
        for i in 0..10_000 {
            u.push(format!("line {i}"));
        }
        assert_eq!((u.len(), u.dropped()), (KEEP, 10_000 - KEEP as u64));
        let read = u.take();
        assert_eq!(read.len(), KEEP + 1);
        assert!(read[0].starts_with(&format!("{} earlier line(s) dropped", 10_000 - KEEP)), "{:?}", read[0]);
        assert_eq!(read[1], format!("line {}", 10_000 - KEEP), "not the newest KEEP, oldest first");
        assert_eq!((u.len(), u.dropped(), u.take().len()), (0, 0, 0), "the read did not drain");

        let mut few = Unusable::default();
        few.push("one".into());
        assert_eq!(few.take(), vec!["one".to_string()]);
    }
}
