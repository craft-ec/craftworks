//! The cold-read verdict's every arm, on demand (F52, sdk#173).

use probe::verdict::{cold_read, Verdict, COLD_READ_MS, STALLED_AFTER_MS};

#[test]
fn a_page_in_eight_seconds_is_green() {
    assert_eq!(cold_read(&[(20, Some(8_020)), (21, Some(1_003))], vec![]), Verdict::Green);
}

#[test]
fn run_11_a_page_at_68_2_seconds_is_stalled_recovered() {
    assert_eq!(
        cold_read(&[(20, Some(68_176)), (21, Some(1_003))], vec![]),
        Verdict::StalledRecovered { secs: 68, req: 20 }
    );
}

#[test]
fn nothing_within_100_seconds_is_red() {
    let Verdict::Red(why) = cold_read(&[(20, None)], vec![]) else { panic!("not red") };
    assert_eq!(why, vec!["req 20 NOT ANSWERED within 100 s".to_string()]);
}

#[test]
fn a_page_later_than_the_window_is_red_not_recovered() {
    assert!(matches!(cold_read(&[(20, Some(COLD_READ_MS + 1))], vec![]), Verdict::Red(_)));
    assert_eq!(cold_read(&[(20, Some(COLD_READ_MS))], vec![]), Verdict::StalledRecovered { secs: 100, req: 20 });
}

#[test]
fn the_stall_threshold_at_both_edges() {
    assert_eq!(cold_read(&[(20, Some(STALLED_AFTER_MS))], vec![]), Verdict::Green);
    assert_eq!(cold_read(&[(20, Some(STALLED_AFTER_MS + 1))], vec![]), Verdict::StalledRecovered { secs: 30, req: 20 });
}

#[test]
fn another_red_check_outranks_a_recovery() {
    let v = cold_read(&[(20, Some(68_176))], vec!["TEST 1 ANSWERED WRONG: read 256 of 300 rows".into()]);
    assert_eq!(v, Verdict::Red(vec!["TEST 1 ANSWERED WRONG: read 256 of 300 rows".into()]));
}

#[test]
fn no_cold_read_sent_is_red() {
    assert!(matches!(cold_read(&[], vec![]), Verdict::Red(_)));
}
