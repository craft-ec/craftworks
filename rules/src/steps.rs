//! THE GROUP's STEPS — one vocabulary for page and node (10-19, inventory D): each step of a space's group (a thing
//! done, nothing changed in its state) by NAME, one meaning each, and whether it is QUIET (journaled, not reported).
//! Who took it is the step's own detail (`by`: the node, or a page). The page's lifecycle table is built from this list
//! (`core::group_steps`); the node records these names (`upkeep::followed`). A name no list knows (an older build's) is
//! recorded as `NODE_STEP`, never a stop.

pub const COMMIT: &str = "commit";
pub const ENTRY_SKIPPED: &str = "entry-skipped";
pub const LIST_SHORT: &str = "list-short";
pub const MOVED_MEANWHILE: &str = "moved-meanwhile";
pub const LOG_REFUSED: &str = "log-refused";
pub const COMMIT_REFUSED: &str = "commit-refused";
pub const NEXT_REFUSED: &str = "next-refused";
pub const STALLED: &str = "stalled";
pub const UNLOADABLE: &str = "unloadable";
pub const INFO_PUBLISHED: &str = "info-published";
pub const INFO_HELD: &str = "info-held";
pub const OUT_PLAN: &str = "out-plan";
pub const OUT_NOT_MY_TURN: &str = "out-not-my-turn";
pub const OUT_NOT_BUILT: &str = "out-not-built";
pub const KEYS_IN_STEP: &str = "keys-in-step";
pub const KEYS_RECONCILED: &str = "keys-reconciled";
pub const KEYS_SHORT: &str = "keys-short";
pub const KEYS_NOT_PLACED: &str = "keys-not-placed";
pub const KEYS_REFUSED: &str = "keys-refused";
pub const KEYS_NO_MEMBER: &str = "keys-no-member";
pub const KEYS_WAIT: &str = "keys-wait";
pub const KEYS_WRITE: &str = "keys-write";
pub const KEYS_OPS_UNREAD: &str = "keys-ops-unread";
pub const NODE_STEP: &str = "node-step";
pub const NODE_DROPPED: &str = "node-dropped";

/// EVERY STEP: `(name, quiet)`.
pub const ALL: &[(&str, bool)] = &[
    (COMMIT, false),
    ("applied", false),
    ("apply-failed", false),
    ("enter", false),
    ("enter-no-info", false),
    ("enter-not-listed", false),
    ("writers-found", true),
    ("writers-waiting", false),
    ("announce-failed", false),
    (ENTRY_SKIPPED, false),
    ("catch-up-deferred", false),
    ("adopt-delegate", false),
    ("pending-published", false),
    ("log-open-written", true),
    ("log-open-left", true),
    ("log-back-added", true),
    ("log-mended", true),
    ("log-mend-cannot", false),
    ("history-received", true),
    ("history-handed", true),
    (LIST_SHORT, false),
    (MOVED_MEANWHILE, false),
    (LOG_REFUSED, false),
    (COMMIT_REFUSED, false),
    (NEXT_REFUSED, false),
    (STALLED, false),
    (UNLOADABLE, false),
    (INFO_PUBLISHED, true),
    (INFO_HELD, false),
    (OUT_PLAN, false),
    (OUT_NOT_MY_TURN, false),
    (OUT_NOT_BUILT, false),
    ("keys-tree-made", false),
    (KEYS_RECONCILED, false),
    (KEYS_IN_STEP, true),
    (KEYS_SHORT, false),
    (KEYS_NOT_PLACED, false),
    (KEYS_REFUSED, false),
    ("keys-waiting", false),
    ("keys-unmapped", false),
    ("keys-bridged", false),
    ("keys-crossed", false),
    (KEYS_NO_MEMBER, false),
    (KEYS_WAIT, false),
    (KEYS_WRITE, false),
    (KEYS_OPS_UNREAD, false),
    (NODE_STEP, false),
    (NODE_DROPPED, false),
];

#[cfg(test)]
mod tests {
    /// One name, one entry.
    #[test]
    fn every_step_is_named_once() {
        let mut names: Vec<&str> = super::ALL.iter().map(|(n, _)| *n).collect();
        let n = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), n);
    }
}
