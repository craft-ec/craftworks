//! THE SDK'S STATUS VOCABULARY (sdk#518): every word a session's status methods answer, each set owned by ONE enum
//! with `code()` and `ALL`. The session emits through these (`web/src/session.rs`), `status_words` exports them as one
//! list, and an app imports that list (`sdk.status`) -- never a copy of its own. `RowState` (store.rs) is the row's.

use crate::store::RowState;

use core_types::vocabulary;

vocabulary! {
    /// Where an app's contract PUT stands (`Session::put_status`'s `state`).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum PutStatus {
        None => "none",
        Pending => "pending",
        Put => "put",
        Refused => "refused",
        Cancelled => "cancelled",
    }
}

vocabulary! {
    /// How an app's site publication stands (`Session::site_status`'s `state`).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SiteStatus {
        None => "none",
        Publishing => "publishing",
        Published => "published",
        Superseded => "superseded",
        Refused => "refused",
        Cancelled => "cancelled",
    }
}

vocabulary! {
    /// Where an app's PUBLISH stands (`Session::app_publish_status`'s `state`, sdk#516; APP-PUBLISH.md): PUBLISHED at
    /// the site's read-back, BACKED_UP at every piece.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum AppPublishStatus {
        None => "none",
        Pieces => "pieces",
        Siting => "siting",
        Published => "published",
        BackedUp => "backed_up",
        BackupAbandoned => "backup_abandoned",
        Refused => "refused",
        Superseded => "superseded",
        Cancelled => "cancelled",
    }
}

vocabulary! {
    /// May this session write a head (`Session::can_write`'s `answer`)?
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum CanWrite {
        Yes => "yes",
        No => "no",
        Unknown => "unknown",
    }
}

vocabulary! {
    /// What the node's signer answered when asked which head it signs for (`Session::asked`'s `state`).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum AskedState {
        Pending => "pending",
        Register => "register",
        NoKey => "nokey",
        NoSigner => "nosigner",
        Refused => "refused",
    }
}

vocabulary! {
    /// What a whole-tree REPAIR did (sdk#479): REPAIRED everything missing was put back; PARTIAL some is not back yet;
    /// CANCELLED the pass was stopped (its counts are what it did). A pass's HEALTH -- nothing missing, missing but
    /// repairable, a group past repair -- is the engine's `GroupHealth` word, never a second spelling here
    /// ([`PassOutcome`]).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum RepairOutcome {
        Repaired => "repaired",
        Partial => "partial",
        Cancelled => "cancelled",
    }
}

/// How a whole-tree pass ended (sdk#479): a word of ONE of two lists -- the tree's health (`GroupHealth`: WHOLE,
/// DEGRADED, DAMAGED) or what a repair did ([`RepairOutcome`]). [`PassOutcome::list`] names which, so an app validates
/// the word against `sdk.status[list]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PassOutcome {
    Health(engine::repair::GroupHealth),
    Repair(RepairOutcome),
}

impl PassOutcome {
    /// THE one derivation (the tab only labels it), in this order: a cancel; a group past repair (DAMAGED); nothing
    /// missing (WHOLE); a CHECK that found some (DEGRADED: missing, every group solvable, nothing put); a repair that put
    /// all of it back, none refused (REPAIRED); else PARTIAL.
    pub fn of(check: bool, missing: u64, put_back: u64, rejected: u64, damaged: u64, cancelled: bool) -> PassOutcome {
        use engine::repair::GroupHealth;
        if cancelled {
            PassOutcome::Repair(RepairOutcome::Cancelled)
        } else if damaged > 0 {
            PassOutcome::Health(GroupHealth::Damaged)
        } else if missing == 0 {
            PassOutcome::Health(GroupHealth::Whole)
        } else if check {
            PassOutcome::Health(GroupHealth::Degraded)
        } else if rejected == 0 && put_back >= missing {
            PassOutcome::Repair(RepairOutcome::Repaired)
        } else {
            PassOutcome::Repair(RepairOutcome::Partial)
        }
    }

    /// The word, from its owner's list.
    pub fn code(self) -> &'static str {
        match self {
            PassOutcome::Health(h) => h.code(),
            PassOutcome::Repair(r) => r.code(),
        }
    }

    /// Which `words()` list the word is in.
    pub fn list(self) -> &'static str {
        match self {
            PassOutcome::Health(_) => "groupHealth",
            PassOutcome::Repair(_) => "repairOutcome",
        }
    }
}

/// Every list, as the JSON `status_words` exports: `{"rowState":[..], "putStatus":[..], "siteStatus":[..],
/// "appPublishStatus":[..], "canWrite":[..], "asked":[..], "groupHealth":[..], "repairOutcome":[..], "failWhy":[..]}`
/// (groupHealth and failWhy the engine's: sdk#524, and why a write `Failed`, sdk#500).
pub fn words() -> serde_json::Value {
    fn list<T: Copy>(all: &[T], code: fn(T) -> &'static str) -> Vec<&'static str> {
        all.iter().map(|w| code(*w)).collect()
    }
    serde_json::json!({
        "rowState": list(&RowState::ALL, RowState::code),
        "putStatus": list(&PutStatus::ALL, PutStatus::code),
        "siteStatus": list(&SiteStatus::ALL, SiteStatus::code),
        "appPublishStatus": list(&AppPublishStatus::ALL, AppPublishStatus::code),
        "canWrite": list(&CanWrite::ALL, CanWrite::code),
        "asked": list(&AskedState::ALL, AskedState::code),
        "groupHealth": list(&engine::repair::GroupHealth::ALL, engine::repair::GroupHealth::code),
        "repairOutcome": list(&RepairOutcome::ALL, RepairOutcome::code),
        "failWhy": list(&engine::FailWhy::ALL, engine::FailWhy::code),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// ONE LIST: `words()` is each enum's ALL, each word once, and nothing else.
    #[test]
    fn words_is_every_enum_s_all_each_word_once() {
        let v = words();
        let want: [(&str, Vec<&str>); 9] = [
            ("rowState", RowState::ALL.iter().map(|w| w.code()).collect()),
            ("putStatus", PutStatus::ALL.iter().map(|w| w.code()).collect()),
            ("siteStatus", SiteStatus::ALL.iter().map(|w| w.code()).collect()),
            ("appPublishStatus", AppPublishStatus::ALL.iter().map(|w| w.code()).collect()),
            ("canWrite", CanWrite::ALL.iter().map(|w| w.code()).collect()),
            ("asked", AskedState::ALL.iter().map(|w| w.code()).collect()),
            ("groupHealth", engine::repair::GroupHealth::ALL.iter().map(|w| w.code()).collect()),
            ("repairOutcome", RepairOutcome::ALL.iter().map(|w| w.code()).collect()),
            ("failWhy", engine::FailWhy::ALL.iter().map(|w| w.code()).collect()),
        ];
        assert_eq!(v.as_object().map(|o| o.len()), Some(want.len()), "words() carries a list no enum owns: {v}");
        for (name, list) in want {
            let got: Vec<&str> = v[name].as_array().expect(name).iter().map(|w| w.as_str().expect("a word")).collect();
            assert_eq!(got, list, "{name} is not its enum's ALL");
            assert_eq!(list.iter().collect::<BTreeSet<_>>().len(), list.len(), "{name} says a word twice");
        }
    }

    /// Every word names its own variant back (`vocabulary!` builds `code`, `ALL` and `from_code` from one list, so
    /// no variant can be missing from `ALL`: a variant added to the list is in all three at once).
    #[test]
    fn every_word_names_its_variant_back() {
        fn round<T: Copy + PartialEq + std::fmt::Debug>(all: &[T], code: fn(T) -> &'static str, from: fn(&str) -> Option<T>) {
            for w in all {
                assert_eq!(from(code(*w)), Some(*w), "{w:?}");
            }
            assert_eq!(from("NOT_A_WORD"), None);
        }
        round(&RowState::ALL, RowState::code, RowState::from_code);
        round(&PutStatus::ALL, PutStatus::code, PutStatus::from_code);
        round(&SiteStatus::ALL, SiteStatus::code, SiteStatus::from_code);
        round(&CanWrite::ALL, CanWrite::code, CanWrite::from_code);
        round(&AskedState::ALL, AskedState::code, AskedState::from_code);
        round(&engine::repair::GroupHealth::ALL, engine::repair::GroupHealth::code, engine::repair::GroupHealth::from_code);
        round(&RepairOutcome::ALL, RepairOutcome::code, RepairOutcome::from_code);
    }

    /// Lost is the rolled-back state, and nothing else.
    #[test]
    fn lost_is_rolled_back_only() {
        for s in RowState::ALL {
            assert_eq!(s.is_lost(), s == RowState::RolledBack, "{}", s.code());
        }
    }

    /// Each outcome, the list it is in, and the order the rules decide in (cancel > damaged > whole > degraded (a
    /// check) > repaired > partial). No word is spelled in two lists.
    #[test]
    fn a_pass_outcome_is_derived_in_one_order_from_its_owners_words() {
        use engine::repair::GroupHealth;
        let w = |o: PassOutcome| (o.list(), o.code());
        assert_eq!(w(PassOutcome::of(false, 0, 0, 0, 0, false)), ("groupHealth", GroupHealth::Whole.code()));
        assert_eq!(w(PassOutcome::of(true, 3, 0, 0, 0, false)), ("groupHealth", GroupHealth::Degraded.code()), "a check that found missing");
        assert_eq!(w(PassOutcome::of(true, 3, 0, 0, 1, false)), ("groupHealth", GroupHealth::Damaged.code()));
        assert_eq!(w(PassOutcome::of(false, 3, 3, 0, 0, false)), ("repairOutcome", "repaired"));
        assert_eq!(w(PassOutcome::of(false, 3, 2, 0, 0, false)), ("repairOutcome", "partial"));
        assert_eq!(w(PassOutcome::of(false, 3, 3, 1, 0, false)), ("repairOutcome", "partial"), "a refused put-back is not repaired");
        assert_eq!(w(PassOutcome::of(false, 3, 3, 0, 1, false)), ("groupHealth", GroupHealth::Damaged.code()), "damaged wins over put-backs");
        assert_eq!(w(PassOutcome::of(true, 3, 3, 0, 1, true)), ("repairOutcome", "cancelled"), "a cancel wins over everything");
        let health: Vec<&str> = GroupHealth::ALL.iter().map(|h| h.code()).collect();
        assert!(RepairOutcome::ALL.iter().all(|r| !health.iter().any(|h| h.eq_ignore_ascii_case(r.code()))), "a word spelled in both lists");
    }
}
