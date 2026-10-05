# Auth, identity and data — progress

Owner (2026-09-27): build auth, identity and data together, to the end design (ARCHITECTURE §4, in
~/dev/craftworks-archive/craftworks-docs). Auth supports: this device + PIN, import/export keys, keycraft. As any
website, auth is asked for only when a page or component needs it.
Owner (2026-09-27, later): email does not work. First login: "log in with this device" + set a PIN; later, the same
device + the same PIN opens the same account. Adding a second device: pairing with device id + PIN (below).

## Design (end state)
- **identity delegate** (on each node): holds THIS DEVICE's key; signs records in the Register's one signed format,
  only for its own key; never two different values at one seq; an app may sign only records named under its own
  site id (origin rule); local tools any.
- **Data**: each device writes its own tail(s) + prolly tree per app; an identity's data = the overlay of its devices.
- **DID** (did:craftec:<owner-seat id>): owner seat Register (owner keyset, identity entry) + member Set (admit /
  retire acts, scopes) + profile row. Recovery = the words (below).
- **Members, by device + PIN:** the DID is the ACCOUNT; a MEMBER is one device key admitted to it, opened on its
  device by a PIN. "Log in with this device" mints a member key (and, for a new person, the DID), the person sets a
  PIN; the identity delegate stores key + DID + PIN hash + home app as ONE record. One device holds many members
  (a shared computer): each PIN opens its own member, of the same DID or of different people's. No two members on a
  device share a PIN (a refused taken PIN counts as a guess). 5 wrong PINs in a row lock PIN unlock on the device;
  a member's key file sets its PIN anew. Each app unlocks its own session (random token in page memory). BUILT in
  identity/ (13 tests).
- **Auth** = ways to get a member into a DID: register with this device, log in with recovery words, device id + PIN
  pairing (a member on another device), import a key file, keycraft (pinned surface).
- **Device id + PIN (pairing, a second device):** device id = an admitted device's public key. On it, "add a device" shows a one-time
  PIN (its identity delegate stores it with a try counter). The new device encrypts {its own key, PIN} to that public
  key and writes it to the admitted device's inbox. Only that device's delegate can open it: it checks the PIN (a few
  tries, then the PIN is void) and admits the new key to the member Set and answers with the DID; the new device stores it with its own key and sets its
  own PIN. The PIN is never checkable offline, so a
  6-digit PIN is enough. No server, no email; the admitted device must be online.
- **Recovery words = the account** (owner 2026-09-27): registering makes BIP39 words (12; 24 accepted). The owner key
  is derived from them (SLIP-0010 ed25519, m/44'/25458'/0'), the DID is its seat, so the words alone name the account
  (no vault, nothing to guess offline, no DID to remember). A wallet's existing words work too (a different key at our
  own path). The identity delegate keeps the words' entropy with the member; the Account page shows them on request.
  Log in with words = this device becomes a new member of that DID, with its own PIN. BUILT (core/src/account.rs).
- **Auth dialog:** Login | Register tabs. Login: this device (PIN), or recovery words. Register: this device (PIN
  only; the words come with the account). Sessions are the app's on this device, kept by the delegate until logout
  (a reload stays logged in). BUILT.
- **Session members:** a public computer's member is admitted as class "session"; logout retires it in the member Set
  and deletes its key from that computer's identity delegate (needs a Forget request: TODO).
- Passkeys: impossible in freenet's app frame (measured 2026-09-27: opaque origin).

## Phases (each: build → private node → publish through B → commit)
- [x] A. identity delegate (members by PIN, sessions, sign with seq guard + origin rule) + page `identity` + `auth`
  dialog (asked only by pages that need it) + first login makes the DID (owner seat; the vault was replaced by recovery words the same day). Verified on a
  private node (create, wrong PIN counted, same PIN reopens, a 2nd person on the same device gets another DID);
  published through B as app site v5. Pages must connect to the host they were served from (localhost ≠ 127.0.0.1).
- [x] B0. member list on the network: a SET under the owner key (owner-only), one owner-signed item per member (key =
  member key, payload MB01 ‖ class ‖ device name). Register and words-login admit this device (PUT merges = union, so
  two devices joining at once never conflict; a Register list would have forked). The seat now names the owner key
  (`ST01 ‖ owner`, at seq 2 so the first seats' seq-1 records are replaced, not forked), so DID → seat (checked:
  that key's seat must be the DID) → owner → member Set. Account page lists the devices. Verified on the private
  node: register → 1 device; words-login → 2 devices, same DID. The Set crate links with default-features off (no
  contract imports in the page wasm).
- [x] B. data: each device's own feed per table (tail + tree, sealed per epoch), merged; subscribed (built through the
  storage/feed work; see "Two devices, one account").
- [-] C. pairing without the words — DROPPED (owner 09-30): words or account id + recovery passphrase.
- [x] D. auth dialog (Login: this node's PIN · words · passphrase; Register). KEY FILE DROPPED (owner 09-30): the
  identity's `Export` is refused (`Why::Retired`; the variant kept for the wire order), the page API removed. What the
  key file did — reopen a node LOCKED by wrong PINs — the words do now: a new member of an account that has one here,
  bringing its data key (checked against `ACCOUNT_CHECK ‖ DID`, a one-way check written at provisioning). Test + a
  planted bug turning two red.
- [ ] E. keycraft — LATER (owner 09-30).
- [-] F. device id + PIN pairing — DROPPED with C.
- [x] G. the sites publish under the node's signer (loader, craftworks, test/alt sites; B's).

## 2026-09-28 — reviewed against freenet's dapp-builder skill (github.com/freenet/freenet-agent-skills)
- DID = the owner's public key (`did:craftec:<base58>`), never a contract address (an address moves with every code
  release; "identity must not be a contract key"). The seat is gone; DID → member Set is derived.
- Recovery words are shown ONCE at registration and kept by no node (a node's disk can be copied; the words are the
  owner and cannot be rotated). Account page no longer shows them.
- The identity delegate: one gate before any request — only a web app the node attests is served (`None` and other
  delegates refused; "own tools" removed). Members keep the account's DATA key (m/44'/25458'/1'). `Handover {pin}`
  gives a member to the next delegate version (its home app, counted as an unlock try): delegate secrets stay with
  the delegate key and freenet never migrates them, so every future re-key needs this. The page-side walk over
  predecessor delegates is built when the first successor ships (nothing before this build can answer).
- Data is per ACCOUNT per app: one tail under the data key, label `site id ‖ app`, so any node of the account reads
  and writes it; Notes is its own package (/notes). Deltas go as `UpdateData::Delta` (wire `frame_update_delta`,
  sdk#576, pinned 615c9d5): a delta sent as a State is refused ("invalid put") — that was the lost second note.
  A refused write resets the local table and re-reads it.
- [x] B. data (tail per account per app, live) — verified on the private node: register, two notes, reload → both.
- [x] B2. flush the tail into the prolly tree (SDK `tail::flush_into`, freenet-prolly 17d67d8 = the SDK's pin). Tree
  blocks in Block contracts, PUT before the step naming the root; reads walk catalog → tail → root → blocks.
  FLUSH_AT = 32 rows. Verified private node: 33 notes → "flushed: 2 tree block(s), tail emptied (seq 33)";
  reload → "tree read notes: 1 block", 34 rows. B app v28.
- [x] B2b. erasure for the tree. Every node's children are coded by freenet-prolly (a short group stands alone), and
  every parity block a flush makes is PUT (test: the root lists parity, all of it put; control: a one-leaf tree lists
  none). The ROOT — the one block no parent covers, so a one-leaf tree's only block — gets its group of one
  (`engine::repair::root_parity`, 8 blocks), ids in the tail's own `\0root-parity` row beside the root, kept out of
  the tree. READ repairs: a block whose GET fails is rebuilt from its group (`engine::repair::find_group`/`rebuild`,
  verified against its id). Tests: a lost root of a one-leaf tree and a lost leaf of a 192-row tree are rebuilt, rows
  whole; controls repair nothing. Live (private node): a flush put 19 blocks (leaves, root, 8 child parity, 8 root
  parity), the node took the step, reload read 67 rows. NOT exercised live: a real missing block (no way to delete
  one from a node) — the repair path is proven in the core tests only. The TAIL itself is one mutable contract:
  freenet's hosting replicates it; no parity.
  RETENTION evidence (KEEPER §11(c) run 3, engineer1, archived KEEPER.md not updated): 10 blocks PUT 2026-09-26T23:24Z
  to a private node on Hetzner; present 10/10 at +1h, +6h, +24h; batched AskHeld 1 op 224/286/281 ms; one-per-op
  median/max 249/329, 240/388, 249/291 ms. Node dir 38M → 98M → 285M over the day (the node's own data, not the
  blocks); host disk 67–69%. So an unrepaired single copy survived a day on a node that stayed up; it says nothing
  about a node going away, which is what parity is for.

## 2026-09-28 — data belongs to the account; sites get the person's grant
- Owner: another developer's front end (or another address for the same app) must show the same data intact.
- Tables are the ACCOUNT's, by kind (`notes`, `pins`, …): a tail under the data key labelled `t/<table>`; no site
  in the label. Any site the person allows reads and writes the same table.
- GRANTS in the identity delegate, per member (person), per site, per table. The first time a site asks, the NODE
  prompts ("Delegate says: Allow this app to read and write your “notes” on this node?"), naming the asking site from
  its own records — a site cannot fake the answer. The member's home site is granted without a prompt. The Account
  page lists grants ("Apps with access") and removes them. A "no" stands for that page; reading needs no grant.
- Verified on the private node with three addresses of the app: a note written at the home address is read at a
  second address; the node's prompt named the second site; after Allow its note appeared at the home address live;
  Remove + "Don't allow" → its writes refused, no re-prompt.
- Open: tables are public (readable by anyone who knows the data key) until encryption; pins/labels as ONE relations
  layer with query() is next.

## Batch grants (2026-09-28)
- [x] A site asks ONCE for every kind of data its manifest `uses` (["notes","pins"]): one node prompt
  "Allow this app to read and write your “notes” and “pins”…"; only tables not yet held are named; a table outside
  `uses` is asked on its own; removal stays per table. Delegate `Grant{tables}` (1–16 names), `Granted{tables}`.
  Verified on the private node (craftworks-alt): one prompt naming both, "allowed: notes, pins", note PUT + pin PUT,
  no second prompt. Published through B (app v24, loader v8). The delegate re-keyed: members log in once with words.
- [x] Account CATALOG (table `tables`): pages read the catalog, fetch only listed tables, open unlisted ones empty
  with NO read and list them on first write; registration creates the catalog (session `fresh`); accounts from before
  the catalog read tables as before and list what they find. Delegate: catalog writable by home site or any site with
  some grant (re-key). Verified private node: new account → "tables: new", PUT; pin → pins PUT + listed; reload →
  tables 2 rows, pins 1 row. B app v26. The prolly tree joins each catalog entry when flush lands.
- [x] Handover walk: manifest `identity_prior` (from contracts/identity-history, appended by publish per new build);
  a PIN the current build does not know is asked of earlier builds newest first, the member provisioned here.
  Verified private node: PIN 556000 (made by 826bb69's build) → "member moved from AP3BuSjF…", its note back.
- [x] Catalog completeness: only a catalog made at registration is `complete`; a later one (older account) never
  hides unlisted tables; a table is listed BEFORE its first write.
- [x] Publish: a piece a peer refuses (peers with a full disk budget: 166 MiB / 128 MiB) is a missing piece, retried,
  judged by k-of-n — not fatal. B app v27, loader v9.
- [x] BLOCKS package: the one door for tree blocks (fetch, put), out of data.js. Fetch RACES (rule 11, sdk#303): a
  block and its whole group asked at once, first of {the block, any k of its group} wins, rebuilt blocks verified
  against their id; each block asked once. Live (private node): reads mixed "arrived first" and "from their group
  first (rebuilt, verified)" — the rebuild path now runs on real network blocks, not only in tests; 67 rows, a write
  after. B app v33.

## Rotatable identity (2026-09-28) — DID = hash of the inception event, keys rotate under it
Why: today DID = owner public key, derived from the words; new words = new DID and new table addresses. A DID must be
a stable "phone number" whose keys rotate (KERI-style self-certifying key event log).
- [x] I1. `contracts/idlog` contract (own workspace, freenet-contracts' reproducible build): params = DID; state = the
      event chain. Inception {K0, next=H(K1), data pub, enc pub (X25519), vault}; Rotation n {prev, K_n revealed ==
      committed, next=H(K_{n+1}), data, enc, vault, sig by K_n}. Valid = verifies from the DID. Merge = longer valid
      chain; same-position equivocation → lowest hash. Tests incl. controls (wrong key, broken commitment, forks).
- [x] I2. core: per-epoch keys from words (owner m/44'/25458'/0'/i', enc /2'/i'), data seed from the original words
      (/1'), vault = data seed sealed to the epoch's enc key; build inception / rotation; resolve a chain; whoami
      Register (key → DID) for words that were rotated in.
- [x] I3. page: register writes the log (PUT); words login resolves DID (whoami, else inception), opens the vault,
      provisions; members Set under the CURRENT owner key; Account: DID shown, "Change recovery words" (two
      rotations + whoami + re-admit this node).
- [x] I4. publish idlog.wasm (SHA256SUMS); verify private node (register, rotate words, log in with NEW words on a
      fresh member: same DID, same notes; OLD words refused); B; commit.
Out of scope now: recovery quorum (guardians), data-key rotation (moves tails), handles (@name → DID).
  Verified (private node, test site): register → DID CQGVSX… (id of the inception); a note; Account → "Change recovery
  words" (old words typed, new shown once) → "Done", same DID; log out; NEW words on a fresh member (new PIN) → same
  DID, the note there, Your nodes = 2 (the first re-admitted under the new owner key); OLD words → "these recovery
  words were replaced by newer ones". idlog.wasm ea00f4e8… (reproducible: two builds, same hash), node accepted it.
  Members from before the log: Account says "log in once with your recovery words (and a new PIN)"; that login puts
  the inception (same data key → same tables, same member Set). All three private sites + B (app v34) published.

## Messaging track (order agreed 2026-09-28): 1 seal own tables → 2 inbox per DID → 3 spaces (governance, membership,
## ordered space log) → 4 MLS + chat → 5 moderation/access → 6 discovery → 7 privacy hardening → 8 keep/health
- [x] 1. SEALED TABLES. Table key = derive(data seed, generation, table) inside the identity delegate; given (TableKey)
  only to the home site or a site granted that table (catalog: any grant) — so a grant now covers READ too. Rows:
  key sealed deterministically (same row → same bytes: updates/deletes find it), value XChaCha20-Poly1305 bound to
  its sealed key; both prefixed [1, generation]. Plaintext rows from before: read, sealed over 16 per step, plaintext
  deleted in the same step (no doubles); a write to an old row drops its plaintext too. Tests: stored state and tree
  blocks hold no plaintext (control: plaintext rows DO show), no key / wrong key read nothing, migration leaves no
  plaintext value. Live: old note sealed over, new note sealed, reload reads both; alt site DENIED → "no key here:
  nothing of it reads", 0 notes (the stale build there showed the rows as ciphertext garbage — proof they are sealed).
  NOT hidden yet (step 7): row count, sizes, table names in their addresses. Old TREE BLOCKS written in plaintext
  before sealing stay on the network (immutable); only what is written from now on is sealed.

## ARCHITECTURE (2026-09-28): docs/ARCHITECTURE.md is the shape — capabilities vs apps; 18 capabilities, 10 apps/pages/
## components. The messaging track above is superseded by its Implementation plan (phases 0–10); next: phase 0.
- [x] PHASE 0 (restructure): `data`→`storage`; `auth` split into `auth` (capability: check, accept, unlock, join, logout,
  nodes, changeWords) + `login` (component: the dialog, `session({tab})`); grants and table keys → `access`; `pins` +
  `tags` → `edge` (pins, labels, adoptPinnedField) + components `pin-button`, `label-menu`; `timeline`→`trace`; notes'
  legacy-pin migration → edge. Tables unchanged (pins, tags, notes). Verified private test site: Home pin, Notes pin +
  label (persist on reload), Account (DID, nodes, grants), logout → dialog → wrong PIN message → login. All sites + B.
- [x] PHASE 0b (theme): packages/theme.js tokens (light + dark), loader applies manifest.theme first; header, footer, home, login, pin-button, label-menu, notes on tokens (only the note palette, which is data, keeps its colours). Screens checked light + dark.
- [ ] PHASE 1 (keys and sealing). MLS library: mls-rs 0.56 (spike: builds for wasm32, 362–698 KB; OpenMLS needs a
  newer getrandom backend with no gain). mls-rs forces getrandom's JS backend and reads Date.now() through JS on wasm,
  so it cannot run inside a delegate without patched crypto deps → the protocol runs in the page's core; the
  delegate is the vault (state + epoch secrets; table keys for granted sites). SINCE 09-30 the crates are vendored and
  patched (no forced JS randomness, a host clock): MLS runs in the identity delegate too, for upkeep (admitting with
  no page open).
  - [x] 1a core `mls`: the account group — create; a node JOINS BY ITSELF with the words (external commit from the
    published group info; credential signed by an owner key of the key log, any owner the log ever had); remove a node;
    epoch secret → table key; save/load one blob. Tests: two nodes share one secret; a removed node's secret no longer
    matches; save/load; a stranger cannot join (control); after a words rotation old nodes stay and the new owner
    admits.
  - [x] 1b delegate as vault: MlsSave/MlsLoad (home only), epoch secrets kept per member, TableKeyAt(table, epoch)
    for granted sites (one derivation, `epoch_table_key`, shared with mls). 19 identity tests.
  - [x] 1c the account's MLS channel: account table `mls` (`info` + `c/<epoch>`; one writer sequence = one order;
    sealed with the words-derived key so a new node can read it). `keys` capability: created at registration, joined
    by itself on a words login (external commit), caught up on a PIN login. MLS SPLIT OUT of the core: crates
    `account` (shared) and `mls` (own wasm package, loaded by `keys` only); core wasm now wasm-opt'd: 717 KB (was
    1.36 MB), mls 695 KB. build.sh inlines wasm-bindgen snippets (mls-rs's Date.now). Live: register → created epoch 0;
    words login → joined epoch 1/2; PIN login → "2 commit(s) applied: epoch 2". FIXED: logout now reloads the app —
    per-page grant/table/group caches leaked into the next session (a new member's commit was refused NotGranted);
    only a REGISTRATION marks the account new (a words login used to, treating an existing catalog as absent).
    OPEN: a member that only ever logs in by PIN and never joined can't join without the words (Add by another member
    comes with spaces).
  - [x] 1d escrow: each epoch's secret sealed to the account's encryption PUBLIC key (key log head) as `e/<epoch>` in
    the mls channel — only the words open it (a removed node, holding no words, cannot). A node joining with the words
    recovers every earlier epoch into its delegate (EpochKeep, home only). Changing the words re-seals every escrow for
    the new words and escrows after that go to them; before applying commits a node reloads the group with the current
    key log (a join signed by the new owner is accepted). Tests: escrow opens with the words only (control: other
    words), after a rotation with the new words only. Live: register → join (1 epoch recovered) → change words
    (2 re-sealed) → NEW words join (2 recovered) → first node by PIN applies both joins.
  - [x] Account page surfaces the keys: Your nodes has Remove on every other node (an MLS removal committed through
    `ordering`; a raced removal reloads and asks again); Security (words changes from the key log, epoch + node count,
    epochs in escrow); Storage (per table: rows, sealed, tail/tree; blocks read/rebuilt this page — another app's table
    is listed by name, never opened, so no prompt). A REMOVED node learns it from the commit (mls `removed`, kept across
    saves), applies nothing after it, and the page says so. Live: register → words join → remove the first node (epoch 2,
    3 escrowed) → first node by PIN: "This node was removed…", same after a reload.
  - [x] 1e sealing: rows sealed with the account's newest EPOCH key (TableKeyAt; header `[2, epoch]`), the `mls`
    channel with the table's own key (`[1, 0]`); readers ask the identity for each epoch they meet (`tail-keys`). Whole
    tree blocks sealed (`seal_block`) in the new SEALED contract (contracts-src/sealed, first write wins, 81 KB) at
    `block_address(table key, cid)`; the tree holds rows in the clear inside (flush opens the tail's rows), marker row
    `\0sealed-tree`. No freenet-prolly change (the ARCHITECTURE "hook" replaced in place). A tree from before is read
    from Block contracts and rebuilt whole, sealed, by the next flush; rows sealed over UPWARD only. A node that knows
    it was removed writes nothing (it had still sealed a catalog row over). Tests: removal (no epoch 2 → old rows only,
    new tree unreadable; controls both ways), upward-only, tree-from-before rebuilt. Live: register → 33 notes → flush →
    reload reads the sealed tree; words-join 2nd member (epoch 0 via escrow) → remove the 1st → note at epoch 2 → 1st
    by PIN reads 32 old notes, not the new one, and cannot write; 2nd reads everything.
    OPEN: a removed node still holds the account's DATA key (tail signing) — writers per space in phase 2.
  - [x] Removed node FORGETS: the identity's `Forget` (home only) wipes the member — key, PIN, grants, MLS state, every
    epoch's secret — and `keys` calls it the moment the group says this node was removed, then logs out (storage
    refuses writes from then). Test: everything naming the member is gone, another member on the node untouched, a
    non-home site refused. Live: register A → words-join B → remove A → A by PIN: forgotten, its PIN opens nothing; B
    at epoch 2. FIXED: storage asking `keys` before a write deadlocked keys' own writes (no group for a new account)
    — now keys tells storage to refuse instead. Changing the recovery words is NOT part of it: the words never reach
    a node.
  - [x] `HandoverKeys`: an update moves a member's MLS state and epoch secrets with it (after `Handover`, on its PIN,
    home only). Builds before it answer unreadably: 0 moved (dev accounts only — owner OK'd losing keys this once).
  - [x] `ordering` capability (packages/ordering.js): one agreed order per object, several types behind one interface
    (`tail` built: the account's nodes share one key, the table's write sequence decides; `log` for spaces and
    `witnessed` later). keys' commits now go through it (append at the epoch it moved from; taken → join again).
    Live: register → join → PIN catch-up unchanged.
- [x] PHASE 0c (capabilities doing their own work): membership = the MLS roster (credentials name the node key;
  `membership.nodes()`); the member Set RETIRED (code, bindings, set.wasm, dependency) — one list of nodes; key log,
  which-account-words-hold and changing words moved from `auth` into `identity` (auth keeps sessions and the ways in);
  the Account page's grants through `access` (grants, revoke); storage's three sign-send loops → one `step()`.
  Core wasm 569 KB. Live: register → note → words join (Account: 2 nodes, this one marked) → change words → new-words
  node (2 epochs recovered) → first node by PIN (3 nodes) → note still there.

## PHASE 2 — writers and merge (Scuttlebutt feeds + Matrix causality + Nostr self-certifying membership)
Design in docs/ARCHITECTURE.md (decisions: writers/versions/membership; plan rows 2a–2c). Owner: implement over
capabilities, one source of truth, everything composes, no duplicate code, packages not the core.
- [x] 2a feeds and merge:
  - `feed` package (own crate + wasm, 72 KB): a row as a VERSION (id = writer + feed seq, `after` = the version it
    replaces, a delete is a version); MERGE = heads, longest chain then highest id; rows from before feeds are the
    oldest version; a version naming another writer is refused; cycles neither hang nor win. 5 tests + a core test of
    two real feeds through a sealed tree (edit, flush, delete).
  - identity: a member's own key signs its own feed of a table (grant rules unchanged); data key still signs the
    shared tails (channels, tables from before feeds) until 2c.
  - core: back to tails + sealing only (`tail_raw` rows as stored, `tail_next`); no feed rules in it.
  - storage: two layers — ONE TAIL (read/settle/keys/write/flush/seal-over, parametrised by its writer key) and a
    TABLE (the writers' feeds merged through `feed`). Existence never guessed: each writer's CATALOG feed lists its
    tables (listed before made); the shared DIRECTORY lists nodes with feeds (`node:<key>`) and tables from before
    feeds. `membership.writers()` says whose feeds count. Another writer's unreadable feed is skipped and counted
    (Storage shows it); only the own feed must open. Channels (`mls`) stay one shared tail.
  - keys: each commit row carries the group info after it (a joiner starts from the newest commit even when the node
    that committed died before publishing `info` — FOUND live: a words join committed, failed before publishing, and
    every later join looped "the group kept moving"; that dev account (HK8k) stays stuck: its only member lost its
    state across delegate builds).
  - Live: B writes → directory `node:`, catalog feed PUT, notes feed PUT, merged shows it. D registers + note, E by
    words: sees D's note, edits it (one note, E's version); D by PIN sees E's edit, deletes; E sees it gone.
  - Live (fresh account): F registers, G and H by words (3 nodes, epoch 2); H writes a note; F by PIN reads it.
- [x] 2b membership and removal:
  - the credential (format + check) moved from `mls` into `account` (one source; mls re-exports); `account::members`
    = credentials checked against the key log, minus removals by nodes still in, in epoch order (lower remover key
    first within an epoch). Test incl. a stranger's credential, a non-member's removal, mutual removal both orders.
  - identity: `members` table read with ANY grant (like the catalog), written by the home site only. Test.
  - membership: gossips in each node's own `members` feed (`n/<node>` credentials it knows, `x/<node>` its removals);
    `writers()` = BFS from this node + the directory over the UNION of members feeds (not a merge: a removal can't be
    overwritten), then the account's rule via the core. `keys` only reports status changes (`onChange`).
  - removal (membership.remove): group removal (keys) → `storage.adopt(node)` (its current rows rewritten in this
    node's feeds) → `x/<node>` told.
  - Live: P registers, Q and R by words; R reads P's and Q's notes (credentials gossiped); P removes R: "4 row(s)
    adopted", removal told; after a reload P's notes come from 1 feed and include R's note. Q removed earlier and,
    logging in by PIN, forgot itself (its PIN opens nothing).
  - OPEN: in the first removal run (two browser scripts overlapped) Q's note was NOT adopted; the page's trace was
    lost to the overlap — cause unknown; the clean rerun adopted correctly. Re-test on the next removal.
- [x] S space, the shape (owner: "no spaces capability? where is the shape defined?"): `space` defines what a space
  is (id, governance `{ kind: "key-log" }`, self, shared, tables named ONCE by the identity: CATALOG, MEMBERS, CHANNEL
  via `account_tables()`); the account is the first. storage/membership/keys take it; membership applies the rule of
  the space's governance. Spaces compose membership, roles, governance, access, administration, moderation, ordering,
  keys, storage, index — a group chat / Discord / Facebook group / subreddit is a space CONFIGURED. Live: P moved to a
  new delegate build carrying "the account's group and 4 epoch(s)" (HandoverKeys, first live), notes + account intact.
- [x] 2c ordering per epoch: a commit from epoch e goes in e's LOG (tail under `epoch_log_key(secret e)`, signed by
  the identity for the home site only, for an epoch whose secret it holds), carrying the info after it and the next
  epoch's secret in escrow; whoever makes an epoch makes its log (known new: no 30 s GET); a node catching up walks
  log → commit → next log, keeping EVERY epoch it passes; a words-joiner walks from the channel's pointer (`e/<L>` +
  `info`, after the commits from before epoch logs) opening each escrow with the words. The data key now signs only
  the channel's pointer and tables from before feeds. FIXED live: a joiner could not sign the log it joins at
  (EpochKeep now advances the newest mark; test); catch-up kept only the last epoch (now each). Live: S registers,
  T/U/V/W by words walk and join (epochs 1–5, earlier epochs recovered), S by PIN walks to epoch 5 with epoch 4's key;
  S removes U through epoch 2's log (adopted 2 rows, removal told), T follows to epoch 3; P (account from before
  epoch logs) unaffected. OPEN: S lacks epoch 1 (missed before the fix) until T's rows are sealed over upward.

## Messaging (owner 2026-09-28: Discord-style group chat first; then "we need private messaging first — the foundation")
- [x] C1 groups per space + a server that works for its maker: identity keeps MLS state/epochs per space (`space: None`
  = the account's, under its old keys; SPACES list; Forget clears all; tests); `mls` Rule { Account | Space } (a space
  admits nodes of any account by their owner-signed credential; `create_space` reuses the node's account credential;
  test); `keys.group(sp)` over one `logsOf` set shared with the account (space logs sealed with each epoch's own key,
  signed in the space); storage over a SCOPE (account | space: group members write, space tables sealed with its
  epochs, address key `identity::space_table_key`); `space.create/mine/tableOf/channel` (a channel = sub-space
  inheriting the server); `content` capability (authored item shape; author-only edits); Chat page (Discord layout).
  Live: server “Craft” made in 0.8 s (group, log, listed), #general, a message posted and there after a reload.
  Bugs fixed on the way: `tail`'s `space` option shadowed the capability; the account group passed an empty space id.
- [x] M private messaging (M1–M5 done) — `conversation` kinds: direct, group, mail (owner: broadcast/thread are NOT kinds —
  roles and relations compose them; reddit/facebook/twitter are social content, not messaging)
  - [x] M1 mls: key packages (secrets in the node's saved state; old states load), add → (commit, welcome), join a
    space from a welcome (space checked BEFORE joining: a wrong welcome cannot use up the key package). Test across
    two accounts.
  - [x] M2 directory: a person's public CARD (handle + a key package per node), a public tail under their account's data
    key, found from the DID alone (key log head: data + enc keys, `idlog_keys`); core public tails (in the clear, never
    flushed; test); own card listed in the directory before made. Account page "Your card". Live: "sam" published with
    1 node key, read back from the network. OPEN: someone else's missing card costs the 30 s GET.
  - [x] M3 inbox: `bag` contract (contracts-src/bag, 121 KB: unordered set at an address, admitted by 12 bits of work,
    union merge, over the cap the most work stays; tests); `identity::seal_to/open_with` = the ONE seal-to-public-key
    (escrow now uses it, same format); the account INBOX key from the data seed (every node has it, in the delegate);
    delegate `InboxKey`, `InboxOpen` (home only; test); core `inbox_address`, `seal_to`, `bag_add/bag_id/bag_payloads`;
    `index` capability (inbox: send to a DID, list mine); the card carries the inbox key and publishing makes the empty
    inbox (never a GET of a missing one).
  - [x] M4 `conversation` (kinds direct | group | mail; direct built): start = card → a two-person space, each of their
    nodes added by its key package, one welcome per node in their inbox, a "started" system item; accept = join from
    the welcome, list it, post "joined", renew this node's key package (one use each). Storage now opens a table on its
    own feed and merges other writers as they arrive (a writer not there yet is asked again every 30 s; `settled` for
    adopt) — no table waits on a person who has not written yet.
  - [x] M5 Chat page: "✉ Direct messages" (list, + New message by id), rooms for channels and conversations alike,
    handles from cards. Live: sam → pat: pat's node welcomed from the card; pat opens Chat, joins (epoch 1), sees
    "hi pat", replies "hi sam"; sam sees "joined" and "hi sam"; names shown as sam/pat.
  - [x] Apps separated (owner): Messages (✉, direct conversations, no member list) and Chat (servers only), both on the
    `room` component (one conversation's messages + composer); people shown as `handle#id` everywhere
    (`directory.shown`/`name`; owner: "user identification always show username#id"). Live: Messages lists
    pat#8r4orC, the conversation shows sam#EAsepm / pat#8r4orC, a message sent from it; Account "Published as sam#EAsepm".
  - OPEN: a space's group (a server, a conversation) is not carried by HandoverKeys: the next delegate build loses it
    (this build change lost S's server "Craft"). Next to fix, before more spaces are relied on.
- [x] C3 invites (codes, join request, a member adds: live S→code→P admitted by S's open Chat), C5 → REPLACED (owner 09-28): one member per DID in a space, its keys the account's (devices only sign in) (C4 roles/moderation: done, plan step 5)
- OPEN: a space's group state is not carried by HandoverKeys yet (the next delegate build loses space groups).
- OWNER ASKED (published size): manifest 54 KB lists every piece of every package; plan: one hash per package (pieces
  derived), split per page. Not done yet.
- [x] Mac-style top bar for every app (owner): the header draws the app's MENU from `ctx.actions[route]` — sub-page links
  (`{ label, href, on }`), actions, ending actions at the right (`end`: Log out); the loader routes a page by its longest
  route prefix and gives the rest as `ctx.sub` (`#/account/nodes`); loader v14. Account split into Card, Nodes,
  Security, Storage, Apps, Recovery (only the open one runs). Live: bar "⌂ Account Card Nodes Security Storage Apps
  Recovery … Log out", each sub-page alone.

## Plan (owner 2026-09-28 "continue with the fixes and implementation")
1. [x] spaces carried across delegate builds: `HandoverSpaces` (each space's state + epoch secrets; test); the page asks
   earlier builds through ONE helper (`askPrior`: handover, keys, spaces). LIVE CHECK PENDING: needs a later delegate
   build with this one as the prior (this build lost the spaces made before it, once more: "Craft", sam↔pat).
2. [x] every account gets a card at registration (and each node joining with the words adds itself); 4 key packages
   per node, one picked at random per conversation, a fresh set on accepting (older secrets kept). Live: U registered
   → card by itself; sam and pat each started a conversation with U while U was away; U opened Messages → joined both.
3. [x] manifest split: the root names each package by kind + sha256 only, and each page's needs; each package's entry
   (its pieces) is its own file of the site `p/<sha16>.json`, read only when needed, the page's asked at once; publish
   skips a package whose hash is already live. 54 KB → 8.2 KB. (Derived piece addresses were NOT possible: the loader
   fetches pieces over the node's web path, whose addresses hash their content.) Measured: needs are 39–40 of 40 per
   page — login pulls the whole keys/storage graph — so today's gain is the 7× smaller manifest and parallel entry
   reads, not fewer bytes per page. Loader v15. Live: messages, notes, chat, account/storage load.
4. [x] server invites by id (Chat's top bar: Invite; `conversation.invite` = the same `welcome` as a direct
   conversation). A space's HISTORY: each epoch log's `open` row carries the epoch before's secret, so a joiner walks
   back and keeps every earlier epoch (`logsOf.history`) — without it U joined "Crafters" and saw no channels (written
   at epoch 0, before U). Live: S made "Guild", posted, invited U; U kept 1 earlier epoch, saw #general + S's
   message, replied; S saw the reply. (Crafters, made before the fix, stays unreadable to U: dev data.)
   UI with it (owner): theme `loading(label)` placeholder — the loader's per component, rooms/channel lists/inbox
   check their own, "start of" only once every feed was tried (`content.settled`); "page ready" in the trace, not the
   footer; welcome page (Craftec · Craft The Future, one stop centre for everything Freenet; apps' `about` in the
   manifest); the app SHELL — the window never scrolls, header and footer fixed, body scrolls, full width;
   home's apps centred; footer one line. Loader v16.
   People by `name#abc123` (`conversation.person`): matched among the people this account knows (its conversations,
   its servers' members); a stranger needs the full id (a 6-character prefix locates no card). Invite and New message
   take either. Live: Google#H33CU9, #8r4orC, pat#8r4orC resolved; nobody#ZZZZZZ refused.
5. [x] roles and moderation (`roles`, `moderation` packages). A space's id is sha-256(owner, nonce): the owner is
   proven, not claimed (servers from before have none: nobody holds rights there — dev data). Acts (`grant`, `remove`,
   `hide`) in the space's `acts` table, replayed by (at, id), each counted only if its signer's role allowed it then;
   a row's signer is its feed's writer node → account (group credentials; removed nodes stay attributed). Owner: all;
   admin: invite, channels, hide, remove members; member: post, invite. Chat: roles in the member list, Make admin /
   member, Remove (their nodes out: one commit each), channel add/delete gated, a message's Delete / Remove. Content's
   author = the row's writer (never the claimed `by`). A removed node sees "You were removed". Live (Anvil): S owner,
   U + P invited; P (member) had no channel controls; S made U admin; U added #news, hid P's "spam", removed P, posted;
   S saw all of it; P saw the removal. Ordering by the signer's claimed time: witnessed ordering is the documented
   later step (ARCHITECTURE §5 Ordering).
6. [x] mail (`conversation.mail`, the Mail page /mail: Inbox · Sent · Compose; Reply). A mail lives in its SENDER's
   public tail `mail` (their account's data key: only they write it — who sent it is whose tail it is in, never what it
   says), sealed to each recipient's inbox key; a pointer {from, key} in each recipient's inbox; the recipient opens
   and keeps it in `mailbox` (private; the sender may later drop old rows). `directory.publicOf(did, name)`: any
   public tail of a person (the card is one). Live: P mailed S and U (S typed as sam#EAsepm); S read it and replied;
   P got the reply.
7. [x] Stale sites. MEASURED on the owner's node (7509, site B8Y3…): a GET answers from the node's copy; a GET that
   subscribes answered in 3 ms from the old copy and changed nothing; and after it, a real new version published via
   B (v68) had not reached the node 5 min later. So following does not keep a node current (why: unknown — needs
   per-hop traces from both nodes). Fixed by RELAY (`publish-craftworks relay <from> <to> <site>…`: the site's
   contract + signed state from B, PUT into the node — the contract checks the signature): 7509 now serves the
   current 9.6 KB manifest (all six pages) and its package entries. The page side: the loader follows both sites on
   every load and re-reads the node's manifest every minute and on focus; a newer one → "A newer version is ready ·
   Reload" (live: 39 s after a publish to the private node). "Get the newest" in the trace's Stack tab. Loader v20.
   OPEN: a node the network never updates stays stale until a relay — publish.sh could relay to the owner's node
   after each publish (owner's say).

## One member per DID in a space (owner 2026-09-28: "a DID is the member; a device only signs in")
- [x] P1 identity: `SpaceMember` — the DID's member key (derived from the data seed: the same on every device), its
      credential signed by the DATA key (checked against the DID's key log by anyone), and Sign allows that key (a
      space's feed is the DID's). Tests.
- [x] P2 mls: a member from (seed, credential); key packages and their secrets exportable (shared by the account's
      devices); create/join/load a space as the DID. Tests.
- [x] P3 pages: `keys.group` state + key packages in the account table `spacekeys` (any device loads the newest; the
      epoch log's order settles two devices); storage's space scope writes as the DID; `roles` maps writer → DID
      verified against the key log (closes: a credential's DID was never checked); card `kp` per DID; one welcome.
- [x] P4 live (Kiln, test v85, new identity delegate): U's card republished with the DID's key packages on load; S
      made Kiln ("made the space's group: epoch 0"), invited U (one welcome); U joined at epoch 1 as its DID, kept 1
      earlier epoch, read S's message, replied; S read the reply. Members: one per DID. (Multi-device not exercised:
      one node per test account; the shared state path is the one every device takes.)
- [x] P5 removing a device refreshes the DID's member in each space (fbc9c20: `keys.remove` → each space's `g.refresh()`).

## People and group conversations (owner 2026-09-28)
- [x] `edge.people`: follow · friend · asked · declined · hide · block (one table, one mechanism). Friend requests via the
      inbox (both asked, or accepted → friends both sides). Block: welcomes, mail, requests refused; hide/block: their
      items unseen (content filters).
- [x] roles: remove (back by invite/code, which clears it) vs ban (never, until unban); invites record `added`.
- [x] `person` menu from any name (rooms, Chat members): Message (reuses the DM; opens `#/messages/<id>`), friend,
      follow, hide, block; in a space by role: role selector (owner), Remove, Ban/Unban.
- [x] Messages: group conversations (`conversation.group`, kind "group"; several people in New message), friend requests
      at the top. Live: S → pat's menu, Message opened the DM; S made "Trio" with U and P (both joined, U posted);
      P accepted S's friend request; U hid S in Kiln (3 → 2 messages).
- [x] Contacts app (Home): the one `people-list` component (requests, friends, following, hidden, blocked; names open
      `person`) + Find by name#id/id. Messages stays conversations only (owner). Mail names open `person`.
      Friends: unfriend clears both sides (a notice); a request counts if newer than its last answer (`answered`, stamped
      at what it answers), a yes if newer than the ask; inbox items read in the order made. Live (test v93): S unfriended
      P, asked again (S: "requests you sent"), P accepted → friends on both sides.

## Managed account key (owner 2026-09-28: WAX-style; no always-on server)
- [x] Measured: a passkey for rp `localhost` (Edge → iCloud Keychain) has PRF, gives the same secret in Edge and Safari,
      and syncs to the owner's iPhone. `127.0.0.1` is not a relying party. The app iframe cannot use WebAuthn (no
      publickey-credentials-* grant; opaque origin). Email OTP/magic link needs an always-on service: not taken.
- [x] Upstream: freenet/freenet-core#5764 filed (shell-brokered passkeys, PRF bound to the contract). Implementation
      pushed to onlyabrak/freenet-core:feat/shell-passkey-5764 (bridge + host-owned bar + binding; unit test 17 checks,
      mutation-verified; Playwright: bridge reachable in all 3 engines, bound PRF in chromium; 59 passed locally, 2
      webkit failures identical on main). The repo auto-closes feature PRs without an approved issue: OPEN THE PR when
      #5764 is approved. Worktree: jobs tmp fc-passkey (delete after the PR).
- [ ] Our side: the account key sealed on the network, opened by words / passphrase now, by the passkey PRF when the
      shell supports it; PIN for daily use; passkey step-up for account acts.

## Two devices, one account (2026-09-29) — measured on a private 3-node network (A gateway, B, C; same binary)
- [x] Words login on a second device: the same account. Mail: one copy on each device. DM from Y: both devices join
      once, no duplicate.
- [x] FIXED: a tail sealed under an epoch this node had no key for yet (a device joined while the page was open) was
      never retried — `craftworks:keys` (identity, on each epoch kept) re-opens it. Notes B→A: missing → live in 3 s.
- [x] FIXED: a space's feed was the DID's, written by both devices → lost writes both ways (measured). Now each
      device writes its own feed; the card lists the devices' credentials (`directory.devices`, key-log checked);
      space writers = every member DID's devices; roles map device → DID. Live: B→A and A→B immediate, Y saw both
      (one after its 30 s re-read), both attributed to X. Messages written under the old DID-wide feed (test
      servers Kiln, Anvil) are no longer read: dev data.
- [x] The identity's `SpaceMember.writer` (DID-wide feed key) dropped (901e47f).
- [x] A space table open before a member's device first writes: now re-gathered on the member's card change (item 4).

## Queue 1–4 (owner 2026-09-29: "Proceed with 1-4")
- [x] 1a unread + notifications (`activity`): read marks in the account table `reads`; counts on Messages, Chat
      channels + rail, Home icons (the manifest's `counts`); a shell notification for a new message not on screen
      (tag = its route; a click opens it). `conversation.channels(server)`: one home for a server's channels (Chat
      uses it). Live (two-node private net): Home badge 1→2, notification in 3 s with the DM's route, list badge 2→0
      on opening, Home cleared.
- [x] 1b replies (`re`, a quote), reactions (their own rows keyed item+emoji+author, gathered onto the item),
      @mentions (suggested while typing; a message mentioning you stands out). Live X↔Y: quote, 👍 chip, mention highlighted.
- [x] 1c edit (inline, "(edited)"); the newest 60 shown, earlier on asking. FIXED on the way: an end-aligned grid
      made the top of a long room unreachable (flex column + margin-top:auto). Live: 60 of 71 → 71, from the top.
- [x] 2a recovery by passphrase (`recovery`; `account::passphrase_seal`: Argon2id 64 MiB ×3, XChaCha20-Poly1305, DID
      as associated data) in the account's public tail `recovery`; set in Account → Security (the words once); login
      "Use a passphrase" (id + passphrase → the words' entropy → auth.join). New words drop the copy. Passkey: a second
      copy sealed with its PRF once #5764 lands. Live: set 1.2 s; wrong passphrase refused; new device B logged in as
      Z in 2 s and read Z's note.
- [x] 2b removing a device refreshes the DID's member in every space: after an account `remove`, the open tails move
      to the newest epoch (`storage.sealNewest`), the card is republished, and every space's group commits an MLS
      update (`Account::update`, test `a_members_update_leaves_a_stale_copy_behind`). Space writes catch up with a fresh
      epoch-log read and `sealNewest` before sealing. Live (private net A/B/C): Y's DM after A removed B → A sees it,
      B does not; Y sealed with epoch 2.
      Recovery copy moved from its own public tail to the card row `recovery`. (Rows missing from the separate `recovery`
      public tail on other nodes: measured 09-29, not reproduced — see Housekeeping.)
- [x] 3 social content (`posts`; page BOARD since owner 09-29 "Reddit style", below; "Posts" in the person menu): posts,
      comments and votes are `content` in the AUTHOR's public tail `posts` (content's public container: the tail's owner
      is the author); a vote = reaction ▲/▼ with `item` = the post's ref `<did>/<id>`; a comment/vote on another's post
      drops `{from}` in the post's public pointer bag (`index.point`, made at post time), resolved by the reader. Feed =
      own + followed tails. Live (private net): post 0.8 s; Y followed → feed at once; Y's vote + comment via UI; A (not
      following Y) found both through the bag (score 1, 1 comment).
- [x] BOARD (owner 2026-09-29: Social renamed Board, Reddit-style). Posts have a title and a BOARD (`b/<name>`, content
      `in`); submitting drops `{from}` in the board's pointer bag (`board:<name>`), so a board lists posts of people
      the reader never followed. Home = follows + own + joined boards (joined = pins `board:<name>`). Sort Hot (Reddit's
      log-score + time) / New / Top. Post page `#/board/p/<ref>`: comment TREE (a comment's `re` = parent, `in` = post),
      votes on comments, fold, reply; side panel = the post's board (Join, Create post). Live (17573, S and P): S
      submitted to a new board (0.8 s); P found it through the board's bag (1.3 s), joined, voted, commented, replied
      nested; S saw the tree and score 1.
- [x] SPACE = MESSAGES + BOARD + NOTES (owner 2026-09-29: "they are literally the same spaces"; "space have multiple
      capabilities composed in it"; "each account has personal space"). The ownerless b/name boards are gone: a BOARD is
      a space's (`space.board(sp)`, table `board` in the space: its members write, its roles and moderation apply, a
      moderator's Remove hides). Every space has one — server, group, direct. A space's NOTES: its table `notes`
      (`#/notes/s/<id>`; pins and labels stay the person's, ref `notes:<space>/<key>`). The PERSONAL space (the account):
      Messages = your conversations, Board = your profile (public tail `posts`, what followers see), Notes = your notes.
      SPACE APPS (owner: "it is apps for that space, like Teams"): `space-apps` component (shared) — ONE dropdown in the
      top bar of every page on a space, `<space> · <app> ▾` (owner: switching is a suite's move, kept out of the way),
      listing the space's apps and, for owner/admins, "Add or remove apps…" (a dialog). Which apps a space has is
      governance: an `app` act ({ app, on }) in the space's acts, counted if the signer may `apps` (owner, admin);
      `roles.apps()` = Messages + those on (a space nobody changed has Board and Notes). A space without Board is left
      off Board Home; without Notes, its notes page says so. Header: `menu` action kind (links, and actions as buttons —
      a `#` link is the loader's). Live (17573): S (owner) removed Notes → S's and P's (member, no manage entry) menus
      showed Messages · Board; the notes page said it has none; S added it back. New board on Board = a new server (with #general, so Chat shows it). Live (17573): S made a board, posted,
      invited; P joined by code on Board, commented; S (owner) removed it; Chat shows the same server; P's note in the
      server's notes seen by S, not in P's own notes; every top bar shows the three places; panel 2 members.
      OPEN: an old S↔P direct conversation (from before per-device feeds) opens with no feeds on P — old test data.
- [x] 4 upkeep. NEW WRITERS: an open table's writers were fixed at open and `directory.devices` cached per page, so a
      member's new device (or a new member) was never seen without a reload. Now `directory.onDevices(did)` (the
      member's card followed) and the group moving (`craftworks:keys`) re-gather writers into every open table; the
      account scope follows its directory. An absent catalog is polled ONCE per catalog (5 s, 10 s, 20 s, then 30 s).
      Live: V's new device (passphrase) wrote in the DM; Y's open page had it without a reload.
      PRUNE: key-package batch ids carry their time; a batch retired (the next one made) over a week ago is dropped
      (live: 10-day-retired dropped, just-retired kept). Mail ids carry their time; the sender's sealed public copies
      older than 30 days are dropped after each send (live: 40-day row dropped, recent kept).

## Spaces rail (owner 2026-09-29, agreed shape; ARCHITECTURE §1)
- [x] R1 rail: Personal (default) · Discover · shared spaces · create/join; the manifest declares each app's views
      (personal / shared / public) and every Home composes from them; each space's Home lists its apps; shared-space
      settings (members, roles, invites, bans, apps) on its Home; routes `#/s/<id>/<app>`; folds on phones
- [x] R2 apps scoped to the space selected: Chat (channels only; its server rail gone), Board (CONFINED: a shared
      space's board = its own posts only; personal = your own posts only, your profile — no aggregate feed, no "Your
      boards"; following keeps a person in Contacts, their profile one click away), Notes; the space-apps dropdown and
      cross-app links gone
- [x] R3 Messages personal only: direct and group conversations without roles, moderation or owner; no Board or
      Notes on conversations
      DONE 09-29 (R1–R3 together: the rail made Chat's own rail and the dropdown throwaway). Loader v21: side slot
      (layout `side: ["rail"]`, a grid beside the body), `/s/<id>/…` sets `ctx.space` (`/s/<id>` = page `/space`).
      `rail` (Personal, shared spaces with unread, + make/join), `space-home` (apps: add/remove by owner/admin — Chat
      added makes #general —, members, Invite, Settings), header APP SWITCHER (owner: "the dropdown change of app on the
      header menu": a shared space's Home + the apps it uses; Personal: Home + personal-view apps). A NEW space has no
      apps (owner: "most basic home and settings, without chat even"): `roles.apps()` counts only `app` acts that turned
      one on — old test spaces need their apps re-added. Manifest `views` per app; personal Home lists personal-view apps.
      Board confined (`#/board` = your posts, `#/s/<id>/board` = the space's). Conversations: no moderation, no role menu
      (content `governed` only for servers). `space-apps` deleted. Live (17573, S): rail S A C F G K M2 M9 +; + made
      "Studio" with no apps; Add Chat → #general; Add Board; switcher Home · Chat · Board; Studio post on its board only;
      Makers switcher Home·Notes → Home·Notes·Chat·Board after adding; DM with P: no Remove on P's messages.
- [x] R2b Board's personal view gains FEED (owner: "app composes that in personal space view"): boards of every
      space you are in + profiles you follow; a shared view never crosses spaces, the personal view may (it is you).
      Done: `#/board/feed` (top bar Feed · Your posts · Create post); a space's post opens in its space (the rail follows).
      Live: Feed listed Studio's, Makers' and profile posts; a Studio post opened in Studio.
- [x] R4 per-app settings: Chat (channels, who may post), Board (rules, who may post), Notes (who may edit)
      A `config` act ({ app, key, value }) in the space's log, by who may `apps` (owner, admin); `roles.config()` and
      `roles.allows(app, did, key, at)` — AS IT WAS when the item was made (a change never hides what came before).
      Readers enforce: content leaves out a message/post by someone not allowed then (Chat's channels, Board's
      posts; comments and votes stay open to members); Notes leaves out such notes. Composers say so ("Only admins
      post here"; Notes' composer hidden). `app-settings` component (shared): a dialog of fields; Chat adds its
      channels (add, rename, delete) — the space's Settings lost its Channels tab. FIXED on the way: the header asked
      `login.session()` (opens the login dialog) — two stacked login dialogs when logged out; now `auth.check()`.
      Notes checked the space's apps before its log was read (race) — now awaits it. Live (17573): S set all three to
      admins, rules, added #news; P: chat composer "Only admins post here", board rules + no Create post, notes
      composer hidden, P's earlier note/post still shown; reverted.
- (after R2) Feed: a personal-space lens over the boards of the spaces you are in (Reddit's Home).
- [x] P1 PUBLIC READ per app (Board first): Board settings "Who may read" (owner only: `config` read, counted only
      for the owner). Public posts go to the space's `pub-board` table — written in the clear (storage: a space table
      named `pub-…` is unsealed, still signed in the space: only members write); members read both boards as one room
      (`posts.boardRoom`: a post goes where the board reads now; a comment/vote beside its post), so earlier posts stay
      members-only. The space's governance goes public with it: acts in `pub-acts` (read by roles with the sealed ones,
      one per key); `r.publish()` (owner) copies the counted acts and a `member` act per member (the ROSTER). From
      OUTSIDE (`roles.ofPublic(desc)`): writers found from the owner (the id proves them) + the roster, each DID's
      devices from its card, their `pub-acts` and `pub-board` tails read by address (`storage.readOnly`: no catalog, no
      keys); content's `outside` container; outsiders cannot post or vote. Live (17573): Studio public → P (not a
      member) read "Public hello" in 2 s, not the members-only posts, post refused; Makers public → P (member) posted
      public → an outside view counted P as member and listed P's post.
- [x] P2 DISCOVER: 🧭 second on the rail (`#/discover`, loader v22: `ctx.space` "discover"); its Home (space-home)
      lists the apps with a public view and the public spaces; header switcher: Discover · Home + public-view apps.
      Public spaces list themselves (`index.listSpace`: one public bag "discover:spaces", the space's description) when
      the owner makes the board public; readers keep one per id, each proved by its id (`space.owner`). Board's public
      view: `#/discover/board` (every public board, Hot/New/Top), `…/b/<id>` (one), `…/p/<ref>` (read-only: members
      comment and vote). Live (17573): Studio + Makers listed; P saw both in Discover, the global feed with both public
      posts, a post read-only.
- [x] P3 OPEN JOIN: the space's own setting `config` space/join ("invite" | "open"; owner, admins — on its Home). An
      open space's join requests go in the bag its id names (no code); a member who may invite welcomes them
      (`conversation.admit`), recorded `admitted` code "open" — counted only while the space was open. Discover shows
      Join on an open public space (read from its public acts), "Open in your space" once in. FIXED: the replay
      crashed on an open admission (no invite to count it on) — P's roles, so its board, failed to load. Live (17573):
      S opened Studio; P joined from Discover; S's Studio Home admitted P; P: Studio on the rail, 2 members, its
      public + members-only posts, Create post.
- [x] P4 MODERATION LISTS (Discover's only moderation: nobody owns it). A person's list = their public tail `modlist`
      (only their account writes it), entries `person:<did>` | `post:<ref>` | `space:<id>`. Each reader applies their
      own and those of whom they chose (edge relation `modlist`: person menu "Use their moderation list"). Discover's
      Board and public-space list leave out what any applied list flags; "Flag post" / "Flag author" on Discover posts;
      Discover's Home shows your list's size and whose lists you apply. Live (17573): P flagged S's public post → gone
      for P; S applied P's list → gone for S too; S un-applied.
- [x] Discover's Home = app tiles only, like every Home (owner: "home for discover is aggregate for all apps"). The
      public-spaces directory is Board's public view (side panel, Join); moderation lists are the person's settings:
      Account → Moderation (your entries with Remove, the lists you apply with Stop applying).
- [x] PERSONAL BOARD AUDIENCE PER POST (owner: "perhaps that is per post instead"): Create post on your profile asks
      "Who sees it": 🌐 Everyone (the public tail `posts`) or 🔒 Only you (sealed account table `journal`); your
      profile room is both as one (`posts.profileRoom`), a comment/vote on a private post stays in the journal (no
      pointer). Live (17573): P posted one of each + a private comment; P's board showed both (🔒 marked); P's public
      tail held neither the private post nor its comment; S saw only the public one.
- [x] ACCESS AS INHERITED POLICIES (owner: "like RLS in access control, a setting that is inherited"). `policy` acts
      { path, action, who } in the space's log (by who may `apps`; `read` owner-only), actions read/post/comment/vote/
      edit/join, who anyone/members/admins/owner/nobody/inherit; the effective policy walks the path (`chat/<channel>`
      → `chat` → space "") to the default (members); time-aware. `roles.allows(action, did, path, at)` is the one check:
      content (a message/post → post, comment → comment, reaction → vote), notes (edit), board (read/post/comment/vote),
      joining (join: anyone = open). Old `config` settings replay as policies — their defaults as INHERIT (a first cut
      made them explicit "members" overrides that beat a space-level policy: found by the test). One dialog
      (`app-settings` = Permissions): space Home → Permissions (join, post, comment, vote, edit); Chat settings (post +
      each channel's own); Board settings (read, post, comment, vote, rules); Notes settings (edit). Live (17573):
      Makers space post=admins → Chat and Board inherit; #news override members → P posts in #news, not #general
      ("Only admins post here"), not on the board, still comments; reverted.
- [x] CHAT'S PUBLIC VIEW = a directory (owner: chat public/discover before contacts; content stays members'): an OPEN
      space (join: anyone) is public — its acts published (who is in, how to join) and listed in Discover; `#/discover/
      chat` lists open spaces with Chat (name#id, members, apps; Join / Open). Board's Discover list = public boards
      only. Live (17573): Studio + Makers open → both in the directory; Board's list the public boards.
- [x] Housekeeping: PUBLIC-TAIL ROW LOSS measured (private 2-node net, X on A writes, Y on B reads): a new tail's
      3 rows seen by Y at once; a later row pushed within 8 s; a card row read; and a tail Y opened BEFORE it existed
      got X's rows by push in 3–9 s. NOT reproduced (the earlier recovery-tail loss was on older code); mail's public
      tail uses the same path. Found + fixed: a tail opened absent stayed `absent` after rows were pushed (took()).
- Housekeeping: dropping `SpaceMember.writer` = an identity.wasm change (NOT the frozen signer): each device re-logs
  via the handover from the prior build (HandoverSpaces never live-checked) — bundle it with the next real identity
  delegate change (owner told 09-29).
- [x] UPKEEP on every page (`upkeep`, started by the header): welcomes accepted + askers admitted every 30 s from any
      page (was only Chat / a space's Home — a joiner on Discover never finished joining). Live: newbie joined Makers
      from Discover; S on Home only let them in; Makers on newbie's rail 3 s later, directory "Open".
- [x] NO PAGE OPEN AT ALL (owner "go ahead" 09-29) — built, live on private 0.2.139 nodes, and PUBLISHED 09-30: owner
      said upgrade both → owner's node (launchd ec.craft.freenet-node, ~/.local/bin/freenet; old kept as
      freenet-0.2.138.bak) and B (freenet-blob.service, /usr/local/bin/freenet; old kept as freenet-0.2.138.bak) on
      0.2.139, each restarted once; 17573 on 0.2.139 with its own webapp cache. Published: 17573 craftworks-test v151,
      craftworks v116, craftworks-alt v86; B site v112 (178/180 pieces). Owner's node serves the new manifest. Each
      person answers the node's "run in the background" card once, and logs in once (the delegate's handover).
      History: step 1 DONE (901e47f) — identity on freenet-stdlib 0.12.1, manifest
      NodeStarted + Background + wakeup `upkeep` 60 s, SpaceMember.writer dropped; handover live across 2 builds incl.
      spaces (first live check of HandoverSpaces). Node 0.2.138 shows the Background consent card on first run ("run
      when installed / each start, tab closed") — so NOT published to the owner's sites until upkeep does real work.
      Slice 2a DONE: the delegate on WakeupFired subscribes + GETs the inbox; live on a v0.2.139 private node
      (binary in jobs tmp fn139, node net139 :17681): 1 → 4 wake-ups in 3 min with the app tab CLOSED, inbox read.
      NOTE: the consent card must be answered (no answer = nothing stored, asked again at next registration).
      [superseded by the replan below] 2b: accept welcomes in the delegate — open inbox items (key is here), MLS join (openmls into the delegate),
      write the account's `spaces` + `spacekeys` rows (tail prepare/sign/PUT from the delegate: GET/PUT/UPDATE are
      available to delegates). 2c: admit askers. Then publish (owner's node + B need ≥ 0.2.139).
      PORT P1 DONE: MLS builds for the delegate with NO browser imports — vendor/ patched mls-rs 0.56.0 (no forced
      getrandom "js", no wasm-bindgen dep) + mls-rs-core 0.27.0 (`time::set_clock`: the host's clock; the page registers
      Date.now in `mls::client`). Probe (create_group, custom getrandom): 415 KB, zero imports. mls tests 5/5, build.sh
      import gate ok. Next P2: delegate randomness (seed from the page + ratchet) and clock (page-handed time /
      wake-up count). Then P3 table reads, P4 table writes, P5 accept, P6 admit, P7 coexistence, P8 live + publish.
      PORT P2 DONE: `UpkeepWatch { inbox, seed, now }` — the page stirs upkeep's randomness POOL (ratcheted per draw,
      next pool stored before the draw is returned; nothing drawn before a stir) and sets its CLOCK (page time + 60 s
      per wake-up since). Live on net139: clock 1790695519 at hand-over → 1790695699 after 3 wake-ups vs real
      1790695687 (+12 s: wake-ups fire a little early; fine for MLS lifetimes). getrandom's custom hook (identity
      delegate build only, feature freenet-main-delegate) comes with MLS in P5.
      REPLANNED 09-29 (value first): ADMIT is the half that stalls (an asker waits for an inviter's page); ACCEPT is the
      joiner's own page, and the joiner is the one waiting with it open. So the delegate does ADMIT first. Admit needs:
      governance replay, the space's acts feeds read, the requests bag, the asker's card, MLS add + commit, the welcome
      sealed to the asker's inbox, an `admitted` act written. Each piece moves into Rust ONCE, used by page and delegate.
      PORT P3 DONE: GOVERNANCE in Rust — crate `gov` (the replay, policies, invites; pure, `now` from the caller), the
      page's roles.js calls it through the core (`Governance`); its own replay deleted. Unit tests 11 (a planted ban bug
      turns one red). DIFFERENTIAL vs the former JS replay (verbatim from HEAD, run twice = its settled answer): 8 seeds
      × 4000 random logs, ~400k rows, ~54k counted acts, ZERO differing fields. Two deliberate changes found by it: (1) a
      removal's named nodes are learned in one pass (JS: by row order until its next replay — same settled answer);
      (2) grant/remove/ban naming nobody no longer count (JS stored `undefined`; the page never writes them). Live on
      net139 (0.2.139): new space, app, open join, board post admins-only, invite code: every answer right, Home and
      Permissions dialog right, no console errors. Harness: jobs tmp gov-diff.py + gov-old.cjs + gov example `replay`.
      NOT on the owner's sites: publishing ships the new identity delegate (consent card) — held until the port lands.
      NEXT P4: table READS in Rust for the delegate (a space's acts: feeds of each writer, epoch keys, merge) — core
      `data::Open` + `feed::merge` driven by delegate messages. [SUPERSEDED: see P4 below — no table reads in the
      delegate: the page hands it a MANDATE per space.]
      P4 (design, 09-30): the delegate ADMITS from a MANDATE the page hands it each tick (per space: open?, live codes
      with uses left, bans, members, MLS state, name/kind/governance) — no space-table reads in the delegate. Its own
      admissions are kept in its secrets (dedupe, uses) and written as `admitted` acts by the next page. Staleness
      accepted: a ban by another admin while every page is closed can still be admitted until a page runs; a removal
      fixes it. Delegate I/O per admission: GET the requests bag; GET the asker's key log (→ data key) and card (key
      package, inbox) — node 0.2.139 fetches unseen contracts for a delegate (#5542, checked in v0.2.139 source); MLS
      add; UPDATE the epoch log (commit), PUT the next epoch's log ("open"); seal the welcome to the asker's inbox.
      P4a DONE: `data` is its own crate (core re-exports it) and free of freenet-stdlib: the SDK moved the pure ids
      (register_params, block_state/_of_state) into contract-keys (sdk PR #577, merged f079914; wire re-exports; its
      one-writer control moved with it); craftworks pinned to f079914. Core builds the stdlib container when it sends.
      The identity delegate now links data + mls-rs + the getrandom hook (upkeep's pool) and passes the import gate.
      Live on net139: new space (PUT), acts + a note (UPDATEs), read back after reload, no console errors.
      P4b-P7 DONE (09-30) — ADMIT WITH NO PAGE OPEN, LIVE: the delegate's entry is its own crate `delegate/` (built as
      identity.wasm, 1.17 MB, imports only the node's 3 secret functions); identity a library; mls page bindings behind
      `page`, account's PUT builders behind `puts`, bag/idlog readers without stdlib (both contracts rebuild
      byte-identical). identity holds upkeep's records (codes, mandate, admissions, member, epoch helpers,
      `invite_address`/`inbox_address` — ONE home; index.js/core use them; Node-SHA256 known answer pinned). The engine
      `delegate/src/upkeep.rs`: a round per wake-up when no page handed the mandate over for 2 wake-ups — request bags →
      key log → card → MLS add → commit into epoch e's log (taken: stop) → next epoch's log → welcome sealed to their
      inbox → recorded. Tests: 3 (a whole round against the real pieces: the asker's own member JOINS from the welcome;
      banned/page-present/expired; commit position taken) — two planted bugs each turn one red. Page: upkeep absorbs
      first (adopts the delegate's newer group, writes `admitted` acts at their time, acks), then admits, then hands the
      mandate over; a mandate older than the delegate's group is refused (stale) — no epoch rolled back.
      LIVE (two 0.2.139 private nodes, A gateway net139 :17681 + B net139b :17682): A's page CLOSED; a new person on B
      asked to join open space Gov 323 → joined after 42 s. A's delegate said "admitted (epoch 1)" (wake-up 5); A's page
      reopened: group epoch 1 adopted, ONE `admitted` act (code open, the delegate's time), members owner + new member
      on both nodes.
      KNOWN LIMITS: [per-member since 09-30, see below] a ban made while every inviter page is closed is not seen until a page hands a new mandate over;
      ACCEPT on the joiner's side stays page-side (the joiner is the one with a page open).
      BANS HOLD (09-30): moderation.enforce() on every upkeep tick of someone who may remove — anyone banned still in the
      group (admitted late by the delegate) taken out. Live: ban act alone → next tick: epoch 1→2, 2→1 members.
      JOIN STATUS (09-30, owner: "left as Join and that is confusing"): component `join-button` (Join → Requested ✓,
      when, what happens next → Open), used by Board's and Chat's Discover views; requests kept in the account's table
      `asks` (every device; gone once in); the rail's code form lists requests by code still waiting. Live on B:
      Requested survived a reload, turned Open by itself when A's delegate admitted (A's page closed).
      CONSOLE 404s FIXED (09-30): pieces 404'd because my test nodes shared the per-user webapp cache
      (~/Library/Caches/The-Freenet-Project-Inc.freenet/webapp_cache, 64 MiB LRU, per-process locks): cross-process
      sweeps left `.hash` markers with missing/empty dirs → the node never re-unpacks → 404 forever. Test nodes now run
      with FREENET_WEBAPP_CACHE_DIR per node; 6 broken shared entries repaired (marker removed → re-unpack). 544/544
      pieces served, no console errors. 17573 still on the shared cache (owner uses it: restart asked). Upstream: the
      node trusts the marker without checking the dir (report only with the owner's say + traces).
      PER MEMBER (09-30): a household shares a node — mandate, admissions, moved groups, stale, said and the page tick are
      each member's (the session's); a wake-up runs the first member whose page is away. Tests 4 (+ shared node: both
      pages open → nothing; one away → only theirs; a planted "ignore the tick" bug turns two red). Welcomes name the
      request they answer (`code`); accepting one clears that request only (older welcomes: every code request). Live:
      B asked by a one-use code with A's page closed — listed as waiting across a reload, joined, the welcome named
      the code, the waiting line gone.
      NEXT: publish — needs the owner's node and B on ≥ 0.2.139 (the owner's call) and the Background grant on each.
- [x] HOMES ALIKE: `app-icons` (one tile: icon, name, pill, pin or "+ Add") for the personal Home, a space's Home and
      Discover's; a space's Chat pill = its unread. Live: newbie saw Chat "1" on Makers' Home, matching the rail.
- [x] Discover's Contacts view: people who chose to be shown (339afd9). NO public Chat (owner 09-29: Discord has none; its
  Discovery is a directory — Discover's Home lists public spaces with Join). A Discord-style preview channel would be a
  Chat setting later if wanted.
- (parked, owner 09-29) Blog: a lens over profile posts.
- (later, owner 09-29) TENANTS: a tenant = a top-level space holding spaces (sub-spaces inherit its members and admins;
  policies: who creates spaces, apps allowed, guests). Personal tenant = today's personal space; business tenants
  onboard companies. Rail gets a tenant switcher. Open: company-issued (managed) identities vs own accounts; what a
  leaver's content leaves behind. R1 builds the rail tenant-shaped (Personal first).

## Files and media (owner 2026-09-30: next after joining; then Reads, then Lifecycle) — ARCHITECTURE §6
Design (owner 09-30: RLNC, not RS): encrypt-then-code; 256 KiB chunks; generations of 16, 24 fragments (16
systematic + 8 coded, GF(2^8), coefficients in each fragment); sealed contracts at key-derived addresses; an index of
fragment hashes checked against the reference `{ key, index hash, size, name, type }`. Homomorphic checks (Pedersen)
come with keepers as a new codec version.
- [x] F1 core: crate `files` (pure, builds for wasm): adaptive chunks (size/16 in 16–256 KiB), XChaCha per chunk,
      systematic RLNC over GF(2^8) (16 per generation, 8 coded, `mint` for more, coefficients in each fragment), content
      keys (public: content alone; else + the space's salt), key-derived addresses, the index as a tree (leaves of 192
      generations, inner levels of 7000: no size limit), `Decoder` (checked against the index, incremental elimination),
      `read_chunk` (a seek). Tests 11 (any 16 of 24 in the worst order, 15 not enough, a minted replacement, forged
      fragment/index/key refused, redundant recognised, one chunk alone, same content+space = same file (dedup,
      resume), another space/public differ, a short last generation from coded fragments only, an inner index level,
      pieces fit a sealed contract); planted bugs (no hash check; coded = a copy) turn 1 and 3 red.
- [x] F2 capability `files` (page) + core bindings: put (content hash in 4 MiB slices, key from the space's salt or
      public, a generation at a time: 24 fragments at once, a fragment not stored replaced by a minted one, progress in
      the account's `uploads` → resume, index last), get / stream (a generation raced: every listed fragment at once,
      decoded on the first 16 valid), chunk (a seek), inline ≤ 64 KiB; `core.take_got` so fragments never pile up in
      the core. LIVE on 17573 (one node): 20 MiB (81 chunks, 6 generations) up 6.4 s, down 0.25 s, same SHA-256, chunk
      21 exact, inline round-trips. TWO NODES (fx gateway :17691 + fy :17692, 0.2.139, own webapp caches, a rehearsal
      site EckhKs… via PUBLISH_KEY_SEED — a fresh node's signer holds no site key): up on X 49.5 s, down on Y 0.7 s,
      same SHA-256, a seek on Y 8 ms; no console errors. Y's speed likely = fragments already propagated to it by X's
      PUTs (its only peer): correctness across nodes proven, network fetch time not measured.
- [x] F3 attachments: component `attachments` (📎 picker: sent as picked with progress, the item waits; an image's WebP
      thumbnail ≤ 320 px made here and kept in the reference; show: images by thumbnail → full in a dialog, other files
      name · size · Download, video Play); `content` items carry `files`; wired into the room (Chat, Messages), Board
      posts (public where the post is public) and Mail (sealed in the mail). LIVE on 17573: a 43 KB PNG on a profile
      post (inline, thumbnail, full view), a 300 KB file in a Chat channel (coded; downloaded byte-identical), a mail to
      self with a 300 KB file shown in Sent; no console errors.
- [x] F4a VIDEOS app, the basic (owner 09-30: name "Videos"; basic first, commercial packaging later): upload (file
      as is + poster + duration), a space's/your list, watch page streaming MP4 by byte range (MSE + mp4box.js from the
      archived craftworks-video.js), whole-file fallback. gov APPS += videos.
      Built as a LENS (owner: "maybe it is content kind?", "refer handcraft"): `kinds` = handcraft's layers (domain
      derived ← kind with its fields ← context = tags ← bundle later); a video is a `posts` item of a video-domain kind
      (video, movie, episode, music-video, short) with `meta`; Videos lists that domain, Board the posts. Channel-first
      (owner: YouTube/TikTok have no spaces; spaces are a side effect). Likes = the post vote; subscribe = follow edge.
      17573: 5.3 MB 720p H.264 uploaded as a Movie (year 2026), feed card with poster + 0:20, watch STREAMS by range
      (mode stream), seek to 15 s plays to the end, like ▲ 1, comment shown, 0 console errors.
- [x] FOLLOW is uniform (owner 09-30 "make it uniform, nothing new"): a follow names a SPACE — a person's DID is their
      personal space; a shared space's id with its public description in the edge (`people.about`). The feed (Board's
      and Videos') reads every followed space; Follow beside Join on a public space; Contacts lists followed spaces.
      fx/fy: B (not a member) followed A's public space → its post in B's feed in 5 s.
- [x] F4 VIDEO PIPELINE (owner 09-30: "full pipeline goes along", "encoding on the user node in the background with
      progress", "h264+h265 both is not efficient", "4K"): video-studio (Mediabunny 1.61.0 MPL-2.0, WebCodecs) →
      fragmented-MP4 renditions + manifest (fragment index, poster, scrub strip, subtitles); LEAN ladder = one
      efficient hardware family (AV1 if hw else HEVC) to the source (4K) + H.264 720/360 net; fast post + background
      pending (lease `encodes`, progress on the video, per-device capability); original released unless kept;
      adaptive player (MediaCapabilities family by screen reach, throughput switching, fragment seeks). Measured M4
      Max: VideoToolbox HW encoders = H.264/HEVC/ProRes/JPEG, NO AV1; Chrome 154: HEVC+AV1 4K decode smooth. 17573:
      4K 10 s clip → posted 12 s, ladder done 32 s, 24.7 MB total (source 26 MB, released), plays HEVC 2160p.
      Fixed on the way: a manifest inline in the item row overflowed it (now always coded, `inline: false`).
- [x] SUBTITLES as data (owner 09-30): attaching kind "subtitle" (`kinds.attaching`), posts.attach/attached/editItem,
      `subtitle-store` (of/add/update/remove, WebVTT⇄SRT), Videos composes tracks, SUBTITLES app (#/subtitles: yours,
      for/<item>, e/<ref> editor + export). Manifest subtitles migrated on the author's watch. 17573: migrated 2
      (manifest 0 left), edit saved + SRT export correct, paste-add "Español", 0 errors.
- [x] SUBTITLES BY VIDEO ID, kept anywhere (owner: "like drive … mapped to videos cross spaces by video id"; "read and
      loading is based on access control and privacy"): manifest `vid` (from the original's key: public global,
      private space-salted), subtitle `meta.for`, `items.attach(..., { place })`, `subs.of` = with-the-video ∪ the
      feed's places filtered by id. SETTINGS per content domain (`kinds.policyDomain`, legacy app-named fallback;
      owner: "posts→Board is just current implementation"). 17573: same id for the space copy; a team-kept and an
      own-kept track show on both copies. NOT measured: a non-member's view across two accounts (by construction).
- [x] AUDIO + sub-types (owner 09-30: "same for audio … lyric for song … podcast … video app handle various sub-type?"):
      `media` page = Videos + Audio by route; sub-type chips; studio audio path (AAC fragmented, cover + tags from the
      file via Mediabunny, id by files.keyOf); `kinds.attachLabel` (Subtitles / Lyrics / Transcript); timed text in sync
      for audio; speed for podcasts/audiobooks. 17573: tagged MP3 → prefilled, AAC 160 streamed, cover, lyrics lit at
      3 s and 8 s, click seeks; chips filter. ACCESS measured fx/fy: A sees both tracks, B (follows A, not in the private
      space) sees only the public one.
- [ ] F4 video (old plan line): CMAF segments, renditions (AV1+Opus, H.264+AAC), remux (mp4box) or WebCodecs encode, MediaSource ABR,
      poster + scrub strip, subtitles.
- [x] F5a DRIVE (owner 09-30: every uploaded file is in Drive; attach from Drive; choose any space's Drive):
      `drive-store` (a `drive` table per space, the account's included: rows = references + when, folder, from; every
      upload lists in yours and in its space's), the Drive app (personal + shared views, a chooser of every Drive,
      folders, upload into the folder open, open/download, Move, Remove from Drive; in-app dialogs, no browser
      prompts), the 📎 picker (From this device / From Drive with the same chooser), "Save to Drive" on a file someone
      shared; `gov` APPS gains "drive". LIVE on 17573: upload in Drive, New folder, Move into it, the chooser listing
      every space, attach from Your Drive into a space's chat with a device file, the device file listed in both Drives;
      no console errors.
- [ ] F5b ACCESS like traditional access control (owner 09-30: "fix the gaps … re-key in the background, queued,
      trigger based"). ARCHITECTURE §6 Access rewritten. Phases:
  - [x] R1 one owner of a file's key: `k/<id>` rows in the space's `files` table; refs carry `id` + `in`; readers
        resolve through the row. Drive-page uploads follow the space's read policy.
  - [x] R2 due rows re-keyed by any member's page (derived from the table: salt `n`, pub vs policy, adopted, no `h`);
        salt rotated when a removal is in the group; adopt on cross-space attach / Save to Drive; a post made private.
  - [x] R3 own uploads kept on removal (the removed page copies them into its own space); public copies carry the
        current key.
  - [ ] R4 the delegate works due rows on wake-up (no page open). Owner 10-05: "Build it fully" (told: ~1,850 lines of
        page logic to port, one identity change). PHASES (each: build, test, commit):
    - [ ] R4a SPACE TABLE READER in the delegate (`delegate/src/table.rs`, a pure step machine like upkeep): writers =
          the group roster's credential WRITER keys (a DID writes a space under `space_writer`, the same on every device);
          the writers bag (sealed with epoch 0's `bag-writers` key, AES-GCM) narrows whose catalogs are asked; each
          writer's catalog `x<id12>-tables` → table listed (blinded: `blind_name(space_table_key)`) → its feed tail →
          tree blocks (Sealed) → epoch keys (`identity::epoch_secret`) → `feed` merge → rows; `departed` caps. Tested
          against a scripted network with feeds written by the real data/feed crates.
    - [ ] R4b WRITER: this member's own feed of a table from the delegate (writers bag + catalog listing first when
          new), rows as feed versions, signed with `space_writer`, sealed with the newest epoch held.
    - [ ] R4c DUE + ROTATE in Rust: `acts`/`pub-acts` replayed by `gov` → removals; the group without them; salt
          rotated (upkeep randomness); due rows as `file-keys.js`; the same rank/takeover hash as pages.
    - [ ] R4d RECODE: a generation per step (GET fragments → decode → encode under the new key → PUT pieces), index and
          root, row changed, old pieces burned; progress `p/<id>` shared with pages; a budget per wake-up.
    - [ ] R4e WIRING + LIVE: wake-ups run re-key rounds after admissions; identity rebuilt (one PIN re-entry); fx/fy: A
          removes B with A's page closed → A's delegate re-keys; B's old reference not found.
  - [x] R5 leaving a space removes the leaver's nodes from its group (a leave = a removal): gov `leave` act + `gone`
        set (test 12/12), moderation.enforce takes out gone DIDs' nodes (upkeep, every 30 s on an admin's page).
  - [x] R6 BURN old pieces at re-key: new `piece` contract (contracts-src/piece, 2 tests; wasm d48911c4…), first
        write names sha-256(burn secret); BURNED state for good; refs/rows carry `b`, rows keep secret `x`; old
        `sealed` files still read (no `b`). fx/fy: B read 300 KB (its node cached it), A removed B → re-key 27 s,
        old root BURNED on both nodes, B's old ref "not found", members read the new copy. Found + fixed: two salts
        made at once in one page (worker + upload) — creation serialized; secret kept in the row.
    Also fixed on the way: a removed member ignored welcomes to a space still listed (re-invite never worked); an
    inviter could reuse a spent key package from a stale card (table `keypacks`); ban/leave acts name nodes; any
    member (not only admins) takes gone members' nodes out of the group; per-row owner rank + takeover, progress in
    the space table. fx/fy: B rejoined, uploaded, left → own file copied in 12 s; A: B out, salt rotated.
    FIXED (was pre-existing): a space table read only the CURRENT group's devices, so all a departed member wrote
    vanished for everyone. Now the member taking their nodes out records where each feed stands (space table
    `departed`, before the commit) and readers count those feeds up to there. fx: B left RK2 → A's fresh page still
    sees B's file row (re-keyed, read 92160 B); B then wrote after leaving (its feed at seq 2, cap 1) → not counted. Also space RK on fx: re-invites after the
    spent-package welcome never open (WelcomeKeyPackageNotFound) while fresh spaces RK2/RK3 invite fine — cause unknown.
    Verified (09-30): 17573 — rotate stand-in re-keyed in 3 s, bytes equal via the old ref; board public→members
    re-keyed in 8 s; adopted copy in 3 s. fx/fy two accounts — A removes B: salt 0→1 and the file re-keyed 4 s
    after, A reads by id alone; B: left, the new row unreadable, its salt view 0.

## Phase 3 READS (owner plan 09-30: files+media → Reads → Lifecycle) — ARCHITECTURE row 3
Uses what freenet-prolly already has (rev 17d67d8: `range`/`range_with` + `frontier_of` fetch only a range's blocks;
`diff` between roots; `aggregate` counts). Nothing new in the tree.
- [x] R3a time-ordered item ids (`content`): `t` ‖ ms base36 (9) ‖ 8 hex — sorts after old hex ids and `r-` reactions.
- [x] R3b `data::Open::page` (lo/hi/after/reverse/limit; tree walked only along the page's path; tail merged in the
      span covered; per-row-sealed legacy trees read whole and cut) + core `tail_page`. Test: latest 20 of 600 rows
      fetched 7 blocks, a whole read 136; pages chain to all 600 in order (data tests 14/14).
- [x] R3c storage LAZY tails/tables (no tree read at open, pushes only poke), `table.page` across feeds (each feed's
      top N, merged, top N kept, `next`), write looks up the key's current version on a lazy table; `content.in(c,
      { paged })` with `older()`/`hasMore()`, reactions by `r-<id>-` ranges, refresh of what is held on change. Chat
      rooms and ACTIVITY (unread counts: it used to read every channel and board whole at startup) read paged.
      Board/items still read whole (Top needs every post). BUG found by the test and fixed: upgrading a lazy tail that
      is not on the network yet marked it made, so its first write skipped the catalog listing (a table no reload
      finds) — whole() now leaves an absent tail absent.
- [x] R3d CHANGE-ONLY: already so — blocks are content-addressed and cached per open tail, so a new root reuses
      every held block and only changed nodes are fetched; a lazy table's readers re-page what they hold.
- [x] R3e LISTS BY TIME WINDOW (owner: "top … bound by time: today, week, month"; "refer to how grid implemented
      this"): Grid's feeds — New (date, a month at a time, auto-older on scroll), Hot (ups+downs+comments), Best
      (net+comments), Rising (interactions per hour of age), Top (net votes or comments) over Today/This week/This
      month, COUNTS windowed too, plus a client SORT (time/votes/comments ▲▼). A window = a key range (ids sort by time).
      Laziness kept: lists default bounded; the subtitle lookup reads only spaces using Subtitles + profiles; public
      copies sync reads only this writer's own feed whole. 17573: all 4 boards stay lazy after Board + Videos; a vote
      now tops "Top today".
- [x] R3f ONE FEED BAR for every content app (owner: "implement the same across all content app. video and audio
      too"): `feed-bar` component (feed/window/top-by/sort ▲▼, New's auto-older) used by Board, Videos, Audio and
      Subtitles. 17573: all four show it; Top adds window + top-by; a sort adds direction; 0 errors.
- [x] R3g ATTACHED ITEMS UNBOUNDED BY WINDOWS (owner: "browsing subtitle can use that but loading subtitle for
      video/audio should not limit by time"): what attaches to an item (tracks, comments) is always newer than it, so
      `attached`, `thread`, `get` and the team-space track search read from the item's time on (the earlier of the
      item's and its manifest's `at`: a later copy still finds older tracks) — complete, never a list window, never the
      place whole. Items from before time ids: the place whole. 17573: a new video's track found, get works, board
      stays lazy; legacy-id videos fall back to whole (as they must).
- Verified 17573: a 150-message channel opens with the newest 50 (lazy: no whole read), pages back to all 150 in
  order, "Load earlier" in Chat, a new message live; Board and Videos unaffected; 0 errors.
- [ ] VIEW COUNTS → roadmap row 9 (owner 09-30: "cover later under observability and analytics"). Shape noted in
      ARCHITECTURE row 9: one entry per viewer per item (Grid's tally: deduped), windowed like the feeds, counted
      where the item's access allows; adds Popular to the feed bar. Not built; the feed bar has no Popular until then.

## Phase 4 LIFECYCLE (owner "Proceed" 09-30, after Reads) — ARCHITECTURE row 4
- [x] L4a FLUSH WHEN QUIET: a tail with rows flushes 1–2 min after its last write (spread so tables quiet together do
      not flush together), and on open for sealed tails this node writes (rows a closed page left). 17573: a note's row
      flushed ~60 s after the write; ~30 tables with leftover rows flushed on open.
- [x] L4b PUBLIC TREES: a public tail (card, profile `posts`, a board's public copies, public acts) could never flush
      (sealed blocks need a key) — so it stopped taking rows at the tail's 256 KiB. Now it flushes into a tree IN THE
      CLEAR (Block contracts named by their ids, no SEALED_TREE mark; readers already read such trees). A public tail
      flushes only after a write here (its writes need the grant a read does not show). Test: data
      `a_public_tail_is_readable_by_anyone_and_flushes_in_the_clear`. Two-node: A wrote 40 profile posts → A's `posts`
      flushed:true pending:0 (42 rows in the tree); B (outside, following) read all 40.
- [x] L4c KEEP: `data::Open::asset` (every group the current tree reaches) + `stored` (a block as put: members from
      what is held, parity coded again, sealed with the deterministic nonce) — test: a READER makes every block of a
      300-row table byte-for-byte as the writer put it (+ control: a group not held makes nothing). `storage` tail
      `keep()`: probe every slot (health per group), put every block again, put the tail's state again. `keep`
      service: due = no `keep` record newer than 7 d, one table a minute while a page is open; Account → Storage
      shows Kept + Health and "Keep all now". The identity's grant limit rose 16 → 32 tables (`uses` is 17 with
      `keep`; 17 was refused whole as BadTable). 17573: 40 tables kept in 25 s, all whole, every block put, 0 unmade;
      `uploads` FAILS: a tree block gone from the node and too little of its group to rebuild (data lost before
      keeping existed — shown as failed, never hidden). Missing-block repair is proven by the unit test only (a
      single node holds everything it was given).
- [x] L4d FILES KEPT: `files.keep(sp, row)` asks every piece (root, index, each generation's listed fragments), puts
      each back as stored (`core.file_keep`; a burned piece never), health per generation (whole/degraded/damaged);
      the `keep` service keeps the account's and every space's coded files on the same weekly clock (`f/<space>/<id>`
      records); Storage shows a Files line naming any needing attention. 17573: the 4 files still there kept whole
      (11–12 pieces each, ~0.4 s); 80 of 84 file rows have NO ROOT on the node — see FINDING.
- FINDING (measured 09-30, the 17573 node's own log): the node EVICTS by CONTRACT COUNT, not bytes — "resident-overhead
  pressure": ~1 MiB estimated per hosted contract against 12.5 % of spare memory (`--hosting-mem-share`, budget
  ~1.6 GB) → a cap of ~1,600 contracts, while state was 37 MB of a 1 GiB byte budget. 4,365 contracts evicted since
  09-28 (343 in the last hour), unsubscribed and least-recently-used first: file fragments and tree blocks (one
  contract each; a small table's tree alone is 9 — its root + 8 root parity). Tails survive (subscribed). A 100 KB
  file is ~12 contracts, a 4K video hundreds. So on ONE machine, data beyond ~1,600 contracts is lost, whatever
  keeping does. Options for the owner: raise the node's `--hosting-mem-share` (config, the owner's node), and/or
  fewer contracts per file/tree (packing — the 09-27 ruling said packs were not worth it; this is new evidence).
- [x] L4e BLINDED TABLE NAMES (see the design below): identity `blind_name` + `Sign.table` (test
      `a_blinded_label_signs_only_with_its_name_and_grant`); data `adopt` (test `a_table_moves_to_a_new_label_in_one_step`:
      60 rows — tree 40, tail 20 — read at the new label); storage opens the blinded label, MOVES its own legacy tables
      (one whole signed step), reads others' where they are. fx: 23 tables moved on first open (files 22 rows before
      and after, spaces 7); a note written, read back at the blinded label after a reload; two accounts: A made a
      space, B joined and read A's post from A's blinded board feed.
- [x] RE-INVITE fixed for real: B's `spacekeys` was GONE from the network (evicted under the old contract cap) while
      its card still offered 4 key packages → every welcome failed; and a second welcome to a DID still in the group
      failed `DuplicateLeafData`. Now: no batch held → upkeep makes a fresh set and puts it on the card; welcoming
      again takes the DID's old entry out of the group first. fx: B renewed, A re-invited, B joined.
- UPSTREAM (09-30): measured a fresh node (fill example, f2ad0b8): immutable one-module contracts cost ~0.107 MiB
  marginal RSS each, vs the node's 1 MiB constant; posted on freenet-core#5647 (issuecomment-5907919358). Owner's
  call: keep small raced pieces (no packing); the count cap is the node's accounting. Until upstream changes it, nodes
  run with a higher `--hosting-mem-share`.

## Markdown + inline media (owner 09-30: "md for text editor like grid; edit post and comment; image/video/audio inline")
- [x] `markdown` (Grid's renderer, escape-first; `file:KEY` names the item's own files), `md-editor` (toolbar, preview,
      🖼 inline media, grows with its text), attachments picker `media: true` (video/audio through `video-studio`:
      streamed, poster / album cover, video id), covers until played (owner: "like youtube"), subtitles by the file's
      video id (`subtitle-store.forFile`, lyrics tag as fallback). Board: posts and comments in Markdown, Edit for
      one's own. A: post with heading/bold/italic/code/link/list/quote + image + video + audio rendered; covers only
      (0 video elements before a click); both played through the pipeline; editor 110→379 px for 17 lines.
- [x] FORK RECOVERY (owner 09-30: "set a handle on a new ID on another node … WouldFork { last_seq: 3 }"): the identity
      signed a tail's steps the page could not read back (never landed, lost, or the read went unanswered). A write it
      refuses as a fork now reads the network again: caught up → written on it; the node ANSWERED and holds less → the
      step goes past `last_seq` as a WHOLE state (`data::Open::skip_to`; a delta must be the exact next step, a higher
      whole state replaces a lower); the node silent → a clear "try again", never a guess that could drop rows.
      Test `a_write_skips_past_steps_the_network_never_saw`. A: network at 47, identity signed 48–49 unsent → the
      write went out as 50, read back 50 with its row.

## L4e BLINDED TABLE NAMES — design (09-30, owner: "do all 1-4")
Today a tail's params carry its label in the clear: `t/notes` under the account key, `t/x<space12>-board` under a
member's key — hosting nodes read what each table is, and every member's feeds of a space share the space prefix (who
is in which space links). Design:
- LABEL `t/~<hex16>` = keyed hash of the table's name with a BLINDING KEY by scope — account tables: from the data
  key (the account's devices); a space's tables: from the space id (its members); public tails: from the owner key
  (findable by anyone who can name the owner — obfuscation only, as they must be).
- The identity signs by the NAME: a sign request carries the plain table; the identity recomputes the blinded label
  and checks it is the params' label, then checks the grant by name (as now).
- MOVE, not copy: a table's tree blocks are addressed by its TABLE KEY (from its plain name, unchanged), so the
  writer's first step at the blinded label is its old state re-signed (root, pending rows, parity ids) — ONE signed
  step per table per writer. Readers open the blinded tail; absent → the legacy one, read-only, until its writer moves.
- Phases: B1 label + blinding keys (data, core, identity), a table moved on its writer's next open; B2 readers'
  fallback; B3 the delegate's reads (card, bags) by the new labels; then the old names stop being written.
- [x] (4c) RE-INVITE FAILURE (RK): the delegate, admitting with no page open, picked ANY key package on the card — not
      the account's record of used ones (`keypacks`), nor recording its own — so page and delegate could spend one
      twice; the second welcome then fails `WelcomeKeyPackageNotFound`. Now the mandate carries the spent tags
      (`identity::kp_tag`, the page's SHA-256 tag), the delegate skips them and those it used since, reports each
      one it uses (`Admitted.kp`), and the page records it before acknowledging. Tested: identity (spent kept, tag =
      page's); a live two-account admission-with-no-page run is still to do.
- [x] (2) WAKE-UPS measured (fx): 14 in 15.00 min at the 60 s floor + jitter — none refused, at most one skipped.

## 2026-10-03 — no lookups of what is not there (3b8cdcd)
- Owner's node log: a not-found GET costs 6.5 s to 115 s (60 s per-peer attempt deadline); the page waited 30 s per
  table, blinded-then-legacy in series (rail 17 s on `spaces`, `reads` 2×30 s).
- Catalog rows carry `b:1` (blinded); others' feeds open where their catalog says; members make their space catalog
  (`~here`) on first open; missing catalogs polled 5 s→5 min; epoch history walked once per device; activity scans
  spaces in parallel. Trace line `asked, not there` names every not-found read.
- fx second load: page ready 0.4 s; only not-founds left = B's two never-made space catalogs (B down; stop once B
  opens the new build). Open: spaces still open ~2 s each in series in the background (cause unmeasured).

## 2026-10-03 — Index-first reads, public participation, settings in one place (owner's request)
Rule: nothing reads a table its writers' data does not say exists; every write and read complies with policy.
Design source: ARCHITECTURE §1 (Discover = the public index: bags public spaces and posts list themselves in) and §5
(federated data: content in the author's table + a pointer in the target's index, sealed for private use).
- [x] 1a SPACE WRITERS BAG (fx: 10 spaces' bags complete, page ready 0.46 s, member catalogs no longer searched): a sealed bag per space listing the members who have a catalog there (dropped once, on first
      write / backfilled on load); readers gather only listed writers; one bag poll replaces N catalog polls.
- [x] 1b DISCOVER BAG (fx: Discover 47 items <1.5 s from bags; real net via B: 59 s first visit — bags searched once then made — 1.5 s after; public acts' writers bag added, empty until an owner/admin loads, old walk until then): public posts/videos/audio drop a pointer {writer, table, item} in a public per-kind, per-month
      bag; Discover reads the bag and only the tails it names; authors and members backfill missing pointers; drop the crawl.
- [x] 2a NOTES AND DRIVE ON `content` (owner: why don't they follow the same path?): a note an item of kind `note`
      (text, tags, pin), a Drive entry an item of kind `file` (its ref, folder); old rows converted ONCE by upkeep's
      migration (never on a page load), then only items read — so authorship, edits, removal, moderation, policy come
      from one place. Owner 10-03: the SAME structure as every app — a note or an uploaded file can be private,
      members-only or PUBLIC (space policy as ceiling), listed in Discover, with comments/votes; Notes/Drive only lenses
      (note colour/archived, file folder in `meta`). One upload door: `files` + Drive's catalogue (a video/audio post
      references the same file).
      DONE NOTES: notes are items of kind `note` (own domain, own policy); the page reads/writes `items`; audience picker
      in the composer (yours start Only you, a space's Members); migration v4 brings old notes over (time, colour,
      archive, pins, labels; `meta.from`). ONE POLICY RESOLVER in roles (policyIn/allowsIn: domain → setting from before
      → space) used by items and content (content checked every board item against "board"; now each its domain's).
      fx: 2 notes brought over (private, journal); new private note → journal; edit kept one card; Board/Discover
      unchanged (49/48, no notes). Found + fixed: a form reset put the picker back on Everyone (a note made public).
      DONE DRIVE: entries are items of kind `file` (an empty folder: `folder`) — drive-store keeps its API over `items`;
      the Drive page's audience picker; migration v5 brings the old catalogues over (time, folder, source; `fid`).
      Lists without vote tallies (`withVotes: false`) for Notes/Drive; the Drive selector no longer waits on every
      space's roles (first load 4 s → 178 ms). fx: 10 entries brought over, uploads listed, Notes intact.
      DONE first part (47e3f2d): Drive files an upload from another app in its TYPE's folder (/Videos /Audio /Images
      /Documents /Files: `kinds.ofType`, `kinds.domainName`); uploads made in Drive stay where put; migration v3.
- [x] ONE AUDIENCE PICKER (owner 10-03; DONE: `audience` component, items `audience`/content `aud`; fx: members-only
      post in a public space on its board, absent from Discover and its outside view; public control listed in both): the one choice of who sees an item, for every app (Board, Videos, Audio,
      Notes, Drive) — today two copies (board.js:396, media.js:318). Personal space: Only you / Friends / Followers /
      Everyone; a shared space: Members / Public (its policy the ceiling: members-only = members post and read, nobody
      makes it public). Stored by one rule in `items`.
- [x] EDIT BY POLICY (owner 10-03; DONE with Notes — content.mayEdit, kinds.collaborative, creator kept as `by`): who may edit an item is the space's `edit` policy, in the one item model, for
      every app (Notes stay collaborative; Board inherits the same shared model) — and an APP may ENFORCE stricter
      (e.g. Board: author-only). An item keeps its creator (`by`) when another member edits it; the editor is recorded.
- [x] FRIENDS and FOLLOWERS audiences (DONE as CIRCLES — `circles`: a hidden owned space per audience, members synced by
      upkeep; follow notices; picker Everyone/Followers/Friends/Only you. fx one account: circle made, post in own feed, not
      in Discover, not on the rail. NOT YET TESTED with a second person (no second test account: 17573 is the owner's).) (owner 10-03: "add friend/follow option"): an item sealed to a KEY shared with
      just those people — one per audience, held like a space's epoch key (handed to each through their inbox; a new
      key on a removal, so a removed friend/follower reads nothing written after). Friends = the mutual friends
      (`edge` friend). Followers = the people who follow you. A follow NEVER needs approval: following drops a notice
      in the followed person's inbox, and their upkeep adds the follower to the followers key by itself (a reader must
      be known to be sealed to; the inbox notice makes them known). Fully automatic — no setting (owner 10-03).
- [x] READ AND WRITE SET APART, as a database's (owner 10-03; DONE: `roles.mayWrite` the one check for both kinds of
      space, `roles.credToCite`; picker's "Who may comment and vote"; circles issue unlisted `cred-<token>` tables. fx one
      account: rule set, author comments, counts unchanged. NOT YET TESTED with a second person.): each item's READ audience and its WRITE (comment,
      vote) audience are separate settings — anyone / followers / friends / members / only its author — in a personal
      or a shared space. No server checks a write: a write limit is VERIFIED BY EACH READER (as `content` checks
      authors against policy now). So a writer must be PROVABLY allowed: a space's member by its member credential; a
      FRIEND or FOLLOWER by a CREDENTIAL that is a signed TABLE WRITE like every other (no new identity operation, no
      signer change): the owner's node writes, per friend/follower, an UNLISTED public table of the owner's account at
      a random name (`cred-<token>`) whose row names the holder's DID; the token is handed to the holder (through the
      circle); a comment/vote cites it; a reader reads that table and checks it names the writer. Never listed (card,
      catalog): not enumerable. Removed: the owner clears it — later writes stop verifying. (Superseded: sealing restricted comments inside the circle, which hid them from
      public readers — the owner: "in a database, write members-only, read public".)
- [x] ONE ACCESS CONTROL, PERSONAL SPACE TOO (DONE 10-05: the personal space's policies are `policy` ACTS on its owner's
      card replayed by the core's `Governance` — inherited (Everything you post → each app → the item) and TIME-AWARE
      (profile threads and counts judge each comment/vote at its own time); gov `who` gains followers/friends/author
      (no space role passes them); `items.writeCred` applies the full rule (Only you closed nothing before). Old
      `policy:` card rows count as acts at time 0. fx: B commented, A set Everything → Only you: B's box closed
      ("Only its author comments here"), the earlier comment kept, one sent after not counted; A set Image → Anyone:
      box back. Test account A left with Everything=Only you, Image=Anyone.) (owner 10-03: "replicate access control for vote/comment for personal
      space … same access control across all, not a separate implementation"): the personal space governed by the SAME
      `roles` model as a shared space — an acts log the account signs; policies per app/domain/item for read, post,
      comment, vote; `r.allows(action, who, path)` the one check everywhere (comments.js today skips it for a profile:
      anyone). Only the GROUPS a policy names differ: a shared space's members/roles, the personal space's
      friends/followers/only you (the same groups the audience picker seals to).
- [x] ONE IMAGE CAPABILITY + MEDIA DOMAINS DECLARED ONCE (DONE: `image-studio`; `kinds.media()`/`mediaOf` — a new domain
      such as BOOKS/COMICS (owner: "as per handcraft") is one entry: its types, label, icon, maker, viewer) (owner 10-03: "image will unify with image capability / app"): an image's making
      (thumbnail; sizes) moves out of `attachments` into one capability, as `video-studio` is for video/audio — used
      by the Images app and every editor alike.
- [x] IMAGES APP (owner 10-03; DONE 222be95: `#/image` on the one media page — a wall (photostream, Following, Saved,
      Discover, a space's), the picture whole on its page (preview first, then the file; click: actual size), kinds
      chosen on upload with their fields, filed in Drive /Images, its rules in Settings. fx: A uploaded a Photo, B opened
      it from Discover at 800 px with comments): a lens on the image domain (public view like Flickr), as Videos is YouTube's and Audio
      Spotify's — same `items`/`content` path, its Drive folder /Images. Its kinds (owner 10-03): IMAGE (the general one, as Videos'
      `video`), with PHOTO and ARTWORK
      (already `kinds`' image domain, with their fields), chosen on upload as Videos' movie/episode/short are.
- [x] BOOKS APP (owner 10-04 "Also for Book kinds app"; DONE 3c30610): `#/book` on the one media page — a shelf of covers;
      book + comic now their own domain `book` (its rules in Settings, Drive /Books). Formats: PDF (PDF.js 6.4.299 vendored,
      its worker on the page: a module worker from bytes does not start in the app's no-origin frame), EPUB and CBZ (own
      `zip` reader, DecompressionStream). `book-studio` makes (cover from page 1 / cover image, title+author prefilled)
      and opens; `book-view` reads: page or spread, right-to-left for manga, EPUB chapters in a fully sandboxed frame
      (pictures as data: URLs, styles inline), A−/A+, full screen, place kept per book on the device. A PDF is a book
      only when made one in Books (a PDF attached elsewhere stays a document). fx: A uploaded PDF/CBZ/EPUB, B read all
      three from Discover (PDF fits the screen; spread 1–2/3–4/5; EPUB ch 2 at 120%).
- [ ] 2  PUBLIC PARTICIPATION: policy `anyone` for post/comment/vote per app; an outsider's comment/vote lives in their
      own public tail + a pointer in the post's bag; readers accept it only if the policy at its time allowed it.
      PER-POST AUDIENCE (owner): the author picks public / members for each post, the space+app policy the ceiling
      (path `board/p/<id>`); comments and votes follow the post's audience. PER-POST COMMENT/VOTE (owner): the author also
      sets who may comment and who may vote on that post (anyone / members / admins / owner / nobody), within the space's
      ceiling and never wider than who reads it — e.g. a public post with members-only or admins-only comments.
      ONE IMPLEMENTATION (owner: every app inherits it): policies by path in `roles` (item path `<app>/p/<id>`), enforced
      in `content` (Board, Videos, Audio, Chat, Messages) — Notes and Drive call the same `roles.allows` for their items;
      Chat keeps per-channel policy (`chat/<channel>`).
- [x] 3  SETTINGS: a Settings app in every space (575130b, a4e8c5a); apps' own settings links removed (d0e6aa5).

- Test account on fy (17692, site EckhKsQW…): PIN 731731; words juice bulk shy trend need book heavy curtain old network arrange disagree (made 2026-10-03, outsider tests).

## CHAT AS A REGULAR APP (owner 10-03: "items them"; personal space gets its own Chat; Message/Mail/Contact stay global)
- [ ] 1. CHANNELS ARE ITEMS (kind `channel`, domain `chat`) in shared spaces — `conversation.channels(server)` the one
      place: lists channel items (title = name, `meta.cid` = its message table's id, so no message moves), add/rename/
      remove = items submit/edit/remove; made only by who may make channels (as before). A channel's WHO MAY POST = its
      own rule (`meta.write.post`, via `roles.mayWrite`) else the space's Chat policy (path `chat`). Owner/admin node
      MIGRATES the old `channels` table (with each `chat/<cid>` policy → `meta.write.post`), then sets the act
      `config chat.channels = "items"`: readers stop opening the old table once that act is read.
- [ ] 2. SETTINGS: Chat section's channel rows edit the items (name, who may post) — no `chat/<id>` path policies.
- [ ] 3. PERSONAL CHAT: channels as items in the personal space; others' messages kept in their own profiles, pointed,
      read through the outsider source (generalized from a space to a place); your Home's Chat section: who may post.

## SPACE GROUP FORKS (owner 10-03: "detect and heal"; no single committer — owners are often offline)
Cause (measured on 7509): two nodes committed from the same epoch at once (auto-admission on every owner/admin page
and delegate since 09-29/30); each node's append saw only its own write (local view) and moved on → two "epoch N"
with different secrets; each epoch's log is addressed by its secret, so the branches never meet. Readers on one branch
cannot open blocks sealed on the other ("block … is not what was asked"), the catalog then counts as absent → the
writer's whole part hidden (Ivvor ↔ onlyabrak in Craftworks and Ivvor's Space).
- [x] ordering: each entry under its own key (`c/<n>~<tag>`), position's entry = lowest; fresh reread after append.
- [x] (a5db5a4) HEAL only what this node committed itself (own `change`, adopted upkeep commit) — never a commit received from
      another (a removed member's late entry for an old epoch must not roll anyone back): snapshot before own commit,
      lost if the log's entry for that epoch is another → restore, apply the winner, REDO the intent (unless satisfied;
      a removal never undone).
- [x] (a5db5a4; every group switch too: a262c33) NO DATA LOSS: the losing branch's epoch secrets kept (spacekeys `<at>~lost`); data crate: per-epoch ALTERNATE
      keys (read only; rows opened with one are stale → resealed under the winner's key by `migrate`).
- [x] (a5db5a4: `storage.sealNewest`) Stale sealing after heal: `sealNewest` switches on a different key, not only a higher epoch.
- [x] (d95ad33: branch announced per epoch in the writers list; a member on the owner's branch re-adds one on another,
      once per divergence; the welcome taken only onto the owner's branch. LIVE 10-05: onlyabrak opens Ivvor's space whole)
      EXISTING forks (no snapshot from before them): the space's owner re-adds members found on another branch
      (remove + add + welcome); their node records its branch keys before joining.
- Test account on fx (17691), made 2026-10-03 for the fork test after the identity rebuild logged the old fx account out (its PIN unknown): PIN 818818; words lobster patrol future pumpkin monkey senior solve trend airport unit notice eye.

## One app frame (owner 2026-10-04): every app the same behaviour for free
Owner: "we should use just one implementation and every app should get the same behavior for free"; "looks stay per
capability — shared capability component that can be used across apps". An APP = its kinds + layout + its own extras.
- [x] 1. `where` (packages/where.js): whose place (mine/space/person/discover), THE read, links, standard tabs
      (landing · app extras · Discover). Every app on it; no app reads ctx.space / `u/` itself (grep).
- [~] 2. `cards` (9929f16: video/audio, note, file looks; embed any item `item:REF`; attachments; pageOf carries place. LEFT: Board's post look + Caption's row — with step 3's actions): each KIND's look (domain: text, video, audio, note, file, caption) — card (lists) + full (item page,
      media-view for media). Mixed lists (Discover, a person's space, Drive) render each item by its kind.
- [x] 3. One ACTIONS component (actions.js; Board, comments, Video on it; Save on every kind; Saved tab standard via `where`; Video's broken Saved fixed): vote/like, comment, share, save, hide, remove, edit — ONE may-check (items.mayWriteOn;
      Board's roles.allows copy goes).
- [ ] 4. (owner 10-04: every kind made AND embedded from the editor — Insert: new item of any kind via the one composer, or an existing one by picker/ref) One COMPOSER (publisher.form for every kind; Board's own goes) and one ITEM PAGE (board post page + media watch).
- [ ] 5. Apps reduced; copies deleted (grep: discovering(), descOf/outsideOf, votes(), own composers).
Then: Settings app in every space (Account moved in; space Home's settings moved in; header Account link gone).
Step 1 done: Board, Video/Audio, Note, Drive, Caption, Contact on `where` (Chat with its rework). Uncommitted on fx/17692 only: personal
Chat circle tabs (to be replaced: channels with an audience, person Chat via `where`).

## Roles composed per space; channels and conversations as one (owner 2026-10-04)
- [x] Roles composed by admins (gov `role`/`assign` acts; permissions ⊆ the maker's; rules name `role:<id>`; one Roles
      column with Admin built in; every app's rule dropdown offers them).
- [x] READ by a role (admins, owner): one GROUP per space and audience (`groups`, circles on it too), any kind —
      posts, videos, files, a channel; kept in step by upkeep; a member removed reads nothing new and sees it no more.
      DONE 10-05: named people as an audience (`list` act, ff0678d); a group's maker offline → the space's admins in it
      keep it after 10 min (b4658b1). Drive uploads to the audience chosen (10-05): before, only public-or-not was
      passed, so "Admins only" became all members. fx: A uploaded admins-only + members files in History test; B (member)
      saw only the members' one. Note: the same bytes uploaded again fold into the existing entry (its audience kept).
- [x] Conversations (direct, group) = your personal Chat's channels; Message merged into Chat (old links forward);
      Friends/Followers channel work dropped. Fixed with it: a paged room missed a late feed's earlier items.

## 2026-10-04/05 session (handover)
Done (commits on main):
- Restored IDs (same words as before the fresh start): card key packages checked against held batches, renewed
  (ef40e4a, 69bc477, 6c68b36); a welcome reads the card fresh (d0e6aa5); welcome-again / renew-keys asks; join
  queue no longer poisoned by one failed op (d53b204) — THE cause of "welcomes never open" for old IDs.
- Late joiners: history walk awaits each log (49bd1bf); welcomes carry earlier epoch secrets, gaps asked of admins
  (a38358f; identity 8ff955b6 — members hand over on PIN).
- Settings app (575130b, a4e8c5a); headers one order, create on the right (6f…); full-width layout; Drive asks
  who sees each upload; cards: a name opens a person/space card, Open → their home; outsider space Home; About;
  spaces panel: Discover spaces + Requested with status; Invites tab: waiting / let in lately / Let in.
- Board's rules are the text domain's (d5b1304) — spaces saved public before must be re-saved once.
- Writers bag fresh reads shared; tail.whole once (measured: tree blocks never fetched twice).
- Named-people lists; audiences kept by admins while the maker is away.
Mail spaces in "To" DONE 10-05 (5 pages): To is chips picked by name from `conversation.mail.addresses()` (people
followed/friends + spaces whose mail is on — yours and public listed), typed name#abc/ids still taken; `#/mail/to/<ref>`
opens the composer to them; ✉ Mail on a person's card and on a space's card (when its mail is on). fx: A turned on
History test's mail; B picked "🏠 History test" (its other spaces, mail off, not offered), sent; A read it in the space's inbox.
Open: F5b R4 (the delegate works due re-key rows
on wake-up). Group-fork rows were done 10-03/04 (a5db5a4, d95ad33, a262c33), ticked 10-05.
Test nodes 17691/17692: fresh accounts, PIN 246810 (older test accounts' PINs above are void).
Late-joiner history CONFIRMED live 10-05: after Ivvor reloaded + PIN, onlyabrak sees Ivvor's New Space whole (apps shown).
