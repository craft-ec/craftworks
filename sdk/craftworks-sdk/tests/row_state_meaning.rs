//! What a row's state MEANS has one owner, `RowState`: every code it can
//! report is in `ALL`, each code names its state back, and exactly `CLEAN`
//! and `BACKED_UP` are saved. Apps ask this (`rowSaved`, `rowStates`) instead
//! of keeping their own copy — the builder's `=== "CLEAN"` stalled every
//! publish the day `BACKED_UP` arrived.
use craftworks_sdk::store::RowState;

#[test]
fn every_state_is_listed_and_its_code_names_it_back() {
    for s in RowState::ALL {
        assert_eq!(RowState::from_code(s.code()), Some(s), "{s:?}");
    }
    let mut codes: Vec<_> = RowState::ALL.iter().map(|s| s.code()).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), RowState::ALL.len(), "two states share a code");
    assert_eq!(RowState::from_code("NOT_A_STATE"), None);
}

#[test]
fn exactly_clean_and_backed_up_are_saved() {
    let saved: Vec<_> = RowState::ALL.iter().filter(|s| s.is_settled()).map(|s| s.code()).collect();
    assert_eq!(saved, ["CLEAN", "BACKED_UP"]);
    let backed: Vec<_> = RowState::ALL.iter().filter(|s| s.is_backed_up()).map(|s| s.code()).collect();
    assert_eq!(backed, ["BACKED_UP"]);
}
