# craftworks-instrument

Permanent, reusable instrumentation middleware for the Craftworks stack.

Every test, measurement and shipped component records through it, so a failure has already
explained itself by the time anyone looks — nobody re-runs with one-off debug added — and the same
recording is the production support tool.

- **A frozen four-event spine:** `Enter` · `Exit(Outcome)` · `Counter` · `Edge` (a request paired
  with its response, so *outstanding* is a subtraction). Everything that changes rides in a small,
  typed, skippable payload. One version number per stream.
- **A probe can never become an input:** `fn event(&self, e)` returns nothing and there is no
  read-back on the trait. A full buffer drops (and counts the drop); it never grows and never
  panics. Events carry a per-recorder sequence, never a clock.
- **No user content, by vocabulary:** every payload key and enumerated value comes from a fixed,
  reviewed list in this crate. A site never emits a value derived from user data, however small
  or well-typed.
- **Contract crates never depend on this** — a dependency's identity alone moves a contract's
  wasm hash. Their gates prove its absence from the dependency closure.

Status: development. Issues and the roadmap live on the
[Craftworks Roadmap](https://github.com/orgs/craft-ec/projects/2). Contribution rules:
[craft-ec/.github](https://github.com/craft-ec/.github/blob/main/CONTRIBUTING.md).
