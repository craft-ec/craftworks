//! The cold-read probe's verdict, as a pure function of what was measured.
//!
//! Separate from the live run so every arm is exercised on demand: a stall that
//! heals (F52) is rare live, and a verdict only a rare event can reach is one
//! nobody has seen work.

/// How long one cold read is waited for before it is RED.
///
/// 100 s, not 60 (F52, sdk#173): a streamed GET response lost between two
/// nodes is healed by the node itself -- its stream wait times out 60 s after
/// the lost response was SENT, a second GET is served, and the page follows
/// (measured: stream failed at +63.7 s, page at +68.2 s). A 60 s window ended
/// every such run at the one moment the node could not yet have spoken.
pub const COLD_READ_MS: u128 = 100_000;

/// A cold read answered later than this STALLED and recovered. An unstalled
/// cold read was answered in 1-9 s in every one of 11 long-window runs.
pub const STALLED_AFTER_MS: u128 = 30_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Every check passed and every cold read came back within `STALLED_AFTER_MS`.
    Green,
    /// Every check passed, but a cold read waited past `STALLED_AFTER_MS`: the
    /// slowest one, in whole seconds, and which request it was.
    StalledRecovered { secs: u64, req: u64 },
    /// Why, one line per failed check.
    Red(Vec<String>),
}

/// `answers`: each cold-read request sent, with how many ms after ITS send its
/// page came, or `None` if it never did. `other`: the red lines of every check
/// that is not about time (rows read, stranded effects, TESTs 2 and 3).
pub fn cold_read(answers: &[(u64, Option<u128>)], other: Vec<String>) -> Verdict {
    let mut red = Vec::new();
    if answers.is_empty() {
        red.push("no cold read was sent".to_string());
    }
    for (req, a) in answers {
        match a {
            Some(ms) if *ms <= COLD_READ_MS => {}
            Some(ms) => red.push(format!("req {req} NOT ANSWERED within {} s (its page came at {ms} ms)", COLD_READ_MS / 1000)),
            None => red.push(format!("req {req} NOT ANSWERED within {} s", COLD_READ_MS / 1000)),
        }
    }
    red.extend(other);
    if !red.is_empty() {
        return Verdict::Red(red);
    }
    let slowest = answers.iter().filter_map(|(r, a)| a.map(|ms| (ms, *r))).max();
    match slowest {
        Some((ms, req)) if ms > STALLED_AFTER_MS => Verdict::StalledRecovered { secs: (ms / 1000) as u64, req },
        _ => Verdict::Green,
    }
}

/// Where one write stands, from every reply seen so far: `None` while more is
/// coming, `Some(Ok(()))` once it is `Published`, `Some(Err(why))` once a
/// TERMINAL state came without it.
///
/// Terminal is [`protocol::WriteState::terminal`], not a list kept here: a
/// hand list missed `Unread` (sdk#283), and the live run then waited 120 s on
/// an answer that was already final and read as a stall.
pub fn write_outcome(id: u64, replies: &[protocol::Reply]) -> Option<Result<(), String>> {
    use protocol::{Reply, WriteState};
    fn peel(r: &Reply) -> &Reply {
        match r {
            Reply::Acked { body, .. } => body,
            r => r,
        }
    }
    let mut unread = None;
    let mut last = None;
    for r in replies.iter().map(peel) {
        match r {
            Reply::SessionWriteState { write_id, state, .. } | Reply::WriteState { write_id, state } if *write_id == id => {
                if *state == WriteState::Published {
                    return Some(Ok(()));
                }
                last = Some(*state);
            }
            Reply::Unread { write_id, key, .. } if *write_id == id => unread = Some(String::from_utf8_lossy(key).into_owned()),
            _ => {}
        }
    }
    let state = last.filter(|s| s.terminal())?;
    Some(Err(match unread {
        Some(key) => format!("write {id} ended {state:?} without Published: it changes {key:?}, which it did not read"),
        None => format!("write {id} ended {state:?} without Published"),
    }))
}

/// THE DETECT CHECK for sdk#433's pinned validation refusal (WORKAROUNDS: re-run at every freenet version bump):
/// `version` is the node's `freenet --version` line, `said` the cause of its answer to a Block PUT its contract must
/// refuse as invalid. Green only when the node is the version the text was read on AND said one of the pinned texts;
/// a new version says so by name (confirm the text, then move `VALIDATION_REFUSED_READ_ON`), and drifted words fail
/// -- a release that changed them would otherwise turn every rejection into an endless re-send.
pub fn refusal_text(version: &str, said: Option<&str>) -> Result<String, String> {
    let read_on = wire::VALIDATION_REFUSED_READ_ON;
    if !version.split_whitespace().any(|w| read_on.contains(&w)) {
        return Err(format!("the node is {version:?}, and the pinned validation texts were read on {read_on:?}: confirm the text on this version, then add it to VALIDATION_REFUSED_READ_ON"));
    }
    match said {
        Some(s) if wire::is_validation_refusal(s) => Ok(format!("the node's validation refusal says {s:?}, pinned")),
        Some(s) => Err(format!("the node refused the invalid block saying {s:?}, which is NOT a pinned validation text {:?}: every rejection would be re-sent for ever", wire::VALIDATION_REFUSED)),
        None => Err("the node did not refuse a Block PUT its contract must reject (no keyed PutFailed)".into()),
    }
}

#[cfg(test)]
mod refusal_text {
    use super::refusal_text;

    #[test]
    fn green_only_on_the_version_read_and_a_pinned_text() {
        assert!(refusal_text("Freenet version: 0.2.136 (7fa2c6605b99)", Some("not valid")).is_ok());
        assert!(refusal_text("Freenet version: 0.2.136 (7fa2c6605b99)", Some("invalid put")).is_ok());
        assert!(refusal_text("Freenet version: 0.2.138 (5fb1aa93e15c)", Some("invalid put")).is_ok());
        // DRIFT: the words changed.
        assert!(refusal_text("Freenet version: 0.2.136 (7fa2c6605b99)", Some("contract state not valid")).is_err());
        // A NEW VERSION: named, even with the same words.
        assert!(refusal_text("Freenet version: 0.2.139 (abc)", Some("not valid")).is_err());
        assert!(refusal_text("Freenet version: 0.2.136 (7fa2c6605b99)", None).is_err());
    }
}
