//! THE EPOCH STATEMENT witnessed with no page open (TOKEN §7a/§7d): part of the witness round (`witness_round`), for
//! the statement due now and the one before (as the page's `witness` service takes them), once its lot is drawn. Where
//! the member's key is SEATED at the statement's place (`rules::witness::seated`, the page's rule), the round reads what
//! the page's `statement` service reads — every vouch registry up to the epoch, the keepers registry, witness queue and
//! posted roots of the epoch before, the posted roots within `CARRY_BACK` — and BUILDS the statement by the page's one
//! rule (`rules::statement::build`, every keeper's record and the previous statement's blocks read through the same
//! round). Its root, when it is one of the roots POSTED for the epoch, is signed through the guard and sent to the
//! statement's signatures container exactly as an entry's (`witness_round::sign_into`: the page's `witness_put`
//! encoding). A root of its own posted by nobody is not signed (said, and not rebuilt until the posted roots change).
//!
//! THE SEALS (an audit re-seals each drawn piece: seconds each) are spread over calls: at most `AUDITS_PER_CALL`
//! keepers' audits are sealed per call, each verdict kept in the secrets (by the record it was sealed for); the build
//! PAUSES and the round resumes on the next wake-up with what it read.

use crate::reads::Round;
use crate::upkeep::Io;
use crate::witness_round::{bag_items, chain_items, sign_into};
use craftworks_identity::{Host, WitnessMandate};
use craftworks_reputation as R;
use craftworks_rules::{statement, tokens, witness};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// Keepers whose audits are sealed in one call (each `standing::record::AUDITS` seals, seconds each).
pub const AUDITS_PER_CALL: usize = 1;
/// Audit verdicts kept per member: (hash of statement epoch, record key and record, passed).
const SEALED: &[u8] = b"identity_witness/sealed/";
const SEALED_MAX: usize = 1024;

/// What the statements need next.
pub enum Next {
    /// Read these first.
    Read(Vec<[u8; 32]>),
    /// Seals left for a later call (nothing to read).
    Pause,
    Done,
}

/// The settled-list key of epoch `e`'s statement (signed, or not seated: never asked again).
fn settled_key(e: u64) -> [u8; 32] {
    blake3::derive_key("craftworks 2026-10-11 delegate statement settled", &e.to_le_bytes())
}
/// The settled-list key of a statement built and matching none of `posted` (rebuilt once the posted roots change).
fn unmatched_key(e: u64, posted: &[[u8; 32]]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("craftworks 2026-10-11 delegate statement unmatched");
    h.update(&e.to_le_bytes());
    let mut posted = posted.to_vec();
    posted.sort();
    posted.dedup();
    posted.iter().for_each(|r| {
        h.update(r);
    });
    *h.finalize().as_bytes()
}

fn sealed<H: Host>(h: &H, member: &[u8; 32]) -> Vec<([u8; 32], bool)> {
    h.get_secret(&[SEALED, &member[..]].concat()).and_then(|b| bincode::deserialize(&b).ok()).unwrap_or_default()
}
fn set_sealed<H: Host>(h: &mut H, member: &[u8; 32], mut all: Vec<([u8; 32], bool)>) {
    if all.len() > SEALED_MAX {
        all.drain(..all.len() - SEALED_MAX);
    }
    h.set_secret(&[SEALED, &member[..]].concat(), &bincode::serialize(&all).expect("verdicts encode"));
}

/// The roots POSTED for epoch `e` (its statement bag), in the bag's order (as the page reads them).
fn posted_roots(got: tokens::Got, bag: &[u8; 32], e: u64) -> Result<Vec<[u8; 32]>, [u8; 32]> {
    Ok(bag_items(got, bag, &R::statement::bag_address(e))?.iter().filter_map(|p| R::statement::Posted::decode(p)).filter(|p| p.epoch == e).map(|p| p.root).collect())
}
/// A read's items, or none with its id added to `want`.
fn take<T: Default>(want: &mut Vec<[u8; 32]>, x: Result<T, [u8; 32]>) -> T {
    x.unwrap_or_else(|w| {
        want.push(w);
        T::default()
    })
}

/// The statements due at beacon height `height`: signatures into `out`, settled keys into `settled`.
pub fn step<H: Host>(h: &mut H, r: &Round, m: &WitnessMandate, height: u64, settled: &mut Vec<[u8; 32]>, out: &mut Vec<Io>) -> Next {
    let Some(due) = height.checked_sub(R::statement::SETTLE).map(|x| x / R::EPOCH) else { return Next::Done };
    let mut epochs = vec![due.saturating_sub(1), due];
    epochs.dedup();
    for e in epochs {
        match epoch(h, r, m, e, height, settled, out) {
            Next::Done => {}
            next => return next,
        }
    }
    Next::Done
}

fn epoch<H: Host>(h: &mut H, r: &Round, m: &WitnessMandate, e: u64, height: u64, settled: &mut Vec<[u8; 32]>, out: &mut Vec<Io>) -> Next {
    let got = |id: &[u8; 32]| r.got.get(id).cloned();
    let ctx = tokens::Ctx { code_hash: m.token, beacon: m.beacon, idlog: m.idlog };
    let lot = tokens::Lot { pool: m.pool, bag: m.bag, beacon_code: m.beacon_code, idlog: m.idlog };
    let lot_height = R::statement::lot_height(e);
    if height < lot_height || settled.contains(&settled_key(e)) {
        return Next::Done;
    }
    // SEATED (before anything is built: most members are not).
    match witness::seated(&ctx, &lot, &got, &ctx.beacon, &R::statement::witness_place(e), 0, lot_height, &r.member) {
        tokens::Step::Want(w) => return Next::Read(w),
        tokens::Step::Done(Some(true)) => {}
        tokens::Step::Done(Some(false)) => {
            settled.push(settled_key(e));
            return Next::Done;
        }
        // Its lot's output not recorded yet: asked again later.
        tokens::Step::Done(None) | tokens::Step::Sign(_) => return Next::Done,
    }
    // THE ROOTS POSTED: none — nothing to sign (yet).
    let posted = match posted_roots(&got, &lot.bag, e) {
        Err(w) => return Next::Read(vec![w]),
        Ok(p) => p,
    };
    if posted.is_empty() || settled.contains(&unmatched_key(e, &posted)) {
        return Next::Done;
    }
    // THE INPUTS, as the page's `statement` service reads them (every read asked at once).
    let mut want: Vec<[u8; 32]> = Vec::new();
    let mut items: Vec<R::item::Item> = Vec::new();
    for x in 0..=e {
        items.extend(take(&mut want, chain_items(&got, &lot.bag, &R::item::registry_address, x)).iter().filter_map(|b| R::item::Item::decode(b)));
    }
    // THE MISDEED REGISTRIES of the epochs before (each item with its bag's epoch).
    let mut misdeeds: Vec<(u64, Vec<u8>)> = Vec::new();
    for x in 0..e {
        misdeeds.extend(take(&mut want, chain_items(&got, &lot.bag, &craftworks_rules::misdeed::registry_address, x)).into_iter().map(|b| (x, b)));
    }
    let (mut keepers, mut queued, mut before): (Vec<Vec<u8>>, Vec<craftworks_wallet::queue::Queued>, Vec<[u8; 32]>) = Default::default();
    if let Some(b) = e.checked_sub(1) {
        keepers = take(&mut want, chain_items(&got, &lot.bag, &R::keepers::address, b));
        queued = take(&mut want, chain_items(&got, &lot.bag, &craftworks_wallet::queue::address, b)).iter().filter_map(|q| craftworks_wallet::queue::Queued::decode(q)).collect();
        before = take(&mut want, posted_roots(&got, &lot.bag, b));
    }
    let mut earlier: Vec<(u64, [u8; 32])> = Vec::new();
    if e >= 2 {
        for x in (e.saturating_sub(R::statement::CARRY_BACK)..=e - 2).rev() {
            earlier.extend(take(&mut want, posted_roots(&got, &lot.bag, x)).into_iter().map(|root| (x, root)));
        }
    }
    if !want.is_empty() {
        return Next::Read(want);
    }
    // THE BUILD (the page's rule): every record read in this round is fresh; the previous statement's lines from its
    // Block contracts; the audits' seals spread over calls.
    let keep = craftworks_rules::keeper::Keep { standing: m.standing, beacon_code: m.beacon_code, beacon: m.beacon };
    let block_id = contract_keys::block::contract_deriver_of_hash(m.block);
    let fresh = |id: &[u8; 32]| r.got.contains_key(id);
    let lines = |root: &[u8; 32]| {
        let block = |c: &[u8; 32]| got(&block_id(c)).filter(|s| s.len() > 1).map(|s| s[1..].to_vec());
        R::statement::lines(root, &block).map_err(|cids| cids.iter().map(&block_id).collect())
    };
    let kept_before = sealed(h, &r.member);
    let verdicts: RefCell<Vec<([u8; 32], bool)>> = RefCell::new(Vec::new());
    let budget = std::cell::Cell::new(AUDITS_PER_CALL);
    let audit = |key: &[u8; 32], s: &craftworks_standing::record::State, seal: &dyn Fn() -> bool| -> Option<bool> {
        let mut x = blake3::Hasher::new_derive_key("craftworks 2026-10-11 delegate audit verdict");
        x.update(&e.to_le_bytes());
        x.update(key);
        x.update(&craftworks_standing::codec::encode(s));
        let k = *x.finalize().as_bytes();
        if let Some((_, v)) = kept_before.iter().chain(verdicts.borrow().iter()).find(|(h, _)| *h == k) {
            return Some(*v);
        }
        if budget.get() == 0 {
            return None;
        }
        budget.set(budget.get() - 1);
        let v = seal();
        verdicts.borrow_mut().push((k, v));
        Some(v)
    };
    let inputs = statement::Inputs { epoch: e, items: &items, misdeeds: &misdeeds, keepers: &keepers, queued: &queued, posted: &before, earlier: &earlier, ctx: &ctx, lot: &lot, keep: &keep, fresh: &fresh, lines: &lines, audit: Some(&audit) };
    let built = statement::build(&inputs, &got);
    let new = verdicts.into_inner();
    if !new.is_empty() {
        set_sealed(h, &r.member, kept_before.into_iter().chain(new).collect());
    }
    let st = match built {
        Err(why) => {
            crate::upkeep::followed(h, &r.member, &craftworks_rules::steps::ACCOUNT, craftworks_rules::steps::STATEMENT_FAILED, &format!("epoch {e}: {why}"));
            return Next::Done;
        }
        Ok(tokens::Step::Want(w)) if w.is_empty() => return Next::Pause,
        Ok(tokens::Step::Want(w)) => return Next::Read(w),
        Ok(tokens::Step::Sign(_)) => return Next::Done,
        Ok(tokens::Step::Done((st, _))) => st,
    };
    let Some((root, _)) = st.tree(&mut BTreeMap::new()) else {
        crate::upkeep::followed(h, &r.member, &craftworks_rules::steps::ACCOUNT, craftworks_rules::steps::STATEMENT_FAILED, &format!("epoch {e}: its tree does not build"));
        return Next::Done;
    };
    // THE COMPARISON: only a posted root this build reproduces is signed.
    if !posted.contains(&root) {
        let hex = |b: &[u8; 32]| b[..6].iter().map(|x| format!("{x:02x}")).collect::<String>();
        crate::upkeep::followed(h, &r.member, &craftworks_rules::steps::ACCOUNT, craftworks_rules::steps::STATEMENT_NOT_SIGNED, &format!("epoch {e}: built root {}…, posted {} — not signed", hex(&root), posted.iter().map(hex).collect::<Vec<_>>().join(", ")));
        settled.push(unmatched_key(e, &posted));
        return Next::Done;
    }
    match witness::statement(&ctx, &lot, &got, e, &root, &r.member) {
        tokens::Step::Want(w) => Next::Read(w),
        tokens::Step::Done(witness::Verdict::Sign { at, entry, owner }) => {
            sign_into(h, &r.member, &owner, &at, entry, height, out);
            settled.push(settled_key(e));
            Next::Done
        }
        _ => Next::Done,
    }
}
