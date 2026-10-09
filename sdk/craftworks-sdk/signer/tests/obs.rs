//! THE OBSERVATION TREE's HEAD (sdk#399, craftworks-docs OBSERVABILITY §1): `Label::Obs`, the same authority as the
//! data head under the name `obs`, signed by the same rule into ITS OWN record, its truth read from ITS register. The
//! harness is sign.rs's (an in-memory Host).
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
    #[allow(dead_code)]
    fn land(&mut self, signed: &[u8]) {
        self.host
            .states
            .insert(register_id(RCODE, &self.params), signed.to_vec());
    }
    fn sign_as(&mut self, prev: Head, next: &Next, label: Label, origin: Origin) -> Answer {
        serve(&mut self.host, &encode_request(1, &Request::Sign { prev, next: next.clone(), label }), origin)
    }
    #[allow(dead_code)]
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

fn obs_params(w: &World) -> Vec<u8> {
    contract_keys::site::obs_params(&w.params).expect("mode-0 params")
}

/// An Obs head signs from genesis, and what it signed is a record of the OBS register -- it verifies under the obs
/// params and not under the data head's (a record that verified under both would be one head for two trees).
#[test]
fn an_obs_head_is_signed_under_the_obs_register() {
    let mut w = World::new();
    w.hold_root(root(1));
    let a = w.sign_as(genesis(), &next(1, 1), Label::Obs, Origin::Local);
    let st = signed(&a);
    assert_eq!(verified_record(&obs_params(&w), &st).map(|(seq, _)| seq), Some(1), "the obs record does not verify under the obs params");
    assert!(verified_record(&w.params, &st).is_none(), "an obs record verified under the DATA head's params");
    assert_eq!(head_of(&st), Some((1, root(1))));
}

/// Head and Obs are two records: each signs its own genesis, neither is AlreadySigned or NotNext by the other, and the
/// obs record is kept under `RECORD/obs` beside the head's.
#[test]
fn the_head_and_the_obs_head_are_separate_records() {
    let mut w = World::new();
    w.hold_root(root(1));
    w.hold_root(root(2));
    let head = w.sign_as(genesis(), &next(1, 1), Label::Head, Origin::Local);
    let obs = w.sign_as(genesis(), &next(1, 2), Label::Obs, Origin::Local);
    assert!(matches!(head, Answer::Signed(_)), "{head:?}");
    assert!(matches!(obs, Answer::Signed(_)), "the obs head was judged by the data head's record: {obs:?}");
    assert!(w.host.secrets.contains_key(RECORD), "the head's record");
    assert!(w.host.secrets.contains_key(&[RECORD, b"/obs"].concat()), "the obs record is not kept under RECORD/obs");
    assert_eq!(record_name(&Label::Obs), [RECORD, b"/obs"].concat());
}

/// A SERVED page signs the obs head as it signs the data head (the page writes its recording from inside any app;
/// a site, the person's own tool's, is the only label refused from an app).
#[test]
fn a_served_page_signs_its_obs_head() {
    let mut w = World::new();
    w.hold_root(root(1));
    let a = w.sign_as(genesis(), &next(1, 1), Label::Obs, Origin::Served);
    assert!(matches!(a, Answer::Signed(_)), "a served page could not sign its obs head: {a:?}");
}

/// The obs head's TRUTH is its own register: once an obs record lands there, a sign from genesis is NotNext at the
/// OBS head -- and the data head's register says nothing about it.
#[test]
fn the_obs_heads_truth_is_its_own_register() {
    let mut w = World::new();
    w.hold_root(root(1));
    let a = w.sign_as(genesis(), &next(1, 1), Label::Obs, Origin::Local);
    let obs = obs_params(&w);
    w.host.states.insert(register_id(RCODE, &obs), signed(&a));
    let again = w.sign_as(genesis(), &next(1, 1), Label::Obs, Origin::Local);
    assert!(matches!(again, Answer::AlreadySigned(_) | Answer::NotNext { .. }), "the landed obs head was not the truth: {again:?}");
    // The data head is untouched: its genesis still signs.
    let head = w.sign_as(genesis(), &next(1, 1), Label::Head, Origin::Local);
    assert!(matches!(head, Answer::Signed(_)), "the obs head's landing moved the data head: {head:?}");
}
