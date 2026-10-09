//! An operation's call tree, assembled and TIMED on the client.
//!
//! The engine has no clock. It is sans-IO by construction, which is what
//! makes it testable, and a duration it invented would be worse than none —
//! so every step arrives without one and the client stamps it as it lands.
//! That measures the thing an app actually cares about anyway: how long it
//! waited, not how long the engine thought it took.
//!
//! The steps arrive INCREMENTALLY, as they happen, because a delegate gets a
//! fresh linear memory on every call (F32) and a tree accumulated until an
//! operation finished would not survive the operation it describes. Nothing
//! can be asked for afterwards; a client turns emission on in advance.
//!
//! # What a trace costs, and why it is bounded here too
//!
//! Steps ride the same connection as the data. The engine caps how many one
//! call emits; this caps how many operations are kept and how deep each goes,
//! because a long-lived tab tracing everything is a memory leak with good
//! intentions.

use protocol::{Step, TraceOf};

/// One step, with the moment it reached the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamped {
    pub depth: u8,
    pub what: Step,
    pub n: u64,
    /// Milliseconds since the operation's first step, as the CLIENT saw them.
    pub at_ms: u64,
}

/// What happened during one operation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trace {
    pub steps: Vec<Stamped>,
    /// Client-observed total: the last step's stamp.
    pub total_ms: u64,
    /// True if steps were dropped because the operation exceeded the cap.
    ///
    /// Stated rather than left to be inferred from a short tree. A truncated
    /// trace read as a complete one is a diagnosis of the wrong thing.
    pub truncated: bool,
}

/// Traces for the operations a client has seen BEGIN, newest kept.
pub struct Traces {
    by_op: Vec<(TraceOf, Trace, u64)>,
    max_ops: usize,
    max_steps: usize,
}

impl Default for Traces {
    fn default() -> Self {
        Traces::new(32, 256)
    }
}

impl Traces {
    pub fn new(max_ops: usize, max_steps: usize) -> Traces {
        Traces {
            by_op: Vec::new(),
            max_ops,
            max_steps,
        }
    }

    /// Record a step, at a client-supplied moment.
    ///
    /// `now_ms` is passed in rather than read: this crate compiles to wasm
    /// and to a host binary, and a clock reached for inside would be a second
    /// source of time disagreeing with the app's.
    pub fn record(&mut self, of: TraceOf, depth: u8, what: Step, n: u64, now_ms: u64) {
        if let Some(i) = self.by_op.iter().position(|(o, _, _)| *o == of) {
            let (_, t, began) = &mut self.by_op[i];
            if t.steps.len() >= self.max_steps {
                t.truncated = true;
                return;
            }
            let at_ms = now_ms.saturating_sub(*began);
            t.steps.push(Stamped {
                depth,
                what,
                n,
                at_ms,
            });
            t.total_ms = at_ms;
            return;
        }
        // A new operation is opened ONLY by its first step.
        //
        // Steps for an operation this client never saw begin are dropped, and
        // that is the point: they are real, but a tree assembled from the
        // middle of an operation has no `Began`, no first hop and no zero
        // point for its stamps — and nothing in it says so. A reader would
        // take it for the whole thing and diagnose the wrong hop. It happens
        // whenever tracing is turned on while an earlier write is still
        // settling, which is ordinary rather than rare.
        if what != Step::Began {
            return;
        }
        if self.by_op.len() >= self.max_ops {
            self.by_op.remove(0);
        }
        self.by_op.push((
            of,
            Trace {
                steps: vec![Stamped {
                    depth,
                    what,
                    n,
                    at_ms: 0,
                }],
                total_ms: 0,
                truncated: false,
            },
            now_ms,
        ));
    }

    /// The tree for one operation.
    pub fn of(&self, of: TraceOf) -> Option<&Trace> {
        self.by_op
            .iter()
            .find(|(o, _, _)| *o == of)
            .map(|(_, t, _)| t)
    }

    pub fn len(&self) -> usize {
        self.by_op.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_op.is_empty()
    }
}

impl Trace {
    /// The tree as indented lines, for a person to read.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for s in &self.steps {
            for _ in 0..s.depth {
                out.push_str("  ");
            }
            out.push_str(&format!("{:?}({}) +{}ms\n", s.what, s.n, s.at_ms));
        }
        if self.truncated {
            out.push_str("... truncated\n");
        }
        out
    }
}
