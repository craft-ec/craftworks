//! sdk#209's acceptance, natively: `serve` over an in-memory Host (a secret map + the node's local contract states).
use contract_keys::register::head_of;
use signer::*;
use std::collections::BTreeMap;

const RCODE: &[u8] = b"a register contract's code, as provisioned";
const BCODE: &[u8] = b"a block contract's code, as provisioned";

#[derive(Default, Clone)]
struct Mem {
    secrets: BTreeMap<Vec<u8>, Vec<u8>>,
    states: BTreeMap<[u8; 32], Vec<u8>>,
    /// A `set_secret` of the RECORD fails (the store is full, or the node refuses).
    record_write_fails: bool,
}
impl Host for Mem {
    fn get_secret(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.secrets.get(k).cloned()
    }
    fn set_secret(&mut self, k: &[u8], v: &[u8]) -> bool {
        if self.record_write_fails && k == RECORD {
            return false;
        }
        self.secrets.insert(k.to_vec(), v.to_vec());
        true
    }
    fn contract_state(&self, id: &[u8; 32]) -> Option<Vec<u8>> {
        self.states.get(id).cloned()
    }
}

struct World {
    host: Mem,
    params: Vec<u8>,
}

impl World {
    fn new() -> World {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let params = wire::register_params(&sk.verifying_key().to_bytes(), wire::HEAD_NAME);
        let mut host = Mem::default();
        let a = serve(
            &mut host,
            &encode_request(
                1,
                &Request::Provision {
                    signing_key: sk.to_bytes().to_vec(),
                    register_code: RCODE.to_vec(),
                    register_params: params.clone(),
                    block_code: BCODE.to_vec(),
                },
            ), Origin::Local,
        );
        assert_eq!(a, Answer::Provisioned);
        World { host, params }
    }
    /// The node holds this tree's root block: its real state (`kind ‖ body`), which hashes to the root.
    fn hold_root(&mut self, root: [u8; 32]) {
        let n = (0..=255u8)
            .find(|n| block_root(*n) == root)
            .expect("a root made by `root(n)`");
        self.host.states.insert(
            contract_keys::block::contract_for(BCODE, &root),
            block_state(n),
        );
    }
    /// The page's UPDATE of the Register landed: the node's local state is now this signed record.
    fn land(&mut self, signed: &[u8]) {
        self.host
            .states
            .insert(register_id(RCODE, &self.params), signed.to_vec());
    }
    fn sign(&mut self, prev: Head, next: &Next) -> Answer {
        serve(
            &mut self.host,
            &encode_request(
                1,
                &Request::Sign {
                    prev,
                    next: next.clone(),
                    label: Label::Head,
                },
            ), Origin::Local,
        )
    }
}

fn block_state(n: u8) -> Vec<u8> {
    let mut st = vec![freenet_prolly::kind::RAW];
    st.extend_from_slice(&[n; 40]);
    st
}
fn block_root(n: u8) -> [u8; 32] {
    let st = block_state(n);
    freenet_prolly::block_id(st[0], &st[1..])
}
/// A root that names a real block (the signer checks the held state hashes to it).
fn root(n: u8) -> [u8; 32] {
    block_root(n)
}
fn genesis() -> Head {
    Head {
        seq: 0,
        root: root(0),
    }
}
fn next(seq: u64, r: u8) -> Next {
    Next {
        seq,
        root: root(r),
        ledger: Vec::new(),
    }
}
fn signed(a: &Answer) -> Vec<u8> {
    match a {
        Answer::Signed(b) | Answer::AlreadySigned(b) => b.clone(),
        other => panic!("not a signature: {other:?}"),
    }
}

/// FAIL CLOSED FOR THE HEAD too (engineer4, the same class #332 closed for sites): a head RECORD that is there and
/// does not decode refuses `CannotSign` and writes nothing -- read as "no record yet", a damaged record would let a
/// prev already signed from be signed again (a fork). THE CONTROL: with no record at all, the same genesis signs.
#[test]
fn an_undecodable_head_record_refuses_and_writes_nothing() {
    let mut w = World::new();
    w.hold_root(root(1));
    let mut damaged = w.host.clone();
    damaged.secrets.insert(RECORD.to_vec(), b"garbage".to_vec());
    let before = damaged.secrets.clone();
    let got = serve(&mut damaged, &encode_request(1, &Request::Sign { prev: genesis(), next: next(1, 1), label: Label::Head }), Origin::Local);
    assert_eq!(got, Answer::Refused(Why::CannotSign), "a damaged head record was read as none");
    assert_eq!(damaged.secrets, before, "a refused sign changed the secrets");
    let a = w.sign(genesis(), &next(1, 1));
    assert!(matches!(a, Answer::Signed(_)), "THE CONTROL: with no record the genesis did not sign: {a:?}");
}

#[test]
fn a_head_is_signed_from_genesis_and_reads_back_as_that_head() {
    let mut w = World::new();
    w.hold_root(root(1));
    let a = w.sign(genesis(), &next(1, 1));
    assert!(matches!(a, Answer::Signed(_)), "{a:?}");
    assert_eq!(
        head_of(&signed(&a)),
        Some((1, root(1))),
        "the signed state is not the head it was asked for"
    );
}

#[test]
fn an_identical_re_ask_gets_the_same_bytes_back() {
    let mut w = World::new();
    w.hold_root(root(1));
    let first = w.sign(genesis(), &next(1, 1));
    let again = w.sign(genesis(), &next(1, 1));
    assert_eq!(again, Answer::AlreadySigned(signed(&first)));
}

/// Two requests from the same prev, SEQUENTIAL here (the delegate runs one call at a time): the second is told what
/// won, BEFORE any UPDATE leaves, and never gets a signature of its own. The real race (two connections at once) is
/// `live-signer`'s.
#[test]
fn two_requests_from_one_prev_in_sequence_get_one_signature() {
    let mut w = World::new();
    w.hold_root(root(1));
    w.hold_root(root(2));
    let a = w.sign(genesis(), &next(1, 1));
    let b = w.sign(genesis(), &next(1, 2));
    assert!(matches!(a, Answer::Signed(_)));
    assert_eq!(
        b,
        Answer::AlreadySigned(signed(&a)),
        "the loser was not handed the winner's record"
    );
    assert_eq!(
        head_of(&signed(&b)),
        Some((1, root(1))),
        "the loser learns the WINNER's root"
    );
}

#[test]
fn the_head_moves_on_once_the_signed_record_has_landed() {
    let mut w = World::new();
    w.hold_root(root(1));
    w.hold_root(root(2));
    let a = w.sign(genesis(), &next(1, 1));
    w.land(&signed(&a));
    let b = w.sign(
        Head {
            seq: 1,
            root: root(1),
        },
        &next(2, 2),
    );
    assert!(matches!(b, Answer::Signed(_)), "{b:?}");
    // And the old prev is now stale: told the current head, not signed again.
    let c = w.sign(genesis(), &next(1, 2));
    assert_eq!(
        c,
        Answer::NotNext {
            current: Head {
                seq: 2,
                root: root(2)
            }
        }
    );
}

/// The signer does not need the page's UPDATE to have landed to move on: its own record is truth.
#[test]
fn the_record_alone_is_enough_to_move_on() {
    let mut w = World::new();
    w.hold_root(root(1));
    w.hold_root(root(2));
    let _ = w.sign(genesis(), &next(1, 1));
    let b = w.sign(
        Head {
            seq: 1,
            root: root(1),
        },
        &next(2, 2),
    );
    assert!(matches!(b, Answer::Signed(_)), "{b:?}");
}

#[test]
fn a_head_read_ahead_of_the_record_is_the_truth() {
    let mut w = World::new();
    w.hold_root(root(1));
    w.hold_root(root(9));
    let _ = w.sign(genesis(), &next(1, 1));
    // Another holder of the key signed further (seq 5): the Register shows it.
    let other = contract_keys::register::head_state(&w.params, &[7u8; 32], 5, &root(5)).unwrap();
    w.land(&other);
    assert_eq!(
        w.sign(
            Head {
                seq: 1,
                root: root(1)
            },
            &next(2, 9)
        ),
        Answer::NotNext {
            current: Head {
                seq: 5,
                root: root(5)
            }
        }
    );
    assert!(matches!(
        w.sign(
            Head {
                seq: 5,
                root: root(5)
            },
            &next(6, 9)
        ),
        Answer::Signed(_)
    ));
}

/// The Register at the SAME seq as the record, a different root: another device of this identity signed a competing
/// head and the Register's tie-break kept THEIRS. The same identity never forks (owner, sdk#225): the Register's head is
/// the truth. Signing goes on from THEIR root; from MY root the answer is `NotNext{their head}`, so this signer never
/// displaces the other write (sdk#210 review §1) and is never stuck either.
#[test]
fn equal_seq_different_roots_the_register_wins_and_signing_goes_on_from_it() {
    let mut w = World::new();
    w.hold_root(root(1));
    w.hold_root(root(3));
    let _ = w.sign(genesis(), &next(1, 1));
    let other = contract_keys::register::head_state(&w.params, &[7u8; 32], 1, &root(4)).unwrap();
    w.land(&other);
    let theirs = Head {
        seq: 1,
        root: root(4),
    };
    assert_eq!(
        w.sign(
            Head {
                seq: 1,
                root: root(1)
            },
            &next(2, 3)
        ),
        Answer::NotNext { current: theirs },
        "signed on from MY root, which the Register's tie-break displaced"
    );
    assert!(
        matches!(w.sign(theirs, &next(2, 3)), Answer::Signed(_)),
        "did not sign on from THEIR root: this key is stuck"
    );
}

#[test]
fn every_refusal_is_named() {
    let mut w = World::new();
    assert_eq!(
        w.sign(genesis(), &next(1, 1)),
        Answer::Refused(Why::RootNotHeld)
    );
    w.hold_root(root(1));
    assert_eq!(
        w.sign(genesis(), &next(2, 1)),
        Answer::Refused(Why::NotSuccessor)
    );
    assert_eq!(
        w.sign(
            Head {
                seq: 4,
                root: root(4)
            },
            &next(5, 1)
        ),
        Answer::Refused(Why::HeadUnknown)
    );
    assert_eq!(
        serve(&mut w.host, b"\x09junk", Origin::Local),
        Answer::Refused(Why::Unreadable)
    );
    assert_eq!(serve(&mut w.host, &[], Origin::Local), Answer::Refused(Why::Unreadable));
    let mut bare = Mem::default();
    assert_eq!(
        serve(
            &mut bare,
            &encode_request(
                1,
                &Request::Sign {
                    prev: genesis(),
                    next: next(1, 1),
                    label: Label::Head,
                }
            )
        , Origin::Local),
        Answer::Refused(Why::NotProvisioned)
    );
    let other = Request::Provision {
        signing_key: vec![8u8; 32],
        register_code: RCODE.to_vec(),
        register_params: w.params.clone(),
        block_code: BCODE.to_vec(),
    };
    assert_eq!(
        serve(&mut w.host, &encode_request(1, &other), Origin::Local),
        Answer::Refused(Why::KeyAlreadyProvisioned)
    );
}

/// NO SIGNATURE WITHOUT ITS RECORD: a record that cannot be written leaves no signature behind, and the same request
/// is signed once the write can succeed.
#[test]
fn a_record_that_cannot_be_written_returns_no_signature() {
    let mut w = World::new();
    w.hold_root(root(1));
    w.host.record_write_fails = true;
    assert_eq!(
        w.sign(genesis(), &next(1, 1)),
        Answer::Refused(Why::RecordNotSaved)
    );
    w.host.record_write_fails = false;
    assert!(matches!(w.sign(genesis(), &next(1, 1)), Answer::Signed(_)));
}

/// THE CRASH ORDERINGS. After a signature at seq 1 the node goes away (the record kept or lost; the head landed or
/// not) and the page re-asks from the same prev with a DIFFERENT next. Counted: the distinct signatures at seq 1.
/// Three orderings yield ONE. The fourth -- the record LOST after it was written and the head never landed -- yields
/// two: it is exactly the platform fact sdk#209's live test measures (is set_secret durable before the reply), pinned
/// here so the dependency is stated rather than assumed.
#[test]
fn only_a_lost_record_with_an_unlanded_head_can_sign_twice_at_one_seq() {
    for (record_kept, head_landed) in [(true, true), (true, false), (false, true), (false, false)] {
        let mut w = World::new();
        w.hold_root(root(1));
        w.hold_root(root(2));
        let first = signed(&w.sign(genesis(), &next(1, 1)));
        if head_landed {
            w.land(&first);
        }
        if !record_kept {
            w.host.secrets.remove(RECORD);
        }
        let again = w.sign(genesis(), &next(1, 2));
        let second = match &again {
            Answer::Signed(b) | Answer::AlreadySigned(b) => Some(b.clone()),
            _ => None,
        };
        let distinct: std::collections::BTreeSet<Vec<u8>> = [Some(first.clone()), second]
            .into_iter()
            .flatten()
            .filter(|b| head_of(b).map(|(s, _)| s) == Some(1))
            .collect();
        println!(
            "  record kept {record_kept}, head landed {head_landed}: re-ask told {}; distinct signatures at seq 1: {}",
            match &again {
                Answer::Signed(_) => "Signed".to_string(),
                a => format!("{a:?}").chars().take(40).collect(),
            },
            distinct.len()
        );
        let expect = if !record_kept && !head_landed { 2 } else { 1 };
        assert_eq!(
            distinct.len(),
            expect,
            "record kept {record_kept}, head landed {head_landed}"
        );
    }
}

/// "Root held" means the RIGHT block: a state under the root's contract that does not hash to the root is refused.
#[test]
fn a_held_state_that_is_not_the_root_block_is_refused() {
    let mut w = World::new();
    let r = root(1);
    w.host.states.insert(
        contract_keys::block::contract_for(BCODE, &r),
        block_state(2),
    );
    assert_eq!(
        w.sign(genesis(), &next(1, 1)),
        Answer::Refused(Why::RootNotHeld)
    );
}

/// A different Register for the same key is refused: the one record is never carried to another Register.
#[test]
fn the_same_key_with_another_register_is_refused() {
    let w0 = World::new();
    let mut w = w0;
    let again = |params: Vec<u8>, code: &[u8]| Request::Provision {
        signing_key: ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])
            .to_bytes()
            .to_vec(),
        register_code: code.to_vec(),
        register_params: params,
        block_code: BCODE.to_vec(),
    };
    let same = serve(
        &mut w.host,
        &encode_request(1, &again(w.params.clone(), RCODE)), Origin::Local,
    );
    assert_eq!(
        same,
        Answer::Provisioned,
        "re-provisioning the SAME Register must still work"
    );
    let mut other = w.params.clone();
    let last = other.len() - 1;
    other[last] ^= 1;
    assert_eq!(
        serve(&mut w.host, &encode_request(1, &again(other, RCODE)), Origin::Local),
        Answer::Refused(Why::RegisterChanged)
    );
    assert_eq!(
        serve(
            &mut w.host,
            &encode_request(1, &again(w.params.clone(), b"another register code"))
        , Origin::Local),
        Answer::Refused(Why::RegisterChanged)
    );
}

/// READ-LOCAL: present / absent per contract, in order, from the node's local states alone -- so a block the node
/// holds is `true` and one it does not is `false`, whatever order they are asked in. Needs no provisioning.
#[test]
fn held_says_which_contracts_this_node_holds_in_the_order_asked() {
    let mut host = Mem::default();
    let have = contract_keys::block::contract_for(BCODE, &block_root(1));
    let lack = contract_keys::block::contract_for(BCODE, &block_root(2));
    host.states.insert(have, block_state(1));
    let ask = |host: &mut Mem, contracts: Vec<[u8; 32]>| {
        serve(
            &mut host.clone(),
            &encode_request(1, &Request::Held { contracts }), Origin::Local,
        )
    };
    assert_eq!(
        ask(&mut host, vec![have, lack, have]),
        Answer::Held {
            present: vec![true, false, true]
        }
    );
    assert_eq!(
        ask(&mut host, vec![lack, have]),
        Answer::Held {
            present: vec![false, true]
        }
    );
    assert_eq!(
        ask(&mut host, vec![]),
        Answer::Refused(Why::BlockCount { max: 128, got: 0 })
    );
    assert_eq!(
        ask(&mut host, vec![have; 129]),
        Answer::Refused(Why::BlockCount { max: 128, got: 129 })
    );
}

/// SG02: every answer carries ITS request's id, so answers to requests in flight together are attributed by id
/// alone -- two `Held`s, answered out of order, each named by what it asked. An answer's variant cannot do this:
/// `Held{present}` names no contract.
#[test]
fn two_requests_in_flight_are_each_answered_under_their_own_id() {
    let mut host = Mem::default();
    let have = contract_keys::block::contract_for(BCODE, &block_root(1));
    let lack = contract_keys::block::contract_for(BCODE, &block_root(2));
    host.states.insert(have, block_state(1));
    let (a, b) = (7u32, 9u32);
    let ask_a = encode_request(
        a,
        &Request::Held {
            contracts: vec![have],
        },
    );
    let ask_b = encode_request(
        b,
        &Request::Held {
            contracts: vec![lack],
        },
    );
    // Served B first, as a node may: the answers arrive in the other order from the asks.
    let replies = [
        reply(&serve_full(&mut host, &ask_b, Origin::Local)),
        reply(&serve_full(&mut host, &ask_a, Origin::Local)),
    ];
    let by_id: std::collections::BTreeMap<u32, Answer> = replies
        .iter()
        .map(|r| wire::signer::read_answer(r).expect("a signer answer"))
        .collect();
    assert_eq!(by_id.len(), 2, "two answers under one id: {by_id:?}");
    assert_eq!(
        by_id.get(&a),
        Some(&Answer::Held {
            present: vec![true]
        }),
        "request {a}'s answer"
    );
    assert_eq!(
        by_id.get(&b),
        Some(&Answer::Held {
            present: vec![false]
        }),
        "request {b}'s answer"
    );
    // A refusal is attributed the same way, and one that does not decode still names the id it carried.
    let bare = reply(&serve_full(
        &mut Mem::default(),
        &encode_request(11, &Request::Held { contracts: vec![] }), Origin::Local,
    ));
    assert_eq!(
        wire::signer::read_answer(&bare),
        Some((11, Answer::Refused(Why::BlockCount { max: 128, got: 0 })))
    );
    let mut broken = encode_request(
        12,
        &Request::Held {
            contracts: vec![have],
        },
    );
    broken.push(0);
    assert_eq!(
        wire::signer::read_answer(&reply(&serve_full(&mut host, &broken, Origin::Local))),
        Some((12, Answer::Refused(Why::Unreadable)))
    );
}

/// The head value's ONE format (signer_proto::head): a value whose ledger is
/// not the format is REFUSED, and nothing is signed, whatever else is right.
#[test]
fn a_malformed_ledger_is_refused_and_nothing_is_signed() {
    let mut w = World::new();
    w.hold_root(root(1));
    let mut bad = next(1, 1);
    bad.ledger = vec![signer_proto::head::LEDGER_VERSION, signer_proto::head::TAG_PREV, 3, 0, 1, 2, 3];
    assert_eq!(w.sign(genesis(), &bad), Answer::Refused(Why::BadLedger));
    // Nothing was signed: the well-formed request from the same prev signs.
    assert!(matches!(w.sign(genesis(), &next(1, 1)), Answer::Signed(_)));
}

/// A ledgered value (PREV filled) is signed; and a REGISTER holding a
/// ledgered head is read by its root, so the signer's truth is right.
#[test]
fn a_prev_ledger_is_signed_and_a_ledgered_register_head_is_read_by_its_root() {
    use signer_proto::head::{value, Ledger};
    let mut w = World::new();
    w.hold_root(root(1));
    w.hold_root(root(2));
    let first = next(1, 1);
    let Answer::Signed(st) = w.sign(genesis(), &first) else { panic!("the genesis head was not signed") };
    w.land(&st);
    let prev = Head { seq: 1, root: root(1) };
    let ledgered = Next { seq: 2, root: root(2), ledger: value(&root(2), &Ledger { prev: Some(prev), ..Ledger::default() })[32..].to_vec() };
    let Answer::Signed(st2) = w.sign(prev, &ledgered) else { panic!("a PREV-ledgered value was not signed") };
    let (seq, v) = signer_proto::head::record_of(&st2).expect("a record");
    let hv = signer_proto::head::read_value(v).expect("a head");
    assert_eq!((seq, hv.root, hv.ledger.prev), (2, root(2), Some(prev)));
    // The register now holds that ledgered head: the signer's truth is it.
    w.land(&st2);
    assert_eq!(w.sign(prev, &next(2, 2)), Answer::AlreadySigned(st2.clone()), "one signature per prev");
    assert_eq!(w.sign(Head { seq: 1, root: root(1) }, &next(3, 2)), Answer::Refused(Why::NotSuccessor));
    assert!(matches!(w.sign(Head { seq: 2, root: root(2) }, &next(3, 1)), Answer::Signed(_) | Answer::Refused(Why::RootNotHeld)), "a ledgered register head was not read as seq 2");
}

/// The signer's HEAD READ on a ledgered head it holds NO record for (another
/// device's, sdk#233): the truth comes from the register alone, so the read
/// must take the root of a `root ‖ ledger` value. The exact-32 reader of
/// before the format read it as no head and answered `HeadUnknown`.
#[test]
fn a_ledgered_register_head_with_no_record_of_mine_is_read_by_its_root() {
    use signer_proto::head::{value, Ledger};
    let mut w = World::new();
    w.hold_root(root(2));
    w.hold_root(root(3));
    let v = value(&root(2), &Ledger { prev: Some(Head { seq: 4, root: root(1) }), ..Ledger::default() });
    let theirs = contract_keys::register::head_state(&w.params, &[7u8; 32], 5, &v).unwrap();
    w.land(&theirs);
    let from = Head { seq: 5, root: root(2) };
    assert!(
        matches!(w.sign(from, &next(6, 3)), Answer::Signed(_)),
        "a ledgered register head was not read: the signer could not sign on from it"
    );
}

/// "WHICH REGISTER DO YOU SIGN FOR?" — none before a key is provisioned; its
/// Register's params after; and never the key. A provisioning record that is
/// not one (a truncated or foreign value) names nothing: never a tree it
/// cannot sign for.
#[test]
fn the_signer_names_the_register_it_signs_for_and_only_with_a_key() {
    let mut fresh = Mem::default();
    assert_eq!(serve(&mut fresh, &encode_request(5, &Request::Register), Origin::Local), Answer::Register { params: None });
    let w = World::new();
    let mut host = w.host.clone();
    let a = serve(&mut host, &encode_request(6, &Request::Register), Origin::Local);
    assert_eq!(a, Answer::Register { params: Some(w.params.clone()) });
    let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    assert!(!format!("{a:?}").contains(&format!("{:?}", sk.to_bytes().to_vec())), "the key left the signer");
    let mut cut = w.host.clone();
    let whole = cut.secrets[PROVISION].clone();
    cut.secrets.insert(PROVISION.to_vec(), whole[..whole.len() - 1].to_vec());
    assert_eq!(serve(&mut cut, &encode_request(7, &Request::Register), Origin::Local), Answer::Register { params: None }, "a cut provisioning record named a Register");
}
