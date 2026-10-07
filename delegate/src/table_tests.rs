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
const IDLOG: &[u8] = b"the key log contract's code";
const SPACE: [u8; 32] = [0x5A; 32];

fn codes() -> Codes<'static> {
    Codes { tail: TAIL, bag_hash: contract_keys::code_hash(BAG), sealed_hash: contract_keys::code_hash(SEALED), block_hash: contract_keys::code_hash(BLOCK), idlog_hash: contract_keys::code_hash(IDLOG) }
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
    // B removed E: its own departed row (a second remover's).
    let e = writer(5);
    catalog(&mut net, &b, &epochs, &["files", "departed"]);
    let mut db = Feed::new(&b, &departed, &blinded("departed"), &epochs);
    db.put(&mut net, &table::hex(&pubkey(&e)), &serde_json::json!({ "heads": { files.clone(): 1 } }).to_string(), None);
    catalog(&mut net, &e, &epochs, &["files"]);
    let mut fe = Feed::new(&e, &files, &blinded("files"), &epochs);
    fe.put(&mut net, "k/e1", "E's e1", None);
    // C: in the roster, never listed in the bag (never wrote here): its catalog is never asked.
    writers_bag(&mut net, &s0, &[pubkey(&a), pubkey(&b), pubkey(&d), pubkey(&e)]);

    let epochs_map: BTreeMap<u64, [u8; 32]> = epochs.into_iter().collect();
    let (mut r, first) = Reading::new(&codes(), SPACE, epochs_map, vec![pubkey(&a), pubkey(&b), pubkey(&c)], &["files"]);
    let asked = run(&mut r, first, &net);
    assert!(r.done(), "the read finished: {:?}", r.notes);

    let rows: BTreeMap<String, String> =
        r.rows("files").into_iter().map(|(k, v)| (String::from_utf8(k).unwrap(), String::from_utf8(v.value).unwrap())).collect();
    let want: BTreeMap<String, String> = [("salt", "S"), ("k/f1", "B's f1"), ("k/f2", "A's f2"), ("k/f3", "B's f3"), ("k/d1", "D's d1"), ("k/e1", "E's e1")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    assert_eq!(rows, want, "notes: {:?}", r.notes);
    // The tree was read (A's flushed rows are there) — and C, never listed, was never asked for.
    let c_catalog = data::Open::new(TAIL, &pubkey(&c), &table::table_name(&SPACE, "tables")).id_bytes();
    assert!(!asked.contains(&c_catalog), "a writer not in the bag is never asked");
    // A's rows before its flush came from its TREE: blocks were fetched from Sealed contracts.
    let sealed_ids: Vec<[u8; 32]> = net.0.keys().copied().filter(|id| asked.contains(id)).filter(|id| {
        ![fa.o.id_bytes(), fb.o.id_bytes(), fd.o.id_bytes(), da.o.id_bytes(), db.o.id_bytes(), fe.o.id_bytes()].contains(id)
            && *id != upkeep::id_of(&contract_keys::code_hash(BAG), &table::sealed_bag_address(&SPACE, "writers"))
            && ![1u8, 2, 4, 5].iter().any(|n| *id == data::Open::new(TAIL, &pubkey(&writer(*n)), &table::table_name(&SPACE, "tables")).id_bytes())
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

// ---- R4b: writing this member's own feed with no page open ----

use super::write::{self, Next};
use craftworks_identity::{Host, Request};

#[derive(Default)]
struct Secrets(HashMap<Vec<u8>, Vec<u8>>);
impl Host for Secrets {
    fn get_secret(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.get(key).cloned()
    }
    fn set_secret(&mut self, key: &[u8], value: &[u8]) -> bool {
        self.0.insert(key.to_vec(), value.to_vec());
        true
    }
}
const SEED: [u8; 32] = [0x31; 32];
const DATA: [u8; 32] = [0xD1; 32];
/// A node holding a member (its data seed too); the member's public key.
fn node() -> (Secrets, [u8; 32]) {
    let mut h = Secrets::default();
    identity::serve(&mut h, Request::Provision { seed: SEED, did: [0x44; 32], pin: "246810".into(), data: DATA.to_vec() }, [9; 32]);
    (h, SigningKey::from_bytes(&SEED).verifying_key().to_bytes())
}
/// What a write sent, applied to the network: PUTs and UPDATEs as the tail's (or bag's, block's) whole state now.
fn apply(net: &mut Net, r: &Reading, sent: &write::Sent, me: &[u8; 32], name: &str) {
    for io in &sent.io {
        match io {
            Io::Put { code: upkeep::Code::Bag, params, state } => {
                net.0.insert(upkeep::id_of(&contract_keys::code_hash(BAG), params), state.clone());
            }
            Io::Put { code: upkeep::Code::Sealed, params, state } => {
                net.0.insert(upkeep::id_of(&contract_keys::code_hash(SEALED), params), state.clone());
            }
            _ => {}
        }
    }
    // The tail the step was for: as the write left it.
    for n in [name, "tables"] {
        if let Some((Some(st), _)) = r.own(me, n) {
            let label = if n == "tables" { table::table_name(&SPACE, "tables") } else { blinded(n) };
            net.0.insert(data::Open::new(TAIL, me, &label).id_bytes(), st);
        }
    }
}
fn read_files(net: &Net, writers: Vec<[u8; 32]>, epochs: &BTreeMap<u64, [u8; 32]>) -> (Reading, BTreeMap<String, String>) {
    let (mut r, first) = Reading::new(&codes(), SPACE, epochs.clone(), writers, &["files"]);
    run(&mut r, first, net);
    let rows = r.rows("files").into_iter().map(|(k, v)| (String::from_utf8(k).unwrap(), String::from_utf8(v.value).unwrap())).collect();
    (r, rows)
}

#[test]
fn upkeep_writes_its_own_feed_listing_it_first_when_new_and_flushes_when_due() {
    let (mut h, member) = node();
    // A page writes a space under its member key (one per device): upkeep the same.
    let me_key = SigningKey::from_bytes(&SEED);
    let me = me_key.verifying_key().to_bytes();
    assert_eq!(identity::upkeep_space_writer(&h, &member), Some(me));
    let epochs: BTreeMap<u64, [u8; 32]> = [(0, [0x10; 32]), (1, [0x11; 32])].into_iter().collect();
    let ep: Vec<(u64, [u8; 32])> = epochs.clone().into_iter().collect();
    let other = writer(2);
    let mut net = Net::default();
    // Another member's feed holds k/x; this member has written nothing here (no catalog, not in the bag).
    catalog(&mut net, &other, &ep, &["files"]);
    let mut fo = Feed::new(&other, &table::table_name(&SPACE, "files"), &blinded("files"), &ep);
    let x = fo.put(&mut net, "k/x", "theirs", None);
    writers_bag(&mut net, &[0x10; 32], &[pubkey(&other)]);
    let writers = vec![pubkey(&other), me];

    // FIRST WRITE: listed first (the bag, its catalog), then the feed made.
    let (mut r, _) = read_files(&net, writers.clone(), &epochs);
    let rows = vec![(b"k/x".to_vec(), Some(b"mine over theirs".to_vec())), (b"k/y".to_vec(), Some(b"new".to_vec()))];
    let Next::More(listing) = write::rows(&mut h, &member, &codes(), &mut r, "files", &rows, [1; 12], false, true).unwrap() else { panic!("listed first") };
    assert!(listing.io.iter().any(|io| matches!(io, Io::Put { code: upkeep::Code::Bag, .. })), "in the writers bag before its catalog");
    apply(&mut net, &r, &listing, &me, "files");
    let Next::Done(sent) = write::rows(&mut h, &member, &codes(), &mut r, "files", &rows, [2; 12], false, true).unwrap() else { panic!("then the rows") };
    assert!(matches!(sent.io[0], Io::Put { code: upkeep::Code::Tail, .. }), "the feed made with a PUT");
    apply(&mut net, &r, &sent, &me, "files");
    let (_, got) = read_files(&net, writers.clone(), &epochs);
    assert_eq!(got.get("k/x").map(String::as_str), Some("mine over theirs"), "a version over the one current (theirs): {got:?}");
    assert_eq!(got.get("k/y").map(String::as_str), Some("new"));
    let _ = x;

    // MANY ROWS: past 32 waiting, a FLUSH first (its tree's blocks put), then the rows; all read back from the tree.
    let (mut r, _) = read_files(&net, writers.clone(), &epochs);
    let many: Vec<(Vec<u8>, Option<Vec<u8>>)> = (0..40).map(|i| (format!("k/m{i:02}").into_bytes(), Some(b"m".to_vec()))).collect();
    let mut flushed = false;
    let mut nonce = 3u8;
    for half in [&many[..20], &many[20..]] {
        // Called until its rows are written: each step answered before the next (a flush: its blocks, then its step).
        let mut blocks_put = false;
        for _ in 0..6 {
            nonce += 1;
            match write::rows(&mut h, &member, &codes(), &mut r, "files", half, [nonce; 12], blocks_put, true).unwrap() {
                Next::More(s) => {
                    flushed |= s.io.iter().any(|io| matches!(io, Io::Put { code: upkeep::Code::Sealed, .. }));
                    if s.blocks {
                        assert!(s.io.iter().all(|io| matches!(io, Io::Put { code: upkeep::Code::Sealed, .. })), "a flush's blocks alone first");
                    }
                    blocks_put = s.blocks;
                    apply(&mut net, &r, &s, &me, "files");
                }
                Next::Done(s) => {
                    apply(&mut net, &r, &s, &me, "files");
                    break;
                }
            }
        }
    }
    assert!(flushed, "a feed past 32 rows is flushed into its tree");
    let (_, got) = read_files(&net, writers.clone(), &epochs);
    assert_eq!(got.keys().filter(|k| k.starts_with("k/m")).count(), 40, "every row read back (tree and tail)");

    // A STEP SIGNED AND LOST (its write never landed): the next write goes past it as a whole state, not wedged.
    let (mut r, _) = read_files(&net, writers.clone(), &epochs);
    let lost = write::rows(&mut h, &member, &codes(), &mut r, "files", &[(b"k/lost".to_vec(), Some(b"never landed".to_vec()))], [40; 12], false, true).unwrap();
    assert!(matches!(lost, Next::Done(_)), "signed (and then lost: not applied to the network)");
    let (mut r, _) = read_files(&net, writers.clone(), &epochs);
    let Next::Done(past) = write::rows(&mut h, &member, &codes(), &mut r, "files", &[(b"k/after".to_vec(), Some(b"written past it".to_vec()))], [41; 12], false, true).unwrap() else {
        panic!("written")
    };
    assert!(matches!(past.io[0], Io::Put { code: upkeep::Code::Tail, .. }), "past the lost step: the whole state");
    apply(&mut net, &r, &past, &me, "files");
    let (_, got) = read_files(&net, writers.clone(), &epochs);
    assert_eq!(got.get("k/after").map(String::as_str), Some("written past it"));
    assert!(!got.contains_key("k/lost"));

    // THE FORK GUARD: a different step at a place already signed is refused (a page holding an older state).
    let (r2, _) = read_files(&net, writers, &epochs);
    let (Some(st), _) = r2.own(&me, "files").unwrap() else { panic!("its feed") };
    let mut o = data::Open::new(TAIL, &me, &blinded("files"));
    o.absorb(&st);
    let p = o.params.clone();
    let seq = o.writer.seq();
    assert!(identity::upkeep_sign_space(&mut h, &member, &p, seq, &[0xEE; 32]).is_err(), "never two different steps at one place");
}

// ---- R4c: what is due, and when a new salt is ----

use super::rekey;

#[test]
fn the_rank_is_the_pages_rank() {
    // Computed by the page's own `rankFor` (file-keys.js) for these DIDs: each member's place per row.
    let dids: Vec<String> = ["did:craftec:aaa", "did:craftec:bbb", "did:craftec:ccc", "did:craftec:ddd"].iter().map(|s| s.to_string()).collect();
    for (id, want) in [("f1", [0, 3, 1, 2]), ("f2", [3, 1, 2, 0]), ("3f2a9c", [3, 1, 2, 0]), ("zz", [3, 0, 2, 1])] {
        let got: Vec<usize> = dids.iter().map(|d| rekey::rank(id, &dids, d)).collect();
        assert_eq!(got, want, "row {id}");
    }
}

#[test]
fn a_removal_rotates_the_salt_once_the_group_moved_and_its_rows_come_due() {
    let (s0, s1) = ([0x10; 32], [0x11; 32]);
    let ep = [(0, s0), (1, s1)];
    let epochs: BTreeMap<u64, [u8; 32]> = ep.into_iter().collect();
    let (owner, gone) = (writer(1), writer(2));
    let (owner_did, gone_did) = ("did:craftec:owner".to_string(), "did:craftec:gone".to_string());
    let mut net = Net::default();
    catalog(&mut net, &owner, &ep, &["files", "acts"]);
    // The files table: salt 0, a row under it, a public row whose app reads members-only, an adopted row, one current.
    let mut f = Feed::new(&owner, &table::table_name(&SPACE, "files"), &blinded("files"), &ep);
    let salt0 = table::hex(&[0x55; 32]);
    f.put(&mut net, "salt", &serde_json::json!({ "s": salt0, "n": 0, "removals": 0 }).to_string(), None);
    let row = |n: i64, public: bool| serde_json::json!({ "key": "aa", "root": "bb", "n": n, "pub": public, "app": "drive", "at": 1 }).to_string();
    f.put(&mut net, "k/under0", &row(0, false), None);
    f.put(&mut net, "k/public", &row(-1, true), None);
    f.put(&mut net, "k/adopted", &row(-2, false), None);
    // The acts: the owner admits, then REMOVES the member.
    let mut acts = Feed::new(&owner, &table::table_name(&SPACE, "acts"), &blinded("acts"), &ep);
    acts.put(&mut net, "a1", &serde_json::json!({ "at": 10, "act": "added", "did": gone_did }).to_string(), None);
    writers_bag(&mut net, &s0, &[pubkey(&owner)]);
    let roster = vec![(pubkey(&owner), owner_did.clone()), (pubkey(&gone), gone_did.clone())];
    let read = |net: &Net| {
        let (mut r, first) = Reading::new(&codes(), SPACE, epochs.clone(), roster.iter().map(|(w, _)| *w).collect(), &["files", "acts", "pub-acts"]);
        run(&mut r, first, net);
        r
    };

    // CONTROL: no removal yet — no new salt; only the adopted and the no-longer-public rows are due.
    let r = read(&net);
    let g = rekey::governance(&r, &roster, &owner_did, 100);
    let p = rekey::plan(&r, &g, &[owner_did.clone(), gone_did.clone()], &owner_did, 100);
    assert_eq!(p.rotate, None);
    let mut ids: Vec<&str> = p.due.iter().map(|d| d.row.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, ["adopted", "public"]);

    // The REMOVAL counted — while the group still holds them: not yet (the commit not seen).
    acts.put(&mut net, "a2", &serde_json::json!({ "at": 20, "act": "remove", "did": gone_did }).to_string(), None);
    let r = read(&net);
    let g = rekey::governance(&r, &roster, &owner_did, 100);
    assert_eq!(rekey::plan(&r, &g, &[owner_did.clone(), gone_did.clone()], &owner_did, 100).rotate, None, "the group must move first");
    // The group without them: a new salt, made after 1 removal — and the row under salt 0 comes due.
    let p = rekey::plan(&r, &g, &[owner_did.clone()], &owner_did, 100);
    assert_eq!(p.rotate, Some(1));
    let mut ids: Vec<&str> = p.due.iter().map(|d| d.row.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, ["adopted", "public", "under0"]);
    assert!(p.due.iter().all(|d| d.wait == 0), "the only member: its turn for every row");
    // A group held from before the removal still lists them: they take no turn (else a removed member ranked first
    // holds rows up for the takeover time). Control: counted as a member, some row is theirs first.
    let held = [owner_did.clone(), gone_did.clone()];
    let p2 = rekey::plan(&r, &g, &held, &owner_did, 100);
    assert!(p2.due.iter().all(|d| d.rank == 0), "the removed take no turn");
    let control: Vec<usize> = ["adopted", "public", "under0", "a", "b", "c", "d"].iter().map(|id| rekey::rank(id, &held, &owner_did)).collect();
    assert!(control.contains(&1), "control: with them counted, the owner is second for some row: {control:?}");
    // The rotation's rows: the old salt kept, the new one, and `salt` naming it.
    let was = rekey::salt(&r.rows("files")).unwrap();
    let rows = rekey::rotation(&r.rows("files"), &was, [0x66; 32], 1);
    assert_eq!(rows[0].0, b"salt/0");
    assert_eq!(rows[1].0, b"salt/1");
    let v: serde_json::Value = serde_json::from_slice(rows[2].1.as_ref().unwrap()).unwrap();
    assert_eq!((v["n"].as_i64(), v["removals"].as_u64()), (Some(1), Some(1)));
}

// ---- R4d: a file re-keyed ----

use super::recode::{self, End, Recode, Target};
use craftworks_files as files;

const PIECE: &[u8] = b"the piece contract's code";
fn piece_hash() -> [u8; 32] {
    contract_keys::code_hash(PIECE)
}
fn sha256(b: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(b).into()
}
/// A file stored under `key` with `burn`: its pieces on the network; its root's hash.
fn store_file(net: &mut Net, key: &[u8; 32], content: &[u8], burn: &[u8; 32]) -> [u8; 32] {
    let plan = files::Plan::of(content.len() as u64);
    let mut stored = Vec::new();
    for g in 0..plan.gens {
        let (a, b) = plan.range(g);
        let mut listed = Vec::new();
        for (j, p) in files::encode(key, &plan, g, &content[a as usize..b as usize], files::EXTRA) {
            listed.push((j, p.hash()));
            net.0.insert(upkeep::id_of(&piece_hash(), &p.address), [&[2u8][..], burn, &p.state].concat());
        }
        stored.push(listed);
    }
    let (pieces, root) = files::index_hashed(key, &plan, &stored);
    for p in pieces {
        net.0.insert(upkeep::id_of(&piece_hash(), &p.address), [&[2u8][..], burn, &p.state].concat());
    }
    root
}
/// A piece as a reader takes it: live only.
fn live_piece(net: &Net, address: &[u8; 32]) -> Option<Vec<u8>> {
    let st = net.0.get(&upkeep::id_of(&piece_hash(), address))?;
    (st.first() == Some(&2) && st.len() > 33).then(|| st[33..].to_vec())
}
/// The file read back whole by a reader holding `key` and `root`.
fn read_file(net: &Net, key: &[u8; 32], root: &[u8; 32]) -> Option<Vec<u8>> {
    let r = files::Root::open(key, root, &live_piece(net, &files::hashed_address(key, root))?).ok()?;
    let plan = r.plan;
    let mut out = Vec::new();
    for (n, leaf) in r.children.iter().enumerate() {
        let gens = files::open_leaf(key, &plan, n as u64, leaf, &live_piece(net, &files::hashed_address(key, leaf))?).ok()?;
        for (i, listed) in gens.into_iter().enumerate() {
            let g = (n * files::GENS_PER_LEAF + i) as u64;
            let mut d = files::Decoder::new(key, &plan, g, listed.clone());
            for (j, _) in &listed {
                if let Some(st) = live_piece(net, &files::fragment_address(key, g, *j)) {
                    let _ = d.add(*j, &st);
                }
            }
            out.extend(d.plain().ok()?);
        }
    }
    Some(out)
}

#[test]
fn a_file_is_re_keyed_under_the_new_salt_read_whole_and_its_old_pieces_burned() {
    let content: Vec<u8> = (0..5 * 1024 * 1024 + 777u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
    let h = *blake3::hash(&content).as_bytes();
    let (s0, s1) = ([0x50; 32], [0x51; 32]);
    let old_key = files::content_key(&h, Some(&s0));
    let secret0 = recode::burn_secret(&s0, &old_key);
    let burn0 = sha256(&secret0);
    let mut net = Net::default();
    let root0 = store_file(&mut net, &old_key, &content, &burn0);
    // Some fragments lost (fewer than its spare ones): it still reads.
    for j in [0u8, 3, 7] {
        net.0.remove(&upkeep::id_of(&piece_hash(), &files::fragment_address(&old_key, 0, j)));
    }
    let row = serde_json::json!({ "key": table::hex(&old_key), "root": table::hex(&root0), "b": table::hex(&burn0),
        "x": table::hex(&secret0), "h": table::hex(&h), "n": 0, "pub": false, "app": "drive" }).to_string();

    let mut hs = Secrets::default();
    let (mut r, mut todo) = Recode::start(&piece_hash(), "f1", &row, &Target { salt: Some((s1, 1)) }).unwrap();
    assert!(!r.same());
    let mut gets = 0;
    while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
        match io {
            Io::Get { id, .. } => {
                gets += 1;
                todo.extend(r.got(&mut hs, &piece_hash(), id, net.0.get(&id).cloned()).unwrap());
            }
            Io::Put { code: upkeep::Code::Piece, params, state } => {
                let id = upkeep::id_of(&piece_hash(), &params);
                net.0.insert(id, state);
                todo.extend(r.put(&mut hs, &piece_hash(), id, true).unwrap());
            }
            other => panic!("a re-key only GETs and PUTs pieces: {other:?}"),
        }
    }
    let Some(End::Done { row: new_row, old }) = r.end() else { panic!("re-keyed") };
    let v: serde_json::Value = serde_json::from_str(&new_row).unwrap();
    let new_key = files::content_key(&h, Some(&s1));
    assert_eq!(v["key"].as_str(), Some(table::hex(&new_key).as_str()));
    assert_eq!(v["n"].as_i64(), Some(1));
    let new_root: [u8; 32] = (0..32).map(|i| u8::from_str_radix(&v["root"].as_str().unwrap()[2 * i..2 * i + 2], 16).unwrap()).collect::<Vec<_>>().try_into().unwrap();
    assert_eq!(read_file(&net, &new_key, &new_root).as_deref(), Some(content.as_slice()), "read whole under the new key ({gets} GETs)");
    assert_ne!(v["x"], serde_json::Value::String(table::hex(&secret0)), "a new burn secret");
    // Its fragments' secrets cleaned out of the node.
    assert!(hs.0.iter().all(|(k, v)| !k.starts_with(b"identity_upkeep/rekey/frag/") || v.is_empty()));

    // The OLD pieces BURNED with the secret its row kept: then the old reference reads nothing.
    let secret = r.burnable().expect("its old pieces were this space's");
    assert_eq!(secret, secret0);
    assert!(read_file(&net, &old_key, &root0).is_some(), "control: the old reference reads before the burn");
    for address in old {
        net.0.insert(upkeep::id_of(&piece_hash(), &address), recode::burned(&secret));
    }
    assert!(read_file(&net, &old_key, &root0).is_none(), "the old reference reads nothing once burned");
    assert_eq!(read_file(&net, &new_key, &new_root).as_deref(), Some(content.as_slice()), "the new one still reads");
}

#[test]
fn a_row_with_no_content_hash_is_left_to_a_page() {
    let row = serde_json::json!({ "key": table::hex(&[1; 32]), "root": table::hex(&[2; 32]), "n": -2 }).to_string();
    assert!(Recode::start(&piece_hash(), "f", &row, &Target { salt: Some(([3; 32], 1)) }).is_err());
}

// ---- R4e: the whole re-key round, with no page open ----

/// The network a round runs against: every contract's state, tails taking deltas as hosts do.
#[derive(Default)]
struct Host2 {
    net: Net,
    params: HashMap<[u8; 32], Vec<u8>>,
}
impl Host2 {
    fn seed_tail(&mut self, o: &data::Open) {
        self.params.insert(o.id_bytes(), o.params.clone());
        self.net.0.insert(o.id_bytes(), o.state());
    }
    /// One ask answered: GETs from the state held, PUTs and UPDATEs taken (a tail's delta applied by its contract).
    fn answer(&mut self, io: &Io) -> upkeep::Reply {
        use craftec_register_contract::wire::Params;
        match io {
            Io::Get { id, .. } => upkeep::Reply::Got { id: *id, state: self.net.0.get(id).cloned() },
            Io::Put { code, params, state } => {
                let code_bytes: &[u8] = match code {
                    upkeep::Code::Tail => TAIL,
                    upkeep::Code::Bag => BAG,
                    upkeep::Code::Sealed => SEALED,
                    upkeep::Code::Piece => PIECE,
                };
                let id = upkeep::id_of(&contract_keys::code_hash(code_bytes), params);
                if *code == upkeep::Code::Tail {
                    self.params.insert(id, params.clone());
                }
                // A bag merges what it holds with what is put (its items, ground): kept both.
                let st = if *code == upkeep::Code::Bag {
                    let mut items = self.net.0.get(&id).and_then(|s| craftworks_bag_contract::read(params, s)).unwrap_or_default();
                    items.extend(craftworks_bag_contract::read(params, state).unwrap_or_default());
                    craftworks_bag_contract::encode(params, &items)
                } else {
                    state.clone()
                };
                self.net.0.insert(id, st);
                upkeep::Reply::Put { id, ok: true }
            }
            Io::Update { id, delta } => {
                let p = Params::parse(&self.params[id]).unwrap();
                let held = self.net.0.get(id).and_then(|s| craftec_tail_contract::read(&self.params[id], s)).and_then(|(_, t)| t);
                let next = craftec_tail_contract::absorb(held.clone(), delta, &p);
                let ok = next.as_ref().map(|t| t.signed.seq) != held.as_ref().map(|t| t.signed.seq);
                if let Some(t) = next {
                    self.net.0.insert(*id, t.encode(&p.authority));
                }
                upkeep::Reply::Updated { id: *id, ok }
            }
        }
    }
}

/// What a round with no page open left: upkeep's secrets, the network, what it said, and the file it found (its content,
/// hash, old key and root).
struct AfterRound {
    h: Secrets,
    net: Net,
    said: Vec<String>,
    content: Vec<u8>,
    hsh: [u8; 32],
    old_key: [u8; 32],
    root0: [u8; 32],
    me: [u8; 32],
    ep: Vec<(u64, [u8; 32])>,
}
/// A member removed from a space with one file, then a re-key round woken with no page open — on a network where a
/// contract the delegate makes does (`creates`) or does not reach it.
fn round_after_a_removal(creates: bool) -> AfterRound {
    use craftworks_identity::{Host, Mandate};
    upkeep::CREATES.with(|c| c.set(creates));
    // The member on this node: a real account (its DID, data seed), its MLS member, a space it owns.
    let entropy = [0x61; 16];
    let ev = craftworks_account::inception(&entropy).unwrap();
    let did = ev.id();
    let data_seed = craftworks_account::data_seed(&entropy).unwrap();
    let me_did = craftworks_account::did(&did);
    let mut h = Secrets::default();
    identity::serve(&mut h, Request::Provision { seed: SEED, did, pin: "246810".into(), data: data_seed.to_vec() }, [9; 32]);
    let member = SigningKey::from_bytes(&SEED).verifying_key().to_bytes();
    let mls_seed = identity::space_member_seed(&data_seed);
    let mls_pub = SigningKey::from_bytes(&mls_seed).verifying_key().to_bytes();
    let sm = craftworks_mls::SpaceMember::new(&mls_seed, identity::space_member_credential(&did, &data_seed, &mls_pub), &[]).unwrap();
    let mut g = sm.create_space(SPACE).unwrap();
    let s0 = g.epoch_secret().unwrap();
    let epoch = g.epoch();
    assert!(identity::keep_epoch(&mut h, &member, Some(SPACE), epoch, &s0));
    let m = Mandate {
        space: SPACE, name: "Makers".into(), kind: "server".into(), owner: me_did.clone(), nonce: None, channel: "ch".into(),
        open: false, codes: vec![], level: None, bans: vec![], members: vec![me_did.clone()], epoch, state: g.save().unwrap(),
    };
    identity::upkeep_set_mandate(&mut h, &member, &me_did, &[m]);
    identity::upkeep_set_tick(&mut h, &member);
    h.set_secret(identity::UPKEEP_WAKEUPS, &30u64.to_le_bytes()); // the page last ticked long ago
    identity::upkeep_stir(&mut h, &[7; 32]);
    for (k, v) in [(identity::UPKEEP_TAIL, TAIL), (identity::UPKEEP_BAG, BAG), (identity::UPKEEP_SEALED, SEALED), (identity::UPKEEP_PIECE, PIECE)] {
        h.set_secret(k, v);
    }
    h.set_secret(identity::UPKEEP_BLOCK, &contract_keys::code_hash(BLOCK));

    // The space as the network holds it: written by this member's own feeds (its space writer).
    // A page writes a space under its member (device) key: upkeep the same.
    let me_key = SigningKey::from_bytes(&SEED);
    let ep = [(epoch, s0)];
    let mut n = Host2::default();
    let cat = table::table_name(&SPACE, "tables");
    let mut c = Feed::new(&me_key, &cat, &cat, &ep);
    for t in ["files", "acts"] {
        c.put(&mut n.net, &table::table_name(&SPACE, t), r#"{"b":1}"#, None);
    }
    n.seed_tail(&c.o);
    // A file stored under salt 0, its row keeping the burn secret.
    let content: Vec<u8> = (0..300_000u32).map(|i| (i.wrapping_mul(2654435761) >> 11) as u8).collect();
    let hsh = *blake3::hash(&content).as_bytes();
    let salt0 = [0x70; 32];
    let old_key = files::content_key(&hsh, Some(&salt0));
    let secret0 = recode::burn_secret(&salt0, &old_key);
    let root0 = store_file(&mut n.net, &old_key, &content, &sha256(&secret0));
    let mut f = Feed::new(&me_key, &table::table_name(&SPACE, "files"), &blinded("files"), &ep);
    f.put(&mut n.net, "salt", &serde_json::json!({ "s": table::hex(&salt0), "n": 0, "removals": 0 }).to_string(), None);
    f.put(&mut n.net, "k/file1", &serde_json::json!({ "key": table::hex(&old_key), "root": table::hex(&root0), "b": table::hex(&sha256(&secret0)),
        "x": table::hex(&secret0), "h": table::hex(&hsh), "n": 0, "pub": false, "app": "drive", "at": 1 }).to_string(), None);
    n.seed_tail(&f.o);
    // The acts: someone let in, then REMOVED (the group no longer holds them: only the owner is in it).
    let mut a = Feed::new(&me_key, &table::table_name(&SPACE, "acts"), &blinded("acts"), &ep);
    a.put(&mut n.net, "a1", &serde_json::json!({ "at": 10, "act": "added", "did": "did:craftec:gone" }).to_string(), None);
    a.put(&mut n.net, "a2", &serde_json::json!({ "at": 20, "act": "remove", "did": "did:craftec:gone" }).to_string(), None);
    n.seed_tail(&a.o);
    writers_bag(&mut n.net, &s0, &[me_key.verifying_key().to_bytes()]);
    // The member's KEY LOG and CARD: its `nodes` name this device (its credential signed by the account's owner key)
    // — how upkeep finds a member's devices, the space's writers.
    let log = craftworks_idlog_contract::Log { events: vec![ev.clone()] };
    n.net.0.insert(upkeep::id_of(&contract_keys::code_hash(IDLOG), &did), log.encode());
    h.set_secret(identity::UPKEEP_IDLOG, &contract_keys::code_hash(IDLOG));
    let cred = identity::credential(&did, &member, &mls_pub, &craftworks_account::owner_seed(&entropy).unwrap());
    let data_key = SigningKey::from_bytes(&data_seed);
    let mut card = data::Open::new(TAIL, &data_key.verifying_key().to_bytes(), "card");
    card.public = true;
    let (seq, hh) = card.prepare(vec![tail::Op::Set { key: b"nodes".to_vec(), value: serde_json::to_vec(&vec![table::hex(&cred)]).unwrap() }]).unwrap();
    let pp = craftec_register_contract::wire::Params::parse(&card.params).unwrap();
    card.commit(data_key.sign(&pp.signed_message(false, seq, &hh)).to_bytes()).unwrap();
    n.net.0.insert(card.id_bytes(), card.state());

    // THE ROUND: woken, every answer routed through upkeep (side by side with admissions).
    let now = 1_800_000_000_000;
    let mut todo = crate::rekey_round::woke(&mut h);
    assert!(!todo.is_empty(), "a round begins: the page is away");
    let mut steps = 0;
    while let Some(io) = (!todo.is_empty()).then(|| todo.remove(0)) {
        steps += 1;
        assert!(steps < 5000, "the round ends");
        let reply = n.answer(&io);
        todo.extend(upkeep::replied(&mut h, reply, now));
    }
    // What upkeep last said (for a failure's message).
    let said: Vec<String> = h.0.iter().filter(|(k, _)| k.windows(4).any(|w| w == b"said")).map(|(_, v)| String::from_utf8_lossy(v).into_owned()).collect();
    AfterRound { h, net: n.net, said, content, hsh, old_key, root0, me: me_key.verifying_key().to_bytes(), ep: ep.to_vec() }
}

/// Read back as any member would: the space's `files`.
fn files_after(a: &AfterRound) -> BTreeMap<Vec<u8>, craftworks_feed::Row> {
    let ep_map: BTreeMap<u64, [u8; 32]> = a.ep.iter().copied().collect();
    let (mut r, first) = Reading::new(&codes(), SPACE, ep_map, vec![a.me], &["files"]);
    run(&mut r, first, &a.net);
    r.rows("files")
}

#[test]
fn with_no_page_open_a_removal_rotates_the_salt_and_the_spaces_files_re_key() {
    let mut a = round_after_a_removal(true);
    let said = &a.said;
    // Read back as any member would: a NEW SALT (made after the removal), the file's row under it, readable whole.
    let files_rows = files_after(&a);
    let st = rekey::salt(&files_rows).expect("a salt");
    assert_eq!((st.n, st.removals), (1, 1), "a new salt after the one removal: {said:?}");
    let row: serde_json::Value = serde_json::from_slice(&files_rows[b"k/file1".as_slice()].value).unwrap();
    let new_key = files::content_key(&a.hsh, Some(&st.s));
    assert_eq!(row["key"].as_str(), Some(table::hex(&new_key).as_str()), "re-keyed under the new salt: {said:?}");
    assert_eq!(row["n"].as_i64(), Some(1));
    let root: [u8; 32] = (0..32).map(|i| u8::from_str_radix(&row["root"].as_str().unwrap()[2 * i..2 * i + 2], 16).unwrap()).collect::<Vec<_>>().try_into().unwrap();
    assert_eq!(read_file(&a.net, &new_key, &root).as_deref(), Some(a.content.as_slice()), "the file reads under its new key");
    // The OLD reference reads nothing: its pieces burned.
    assert!(read_file(&a.net, &a.old_key, &a.root0).is_none(), "the old pieces burned: {said:?}");
    // A late answer to one of its burns is the re-key round's — never taken by an admission round.
    let burn_id = upkeep::id_of(&piece_hash(), &files::hashed_address(&a.old_key, &a.root0));
    assert!(crate::rekey_round::wants(&a.h, &burn_id), "its own put, remembered");
    assert!(upkeep::replied(&mut a.h, upkeep::Reply::Put { id: burn_id, ok: true }, 1_800_000_000_000).is_empty());
}

#[test]
fn where_a_delegates_new_contracts_stay_on_its_node_the_salt_rotates_and_the_file_is_left_to_a_page() {
    // freenet ≤ 0.2.141 (`upkeep::NEW_CONTRACTS_REACH_THE_NETWORK`): the salt is a row of a feed already there — written;
    // the file's new pieces would be new contracts — left to a page, its row and old pieces as they were.
    assert!(!upkeep::NEW_CONTRACTS_REACH_THE_NETWORK);
    let a = round_after_a_removal(upkeep::NEW_CONTRACTS_REACH_THE_NETWORK);
    let said = &a.said;
    let files_rows = files_after(&a);
    let st = rekey::salt(&files_rows).expect("a salt");
    assert_eq!((st.n, st.removals), (1, 1), "the new salt written (an existing feed): {said:?}");
    let row: serde_json::Value = serde_json::from_slice(&files_rows[b"k/file1".as_slice()].value).unwrap();
    assert_eq!(row["key"].as_str(), Some(table::hex(&a.old_key).as_str()), "the row untouched: {said:?}");
    assert_eq!(read_file(&a.net, &a.old_key, &a.root0).as_deref(), Some(a.content.as_slice()), "the file still reads: nothing burned");
    assert!(said.iter().any(|l| l.contains("left to a page")), "it says so: {said:?}");
}
