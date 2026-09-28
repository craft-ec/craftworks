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
- [ ] B. data: device tail per app (identity signs), subscribe, write rows, flush into the tree, read tail→tree
- [ ] C. pairing admits a member without the words (the admitted device holding the words signs the addition)
- [ ] D. auth package: on-demand sign-in dialog; import/export key file
- [ ] E. auth: keycraft
- [ ] F. auth: device id + PIN (pairing through the admitted device's inbox; needs C)
- [ ] G. republish Craftworks under the new identity; retire the old signer

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
  so it cannot run inside a delegate without patched crypto deps → DECIDED: the protocol runs in the page's core; the
  delegate is the vault (state + epoch secrets; table keys for granted sites).
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
- [ ] M private messaging — `conversation` kinds: direct, group, mail (owner: broadcast/thread are NOT kinds —
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
- [ ] C3 invites (join request, a member adds), C5 every node of each member   (C4 roles/moderation: done, plan step 5)
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
