//! A recorded conversation, kept so it can be replayed for ever.
//!
//! §19 says every published version keeps working. That is a claim about the
//! future, and the only way to check it is to have something from the PAST
//! to check against — so a v1 session is recorded, byte for byte, and
//! committed. When v2 exists, this file is what proves v1 still does.
//!
//! What is recorded is BYTES ON THE WIRE, not the messages they decode to.
//! Recording the decoded form would mean the fixture is written by the same
//! encoder it is meant to check, and a change to the encoding would rewrite
//! the fixture and the test together — green, and proving nothing.

use crate::{decode_reply, decode_request, Incoming, Reply};

/// One thing that crossed the wire, and which way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// The client said this.
    Sent(Vec<u8>),
    /// The engine said this.
    Received(Vec<u8>),
}

/// A recorded session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    pub lines: Vec<Line>,
}

const MAGIC: &[u8; 8] = b"CWSESS01";

impl Session {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::from(*MAGIC);
        out.extend_from_slice(&(self.lines.len() as u32).to_le_bytes());
        for l in &self.lines {
            let (tag, b) = match l {
                Line::Sent(b) => (0u8, b),
                Line::Received(b) => (1u8, b),
            };
            out.push(tag);
            out.extend_from_slice(&(b.len() as u32).to_le_bytes());
            out.extend_from_slice(b);
        }
        out
    }

    /// Read a session back, refusing anything that is not exactly one.
    ///
    /// A truncated recording must be an error and never a short session: a
    /// fixture that silently loses its tail is a replay that silently checks
    /// less, and it would read as a pass.
    pub fn decode(b: &[u8]) -> Result<Session, String> {
        let mut at = 0usize;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let s = b
                .get(at..at + n)
                .ok_or_else(|| format!("session truncated at {at}: wanted {n} B"))?;
            at += n;
            Ok(s)
        };
        if take(8)? != MAGIC {
            return Err("not a recorded session".into());
        }
        let n = {
            let s = take(4)?;
            u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize
        };
        let mut lines = Vec::with_capacity(n);
        for _ in 0..n {
            let tag = take(1)?[0];
            let len = {
                let s = take(4)?;
                u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize
            };
            let body = take(len)?.to_vec();
            lines.push(match tag {
                0 => Line::Sent(body),
                1 => Line::Received(body),
                other => return Err(format!("unknown direction {other}")),
            });
        }
        if at != b.len() {
            return Err(format!("session has {} trailing byte(s)", b.len() - at));
        }
        Ok(Session { lines })
    }
}

/// What replaying a recorded session found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Replay {
    pub requests: usize,
    pub replies: usize,
    /// Lines this build could not read at all. Must be zero for a session
    /// recorded at a version it serves.
    pub unreadable: Vec<usize>,
    /// Requests whose version this build does not serve.
    pub unsupported: Vec<u16>,
}

/// Read every line of a recorded session with THIS build's decoders.
///
/// Not "does it still run" — does this build still UNDERSTAND what was said.
/// A replay that merely avoided panicking would pass for a decoder that
/// returned an error on every line.
pub fn replay(s: &Session) -> Replay {
    let mut out = Replay::default();
    for (i, line) in s.lines.iter().enumerate() {
        match line {
            Line::Sent(b) => match decode_request(b) {
                Incoming::Ok(_) => out.requests += 1,
                Incoming::Unsupported(v) => out.unsupported.push(v),
                Incoming::Dropped(_) => out.unreadable.push(i),
            },
            Line::Received(b) => match decode_reply(b) {
                Ok(Reply::Dropped { .. }) | Ok(_) => out.replies += 1,
                Err(_) => out.unreadable.push(i),
            },
        }
    }
    out
}
