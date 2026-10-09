//! # page-io: the page executor's I/O over Freenet's client API (ruling B, part 2)
//!
//! [`page::server::Server`] answers the page ↔ engine protocol in-process and
//! emits [`page::Op`]s; this crate is the ONLY place they become client-API
//! frames (`wire`, `wire::signer`) and the node's answers become
//! [`page::Answer`]s again (main's condition 3: no other path to the node).
//! Sans-IO like everything under it: frames in, frames out, and the time.
//!
//! | op | frame |
//! |---|---|
//! | `Put { id, bytes }` | PUT of the Block contract for `id` (`wire::block`), state `kind ‖ body` |
//! | `Get { id }` | GET of that contract |
//! | `ReadHead` | GET **with subscribe** of the head Register: on a peered node a delegate-created register is not served to a bare GET (F55), and the subscription is how a head move reaches the page |
//! | `Update { state }` | the first head this page has never seen a register for → PUT of the Register contract (it CREATES it); after that, UPDATE |
//! | `Sign { id, .. }` | `wire::signer::frame_sign` under the request's own id (SG02) |
//! | `AskHeld { batch, ids }` | ONE `wire::signer::frame_held` of the blocks' contracts (1..=MAX_HELD, sdk#455) |
//!
//! | answer | becomes |
//! |---|---|
//! | `Got` of the Register | `Head(record)`; the register now exists |
//! | NotFound of the Register | `Head(None)` ONLY if the signer holds no record for it (asked once, `ask_record`); otherwise silence — re-asked on the RTO with no end, shown as "not answering for N s" (sdk#175; a peered NotFound can be false, F55) |
//! | a REFUSED GET (`ContractError::Get`) of anything | nothing: it says nothing about the contract, so it is not an answer — re-asked on the RTO (rule 7), never read as absent |
//! | `Got` of a block | `Got { id, body }` (the executor verifies it against its id) |
//! | NotFound of a block | `GetMissed` |
//! | `Ack(Put)` of a block | `PutOk` |
//! | `Ack(Put)` / `Ack(Updated)` of the Register | `Updated` (it says nothing more, F56); the register exists |
//! | a signer answer | `Signer { id, answer }`; a `Held { present }` goes back to the blocks its id asked about |
//! | `Ack(Put)` / `PutFailed` of any OTHER contract | handed back unread ([`PageIo::take_others`]): the app's own PUTs ([`PageIo::put_contract`], builder#104) are the caller's to match |
//!
//! THE FIRST-PUT RACE: two pages on one key both see no register and both
//! PUT it. The signer signs ONE record from the genesis (at most one
//! signature per prev), so the second page is answered `AlreadySigned` with
//! the FIRST record and PUTs the same bytes: an equal decision, the held
//! bytes win (F56 only ever displaced DIFFERENT roots). Neither PUT fails;
//! the loser's own commit is judged by the read-back and told `Lost`.

use freenet_prolly::Cid;
use freenet_stdlib::prelude::*;
use page::server::{Server, SignerFacts};
use page::{Answer, Ext, Label, Ms, Op, Publication};
use std::collections::BTreeMap;
use wire::{DelegateKey, Incoming};

mod opening;
mod app_publish;
#[cfg(test)]
mod seeded;
use app_publish::{AppPublishState, PublishEffect, PublishEvent};
pub use app_publish::{AppPublish, SetLine, SetProgress};
use opening::{Cell, First, HeadEffect, HeadEvent, HeadKnownState, HeadSub, HeadSubState, OpenEffect, OpenEvent, Opening, OpeningState, SubEvent, Via};

/// What the page needs to know about the platform: the Block contract's code
/// (a page PUT carries it), the head Register's code and params, and the
/// signer delegate's key.
pub struct Artefacts {
    pub block_code: Vec<u8>,
    pub register_code: Vec<u8>,
    pub register_params: Vec<u8>,
    pub signer: DelegateKey,
}

/// What the node's signer said to [`PageIo::ask`]: whose node this is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// It signs for this Register (instance id): the node holds this key.
    Register([u8; 32]),
    /// The signer is there and holds no key.
    NoKey,
    /// The node has no signer delegate at all (a node that never opened this
    /// person's tree): measured on 0.2.136, it answers the request EMPTY.
    /// In its words.
    NoSigner(String),
    /// The node or the signer refused: in their words.
    Refused(String),
}

/// What every refusal of a view says first (a reader, `PageIo::reader`).
pub const READ_ONLY: &str = "read-only: this is a view of somebody's published data, and writing needs write access";

/// MAY THIS PAGE WRITE a head ([`PageIo::may_write`]): the ONE decision
/// inputs are shown from and writes are refused by (DATA-SOURCE; the
/// architect's point 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MayWrite {
    Yes,
    No(String),
    /// The signer ANSWERED and did not say yes or no for this head (it
    /// refused to say, or refused the provisioning): inputs are shown
    /// DISABLED, with this reason.
    Unknown(String),
    /// NOT DECIDED YET: the signer has not answered (still asking, or silent
    /// -- rule 8, silence is not an answer). A write WAITS for the answer;
    /// inputs are shown disabled meanwhile, with this reason.
    Undecided(String),
}

/// The head subscription as page-io can honestly report it (sdk#259).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadSubscription {
    /// The head was read with `subscribe`.
    pub asked: bool,
    /// The node answered that read: it holds this page's subscription.
    pub answered: bool,
    /// Head moves the subscription has delivered.
    pub changes: usize,
    /// Head reads answered with a failure (no head yet, a refusal, or F55).
    pub failed: usize,
    /// Opening ended, in words: no head read is coming.
    pub ended: Option<String>,
}

/// What a page tells its app about being kept up to date (sdk#259): the
/// MAPPING from the facts above to `LiveMode`, here — beside the facts, and
/// natively testable — rather than inside the web Session, which no native
/// test can build (a mutant that made "subscribed" unreachable there survived
/// every suite).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveMode {
    /// `HeadSubscribed` or `Polled`.
    pub mode: &'static str,
    /// Why not, in words; empty when subscribed.
    pub why: String,
    /// Head moves the subscription has delivered.
    pub head_changes: usize,
}

impl LiveMode {
    /// A connection with no page yet.
    pub fn no_page() -> LiveMode {
        LiveMode { mode: "Polled", why: "there is no page on this connection yet".into(), head_changes: 0 }
    }
}

impl HeadSubscription {
    /// The report, in the order a reader should look: ANSWERED wins over
    /// everything (a failure counted before the head existed does not undo a
    /// subscription the node now holds); then an opening that ENDED; then
    /// head reads the node FAILED; then asked-and-unanswered; then unasked.
    pub fn live_mode(&self) -> LiveMode {
        let tick = " The tick keeps the data right meanwhile.";
        let (mode, why) = if self.answered {
            ("HeadSubscribed", String::new())
        } else if let Some(said) = &self.ended {
            ("Polled", format!("opening ended, so the head is never read: {said}"))
        } else if self.failed > 0 {
            (
                "Polled",
                format!(
                    "the node answered the head read with a failure {} time(s) — no head yet, a refusal, or a peered node's false NotFound (F55); it is asked again.{tick}",
                    self.failed
                ),
            )
        } else if self.asked {
            ("Polled", format!("the head read with subscribe has not been answered yet.{tick}"))
        } else {
            ("Polled", "the head has not been read on this connection yet".to_string())
        };
        LiveMode { mode, why, head_changes: self.changes }
    }
}

/// A site this page reads: its instance id, and WHY it holds it (one list of sites, sdk#493).
struct Site {
    id: [u8; 32],
    role: SiteRole,
}

impl Site {
    /// Its contract key, when this page PUBLISHES it (a PUT's or a change's answer names it); an audited site has none
    /// this page follows.
    fn published_key(&self) -> Option<&str> {
        match &self.role {
            SiteRole::Publishing { key, .. } => Some(key),
            SiteRole::Auditing => None,
        }
    }
}

/// Why a page holds a site. ONE read (rule 4): a site is read by the same GET whether it is published or audited;
/// only a publisher PUTs, signs and follows it.
enum SiteRole {
    /// Being published from this page (builder#117): its contract (code + relabelled params), key and web part.
    Publishing { contract: ContractContainer, key: String, web: Vec<u8> },
    /// Being AUDITED (a keeper's pass, sdk#493): read only, never put, signed or followed.
    Auditing,
}

/// Why a site cannot be published or linked yet: the node's signer has not named the Register (the key) this
/// page signs under, so there is no authority to relabel. Not a "no": it waits on the signer's answer.
pub const NO_REGISTER_YET: &str = "not yet: the node's signer has not said which key this head signs under";

/// A site's contract: `site_code` under `app`'s site params, derived from the person's Register params
/// (`contract_keys::site::site_params`, the ONE relabelling). `None` for a bad app id or a keyset that is not
/// mode 0. Its id is the site's stable link (builder#117).
pub fn site_contract(site_code: &[u8], register_params: &[u8], app: &str) -> Option<ContractContainer> {
    let params = contract_keys::site::site_params(register_params, app)?;
    Some(ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
        std::sync::Arc::new(ContractCode::from(site_code.to_vec())),
        Parameters::from(params),
    ))))
}

/// WHICH SITE A PAGE WAS SERVED FROM (sdk#399 step 4, the architect): the site contract's instance id in the page's
/// own path, `/v1/contract/web/<link>/...` -- the inverse of [`PageIo::site_link`], and the one owner of "which site"
/// a path or an address names (a person running someone else's app runs THAT publisher's site, which no derivation
/// from their own params names). STRICT: the link must decode to the 32-byte id AND encode back to itself
/// (`from_base58` zero-pads a short text into a well-formed wrong id); anything else is `None`, never a guess.
pub fn site_id_of_path(path: &str) -> Option<[u8; 32]> {
    site_path(path).ok()
}

/// A site ADDRESS as a person gives one (sdk#472's keepSet; the loader handover uses [`site_id_of_path`]): a bare link
/// (`<id>`), a node URL (`http(s)://<host:port>/v1/contract/web/<id>/...`, its scheme and authority stripped), or the
/// path itself -- with the same strict round-trip. Anything else is REFUSED BY NAME ([`AddressRefused`]); a caller
/// shows or maps the name, never guesses.
pub fn site_id_of_address(text: &str) -> Result<[u8; 32], AddressRefused> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AddressRefused::Empty);
    }
    if let Some(rest) = text.strip_prefix("http://").or_else(|| text.strip_prefix("https://")) {
        let path = rest.find('/').map(|at| &rest[at..]).ok_or(AddressRefused::NotASitePath)?;
        return site_path(path);
    }
    if text.contains("://") {
        // `craftec://` among them: nothing defines one, so nothing reads one.
        return Err(AddressRefused::UnknownScheme);
    }
    if text.starts_with('/') {
        return site_path(text);
    }
    if text.contains(['/', '?', '#']) {
        return Err(AddressRefused::NotASitePath);
    }
    site_id_of_link(text).ok_or(AddressRefused::NotASiteLink)
}

/// THE path form, parsed in ONE place (both [`site_id_of_path`] and [`site_id_of_address`] come through here).
fn site_path(path: &str) -> Result<[u8; 32], AddressRefused> {
    let link = path.strip_prefix("/v1/contract/web/").ok_or(AddressRefused::NotASitePath)?.split(['/', '?', '#']).next().unwrap_or("");
    site_id_of_link(link).ok_or(AddressRefused::NotASiteLink)
}

/// THE LOADER'S POINTER CHECK (ARCHITECTURE §19, the bootstrap): a site's `pointer.json`, read by its one owner
/// (`wire::webapp::read_site_pointer`), its site link RECOMPUTED -- `site_contract(site_code, register_params, app)`,
/// the one relabelling -- and REFUSED unless that is the link the page was served from (`site_id_of_path`): a
/// swapped pointer names another link. Returns the app id and the owner's Register id (`contract_keys::instance` over the code hash of
/// `register_code`), whose tree the app is in. Pure: the loader runs it before any node is asked.
pub fn open_pointer(pointer: &[u8], page_path: &str, site_code: &[u8], register_code: &[u8]) -> Result<(String, [u8; 32]), PointerRefused> {
    let (params, app) = wire::webapp::read_site_pointer(pointer).ok_or(PointerRefused::NotAPointer)?;
    let page = site_id_of_path(page_path).ok_or(PointerRefused::NotASitePath)?;
    let site = site_contract(site_code, &params, &app).ok_or(PointerRefused::NotAPointer)?;
    let mut names = [0u8; 32];
    names.copy_from_slice(&site.key().id().as_bytes()[..32]);
    if names != page {
        return Err(PointerRefused::NotThisSite { names, page });
    }
    Ok((app, contract_keys::instance(&contract_keys::code_hash(register_code), &params)))
}

/// Why a page's pointer is refused, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerRefused {
    /// Not the bytes a site's pointer is (`wire::webapp::site_pointer`), or it names no site.
    NotAPointer,
    /// The page's own path is not a site's (`/v1/contract/web/<link>/`).
    NotASitePath,
    /// The pointer names ANOTHER site's link than the page was served from: never read.
    NotThisSite { names: [u8; 32], page: [u8; 32] },
}

impl std::fmt::Display for PointerRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PointerRefused::NotAPointer => write!(f, "this site's pointer.json is not a site pointer"),
            PointerRefused::NotASitePath => write!(f, "this page is not served from a site's path (/v1/contract/web/<link>/)"),
            PointerRefused::NotThisSite { .. } => write!(f, "this site's pointer names another site: it is not this page's own, and is not read"),
        }
    }
}

/// Why a text is not a site address, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressRefused {
    /// Nothing given.
    Empty,
    /// A scheme other than http(s) -- `craftec://` included: nothing defines it.
    UnknownScheme,
    /// A URL or path that is not `/v1/contract/web/<link>/...`, or a bare text with a path in it.
    NotASitePath,
    /// The link is not exactly a site's 32-byte id: not base58, or a short text that decodes (the round-trip trap).
    NotASiteLink,
}

/// A site id as TEXT (base58, as the node serves it at `/v1/contract/web/<text>/`): the inverse of
/// [`site_id_of_address`], so the text form of a site id has one owner in both directions (sdk#472 names targets by it).
pub fn site_text(id: &[u8; 32]) -> String {
    freenet_stdlib::prelude::ContractInstanceId::new(*id).encode()
}

/// A bare link: exactly the 32-byte id that encodes back to it.
fn site_id_of_link(link: &str) -> Option<[u8; 32]> {
    if link.is_empty() {
        return None;
    }
    let id = freenet_stdlib::prelude::ContractInstanceId::from_base58(link).ok()?;
    (site_text(&id) == link).then(|| *id)
}

/// A site this page could READ BACK (the TooLargeToSend of a site, src/writes.rs's kind): its web within
/// `wire::MAX_SITE_WEB`, so the state the node answers -- the web behind the site's signed record -- is within the
/// one bound a page reads. A larger one is refused by name, before anything is sent: its publication could never
/// be read back, and would wait for ever.
fn site_fits(app: &str, web: &[u8]) -> Result<(), String> {
    if web.len() > wire::MAX_SITE_WEB {
        return Err(format!(
            "{app}'s site is {} bytes, over the {} a site's read-back can carry (wire::MAX_SITE_WEB): it could never be read back",
            web.len(),
            wire::MAX_SITE_WEB
        ));
    }
    Ok(())
}

pub struct PageIo {
    pub server: Server,
    art: Artefacts,
    register: ContractContainer,
    register_id: [u8; 32],
    register_key: String,
    /// DOES THE HEAD EXIST (OPENING.md, Machine 2): read, "no head" said, the signer's record, or named. Changed
    /// only by [`PageIo::head`].
    head_known: HeadKnownState,
    /// THE HEAD SUBSCRIPTION, as page-io can honestly report it (sdk#259):
    /// whether the GET-with-subscribe has been SENT, whether the node has
    /// ANSWERED it, and how many head moves it has delivered. A page that
    /// believed it was being notified while it was polling is the failure
    /// `LiveMode` exists to make impossible, and on the page path the
    /// subscription is page-io's, not the Session's.
    /// Machine 3 (sdk#490): asked, and answered or proven. Its one writer is its own `step` (by type).
    head_sub: HeadSubState,
    head_changes: usize,
    /// Head reads the node answered with a FAILURE: no head yet, a refusal,
    /// or a peered node's false NotFound (F55) — page-io cannot tell which,
    /// and says so rather than guessing (sdk#259).
    head_failed: usize,
    /// Blocks in flight, by contract id and by key string (a PUT's answer
    /// names the key).
    by_contract: BTreeMap<[u8; 32], Cid>,
    by_key: BTreeMap<String, Cid>,
    /// `Held` asks in flight, by signer request id: the page's batch it answers (the ids are the batch's op's, sdk#455).
    held: BTreeMap<u32, u32>,
    next_held_id: u32,
    /// A READER's own node's signer, for `Held` ONLY (sdk#493: a keeper auditing someone else's app). By TYPE it frames
    /// nothing else, so a reader still cannot Sign, Update or provision. `None`: a plain reader, whose `Held` asks are
    /// answered "not held" here and fetched.
    held_signer: Option<wire::signer::HeldSigner>,
    frames: wire::Reassembler,
    stream: u32,
    out: Vec<Vec<u8>>,
    replies: Vec<Vec<u8>>,
    /// The lines said to the app (sdk#482): capped, drained by [`PageIo::take_unusable`], each recorded where said.
    unusable: page::unusable::Unusable,
    /// Node errors that named no op (sdk#433), counted by reason code ([`wire::Refused::code`]).
    node_errors: BTreeMap<&'static str, u64>,
    /// THE PAGE'S STANDING WITH ITS NODE'S SIGNER (OPENING.md, Machine 1): new, asking, an owner registering /
    /// querying / needing a key / provisioning / open / refused, or a reader. Changed only by [`PageIo::open`].
    opening: OpeningState,
    /// Events that landed in an IMPOSSIBLE cell of either machine (an answer to a request that state never sent):
    /// a diagnostic, never a panic (#450); the model test holds it at 0.
    impossible_cells: u64,
    /// "No head" was said and then the signer held a record (another tab or device signed first; H1's premise moved).
    no_head_contradicted: u64,
    /// Node answers about contracts that are not this page's register or
    /// blocks — the app's own PUTs — handed back unread (`take_others`).
    others: Vec<Incoming>,
    /// The app's PUTs by contract key: the exact contract and state, framed
    /// again whenever the page re-sends [`Op::PutApp`].
    app_contracts: BTreeMap<String, (ContractContainer, WrappedState)>,
    /// SITES being published (builder#117), by app: the site contract and the web part the page's record is
    /// framed around. The page owns the publication; these bytes are page-io's only while it is `Publishing`
    /// and are dropped when it ends (no app PUT entry: the site's PUT is the page's `Update`).
    sites: BTreeMap<String, Site>,
    /// APP PUBLISHES (sdk#516), by app: each one's state (one writer by type), the apps whose sets name a piece's key
    /// (the SDK's pieces are shared by every app of a build), and each app's site to send once its sets are at k.
    publishes: BTreeMap<String, AppPublishState>,
    publish_of_key: BTreeMap<String, std::collections::BTreeSet<String>>,
    publish_sites: BTreeMap<String, (Vec<u8>, Vec<u8>)>,
    /// A reader's stream-id range (`reader`), in the top byte; 0 for the
    /// person's own page, which keeps the whole space below it.
    stream_base: u32,
    /// The signer's container, framed again whenever the page re-sends its
    /// registration.
    signer_container: Option<DelegateContainer>,
}

/// A PUT of ours a node refusal names (sdk#516): the one finality rule, [`PageIo::put_refused`], decides it.
enum Refused {
    Block(Cid),
    /// An app contract's PUT (`put_contract`: a load piece, a container), by its key.
    App(String),
    /// A site's PUT (its publication, builder#117), by its app.
    Site(String),
}

/// The counter part of a reader's stream id; the top byte is its range.
const STREAM_COUNTER: u32 = 0x00FF_FFFF;

/// The id the record query goes out under.
const RECORD_QUERY_ID: u32 = (1 << 31) - 2;

/// The id the "which Register do you sign for?" query goes out under (`begin`).
const REGISTER_QUERY_ID: u32 = (1 << 31) - 3;

/// A root no node holds: the record query names it so that nothing can be
/// signed (see `ask_record`).
const UNHELD_ROOT: [u8; 32] = [0xA5; 32];

/// The id a provisioning request goes out under: far from the executor's own
/// sign ids (from 1) and from the `Held` ids (from 2³¹).
const PROVISION_ID: u32 = (1 << 31) - 1;

/// The page's I/O as the store's host (READ-STATE): a call on the server is
/// carried out at once — its node ops framed, its replies kept for the client.
impl page::server::Host for PageIo {
    fn with_server<R>(&mut self, f: impl FnOnce(&mut Server) -> R) -> R {
        let r = f(&mut self.server);
        self.pump();
        r
    }
    fn peek<R>(&self, f: impl FnOnce(&Server) -> R) -> R {
        f(&self.server)
    }
    fn client(&mut self, frame: &[u8]) {
        PageIo::client(self, frame);
    }
    fn take_replies(&mut self) -> Vec<Vec<u8>> {
        PageIo::take_replies(self)
    }
}

impl PageIo {
    pub fn new(server: Server, art: Artefacts) -> PageIo {
        PageIo::build(server, art, OpeningState::owner(), HeadKnownState::unknown())
    }

    /// The ONE constructor: each machine starts in the state its kind of page starts in (an owner's page `New` /
    /// `Unknown`, a reader `Reader` / `Named`), and from then on only its transition function changes it.
    fn build(server: Server, art: Artefacts, opening: OpeningState, head_known: HeadKnownState) -> PageIo {
        let register = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
            std::sync::Arc::new(ContractCode::from(art.register_code.clone())),
            Parameters::from(art.register_params.clone()),
        )));
        let mut register_id = [0u8; 32];
        register_id.copy_from_slice(&register.key().id().as_bytes()[..32]);
        let register_key = register.key().to_string();
        PageIo {
            server,
            art,
            register,
            register_id,
            register_key,
            head_known,
            head_sub: HeadSubState::unasked(),
            head_changes: 0,
            head_failed: 0,
            by_contract: BTreeMap::new(),
            by_key: BTreeMap::new(),
            held: BTreeMap::new(),
            // High, so it never meets the executor's own sign ids (from 1).
            next_held_id: 1 << 31,
            held_signer: None,
            frames: wire::Reassembler::default(),
            stream: 0,
            out: Vec::new(),
            replies: Vec::new(),
            unusable: Default::default(),
            node_errors: BTreeMap::new(),
            opening,
            impossible_cells: 0,
            no_head_contradicted: 0,
            others: Vec::new(),
            app_contracts: BTreeMap::new(),
            sites: BTreeMap::new(),
            publishes: BTreeMap::new(),
            publish_of_key: BTreeMap::new(),
            publish_sites: BTreeMap::new(),
            stream_base: 0,
            signer_container: None,
        }
    }

    /// A READER of somebody's published head (sdk#239): the page reads the
    /// Register `register_id` names and the blocks under it, and can do
    /// nothing else. Published data is readable by default; writing is access
    /// control, which a reader does not have.
    ///
    /// * NO SIGNER, and nothing provisioned or registered on the node: a
    ///   reader leaves no trace on the node it reads from beyond the blocks
    ///   it caches and its subscription to the head.
    /// * The head is read by GET with subscribe (a peered node serves a bare
    ///   GET of a delegate-created register falsely, F55; and the subscription
    ///   carries the publisher's next head here).
    /// * A failed head read is SILENCE, re-asked on the RTO — never "no head":
    ///   the head was named because it was published.
    /// * `Put`, `Update` and `Sign` are never framed. `AskHeld` (is a block
    ///   local to the signer?) is answered here: not held, so it is fetched.
    ///
    /// `range` (1..=255) is this reader's stream-id range on the socket it
    /// shares: one per tree, so their chunked frames can never be joined.
    pub fn reader(server: Server, block_code: Vec<u8>, register_id: [u8; 32], range: u8) -> PageIo {
        let (_, no_signer) = wire::delegate_from_code(&[]);
        // A reader: provisioned by construction (nothing of its own to open), and its head was NAMED because it was
        // published, so a failed read of it is silence and the signer is never asked (there is none).
        let mut io = PageIo::build(
            server,
            Artefacts { block_code, register_code: Vec::new(), register_params: Vec::new(), signer: no_signer },
            OpeningState::reader(),
            HeadKnownState::named(),
        );
        io.register_id = register_id;
        io.register_key = wire::contract_id(register_id).to_string();
        io.server.page.set_read_only();
        io.stream_base = u32::from(range.max(1)) << 24;
        io.next_held_id = (1u32 << 31) | (io.stream_base >> 1);
        io.server.set_facts(SignerFacts { head_writable: false, head_id: register_id });
        io
    }

    /// A READER that may ask its OWN node's signer `Held` (sdk#493): a keeper auditing SOMEONE ELSE's app measures
    /// presence in the node it runs on, and that is a read, not a signature. By type it can ask nothing else
    /// ([`wire::signer::HeldSigner`]): it is still a reader, and cannot Sign, Update or provision.
    pub fn reader_with_held(server: Server, block_code: Vec<u8>, register_id: [u8; 32], range: u8, held: wire::signer::HeldSigner) -> PageIo {
        let mut io = PageIo::reader(server, block_code, register_id, range);
        io.held_signer = Some(held);
        io
    }

    /// The next `Held` request id, in THIS page's own space: `2^31 | range << 23 | counter`. The pages on one socket
    /// (the person's own, range 0, and each reader's range) never share an id, so an answer reaches only the page that
    /// asked (sdk#493: a reader with a HeldSigner asks the same node's signer as the person's own page).
    fn take_held_id(&mut self) -> u32 {
        let base = (1u32 << 31) | (self.stream_base >> 1);
        let hid = self.next_held_id;
        self.next_held_id = base | (hid.wrapping_add(1) & 0x007F_FFFF);
        hid
    }

    /// WHOSE NODE IS THIS? Ask the node's EXISTING signer which Register it
    /// signs for — the same query [`PageIo::begin`] sends, on the same first-
    /// request clock — WITHOUT registering it first. On the node that holds
    /// a person's key the answer names their Register; anywhere else the node
    /// has no such delegate (refused), or it holds no key: either way nothing
    /// is installed, minted or provisioned, and a reader leaves no trace.
    /// The answer: [`PageIo::asked`].
    pub fn ask(&mut self) {
        self.open(OpenEvent::Ask);
    }

    /// [`PageIo::ask`]'s answer, once there is one -- and after a claim, still the answer it was.
    pub fn asked(&self) -> Option<&Asked> {
        match self.opening.get() {
            Opening::Asking { answer } => answer.as_ref(),
            Opening::New | Opening::Reader => None,
            owner @ (Opening::Registering { .. } | Opening::Querying { .. } | Opening::NeedsKey { .. } | Opening::Provisioning { .. } | Opening::Open { .. } | Opening::Refused { .. }) => {
                match owner.via() {
                    Some(Via::Claimed(a)) => Some(a),
                    Some(Via::Begun) | None => None,
                }
            }
        }
    }

    /// MAY THIS PAGE WRITE `head` (`None`: its OWN tree, a `mine`
    /// component's)? Derived each time from what this page holds -- the
    /// signer's answer, the opening -- and kept nowhere else: ONE match over
    /// [`Opening`], with no reading order.
    ///
    /// A display gate, not the enforcement: the signer refuses to sign for
    /// anyone else whatever this says, so a wrong answer can only show or hide
    /// inputs. "Yes" means THIS NODE'S SIGNER SIGNS FOR that head; a keyset
    /// (a device key among an identity's keys) is Phase 6.
    pub fn may_write(&self, head: Option<[u8; 32]>) -> MayWrite {
        let other = |r: &[u8; 32]| MayWrite::No(format!("this node signs for another head ({})", hex(r)));
        match self.opening.get() {
            Opening::Reader => MayWrite::No(READ_ONLY.into()),
            Opening::Asking { answer } => match (answer, head) {
                (None, _) => MayWrite::Undecided("asking this node's signer whose node it is".into()),
                (Some(Asked::Register(_)), None) => MayWrite::Yes,
                (Some(Asked::Register(r)), Some(h)) if h == *r => MayWrite::Yes,
                (Some(Asked::Register(r)), Some(_)) => other(r),
                // Nothing of this person's here yet: their own tree is made on
                // first use (a key minted, the head created by the first
                // write); anyone else's head is not theirs to write.
                (Some(Asked::NoKey | Asked::NoSigner(_)), None) => MayWrite::Yes,
                (Some(Asked::NoKey | Asked::NoSigner(_)), Some(_)) => MayWrite::No("this node holds no key for that head".into()),
                (Some(Asked::Refused(w)), None) => MayWrite::Unknown(w.clone()),
                (Some(Asked::Refused(w)), Some(_)) => MayWrite::No(w.clone()),
            },
            Opening::Refused { why, .. } => {
                if head.is_none() {
                    MayWrite::Unknown(why.clone())
                } else {
                    MayWrite::No(why.clone())
                }
            }
            Opening::Open { .. } => match head {
                None => MayWrite::Yes,
                Some(h) if h == self.register_id => MayWrite::Yes,
                Some(_) => other(&self.register_id),
            },
            // Still opening its own tree: a write waits in the engine for the
            // head, as it always has; another head is not known to be ours yet.
            Opening::New | Opening::Registering { .. } | Opening::Querying { .. } | Opening::NeedsKey { .. } | Opening::Provisioning { .. } => match head {
                None => MayWrite::Yes,
                Some(_) => MayWrite::Undecided("opening: this node's signer has not said whose node it is yet".into()),
            },
        }
    }

    /// Was this page only asked (`ask`), and not (yet) claimed?
    pub fn asking(&self) -> bool {
        matches!(self.opening.get(), Opening::Asking { .. })
    }

    /// THE USER HAS NO TREE HERE YET (DATA-SOURCE `mine`: made on the first
    /// WRITE): an asked page whose node's signer holds no key, or has no
    /// signer, and which has not been provisioned -- asked still, or claimed
    /// and not yet open. Its tree reads as EMPTY -- its head read is answered
    /// "missing" here, and nothing goes to the node -- until a first write
    /// provisions it ([`PageIo::claim`], then the existing provision path).
    pub fn no_tree_yet(&self) -> bool {
        let none_here = |a: &Asked| matches!(a, Asked::NoKey | Asked::NoSigner(_));
        match self.opening.get() {
            Opening::Asking { answer } => answer.as_ref().is_some_and(none_here),
            Opening::Open { .. } | Opening::Reader | Opening::New => false,
            Opening::Registering { via, .. } | Opening::Querying { via } | Opening::NeedsKey { via } | Opening::Provisioning { via, .. } | Opening::Refused { via, .. } => {
                matches!(via, Via::Claimed(a) if none_here(a))
            }
        }
    }

    /// OPEN THE PERSON'S OWN TREE ON AN ASKED PAGE (DATA-SOURCE `mine`):
    /// the same opening as [`PageIo::begin`], from where the answer left it.
    /// - It signs for a Register: that is this person's tree on this node,
    ///   opened as it is. Nothing is registered or minted.
    /// - It holds no key: [`PageIo::needs_key`], and the caller mints one
    ///   (`provision_with`), as `begin`'s.
    /// - There is no signer here: it is registered and asked, as `begin`'s.
    ///
    /// Refused, or not answered yet: nothing is claimed, and
    /// `false` says so. The head itself is created on the first write, by
    /// the first commit's PUT (as `begin`'s). A page claimed already answers `true`.
    pub fn claim(&mut self, signer: DelegateContainer) -> bool {
        self.open(OpenEvent::Claim(signer))
    }

    /// OPEN THE PERSON'S OWN TREE (a switch-over blocker): register the signer
    /// and ASK it which Register it signs for, before anything is minted. A
    /// page that minted a key on every load would be a new identity after
    /// every reload and in every second tab. The answer either names the
    /// Register — this page opens it, and nothing is provisioned — or says the
    /// signer holds no key: [`PageIo::needs_key`], and the caller mints one
    /// and calls [`PageIo::provision_with`]. The key never leaves the signer.
    pub fn begin(&mut self, signer: DelegateContainer) {
        self.open(OpenEvent::Begin(signer));
    }

    /// The signer holds no key (`begin`'s answer): mint one and `provision_with` it.
    pub fn needs_key(&self) -> bool {
        matches!(self.opening.get(), Opening::NeedsKey { .. })
    }

    /// Provision a signer that holds no key, for the Register `register_params`
    /// names (the minted key's). Only after `begin` said it needs one.
    pub fn provision_with(&mut self, signing_key: Vec<u8>, register_params: Vec<u8>) {
        self.open(OpenEvent::ProvisionWith(signing_key, register_params));
    }

    /// THE ONE WRITER OF [`Opening`] (a source test holds it): the table's step, applied -- the state set, then its
    /// effects carried out in order. Returns `claim`'s answer.
    fn open(&mut self, ev: OpenEvent) -> bool {
        let now = self.server.page.now();
        let step = self.opening.step(ev);
        if step.cell == Cell::Impossible {
            self.impossible_cells += 1;
        }
        for effect in step.effects {
            match effect {
                OpenEffect::Register(c) => {
                    self.signer_container = Some(c);
                    self.server.page.send_ext(Ext::RegisterSigner, now);
                }
                OpenEffect::RegistrationAnswered => {
                    self.server.page.ext_answered(Ext::RegisterSigner, now);
                    self.server.page.send_ext(Ext::SignerFirst, now);
                }
                OpenEffect::SendFirst => self.server.page.send_ext(Ext::SignerFirst, now),
                OpenEffect::FirstAnswered => self.server.page.ext_answered(Ext::SignerFirst, now),
                OpenEffect::SetRegister(params) => self.set_register(params),
                OpenEffect::SignerProvisioned => self.signer_provisioned(),
                OpenEffect::StepCanSign => self.step_can_sign(),
                OpenEffect::Unusable(why) => self.say(page::unusable::Site::of("page-io::said::opening"), why),
            }
        }
        self.pump();
        step.claimed
    }

    /// THE ONE WRITER OF [`HeadKnown`] (a source test holds it).
    fn head(&mut self, ev: HeadEvent, now: Ms) {
        let (effects, cell) = self.head_known.step(ev);
        if cell == Cell::Impossible {
            self.impossible_cells += 1;
        }
        for effect in effects {
            match effect {
                HeadEffect::AskRecord => self.ask_record(now),
                HeadEffect::NoHead => self.server.node(Answer::Head { label: Label::Head, read: None }, now),
                HeadEffect::Contradicted => self.no_head_contradicted += 1,
            }
        }
    }

    /// Machine 3's step, applied, its impossible cells counted (sdk#490).
    fn sub(&mut self, ev: SubEvent) {
        if self.head_sub.step(ev) == Cell::Impossible {
            self.impossible_cells += 1;
        }
    }

    /// Events that landed in an impossible cell of the opening tables (a diagnostic; 0 on any real node's answers).
    pub fn impossible_cells(&self) -> u64 {
        self.impossible_cells
    }

    /// Times "no head" was said and the signer then held a record (another tab or device signed first).
    pub fn no_head_contradicted(&self) -> u64 {
        self.no_head_contradicted
    }

    /// The instance id of the Register `params` name (under this page's Register code): what an asked page's answer
    /// records, derived by the same construction `set_register` makes.
    fn register_id_of(&self, params: &[u8]) -> [u8; 32] {
        let register = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
            std::sync::Arc::new(ContractCode::from(self.art.register_code.clone())),
            Parameters::from(params.to_vec()),
        )));
        let mut id = [0u8; 32];
        id.copy_from_slice(&register.key().id().as_bytes()[..32]);
        id
    }

    /// Name the Register this page's head lives in.
    fn set_register(&mut self, params: Vec<u8>) {
        let register = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
            std::sync::Arc::new(ContractCode::from(self.art.register_code.clone())),
            Parameters::from(params.clone()),
        )));
        self.register_id.copy_from_slice(&register.key().id().as_bytes()[..32]);
        self.register_key = register.key().to_string();
        self.register = register;
        self.art.register_params = params;
    }

    /// THE SEQ THIS PAGE'S HEAD IS PUBLISHED AT, as the network acknowledged
    /// it: the page's published seq, which moves only when the register's
    /// read-back shows this page's own head (`HeadConfirmed`) or a head is
    /// read from the network. A commit signed or in flight, or one that is
    /// later Lost, never moves it -- so a publisher that records it (an app's
    /// published-head floor, sdk#349) names a head that exists. 0: none.
    pub fn published_seq(&self) -> u64 {
        self.server.page.published().0
    }

    /// A reader of a named head (`reader`): nothing can be written.
    pub fn read_only(&self) -> bool {
        // THE PAGE's: the one owner of "read-only" (it makes no commit op).
        self.server.page.read_only()
    }

    /// PUT a contract the APP names (builder#104: a web container). Framed
    /// here, on this page's stream counter, because this is the only path to
    /// the node (main's condition 3) and two chunked requests on one stream id
    /// would be reassembled into each other. The answer comes back through
    /// [`PageIo::take_others`], named by the contract's key.
    pub fn put_contract(&mut self, contract: ContractContainer, state: WrappedState, now: Ms) -> Result<(), String> {
        if self.read_only() {
            return Err("read-only: a reader PUTs nothing".into());
        }
        let key = contract.key().to_string();
        self.app_contracts.insert(key.clone(), (contract, state));
        // THE PAGE SENDS IT (its deadline, re-send and end), like every op.
        self.server.page.put_app(key, now);
        self.pump();
        Ok(())
    }

    /// PUBLISH `web` as `app`'s site (builder#117): the page reads the site, signs the next version through the
    /// ONE head path, PUTs the record framed around `web`, and reads it back ([`PageIo::publication`]). A reader,
    /// a bad app id or a Register that is not this person's own key publishes nothing, by name.
    pub fn publish_site(&mut self, app: &str, site_code: &[u8], web: Vec<u8>, now: Ms) -> Result<(), String> {
        // THE DIRECT DOOR (Codex on #527, B): while an app publish of `app` is in flight its site is the machine's; a
        // publish here would replace the publication it waits on. The machine sends its own through `send_site`.
        if self.publishes.get(app).is_some_and(|p| p.get().in_flight()) {
            return Err(format!("{app} is being published (its pieces and site): its site is that publish's"));
        }
        self.send_site(app, site_code, web, now)
    }

    /// The site's publication itself: the direct door's, and an app publish's SendSite (P1 holds before it).
    fn send_site(&mut self, app: &str, site_code: &[u8], web: Vec<u8>, now: Ms) -> Result<(), String> {
        if self.read_only() {
            return Err("read-only: a reader publishes nothing".into());
        }
        site_fits(app, &web)?;
        if self.art.register_params.is_empty() {
            return Err(NO_REGISTER_YET.into());
        }
        let Some(contract) = site_contract(site_code, &self.art.register_params, app) else {
            return Err(format!("no site for {app:?}: not an app id, or this head has no single key to sign it"));
        };
        let mut id = [0u8; 32];
        id.copy_from_slice(&contract.key().id().as_bytes()[..32]);
        let key = contract.key().to_string();
        let value = *blake3::hash(&web).as_bytes();
        self.sites.insert(app.to_string(), Site { id, role: SiteRole::Publishing { contract, key, web } });
        self.server.page.publish_site(app, value, now);
        self.pump();
        Ok(())
    }

    /// CREATE (OR UPGRADE) AN APP'S SITE -- the ONE thing a site is written for (ARCHITECTURE §19, the bootstrap;
    /// app-as-data P4): freenet serves a page only from a web container, so the site holds a fixed STARTER (the build's
    /// loader, decoder and starter modules, naming its ONE piece set) plus the POINTER (`wire::webapp::site_pointer`,
    /// composed HERE from this session's register params and `app`: it names whose tree the app is in). The app
    /// itself is data in that tree (its definition doors), never in a site; so a site is written at the app's first
    /// Publish and on a platform upgrade (the builder's decision), never per app version.
    ///
    /// The build's piece `set` is PUT (its containers in the set's order), and the site sent only once the set has
    /// `k` pieces acked (P1). PUBLISHED is the site read back; BACKED_UP every piece acked; a set that cannot reach
    /// `k` ends it REFUSED before any site. [`PageIo::app_publish_status`] says where it stands. Refused by name: a
    /// reader, a bad set, a piece not at the address its set names, a starter carrying its own pointer or an
    /// `app.json` (the app is in the tree), or a publish of `app` already in flight.
    pub fn publish_app(&mut self, app: &str, set: (pieces::PieceSet, Vec<(ContractContainer, WrappedState)>), starter: &[(String, Vec<u8>)], site_code: &[u8], now: Ms) -> Result<(), String> {
        if self.read_only() {
            return Err("read-only: a reader publishes nothing".into());
        }
        if site_contract(site_code, &self.art.register_params, app).is_none() {
            return Err(if self.art.register_params.is_empty() { NO_REGISTER_YET.into() } else { format!("no site for {app:?}: not an app id, or this head has no single key to sign it") });
        }
        if self.publishes.get(app).is_some_and(|p| p.get().in_flight()) {
            return Err(format!("{app} is being published already"));
        }
        // Nothing to PUT means no ack could ever send the site (Codex on #527, A): refused, never a publish stuck at
        // `pieces` that blocks every later one.
        if set.0.pieces.is_empty() {
            return Err(format!("{app}'s publish names no piece: nothing could ever send its site"));
        }
        // THE SITE: the starter and the pointer, and nothing per version.
        if let Some((path, _)) = starter.iter().find(|(p, _)| p == wire::webapp::POINTER_PATH || p == wire::webapp::LEGACY_APP_JSON) {
            return Err(format!("the starter carries its own `{path}`: a site holds the starter and the pointer page-io composes, and the app is in the tree"));
        }
        let pointer = wire::webapp::site_pointer(&self.art.register_params, app).ok_or_else(|| format!("no pointer for {app:?}: not an app id, or no register yet"))?;
        let mut files: Vec<(&str, &[u8])> = starter.iter().map(|(p, b)| (p.as_str(), b.as_slice())).collect();
        files.push((wire::webapp::POINTER_PATH, &pointer));
        let web = wire::webapp::app_web(&files)?;
        // The site AS COMPOSED (starter + pointer) must be one this page can read back: refused BEFORE any piece is PUT.
        site_fits(app, &web)?;
        let sets = vec![set];
        let mut progress = Vec::new();
        let mut puts = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (set, containers) in sets {
            set.check()?;
            if containers.len() != set.pieces.len() {
                return Err(format!("piece set {}: {} containers for {} pieces", set.name, containers.len(), set.pieces.len()));
            }
            let mut keys = Vec::new();
            for (i, ((c, st), named)) in containers.into_iter().zip(&set.pieces).enumerate() {
                let at = c.key().id().encode();
                if at != named.address {
                    return Err(format!("piece {i} of {} is at {at}, not the {} its set names", set.name, named.address));
                }
                // ONE place per address in a publish: an answer is marked in one set, so a second would never be.
                if !seen.insert(at.clone()) {
                    return Err(format!("piece {i} of {} is at {at}, which another piece of this publish names too", set.name));
                }
                keys.push(c.key().to_string());
                puts.push((c, st));
            }
            progress.push(SetProgress::new(set, keys));
        }
        // A publish of `app` that ended, or is PUBLISHED and still owed pieces, is REPLACED (Codex on #527, C): its owed
        // pieces that neither this publish nor any other owes are withdrawn (R1's one rule), then its routing goes.
        let new_keys: std::collections::BTreeSet<String> = progress.iter().flat_map(|p| p.keys.iter().cloned()).collect();
        let left: Vec<String> = self.publishes.get(app).map(|p| p.get().owed_keys()).unwrap_or_default();
        for k in left {
            if !new_keys.contains(&k) && !self.owed_elsewhere(&k, app) {
                self.server.page.cancel_app_put(&k);
            }
        }
        self.unroute(app);
        for (c, st) in puts {
            let key = c.key().to_string();
            self.publish_of_key.entry(key).or_default().insert(app.to_string());
            self.put_contract(c, st, now)?;
        }
        self.publish_sites.insert(app.to_string(), (site_code.to_vec(), web));
        self.publishes.insert(app.to_string(), AppPublishState::start(progress));
        self.pump();
        Ok(())
    }

    /// Where `app`'s publish stands: the machine's state, to READ (the Session maps it to its status word, its
    /// version, each set's line and its words). `None`: never published here.
    pub fn app_publish(&self, app: &str) -> Option<&AppPublish> {
        self.publishes.get(app).map(AppPublishState::get)
    }

    /// Does a publish OTHER than `app`'s still owe the piece `key`? (R1: a shared piece is withdrawn only when none does.)
    fn owed_elsewhere(&self, key: &str, app: &str) -> bool {
        self.publish_of_key.get(key).is_some_and(|apps| apps.iter().any(|a| a != app && self.publishes.get(a).is_some_and(|p| p.get().owes(key))))
    }

    /// `app`'s piece routing and site bytes go (an end that owes nothing, or a publish replaced).
    fn unroute(&mut self, app: &str) {
        self.publish_of_key.retain(|_, apps| {
            apps.remove(app);
            !apps.is_empty()
        });
        self.publish_sites.remove(app);
    }

    /// A PERSON cancels `app`'s publish: before PUBLISHED it ends `cancelled` (the site and the pieces withdrawn);
    /// after, the site stays live and the backup is abandoned.
    pub fn cancel_app_publish(&mut self, app: &str) {
        self.publish_step(app, PublishEvent::Cancel);
        self.pump();
    }

    /// A piece's PUT was answered (acked, or FINALLY refused): every publish whose sets name it takes the event.
    fn piece_answered(&mut self, key: &str, acked: bool) {
        let apps: Vec<String> = self.publish_of_key.get(key).map(|a| a.iter().cloned().collect()).unwrap_or_default();
        for app in apps {
            self.publish_step(&app, if acked { PublishEvent::PieceAcked(key.to_string()) } else { PublishEvent::PieceRefused(key.to_string()) });
        }
    }

    /// The site's answers, for a publish whose site is in flight: the page's `Publication` says how it ended.
    fn observe_publishes(&mut self) {
        let siting: Vec<String> = self.publishes.iter().filter(|(_, p)| p.get().siting()).map(|(a, _)| a.clone()).collect();
        for app in siting {
            let ev = match self.server.page.publication(&app) {
                Some(Publication::Published { version }) => Some(PublishEvent::SitePublished(version)),
                Some(Publication::Superseded { version }) => Some(PublishEvent::SiteSuperseded(version)),
                Some(Publication::Refused(w)) => Some(PublishEvent::SiteRefused(w.clone())),
                // A cancel of the SITE (the Session's cancel_site) ends the publish too (the architect's R2): the
                // table's own Siting x Cancel cell, never a Siting left waiting for an answer that cannot come.
                Some(Publication::Cancelled) => Some(PublishEvent::Cancel),
                Some(Publication::Publishing { .. }) | None => None,
            };
            if let Some(ev) = ev {
                self.publish_step(&app, ev);
            }
        }
    }

    /// An app publish's step, applied: the table's (by type, `AppPublishState::step`), then its effects.
    fn publish_step(&mut self, app: &str, ev: PublishEvent) {
        let now = self.server.page.now();
        let Some(p) = self.publishes.get_mut(app) else { return };
        let (effects, cell) = p.step(&ev);
        if cell == app_publish::Cell::Impossible {
            self.impossible_cells += 1;
        }
        for effect in effects {
            match effect {
                PublishEffect::SendSite => {
                    let Some((code, web)) = self.publish_sites.get(app).cloned() else { continue };
                    if let Err(w) = self.send_site(app, &code, web, now) {
                        self.publish_step(app, PublishEvent::SiteRefused(w));
                    }
                }
                // A SHARED piece (every app of one build carries the same sets) is withdrawn only when NO other
                // publish still owes it (the architect's R1): derived from the routing and each publish's own state.
                PublishEffect::WithdrawPieces(keys) => {
                    for k in keys {
                        if !self.owed_elsewhere(&k, app) {
                            self.server.page.cancel_app_put(&k);
                        }
                    }
                }
                PublishEffect::CancelSite => self.server.page.cancel_site(app),
            }
        }
        if self.publishes.get(app).is_some_and(|p| p.get().ended()) {
            self.unroute(app);
        }
    }

    /// `app`'s site LINK: its contract's instance id, as the node serves it (`/v1/contract/web/<link>/`). The same
    /// for every publish (builder#117). `None`: not an app id, or this head has no single key.
    pub fn site_link(&self, site_code: &[u8], app: &str) -> Option<String> {
        site_contract(site_code, &self.art.register_params, app).map(|c| {
            let mut id = [0u8; 32];
            id.copy_from_slice(&c.key().id().as_bytes()[..32]);
            site_text(&id)
        })
    }

    /// Has the node's signer named the Register this page signs under (so a site has an authority)?
    pub fn register_params_known(&self) -> bool {
        !self.art.register_params.is_empty()
    }

    /// How `app`'s site publication stands: the page's, the one owner. `None`: never published here.
    pub fn publication(&self, app: &str) -> Option<Publication> {
        self.server.page.publication(app)
    }

    /// The person cancels `app`'s publication: it ends `Cancelled` and its bytes go.
    pub fn cancel_site(&mut self, app: &str) {
        self.server.page.cancel_site(app);
        self.pump();
    }

    /// The site in flight whose contract id is `id`, by app.
    fn site_by_id(&self, id: &[u8; 32]) -> Option<String> {
        self.sites.iter().find(|(_, s)| s.id == *id).map(|(a, _)| a.clone())
    }

    /// The site in flight whose contract key is `key`, by app.
    fn site_by_key(&self, key: &str) -> Option<String> {
        self.sites.iter().find(|(_, s)| s.published_key() == Some(key)).map(|(a, _)| a.clone())
    }

    /// A site's READ (rule 4: one read, published or audited): its id, and whether to FOLLOW it (subscribe; only a
    /// publisher does). Refused, by name, for a site this page neither publishes nor audits. `pump` frames it.
    fn site_read(&self, app: &str) -> Result<([u8; 32], bool), String> {
        match self.sites.get(app) {
            Some(site) => Ok((site.id, site.published_key().is_some())),
            None => Err(format!("a read of {app}'s site, which is not being published or audited")),
        }
    }

    /// A site's PUT: its contract and web part, only while this page PUBLISHES it. `pump` frames it.
    fn site_put(&self, app: &str) -> Result<(ContractContainer, Vec<u8>), String> {
        match self.sites.get(app).map(|s| &s.role) {
            Some(SiteRole::Publishing { contract, web, .. }) => Ok((contract.clone(), web.clone())),
            Some(SiteRole::Auditing) => Err(format!("a PUT of {app}'s site, which is only being audited")),
            None => Err(format!("a PUT of {app}'s site, which is not being published")),
        }
    }

    /// READ `id`'s SITE FOR AN AUDIT, under `label` (sdk#493): the site's `ReadHead` is then framed, by the same GET a
    /// publisher's is (never subscribed, never PUT or signed), and its answer comes back as that label's `Head`. A
    /// site this page PUBLISHES is read already. Ended by [`PageIo::end_site_audit`].
    pub fn audit_site(&mut self, label: &str, id: [u8; 32]) {
        self.sites.entry(label.to_string()).or_insert(Site { id, role: SiteRole::Auditing });
    }

    /// The audit of `label`'s site is over: it is no longer read (a site being published stays).
    pub fn end_site_audit(&mut self, label: &str) {
        if self.sites.get(label).is_some_and(|s| matches!(s.role, SiteRole::Auditing)) {
            self.sites.remove(label);
        }
    }

    /// A person cancels the pending PUT of `key` (the page's, named).
    pub fn cancel_app_put(&mut self, key: &str) {
        self.server.page.cancel_app_put(key);
    }

    /// Where the app's PUT of `key` stands (the page's [`page::AppPut`]).
    pub fn app_put(&self, key: &str) -> Option<&page::AppPut> {
        self.server.page.app_put(key)
    }

    /// Node answers this page did not own (see `others`), oldest first.
    pub fn take_others(&mut self) -> Vec<Incoming> {
        std::mem::take(&mut self.others)
    }

    /// PROVISION the signer: register its delegate (from its wasm) and hand it
    /// the head's key, the Register it signs for and the Block code. The key
    /// is the page's TEST key, minted by the caller and forgotten after this
    /// (real device keys are sdk#14). `provisioned()` turns true when the
    /// signer answers `Provisioned`.
    pub fn provision(&mut self, signer: DelegateContainer, signing_key: Vec<u8>) {
        // A READER installs nothing on the node it reads from (sdk#239): the table's refused call, named.
        self.open(OpenEvent::Provide(signer, signing_key));
    }

    /// The signer said it holds the key and the naming (or this is a reader: nothing of its own to open).
    pub fn provisioned(&self) -> bool {
        matches!(self.opening.get(), Opening::Open { .. } | Opening::Reader)
    }

    /// THE SOCKET WAS REPLACED (sdk#376). Everything that belonged to the old
    /// connection is dropped or asked again, in THIS call, so the frames leave
    /// with the caller's next `take_frames`:
    /// - the reassembly of a chunked reply half-received on the old socket (its
    ///   stream ids restart; joined to the new connection's it would decode as a
    ///   message nobody sent);
    /// - the head subscription: the node held it for the old connection, so it
    ///   is not "answered" on this one until the re-sent head read is
    ///   (`Page::reconnected` re-sends it: a GET with subscribe, the one path).
    pub fn reconnected(&mut self, now: Ms) {
        self.frames = wire::Reassembler::default();
        self.sub(SubEvent::Reconnected);
        // Both tables' Reconnected columns: nothing of the opening is lost (the signer's install survives; what is
        // out is re-sent on the RTO), and the head exists or not whatever the socket did.
        self.open(OpenEvent::Reconnected);
        self.head(HeadEvent::Reconnected, now);
        self.server.page.reconnected(now);
        self.tick(now);
    }

    /// The head Register's instance id (what `Identity` reports as `head_id`).
    /// THE HEAD SUBSCRIPTION AS IT REALLY IS (sdk#259): asked, answered, and
    /// how many head moves it has delivered. The page path's `LiveMode` is
    /// built from this — the Session's own `subscribed`/`watching` belong to
    /// the delegate path, which no longer exists, so reporting from them said
    /// "Polled" on a page that was subscribed the whole time.
    pub fn head_subscription(&self) -> HeadSubscription {
        HeadSubscription {
            asked: self.head_sub.get() != HeadSub::Unasked,
            answered: self.head_sub.get() == HeadSub::Subscribed,
            changes: self.head_changes,
            failed: self.head_failed,
            // Opening ended — refused in someone's words, or its re-asks spent:
            // there is no head read coming, so no subscription either.
            ended: self.refused().map(str::to_string),
        }
    }

    pub fn register_id(&self) -> [u8; 32] {
        self.register_id
    }

    /// Tell the server what the signer holds, once it is provisioned.
    pub fn signer_provisioned(&mut self) {
        if self.read_only() {
            return;
        }
        self.server.set_facts(SignerFacts { head_writable: true, head_id: self.register_id });
        // The user's own tree was NOT yet theirs to sign (`no_tree_yet`)
        // and now is: the queue held while it could not sign is cut, as ONE
        // commit.
        if matches!(self.asked(), Some(Asked::NoKey | Asked::NoSigner(_))) {
            self.step_can_sign();
        }
    }

    /// Tell the engine whether this page can SIGN its head now
    /// (`engine::Event::CanSign`): derived from the ONE signer decision at
    /// each of its changes -- the answer "no key here" / "no signer here"
    /// (it cannot), and that tree's provisioning (it can). Kept nowhere else.
    fn step_can_sign(&mut self) {
        let can = !self.no_tree_yet();
        self.server.page.event(engine::Event::CanSign(can));
    }

    /// A client protocol frame (what `sdk::Client` would have sent a delegate).
    pub fn client(&mut self, bytes: &[u8]) {
        self.server.client(bytes);
        self.pump();
    }

    /// A frame from the node.
    ///
    /// Returns whether the frame was THIS page's — what it asked for, by
    /// contract id or key. One socket can carry several pages (a person's own
    /// tree and the trees they read, sdk#239): each is offered every frame and
    /// takes only its own, and a frame nobody takes is counted once, by the
    /// caller. So a frame that is not this page's is left alone here, not
    /// counted as unusable.
    pub fn inbound(&mut self, bytes: &[u8], now: Ms) -> bool {
        let incoming = wire::unframe(&mut self.frames, bytes);
        if !self.owns(&incoming) {
            return false;
        }
        match incoming {
            Incoming::Got { id, state } => {
                if id == self.register_id {
                    self.head(HeadEvent::Got, now);
                    // The GET that carried `subscribe` was answered: the node
                    // holds this page's subscription to the head (sdk#259).
                    self.sub(SubEvent::Got);
                    // The head WHOLE (root ‖ ledger), tolerantly: the root is
                    // the value's first 32 bytes whatever ledger follows.
                    self.server.node(Answer::Head { label: Label::Head, read: page::HeadRead::from_record(&state) }, now);
                } else if let Some(app) = self.site_by_id(&id) {
                    // A site's record is its framing's META. A state that does not frame is no answer (the site
                    // contract admits none): named, and the read stays silent, re-asked on the RTO.
                    match contract_keys::site::framing(&state) {
                        Some((meta, _)) => {
                            let read = page::HeadRead::from_record(meta);
                            self.server.node(Answer::Head { label: Label::Site(app), read }, now)
                        }
                        None => self.say(page::unusable::Site::of("page-io::said::site-not-web"), format!("a site state for {app} that is not a web framing")),
                    }
                } else if let Some(cid) = self.by_contract.get(&id).copied() {
                    let body = wire::block::block_of_state(&state).map(|(_, b)| b.to_vec()).unwrap_or_default();
                    self.server.node(Answer::Got { id: cid, bytes: body }, now);
                } else {
                    self.say(page::unusable::Site::of("page-io::said::get-unasked"), "a GET answer for a contract this page never asked".into());
                }
            }
            // A REFUSAL is not an answer about the contract: only the node's
            // NotFound says it is absent (#332 ruling). A refused GET stays
            // unanswered and the page's sender re-asks it on the RTO (rule 7);
            // read as absent, a head refusal on a signer with no record would
            // open an empty tree over an app that exists, and a block
            // refusal would end a read the next ask could serve.
            Incoming::GetFailed { id, why: wire::GetFail::Refused(_) } => {
                if id == self.register_id {
                    self.head_failed += 1;
                    self.sub(SubEvent::Refused);
                }
            }
            // TOO LARGE TO READ IS AN ANSWER (rule 8; the builder's site re-asked for ever): a site's read-back ENDS its
            // publication, named; any other GET is named (a head or a block is never near the bound).
            Incoming::GetFailed { id, why: wire::GetFail::TooLarge { bytes } } => {
                let said = |what: &str| format!("the node answered {what} with {bytes} bytes, over the {} this page reads (wire::MAX_REASSEMBLED)", wire::MAX_REASSEMBLED);
                if let Some(app) = self.site_by_id(&id) {
                    let said = said(&format!("{app}'s site read"));
                    self.say(page::unusable::Site::of("page-io::said::site-too-large"), said.clone());
                    self.server.node(Answer::SiteUnreadable { app, said }, now);
                } else if id == self.register_id {
                    self.say(page::unusable::Site::of("page-io::said::head-too-large"), said("the head's read"));
                } else if self.by_contract.contains_key(&id) {
                    self.say(page::unusable::Site::of("page-io::said::block-too-large"), said("a block's read"));
                } else {
                    self.say(page::unusable::Site::of("page-io::said::get-unasked"), "a GET answer for a contract this page never asked".into());
                }
            }
            Incoming::GetFailed { id, why: wire::GetFail::NotFound } => {
                if let Some(app) = self.site_by_id(&id) {
                    // No site at this address: the genesis (only a NotFound says so).
                    self.server.node(Answer::Head { label: Label::Site(app), read: None }, now);
                } else if id == self.register_id {
                    self.head_failed += 1;
                    self.sub(SubEvent::NotFound);
                    // The node's explicit NotFound for the head — which a
                    // PEERED node can answer falsely (F55) — is "no head" ONLY if the signer holds no
                    // record for this register. Otherwise the head exists and
                    // this is SILENCE: re-asked on the RTO with no end (rule 8),
                    // shown as "not answering for N s". Opening an empty tree over an existing app
                    // would have its first commit PUT a second register that
                    // F56 then merges against the real one (sdk#175). Machine 2 decides which.
                    self.head(HeadEvent::NotFound, now);
                } else if let Some(cid) = self.by_contract.get(&id).copied() {
                    self.server.node(Answer::GetMissed(cid), now);
                }
            }
            Incoming::Ack(wire::AckKind::Put(key)) | Incoming::Ack(wire::AckKind::Updated(key))
                if key == self.register_key || self.by_key.contains_key(&key) =>
            {
                if key == self.register_key {
                    self.head(HeadEvent::PutAcked, now);
                    self.server.node(Answer::Updated { label: Label::Head }, now);
                } else if let Some(cid) = self.by_key.get(&key).copied() {
                    self.server.node(Answer::PutOk(cid), now);
                }
            }
            // THE SIGNER'S REGISTRATION ANSWERED — or, once it has been, an
            // EMPTY response to its outstanding first request. `wire::unframe`
            // maps ANY empty DelegateResponse to `Ack(Registered)`, so the two
            // look alike: the first is the registration, and any after it,
            // while the first request is out, is "no answer" — re-sent by the
            // page's sender on its RTO — never a second registration.
            //
            // Except when this page only ASKS: then it registered nothing, and
            // (measured on 0.2.136) a node WITHOUT the signer delegate answers
            // a request to it EMPTY — the node's own answer, "no such signer
            // here" (another user's node). A page that registered the signer itself
            // reads an EMPTY as a request that arrived before the registration
            // took (#260) — NOT an answer — and leaves it to the RTO.
            Incoming::Ack(wire::AckKind::Registered(key)) if key == self.art.signer.to_string() => {
                self.open(OpenEvent::EmptyAck("no signer on this node: it answered EMPTY".into()));
            }
            // "NO SUCH DELEGATE HERE", naming it: what 0.2.137+ answers where 0.2.136 answered EMPTY (#5729, sdk#439).
            // The same two readings as the EMPTY above: to a page only ASKING, the answer; to a page that registered
            // the signer itself, a request that arrived before the registration took -- not an answer, re-sent on
            // the RTO.
            Incoming::DelegateMissing { key } if key == self.art.signer.to_string() => {
                self.open(OpenEvent::DelegateMissing("no signer on this node: it answered Missing".into()));
            }
            Incoming::DelegateMissing { key } => self.say(page::unusable::Site::of("page-io::said::delegate-missing"), format!("the node has no delegate {key}")),
            // The node's delegate BACKOFF after a failure: not an answer to anything; the request it throttled is
            // re-sent on the page's clock. Never a refusal (sdk#439).
            Incoming::DelegateThrottled { .. } => {}
            // A site's PUT was answered: the page reads it back (it says nothing about which record was kept).
            Incoming::Ack(wire::AckKind::Put(key)) | Incoming::Ack(wire::AckKind::Updated(key)) if self.site_by_key(&key).is_some() => {
                let app = self.site_by_key(&key).expect("matched");
                self.server.node(Answer::Updated { label: Label::Site(app) }, now)
            }
            Incoming::PutFailed { key, said } if self.site_by_key(&key).is_some() => {
                let app = self.site_by_key(&key).expect("matched");
                self.put_refused(Refused::Site(app), said, now)
            }
            // The app's PUT: the page ends its deadline.
            Incoming::Ack(wire::AckKind::Put(key)) if self.app_contracts.contains_key(&key) => {
                self.server.node(Answer::AppPutOk(key.clone()), now);
                self.piece_answered(&key, true);
            }
            // A PUT answer nobody here sent: handed back unread.
            answer @ Incoming::Ack(wire::AckKind::Put(_)) => self.others.push(answer),
            // A REFUSAL OF A PUT OF OURS, keyed or by the node's text (the keyless form 0.2.136/0.2.138 use, sdk#433),
            // a block's or an app contract's (a piece, sdk#516): ONE finality rule, `put_refused`.
            Incoming::PutFailed { key, said } if self.by_key.contains_key(&key) => {
                let cid = self.by_key[&key];
                self.put_refused(Refused::Block(cid), said, now)
            }
            Incoming::PutFailed { key, said } if self.app_contracts.contains_key(&key) => self.put_refused(Refused::App(key), said, now),
            // A string from the node is attributed ONLY to a PUT of ours still on the wire.
            Incoming::PutFailedByText { key, said } => match self.refused_by_text(&key) {
                Some(whose) => self.put_refused(whose, said, now),
                // It never names an op this page does not have: counted, ends nothing.
                None => *self.node_errors.entry("put_error_unattributed").or_insert(0) += 1,
            },
            // OUR OWN REGISTER never takes the finality rule: a head update's fate is decided by its READ-BACK (sdk#225:
            // the page reads the register and adopts only what it shows -- Published, or Lost to a writer that won),
            // never by the PUT's answer. A refusal, validation words included (a stale seq another writer beat), is
            // reported, and the op stays waiting -- re-sent on its RTO (sdk#431's pin) -- until the read-back decides.
            Incoming::PutFailed { key, said } if key == self.register_key => self.say(page::unusable::Site::of("page-io::said::register-put-refused"), format!("the node refused: {said}")),
            answer @ Incoming::PutFailed { .. } => self.others.push(answer),
            Incoming::EngineBytes(msgs) => {
                for m in msgs {
                    let answer = wire::signer::read_answer(&m);
                    // A READER (with a HeldSigner) takes only the Held answers it asked for; the rest is another page's.
                    if self.read_only() && !matches!(answer, Some((id, signer_proto::Answer::Held { .. })) if self.held.contains_key(&id)) {
                        continue;
                    }
                    // A REAL answer to the signer's first request ends it: no
                    // more re-sends, and a later empty response is nobody's.
                    if matches!(answer, Some((REGISTER_QUERY_ID | PROVISION_ID, _))) {
                        self.server.page.ext_answered(Ext::SignerFirst, now);
                    }
                    if matches!(answer, Some((RECORD_QUERY_ID, _))) {
                        self.server.page.ext_answered(Ext::AskRecord, now);
                    }
                    match answer {
                        // The first exchange's answers: Machine 1 decides what each means in the page's state.
                        Some((REGISTER_QUERY_ID, signer_proto::Answer::Register { params })) => {
                            let named = params.map(|p| {
                                let id = self.register_id_of(&p);
                                (p, id)
                            });
                            self.open(OpenEvent::Register(named));
                        }
                        Some((_, signer_proto::Answer::Provisioned)) => {
                            self.open(OpenEvent::Provisioned);
                        }
                        Some((PROVISION_ID, signer_proto::Answer::Refused(signer_proto::Why::KeyAlreadyProvisioned))) => {
                            self.open(OpenEvent::KeyAlreadyHere);
                        }
                        Some((PROVISION_ID, signer_proto::Answer::Refused(why))) => {
                            self.open(OpenEvent::ProvisionRefused(format!("{why:?}")));
                        }
                        Some((REGISTER_QUERY_ID, signer_proto::Answer::Refused(why))) => {
                            self.open(OpenEvent::QueryRefused(format!("{why:?}")));
                        }
                        // The record query's answer (`ask_record`).
                        Some((RECORD_QUERY_ID, answer)) => {
                            use signer_proto::{Answer as A, Why};
                            let has = match answer {
                                // No record, no head it can read: only genesis
                                // would be signable, and the unheld root stops it.
                                A::Refused(Why::RootNotHeld) => Some(false),
                                // A record from genesis, or a later truth.
                                A::AlreadySigned(_) | A::NotNext { .. } | A::Refused(Why::Forked { .. }) => Some(true),
                                _ => None,
                            };
                            self.head(HeadEvent::Record(has), now);
                        }
                        Some((id, signer_proto::Answer::Held { present })) => {
                            // As the signer said it: `present[i]` answers the op's `ids[i]`; a short one is the page's
                            // to see (sdk#455). An answer to no batch of ours is not ours.
                            if let Some(batch) = self.held.remove(&id) {
                                self.server.node(Answer::Held { batch, present }, now);
                            }
                        }
                        Some((id, answer)) => self.server.node(Answer::Signer { id, answer }, now),
                        None => self.say(page::unusable::Site::of("page-io::said::delegate-not-signer"), "a delegate message that is not the signer's".into()),
                    }
                }
            }
            // The head moved on the node (the subscription the head read
            // took): the RELOAD TRIGGER. A hint only — the page READS the
            // register and adopts only what that read shows (sdk#225). A FULL
            // state may confirm this page's OWN owed head (sdk#378 P3); the
            // page decides, through its one read-back rule.
            Incoming::HeadChanged { key, state } if key == self.register_key => {
                // What the subscription DELIVERED: counted, so "subscribed"
                // can be told from "subscribed and being told" (sdk#259).
                self.head_changes += 1;
                // S2: a move delivered on this connection proves the subscription (sdk#490).
                self.sub(SubEvent::HeadChanged);
                match state.as_deref().and_then(page::HeadRead::from_record) {
                    Some(read) => self.server.head_pushed(read),
                    None => self.server.head_hint(),
                }
            }
            Incoming::Refused(r) => {
                // A node error naming NO op (sdk#433): COUNTED by its reason code, and it ends nothing and re-arms
                // nothing -- the RTO stays the one clock for every op on the wire.
                *self.node_errors.entry(r.code).or_insert(0) += 1;
                // While the first exchange is unanswered, a refusal that names
                // nothing is the node refusing IT: opening ends, by name (Machine 1's NodeRefused column).
                self.open(OpenEvent::NodeRefused(r.said.clone()));
                self.say(page::unusable::Site::of("page-io::said::node-refused-reply"), format!("the node refused: {}", r.said))
            }
            Incoming::Unusable(u) => self.say(page::unusable::Site::of("page-io::said::incoming-unusable"), format!("{u:?}")),
            // EVERY KIND owns() claims is DECIDED here, with no catch-all (sdk#483, "Structure before code"): a new
            // wire answer fails to compile until it is handled, never falls into `_` and vanishes.
            // A registration of a delegate that is not this page's signer: this page registered no other, so it is
            // named, and it answers nothing here.
            Incoming::Ack(wire::AckKind::Registered(key)) => self.say(page::unusable::Site::of("page-io::said::foreign-registered"), format!("the node registered a delegate this page did not register: {key}")),
            // An UPDATE answer for a contract this page did not update (owns() claims only its own keys, so this is a
            // key of its own it has no update out for): named.
            Incoming::Ack(wire::AckKind::Updated(key)) => self.say(page::unusable::Site::of("page-io::said::update-unasked"), format!("an UPDATE answer this page has no update out for: {key}")),
            // The node confirms the HEAD subscription (the GET-with-subscribe's own answer already says so): the same
            // fact, recorded where `head_subscription()` reads it (sdk#259).
            Incoming::Ack(wire::AckKind::Subscribed(key)) if key == self.register_key => self.sub(SubEvent::SubscribedAck),
            // A SITE's subscription answer: the page does not follow a site (owns() takes it; read by nobody).
            Incoming::Ack(wire::AckKind::Subscribed(_)) => {}
            // `Ok` names nothing and answers nothing (wire: "a step must not rely on this one"); every op this page
            // sent ends by its own named answer or is re-sent on the RTO.
            Incoming::Ack(wire::AckKind::Ok) => {}
            // A SITE's change (owns() takes it): the page does not follow a site.
            Incoming::HeadChanged { .. } => {}
            // A chunk of a larger message: the reassembler holds it; nothing to hand on yet.
            Incoming::Partial => {}
        }
        self.pump();
        true
    }

    /// Which PUT of ours ON THE WIRE the node's keyless refusal text names: a block still waiting, or an app contract
    /// (a piece) still pending. `None`: not ours, or already answered.
    fn refused_by_text(&self, key: &str) -> Option<Refused> {
        if let Some(cid) = self.by_key.get(key).copied().filter(|cid| self.server.page.put_waiting(cid)) {
            return Some(Refused::Block(cid));
        }
        if self.app_contracts.contains_key(key) && matches!(self.server.page.app_put(key), Some(page::AppPut::Pending)) {
            return Some(Refused::App(key.to_string()));
        }
        // A site whose publication is still in flight (its PUT is the page's `Update` of that site).
        self.site_by_key(key).filter(|app| matches!(self.server.page.publication(app), Some(Publication::Publishing { .. }))).map(Refused::Site)
    }

    /// THE ONE FINALITY RULE for a refusal of a PUT of ours (sdk#516; #433 for blocks): FINAL only in a contract's
    /// own VALIDATION words (`wire::is_validation_refusal`), for a block, an app contract (a piece) and a site alike,
    /// keyed or keyless (the register's head update is decided by its read-back instead; see its arm). A final one ends the PUT (a block: `PutRefused`, never put again; an app contract: `AppPutRefused`).
    /// Any other words are TRANSIENT: reported, and the op stays waiting, re-sent on its RTO (rule 7).
    fn put_refused(&mut self, whose: Refused, said: String, now: Ms) {
        let fin = wire::is_validation_refusal(&said);
        match (whose, fin) {
            (Refused::Block(cid), true) => {
                self.say(page::unusable::Site::of("page-io::said::block-invalid"), format!(
                    "the node's Block contract refused block {} as invalid (\"{said}\"): an encoding defect, or a node on another contract epoch",
                    engine::short_id(&cid)
                ));
                self.server.node(Answer::PutRefused { id: cid, transient: false }, now)
            }
            (Refused::Block(cid), false) => self.say(page::unusable::Site::of("page-io::said::block-refused"), format!("the node refused block {}: {said}", engine::short_id(&cid))),
            (Refused::App(key), true) => {
                self.server.node(Answer::AppPutRefused { key: key.clone(), said }, now);
                self.piece_answered(&key, false);
            }
            (Refused::App(key), false) => self.say(page::unusable::Site::of("page-io::said::app-put-refused"), format!("the node refused app contract {key} (not final; re-sent): {said}")),
            // A site's FINAL refusal ends its publication (the page's consequence, `Publication::Refused`); a transient
            // one is reported, and the site's PUT is re-sent on its RTO.
            (Refused::Site(app), true) => self.server.node(Answer::SiteRefused { app, said }, now),
            (Refused::Site(app), false) => self.say(page::unusable::Site::of("page-io::said::site-put-refused"), format!("the node refused site {app}'s PUT (not final; re-sent): {said}")),
        }
    }

    /// Is this frame one this page asked for? A READER owns only its head and
    /// its blocks; a writer also owns its signer's answers, answers that name
    /// nothing, and PUT answers for the app's own contracts (handed back).
    fn owns(&self, incoming: &Incoming) -> bool {
        let mine = |id: &[u8; 32]| *id == self.register_id || self.by_contract.contains_key(id) || self.sites.values().any(|s| s.id == *id);
        let my_key = |k: &String| *k == self.register_key || self.by_key.contains_key(k) || self.sites.values().any(|s| s.published_key() == Some(k.as_str()));
        match incoming {
            Incoming::Got { id, .. } | Incoming::GetFailed { id, .. } => mine(id),
            Incoming::Ack(wire::AckKind::Put(k)) | Incoming::PutFailed { key: k, .. } => my_key(k) || !self.read_only(),
            Incoming::PutFailedByText { key, .. } => my_key(key) || !self.read_only(),
            Incoming::Ack(wire::AckKind::Updated(k)) | Incoming::Ack(wire::AckKind::Subscribed(k)) => my_key(k),
            // A site's change is its own (taken, and read by nobody: the page does not follow a site).
            Incoming::HeadChanged { key, .. } => *key == self.register_key || self.sites.values().any(|s| s.published_key() == Some(key.as_str())),
            Incoming::Partial => true,
            Incoming::DelegateMissing { key } => *key == self.art.signer.to_string() || !self.read_only(),
            // A reader takes a signer message only if it answers a `Held` it asked (its HeldSigner, sdk#493): the
            // person's own page on the same socket asks the same signer, and its answers are its own.
            Incoming::EngineBytes(msgs) if self.read_only() => {
                self.held_signer.is_some() && msgs.iter().any(|m| matches!(wire::signer::read_answer(m), Some((id, signer_proto::Answer::Held { .. })) if self.held.contains_key(&id)))
            }
            Incoming::EngineBytes(_) | Incoming::Ack(_) | Incoming::DelegateThrottled { .. } | Incoming::Refused(_) | Incoming::Unusable(_) => {
                !self.read_only()
            }
        }
    }

    /// The page's clock.
    pub fn tick(&mut self, now: Ms) {
        self.server.tick(now);
        self.pump();
    }

    /// Opening was REFUSED — by the signer or the node, in its words.
    pub fn refused(&self) -> Option<&str> {
        match self.opening.get() {
            Opening::Refused { why, .. } => Some(why),
            Opening::New | Opening::Registering { .. } | Opening::Querying { .. } | Opening::NeedsKey { .. } | Opening::Provisioning { .. } | Opening::Open { .. } | Opening::Asking { .. } | Opening::Reader => None,
        }
    }

    /// The request that has waited longest for an answer, and for how long
    /// (ms): what a page shows as "not answering for N s" (rule 8). The
    /// page's sender keeps re-sending it; this never ends anything.
    pub fn not_answering(&self) -> Option<(String, u64)> {
        self.server.page.not_answering()
    }

    /// Still waiting on the first exchange past its first RTO.
    pub fn stalled(&self) -> bool {
        (self.server.page.ext_waiting(Ext::RegisterSigner) || self.server.page.ext_waiting(Ext::SignerFirst))
            && self.server.page.not_answering().is_some_and(|(_, ms)| ms >= page::rto::RTO_INITIAL_MS as u64)
    }

    /// Is anything still owed an answer or a re-send (`page::Page::waiting`)? `false`: at rest until something
    /// new arrives -- only the idle page's backstop read is left on its clock.
    pub fn waiting(&self) -> bool {
        self.server.page.waiting()
    }

    /// When the page's next timer falls (a host arms a one-shot for it): the
    /// page's own deadlines, which the signer's requests are among.
    pub fn next_due(&self) -> Option<Ms> {
        self.server.page.next_due()
    }

    /// Frames for the node, in order.
    pub fn take_frames(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.out)
    }

    /// Protocol replies for the client, in order.
    pub fn take_replies(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.replies)
    }

    /// Writes the engine took forced past their reads (sdk#235).
    pub fn forced_writes(&self) -> u64 {
        self.server.forced_writes()
    }

    /// Node errors that named no op, by reason code (sdk#433): the unattributed-node-error diagnostic.
    pub fn node_errors(&self) -> &BTreeMap<&'static str, u64> {
        &self.node_errors
    }

    /// A look at page-io's own lines said and not yet taken, for TESTS only (the app's one reader is
    /// [`PageIo::take_unusable`]).
    #[doc(hidden)]
    pub fn unusable(&self) -> Vec<String> {
        self.unusable.peek()
    }

    /// THE ONE READ of the lines said to the app (sdk#482), draining: page-io's own, then its page's.
    pub fn take_unusable(&mut self) -> Vec<String> {
        let mut all = self.unusable.take();
        all.extend(self.server.page.take_unusable());
        all
    }

    /// SAY `line` to the app at `site`: recorded as one `Key::Said` in the page's recording (no text), and kept.
    fn say(&mut self, site: page::unusable::Site, line: String) {
        self.server.page.record_said(site);
        self.unusable.push(line);
    }

    /// Ask the signer whether it holds a record for this register, WITHOUT
    /// being able to sign anything: a sign request from the genesis naming a
    /// root no node holds. `signer::decide` answers it in this order —
    /// `AlreadySigned` if its record's prev is the genesis, `NotNext` if its
    /// record or a head it can read is the truth, and only then, with neither,
    /// `Refused(RootNotHeld)` because the root is not held. Nothing is ever
    /// signed: the root check comes before any signature. ASSUMPTION (a verb
    /// of its own would be clearer; the signer's owner may add one): the
    /// order of `decide` stays as it is, which signer/tests pin.
    fn ask_record(&mut self, now: Ms) {
        self.server.page.send_ext(Ext::AskRecord, now);
    }

    fn next_stream(&mut self) -> u32 {
        if self.stream_base == 0 {
            self.stream = self.stream.wrapping_add(1).max(1);
            return self.stream;
        }
        // A READER's ids stay in its own range: `base` in the top byte, a
        // counter below it, so no two pages on one socket share a stream id.
        self.stream = (self.stream.wrapping_add(1) & STREAM_COUNTER).max(1);
        self.stream_base | self.stream
    }


    /// Every op the server emitted becomes a frame; every reply it made is
    /// queued for the client.
    fn pump(&mut self) {
        self.replies.extend(self.server.take_replies());
        let mut not_held = Vec::new();
        let mut not_sent = Vec::new();
        let mut no_head = false;
        for op in self.server.take_ops() {
            // No tree yet: there is no head to read, and asking the node for
            // one would name a Register nobody has made. Answered here.
            if matches!(op, Op::ReadHead { label: Label::Head }) && self.no_tree_yet() {
                no_head = true;
                continue;
            }
            if self.read_only() {
                match op {
                    // A plain reader has no signer to ask: its blocks are "not held" here, and fetched. A reader with its own
                    // node's HeldSigner (a keeper's audit, sdk#493) asks it, through the one framing below.
                    Op::AskHeld { batch, ids } if self.held_signer.is_none() => {
                        not_held.push((batch, ids.len()));
                        continue;
                    }
                    Op::AskHeld { .. } => {}
                    // A view's page makes no commit op; one that arrives is
                    // REFUSED BY NAME and handed back as the op's answer --
                    // loud, and ended: nothing waits on a frame never sent.
                    Op::Update { .. } | Op::Sign { .. } | Op::PutApp { .. } | Op::Ext(_) => {
                        let why = format!("read-only: a {} is never sent from a view", op_name(&op));
                        self.say(page::unusable::Site::of("page-io::said::read-only-op"), why.clone());
                        not_sent.push((op, why));
                        continue;
                    }
                    // A repair PUT (the only PUT a view's page makes), reads.
                    Op::Put { .. } | Op::Get { .. } | Op::ReadHead { .. } => {}
                }
            }
            let stream = self.next_stream();
            let framed = match op {
                Op::Put { id, bytes } => match wire::block::block_state(&id, &bytes) {
                    Some(state) => {
                        let c = wire::block::block_contract(&self.art.block_code, &id);
                        self.by_key.insert(c.key().to_string(), id);
                        self.by_contract.insert(wire::block::contract_for(&self.art.block_code, &id), id);
                        wire::frame_put(c, WrappedState::new(state), stream)
                    }
                    None => Err("a block whose bytes hash under no kind".into()),
                },
                Op::Get { id } => {
                    let contract = wire::block::contract_for(&self.art.block_code, &id);
                    self.by_contract.insert(contract, id);
                    wire::frame_get(wire::contract_id(contract), false, stream)
                }
                Op::ReadHead { label: Label::Head } => {
                    self.sub(SubEvent::ReadSent);
                    wire::frame_get(wire::contract_id(self.register_id), true, stream)
                }
                // ONE read of a site, published or audited (rule 4); only a publisher follows it (subscribe).
                Op::ReadHead { label: Label::Site(app) } => self.site_read(&app).and_then(|(id, follow)| wire::frame_get(wire::contract_id(id), follow, stream)),
                // A site's write is a PUT of its framing around exactly the signer's record (invariant 2).
                Op::Update { label: Label::Site(app), state } => self.site_put(&app).and_then(|(contract, web)| wire::frame_put(contract, WrappedState::new(contract_keys::site::frame(&state, &web)), stream)),
                Op::Update { label: Label::Head, state } => {
                    if self.head_known.get().seen() {
                        wire::frame_update(self.register.key(), state, stream)
                    } else {
                        // The first head: the Register does not exist yet, and
                        // a PUT is what creates it (no delegate Install on this
                        // path). An existing one merges a PUT like an UPDATE.
                        wire::frame_put(self.register.clone(), WrappedState::new(state), stream)
                    }
                }
                Op::Sign { id, prev_seq, prev_root, seq, root, ledger, label } => {
                    let label = match label {
                        Label::Head => Ok(signer_proto::Label::Head),
                        Label::Site(app) => match self.sites.get(&app) {
                            Some(Site { id, role: SiteRole::Publishing { .. } }) => Ok(signer_proto::Label::Site { contract: *id, app }),
                            Some(Site { role: SiteRole::Auditing, .. }) => Err(format!("a sign for {app}'s site, which is only being audited")),
                            None => Err(format!("a sign for {app}'s site, which is not being published")),
                        },
                    };
                    label.and_then(|label| {
                        wire::signer::frame_sign(
                            &self.art.signer,
                            id,
                            signer_proto::Head { seq: prev_seq, root: prev_root },
                            signer_proto::Next { seq, root, ledger },
                            label,
                            stream,
                        )
                    })
                }
                // Page-io's own requests, sent and re-sent by the page's
                // sender (rule 5): framed HERE and nowhere else.
                Op::Ext(Ext::RegisterSigner) => match self.signer_container.clone() {
                    Some(c) => wire::frame_register_delegate(c, stream),
                    None => Err("the signer's registration, with no signer to register".into()),
                },
                Op::Ext(Ext::SignerFirst) => match self.opening.get().first() {
                    Some(First::Query) => wire::signer::frame_register_query(&self.art.signer, REGISTER_QUERY_ID, stream),
                    Some(First::Provision(key)) => wire::signer::frame_provision(
                        &self.art.signer,
                        PROVISION_ID,
                        key,
                        self.art.register_code.clone(),
                        self.art.register_params.clone(),
                        self.art.block_code.clone(),
                        stream,
                    ),
                    None => Ok(Vec::new()),
                },
                Op::Ext(Ext::AskRecord) => wire::signer::frame_sign(
                    &self.art.signer,
                    RECORD_QUERY_ID,
                    signer_proto::Head { seq: 0, root: self.server.page.published().1 },
                    signer_proto::Next { seq: 1, root: UNHELD_ROOT, ledger: Vec::new() },
                    signer_proto::Label::Head,
                    stream,
                ),
                Op::PutApp { key } => match self.app_contracts.get(&key) {
                    Some((c, st)) => wire::frame_put(c.clone(), st.clone(), stream),
                    None => Err(format!("an app PUT of {key}, whose contract this page does not hold")),
                },
                // ONE `Held` request of every id's contract, in the op's order (sdk#455).
                Op::AskHeld { batch, ids } => {
                    let hid = self.take_held_id();
                    self.held.insert(hid, batch);
                    let contracts = ids.iter().map(|id| wire::block::contract_for(&self.art.block_code, id)).collect();
                    // A reader's is its HeldSigner (Held only, by type); the person's own page asks its signer.
                    match self.held_signer.as_ref() {
                        Some(held) => held.frame_held(hid, contracts, stream),
                        None => wire::signer::frame_held(&self.art.signer, hid, contracts, stream),
                    }
                }
            };
            match framed {
                Ok(f) => self.out.extend(f),
                Err(e) => self.say(page::unusable::Site::of("page-io::said::frame-failed"), format!("could not frame an op: {e}")),
            }
        }
        if !not_held.is_empty() || !not_sent.is_empty() || no_head {
            // THE PAGE'S clock: page-io keeps no copy of it (one owner; a copy with another origin was sdk#397).
            let now = self.server.page.now();
            for (batch, n) in not_held {
                self.server.node(Answer::Held { batch, present: vec![false; n] }, now);
            }
            for (op, why) in not_sent {
                // An app's PUT has its refusal already (`AppPutRefused`); the
                // commit ops a view never makes have none, so `NotSent`.
                let answer = match op {
                    Op::PutApp { key } => Answer::AppPutRefused { key, said: why },
                    op => Answer::NotSent { op, why },
                };
                self.server.node(answer, now);
            }
            if no_head {
                self.server.node(Answer::Head { label: Label::Head, read: None }, now);
            }
            self.pump();
        }
        // An app publish whose site is in flight learns how the site ended (sdk#516).
        self.observe_publishes();
        // A site's bytes are page-io's only while its publication is in flight.
        let page = &self.server.page;
        self.sites.retain(|app, site| match site.role {
            SiteRole::Publishing { .. } => matches!(page.publication(app), Some(Publication::Publishing { .. })),
            // An audit ends by `end_site_audit`, not by a publication.
            SiteRole::Auditing => true,
        });
    }
}

fn op_name(op: &Op) -> &'static str {
    match op {
        Op::Put { .. } => "block PUT",
        Op::Update { .. } => "head update",
        Op::Sign { .. } => "sign request",
        Op::Get { .. } => "block GET",
        Op::ReadHead { .. } => "head read",
        Op::AskHeld { .. } => "held query",
        Op::PutApp { .. } => "app PUT",
        Op::Ext(_) => "signer request",
    }
}

use core_types::hex::encode as hex;

/// THE BATCHED `Held` ON THE WIRE (sdk#455), through `pump` (rule 5: the one place an op is framed). Batching is per
/// page STEP: the asks one step makes go as ONE signer `Held` request of every id's contract, in the op's order, and
/// its answer reaches the page under the op's batch.
#[cfg(test)]
mod held_batch {
    use super::*;
    use freenet_stdlib::client_api::{ClientError, ClientRequest, DelegateRequest, HostResponse};
    use freenet_stdlib::prelude::{ApplicationMessage, InboundDelegateMsg, OutboundDelegateMsg};

    fn io() -> PageIo {
        let (_, signer) = wire::delegate_from_code(b"held batch signer code");
        let mut io = PageIo::new(
            page::server::Server::new(page::Page::unstarted(engine::Params::default(), page::PutPath::Wrapper, Ms(0)), page::server::SignerFacts { head_writable: true, head_id: [0; 32] }),
            Artefacts { block_code: b"held batch block code".to_vec(), register_code: b"held batch register code".to_vec(), register_params: wire::register_params(&[1u8; 32], wire::HEAD_NAME), signer },
        );
        io.signer_provisioned();
        io
    }

    /// The signer `Held` requests in page-io's frames, as the node reads them: (signer id, contracts).
    fn helds_of(frames: &[Vec<u8>]) -> Vec<(u32, Vec<[u8; 32]>)> {
        let mut out = Vec::new();
        for f in frames {
            let Ok(ClientRequest::DelegateOp(DelegateRequest::ApplicationMessages { inbound, .. })) = bincode::deserialize::<ClientRequest>(f) else { continue };
            for m in inbound {
                if let InboundDelegateMsg::ApplicationMessage(am) = m {
                    if let Some((id, signer_proto::Request::Held { contracts })) = signer_proto::decode_request(&am.payload) {
                        out.push((id, contracts));
                    }
                }
            }
        }
        out
    }

    /// The signer's `Held` answer, as a node delivers it.
    fn held_answer(io: &PageIo, hid: u32, present: Vec<bool>) -> Vec<u8> {
        let answer = signer_proto::encode_answer(hid, &signer_proto::Answer::Held { present });
        bincode::serialize(&Ok::<HostResponse, ClientError>(HostResponse::DelegateResponse {
            key: io.art.signer.clone(),
            values: vec![OutboundDelegateMsg::ApplicationMessage(ApplicationMessage::new(answer))],
        }))
        .expect("encodes")
    }

    #[test]
    fn the_asks_of_one_page_step_are_one_held_request_of_every_id() {
        let mut io = io();
        let ids: Vec<Cid> = (0..signer_proto::MAX_HELD).map(|i| { let mut c = [0u8; 32]; c[..8].copy_from_slice(&(i as u64 + 1).to_be_bytes()); c }).collect();
        // 128 PUT answers in ONE page step (the page's own `answer`, no flush between them): each asks Held
        // (a wrapper-path PutOk), and the asks leave together when page-io pumps.
        for id in &ids {
            io.server.page.answer(Answer::PutOk(*id), Ms(1));
        }
        io.pump();
        let batch = helds_of(&io.take_frames());
        println!("{} ids asked in one step: {} Held request(s) of {:?} contract(s)", ids.len(), batch.len(), batch.iter().map(|(_, c)| c.len()).collect::<Vec<_>>());
        assert_eq!(batch.len(), 1, "the asks of one step were not ONE Held request");
        let want: Vec<[u8; 32]> = ids.iter().map(|id| wire::block::contract_for(&io.art.block_code, id)).collect();
        assert_eq!(batch[0].1, want, "the one request does not carry every id's contract in the op's order");
        // Its answer reaches the page under the op's batch, mapped by id: all present, every block confirmed.
        let bytes = held_answer(&io, batch[0].0, vec![true; ids.len()]);
        assert!(io.inbound(&bytes, Ms(2)), "the signer's Held answer was not taken");
        assert!(io.held.is_empty(), "the answered batch is still mapped");
        assert!(!io.server.page.waiting(), "a block of the answered batch is still owed an ask");
    }
}

/// THE OPENING TABLES' WIRING (sdk#484): a reader starts where its provenance says (each machine's ONE writer is by TYPE,
/// in opening.rs: a private field whose only `&mut` method is `step`), and
/// the Reconnected column holds through PageIo (its live gate is the probe live-reconnect, sdk#491).
#[cfg(test)]
mod opening_wiring {
    use super::*;

    fn owner() -> PageIo {
        let (_, signer) = wire::delegate_from_code(b"opening wiring signer");
        PageIo::new(
            page::server::Server::new(page::Page::unstarted(engine::Params::default(), page::PutPath::Page, Ms(0)), page::server::SignerFacts::default()),
            Artefacts { block_code: b"b".to_vec(), register_code: b"r".to_vec(), register_params: wire::register_params(&[1u8; 32], wire::HEAD_NAME), signer },
        )
    }

    /// Frames that register a delegate or carry a signer request: what a reconnect must NOT re-send for an open page.
    fn signer_frames(frames: &[Vec<u8>]) -> usize {
        use freenet_stdlib::client_api::{ClientRequest, DelegateRequest};
        frames
            .iter()
            .filter(|f| matches!(bincode::deserialize::<ClientRequest>(f), Ok(ClientRequest::DelegateOp(DelegateRequest::RegisterDelegate { .. } | DelegateRequest::ApplicationMessages { .. }))))
            .count()
    }

    /// THE MACHINES START ONLY AT CONSTRUCTION (the architect on #499/#503: what the type cannot say). Each machine's
    /// state is private and only its `step` writes it, but its INITIAL constructors are crate-visible, so a mid-life
    /// `self.head_sub = HeadSubState::unasked()` would reset it past `step`. This is a source check, the rule's allowed
    /// case, with its reason: no `.opening =` / `.head_known =` / `.head_sub =` anywhere, and the constructors are
    /// called only inside `new`, `build` or `reader`. THE CONTROL: a planted mid-life reset is caught.
    #[test]
    fn the_machines_are_started_only_at_construction() {
        fn violations(src: &str) -> Vec<String> {
            let code = &src[..src.find("#[cfg(test)]\nmod held_batch").expect("the first tests module")];
            let mut fn_name = String::new();
            let mut out = Vec::new();
            for (i, raw) in code.lines().enumerate() {
                let line = raw.split("//").next().unwrap_or_default();
                if let Some(rest) = line.trim_start().strip_prefix("pub fn ").or_else(|| line.trim_start().strip_prefix("fn ")) {
                    if raw.starts_with("    ") && !raw.starts_with("        ") {
                        fn_name = rest.split(['(', '<']).next().unwrap_or_default().to_string();
                    }
                }
                let reassigns = [".opening =", ".head_known =", ".head_sub ="].iter().any(|w| line.contains(w));
                let constructs = ["OpeningState::", "HeadKnownState::", "HeadSubState::"].iter().any(|w| line.contains(w));
                if reassigns || (constructs && !matches!(fn_name.as_str(), "new" | "build" | "reader")) {
                    out.push(format!("{}: in fn {fn_name}: {}", i + 1, raw.trim()));
                }
            }
            out
        }
        let src = include_str!("lib.rs");
        assert_eq!(violations(src), Vec::<String>::new(), "a machine started or reset outside construction");
        // THE CONTROL: a mid-life reset, planted in `reconnected`, is caught (so the reader finds real bodies).
        let planted = src.replacen("        self.frames = wire::Reassembler::default();", "        self.frames = wire::Reassembler::default();\n        self.head_sub = HeadSubState::unasked();", 1);
        assert_ne!(planted, src, "the control planted nothing");
        let caught = violations(&planted);
        assert!(caught.len() == 1 && caught[0].contains("in fn reconnected"), "a planted mid-life reset was not caught: {caught:?}");
    }

    /// A READER's head is NAMED (someone published it), never "the signer has a record" (nobody said so).
    #[test]
    fn a_reader_starts_named_and_reader() {
        let io = PageIo::reader(page::server::Server::new(page::Page::unstarted(engine::Params::default(), page::PutPath::Page, Ms(0)), page::server::SignerFacts::default()), b"b".to_vec(), [4; 32], 1);
        assert_eq!(io.head_known.get(), opening::HeadKnown::Named);
        assert_eq!(*io.opening.get(), Opening::Reader);
        assert!(io.provisioned() && io.read_only());
    }

    /// THE RECONNECTED COLUMN through PageIo: an OPEN page re-registers nothing and asks the signer nothing (the install
    /// survives; freenet v0.2.138 client_events.rs 1712-1730) and stays open (its head's re-read WITH subscribe is the
    /// live probe's to show, live-reconnect). THE CONTROL: a page still QUERYING keeps its query out (re-sent on its RTO), and is not open.
    #[test]
    fn a_reconnect_keeps_an_open_page_open_and_sends_the_signer_nothing() {
        let mut io = owner();
        let (c, _) = wire::delegate_from_code(b"opening wiring signer");
        io.begin(c);
        io.open(OpenEvent::EmptyAck("registered".into()));
        assert!(matches!(io.opening.get(), Opening::Querying { .. }), "THE SETUP: the registration's answer did not move to Querying");
        let mut querying = owner();
        let (c2, _) = wire::delegate_from_code(b"opening wiring signer");
        querying.begin(c2);
        querying.open(OpenEvent::EmptyAck("registered".into()));
        let params = wire::register_params(&[2u8; 32], wire::HEAD_NAME);
        let id = io.register_id_of(&params);
        io.open(OpenEvent::Register(Some((params, id))));
        assert!(io.provisioned(), "THE SETUP: the page is not open");
        io.take_frames();
        querying.take_frames();
        io.reconnected(Ms(5_000));
        let frames = io.take_frames();
        assert!(io.provisioned() && matches!(io.opening.get(), Opening::Open { .. }), "a reconnect changed an open page's opening");
        assert_eq!(signer_frames(&frames), 0, "an open page re-registered or asked the signer on a reconnect");
        assert_eq!(io.impossible_cells(), 0);
        querying.reconnected(Ms(5_000));
        assert!(matches!(querying.opening.get(), Opening::Querying { .. }) && !querying.provisioned(), "THE CONTROL: a querying page's opening moved on a reconnect");
    }
}

/// A READER WITH ITS OWN NODE'S HeldSigner (sdk#493: a keeper auditing someone else's app). It asks `Held` through
/// that signer and nothing else; on a socket it shares with the person's own page, each page takes only its own answers.
#[cfg(test)]
mod reader_held {
    use super::*;
    use freenet_stdlib::client_api::{ClientError, ClientRequest, DelegateRequest, HostResponse};
    use freenet_stdlib::prelude::{ApplicationMessage, InboundDelegateMsg, OutboundDelegateMsg};

    const SIGNER: &[u8] = b"reader held signer code";

    fn server() -> page::server::Server {
        page::server::Server::new(page::Page::unstarted(engine::Params::default(), page::PutPath::Wrapper, Ms(0)), page::server::SignerFacts::default())
    }

    fn reader(held: bool) -> PageIo {
        let (_, key) = wire::delegate_from_code(SIGNER);
        if held {
            PageIo::reader_with_held(server(), b"block code".to_vec(), [5; 32], 3, wire::signer::HeldSigner::of(key))
        } else {
            PageIo::reader(server(), b"block code".to_vec(), [5; 32], 3)
        }
    }

    fn owner() -> PageIo {
        let (_, signer) = wire::delegate_from_code(SIGNER);
        let mut io = PageIo::new(server(), Artefacts { block_code: b"block code".to_vec(), register_code: b"r".to_vec(), register_params: wire::register_params(&[1u8; 32], wire::HEAD_NAME), signer });
        io.signer_provisioned();
        io
    }

    /// The signer requests in `frames`, as the node reads them: (request id, is it a `Held`).
    fn requests(frames: &[Vec<u8>]) -> Vec<(u32, bool)> {
        let mut out = Vec::new();
        for f in frames {
            let Ok(ClientRequest::DelegateOp(DelegateRequest::ApplicationMessages { inbound, .. })) = bincode::deserialize::<ClientRequest>(f) else { continue };
            for m in inbound {
                if let InboundDelegateMsg::ApplicationMessage(am) = m {
                    if let Some((id, req)) = signer_proto::decode_request(&am.payload) {
                        out.push((id, matches!(req, signer_proto::Request::Held { .. })));
                    }
                }
            }
        }
        out
    }

    fn answer(id: u32, a: &signer_proto::Answer) -> Vec<u8> {
        let (_, key) = wire::delegate_from_code(SIGNER);
        bincode::serialize(&Ok::<HostResponse, ClientError>(HostResponse::DelegateResponse {
            key,
            values: vec![OutboundDelegateMsg::ApplicationMessage(ApplicationMessage::new(signer_proto::encode_answer(id, a)))],
        }))
        .expect("encodes")
    }

    fn ask(io: &mut PageIo, n: u64) {
        for i in 0..n {
            let mut c = [0u8; 32];
            c[..8].copy_from_slice(&(i + 1).to_be_bytes());
            io.server.page.answer(Answer::PutOk(c), Ms(1));
        }
        io.pump();
    }

    /// The reader asks `Held` through its OWN node's signer, and the answer lands on its page. THE CONTROL: a plain
    /// reader frames no signer request at all (its blocks are "not held" and fetched).
    #[test]
    fn a_reader_with_a_held_signer_asks_held_and_nothing_else() {
        let mut io = reader(true);
        io.server.page.answer(Answer::PutOk([9; 32]), Ms(1));
        io.pump();
        let asked = requests(&io.take_frames());
        assert_eq!(asked.len(), 1, "one Held request: {asked:?}");
        assert!(asked[0].1, "the reader framed a signer request that is not a Held");
        assert!(asked[0].0 >= (1 << 31) | (3 << 23), "the reader's id is not in its own range: {:#x}", asked[0].0);
        assert!(io.inbound(&answer(asked[0].0, &signer_proto::Answer::Held { present: vec![true] }), Ms(2)), "its own Held answer was not taken");
        assert!(io.held.is_empty(), "the answered batch is still mapped");
        let mut plain = reader(false);
        plain.server.page.answer(Answer::PutOk([9; 32]), Ms(1));
        plain.pump();
        assert!(requests(&plain.take_frames()).is_empty(), "THE CONTROL: a plain reader asked a signer");
    }

    /// ONE SOCKET, TWO PAGES, ONE SIGNER: the person's own page and a keeper's reader never share a `Held` id, and each
    /// takes only its own answer. A reader never takes a signing answer. THE CONTROL: each takes its own.
    #[test]
    fn each_page_on_a_socket_takes_only_its_own_signer_answers() {
        let (mut me, mut keeper) = (owner(), reader(true));
        ask(&mut me, 1);
        ask(&mut keeper, 1);
        let mine = requests(&me.take_frames());
        let theirs = requests(&keeper.take_frames());
        assert_eq!((mine.len(), theirs.len()), (1, 1));
        assert_ne!(mine[0].0, theirs[0].0, "the two pages asked under one id: an answer would reach both");
        let to_me = answer(mine[0].0, &signer_proto::Answer::Held { present: vec![true] });
        let to_keeper = answer(theirs[0].0, &signer_proto::Answer::Held { present: vec![false] });
        assert!(!keeper.inbound(&to_me, Ms(2)), "the reader took the owner page's Held answer");
        assert!(keeper.inbound(&to_keeper, Ms(2)), "THE CONTROL: the reader did not take its own");
        assert!(me.inbound(&to_me, Ms(2)), "THE CONTROL: the owner page did not take its own");
        let signed = answer(1, &signer_proto::Answer::AlreadySigned(vec![1; 96]));
        assert!(!keeper.inbound(&signed, Ms(3)), "a reader took a signing answer");
    }
}

/// AN AUDIT'S SITE READ IS THE PUBLISHER'S (sdk#493; rule 4, one read path): an audited site is read by the same GET,
/// never subscribed, PUT or signed, and its answer comes back as its label's `Head`.
#[cfg(test)]
mod site_audit {
    use super::*;
    fn page() -> PageIo {
        let (_, signer) = wire::delegate_from_code(b"site audit signer");
        PageIo::new(
            page::server::Server::new(page::Page::unstarted(engine::Params::default(), page::PutPath::Page, Ms(0)), page::server::SignerFacts::default()),
            Artefacts { block_code: b"b".to_vec(), register_code: b"r".to_vec(), register_params: wire::register_params(&[1u8; 32], wire::HEAD_NAME), signer },
        )
    }

    #[test]
    fn an_audited_sites_read_is_the_publishers_get_never_followed_put_or_signed() {
        let mut io = page();
        let id = [0x51; 32];
        // THE CONTROL: a site neither published nor audited is refused by name.
        assert!(io.site_read("kept").expect_err("refused").contains("not being published or audited"));
        io.audit_site("kept", id);
        assert_eq!(io.site_read("kept"), Ok((id, false)), "an audited site is not read by id, UNfollowed");
        assert!(io.site_put("kept").expect_err("refused").contains("only being audited"), "an audited site was PUT");
        // Its answer is this page's, as the label's head (an audited site is in the ONE list of sites).
        assert_eq!(io.site_by_id(&id).as_deref(), Some("kept"));
        io.end_site_audit("kept");
        assert!(io.site_read("kept").is_err(), "the audit ended and the site is still read");
    }
}

/// THE LINES SAID TO THE APP ARE BOUNDED with NO reader (sdk#482; the architect): page-io's list, driven through its
/// own inbound -- 10,000 frames it cannot read, and nothing ever reads the list -- holds the newest
/// `KEEP` and counts the rest; the one read drains it and says how many were dropped.
#[cfg(test)]
mod said_bounded {
    use super::*;

    #[test]
    fn ten_thousand_unusable_frames_with_no_reader_stay_bounded() {
        let (_, signer) = wire::delegate_from_code(b"said signer code");
        let mut io = PageIo::new(
            page::server::Server::new(page::Page::unstarted(engine::Params::default(), page::PutPath::Page, Ms(0)), page::server::SignerFacts::default()),
            Artefacts { block_code: b"said block code".to_vec(), register_code: b"said register code".to_vec(), register_params: wire::register_params(&[1u8; 32], wire::HEAD_NAME), signer },
        );
        // Frames the node could send that page-io cannot read (`Incoming::Unusable`): a writer's page owns them.
        for i in 0..10_000u32 {
            io.inbound(&[0xff, 0xfe, (i & 0xff) as u8], Ms(1 + u64::from(i)));
        }
        let kept = io.unusable().len();
        println!("10,000 unusable frames: {kept} kept");
        assert_eq!(kept, page::unusable::KEEP, "page-io's list is not capped at KEEP (or the answers never reached it)");
        let read = io.take_unusable();
        assert!(read[0].starts_with(&format!("{} earlier line(s) dropped", 10_000 - page::unusable::KEEP)), "{:?}", read.first());
        assert!(io.unusable().is_empty(), "the one read did not drain");
    }
}
