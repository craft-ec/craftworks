//! What a page says about its redundancy, from the moment it opens
//! (craftworks-sdk#119): a page that read a non-empty head never looked at
//! that tree's parity, so it says `NotScanned` — never "0 owed". Only a page
//! that started from the empty tree knows the tree in full.

use engine::{ParityScan, Params};
use page::{Answer, Ms, Op, Page, PutPath};

fn opened_on(head: Option<(u64, [u8; 32])>) -> Page {
    let mut p = Page::new(Params::default(), PutPath::Page);
    let ops = p.take_ops();
    assert!(ops.iter().any(|o| matches!(o, Op::ReadHead { label: page::Label::Head })), "a page reads its head first: {ops:?}");
    p.answer(Answer::Head { label: page::Label::Head, read: head.map(Into::into) }, Ms(1));
    p
}

#[test]
fn a_page_opened_on_an_existing_head_says_not_scanned_never_zero_owed() {
    let p = opened_on(Some((7, [9; 32])));
    assert_eq!(p.published().0, 7, "the head was not taken, so this proves nothing");
    assert_eq!(p.parity_scan(), &ParityScan::NotScanned);
}

/// The control: a page with no head starts from the empty tree, which lists no
/// parity at all — its zero IS "all put".
#[test]
fn a_page_opened_on_no_head_knows_the_empty_tree_in_full() {
    let p = opened_on(None);
    assert!(matches!(p.parity_scan(), ParityScan::Done { .. }), "{:?}", p.parity_scan());
}

/// RACE PUT's MARK (COMMIT-LIFE §P): a head this build signs carries a
/// `TAG_PARITY` in a v2 ledger, meaning every group it lists was recoverable
/// when it was signed. A page opened on such a head knows that without a
/// scan: `Done`. The same root with NO mark (the control above) is
/// `NotScanned`, and so is a v1 ledger's parity field (#119's scan front, a
/// different meaning, dropped on read).
#[test]
fn a_page_opened_on_a_marked_head_says_done_and_a_v1_parity_field_is_not_the_mark() {
    let root = [9u8; 32];
    let signer = Page::new(Params::default(), PutPath::Page);
    let marked: Vec<u8> = root.iter().copied().chain(signer.sign_ledger_of(6, [8; 32], 7, root)).collect();
    let head = page::HeadRead::from_value(7, &marked).expect("a head");
    assert!(head.mark().is_some(), "this build's signed head carries no §P mark");
    let mut p = Page::new(Params::default(), PutPath::Page);
    let _ = p.take_ops();
    p.answer(Answer::Head { label: page::Label::Head, read: Some(head) }, Ms(1));
    assert_eq!(p.published().0, 7, "the head was not taken, so this proves nothing");
    assert_eq!(p.parity_scan(), &ParityScan::Done { root }, "a marked head was not taken as scanned");

    // A v1 ledger whose parity field is the OLD scan front (here empty too):
    // never the mark.
    use signer_proto::head::{LEDGER_VERSION_V1, TAG_PARITY};
    let mut v1 = root.to_vec();
    v1.extend_from_slice(&[LEDGER_VERSION_V1, TAG_PARITY, 0, 0]);
    let old = page::HeadRead::from_value(7, &v1).expect("a head");
    assert!(old.mark().is_none(), "a v1 parity field was read as the §P mark");
    let mut q = Page::new(Params::default(), PutPath::Page);
    let _ = q.take_ops();
    q.answer(Answer::Head { label: page::Label::Head, read: Some(old) }, Ms(1));
    assert_eq!(q.parity_scan(), &ParityScan::NotScanned);
}

/// The mark LISTS the root's parity ids (sdk#335) and a reader reads them
/// back; a body that is not whole ids is still the mark, listing none (the
/// root is then fetched alone, never rebuilt from a guess).
#[test]
fn the_mark_lists_the_roots_parity_and_reads_it_back() {
    let root = [9u8; 32];
    let ids: Vec<[u8; 32]> = (1..=engine::PARITY as u8).map(|i| [i; 32]).collect();
    use signer_proto::head::{value, Ledger};
    let v = value(&root, &Ledger { parity: Some(ids.concat()), ..Ledger::default() });
    let head = page::HeadRead::from_value(7, &v).expect("a head");
    assert_eq!(head.mark(), Some(ids), "the root's parity did not round-trip through the head");
    use signer_proto::head::{LEDGER_VERSION, TAG_PARITY};
    let mut odd = root.to_vec();
    odd.extend_from_slice(&[LEDGER_VERSION, TAG_PARITY, 3, 0, 1, 2, 3]);
    let odd = page::HeadRead::from_value(7, &odd).expect("a head");
    assert_eq!(odd.mark(), Some(Vec::new()), "a mark of 3 bytes was read as ids");
}

/// The root's parity fits the mark at the tree's parity per group and at
/// m = 8 (#321): checked when this test builds, so it cannot pass by not
/// running.
const _: () = assert!(engine::PARITY * 32 <= signer_proto::head::PARITY_MAX);
const _: () = assert!(8 * 32 <= signer_proto::head::PARITY_MAX);

/// THE BUDGET (sdk#335, core dev): the root's parity fits the mark at the
/// tree's parity per group, and at m = 8 (#321) -- and a value with EVERY
/// field at its largest (prev, a mark of 8 ids, THROUGH_MAX devices) still
/// fits the Register's value and passes the signer's check. A tree with more
/// parity than the mark holds fails here (and the page's const assert fails
/// the build) before a head is ever cut short.
#[test]
fn the_root_parity_fits_the_head_value_at_m_8() {
    use signer_proto::head::{check, value, Ledger, Through, THROUGH_MAX, VALUE_MAX};
    let through: Vec<Through> = (0..THROUGH_MAX).map(|i| Through { device: (i as u128).to_be_bytes(), seq: u64::MAX, last: u64::MAX }).collect();
    let prev = Some(signer_proto::Head { seq: u64::MAX, root: [7; 32] });
    let full = value(&[9; 32], &Ledger { prev, through: through.clone(), parity: Some(vec![0xAB; 8 * 32]) });
    println!("  a head at every field's largest: {} of {VALUE_MAX} B ({} devices)", full.len(), through.len());
    assert!(full.len() <= VALUE_MAX, "{} B > VALUE_MAX", full.len());
    assert_eq!(check(&full), Ok(()), "the signer refuses the largest head");
    let read = signer_proto::head::read_value(&full).expect("reads");
    assert_eq!(read.ledger.through.len(), THROUGH_MAX, "a device was evicted to fit the mark");
}
