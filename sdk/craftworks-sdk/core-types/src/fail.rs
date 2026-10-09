//! WHY A WRITE ENDED `Failed` (sdk#500): ONE list, read by the engine that ends a write, the page that refuses one at
//! its door, the protocol that carries it, and the SDK that tells the app. Layer 0 because both the engine's context
//! and the protocol serialize it, and neither depends on the other.
//!
//! `Failed` is TERMINAL and nothing of the write is in the tree. Each word below names the one cause that ended it,
//! as the code that decides it says (the site is named on each).

crate::vocabulary! {
    /// The cause of a write's `Failed`. APPEND ONLY: a variant's position is its wire tag.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
    pub enum FailWhy {
        /// A block on the tree's path does not parse, or does not match its parent: the write's reads could not be
        /// checked, or its edits applied (the engine's `ReadsCheck::Unreadable`, `ApplyError::Read`).
        TreeDamaged => "TREE_DAMAGED",
        /// The tree refused the edit itself: keys out of order, a key or value past its bound, or a node that could
        /// not be built (the engine's other `ApplyError`s).
        EditRefused => "EDIT_REFUSED",
        /// Its path needed more fetch rounds than the engine gives one write (`max_apply_rounds`).
        FetchBudget => "FETCH_BUDGET",
        /// A TIME CUT-OFF (rule 8): parked for blocks, it heard nothing -- no block it asked for, no ask after it
        /// from its page -- for `3 * reask_after` engine ticks (`release_silent_parked_write`). Its page's
        /// `AskWrite` is what normally keeps this from firing (sdk#257).
        SilentRelease => "SILENT_RELEASE",
        /// The engine's context was over its bound with every parked read shed: `worst_case_fixed_bytes` was
        /// estimated too low (a debug build asserts; a release one fails the parked write).
        ContextFull => "CONTEXT_FULL",
        /// The node's Block contract rejected a block its commit needed (`Event::PutRejected`, sdk#433).
        BlockRejected => "BLOCK_REJECTED",
        /// The signer finally refused its commit's head (`Event::HeadRefused`, PUBLISH-LIFE ⁵). The page names the
        /// signer's own why beside it.
        SignerRefused => "SIGNER_REFUSED",
        /// Refused at the page's door: this page is a VIEW of another identity's tree, and a view writes nothing
        /// (`page::server`). Never the engine's.
        ReadOnly => "READ_ONLY",
    }
}

impl FailWhy {
    /// What it means to the person whose write it was, in a sentence that follows "could not be saved: ".
    pub fn says(self) -> &'static str {
        match self {
            FailWhy::TreeDamaged => "a block of the saved tree is damaged",
            FailWhy::EditRefused => "the tree refused the edit (a key or value too long, or keys out of order)",
            FailWhy::FetchBudget => "reaching its place in the tree took more fetches than one write is allowed",
            FailWhy::SilentRelease => "it waited for the network and heard nothing for too long",
            FailWhy::ContextFull => "the engine ran out of room (an SDK bug: please report it)",
            FailWhy::BlockRejected => "the network rejected a block of its commit",
            FailWhy::SignerRefused => "the signer refused to sign its commit",
            FailWhy::ReadOnly => "this page is a view of someone else's data, and a view writes nothing",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FailWhy;

    #[test]
    fn every_code_is_one_word_and_reads_back() {
        let mut seen = std::collections::BTreeSet::new();
        for w in FailWhy::ALL {
            assert!(seen.insert(w.code()), "{} is two words' code", w.code());
            assert_eq!(FailWhy::from_code(w.code()), Some(w));
            assert!(!w.says().is_empty());
        }
        assert_eq!(FailWhy::from_code("NOT_A_CAUSE"), None);
    }
}
