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
    Definition { code_hash: CODE, decimals: 0, min_send: 1, genesis_owner: did(1), genesis_amount: 1000, name: b"W".to_vec(), nonce: [0; 16], beacon: BEACON, margin: 2, idlog: IDLOG, mint: None, freeze: None, ids: t::Ids::None, royalty: None, cap: 0, collection: [0; 32], item: [0; 32], version: [0; 32], soulbound: false, transfer_fee: None }
}
fn held(n: u8, d: &Definition) -> Held {
    Held { params: ChainParams { code_hash: CODE, definition: t::id(&CODE, &d.encode()), owner: did(n), beacon: BEACON, idlog: IDLOG, item: [0; 32] }, definition: d.clone(), state: State::default() }
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
