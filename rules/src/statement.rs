//! THE EPOCH STATEMENT's build, pure over what is read (TOKEN §7d) — the page's (core `statement_build`) and, so a
//! seated node signs a statement with no page open, the identity delegate's (audit 10-11): one implementation. From
//! the signed vouch items, the keepers registry of the epoch before (each listed keeper's standing record read FRESH),
//! the witness queue and statement roots of the epoch before (witness fees), and the newest ACCEPTED statement within
//! `CARRY_BACK` epochs (each account's last active epoch): the statement and how many accounts KEPT.

use crate::keeper::{self, Keep};
use crate::tokens::{Ctx, Got, Lot, Step};
use crate::witness;
use craftworks_reputation as R;
use std::collections::BTreeMap;

/// What a statement is built from, besides what is read through `got`.
pub struct Inputs<'a> {
    pub epoch: u64,
    /// The signed vouch items of every epoch up to this one.
    pub items: &'a [R::item::Item],
    /// The keepers registry's items of the epoch before.
    pub keepers: &'a [Vec<u8>],
    /// The witness queue of the epoch before (its seated signatures earn fees).
    pub queued: &'a [craftworks_wallet::queue::Queued],
    /// The roots posted for the epoch before's statement, and for earlier epochs within `CARRY_BACK` ((epoch, root)).
    pub posted: &'a [[u8; 32]],
    pub earlier: &'a [(u64, [u8; 32])],
    /// The token, its lot (the $CRAFT pool, the bag's and beacon's codes) and where keepers' records are.
    pub ctx: &'a Ctx,
    pub lot: &'a Lot,
    pub keep: &'a Keep,
    /// A keeper's record READ FRESH for this build (`false`: ask for it again, whatever is held).
    pub fresh: &'a dyn Fn(&[u8; 32]) -> bool,
    /// A statement's lines from its root (`Err(ids)`: read these blocks' contracts first).
    pub lines: &'a dyn Fn(&[u8; 32]) -> Result<BTreeMap<R::Did, R::statement::Line>, Vec<[u8; 32]>>,
}

/// The statement of `i.epoch` and how many accounts kept the epoch before — `Want` what is not read yet.
pub fn build(i: &Inputs, got: Got) -> Result<Step<(R::statement::Statement, usize)>, String> {
    let epoch = i.epoch;
    // Every voucher's and listed keeper's KEY LOG.
    let log_id = |d: &R::Did| craftworks_token_contract::id(&i.ctx.idlog, d);
    let accounts = i.items.iter().map(|it| it.from).chain(i.keepers.iter().filter_map(|b| R::keepers::Keeper::decode(b)).map(|k| k.did));
    let mut want: Vec<[u8; 32]> = Vec::new();
    for d in accounts {
        let id = log_id(&d);
        if got(&id).is_none() && !want.contains(&id) {
            want.push(id);
        }
    }
    if !want.is_empty() {
        return Ok(Step::Want(want));
    }
    let log_of = |d: &R::Did| got(&log_id(d)).filter(|b| !b.is_empty()).and_then(|b| craftworks_idlog_contract::read(d, &b));
    // KEPT (the epoch before): each listed keeper's standing record, read fresh.
    let listed = R::keepers::listed(i.keepers, &log_of);
    let want: Vec<[u8; 32]> = listed.iter().map(|(_, key)| keeper::record_id(i.keep, key)).filter(|id| !(i.fresh)(id)).collect();
    if !want.is_empty() {
        return Ok(Step::Want(want));
    }
    let record_of = |key: &[u8; 32]| got(&keeper::record_id(i.keep, key)).filter(|b| !b.is_empty()).and_then(|b| craftworks_standing::codec::decode(&b));
    let kept = match epoch.checked_sub(1) {
        Some(e) => craftworks_standing::record::kept_accounts(&listed, &record_of, e),
        None => Vec::new(),
    };
    // SIGNED (the epoch before's witness fees) and THE PREVIOUS STATEMENT (the newest accepted within CARRY_BACK).
    let mut prev = BTreeMap::new();
    let signed = match epoch.checked_sub(1) {
        None => Default::default(),
        Some(before) => {
            let place = |e: u64, root: [u8; 32]| craftworks_wallet::fees::Statement { place: R::statement::witness_place(e), root, h: R::statement::lot_height(e) };
            let statements: Vec<craftworks_wallet::fees::Statement> = i.posted.iter().map(|r| place(before, *r)).collect();
            match witness::signed(i.ctx, i.lot, got, i.queued, &statements) {
                Step::Want(w) => return Ok(Step::Want(w)),
                Step::Sign(_) => return Err("a count never signs".into()),
                Step::Done(s) => {
                    let mut epochs: Vec<Vec<craftworks_wallet::fees::Statement>> = vec![statements];
                    for b in 2..=R::statement::CARRY_BACK {
                        let Some(e) = epoch.checked_sub(b) else { break };
                        epochs.push(i.earlier.iter().filter(|x| x.0 == e).map(|x| place(e, x.1)).collect());
                    }
                    if let Some(st) = epochs.iter().find_map(|sts| witness::accepted_statement(i.ctx, i.lot, got, sts)) {
                        match (i.lines)(&st.root) {
                            Ok(l) => prev = l,
                            Err(w) => return Ok(Step::Want(w)),
                        }
                    }
                    s
                }
            }
        }
    };
    let pots = R::statement::Pots { keeper: craftworks_wallet::craft::keeper_pot(epoch), allowance: (craftworks_allowance_contract::POT, craftworks_allowance_contract::CAP), witness: craftworks_wallet::craft::witness_pot(epoch), halvings: craftworks_wallet::craft::halvings(epoch) };
    Ok(Step::Done((R::statement::build(epoch, i.items, &log_of, &kept, &signed, &prev, pots), kept.len())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The build asks for each voucher's key log first, and builds once it is read (as the page and the delegate run it).
    #[test]
    fn a_statement_asks_for_its_vouchers_key_logs_then_builds() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let lot = Lot { pool: [1; 32], bag: [2; 32], beacon_code: [3; 32] };
        let keep = Keep { standing: [4; 32], beacon_code: [3; 32], beacon: [7; 32] };
        let items = vec![R::item::Item { from: [11; 32], to: [12; 32], at: 5, on: true, cred: vec![], key_seq: 0, sig: [0; 64] }];
        let none = |_: &[u8; 32]| false;
        let lines = |_: &[u8; 32]| Ok(BTreeMap::new());
        let i = Inputs { epoch: 0, items: &items, keepers: &[], queued: &[], posted: &[], earlier: &[], ctx: &ctx, lot: &lot, keep: &keep, fresh: &none, lines: &lines };
        let mut got: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
        let g = got.clone();
        let Ok(Step::Want(w)) = build(&i, &|x| g.get(x).cloned()) else { panic!("asks first") };
        assert_eq!(w, vec![craftworks_token_contract::id(&ctx.idlog, &[11; 32])], "the voucher's key log");
        got.insert(w[0], vec![]);
        let Ok(Step::Done((st, kept))) = build(&i, &|x| got.get(x).cloned()) else { panic!("built") };
        assert_eq!((st.epoch, kept), (0, 0));
    }
}
