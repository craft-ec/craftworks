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
  - [ ] 1d escrow: epoch secrets sealed to the words, so words alone recover history
  - [ ] 1e sealing whole tree nodes (a hook in freenet-prolly); tables sealed over
