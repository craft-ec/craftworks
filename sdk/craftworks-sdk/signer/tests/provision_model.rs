//! THE SIGNER'S PROVISIONING THROUGH ITS TABLE (sdk#334; the table is on the
//! issue). A reference model of the table is stepped beside the real `serve`
//! over every starting state (unprovisioned, or provisioned as one of two
//! identities), every pair of events (the Register query; a Provision of the
//! same, a new block code, another Register, another key) and every failure
//! point (the Nth `set_secret` of the first event fails). After every step the
//! signer's answer must be the model's, and:
//!
//! * I1: the signer is unprovisioned or provisioned WHOLE as the model says --
//!   its one reader (`provisioned`) and its Register query agree with the
//!   model, never a key without its Register;
//! * I2: a retry of the same Provision after a failure completes it;
//! * the construction: a Provision makes at most ONE `set_secret`.

use signer::*;
use std::collections::BTreeMap;

/// One provisioning as sent: key, Register code, Register params, Block code.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Prov {
    key: Vec<u8>,
    code: Vec<u8>,
    params: Vec<u8>,
    block: Vec<u8>,
}

fn prov(key: u8, register: u8, block: u8) -> Prov {
    Prov {
        key: vec![key; 32],
        code: format!("register code {register}").into_bytes(),
        params: format!("register params {register} of key {key}").into_bytes(),
        block: format!("block code {block}").into_bytes(),
    }
}

/// What the signer must hold for `p`: hashes, not code.
fn held(p: &Prov) -> Provisioned {
    Provisioned {
        key: p.key.clone(),
        register_params: p.params.clone(),
        register_code_hash: contract_keys::code_hash(&p.code),
        block_code_hash: contract_keys::code_hash(&p.block),
    }
}

/// An in-memory host whose `fail_at`-th `set_secret` (counted within an event)
/// fails, leaving that secret as it was: a host write is all or nothing per
/// secret.
#[derive(Default, Clone)]
struct Mem {
    secrets: BTreeMap<Vec<u8>, Vec<u8>>,
    fail_at: Option<usize>,
    writes: usize,
    fired: bool,
}

impl Host for Mem {
    fn get_secret(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.secrets.get(k).cloned()
    }
    fn set_secret(&mut self, k: &[u8], v: &[u8]) -> bool {
        self.writes += 1;
        if self.fail_at == Some(self.writes) {
            self.fired = true;
            return false;
        }
        self.secrets.insert(k.to_vec(), v.to_vec());
        true
    }
    fn contract_state(&self, _: &[u8; 32]) -> Option<Vec<u8>> {
        None
    }
}

#[derive(Clone, Debug)]
enum Ev {
    Query,
    Provision(Prov),
    /// A genesis Sign under a label (builder#117's labelled Record): the
    /// provisioning is label-independent -- refused NotProvisioned exactly
    /// when the signer is unprovisioned, whichever label.
    Sign(Label),
}

fn request(ev: &Ev) -> Request {
    match ev {
        Ev::Query => Request::Register,
        Ev::Sign(label) => Request::Sign {
            prev: Head { seq: 0, root: [0; 32] },
            next: Next { seq: 1, root: [9; 32], ledger: Vec::new() },
            label: label.clone(),
        },
        Ev::Provision(p) => Request::Provision {
            signing_key: p.key.clone(),
            register_code: p.code.clone(),
            register_params: p.params.clone(),
            block_code: p.block.clone(),
        },
    }
}

/// THE TABLE, as a model: the state is the one provisioning held, or none.
/// `failed`: the event's write failed.
fn model(state: &Option<Prov>, ev: &Ev, failed: bool) -> (Answer, Option<Prov>) {
    match (state, ev) {
        (s, Ev::Query) => (Answer::Register { params: s.as_ref().map(|p| p.params.clone()) }, s.clone()),
        // Only the provisioning half of the Sign rule is the table's: the rest
        // (the record, the head read, the root) is `decide`'s, tested in sign.rs.
        (None, Ev::Sign(_)) => (Answer::Refused(Why::NotProvisioned), None),
        (s @ Some(_), Ev::Sign(_)) => (Answer::Refused(Why::NotProvisioned), s.clone()),
        (None, Ev::Provision(_)) if failed => (Answer::Refused(Why::NotProvisioned), None),
        (None, Ev::Provision(x)) => (Answer::Provisioned, Some(x.clone())),
        (Some(p), Ev::Provision(x)) if x.key != p.key => (Answer::Refused(Why::KeyAlreadyProvisioned), Some(p.clone())),
        (Some(p), Ev::Provision(x)) if (&x.code, &x.params) != (&p.code, &p.params) => {
            (Answer::Refused(Why::RegisterChanged), Some(p.clone()))
        }
        (Some(p), Ev::Provision(_)) if failed => (Answer::Refused(Why::NotProvisioned), Some(p.clone())),
        (Some(_), Ev::Provision(x)) => (Answer::Provisioned, Some(x.clone())),
    }
}

/// I1: what the signer IS (its one reader) and what it SAYS (its Register
/// query) are both the model's state.
fn i1(host: &Mem, state: &Option<Prov>) -> Result<(), String> {
    let mut h = host.clone();
    h.fail_at = None;
    let is = provisioned(&h);
    let says = serve(&mut h, &encode_request(9, &Request::Register), Origin::Local);
    let want_is = state.as_ref().map(held);
    let want_says = Answer::Register { params: state.as_ref().map(|p| p.params.clone()) };
    if is != want_is || says != want_says {
        return Err(format!("holds {is:?} and says {says:?}; the table: {want_is:?}"));
    }
    Ok(())
}

#[test]
fn every_state_event_and_failure_point_follows_the_table() {
    let a = prov(1, 1, 1);
    let events = [
        Ev::Query,
        Ev::Provision(a.clone()),      // the same provisioning
        Ev::Provision(prov(1, 1, 2)),  // same key and Register, a new Block code
        Ev::Provision(prov(1, 2, 1)),  // same key, another Register
        Ev::Provision(prov(2, 2, 1)),  // another key
        Ev::Sign(Label::Head),
        Ev::Sign(Label::Site { app: "notes".into(), contract: [7; 32] }),
    ];
    let starts: [Option<Prov>; 3] = [None, Some(a.clone()), Some(prov(3, 3, 3))];
    let mut cases = 0usize;
    let mut violations: Vec<String> = Vec::new();
    for start in &starts {
        let mut host = Mem::default();
        if let Some(p) = start {
            assert_eq!(serve(&mut host, &encode_request(1, &request(&Ev::Provision(p.clone()))), Origin::Local), Answer::Provisioned);
        }
        i1(&host, start).expect("the starting state");
        for first in &events {
            for second in &events {
                for fail_at in [None, Some(1), Some(2)] {
                    cases += 1;
                    let mut h = host.clone();
                    let mut s = start.clone();
                    for (n, ev) in [first, second].into_iter().enumerate() {
                        h.writes = 0;
                        h.fired = false;
                        h.fail_at = if n == 0 { fail_at } else { None };
                        let got = serve(&mut h, &encode_request(2, &request(ev)), Origin::Local);
                        let (want, next) = model(&s, ev, h.fired);
                        let why = if let Ev::Sign(_) = ev {
                            // Provisioned: anything but NotProvisioned; not: exactly it.
                            let refused = got == Answer::Refused(Why::NotProvisioned);
                            (refused != s.is_none()).then(|| format!("a Sign answered {got:?} with the signer {}", if s.is_some() { "provisioned" } else { "unprovisioned" }))
                        } else if got != want {
                            Some(format!("answered {got:?}, the table {want:?}"))
                        } else if matches!(ev, Ev::Provision(_)) && h.writes > 1 {
                            Some(format!("{} set_secret calls: a Provision is ONE write", h.writes))
                        } else {
                            i1(&h, &next).err()
                        };
                        if let Some(why) = why {
                            violations.push(format!("start {:?} | {first:?} then {second:?}, write {fail_at:?} fails | step {n}: {why}", start.as_ref().map(|p| p.key[0])));
                            break;
                        }
                        s = next;
                    }
                    // I2: after a failed Provision, the same Provision again completes.
                    if let (Ev::Provision(_), Some(1)) = (first, fail_at) {
                        let mut r = host.clone();
                        r.fail_at = Some(1);
                        let _ = serve(&mut r, &encode_request(3, &request(first)), Origin::Local);
                        r.fail_at = None;
                        r.writes = 0;
                        let got = serve(&mut r, &encode_request(4, &request(first)), Origin::Local);
                        let (want, next) = model(start, first, false);
                        if got != want {
                            violations.push(format!("I2: the retry of {first:?} answered {got:?}, the table {want:?}"));
                        } else if let Err(e) = i1(&r, &next) {
                            violations.push(format!("I2: after the retry of {first:?}: {e}"));
                        }
                    }
                }
            }
        }
    }
    println!("  {cases} cases (3 starts x {} x {} events x 3 failure points)", events.len(), events.len());
    assert!(violations.is_empty(), "{} of {cases} cases break the table:\n{}", violations.len(), violations.iter().take(12).cloned().collect::<Vec<_>>().join("\n"));
}

/// The record's layout: what is written is read back exactly, and anything
/// that is not exactly one record reads as none.
#[test]
fn the_provisioning_record_reads_back_exactly_and_nothing_else_reads() {
    let p = held(&prov(4, 4, 4));
    let b = p.encode();
    assert_eq!(Provisioned::decode(&b), Some(p.clone()));
    for cut in 0..b.len() {
        assert_eq!(Provisioned::decode(&b[..cut]), None, "a record cut at {cut} read");
    }
    let mut longer = b.clone();
    longer.push(0);
    assert_eq!(Provisioned::decode(&longer), None, "trailing bytes read");
    let mut other = b.clone();
    other[0] ^= 0xFF;
    assert_eq!(Provisioned::decode(&other), None, "another layout version read");
    println!("  a provisioning record: {} B", b.len());
}
