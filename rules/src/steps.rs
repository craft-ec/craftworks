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
// The node's OTHER rounds — a space's files re-keyed, an account's keeper and witness work: the same one record
// (`upkeep::followed`), never free text. Not about a space: the account's (`ACCOUNT`).
/// The space a step not about any space is recorded under (the page: the account's).
pub const ACCOUNT: [u8; 32] = [0; 32];
pub const REKEY_IDLE: &str = "rekey-idle";
pub const REKEY_NO_CODES: &str = "rekey-no-codes";
pub const FILES_READ: &str = "files-read";
pub const FILES_NEW_SALT: &str = "files-new-salt";
pub const FILES_REKEYED: &str = "files-rekeyed";
pub const FILES_NOT_REKEYED: &str = "files-not-rekeyed";
pub const FILES_LEFT: &str = "files-left-to-page";
pub const FILES_BURNED: &str = "files-burned";
pub const FILES_WRITE_REFUSED: &str = "files-write-refused";
pub const FILES_NOT_WRITTEN: &str = "files-not-written";
pub const KEEPER_FAILED: &str = "keeper-failed";
pub const KEEPER_EPOCH: &str = "keeper-epoch-opened";
pub const KEEPER_CLAIMED: &str = "keeper-claimed";
pub const KEEPER_POINTED: &str = "keeper-claim-pointed";
pub const KEEPER_ANSWERED: &str = "keeper-answered";
pub const WITNESS_FAILED: &str = "witness-failed";
pub const WITNESS_REFUSED: &str = "witness-refused";
pub const WITNESS_NOT_SIGNED: &str = "witness-not-signed";
pub const STATEMENT_FAILED: &str = "statement-failed";
pub const STATEMENT_NOT_SIGNED: &str = "statement-not-signed";

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
    ("keys-republished", false),
    (KEYS_NO_MEMBER, false),
    (KEYS_WAIT, false),
    (KEYS_WRITE, false),
    (KEYS_OPS_UNREAD, false),
    (NODE_STEP, false),
    (NODE_DROPPED, false),
    (REKEY_IDLE, true),
    (REKEY_NO_CODES, false),
    (FILES_READ, true),
    (FILES_NEW_SALT, true),
    (FILES_REKEYED, true),
    (FILES_NOT_REKEYED, false),
    (FILES_LEFT, false),
    (FILES_BURNED, true),
    (FILES_WRITE_REFUSED, false),
    (FILES_NOT_WRITTEN, false),
    (KEEPER_FAILED, false),
    (KEEPER_EPOCH, true),
    (KEEPER_CLAIMED, true),
    (KEEPER_POINTED, true),
    (KEEPER_ANSWERED, true),
    (WITNESS_FAILED, false),
    (WITNESS_REFUSED, false),
    (WITNESS_NOT_SIGNED, false),
    (STATEMENT_FAILED, false),
    (STATEMENT_NOT_SIGNED, false),
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
