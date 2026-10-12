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
    /// The MISDEED REGISTRY's items of every epoch BEFORE this one, each with the epoch of the bag it was in
    /// (`misdeed::registry_address`): proofs that hold zero their account and charge its direct vouchers.
    pub misdeeds: &'a [(u64, Vec<u8>)],
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
    /// THE SEALS SPREAD OVER CALLS (the identity delegate: an audit re-seals pieces, seconds each, more than one wake-up
    /// takes): for a kept keeper's record key and record, `audit(key, record, seal)` gives its verdict — sealed earlier,
    /// or `seal()` run now — or `None`: not sealed yet, and the build PAUSES (`Step::Want(vec![])`: nothing to read, call
    /// again). `None` here: every audit sealed in this call (the page's).
    pub audit: Option<&'a dyn Fn(&[u8; 32], &craftworks_standing::record::State, &dyn Fn() -> bool) -> Option<bool>>,
}

/// The statement of `i.epoch` and how many accounts kept the epoch before — `Want` what is not read yet.
pub fn build(i: &Inputs, got: Got) -> Result<Step<(R::statement::Statement, usize)>, String> {
    let epoch = i.epoch;
    // Every voucher's and listed keeper's KEY LOG.
    let log_id = |d: &R::Did| craftworks_token_contract::id(&i.ctx.idlog, d);
    let accounts = i.items.iter().map(|it| it.from).chain(i.keepers.iter().filter_map(|b| R::keepers::Keeper::decode(b)).map(|k| k.did)).chain(crate::misdeed::accused(i.misdeeds));
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
    // Their SIGNATURES read FRESH for this build, as keepers' records are (10-11, measured on fx: a page that read epoch
    // 24's signatures before its witness signed kept that copy, found it not accepted, carried nothing, and posted a root
    // Wren's tool — reading fresh — did not build, for epochs 25 and 26). Statement blocks are content-addressed: any
    // copy is the copy.
    // A read REFUSED as not fresh is a request to read it, never an absence (10-12, measured on fx: a page that held
    // the epoch before's signatures from an earlier pass read them as missing, found no accepted statement and carried
    // no lines — a root no witness rebuilt): each one refused is asked for (`Want`) before anything is decided.
    let held = got;
    let refused = std::cell::RefCell::new(Vec::<[u8; 32]>::new());
    let fresh_got = |id: &[u8; 32]| {
        if (i.fresh)(id) {
            held(id)
        } else {
            refused.borrow_mut().push(*id);
            None
        }
    };
    let got: Got = &fresh_got;
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
    {
        let mut asked = refused.borrow().clone();
        if !asked.is_empty() {
            asked.sort();
            asked.dedup();
            return Ok(Step::Want(asked));
        }
    }
    // KEPT, audited — last, once every read is in (the seals take seconds each: never redone for a later want).
    let kept = match audit {
        Some((e, lot_out)) => {
            let out = |h: u64| beacon_out(h).ok().and_then(|r| r.ok());
            // A PIECE is content-addressed: any copy held is the piece (never "not fresh": a page that held it from an
            // earlier pass found every keeper's audit failing, 10-12).
            let piece_of = |id: &[u8; 32]| held(id).filter(|b| !b.is_empty());
            let seal = |key: &[u8; 32], s: &craftworks_standing::record::State| craftworks_standing::record::audited(s, key, e, &lot_out, &out, &piece_of);
            match i.audit {
                None => craftworks_standing::record::kept_audited_by(&listed, &record_of, e, &seal),
                Some(audit) => {
                    let paused = std::cell::Cell::new(false);
                    let kept = craftworks_standing::record::kept_audited_by(&listed, &record_of, e, &|key, s| {
                        audit(key, s, &|| seal(key, s)).unwrap_or_else(|| {
                            paused.set(true);
                            false
                        })
                    });
                    if paused.get() {
                        return Ok(Step::Want(Vec::new()));
                    }
                    kept
                }
            }
        }
        None => Vec::new(),
    };
    let pots = R::statement::Pots { keeper: craftworks_wallet::craft::keeper_pot(epoch), allowance: (craftworks_allowance_contract::POT, craftworks_allowance_contract::CAP), witness: craftworks_wallet::craft::witness_pot(epoch), halvings: craftworks_wallet::craft::halvings(epoch) };
    // MISDEEDS proven in the registries before this epoch (each proof checked against its account's key log).
    let proven = crate::misdeed::proven(i.misdeeds, &log_of);
    Ok(Step::Done((R::statement::build(epoch, i.items, &log_of, &kept, &signed, &prev, pots, &proven), kept.len())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The build asks for each voucher's key log first, and builds once it is read (as the page and the delegate run it).
    #[test]
    fn a_statement_asks_for_its_vouchers_key_logs_then_builds() {
        let ctx = Ctx { code_hash: [9; 32], beacon: [7; 32], idlog: [6; 32] };
        let lot = Lot { pool: [1; 32], bag: [2; 32], beacon_code: [3; 32], idlog: [6; 32] };
        let keep = Keep { standing: [4; 32], beacon_code: [3; 32], beacon: [7; 32] };
        let items = vec![R::item::Item { from: [11; 32], to: [12; 32], at: 5, on: true, cred: vec![], key_seq: 0, sig: [0; 64] }];
        let none = |_: &[u8; 32]| false;
        let lines = |_: &[u8; 32]| Ok(BTreeMap::new());
        let i = Inputs { epoch: 0, items: &items, misdeeds: &[], keepers: &[], queued: &[], posted: &[], earlier: &[], ctx: &ctx, lot: &lot, keep: &keep, fresh: &none, lines: &lines, audit: None };
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
        let lot = Lot { pool: [1; 32], bag: [2; 32], beacon_code: [3; 32], idlog: [6; 32] };
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
        let i = Inputs { epoch: 4, items: &[], misdeeds: &[], keepers: &keepers, queued: &[], posted: &[], earlier: &[], ctx: &ctx, lot: &lot, keep: &keep, fresh: &fresh, lines: &lines, audit: None };
        // The build ASKS for the drawn pieces (2 audits × 2 keepers), then for what the fees count reads (none here).
        let g = got.clone();
        let Ok(Step::Want(w)) = build(&i, &|x| g.get(x).cloned()) else { panic!("asks for the drawn pieces") };
        assert!(!w.is_empty() && w.len() <= 2 * record::AUDITS && w.iter().all(|id| hosted.iter().any(|x| x.0 == *id)), "{w:?}");
        let run_with = |i: &Inputs, got: &HashMap<[u8; 32], Vec<u8>>, piece_there: bool| {
            let mut g = got.clone();
            for _ in 0..8 {
                let gg = g.clone();
                match build(i, &|x| gg.get(x).cloned()).unwrap() {
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
        let run = |got: &HashMap<[u8; 32], Vec<u8>>, piece_there: bool| run_with(&i, got, piece_there);
        let (honest, kept) = run(&got, true);
        assert_eq!(kept, 1, "the honest keeper alone");
        // THE PAGE's LATER PASS (10-12, measured on fx): every piece already held from the pass before, and only what
        // this build asks for read FRESH — the same keepers kept, the same root as a build reading everything fresh.
        let mut held = got.clone();
        for (id, _) in &hosted {
            held.insert(*id, piece(id));
        }
        let asked = std::cell::RefCell::new(std::collections::HashSet::<[u8; 32]>::new());
        let fresh_now = |id: &[u8; 32]| asked.borrow().contains(id);
        let later = Inputs { fresh: &fresh_now, ..i };
        let mut ended = None;
        for _ in 0..8 {
            let h = held.clone();
            match build(&later, &|x| h.get(x).cloned()).unwrap() {
                Step::Done(d) => {
                    ended = Some(d);
                    break;
                }
                Step::Want(w) => w.into_iter().for_each(|id| {
                    asked.borrow_mut().insert(id);
                    held.entry(id).or_insert_with(Vec::new);
                }),
                Step::Sign(_) => panic!("signs"),
            }
        }
        let (later_st, later_kept) = ended.expect("the later pass ends");
        println!("later pass (pieces held, fresh only what it asked): kept {later_kept}");
        assert_eq!((&later_st, later_kept), (&honest, 1), "a later pass holding the pieces builds the same root");
        // THE MISDEED REGISTRY (TOKEN §7d decision 3) through the one Inputs path: the honest keeper's device key — the
        // credential its owner gave it — signed two entries at one place, proven in epoch 3's registry. Epoch 4's
        // statement then pays it nothing (bootstrap would: it kept) and lists it at reputation 0; a proof that does not
        // hold (another's signature as the second) and junk are ignored; the items' order does not matter.
        let (k0, pk0) = (SigningKey::from_bytes(&[100; 32]), SigningKey::from_bytes(&[100; 32]).verifying_key().to_bytes());
        let wsig = |k: &SigningKey, entry: u8| {
            let m = craftworks_token_contract::witness::WitnessSig::message(&[9; 32], 4, &[entry; 32], 4 * EPOCH);
            craftworks_token_contract::witness::WitnessSig { chain: [9; 32], seq: 4, entry: [entry; 32], at: 4 * EPOCH, sig: k.sign(&m).to_bytes() }
        };
        let proof = crate::misdeed::Equivocation { did: dids[0], cred: craftworks_identity::credential(&dids[0], &pk0, &[7; 32], &[60; 32]), key_seq: 0, key: pk0, a: wsig(&k0, 1), b: wsig(&k0, 2) };
        let forged = crate::misdeed::Equivocation { b: wsig(&SigningKey::from_bytes(&[101; 32]), 2), ..proof.clone() };
        assert!(honest.lines[&dids[0]].keeper > 0, "the control: paid without its misdeed");
        let with = |misdeeds: &[(u64, Vec<u8>)]| run_with(&Inputs { misdeeds, ..i }, &got, true).0;
        let ignored = with(&[(3, forged.encode()), (2, b"junk".to_vec())]);
        assert_eq!(ignored, honest, "an item that does not prove is ignored");
        let cheat = with(&[(3, forged.encode()), (3, proof.encode()), (2, b"junk".to_vec())]);
        println!("keeper share: {} without its misdeed, {} with it", honest.lines[&dids[0]].keeper, cheat.lines[&dids[0]].keeper);
        assert_eq!((cheat.lines[&dids[0]].keeper, cheat.lines[&dids[0]].rep), (0, 0), "a proven cheat earns nothing");
        assert_eq!(with(&[(2, b"junk".to_vec()), (3, proof.encode()), (3, forged.encode())]), cheat, "order-independent");
        assert_eq!(with(&[(4, proof.encode())]), honest, "a proof in this epoch's own registry counts from the next");
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
