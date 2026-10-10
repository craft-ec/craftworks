use super::*;
use ed25519_dalek::{Signer as _, SigningKey};

const CODE: [u8; 32] = [9; 32];
const BEACON: [u8; 32] = [7; 32];
const IDLOG: [u8; 32] = [6; 32];

fn owner_seed(n: u8) -> [u8; 32] {
    [n.wrapping_add(100); 32]
}
fn next_seed(n: u8) -> [u8; 32] {
    [n.wrapping_add(150); 32]
}
fn did(n: u8) -> [u8; 32] {
    log(n).events[0].id()
}
fn log(n: u8) -> craftworks_idlog_contract::Log {
    let next = SigningKey::from_bytes(&next_seed(n)).verifying_key().to_bytes();
    let e = craftworks_idlog_contract::Event::signed(0, [0; 32], &owner_seed(n), craftworks_idlog_contract::commit(&next), [n; 32], [n; 32], vec![]).unwrap();
    craftworks_idlog_contract::Log { events: vec![e] }
}
/// Person `n`'s device `dev` (its node key), credentialed by n's owner key at seq 0.
struct Dev {
    n: u8,
    key: SigningKey,
}
impl Dev {
    fn new(n: u8, dev: u8) -> Dev {
        Dev { n, key: SigningKey::from_bytes(&[dev; 32]) }
    }
}
impl Device for Dev {
    fn public(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }
    fn sign(&self, m: &[u8]) -> [u8; 64] {
        self.key.sign(m).to_bytes()
    }
    fn credential(&self) -> Vec<u8> {
        craftworks_identity::credential(&did(self.n), &self.public(), &[], &owner_seed(self.n))
    }
    fn key_seq(&self) -> u32 {
        0
    }
}
fn def() -> Definition {
    Definition { code_hash: CODE, decimals: 0, min_send: 1, genesis_owner: did(1), genesis_amount: 1000, name: b"W".to_vec(), nonce: [0; 16], beacon: BEACON, margin: 2, idlog: IDLOG, mint: None, freeze: None, ids: t::Ids::None, royalty: None, cap: 0, item: [0; 32], version: [0; 32], soulbound: false, transfer_fee: None, sale: None }
}
fn held(n: u8, d: &Definition) -> Held {
    Held { params: ChainParams { code_hash: CODE, definition: t::id(&CODE, &d.encode()), owner: did(n), beacon: BEACON, idlog: IDLOG, item: [0; 32] }, definition: d.clone(), state: State::default(), blocks: Default::default() }
}
/// 1's chain with its genesis, written by device `dev`.
fn genesis(dev: &Dev, d: &Definition) -> Held {
    let lg = log(1);
    let w = Wallet { device: dev, keys: &lg };
    let mut h = held(1, d);
    let s = &mut h.state;
    let first = Entry { seq: 0, prev: [0; 32], nonce: nonce_of(&h.params, 0), stamp: 0, body: Body::Writer { cred: dev.credential(), key_seq: 0 }, balance: 0, auth: t::Auth::Work(0) };
    let first = Entry { auth: t::Auth::Sig(dev.sign(&first.message())), ..first };
    s.entries.push(first);
    let g = Entry { seq: 1, prev: s.head(), nonce: nonce_of(&h.params, 1), stamp: 0, body: Body::Genesis { amount: 1000 }, balance: 1000, auth: t::Auth::Work(0) };
    let g = Entry { auth: t::Auth::Sig(dev.sign(&g.message())), ..g };
    s.entries.push(g);
    let _ = w;
    h
}

#[test]
fn a_debit_is_the_chains_next_entry_signed_by_this_device_and_checked() {
    let dev = Dev::new(1, 11);
    let lg = log(1);
    let w = Wallet { device: &dev, keys: &lg };
    let h = genesis(&dev, &def());
    let out = w.debit(&h, Body::Send { to: did(2), amount: 300 }, 5).unwrap();
    assert_eq!(out.len(), 1, "this device writes the chain already");
    let e = &out[0].clone();
    assert_eq!((e.seq, e.balance, e.stamp, e.prev), (2, 700, 5, h.state.head()));
    assert!(w.signer().signs(e));
    let mut s = h.state.clone();
    s.entries.extend(out.clone());
    assert_eq!(t::check(&h.params, &h.definition, &s, &|_, _| None, &|_| None, Some(&lg), &t::Extra::NONE), Ok(()));
    // Refused as the contract refuses: over the balance; under the minimum; to itself.
    assert_eq!(w.debit(&h, Body::Send { to: did(2), amount: 1001 }, 5).err(), Some("a debit over the balance"));
    assert_eq!(w.debit(&h, Body::Send { to: did(1), amount: 10 }, 5).err(), Some("a send to itself"));
    assert_eq!(w.debit(&h, Body::Receive { from: did(2), nonce: [0; 16], amount: 1, cited: 0, ev: t::Evidence::default() }, 5).err(), Some("not a debit this wallet builds"));
    // A transfer fee on top (1%, to 9): 300 takes 303.
    let fd = Definition { transfer_fee: Some((100, 0, did(9))), ..def() };
    let hf = genesis(&dev, &fd);
    assert_eq!(w.debit(&hf, Body::Send { to: did(2), amount: 300 }, 5).unwrap()[0].balance, 697);
    // Soulbound: refused, as the contract refuses.
    let sb = Definition { soulbound: true, ..def() };
    assert_eq!(w.debit(&genesis(&dev, &sb), Body::Send { to: did(2), amount: 1 }, 5).err(), Some("a transfer of a token that cannot be transferred"));
    // Two debits: each named by its place (never one nonce twice).
    let mut h2 = h.clone();
    h2.state.entries.extend(out);
    let next = w.debit(&h2, Body::Escrow { program: [5; 32], terms: vec![], amount: 100, until: 0 }, 6).unwrap();
    assert_ne!(next[0].nonce, e.nonce);
    assert_eq!(next[0].balance, 600);
    // A beacon read lagging behind the chain's last stamp (another node's view): never stamped below it.
    assert_eq!(w.debit(&h2, Body::Burn { amount: 1 }, 2).unwrap()[0].stamp, 5);
}

#[test]
fn another_device_of_the_account_takes_the_chain_with_its_writer_entry() {
    let (first, second) = (Dev::new(1, 11), Dev::new(1, 12));
    let lg = log(1);
    let h = genesis(&first, &def());
    let w2 = Wallet { device: &second, keys: &lg };
    let out = w2.debit(&h, Body::Burn { amount: 10 }, 5).unwrap();
    assert_eq!(out.len(), 2, "its writer entry, then the burn");
    assert!(matches!(out[0].body, Body::Writer { .. }));
    let mut s = h.state.clone();
    s.entries.extend(out);
    assert_eq!(t::check(&h.params, &h.definition, &s, &|_, _| None, &|_| None, Some(&lg), &t::Extra::NONE), Ok(()));
    assert_eq!(t::writer_at(&s, s.entries.len()), Some(w2.signer()));
    // Another ACCOUNT's device cannot: its credential is not this owner's.
    let stranger = Dev::new(2, 13);
    let w3 = Wallet { device: &stranger, keys: &lg };
    assert_eq!(w3.debit(&h, Body::Burn { amount: 10 }, 5).err(), Some("a writer credential for another account"));
}

/// Apply `es` to `h`'s chain.
fn with(h: &Held, es: Vec<Entry>) -> Held {
    let mut x = h.clone();
    x.state.entries.extend(es);
    x
}

#[test]
fn a_wallet_collects_receives_vesting_steps_and_fees_only_once_final() {
    // F: 1% fee to 9. 1 sends 2 300 and vests it 50 at 4 and 50 at 20 (stamp 1). Margin 2.
    let fd = Definition { transfer_fee: Some((100, 0, did(9))), ..def() };
    let d1 = Dev::new(1, 11);
    let l1 = log(1);
    let w1 = Wallet { device: &d1, keys: &l1 };
    let mut a = genesis(&d1, &fd);
    a = with(&a, w1.debit(&a, Body::Send { to: did(2), amount: 300 }, 1).unwrap());
    a = with(&a, w1.debit(&a, Body::Vest { to: did(2), schedule: vec![(4, 50), (20, 50)] }, 1).unwrap());
    let chains = [(did(1), a.state.clone())];
    let src = Sources { blocks: &Default::default(), chains: &chains, programs: &[], shards: &[] };
    let (d2, l2) = (Dev::new(2, 22), log(2));
    let w2 = Wallet { device: &d2, keys: &l2 };
    let b = held(2, &fd);
    assert!(w2.collect(&b, &src, 2).unwrap().is_empty(), "not final yet (1 + 2 > 2)");
    let got = w2.collect(&b, &src, 5).unwrap();
    assert_eq!(got.iter().map(|e| e.balance).collect::<Vec<_>>(), vec![300, 350], "the receive, then step 0 (at 4); step 1 not yet");
    let b = with(&b, got);
    let read = |_: &t::Tok, o: &[u8; 32]| (*o == did(1)).then(|| a.state.clone());
    assert_eq!(t::check(&b.params, &fd, &b.state, &read, &|_| None, Some(&l2), &t::Extra::NONE), Ok(()), "the contract takes them");
    assert!(w2.collect(&b, &src, 5).unwrap().is_empty(), "never twice");
    let later = w2.collect(&b, &src, 25).unwrap();
    assert_eq!(later.len(), 1);
    assert!(matches!(later[0].body, Body::Vested { step: 1, .. }));
    // The COLLECTOR (9) claims the fees: 3 on the send, 1 on the vest of 100.
    let (d9, l9) = (Dev::new(9, 29), log(9));
    let w9 = Wallet { device: &d9, keys: &l9 };
    let c = held(9, &fd);
    let fees = w9.collect(&c, &src, 5).unwrap();
    assert_eq!(fees.iter().map(|e| match e.body { Body::FeeClaim { amount, .. } => amount, _ => 0 }).collect::<Vec<_>>(), vec![3, 1]);
    let c = with(&c, fees);
    assert_eq!(t::check(&c.params, &fd, &c.state, &read, &|_| None, Some(&l9), &t::Extra::NONE), Ok(()));
    // Someone else (8) is owed nothing.
    let (d8, l8) = (Dev::new(8, 28), log(8));
    assert!(Wallet { device: &d8, keys: &l8 }.collect(&held(8, &fd), &src, 25).unwrap().is_empty());
}

#[test]
fn a_wallet_collects_releases_payouts_and_reclaims_as_programs_decide() {
    const PROG: [u8; 32] = [77; 32];
    const GONE: [u8; 32] = [78; 32];
    let d1 = Dev::new(1, 11);
    let l1 = log(1);
    let w1 = Wallet { device: &d1, keys: &l1 };
    let mut a = genesis(&d1, &def());
    a = with(&a, w1.debit(&a, Body::Escrow { program: PROG, terms: b"t".to_vec(), amount: 300, until: 10 }, 1).unwrap());
    let e1 = a.state.entries.last().unwrap().nonce;
    a = with(&a, w1.debit(&a, Body::Escrow { program: GONE, terms: vec![], amount: 200, until: 10 }, 1).unwrap());
    // PROG's verdict: 50 back to 1, 250 to 2. GONE never existed (a wrong id).
    let sec = t::escrow::Section { escrows: vec![((a.params.tok(), did(1), e1), 300, t::escrow::terms_hash(b"t"))], refunds: vec![((a.params.tok(), did(1), e1), 50)], pays: vec![([1; 16], a.params.tok(), did(2), 250)] };
    let programs = [(PROG, sec.encode())];
    let src = Sources { blocks: &Default::default(), chains: &[], programs: &programs, shards: &[] };
    let before = w1.collect(&a, &src, 5).unwrap();
    assert_eq!(before.len(), 1, "the release; the reclaim not before its time");
    assert!(matches!(before[0].body, Body::Unescrow { refund: 50, verdict, .. } if verdict == sec.hash()));
    let at = w1.collect(&a, &src, 10).unwrap();
    assert_eq!(at.len(), 2, "the release, and the escrow to no program reclaimed");
    assert!(matches!(at[1].body, Body::Reclaim { .. }));
    // A program that LISTS the escrow, no verdict yet: it took it — never reclaimed, whatever the time.
    let listing = t::escrow::Section { escrows: vec![((a.params.tok(), did(1), e1), 300, t::escrow::terms_hash(b"t"))], ..Default::default() };
    let listed = [(PROG, listing.encode())];
    let pending = w1.collect(&a, &Sources { blocks: &Default::default(), chains: &[], programs: &listed, shards: &[] }, 50).unwrap();
    assert_eq!(pending.len(), 1, "only GONE's escrow comes back");
    assert!(matches!(pending[0].body, Body::Reclaim { escrow } if escrow != e1));
    let a2 = with(&a, at);
    assert_eq!(a2.state.balance(), 1000 - 300 - 200 + 50 + 200);
    let prog = |id: &[u8; 32]| (*id == PROG).then(|| sec.encode());
    assert_eq!(t::check(&a2.params, &def(), &a2.state, &|_, _| None, &|_| None, Some(&l1), &t::Extra { ops: None, program: Some(&prog) }), Ok(()));
    // 2 is paid its 250 under the verdict, once.
    let (d2, l2) = (Dev::new(2, 22), log(2));
    let w2 = Wallet { device: &d2, keys: &l2 };
    let b = held(2, &def());
    let paid = w2.collect(&b, &src, 5).unwrap();
    assert!(matches!(paid[..], [Entry { body: Body::Payout { amount: 250, .. }, .. }]));
    let b = with(&b, paid);
    assert_eq!(t::check(&b.params, &def(), &b.state, &|_, _| None, &|_| None, Some(&l2), &t::Extra { ops: None, program: Some(&prog) }), Ok(()));
    assert!(w2.collect(&b, &src, 5).unwrap().is_empty());
    // STAKE: unstaked, released once its delay is over.
    let mut s = genesis(&d1, &def());
    s = with(&s, w1.debit(&s, Body::Stake { amount: 100 }, 1).unwrap());
    s = with(&s, w1.debit(&s, Body::Unstake { amount: 100 }, 1).unwrap());
    let none = Sources { blocks: &Default::default(), chains: &[], programs: &[], shards: &[] };
    assert!(w1.collect(&s, &none, t::UNBOND).unwrap().is_empty(), "not yet");
    let rel = w1.collect(&s, &none, 1 + t::UNBOND).unwrap();
    assert!(matches!(rel[..], [Entry { body: Body::Release { .. }, balance: 1000, .. }]));
    assert!(w1.collect(&with(&s, rel), &none, 1 + t::UNBOND).unwrap().is_empty(), "released once");
}

#[test]
fn a_wallet_collects_its_mints_and_its_numbered_copies() {
    let set = t::authority::KeySet { k: 1, keys: vec![SigningKey::from_bytes(&[60; 32]).verifying_key().to_bytes()] };
    let md = Definition { mint: Some(set.clone()), genesis_amount: 0, ..def() };
    let mid = t::id(&CODE, &md.encode());
    let mint = |d: &[u8; 32], to: u8, nonce: [u8; 16], amount: u128, stamp: u64| {
        let mut m = t::supply::Mint { to: did(to), item: [0; 32], amount, nonce, stamp, sigs: vec![], paid: None };
        m.sigs = vec![(0, SigningKey::from_bytes(&[60; 32]).sign(&m.message(d)).to_bytes())];
        m
    };
    let shard = t::supply::Shard { mints: vec![mint(&mid, 2, [1; 16], 500, 3), mint(&mid, 3, [2; 16], 70, 3)] };
    let shards = [shard.clone()];
    let src = Sources { blocks: &Default::default(), chains: &[], programs: &[], shards: &shards };
    let (d2, l2) = (Dev::new(2, 22), log(2));
    let w2 = Wallet { device: &d2, keys: &l2 };
    let b = held(2, &md);
    let got = w2.collect(&b, &src, 5).unwrap();
    assert!(matches!(got[..], [Entry { body: Body::Mint { amount: 500, .. }, .. }]), "its mint, not 3's");
    let b = with(&b, got);
    assert_eq!(t::check(&b.params, &md, &b.state, &|_, _| None, &|e| (e == 0).then(|| shard.clone()), Some(&l2), &t::Extra::NONE), Ok(()));
    assert!(w2.collect(&b, &src, 5).unwrap().is_empty());
    // A NUMBERED edition: copies 1–3 minted to 2 in one range — 2's chain of copy #2 takes copy #2.
    let nd = Definition { ids: t::Ids::Numbered, cap: 10, genesis_amount: 0, mint: Some(set), ..def() };
    let nid = t::id(&CODE, &nd.encode());
    let rec = t::supply::Shard { mints: vec![mint(&nid, 2, 1u128.to_be_bytes(), 3, 3)] };
    let recs = [rec.clone()];
    let copy2 = Held { params: ChainParams { item: t::numbered_item(&nid, 2), ..held(2, &nd).params }, definition: nd.clone(), state: State::default(), blocks: Default::default() };
    let got = w2.collect(&copy2, &Sources { blocks: &Default::default(), chains: &[], programs: &[], shards: &recs }, 5).unwrap();
    assert!(matches!(&got[..], [Entry { body: Body::CappedMint { nonce, amount: 1, .. }, .. }] if *nonce == 2u128.to_be_bytes()));
    let c = with(&copy2, got);
    assert_eq!(t::check(&c.params, &nd, &c.state, &|_, _| None, &|k| (k == 0).then(|| rec.clone()), Some(&l2), &t::Extra::NONE), Ok(()));
}

/// `h` after its cut `(es, blocks)`: appended, then pruned to the cut, its blocks kept.
fn after_cut(h: &Held, (es, blocks): (Vec<Entry>, t::cut::Blocks)) -> Held {
    let mut x = with(h, es);
    x.state = t::prune(&x.state);
    x.blocks = blocks;
    x
}

#[test]
fn a_wallet_cuts_its_chain_and_collects_across_cuts_with_their_proofs() {
    const PROG: [u8; 32] = [77; 32];
    let (d1, l1) = (Dev::new(1, 11), log(1));
    let w1 = Wallet { device: &d1, keys: &l1 };
    let mut a = genesis(&d1, &def());
    a = with(&a, w1.debit(&a, Body::Send { to: did(2), amount: 300 }, 1).unwrap());
    // 1 CUTS (stamp 5): its send to 2 is now behind the cut.
    let a_cut = after_cut(&a, w1.cut(&a, 5).unwrap());
    assert_eq!(a_cut.state.entries.len(), 1, "only the cut is live");
    assert_eq!(a_cut.whole().unwrap().entries.len(), a.state.entries.len() + 1, "its whole history read back from its blocks");
    // 2 receives it from 1's HISTORY (the cut's blocks), with the proof the contract asks.
    let (d2, l2) = (Dev::new(2, 22), log(2));
    let w2 = Wallet { device: &d2, keys: &l2 };
    let chains = [(did(1), a_cut.state.clone())];
    let none = t::cut::Blocks::new();
    assert!(w2.collect(&held(2, &def()), &Sources { blocks: &none, chains: &chains, programs: &[], shards: &[] }, 10).unwrap().is_empty(), "its history not read: nothing seen");
    let src = Sources { blocks: &a_cut.blocks, chains: &chains, programs: &[], shards: &[] };
    let got = w2.collect(&held(2, &def()), &src, 10).unwrap();
    assert!(matches!(&got[..], [Entry { body: Body::Receive { amount: 300, ev, .. }, .. }] if ev.theirs.len() == 1));
    let b = with(&held(2, &def()), got);
    let read1 = |_: &t::Tok, o: &[u8; 32]| (*o == did(1)).then(|| a_cut.state.clone());
    assert_eq!(t::check(&b.params, &def(), &b.state, &read1, &|_| None, Some(&l2), &t::Extra::NONE), Ok(()));
    // 2 CUTS too (stamp 12) — then a payout to it, and 1's send again: the payout with its ABSENCE proof against 2's
    // received set; the send, already received behind 2's cut, never again.
    let b_cut = after_cut(&b, w2.cut(&b, 12).unwrap());
    let sec = t::escrow::Section { escrows: vec![((b.params.tok(), did(1), [7; 16]), 100, [0; 32])], refunds: vec![], pays: vec![([1; 16], b.params.tok(), did(2), 100)] };
    let programs = [(PROG, sec.encode())];
    let src = Sources { blocks: &a_cut.blocks, chains: &chains, programs: &programs, shards: &[] };
    let got = w2.collect(&b_cut, &src, 13).unwrap();
    assert!(matches!(&got[..], [Entry { body: Body::Payout { amount: 100, ev, .. }, .. }] if !ev.mine.is_empty()), "the payout only, with its proof");
    let b2 = with(&b_cut, got);
    let prog = |id: &[u8; 32]| (*id == PROG).then(|| sec.encode());
    assert_eq!(t::check(&b2.params, &def(), &b2.state, &read1, &|_| None, Some(&l2), &t::Extra { ops: None, program: Some(&prog) }), Ok(()));
    // A SECOND cut of 2's chain: its new blocks alone read the whole history back (the first cut's are needed no more).
    let b3 = after_cut(&b2, w2.cut(&b2, 14).unwrap());
    assert_eq!(b3.whole().unwrap().entries.len(), b2.whole().unwrap().entries.len() + 1);
    // A cut over an open escrow: refused, as the contract refuses.
    let mut e = genesis(&d1, &def());
    e = with(&e, w1.debit(&e, Body::Escrow { program: PROG, terms: vec![], amount: 10, until: 0 }, 1).unwrap());
    assert_eq!(w1.cut(&e, 5).err(), Some("a cut over an open escrow"));
}

#[test]
fn a_wallet_starts_its_token_and_credits_a_payment_only_once_witnessed() {
    // GENESIS: this device's writer entry, then the definition's amount.
    let (d1, l1) = (Dev::new(1, 11), log(1));
    let w1 = Wallet { device: &d1, keys: &l1 };
    let a0 = held(1, &def());
    let g = w1.genesis(&a0, 0).unwrap();
    let a = with(&a0, g);
    assert_eq!(a.state.balance(), 1000);
    assert_eq!(t::check(&a.params, &def(), &a.state, &|_, _| None, &|_| None, Some(&l1), &t::Extra::NONE), Ok(()));
    assert_eq!(w1.genesis(&a, 0).err(), Some("a genesis of a chain begun"));
    assert_eq!(Wallet { device: &Dev::new(2, 22), keys: &log(2) }.genesis(&held(2, &def()), 0).err(), Some("a genesis that is not the definition's"), "not the definition's owner");
    // 1 pays 2 300: VALUE only once its committee (drawn by lot from 20 stakers, 100 each) signed it.
    let a = with(&a, w1.debit(&a, Body::Send { to: did(2), amount: 300 }, 1).unwrap());
    let send = a.state.entries.last().unwrap().clone();
    let pool = [40; 32];
    let stakers: Vec<t::draw::StakeClaim> = (60..80u8)
        .map(|n| {
            let k = SigningKey::from_bytes(&[n; 32]);
            let kb = k.verifying_key().to_bytes();
            t::draw::StakeClaim { owner: [n; 32], amount: 100, epoch: 0, key: kb, sig: k.sign(&t::draw::StakeClaim::message(&pool, &[n; 32], 100, 0, &kb)).to_bytes() }
        })
        .collect();
    let out = |h: u64| *blake3::hash(&h.to_le_bytes()).as_bytes();
    let pred = &a.state.entries[a.state.entries.len() - 2];
    let (epoch, h) = t::wallet::lot_of(&send, Some(pred), def().margin).unwrap();
    let members = t::draw::committee(&pool, &out(h), &a.params.id(), send.seq, epoch, &stakers, t::wallet::COMMITTEE);
    let sign = |n: usize| {
        members.iter().take(n).map(|m| {
            let k = SigningKey::from_bytes(&[stakers[*m].owner[0]; 32]);
            t::witness::WitnessSig { chain: a.params.id(), seq: send.seq, entry: send.hash(), sig: k.sign(&t::witness::WitnessSig::message(&a.params.id(), send.seq, &send.hash())).to_bytes() }
        }).collect::<Vec<_>>()
    };
    let check_with = |sigs: Vec<t::witness::WitnessSig>| {
        let d = def();
        let src = t::wallet::Source {
            pool,
            claims: &|e| if e == 0 { stakers.clone() } else { vec![] },
            sigs: &|_, _| sigs.clone(),
            out: &|x| Some(out(x)),
            stake_of: &|o, at| (at == 0).then(|| (100, vec![SigningKey::from_bytes(&[o[0]; 32]).verifying_key().to_bytes()])),
            definition: &|_| Some(d.clone()),
            program: &|_| None,
            freeze: &|_| None,
            keys: &|_| None,
            history: &|_, _| None,
        };
        is_value(&src, &a.params, &a.state, &send.nonce)
    };
    assert_eq!(check_with(vec![]), Err("an entry its committee has not witnessed"));
    assert_eq!(check_with(sign(t::wallet::NEED - 1)), Err("an entry its committee has not witnessed"), "one short");
    assert_eq!(check_with(sign(t::wallet::NEED)), Ok(t::wallet::NEED));
    assert_eq!(is_value(&t::wallet::Source { pool, claims: &|_| vec![], sigs: &|_, _| vec![], out: &|_| None, stake_of: &|_, _| None, definition: &|_| None, program: &|_| None, freeze: &|_| None, keys: &|_| None, history: &|_, _| None }, &a.params, &a.state, &[0; 16]), Err("a payment whose debit is not there"));
}

#[test]
fn a_wallet_buys_a_copy_of_an_edition_sold_on_demand() {
    // S: numbered copies (cap 10) sold for 10 W each to 9 — no mint authority at all. 1 holds 1000 W.
    let dev = Dev::new(1, 11);
    let lg = log(1);
    let w = Wallet { device: &dev, keys: &lg };
    let wid = t::id(&CODE, &def().encode());
    let sd = Definition { ids: t::Ids::Numbered, cap: 10, genesis_amount: 0, name: b"S".to_vec(), idlog: IDLOG, sale: Some((t::Tok { definition: wid, item: [0; 32] }, 10, did(9))), ..def() };
    let sid = t::id(&CODE, &sd.encode());
    let pay = genesis(&dev, &def());
    // Copies 1–2 of record 0 bought already (by 4).
    let other = t::supply::Mint { to: did(4), item: [0; 32], amount: 2, nonce: 1u128.to_be_bytes(), stamp: 1, sigs: vec![], paid: Some(([4; 32], [4; 16])) };
    let rec = t::supply::Shard { mints: vec![other] };
    let (es, mint) = w.buy(&pay, &sd, &sid, (0, &rec), 1, 5).unwrap();
    assert_eq!((u128::from_be_bytes(mint.nonce), mint.amount, mint.to), (3, 1, did(1)), "the first free number");
    let paid = with(&pay, es.clone());
    assert_eq!(t::check(&paid.params, &paid.definition, &paid.state, &|_, _| None, &|_| None, Some(&lg), &t::Extra::NONE), Ok(()));
    assert!(matches!(es.last().unwrap().body, Body::Send { amount: 10, .. }));
    // The record holds it on the payment alone.
    let rp = t::supply::Params { code_hash: CODE, definition: sid, beacon: BEACON, epoch: 0 };
    let after = t::supply::Shard { mints: vec![mint.clone()] };
    assert_eq!(t::supply::holds(&rp, &sd, &Default::default(), &after, &|m| t::supply::payment_ok(&rp, &sd, m, Some(&paid.state))), Ok(()));
    // …and 1's chain of copy #3 collects it.
    let copy3 = Held { params: ChainParams { item: t::numbered_item(&sid, 3), ..held(1, &sd).params }, definition: sd.clone(), state: State::default(), blocks: Default::default() };
    let recs = [after.clone()];
    let got = w.collect(&copy3, &Sources { blocks: &Default::default(), chains: &[], programs: &[], shards: &recs }, 6).unwrap();
    assert!(matches!(&got[..], [Entry { body: Body::CappedMint { record: 0, amount: 1, .. }, .. }]));
    let c = with(&copy3, got);
    assert_eq!(t::check(&c.params, &sd, &c.state, &|_, _| None, &|k| (k == 0).then(|| after.clone()), Some(&lg), &t::Extra::NONE), Ok(()));
    // Two copies: 3–4 at 20 W. A full record: none.
    assert_eq!(w.buy(&pay, &sd, &sid, (0, &rec), 2, 5).map(|(e, m)| (e.last().unwrap().body.clone(), u128::from_be_bytes(m.nonce))).ok(), Some((Body::Send { to: did(9), amount: 20 }, 3)));
    let full = t::supply::Shard { mints: vec![t::supply::Mint { amount: 7, ..rec.mints[0].clone() }] };
    assert_eq!(w.buy(&pay, &sd, &sid, (0, &full), 1, 5).err(), Some("no free numbers in the record"));
    let one = t::supply::Shard { mints: vec![t::supply::Mint { amount: 1, ..rec.mints[0].clone() }] };
    assert_eq!(w.buy(&pay, &sd, &sid, (0, &one), 1, 5).map(|(_, m)| u128::from_be_bytes(m.nonce)).ok(), Some(2), "the number right after a taken one");
    // Record 1 holds 8–10 only (records of 7; the cap).
    assert_eq!(w.buy(&pay, &sd, &sid, (1, &Default::default()), 3, 5).map(|(_, m)| u128::from_be_bytes(m.nonce)).ok(), Some(8));
    assert_eq!(w.buy(&pay, &sd, &sid, (1, &Default::default()), 4, 5).err(), Some("no free numbers in the record"));
    // A RACE: 1 paid for #3 of record 0, but others took 3–7 first — the record is full: its copy is #8, record 1.
    let payment = es.last().unwrap();
    let raced = t::supply::Shard { mints: vec![rec.mints[0].clone(), t::supply::Mint { nonce: 3u128.to_be_bytes(), amount: 5, ..rec.mints[0].clone() }] };
    let (k, m) = remint(&pay.params, payment, &sd, &sid, (0, &raced), &Default::default()).unwrap();
    assert_eq!((k, u128::from_be_bytes(m.nonce), m.paid), (1, 8, mint.paid));
    let rp1 = t::supply::Params { code_hash: CODE, definition: sid, beacon: BEACON, epoch: 1 };
    let r1 = t::supply::Shard { mints: vec![m] };
    assert_eq!(t::supply::holds(&rp1, &sd, &Default::default(), &r1, &|x| { t::supply::payment_ok(&rp1, &sd, x, Some(&paid.state))?; t::supply::spill_ok(&sd, 1, Some(&raced), x) }), Ok(()));
    // Not full: other free numbers of record 0. Its payment taken already: refused.
    let part = t::supply::Shard { mints: vec![rec.mints[0].clone(), t::supply::Mint { nonce: 3u128.to_be_bytes(), amount: 1, ..rec.mints[0].clone() }] };
    assert_eq!(remint(&pay.params, payment, &sd, &sid, (0, &part), &Default::default()).map(|(k, m)| (k, u128::from_be_bytes(m.nonce))), Ok((0, 4)));
    assert_eq!(remint(&pay.params, payment, &sd, &sid, (0, &after), &Default::default()).err(), Some("a payment its record took already"));
    assert_eq!(remint(&pay.params, payment, &sd, &sid, (1, &raced), &Default::default()).err(), Some("not a payment for this record"));
    // Two copies paid, ONE number left in record 0 (#7): not full, so no spill — and no room: refused.
    let (es2, _) = w.buy(&pay, &sd, &sid, (0, &rec), 2, 5).unwrap();
    let one_left = t::supply::Shard { mints: vec![rec.mints[0].clone(), t::supply::Mint { nonce: 3u128.to_be_bytes(), amount: 4, ..rec.mints[0].clone() }] };
    assert_eq!(remint(&pay.params, es2.last().unwrap(), &sd, &sid, (0, &one_left), &Default::default()).err(), Some("a spilled mint whose record before is not full"));
    // Paid from a chain of another token: refused.
    assert_eq!(w.buy(&genesis(&dev, &Definition { name: b"X".to_vec(), ..def() }), &sd, &sid, (0, &rec), 1, 5).err(), Some("a payment from another chain than the sale's token's"));
}

#[test]
fn a_device_that_asks_builds_the_same_entries_one_signature_a_round() {
    // A second device of 1 (so a writer entry comes first), signing only by asking.
    let (first, real) = (Dev::new(1, 11), Dev::new(1, 12));
    let lg = log(1);
    let h = genesis(&first, &def());
    let mut sigs: Vec<(Vec<u8>, [u8; 64])> = Vec::new();
    let mut rounds = 0;
    let out = loop {
        let asking = Asking::new(real.public(), real.credential(), 0, sigs.clone());
        let w = Wallet { device: &asking, keys: &lg };
        let got = w.debit(&h, Body::Send { to: did(2), amount: 300 }, 5);
        match asking.next() {
            Some(m) => {
                assert!(m.starts_with(t::ENTRY_DOMAIN), "only entries are asked");
                sigs.push((m.clone(), real.sign(&m)));
                rounds += 1;
            }
            None => break got.unwrap(),
        }
    };
    assert_eq!(rounds, 2, "the writer entry, then the send");
    let direct = Wallet { device: &real, keys: &lg }.debit(&h, Body::Send { to: did(2), amount: 300 }, 5).unwrap();
    assert_eq!(out, direct, "the same entries as the device signing itself");
}
