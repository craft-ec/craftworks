//! A SPACE TABLE READ against a scripted network built with the real pieces (`data` tails sealed with epoch keys, a
//! flushed tree in Sealed contracts, `feed` versions, a writers bag sealed AES-GCM): the merge comes out as a page's.

use super::table::{self, Codes, Reading};
use super::upkeep::{self, Io};
use craftworks_data as data;
use craftworks_identity as identity;
use ed25519_dalek::{Signer, SigningKey};
use std::collections::{BTreeMap, HashMap};

const TAIL: &[u8] = b"the tail contract's code";
const BAG: &[u8] = b"the bag contract's code";
const SEALED: &[u8] = b"the sealed contract's code";
const BLOCK: &[u8] = b"the block contract's code";
const SPACE: [u8; 32] = [0x5A; 32];

fn codes() -> Codes<'static> {
    Codes { tail: TAIL, bag_hash: contract_keys::code_hash(BAG), sealed_hash: contract_keys::code_hash(SEALED), block_hash: contract_keys::code_hash(BLOCK) }
}

/// The network: contract id → state.
#[derive(Default)]
struct Net(HashMap<[u8; 32], Vec<u8>>);

/// A writer's FEED of a space table, as a page writes it: sealed with the table's key and the newest epoch's.
struct Feed {
    key: SigningKey,
    o: data::Open,
}
impl Feed {
    fn new(key: &SigningKey, table: &str, label: &str, epochs: &[(u64, [u8; 32])]) -> Feed {
        let mut o = data::Open::new(TAIL, &key.verifying_key().to_bytes(), label);
        o.set_table_key(identity::space_table_key(&SPACE, table));
        for (e, s) in epochs {
            o.epoch_key(*e, Some(identity::epoch_table_key(s, table)), true);
        }
        Feed { key: key.clone(), o }
    }
    fn sign(&self, seq: u64, hash: [u8; 32]) -> [u8; 64] {
        let p = craftec_register_contract::wire::Params::parse(&self.o.params).unwrap();
        self.key.sign(&p.signed_message(false, seq, &hash)).to_bytes()
    }
    /// A row as a VERSION (replacing `after`), written and on the network. Its id.
    fn put(&mut self, net: &mut Net, k: &str, v: &str, after: Option<craftworks_feed::Id>) -> craftworks_feed::Id {
        let id = (self.key.verifying_key().to_bytes(), self.o.writer.seq() + 1);
        let env = craftworks_feed::envelope(id, after, Some(v.as_bytes()));
        let (seq, h) = self.o.prepare_row(k.as_bytes(), &env).unwrap();
        self.o.commit(self.sign(seq, h)).unwrap();
        net.0.insert(self.o.id_bytes(), self.o.state());
        id
    }
    /// Its rows FLUSHED into its tree: the blocks put in their Sealed contracts first.
    fn flush(&mut self, net: &mut Net) {
        let Ok(data::Step::Ready(f)) = self.o.flush() else { panic!("a flush of held rows is ready") };
        let ak = f.address_key.expect("a sealed tree");
        for (cid, state) in &f.blocks {
            net.0.insert(upkeep::id_of(&contract_keys::code_hash(SEALED), &data::block_address(&ak, cid)), state.clone());
        }
        self.o.commit(self.sign(f.seq, f.hash)).unwrap();
        net.0.insert(self.o.id_bytes(), self.o.state());
    }
}

fn writer(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}
fn pubkey(k: &SigningKey) -> [u8; 32] {
    k.verifying_key().to_bytes()
}
/// A writer's catalog listing `tables` (blinded).
fn catalog(net: &mut Net, k: &SigningKey, epochs: &[(u64, [u8; 32])], tables: &[&str]) {
    let cat = table::table_name(&SPACE, "tables");
    let mut f = Feed::new(k, &cat, &cat, epochs);
    for t in tables {
        f.put(net, &table::table_name(&SPACE, t), r#"{"at":1,"b":1}"#, None);
    }
}
fn blinded(name: &str) -> String {
    let t = table::table_name(&SPACE, name);
    identity::blind_name(&identity::space_table_key(&SPACE, &t), &t)
}
/// The writers bag: each writer listed, sealed with epoch 0's key.
fn writers_bag(net: &mut Net, s0: &[u8; 32], ws: &[[u8; 32]]) {
    use aes_gcm::aead::{Aead, KeyInit};
    let address = table::sealed_bag_address(&SPACE, "writers");
    let key = identity::epoch_table_key(s0, "bag-writers");
    let cipher = aes_gcm::Aes256Gcm::new_from_slice(&key).unwrap();
    let items: Vec<Vec<u8>> = ws
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let nonce = [i as u8; 12];
            let ct = cipher.encrypt(aes_gcm::Nonce::from_slice(&nonce), serde_json::json!({ "w": table::hex(w) }).to_string().as_bytes()).unwrap();
            let item = [0u32.to_be_bytes().as_slice(), &nonce, &ct].concat();
            craftworks_bag_contract::grind(&address, &item)
        })
        .collect();
    net.0.insert(upkeep::id_of(&contract_keys::code_hash(BAG), &address), craftworks_bag_contract::encode(&address, &items));
}

/// Drive a read to its end against `net`; every GET made.
fn run(r: &mut Reading, first: Vec<Io>, net: &Net) -> Vec<[u8; 32]> {
    let c = codes();
    let mut asked = Vec::new();
    let mut todo = first;
    while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
        let Io::Get { id, .. } = io else { panic!("a read only GETs") };
        asked.push(id);
        todo.extend(r.got(&c, id, net.0.get(&id).cloned()));
    }
    asked
}

#[test]
fn a_space_table_reads_as_its_feeds_merged_departed_writers_capped() {
    let (s0, s1) = ([0x10; 32], [0x11; 32]);
    let epochs = [(0, s0), (1, s1)];
    let (a, b, c, d) = (writer(1), writer(2), writer(3), writer(4));
    let files = table::table_name(&SPACE, "files");
    let departed = table::table_name(&SPACE, "departed");
    let mut net = Net::default();
    // A: salt and f1, FLUSHED into a sealed tree; then f2 in its tail. The departed table: D's files feed capped at 1.
    catalog(&mut net, &a, &epochs, &["files", "departed"]);
    let mut fa = Feed::new(&a, &files, &blinded("files"), &epochs);
    fa.put(&mut net, "salt", "S", None);
    let f1 = fa.put(&mut net, "k/f1", "A's f1", None);
    fa.flush(&mut net);
    fa.put(&mut net, "k/f2", "A's f2", None);
    let mut da = Feed::new(&a, &departed, &blinded("departed"), &epochs);
    da.put(&mut net, &table::hex(&pubkey(&d)), &serde_json::json!({ "heads": { files.clone(): 1 } }).to_string(), None);
    // B: f3, and f1 again over A's version.
    catalog(&mut net, &b, &epochs, &["files"]);
    let mut fb = Feed::new(&b, &files, &blinded("files"), &epochs);
    fb.put(&mut net, "k/f3", "B's f3", None);
    fb.put(&mut net, "k/f1", "B's f1", Some(f1));
    // D, departed: d1 at seq 1 counts, d2 after its cap does not.
    catalog(&mut net, &d, &epochs, &["files"]);
    let mut fd = Feed::new(&d, &files, &blinded("files"), &epochs);
    fd.put(&mut net, "k/d1", "D's d1", None);
    fd.put(&mut net, "k/d2", "D's d2 (after it left)", None);
    // C: in the roster, never listed in the bag (never wrote here): its catalog is never asked.
    writers_bag(&mut net, &s0, &[pubkey(&a), pubkey(&b), pubkey(&d)]);

    let epochs_map: BTreeMap<u64, [u8; 32]> = epochs.into_iter().collect();
    let (mut r, first) = Reading::new(&codes(), SPACE, epochs_map, vec![pubkey(&a), pubkey(&b), pubkey(&c)], &["files"]);
    let asked = run(&mut r, first, &net);
    assert!(r.done(), "the read finished: {:?}", r.notes);

    let rows: BTreeMap<String, String> =
        r.rows("files").into_iter().map(|(k, v)| (String::from_utf8(k).unwrap(), String::from_utf8(v.value).unwrap())).collect();
    let want: BTreeMap<String, String> = [("salt", "S"), ("k/f1", "B's f1"), ("k/f2", "A's f2"), ("k/f3", "B's f3"), ("k/d1", "D's d1")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    assert_eq!(rows, want, "notes: {:?}", r.notes);
    // The tree was read (A's flushed rows are there) — and C, never listed, was never asked for.
    let c_catalog = data::Open::new(TAIL, &pubkey(&c), &table::table_name(&SPACE, "tables")).id_bytes();
    assert!(!asked.contains(&c_catalog), "a writer not in the bag is never asked");
    // A's rows before its flush came from its TREE: blocks were fetched from Sealed contracts.
    let sealed_ids: Vec<[u8; 32]> = net.0.keys().copied().filter(|id| asked.contains(id)).filter(|id| {
        ![fa.o.id_bytes(), fb.o.id_bytes(), fd.o.id_bytes(), da.o.id_bytes()].contains(id)
            && *id != upkeep::id_of(&contract_keys::code_hash(BAG), &table::sealed_bag_address(&SPACE, "writers"))
            && ![1u8, 2, 4].iter().any(|n| *id == data::Open::new(TAIL, &pubkey(&writer(*n)), &table::table_name(&SPACE, "tables")).id_bytes())
    }).collect();
    assert!(!sealed_ids.is_empty(), "the flushed tree's blocks were read");
}

#[test]
fn without_the_epoch_a_feed_is_sealed_with_nothing_of_it_reads() {
    let (s0, s1) = ([0x10; 32], [0x11; 32]);
    let epochs = [(0, s0), (1, s1)];
    let a = writer(1);
    let files = table::table_name(&SPACE, "files");
    let mut net = Net::default();
    catalog(&mut net, &a, &epochs, &["files"]);
    let mut fa = Feed::new(&a, &files, &blinded("files"), &epochs);
    fa.put(&mut net, "k/f1", "sealed with epoch 1", None);
    writers_bag(&mut net, &s0, &[pubkey(&a)]);
    // A member holding epoch 0 only (removed before epoch 1): the catalog (epoch 1 too) does not read — nothing does.
    let only0: BTreeMap<u64, [u8; 32]> = [(0, s0)].into_iter().collect();
    let (mut r, first) = Reading::new(&codes(), SPACE, only0, vec![pubkey(&a)], &["files"]);
    run(&mut r, first, &net);
    assert!(r.done());
    assert!(r.rows("files").is_empty(), "nothing sealed with epoch 1 reads without it");
    // Control: with epoch 1, it reads.
    let all: BTreeMap<u64, [u8; 32]> = epochs.into_iter().collect();
    let (mut r, first) = Reading::new(&codes(), SPACE, all, vec![pubkey(&a)], &["files"]);
    run(&mut r, first, &net);
    assert_eq!(r.rows("files").len(), 1);
}
