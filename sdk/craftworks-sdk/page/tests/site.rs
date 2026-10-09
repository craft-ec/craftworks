//! A SITE THROUGH THE PAGE'S ONE HEAD PATH (builder#117; the module table's Site column). The far side is the
//! REAL signer (`signer::serve`, its one `decide`) and the REAL Register merge under the site's params (the site
//! contract's merge is the Register's), never a model of either. Each case names the mutant it turns red on.
use craftec_register_contract::Register;
use engine::Params;
use freenet_stdlib::prelude::*;
use page::{Answer, HeadRead, Label, Ms, Op, Page, Publication, PutPath};
use std::collections::BTreeMap;

const RCODE: &[u8] = b"a register contract's code, as provisioned";
const BCODE: &[u8] = b"a block contract's code, as provisioned";
/// The site contract's id, as page-io would supply it (the signer reads its state there).
const SITE: [u8; 32] = [0x51; 32];
const APP: &str = "notes";

/// One device's signer: its secrets, and the node's site state as that node holds it locally.
#[derive(Default, Clone)]
struct Mem {
    secrets: BTreeMap<Vec<u8>, Vec<u8>>,
    states: BTreeMap<[u8; 32], Vec<u8>>,
}
impl signer::Host for Mem {
    fn get_secret(&self, k: &[u8]) -> Option<Vec<u8>> {
        self.secrets.get(k).cloned()
    }
    fn set_secret(&mut self, k: &[u8], v: &[u8]) -> bool {
        self.secrets.insert(k.to_vec(), v.to_vec());
        true
    }
    fn contract_state(&self, id: &[u8; 32]) -> Option<Vec<u8>> {
        self.states.get(id).cloned()
    }
}

fn key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])
}
fn register_params() -> Vec<u8> {
    wire::register_params(&key().verifying_key().to_bytes(), wire::HEAD_NAME)
}
fn site_params() -> Vec<u8> {
    contract_keys::site::site_params(&register_params(), APP).expect("site params")
}
/// A signer provisioned with THE person's key (every device of one person holds it: rule 15).
fn device() -> Mem {
    let mut host = Mem::default();
    let req = signer::Request::Provision { signing_key: key().to_bytes().to_vec(), register_code: RCODE.to_vec(), register_params: register_params(), block_code: BCODE.to_vec() };
    assert_eq!(signer::serve(&mut host, &signer::encode_request(1, &req), signer::Origin::Local), signer::Answer::Provisioned);
    host
}
fn bundle(n: u8) -> [u8; 32] {
    *blake3::hash(&[n; 100]).as_bytes()
}

/// The network's copy of the site: the Register state under the site's params, merged by the REAL contract.
#[derive(Default)]
struct Node {
    meta: Option<Vec<u8>>,
}
impl Node {
    fn put(&mut self, state: &[u8]) {
        let params = Parameters::from(site_params());
        self.meta = Some(match &self.meta {
            None => {
                let v = <Register as ContractInterface>::validate_state(params, State::from(state.to_vec()), RelatedContracts::default());
                assert!(matches!(v, Ok(ValidateResult::Valid)), "the signer's site record is not a valid Register state under the site params");
                state.to_vec()
            }
            Some(cur) => <Register as ContractInterface>::update_state(params, State::from(cur.clone()), vec![UpdateData::State(State::from(state.to_vec()))])
                .expect("the site merged")
                .new_state
                .expect("a state")
                .as_ref()
                .to_vec(),
        });
    }
    fn read(&self) -> Option<HeadRead> {
        self.meta.as_deref().and_then(HeadRead::from_record)
    }
    fn seq(&self) -> Option<u64> {
        self.read().map(|h| h.seq)
    }
}

/// What the far side does to one op, for a case to script.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fate {
    Answer,
    /// No answer (a refused GET, a lost frame): the page's RTO re-asks.
    Silence,
}

/// One publisher: its page and its device's signer.
struct Publisher {
    page: Page,
    host: Mem,
    origin: signer::Origin,
    /// Head reads this publisher's node answers from a STALE copy (a peered node behind the network): `Some(x)`
    /// answers `x` for the next read and is then spent.
    stale_read: Option<Option<Vec<u8>>>,
    /// Sign ops seen, and whether any op touched the head (the engine's).
    signs: usize,
    head_ops: usize,
    /// Sign asks this device's OWN node answers before it holds the site (the signer's local read lags: a new
    /// device's node fetches the site with the page's first read, and serves it locally only after that).
    blind_signs: usize,
    /// Sign asks answered `Refused(RootNotHeld)` before the signer is asked (a signer that says it of a site: today's
    /// never does, but the ONE retryable set, sdk#486, decides what a page does if one does).
    root_not_held: usize,
}
impl Publisher {
    fn new(host: Mem) -> Publisher {
        Publisher { page: Page::new(Params::default(), PutPath::Page), host, origin: signer::Origin::Local, stale_read: None, signs: 0, head_ops: 0, blind_signs: 0, root_not_held: 0 }
    }
    fn publication(&self) -> Option<Publication> {
        self.page.publication(APP)
    }
    /// One round: every op the page has out is served against `node` (the signer's local view of the site is
    /// the node's own), `fate` deciding per op.
    fn step(&mut self, node: &mut Node, now: u64, fate: &mut dyn FnMut(&Op) -> Fate) {
        self.page.tick(Ms(now));
        for op in self.page.take_ops() {
            let site = Label::Site(APP.into());
            let a = match &op {
                Op::ReadHead { label } if *label == site => {
                    let read = match self.stale_read.take() {
                        Some(stale) => stale.as_deref().and_then(HeadRead::from_record),
                        None => node.read(),
                    };
                    Answer::Head { label: site, read }
                }
                Op::Sign { id, .. } if *label_of(&op) == site && self.root_not_held > 0 => {
                    self.signs += 1;
                    self.root_not_held -= 1;
                    Answer::Signer { id: *id, answer: signer_proto::Answer::Refused(signer_proto::Why::RootNotHeld) }
                }
                Op::Sign { id, prev_seq, prev_root, seq, root, ledger, label } if *label == site => {
                    self.signs += 1;
                    let blind = self.blind_signs > 0;
                    self.blind_signs = self.blind_signs.saturating_sub(1);
                    match &node.meta {
                        Some(m) if !blind => {
                            self.host.states.insert(SITE, contract_keys::site::frame(m, b""));
                        }
                        _ => {
                            self.host.states.remove(&SITE);
                        }
                    }
                    let req = signer::Request::Sign {
                        prev: signer::Head { seq: *prev_seq, root: *prev_root },
                        next: signer::Next { seq: *seq, root: *root, ledger: ledger.clone() },
                        label: signer::Label::Site { app: APP.into(), contract: SITE },
                    };
                    Answer::Signer { id: *id, answer: signer::serve(&mut self.host, &signer::encode_request(*id, &req), self.origin) }
                }
                Op::Update { label, state } if *label == site => {
                    if fate(&op) == Fate::Answer {
                        node.put(state);
                    }
                    Answer::Updated { label: site }
                }
                Op::ReadHead { label: Label::Head } | Op::Update { label: Label::Head, .. } | Op::Sign { label: Label::Head, .. } => {
                    self.head_ops += 1;
                    continue;
                }
                _ => continue,
            };
            if fate(&op) == Fate::Answer {
                self.page.answer(a, Ms(now));
            }
        }
    }
}

/// A Sign op's label.
fn label_of(op: &Op) -> &Label {
    match op {
        Op::Sign { label, .. } => label,
        other => panic!("not a sign: {other:?}"),
    }
}

fn always(_: &Op) -> Fate {
    Fate::Answer
}

/// Run until every publisher's publication ends, or `rounds` pass; 50 ms a round.
fn run(ps: &mut [&mut Publisher], node: &mut Node, now: &mut u64, rounds: usize, fate: &mut dyn FnMut(&Op) -> Fate) {
    for _ in 0..rounds {
        *now += 50;
        for p in ps.iter_mut() {
            p.step(node, *now, fate);
        }
        if ps.iter().all(|p| !matches!(p.publication(), Some(Publication::Publishing { .. }))) {
            return;
        }
    }
}

/// **No site yet (NotFound) -> version 1, Published only once the read-back shows it.** And the site never
/// feeds the engine: the page's own head does not move and no head op goes out. Mutant "a site read is the
/// head's" -> the head moves -> red.
#[test]
fn a_first_publish_is_version_one_and_never_touches_the_head() {
    let (mut node, mut now) = (Node::default(), 1_000);
    let mut a = Publisher::new(device());
    let head_before = a.page.published();
    a.page.publish_site(APP, bundle(1), Ms(now));
    let start = now;
    run(&mut [&mut a], &mut node, &mut now, 100, &mut always);
    assert_eq!(a.publication(), Some(Publication::Published { version: 1 }));
    assert_eq!(node.read().map(|h| (h.seq, h.value().to_vec())), Some((1, bundle(1).to_vec())), "the node does not hold (1, blake3(web))");
    assert_eq!(a.page.published(), head_before, "a site's read moved the page's HEAD");
    // THE CONTROL: a page that publishes nothing, over the same rounds, sends the same head ops (its own recovery).
    let mut idle = Publisher::new(device());
    let (mut nothing, mut t) = (Node::default(), start);
    for _ in 0..(now - start) / 50 {
        t += 50;
        idle.step(&mut nothing, t, &mut always);
    }
    assert_eq!(a.head_ops, idle.head_ops, "publishing a site sent head ops of its own");
}

/// **A published site's next publish is the next version**, read from the site (no local counter).
#[test]
fn each_publish_is_the_next_version() {
    let (mut node, mut now) = (Node::default(), 1_000);
    let mut a = Publisher::new(device());
    for (n, v) in [(1u8, 1u64), (2, 2), (3, 3)] {
        a.page.publish_site(APP, bundle(n), Ms(now));
        run(&mut [&mut a], &mut node, &mut now, 100, &mut always);
        assert_eq!(a.publication(), Some(Publication::Published { version: v }), "publish {n}");
    }
    // Another device of the same person, knowing nothing, follows the site: v4.
    let mut b = Publisher::new(device());
    b.page.publish_site(APP, bundle(4), Ms(now));
    run(&mut [&mut b], &mut node, &mut now, 100, &mut always);
    assert_eq!(b.publication(), Some(Publication::Published { version: 4 }));
}

/// **A silent read signs nothing** (a refused GET is silence, re-asked on the RTO): version 1 is signed only
/// after the read is answered.
#[test]
fn a_silent_site_read_signs_nothing_until_answered() {
    let (mut node, mut now) = (Node::default(), 1_000);
    let mut a = Publisher::new(device());
    a.page.publish_site(APP, bundle(1), Ms(now));
    let mut reads = 0;
    let mut silent_first_three = |op: &Op| match op {
        Op::ReadHead { .. } => {
            reads += 1;
            if reads <= 3 {
                Fate::Silence
            } else {
                Fate::Answer
            }
        }
        _ => Fate::Answer,
    };
    run(&mut [&mut a], &mut node, &mut now, 4_000, &mut silent_first_three);
    assert!(reads >= 4, "the silent read was not re-asked ({reads} reads)");
    assert_eq!(a.publication(), Some(Publication::Published { version: 1 }));
    assert_eq!(a.signs, 1, "a silent read was signed from ({} signs)", a.signs);
}

/// **THE LIVELOCK ENDS (item 2).** The signer recorded v2 for bundle A and A's PUT was lost; the node still reads
/// v1. Publishing B asks from v1 -> AlreadySigned(A's record), a FOREIGN value -> the page asks FROM that record
/// -> Signed v3 -> Published v3. Mutant "re-ask from the same prev" -> never ends -> red.
#[test]
fn a_record_whose_put_was_lost_is_passed_from_the_record() {
    let (mut node, mut now) = (Node::default(), 1_000);
    let mut a = Publisher::new(device());
    a.page.publish_site(APP, bundle(1), Ms(now));
    run(&mut [&mut a], &mut node, &mut now, 100, &mut always);
    assert_eq!(a.publication(), Some(Publication::Published { version: 1 }));
    // v2 of bundle A: signed, its PUT lost.
    a.page.publish_site(APP, bundle(2), Ms(now));
    let mut lose_puts = |op: &Op| if matches!(op, Op::Update { .. }) { Fate::Silence } else { Fate::Answer };
    run(&mut [&mut a], &mut node, &mut now, 10, &mut lose_puts);
    assert_eq!(node.seq(), Some(1), "THE SETUP: the lost PUT reached the node");
    a.page.cancel_site(APP);
    // Bundle B: the node reads v1, the signer holds v2 (A).
    let signs_before = a.signs;
    a.page.publish_site(APP, bundle(3), Ms(now));
    run(&mut [&mut a], &mut node, &mut now, 200, &mut always);
    assert_eq!(a.publication(), Some(Publication::Published { version: 3 }), "the lost record was not passed (livelock)");
    assert_eq!(a.signs - signs_before, 2, "the livelock took other than two asks (AlreadySigned, then Signed)");
}

/// **NotNext -> ask from `current`.** The node this page reads answers a STALE v1 while the signer's own node
/// holds v3 (another device's): the ask from v1 is NotNext { v3 }, the page asks from it, and v4 is published.
#[test]
fn a_stale_read_is_passed_by_the_signers_current() {
    let (mut node, mut now) = (Node::default(), 1_000);
    let mut other = Publisher::new(device());
    for n in 1..=3u8 {
        other.page.publish_site(APP, bundle(n), Ms(now));
        run(&mut [&mut other], &mut node, &mut now, 100, &mut always);
    }
    assert_eq!(node.seq(), Some(3));
    // This device's page reads a stale v1 (as the node held it after publish 1).
    let mut v1 = Node::default();
    let mut first = Publisher::new(device());
    first.page.publish_site(APP, bundle(1), Ms(now));
    run(&mut [&mut first], &mut v1, &mut now, 100, &mut always);
    let mut a = Publisher::new(device());
    a.stale_read = Some(v1.meta.clone());
    a.page.publish_site(APP, bundle(9), Ms(now));
    run(&mut [&mut a], &mut node, &mut now, 200, &mut always);
    assert_eq!(a.publication(), Some(Publication::Published { version: 4 }));
}

/// **Two publishers at one version: exactly one Published, the other Superseded** (the merge keeps the lower
/// blake3(value); the loser reports, never overwrites). Mutant "Published on the PUT's ack" -> both Published
/// -> red.
#[test]
fn two_publishers_at_one_version_end_one_published_one_superseded() {
    let (mut node, mut now) = (Node::default(), 1_000);
    // Two devices, each with no record: both sign v1 from the genesis.
    let (mut a, mut b) = (Publisher::new(device()), Publisher::new(device()));
    a.page.publish_site(APP, bundle(1), Ms(now));
    b.page.publish_site(APP, bundle(2), Ms(now));
    run(&mut [&mut a, &mut b], &mut node, &mut now, 200, &mut always);
    let ends = [a.publication(), b.publication()];
    let published = ends.iter().filter(|e| matches!(e, Some(Publication::Published { version: 1 }))).count();
    let superseded = ends.iter().filter(|e| matches!(e, Some(Publication::Superseded { version: 1 }))).count();
    assert_eq!((published, superseded), (1, 1), "two publishers at v1 ended {ends:?}");
    // The Published one is the one the node holds.
    let live = node.read().expect("a site").value().to_vec();
    let winner = if published == 1 && matches!(a.publication(), Some(Publication::Published { .. })) { bundle(1) } else { bundle(2) };
    assert_eq!(live, winner.to_vec(), "Published names a bundle the node does not hold");
}

/// **A stale v1 against a node at v7 is Superseded { 7 }**, never an overwrite: the node's copy read absent (a
/// false NotFound) and the signer here holds nothing, so v1 is signed; the merge keeps v7.
#[test]
fn a_publish_behind_the_network_is_superseded_by_it() {
    let (mut node, mut now) = (Node::default(), 1_000);
    let mut other = Publisher::new(device());
    for n in 1..=7u8 {
        other.page.publish_site(APP, bundle(n), Ms(now));
        run(&mut [&mut other], &mut node, &mut now, 100, &mut always);
    }
    let mut a = Publisher::new(device());
    a.stale_read = Some(None);
    // Its signer's node does not hold the site either (a new device, peered).
    let mut empty = Node::default();
    a.page.publish_site(APP, bundle(20), Ms(now));
    // The first read (stale: none) and the sign run against the empty local node; the PUT and read-back against the network.
    a.step(&mut empty, now + 50, &mut always);
    a.step(&mut empty, now + 100, &mut always);
    now += 100;
    run(&mut [&mut a], &mut node, &mut now, 200, &mut always);
    assert_eq!(a.publication(), Some(Publication::Superseded { version: 7 }));
    assert_eq!(node.seq(), Some(7), "the stale publish overwrote the live site");
}

/// **`RootNotHeld` for a SITE is "not yet", like the head's** (sdk#486, the ONE retryable set, `Why::retryable`): a
/// signer that refuses a site's sign so twice is asked again on the backoff, and the site is then published -- never
/// ended as refused. (Before, the site's set lacked RootNotHeld and the head's had it.) CONTROL: a FINAL refusal
/// (`FromApp`, next test) ends the publication.
#[test]
fn a_site_refused_root_not_held_is_asked_again_and_published() {
    let (mut node, mut now) = (Node::default(), 1_000);
    let mut a = Publisher::new(device());
    a.root_not_held = 2;
    a.page.publish_site(APP, bundle(1), Ms(now));
    run(&mut [&mut a], &mut node, &mut now, 400, &mut always);
    println!("sign asks {}, publication {:?}", a.signs, a.publication());
    assert_eq!(a.root_not_held, 0, "THE SETUP: the refusals were not all given");
    assert_eq!(a.publication(), Some(Publication::Published { version: 1 }), "a site refused RootNotHeld was not asked again to the end");
    assert_eq!(a.signs, 3, "not exactly two refused asks and one signed");
}

/// **A served app's site request is refused and the publication ENDS, named** (the signer's `FromApp`).
#[test]
fn a_refused_sign_ends_the_publication_by_name() {
    let (mut node, mut now) = (Node::default(), 1_000);
    let mut a = Publisher::new(device());
    a.origin = signer::Origin::Served;
    a.page.publish_site(APP, bundle(1), Ms(now));
    run(&mut [&mut a], &mut node, &mut now, 100, &mut always);
    assert!(matches!(a.publication(), Some(Publication::Refused(ref w)) if w.contains("FromApp")), "{:?}", a.publication());
    assert_eq!(node.seq(), None, "a refused publication wrote the site");
}

/// **INVARIANT 4 for sites: every publication ENDS once faults stop** (Published, Superseded or Refused), over a
/// seeded sweep of two publishers with silent ops, lost PUTs and stale reads. A floor on how many seeds had a
/// fault keeps the sweep from passing on the easy path.
#[test]
fn every_publication_ends_once_faults_stop() {
    let mut faulted = 0;
    for seed in 0..120u64 {
        let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let (mut node, mut now) = (Node::default(), 1_000);
        let (mut a, mut b) = (Publisher::new(device()), Publisher::new(device()));
        a.page.publish_site(APP, bundle((seed % 5) as u8), Ms(now));
        b.page.publish_site(APP, bundle((seed % 7) as u8 + 10), Ms(now));
        let mut faults = 0;
        let mut flaky = |_: &Op| {
            if next() % 4 == 0 {
                faults += 1;
                Fate::Silence
            } else {
                Fate::Answer
            }
        };
        run(&mut [&mut a, &mut b], &mut node, &mut now, 30, &mut flaky);
        if faults > 0 {
            faulted += 1;
        }
        run(&mut [&mut a, &mut b], &mut node, &mut now, 2_000, &mut always);
        for (who, p) in [("a", &a), ("b", &b)] {
            assert!(!matches!(p.publication(), Some(Publication::Publishing { .. }) | None), "seed {seed}: {who}'s publication never ended");
        }
        // At most one of them is Published at a version the node does not hold.
        let live = node.read().map(|h| h.seq);
        for p in [&a, &b] {
            if let Some(Publication::Published { version }) = p.publication() {
                assert!(live >= Some(version), "seed {seed}: Published v{version} above the node's {live:?}");
            }
        }
    }
    assert!(faulted >= 100, "only {faulted} of 120 seeds had a fault: the sweep ran the easy path");
}

/// **THE PAGE'S READ IS WHERE A NEW DEVICE'S NEXT VERSION COMES FROM.** Its signer holds no record and its own
/// node does not hold the site YET, so the signer has nothing of its own: asked from the page's read (v3) it
/// answers HeadUnknown, the page backs off and asks again, and once the node holds the site v4 is signed. Mutant
/// "sign from the genesis whatever the read says" -> the blind signer signs v1, the merge keeps v3 ->
/// Superseded { 3 } -> red. (With the signer's local read present, its NotNext masks that mutant: the other
/// cases.)
#[test]
fn a_new_device_follows_the_site_it_reads() {
    let (mut node, mut now) = (Node::default(), 1_000);
    let mut other = Publisher::new(device());
    for n in 1..=3u8 {
        other.page.publish_site(APP, bundle(n), Ms(now));
        run(&mut [&mut other], &mut node, &mut now, 100, &mut always);
    }
    let mut a = Publisher::new(device());
    a.blind_signs = 1;
    a.page.publish_site(APP, bundle(9), Ms(now));
    // While the node has not fetched the site, the publication SAYS so (the signer answered: not "not answering").
    let mut saw_wait = false;
    for _ in 0..200 {
        run(&mut [&mut a], &mut node, &mut now, 1, &mut always);
        saw_wait |= a.publication() == Some(Publication::Publishing { waiting_for: Some(page::WAITING_FOR_SITE) });
        if !matches!(a.publication(), Some(Publication::Publishing { .. })) {
            break;
        }
    }
    assert!(saw_wait, "the HeadUnknown wait was never stated");
    assert_eq!(a.publication(), Some(Publication::Published { version: 4 }));
}

/// **A SITE THAT ALREADY HOLDS THIS EXACT BUNDLE IS PUBLISHED AT ITS VERSION, WRITTEN NOT AGAIN** (the builder's
/// first Publish after a reload: the tab had no record of its site, and the same bytes went out as the next version).
/// A reloaded page (a fresh `Page`, the device's signer kept) publishes the bundle the site holds: `Published` at
/// that version, NO sign, the node's site unmoved. CONTROL: the same reloaded page with ANOTHER bundle signs and
/// writes the next version. Mutant "the current-site cell removed" -> it signs v2 of the same bytes -> red.
#[test]
fn a_site_that_holds_this_exact_bundle_is_published_and_not_written_again() {
    let (mut node, mut now) = (Node::default(), 1_000);
    let host = device();
    let mut first = Publisher::new(host);
    first.page.publish_site(APP, bundle(1), Ms(now));
    run(&mut [&mut first], &mut node, &mut now, 100, &mut always);
    assert_eq!(first.publication(), Some(Publication::Published { version: 1 }), "THE SETUP: the first publish");
    // The reload: a new page, the same device.
    let mut reloaded = Publisher::new(first.host.clone());
    reloaded.page.publish_site(APP, bundle(1), Ms(now));
    run(&mut [&mut reloaded], &mut node, &mut now, 100, &mut always);
    assert_eq!(reloaded.publication(), Some(Publication::Published { version: 1 }), "the current site was not taken as published at its version");
    assert_eq!(reloaded.signs, 0, "the same bundle was signed again");
    assert_eq!(node.seq(), Some(1), "the same bundle was written again as the next version");
    // THE CONTROL: the reloaded page, another bundle -> signed and written as v2.
    reloaded.page.publish_site(APP, bundle(2), Ms(now));
    run(&mut [&mut reloaded], &mut node, &mut now, 100, &mut always);
    assert_eq!(reloaded.publication(), Some(Publication::Published { version: 2 }));
    assert_eq!((reloaded.signs, node.seq()), (1, Some(2)), "another bundle was not signed and written once");
}
