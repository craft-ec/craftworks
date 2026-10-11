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
    // THE AUDITS (`standing::record::kept_audited`, TOKEN §8 item 6): the beacon's output at this statement's LOT draws,
    // per keeper whose record would count as kept, which answers' picks are re-sealed from their pieces. Read first: the
    // lot's output (the beacon read FRESH: a held copy may predate it), each drawn answer's height's output, each drawn
    // pick's piece (a piece is fixed by its id: any copy will do).
    let beacon_out = |h: u64| witness::beacon_out(&i.keep.beacon_code, got, &i.keep.beacon, h);
    let audit = match epoch.checked_sub(1) {
        Some(e) if !listed.is_empty() => {
            let lot_out = match beacon_out(R::statement::lot_height(epoch)) {
                Err(w) => return Ok(Step::Want(vec![w])),
                Ok(Ok(o)) => o,
                Ok(Err(_)) if !(i.fresh)(&i.keep.beacon) => return Ok(Step::Want(vec![i.keep.beacon])),
                Ok(Err(why)) => return Err(format!("the statement's lot (height {}): {why}", R::statement::lot_height(epoch))),
            };
            let mut want: Vec<[u8; 32]> = Vec::new();
            for (_, key) in &listed {
                let Some(s) = record_of(key) else { continue };
                let (heights, pieces) = craftworks_standing::record::audit_reads(&s, key, e, &lot_out);
                for h in heights {
                    if let Err(w) = beacon_out(h) {
                        want.push(w);
                    }
                }
                want.extend(pieces.into_iter().filter(|id| got(id).is_none()));
            }
            want.sort();
            want.dedup();
            if !want.is_empty() {
                return Ok(Step::Want(want));
            }
            Some((e, lot_out))
        }
        _ => None,
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
    // KEPT, audited — last, once every read is in (the seals take seconds each: never redone for a later want).
    let kept = match audit {
        Some((e, lot_out)) => {
            let out = |h: u64| beacon_out(h).ok().and_then(|r| r.ok());
            let piece_of = |id: &[u8; 32]| got(id).filter(|b| !b.is_empty());
            craftworks_standing::record::kept_audited(&listed, &record_of, e, &lot_out, &out, &piece_of)
        }
        None => Vec::new(),
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

    /// AUDITS BY LOT (TOKEN §8 item 6): two listed keepers each answer KEEP_MIN sampled heights of epoch 3 — one with its
    /// pieces' real seals, one with JUNK hashes (it holds nothing). Both records hold (the contract cannot tell); the
    /// statement of epoch 4 draws an audit per keeper from its lot's output, asks for the drawn pieces, re-seals them:
    /// the honest keeper is KEPT, the fake is not. A drawn piece that cannot be read fails its keeper too.
    #[test]
    fn a_keeper_answering_junk_hashes_is_not_kept_an_honest_one_is() {
        use crate::keeper::{answer, asked, open_epoch, record_id};
        use craftworks_standing::{record, EPOCH};
        use ed25519_dalek::{Signer as _, SigningKey};
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let lot = Lot { pool: [1; 32], bag: [2; 32], beacon_code: [3; 32] };
        let keep = Keep { standing: [4; 32], beacon_code: [3; 32], beacon: [7; 32] };
        let out = |h: u64| *blake3::hash(&h.to_le_bytes()).as_bytes();
        let mut got: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
        // THE BEACON: every height of epoch 3, and the statement of epoch 4's lot.
        let steps = (3 * EPOCH..=R::statement::lot_height(4)).map(|h| craftworks_beacon_contract::Step { height: h, prev: [0; 32], mix: [0; 32], out: out(h), root: [0; 32], points: None }).collect();
        got.insert(keep.beacon, craftworks_beacon_contract::wire::encode(&craftworks_beacon_contract::State { steps, alts: vec![], frauds: vec![] }));
        let piece = |id: &[u8; 32]| [id.to_vec(), vec![7u8; 100]].concat();
        let hosted: Vec<([u8; 32], u64)> = (0..40u32).map(|i| (*blake3::hash(&i.to_le_bytes()).as_bytes(), 132)).collect();
        // Each keeper: an account (its key log), its node key bound by a credential, listed in the keepers registry —
        // and its record made by the keeper loop (`honest`: real seals; else junk hashes).
        let mut keepers = Vec::new();
        let mut dids = Vec::new();
        for (n, honest) in [(0u8, true), (1u8, false)] {
            let owner_seed = [60 + n; 32];
            let next = SigningKey::from_bytes(&[80 + n; 32]).verifying_key().to_bytes();
            let ev = craftworks_idlog_contract::Event::signed(0, [0; 32], &owner_seed, craftworks_idlog_contract::commit(&next), [1; 32], [1; 32], vec![]).unwrap();
            let log = craftworks_idlog_contract::Log { events: vec![ev] };
            let did = log.events[0].id();
            let key = SigningKey::from_bytes(&[100 + n; 32]);
            let public = key.verifying_key().to_bytes();
            got.insert(craftworks_token_contract::id(&ctx.idlog, &did), log.encode());
            keepers.push(R::keepers::Keeper { did, cred: craftworks_identity::credential(&did, &public, &[7; 32], &owner_seed), key_seq: 0 }.encode());
            dids.push(did);
            let rid = record_id(&keep, &public);
            got.insert(rid, vec![]);
            let signing = |step: &dyn Fn(&[(Vec<u8>, [u8; 64])]) -> Result<Step<Option<crate::keeper::Put>>, String>| {
                let mut sigs = Vec::new();
                loop {
                    match step(&sigs).unwrap() {
                        Step::Done(p) => return p,
                        Step::Sign(m) => sigs.push((m.clone(), key.sign(&m).to_bytes())),
                        Step::Want(w) => panic!("want {w:?}"),
                    }
                }
            };
            let g = got.clone();
            let p = signing(&|s| open_epoch(&keep, &|x| g.get(x).cloned(), &public, 3, &hosted, s)).expect("opened");
            got.insert(rid, p.state);
            let peer = craftworks_standing::keeper_peer(&public);
            let heights: Vec<u64> = (3 * EPOCH..4 * EPOCH).filter(|h| craftworks_standing::sampled(&out(*h), &peer)).take(record::KEEP_MIN).collect();
            assert_eq!(heights.len(), record::KEEP_MIN);
            for h in heights {
                let g = got.clone();
                let Step::Done(Some(ids)) = asked(&keep, &|x| g.get(x).cloned(), &public, h, &out(h), &hosted).unwrap() else { panic!("asked") };
                let hashes: Vec<([u8; 32], [u8; 32])> = ids.iter().map(|id| (*id, if honest { craftworks_standing::sealed_hash(&public, id, &piece(id), &out(h), &peer) } else { [0xEE; 32] })).collect();
                let p = signing(&|s| answer(&keep, &|x| g.get(x).cloned(), &public, h, &out(h), &hosted, &hashes, s)).expect("answered");
                got.insert(rid, p.state);
            }
            // The record HOLDS (as the standing contract checks it) and counts as kept BEFORE its audit.
            let rec = craftworks_standing::codec::decode(&got[&rid]).unwrap();
            assert_eq!(record::holds(&peer, &rec, &|x| Some(out(x))), Ok(()));
            assert!(record::kept(&rec, &public, 3), "kept before the audit (honest: {honest})");
        }
        let fresh = |_: &[u8; 32]| true;
        let lines = |_: &[u8; 32]| Ok(BTreeMap::new());
        let i = Inputs { epoch: 4, items: &[], keepers: &keepers, queued: &[], posted: &[], earlier: &[], ctx: &ctx, lot: &lot, keep: &keep, fresh: &fresh, lines: &lines };
        // The build ASKS for the drawn pieces (2 audits × 2 keepers), then for what the fees count reads (none here).
        let g = got.clone();
        let Ok(Step::Want(w)) = build(&i, &|x| g.get(x).cloned()) else { panic!("asks for the drawn pieces") };
        assert!(!w.is_empty() && w.len() <= 2 * record::AUDITS && w.iter().all(|id| hosted.iter().any(|x| x.0 == *id)), "{w:?}");
        let run = |got: &HashMap<[u8; 32], Vec<u8>>, piece_there: bool| {
            let mut g = got.clone();
            for _ in 0..8 {
                let gg = g.clone();
                match build(&i, &|x| gg.get(x).cloned()).unwrap() {
                    Step::Done((st, kept)) => return (st, kept),
                    Step::Want(w) => w.into_iter().for_each(|id| {
                        let state = if piece_there && hosted.iter().any(|x| x.0 == id) { piece(&id) } else { vec![] };
                        g.entry(id).or_insert(state);
                    }),
                    Step::Sign(_) => panic!("signs"),
                }
            }
            panic!("no end")
        };
        let (_, kept) = run(&got, true);
        assert_eq!(kept, 1, "the honest keeper alone");
        // The same through the tools' rule: which account.
        let log_of = |d: &R::Did| got.get(&craftworks_token_contract::id(&ctx.idlog, d)).and_then(|b| craftworks_idlog_contract::read(d, b));
        let listed = R::keepers::listed(&keepers, &log_of);
        let record_of = |key: &[u8; 32]| got.get(&record_id(&keep, key)).and_then(|b| craftworks_standing::codec::decode(b));
        let lot_out = out(R::statement::lot_height(4));
        assert_eq!(record::kept_audited(&listed, &record_of, 3, &lot_out, &|h| Some(out(h)), &|id| Some(piece(id))), vec![dids[0]]);
        // A drawn piece the network cannot give: FAILED (the keeper claimed it).
        let (_, kept) = run(&got, false);
        assert_eq!(kept, 0, "a drawn piece not readable fails its keeper");
    }
}
