# Reads: once, then followed (FOUNDATION §2 R6) — started 10-15

Owner 10-15: "we should not be making unnecessary call … i feel like we are getting the data new all the time";
"do the whole fix" (no release of the 4f6804d patches on their own).

Phases (each: build, fx probe with a control, commit):
- [x] 1 core: a pushed change of ANY followed contract stores its state (`HeadChanged` → `got`), not only tails.
- [x] 2 node: `node.contract(id)` — the one door, read state per contract (UNKNOWN/ASKED/HELD/ABSENT/STALE),
      subscribe on first read, HELD answers from the copy, ABSENT re-asked by the door's one backoff, reconnect → STALE,
      `onContract(id, fn)`; `node.read` and `take_got` callers moved onto it.
- [x] 3 index: bags through the door (inbox, requests, pointers, sealed bags, chains); watchers from `onContract`.
- [x] 4 storage: tails through the door; `reread`/`writersBag(fresh)`/`pollBag`/`absentCatalog` retired.
- [x] 5 build check `tools/read-check.mjs` (GET doors, take_got, backoff, fresh flags) with a positive control.
- [~] 6 fx probe: reload 37 contracts / 46 GETs (repeats: an absent catalog on the door's backoff, the health bag's
      own read-back); frames sent per reload 155 → 117 vs old; other-space key events: bag reads 5 → 0; two-page write
      race 97/97; admission with A's page open + loop check pass. OPEN: A's node never let C in with A's page closed —
      the pre-door control fails the same on the crowded fx (≈ a dozen accounts), so not the door; whether 2de2571
      (live v392) or crowding: settle with a delegate test (N members on one node, wake-ups until each is served).
- [x] 7 R3: no reload for keys or a reconnect. Tables opened before their keys reopen in place (`showAgain`); a node
      that comes back has the door read each followed contract ONCE (re-subscribed, answer handed on as a push); a table
      made here by a PUT is read once through the door so the page follows it. fx (drop.mjs): connection dropped, the
      other page writes — new: no reload (window marker kept), 22 contracts read again once, the row arrives in 5 s;
      old: the page reloaded (marker lost). Write race 97/97 again. Kept on purpose: log out, a login, a newer version,
      the trace's "newest" button.
- [x] 8 R4/R5: `ctx.cache` (loader) — the one per-page cache: a failure forgotten, fallback answered; 11 caches moved
      onto it (items ×3, directory ×2, roles ×2, usage ×2, markdown, video-player, index); read-check bans a cache kept
      by hand (HEAD: 9 sites). Roles' view carries R5 `state`/`decided`; Discover leaves an unread space out NOW without
      remembering it. fx: ctx.cache forgot a failure and read again (made 2); reload probe unchanged (38 / 47).
- [x] 9 Phase 4 (FOUNDATION §7.4): a timer is a lifecycle JOB (background work) or a view's clock (`ctx.every(el,
      ms, fn)`, stops when its element leaves). Header's ⏳ box drawn on change (files.onUploads, studio.onQueue) — no
      3 s / 30 s polls; loader's version check a job ("newest"); tokens' queue a job; apphost, usage, video ×2, theme on
      ctx.every. tools/job-check.mjs (HEAD: 7 files). fx: an upload row → header badge in 2 ms (old: 29.9 s).
- [x] 10 Phase 5: `save` machine (per change: OWED → HELD | REFUSED, forgotten once HELD) moved by storage on every
      write — the lifecycle pill is the ONE place "Saving…"/"Not saved" is said (the loader's own line and the
      `craftworks:save` event are gone); a refusal is reported. `page` machine (BOOTING → RUNNING ⇄ HIDDEN, UPDATING); an
      update waits for no save owed. Found: a merged table's put dropped `user` (a space's saves never said "Saving…"
      unless the click was still live) — carried through now. caption + app-settings on theme.action (the one button
      feedback). fx v747: a refused save → "Not saved: …" in the pill, owed 0, refused 1, page RUNNING.
- [x] 11 Phase 7: the named private loops (§4) — statement's 40 rounds are a step loop (kept), files' rounds a race,
      keep's `tries` a record, keys already on `attempt`; auth's earlier-build ask had TWO retry layers (attempt ×3 inside
      a 6 × 10 s loop) → one `attempt` with the `not-yet` policy, `.catch(fault.absentOnly)`. 19 places showed a fault's
      raw message to people → `ctx.say` (its kind's words); failure-check fails a raw message shown (HEAD: 19). fx v748:
      13 changed pages open with no error.
- [~] 12 Phase 8 (apps in their own frames): DRAWN (421535d) — FOUNDATION §3 says which pages are the OS and which
      are apps; system controls are SYSTEM PICKERS; tools/app-check.mjs ratchet (10 apps not yet framed). NOT BUILT: a
      rich app (wiki, notes, board, blog, forms, media, drive) builds on `md-editor`/`markdown` (files, cards, mentions,
      the publisher dialog — host capabilities a sandboxed frame cannot reach: no host blobs, no host elements). Moving
      them needs a FRAME-SIDE KIT first: rich text, file views and embeds that work inside a frame (data URLs, a host
      bridge for capability calls per the app's declared `uses` — where per-app GRANTS live, an identity-build change).
      Order: the kit + bridge → Vote (smallest: audience picker only) → Forms → Wiki → Notes/Board/Blog → media/Drive.
- HANDOVER 10-15: committed since v392 and not released — 4f6804d e3c7f83 72115b3 c8613f3 01499cd b75248f fb328b9
  df6018d 421535d. The identity build moved (762eb343…, 72115b3): a release asks everyone's PIN once. fx = v748.
- [ ] 13 Phase 8 (frames), the plan (FOUNDATION §3 "THE FRAME RUNTIME"): 8.1 runtime (in-frame loader by hash, ctx,
      host proxies, grants per app) · 8.2 pure splits (item helpers) + name cache · 8.3 pickers (person menu, audience,
      files, publish) · 8.4 apps one at a time, Vote first. Measured: UI code's sync capability calls — space.shown 29,
      directory.nameEl 19, kinds.* ~30, items pure helpers ~25, person.open 6; the rest async (proxied).
      DONE 10-15: 645940a runtime.js (helpers, both hosts) · 5137525 frame runtime (mountCtx, proxies, grants) ·
      3f65f98 trust split · 38bb7d4 menu + gesture · 62ce196 Vote framed (bridge parcel, frame() helpers, fail-closed
      gestures, vote-tally host-side) · c68952c ALL TEN APPS FRAMED (one call path, frame() pure parts, host rules,
      system surfaces, theme.ask, grants computed by app-check). fx v793: 22 framed pages, 7 flows, 25 OS pages.
      efde90d grants narrower (index, storage gone; app-settings host-side); pickers stay in the app (decided).
      fx v794 framed: Drive upload, image post, form made + answered — pass. Untested framed: video upload/encode,
      caption editing, the editions sheet. NEXT: per-call grants (a files picker); release (identity unchanged, no PIN).
      c777e2b framed media (watch time, small videos encoded, captions on id-less videos) · 78a8ab7 grants call by
      call · (this) lists scoped to an app's kinds + two system pickers. PHASE 8 COMPLETE (fx v804: every probe, 0
      calls refused).
      b9f4951 admission: a JOIN from a held account let in again, once per request (the owner's Ivvor's New Space
      deadlock: "in the group already" ×10 in the live health bags). RELEASED v398 (10-16) = b9f4951, tmp/release19,
      identity 762eb343 → 0d016a24 (PIN once). NEXT: watch the owner's join to f5697a in health; the full-recovery
      check; task #37 (group machines); then shared items.
- Live: v397 (ed44f42) — every screen a view, one menu door, side panel, Discover first.
- [x] Found on the way: 72115b3 — a stalled round that goes on let the next person's start a wake-up late (2de2571,
      live v392): 13 people, 52 wake-ups a turn → 39. Not settled on fx (crowded): fx serves ~a dozen accounts.
Done before (4f6804d, kept): key-history walk stops at the first held epoch; key events name their space.

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
- [x] F4 video (old plan line): CMAF segments, renditions (AV1+Opus, H.264+AAC), remux (mp4box) or WebCodecs encode, MediaSource ABR,
      poster + scrub strip, subtitles. — 10-08: SUPERSEDED by "F4 VIDEO PIPELINE" above (built: fMP4 + manifest, lean
      ladder AV1|HEVC + H.264 720/360, WebCodecs/Mediabunny, adaptive player, poster + scrub, subtitles as data).
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
    - [x] (10-05, tests 2/2 + a cap mutant caught) R4a SPACE TABLE READER in the delegate (`delegate/src/table.rs`, a pure step machine like upkeep): writers =
          each member's DEVICE keys (key log → card `nodes` → credentials; a page writes a space under its member key);
          the writers bag (sealed with epoch 0's `bag-writers` key, AES-GCM) narrows whose catalogs are asked; each
          writer's catalog `x<id12>-tables` → table listed (blinded: `blind_name(space_table_key)`) → its feed tail →
          tree blocks (Sealed) → epoch keys (`identity::epoch_secret`) → `feed` merge → rows; `departed` caps. Tested
          against a scripted network with feeds written by the real data/feed crates. NOTE: identity.wasm 1.19→1.58 MB
          and its hash moves with these deps — NOTHING is published until R4e (packages/build holds the new one).
    - [x] (10-05: `delegate/src/write.rs`, `identity::upkeep_sign_space` through the page's fork guard, `data::prepare_rows`;
          test: first write lists bag+catalog then PUTs the feed, a version over another's row, flush past 32, guard; a
          listing mutant caught) R4b WRITER: this member's own feed of a table from the delegate (writers bag + catalog listing first when
          new), rows as feed versions, signed with `space_writer`, sealed with the newest epoch held.
    - [x] (10-05: `delegate/src/rekey.rs`; rank checked against the page's rankFor values; rotate waits for the group to
          move; control without a removal. KNOWN GAP: the page checks each credential's signer against the DID's key log,
          upkeep trusts the MLS roster) R4c DUE + ROTATE in Rust: `acts`/`pub-acts` replayed by `gov` → removals; the group without them; salt
          rotated (upkeep randomness); due rows as `file-keys.js`; the same rank/takeover hash as pages.
    - [x] (10-05: `delegate/src/recode.rs`; test: a 5 MiB file, 3 fragments lost, re-keyed salt 0→1, read whole under
          the new key, old pieces burned with the row's secret — old reference reads nothing; an old-key mutant caught.
          Left to a page: rows with no content hash, indexes deeper than one level) R4d RECODE: a generation per step (GET fragments → decode → encode under the new key → PUT pieces), index and
          root, row changed, old pieces burned; progress `p/<id>` shared with pages; a budget per wake-up.
    - [~] R4e WIRING + LIVE: wake-ups run re-key rounds after admissions (live: rounds read every space, say what is due).
          BLOCKED BY THE NODE (measured 10-05 on fx, node log): a delegate's PutContractRequest of a NEW contract is stored
          on its own node and broadcast only to interested peers — none for a new piece (BROADCAST_NO_TARGETS, giving
          up); every client GET (its own node's page too: no local interest) answers NotFound. Same on 0.2.140/141;
          freenet #5542 open. So the re-keyed rows pointed at pieces nobody could read (big-file3/4 on fx lost that way).
          NOW: `upkeep::NEW_CONTRACTS_REACH_THE_NETWORK = false` — upkeep writes only to contracts already there (the
          salt rotation into an existing feed); a file's new pieces, a new feed, a flush's blocks: left to a page, said
          ("left to a page — its new pieces would stay on this node"). Test + gate mutant. Live: big-file5 left to a page.
          Turn the const on when a delegate PUT reaches the network.
    - [x] (10-05) EARLIER BUILDS' UPKEEP RETIRED: every identity build stays registered and wakes every minute; a build
          the member left acted on its last mandate (live: an older build re-keyed big-file5 after the new one left it to
          a page). On a PIN login the page hands each earlier build (newest first) an Unlock + an EMPTY mandate, up to
          the first that does not hold the PIN (one wrong try at most; a silent build is passed over). Live fx: 7 builds
          retired. Builds from before the fresh start (other Request layout) are not in identity-history: not asked.
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
- [x] 2  PUBLIC PARTICIPATION: policy `anyone` for post/comment/vote per app; an outsider's comment/vote lives in their
      own public tail + a pointer in the post's bag; readers accept it only if the policy at its time allowed it.
      PER-POST AUDIENCE (owner): the author picks public / members for each post, the space+app policy the ceiling
      (path `board/p/<id>`); comments and votes follow the post's audience. PER-POST COMMENT/VOTE (owner): the author also
      sets who may comment and who may vote on that post (anyone / members / admins / owner / nobody), within the space's
      ceiling and never wider than who reads it — e.g. a public post with members-only or admins-only comments.
      ONE IMPLEMENTATION (owner: every app inherits it): policies by path in `roles` (item path `<app>/p/<id>`), enforced
      in `content` (Board, Videos, Audio, Chat, Messages) — Notes and Drive call the same `roles.allows` for their items;
      Chat keeps per-channel policy (`chat/<channel>`).
      10-08 CHECKED in code and LIVE (fy owner, fx outsider, new public space "PP test" #51d676, board read/comment/
      vote = anyone): fx finds the public post in Discover, comments + votes (kept in fx's profile, pointed) → fy sees
      ▲1 💬1. GAP FIXED: a space's post could only say "as the space allows" or "only you" — now also its members /
      admins / owner (audience picker); and an item's own rule is now held WITHIN its space's (`roles.mayWrite`: the
      writer passes both — a post narrows who may, never widens). Live: post with "Its members" → fx sees "Only the
      space's members comment here", no composer, ▲ disabled ("Not yours to vote on").
- [x] 3  SETTINGS: a Settings app in every space (575130b, a4e8c5a); apps' own settings links removed (d0e6aa5).

- Test account on fy (17692, site EckhKsQW…): PIN 731731; words juice bulk shy trend need book heavy curtain old network arrange disagree (made 2026-10-03, outsider tests).

## CHAT AS A REGULAR APP (owner 10-03: "items them"; personal space gets its own Chat; Message/Mail/Contact stay global)
- [x] 1. CHANNELS ARE ITEMS (kind `channel`, domain `chat`) in shared spaces — `conversation.channels(server)` the one
      place: lists channel items (title = name, `meta.cid` = its message table's id, so no message moves), add/rename/
      remove = items submit/edit/remove; made only by who may make channels (as before). A channel's WHO MAY POST = its
      own rule (`meta.write.post`, via `roles.mayWrite`) else the space's Chat policy (path `chat`). Owner/admin node
      MIGRATES the old `channels` table (with each `chat/<cid>` policy → `meta.write.post`), then sets the act
      `config chat.channels = "items"`: readers stop opening the old table once that act is read.
- [x] 2. SETTINGS: Chat section's channel rows edit the items (name, who may post) — no `chat/<id>` path policies.
- [-] 3. PERSONAL CHAT: REMOVED (owner 10-08: "no such need" — a profile shows no chat, nobody to join). Built as
      channel items in the personal space (e9c344b), reverted. Kept from that work: `index.watch` (a bag followed, new
      writers by the node's push) — the spaces' outsiders read by it. Open question to the owner: conversations (direct,
      group) as items of the personal space, as a space's channels are — not built.
      Found on the way (da9de36, kept): Chat in a space without Chat on offered channels with nowhere to live.

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
- [x] 2. `cards` (9929f16: video/audio, note, file looks; embed any item `item:REF`; attachments; pageOf carries place. Board's posts and Caption's rows on `cards.card` — checked 10-05): each KIND's look (domain: text, video, audio, note, file, caption) — card (lists) + full (item page,
      media-view for media). Mixed lists (Discover, a person's space, Drive) render each item by its kind.
- [x] 3. One ACTIONS component (actions.js; Board, comments, Video on it; Save on every kind; Saved tab standard via `where`; Video's broken Saved fixed): vote/like, comment, share, save, hide, remove, edit — ONE may-check (items.mayWriteOn;
      Board's roles.allows copy goes).
- [x] 4. (10-05) One COMPOSER: `publisher.form` makes written items too (`kinds.written()`: text, note — title, the one
      editor, who sees it; from outside a public board: public) beside the media ones; Board's own composer deleted (its
      Create post is the form). Insert → NEW: Post · Note · Image · Video · Audio · Book in any editor (`publisher.dialog`,
      the editor's space and audience), embedded where the caret was. One ITEM PAGE was already shared (Board, media).
      Live fx: a post with a note made from Insert (embedded after its text); a comment with an image made from Insert.
      Note app keeps its in-place note editor (editing, not a second composer); Drive's upload and Chat's channel are their
      apps' own acts (a file into a folder, a channel).
- [x] 5. (10-05) Copies deleted: a space's public description and "read from outside" have one owner
      (`items.publicSpace`, `items.outsideOf`) — Board's descOf/outsideFor, Media's outsideOf, where's saved read,
      conversation, spaces-panel, space-home. `discovering()` is `where`'s; votes() is `actions`' only. Server settings'
      nav "null" (a hosted page has no Close) fixed.
Settings app in every space: DONE 10-04 (575130b, a4e8c5a — Account is the personal space's Settings; a space's
settings a page; a tile on every Home; no header Account link).
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

## Item observability: SIGNALS, reactions, labels, tags, NSFW (owner 10-05)
Owner: "the whole item observability as one implementation … save, share … tags … composable and inherited like
policy"; "save counted, shown like TikTok"; "tags to all kinds"; "react only on chat — extend to all kinds"; "labels
for individual sorting, tags for global categorization, a setting to view NSFW or not".
- [x] S1 `signals` catalog (packages/signals.js): vote, view, save, share, tag, react — mark, value (once/updown/many),
      policy ACTION (inherited as policy: view/save/share by who may READ, vote/react/tag by who may vote), counted,
      point, activity. ONE writer `items.signal`, ONE tally (`counts`, `signaled`), content's check by signal action.
- [x] S2 counts on the buttons (TikTok): ☆ Save N, ↗ Share N, 👁 N; card line " · 3 views · 1 save"; Popular sort +
      Views reorder; Hot/Rising from the catalog's activity signals. Live: ★ Saved 1, ↗ Share 1, 👁 1 (B's view from Discover, outside).
- [x] S3 REACT on every kind: the react signal = any emoji (Chat's rows as they are); chips with counts + picker in
      the one action bar; Chat reads its emoji list from `signals`. Live: 👍 1 kept across a reload.
- [x] S4 LABELS on every kind (private, one account list): one key `item:<ref>` (label-menu), Notes' `notes:` rows
      moved once; 🏷 Label in the action bar; chips on every card look.
- [x] S5 TAGS (public, global): author's `meta.tags` set in the one composer (written + media) and on edit; #chips on
      cards and the item page; `#/tag/<t>` lists every kind with it (feed + Discover); `items.list({ tag })`.
- [x] S6 NSFW: `nsfw` tag + "Adult content" box in the composer; Settings → "Show NSFW content" (edge.prefs, every
      device, off by default); lists hide NSFW (own always shown); item page gate "Show anyway"; 🔞 badge when shown.
- [x] S7 LOGIN KEYS fix: a member moved from an OLDER build (a newer one slow) lost later epochs (B: "no key for epoch
      17"); the retire walk now gathers EPOCH keys only (never group state) from every build holding the PIN.
- [x] S8 Live: A posts in History test; B (17692) views, reacts, saves, shares, labels; A sees counts; tag page;
      NSFW hidden/shown by the setting. Then review, commit, publish to all sites.
      Live 10-05 (fx A, fy B): A — 👍 1, ★ Saved 1, ↗ Share 1, label "watch-later" on a post; tags cooking/travel → the
      card's #chips → `#/tag/cooking` lists it; A's own NSFW post shown to A. B (outside, Discover) — setting off: the
      adult post left out, opening it shows the gate, Show anyway opens it; Settings → Content on → listed, 🔞 #spicy.
      A sees B's view (· 1 view, 👁 1) after a reload. S7: B's walk gathered keys from 3 builds. B's remaining "no key
      for epoch N" is CORRECT (A removed B from those spaces in the re-key tests); B's spaces list still names spaces it
      was removed from — open, separate.
- [x] S9 (10-05) HIDE and FLAG, the same six everywhere: hide (private, everywhere) / flag (public list, Discover) a
      post, an author or a space — ⋯ on every item (`actions.more`), a person's card (Hide everything they post · Flag
      author · Block), a space's card (Hide space · Flag space). One API: `moderation.lists` isHidden/setHidden,
      isFlagged/setFlagged, hiddenList. Settings → Moderation lists both (Hidden by you · Flagged) with undo.
      Live fx: Hide author → their 5 Discover posts gone; Settings → Moderation → Show again → back.
- [x] S10 (10-05) PERMISSIONS PER SIGNAL: react, save, share, tag are policy actions of their own (gov ACTIONS 7 → 11,
      test + mutant; identity 5dea06e6, PIN once), each unset = its parent (save, share → read; react, tag → vote) at
      item, domain and space level (`roles`). Every settings section (space, each app, Everything you post) lists them
      from the catalog with "Default (as who may …)"; the composer's "Who may interact" + Customize each. React hidden
      where not allowed. Live: Card test Board "Who may react: Admins" → B (outside) has no React on its posts,
      React still on profile posts.
- [x] S11 (10-05) ONE CATALOG for every interaction's permission: comment joins `signals` (kept as an item, declared
      for its rule), comment and vote declare `openWhereRead`; settings sections, the composer's Customize and roles'
      public-participation default all read `signals.interactions()` — nothing per interaction written elsewhere. gov
      test reads packages/signals.js: every action it names must be a gov ACTION (control: a renamed action fails).
      Live fx: Card test settings list comment · vote · save · share · react · tag in one order with their defaults.
- [x] S12 (10-05) Loose ends: spaces this account was removed from leave its list (upkeep: `keys.group().removed()` →
      `space.forget`; live fy: B's 6 removed spaces gone — each checked: A's member list has only A); NSFW gated in
      embeds and filtered in Saved (`items.visible`); Images wall shows tags; OTHERS' TAGS (＃ Tag, the tag signal: chips
      "#greeting 1", the tag page finds them); a Discover space read that fails is said, never an empty list.
      MEASURED: outsiders' views reach an open page LIVE (no reload: "· 1 view" within ~30 s, same document) — the
      earlier "needs a reload" was B's first pointer to the space.
- [x] ARCHITECTURE plan table brought to 10-05: 5–8 done, 9 observability done / privacy not, 11 Global analytics
      (proposed: tallies, qualified counts, resource quotas, settled periods — awaiting the owner's two decisions),
      12 Compute, 13 App domains, 14 Templates.

## HISTORY in the storage layer (owner 10-05: "records on prolly tree… history"; baseline 7 days and 20 versions;
## premium a higher ceiling later; personal and shared spaces the same; every kind)
Today a tail row is replaced in place and a flush replaces the root: old versions sit in unreferenced blocks, unkept.
- [x] H1 data crate (new fns only — the identity delegate links this crate; its hash must stay 5dea06e6): `root()`,
      `value_at(root, key)` (prolly::read::get through the same Step/Need loop), `pending_rows_without(key)`,
      `asset_at(root)` for keep.
- [x] H2 ROOT LOG: after each flush, the replaced root (+ when) appended to a reserved row of the table, pruned to
      7 days; flush-when-quiet ignores that row (no flush loop). A version = the state after a quiet period (edits
      within one are one version) — honest grain without a flush per keystroke.
- [x] H3 KEEP also keeps the blocks every logged root reaches, until it ages out.
- [x] H4 items.history(ref): each member's feed walked root by root → [{ at, by (the feed's writer), value }],
      consecutive duplicates folded, newest 20; History view (item page, note) with Restore.
- [x] H5 Retention a space setting (5d49af7) (≤ the plan's ceiling: baseline 7 d / 20 v); premium ceiling with payments.
Known limit: a table RESEALED (an epoch moved after a removal) starts its history again — older trees sit under the
old key's addresses.

## 2026-10-05 session
- [x] History H1–H4 live (2ca407d): flush logs replaced root (7 d), keep keeps logged roots' blocks, items.history, 🕘 History + Restore. H5 (retention as a space setting) open. Not yet tested live.
- [x] Key catch-up on load: a member behind the owner's branch asks every member; any member on the branch answers (5e8eccb, 8198d25). Awaiting TinyIvvor's test.
- [x] Publisher confirms pieces served (re-put if not); loader never replays a failed cached piece (d2fe15c). Test nodes run with their own HOME (shared webapp_cache clobbering).
- [x] Mentions one way (person:DID links → card; inbox notices checked against the item), shared links open from any address/custom domain, header search + 🔔 + @ (9a663d8).
- [x] Live 10-07 22:30 (fx→fy): a mention notices fy (🔔 1, @ 1), the notice opens the conversation at the message, highlighted; a note's History shows the version before the edit with Restore. (Typed `[..](person:..)` is escaped by the composer — mentions come from the @ list.)
- [ ] F5b R4 delegate re-keys: BUILT (R4a–d); R4e BLOCKED BY THE NODE (freenet #5542: a delegate's PUT of a new contract stays local). Nothing to build until the node fixes it; flip `upkeep::NEW_CONTRACTS_REACH_THE_NETWORK` then.
- [x] Your node log: contracts HQk7zGRQ…/J4bh5TcQ… (10-07, measured): J4bh gone; HQk7 a public contract relayed by the node, NOT ours (68 "Invalid update" deltas from peers in 12 h, resynced) — nothing of ours to fix.

## Owner decisions 2026-10-06
- Notes votable: YES. Every kind the same capabilities (votes, comments…): DONE in kinds PARTS (uncommitted with this batch).
- Drive files become items, the same row as every item: YES — how an app composes/views is its theme's. TODO (after style work).
- Key/read chips: DEBUG ONLY — behind Settings → Content "Show diagnostics" (pref `diagnostics`).
- Analytics/rewards: PAID DEMAND — users pay a monthly subscription; distributed among content owners and bandwidth providers. Design doc TODO.
- Live checks: History seen; restore, notifications, mentions not checked by owner — owner flags issues separately.

## Sequence (owner: in order)
- [x] 2 Style: outside overrides moved into their components; build check (tools/style-check.mjs: no cross-package styling,
      no duplicate class names)
- [x] 3 Cascade layers (10-06): `@layer base, apps, components, templates` declared once in the theme; every package's CSS
      in its layer (28 style blocks + 13 in-template <style>); `body` prefixes dropped. The boot page (wrapper/index.html)
      in a lowest `boot` layer — unlayered, its `body { max-width: 900px }` had capped the whole app.
- [x] 4 Strip each app's own buttons/headings/widths → theme parts; screenshot each app at phone width — 10-07: theme part `.cw-flat`, used by the header (19 other packages' flat-button rules still their own: each tuned); PHONE (390 px) checked: Home, Board, Chat, Note, Drive, Mail, Settings, Video. Fixed: header on two rows (app actions + search below, the header sets --cw-bar to its height), nothing wraps or runs off; Chat one pane with Back. 10-08: Usage, Contact, Image, Audio, Book, Paste, Caption seen at 390 px — bodies fit; the header's first row wrapped where the app had an action (Paste: "@" on a row of its own) and its tabs ran under the edge unseen (Image "Di…", Usage's Reward): the app's action now on the second row, the tabs faded at the edge while more are past it. The 19 packages' flat-button rules KEPT (owner 10-08: no change where nothing improves): each sets its own size, colour, shape; .cw-flat on them would add a hover some do without.
- [x] 5 Remaining live checks (10-07, see the 10-05 session's list)
- [x] 6 Identify node-log contracts HQk7…/J4bh… (10-07: J4bh gone; HQk7 is a public contract the owner's node relays to ~40 peers — not ours; 68 'Invalid update' deltas from other peers in 12 h, each answered by the node's own ResyncRequest)
- [x] Catch-up made SAFE and back on (10-06, 241d8df): asked after 10 min behind in a page (page memory: the app frame
      has no localStorage), the ask carries epoch+branch; answered only when confirmed behind (ask + device announcements).
- [x] Drive files as items (already items in drive-store); 10-06: feed tiles open the file's page (bar + Open/Move…).
- [x] FEEDS without the action row (owner 10-06): note/video/audio/image/book/drive tiles; the row on the item's page
      (a note's: its editor).
- [x] Rewards design: docs/REWARDS.md (user-centric split of each subscriber's fee; keepers by possession proofs;
      treasury pass + monthly ledger; shadow mode first).
- [x] PASTE app (10-06): kind `paste` (note family; meta language/expires/unlisted), #/paste composer + your pastes,
      item page with `highlight` (per-family tokenizer, text in spans), raw view + download; unlisted = public, never in
      Discover (profile submit + space sync both skip). fx: rust paste coloured, raw exact, Discover 0.

## Video / storage hardening (10-06)
- Upload 6 generations in flight (a402b1a: 56 KB/s → link speed); playback chunks at once, 4 fragments ahead, raced
  generations, 2 s hedge; ONE cap on gets (node.slot: 32, urgent first, background ≤16); late answers taken; a fragment
  late/failed raced from lower renditions, a moment none can give skipped and jumped (d1fce88).
- keep reads and REPAIRS (deterministic fragments re-made; tested byte-identical), never re-puts what answers; readers
  repair files (f6e1ab1) and tables (blocks: a block "not there" rebuilt from its group is put back).
- Measured: from B 428/433 pieces of a 360p rendition answer in 1st try (p50 0.9 s), the "unanswered" were slow gets;
  the 3 "missing" fragments of the probe were never listed (upload replaced them with higher numbers).
- Player chrome on the picture (timeline + frames, settings: quality/speed, CC, sound, full screen); FreenetTube demo
  at DkG6Jqkc… (demo/tube) with the contracts tree live.

## Rewards track (10-06/07) — docs/REWARDS.md is the design
- [x] Step 1 usage record: Usage app `#/usage` (per month/item: time, data, opens; own table `usage`). One unit for
      every kind: time played, else time on the item's page (shown, focused, input in last 2 min). 7f22402, a11f34a.
- [x] Time/data as PUBLIC counts beside views (signal kind `sum`: each person's running total; owner: as view count
      for now, confidential roll-up later — signed per-person entries show who spent how long). 7f22402.
- [x] Step 2 keep what you watched: `keep.watched` (from `usage.track` file refs), table `kept`, re-read a day apart
      via files.keep, limit in Usage app (default 512 MB, oldest dropped). Tested fx: 4 files noted, re-read 12/12 and
      5/5 whole within 4 min. 3e826cb. Measured basis: re-read piece survives 94 evictions, control evicted (6c618c7);
      subscriptions ruled out (2-minute renewal per piece).
- [x] Step 3 keepers bag (claims, monthly, public) + read-only keeper pass. 2379ad9.
- [x] FINAL BY THE DATE, by anyone (owner 10-08: "only the contributor can finalize its own account … we need everyone
      to be able to finalize"): a day final once over + a grace day — the contributor's last row stands; a covered day with
      no row → its share to the network; the contributor's page writes only within the grace. Contributors listed in every
      month's bag their contribution spans when it is MADE; the card keeps contributions not yet settled. No usage made
      public. fy, page clock set to 10-11 06:00 (fx untouched since today): 10-08 "final" (its running row as written),
      10-09 "final" 7.83333333 all to network, 10-10 still open. CONTROL (HEAD, same clock): 10-08 "running", 10-09 absent.
- TOKEN STANDARD (owner 10-08: own token on Freenet, blockchain BEHAVIOUR not its method; no authority/sequencer —
  memory craftworks-token-behavior-not-blockchain). Step 1 SPIKE `tools/related-spike` (a lock and a fill, one code):
  fx/fy private 2-node net, freenet 0.2.142 — A fill citing a missing lock: REFUSED "missing related contract" (60 ms);
  B lock LOCK on fx → fill on fy: ACCEPTED (72 ms); C lock NOPE → fill: REFUSED "invalid put" (15 ms); D both on fy:
  ACCEPTED (60 ms). So validation reads another contract's state and decides by it. FETCH over the network, shown too:
  fy STOPPED, lock written on fx alone (a lone gateway never acked its PUT: 90 s), fy restarted (no trace of the lock),
  then the fill on fy: the lock "initial state installed" at 10:28:52.544 — the fill's validation fetched it from fx —
  and the fill ACCEPTED, 100 ms end to end. Still to measure: the same on the real network (latency, a far lock). Source
  limits (0.2.135): depth 1, ≤10 related, 10 s. Next: docs/TOKEN.md for review.
- TOKEN STEP 3 (10-08): contract core BUILT — `contracts-src/token` (one code: definition + account chain), `genesis`/
  `send`/`receive`/`burn`, carried balances, cell-wise merge (lowest hash between different writers; two writer entries
  freeze the chain, earliest fork wins), receives among the last 9 entries checked against their senders (+ the
  definition = 10 related), work (16 bits) on receives. 18 tests; mutants: 8 killed, 1 showed a dead check (post-fork
  sends are structurally absent) → removed. Live fx/fy: definition ✓, A genesis+send ✓, B's receive on fy ✓ (75 ms,
  fetched def + A), double receive REFUSED, receive of a missing send REFUSED, two writer sends at seq 2 via fx and
  fy → both nodes the SAME frozen chain in ≤5 s. Deviation: no accumulator yet (whole chain scanned; needed once
  pruning exists). Next: step 4 — the beacon contract, then the real network.
- TOKEN STEP 4 (10-08): BEACON BUILT — `contracts-src/beacon`: sequential blake3 steps, points published in the
  fraud window (F steps), sampled re-run of the newest step, fraud proofs (one segment re-run), merge from the UNION
  of known steps with bounded candidates (lowest heights first; orphans wait). Found while testing: a merge that
  discards the losing step is order-dependent (a proof later removes the fake AND the honest step it beat) → union;
  a window-based candidate drop lets a racing fake push the honest step out → lowest heights; dropping orphans
  loses honest steps built on a fake's true output → kept. 12 tests, 7 mutants killed. Live fx/fy: extension ✓,
  fake past the sample ✓ (first try), proof → both nodes back to the honest chain in ≤5 s; REAL step K=1.07e9:
  61.5 s, 65 KB, accepted in 1.2 s. Not yet: entropy mix. Next: real network (finality margin) — or step 5.
- TOKEN REAL NETWORK (10-08): second node started temporarily on B (lock held, removed after); 5 rounds: related
  fetch + accept 4.4/62.3/11.3/8.1/8.6 s (62 s unexplained), equivocation converged on both nodes 4.9–5.8 s. Margin
  set to 10 beacon steps. A fresh node on this Mac (phone hotspot, 172.20.10.x) made no connection in 3 min while the
  owner's node held 37 — cause not measured. Next: step 5 (allow/spend, offer/fill/lock/claim/refund).
- TOKEN STEP 5a (10-08): ALLOWANCES — `allow(spender, limit)` (owner; replaces, 0 revokes), `spend(to, amount)` signed by
  the SPENDER (Auth::Delegated), within allowance and balance; receivable like a send; an owner entry and a spender
  entry at one seq settle by lowest hash (never a freeze). 25 tests; 5 mutants killed. Live fx/fy: allow 200 → spend
  150 via fy → received via fx; spend 51 (50 left) REFUSED. Allowances scanned from the chain (with the accumulator:
  carried once pruning exists). Next 5b: locks/offers/fills/claims/refunds — needs APPEND rules (time vs the beacon).
- TOKEN STEP 5b (10-08): SWAPS + APPEND RULES built and live (see TOKEN §9 step 5). Found while building: a claim
  needs its fill's token carried (one related round: the lock and the fill read at once); update_state needs the
  definition and the beacon in one round → the network's one beacon in every chain's params. Spike first: update_state
  related round trip measured (guard: LOCK taken, NOPE refused). Next: multisig, ids, vesting — or wallet verification
  (step 6, required before value).
- TOKEN STEP 6 (10-08): wallet verification built + live (TOKEN §9 step 6). Owner: "if zk can solve it, why a half
  measure?" → the fix belongs in the contract: recursive validity proofs. Next: zk spike (prove + verify one transfer
  on our rules; proving time native/laptop, proof size, verify time in contract WASM under the 5 s budget).
  zk then SHELVED on cost (TOKEN §9 step 6): the receiving wallet is the gate.
- TOKEN FOUNDATION list (owner 10-08): [x] 1 prune cut on prolly (writer + contract + wallet across cuts) — BUILT +
  LIVE 10-09 (TOKEN §2, §9 step 7); [x] 2 chain owner = DID via idlog — BUILT + LIVE 10-09 (TOKEN §1, §9 step 8: Writer entries with the account's node credential, rotation revokes, recovery outranks a stolen device); entries name the key seq);
  [x] 3 mint + supply bag + authority chain + freeze — BUILT + LIVE 10-09 (TOKEN §1a, §9 step 9):
      [x] 3a key sets (k-of-n) + the AUTHORITY CHAIN (contract kind 3; changes signed by the set they replace; dropped
          for good; an update's new changes stamped within the margin)
      [x] 3b MINT entry (anyone appends, with work; once each by its nonce, in the received set); Definition.mint /
          .freeze (none = fixed supply / never frozen)
      [x] 3c the SUPPLY SHARD (kind 4, per EPOCH 4096 heights; only mints signed by the mint set of their stamp; a Mint
          entry needs its mint already there). Found building it: a replaced mint key could PUT a never-made past
          shard from scratch (no "now" on a first PUT) → every mint change SEALS the shards since the last one
      [x] 3d FREEZE LIST (kind 5; records per account, later seq wins); the append rule refuses a frozen owner's new
          debits. Measured live: the node takes ONE related round only ("depth>1 not supported") → every token has
          an authority chain and freeze list made with its definition; a debit reads the list in its one round
      [x] 3e live fx/fy (`token-live mint`): 14 steps as designed
      [x] 4 multisig owners — BUILT + LIVE 10-09 (TOKEN §1, §9 step 10): Signer = Device | Set; Members entries;
          Auth::Multi; no key log for a set's chain
      [x] 5 ids (NFT/1155) + royalties — BUILT + LIVE 10-09 (TOKEN §1b, §9 step 11): a chain per (token, item,
          owner), Tok refs; unique items made by their mint; lock fee + fill royalty rule + creator claim. Deviation
          from the old §1b text (one chain per owner, amounts per id): a 1155 batch is one send per item, not atomic
      [ ] STORAGE PROOFS (before 6; TOKEN §7a) — in parts:
          [x] sdk/ in repo (d8daf2f): outside code as path deps; devices + token re-keyed, identity new hash
          [x] wire: node-self (location, hosted ids+sizes) + connected peers (a58d81f); live fx/fy ~210 ms
          [x] the core's location hash ported and matched live (fx 0.596280, fy 0.487930)
          [x] standing crate (2ee3673): ring_of_addr (core's own, live-matched), arc, Attest/located/Bind, Holdings,
              challenge, Answer, check — 6 tests, mutants killed (2 equivalent)
          [x] page-io build fix (5376efd) — a58d81f broke the workspace build (lesson: build --workspace before commit)
          [ ] keeper loop in the page (keep.js): attest peers, bind, answer challenges each beacon tick
          [x] standing records (594fc06) + seal/16-pick replica answers (sealing measured 243 ms/256 KiB native)
          DECIDED 10-09 (owner): witness weight = EARNED STAKE — keeper rewards land locked as witness stake, grow with
              time, burned on equivocation; storage proofs pay keepers (weak guarantee fine there). Order: keeper
              payouts (REWARDS) → reward locks as stake → witness lot by stake.
          [ ] standing per location (ramp), read by witnesses
      FOUNDATION COMPLETE 10-09 (owner: "we just need the foundation build"). ROLLOUT items, not foundation — build
      when the token rolls out: witnesses run by page/delegate (drawn, re-check, sign); the app showing and verifying
      payments; the network beacon extended by open pages; the beacon's ENTROPY MIX (outputs are a hash chain from a
      fixed seed — computable ahead by a fast machine, so committees could be known early; must land before value moves);
      equivocation watchers appending Slash; a record of past beacon outputs (old lots, >600 steps); the keeper loop,
      standing contract and reward payouts; refusing far back-dated entries; pools, lending (these and the entropy mix / output history / equivocation
      detection / standing contract are contract work to build now — owner 10-09; only the page/delegate RUNNING them is
      rollout).
      ECONOMICS (owner decides, later): supply, initial allocation and its vesting, keeper reward size and source,
      reward→stake lock, fees.
      [x] 9 BEACON MIX + LOT DELAY — BUILT 10-09: each step carries its extender's fresh mix (start(prev, height, mix));
          the contract always re-runs the newest step's first segment (a lied mix refused at once); tools extend with
          fresh entropy. The witness lot is drawn at stamp + margin + 2 (`lot_of(stamp, margin)`, margin from the
          entry's own token: `Source.definition`), so a writer cannot choose its committee by stamp or delay. Beacon 13
          tests, 4 mutants killed; token 95 tests, 3 mutants killed. Residual (recorded in TOKEN §5a): parallel mix
          grinding by an extender, a faster core's lead. beacon.wasm 92300b2a, token.wasm 65f67643.
          LIVE fx/fy: beacon-live fake caught + honest step back on both nodes; token-live witness on these contracts:
          every honest entry 16/16, the buried fake and its payment refused by all, 3's wallet Ok(16) in 34 ms, burn ACCEPTED.
      [ ] 18 APPS OUT OF THE TOKEN (owner 10-09: "are not swap and lending supposed to be separate contracts? in
          solana it is… it is not the underlying token foundation" → "refactor and don't build lending"). The token
          keeps only what a token is; apps are their own contracts over ONE generic ESCROW:
          [x] 18a ESCROW in the token (`escrow.rs`): `Escrow { program, terms, amount }` (owner-signed debit), `Unescrow`
              (its refund, once, as the program's verdict says) and `Payout` (a verdict line, once, to its recipient),
              both checked against the program's state read in the same round (`Extra.program`; `programs_from`). Every
              program's state begins with a standard SECTION (escrows, refunds, pays) whose rule — per token, refunds +
              payouts within what was escrowed — any reader checks; witnesses also check every listed escrow is a real,
              ACCEPTED escrow to THAT program of that amount (`wallet::program_backed`). No cut over an open escrow.
              113 tests; 16 mutants killed (1 after an other-program case). Live with 18b (the first program).
          [x] 18b POOLS as their own contract (`contracts-src/pool`, pool.wasm f289a5b7): one contract per batch; state =
              the standard section (orders as escrows, listed with their TERMS hash) + the orders; no verdict before the
              settle height (beacon read), exactly the clearing after; orders merge until settle, nothing after; two
              settled copies → more orders. The section now carries each escrow's terms hash, and witnesses check it
              against the escrow's own terms (a program cannot list anyone's escrow on terms they did not sign). Pool 2
              tests, 10 mutants killed; token 113. LIVE fx/fy (`token-live pool`): escrows 300 A / 500 B; batch listed;
              a verdict before settle REFUSED, at settle ACCEPTED (250 A for 500 B at 2); a release claiming 10 more
              REFUSED; all four releases/payouts ACCEPTED; a second payout REFUSED; 1: 750 A / 500 B, 2: 250 A / 500 B.
              (First live run refused everything: I had deployed a token wasm built before the terms-hash change —
              the local check on the node's bytes passed, which pointed at the deployed code. Rebuild before every run.)
          [x] 18c SWAPS as their own contracts: an order book = a pool batch (one maker, many takers — fair, no queue
              jumping); a DIRECT DEAL between two parties = `contracts-src/swap` (swap.wasm e61650df): both escrow → the
              verdict CROSSES them; one alone → refunded from deadline + GRACE; a verdict once taken is NEVER replaced
              (a refund paid out then a crossing would pay out of an escrow already gone — caught while writing it).
              2 tests; 9 mutants killed (1 after a canonical-order case). LIVE fx/fy (`token-live deal`): deal 1
              crossed (1 got 500 B, 2 got 300 A), all four releases/payouts ACCEPTED; deal 2: a refund before the
              cutoff REFUSED, at it ACCEPTED, 1's 300 A back; 1: 700 A / 500 B, 2: 300 A / 500 B.
          [x] 18d the app-specific bodies REMOVED from the token (Order/Settle/Bought, Offer/Lock/Fill/Claim/Refund; the
              pool/registry kinds; clear.rs → the pool crate). Found doing it: an app that ignored a royalty would
              STRAND an item (the token refuses the item's payout, the seller's payment still goes) — so the swap and
              the pool read both tokens' definitions (related) and pay the creator's fee out of the seller's proceeds
              (`escrow::royalty_fee`, rounded up, one copy); the token's generic rule no longer counts the sold token
              coming back to a seller as proceeds. token.wasm bc0a0ccb, pool.wasm e7552af4, swap.wasm a59005df.
              Token 92 tests, swap 3, pool 9; mutants on the royalty rule: 4 killed, 1 equivalent survivor (the
              creator's exclusion from sellers — its proceeds count as its own fee). LIVE fx/fy: nft (sale through a
              deal: crossing without the fee REFUSED, with it ACCEPTED; item→T, 95 B→M, 5 B→creator; outsider REFUSED),
              deal and pool re-run unchanged; regression modes basic/allow/vest/cut/witness re-run (see commit).
          NOT building lending (owner).
      [x] 19 EDITIONS (TOKEN §1c) — DONE 10-10 (owner: collection → editions → copies; per-edition royalty). An edition
          is a definition (its own royalty, so swap/pool pay it unchanged) + cap, collection, item, version;
          Ids::Numbered (copy n = numbered_item); capped tokens mint by NUMBER into records of 1,024 made on demand
          (supply::record_of; owner: minting on demand must scale past 1M — first built as one shard for all time,
          capped at 1,024 mints, replaced the same day); admitted in stamp order; a mint change's FRONTIER record
          sealed, none past it back-dated; Body::CappedMint. Item authorship checked app-side (the
          token cannot read a feed's credentials). token.wasm 18710494, pool 8fb6d07c, swap d7ab3996 (rebuilt: they
          decode definitions). 94/9/3 tests; 16 mutants killed. LIVE fx/fy `editions` as in TOKEN step 16.
          token.wasm/pool/swap rebuilt again in 20 (below).
      [x] 20 HOLDERS' ROYALTY (TOKEN §1c) — DONE 10-10 (owner: the seller's own copy gets no cut). Royalty recipient
          HOLDERS (numbered, cap ≥ 2); apps pay the fee unchanged; each other copy's holder claims fee/(cap−1) by
          Body::HolderPayout (copy chain cited, edition definition read by id, once per copy+fee, sold copies none);
          witnesses: program backed + the copy chain's last entry accepted. No escrow-to-program change was needed: the
          fee stays in the sale program and the holders claim straight from its section. token.wasm 73f93a2c, pool
          8220e0a2, swap fcce67fa. 95 tests; 10 mutants killed. LIVE fx/fy `holders` as in TOKEN step 17.
          NOT BUILT: fractional ownership (not ordered).
      [x] 21 ESCROW RECLAIM (TOKEN §4.5) — DONE 10-10: Escrow.until + Body::Reclaim; chain checks time (timed) and
          open escrow only; witnesses check the program did not take it, and refuse programs listing a reclaimed
          escrow (a node refuses updates naming a missing related contract — measured). token 413edcab, pool 15905a43,
          swap fcce67fa. 96 tests. Collections: NO verified-collection mechanism (owner 10-10: the collection is a
          Craftworks item, its metadata inherited, ownership checked app-side) — documented in §1c.
      [x] 22 NON-TRANSFERABLE (soulbound) — DONE 10-10: Definition.soulbound; no send/spend/allow/escrow/vest; burn ok.
          token d9fc2478, pool 880cdd3a, swap add7796e. 97 tests; 5 mutants killed. LIVE fx/fy `soulbound`.
      [x] 23 TRANSFER FEES — DONE 10-10: Definition.transfer_fee (bps, most, collector); paid ON TOP by the sender on
          send/spend/escrow/vest (receivers and program payouts whole); Body::FeeClaim by the collector, once, after
          the margin. token 3f2034f1, pool 03c48c2b, swap 24f8e3f8. 98 tests; 13 mutants killed. LIVE fx/fy `fees`.
          Foundation list from the 10-10 audit now done (reclaim, soulbound, fees; collections = Craftworks items).
      [x] 24 PAUSE (whole-token freeze) — DONE 10-10 (owner: "the last foundation"): freeze::EVERYONE in the freeze
          list. token 33d859a2 (pool/swap unchanged). 100 tests. LIVE fx/fy `pause`.
          Foundation COMPLETE per the 10-10 audit. Open only: confidential amounts (big, not asked).
      [x] 25 AUDIT 10-10 — four parallel reviewers; ~25 findings verified; fixed: authority extend-only, payout re-take
          after cut, holders share (sale-bound, holder at sale), one verdict per program (release names verdict,
          payouts gated), pool verdict kept, supply held-mints + every record sealed, witness freeze + taken place,
          finality on overrides, small rules (reclaim time, definition checks, HOLDERS chain, debit nonce, related cap,
          swap pays-nothing). Open residuals listed in TOKEN step 22.
      [x] 26 AUDIT GAPS (same day): witness current-key rule, spend within margin, cut history for payout witnesses,
          contract checks cut trees from seq 0. Then payouts name their verdict + Body::FalseVerdict (owner: content
          hash makes it unframeable). Then mint credits carry their signed mint + Body::FalseMint, and freeze changes
          seal the freeze list (a back-dated pause refused). Audit: nothing open in the rules.
      [ ] 27 WALLET CORE (owner agreed 10-10: before the shop) — user-side token logic on the existing account and
          device keys; library + fx/fy, no release. Crate `wallet/` (main workspace), TOKEN §7c.
          [x] W1 DEBITS: Wallet::debit (writer handover, fee on top, place-derived nonce, stamp never below, checked by
              the token's walk). 2 tests; 4 mutants killed (1 after a lagging-beacon case).
          [x] W2 COLLECTING: Wallet::collect — receives, vesting steps, fee claims, releases (verdict-named), payouts,
              reclaims, mints (numbered copies too), stake releases; once final, once each; 5 tests (each output through
              the token's check); 10 mutants (1 equivalent check removed, 1 killed after a pending-verdict case).
              token.wasm 3fab0d85 (unbonding_of made pub).
          [x] W3 CUTS: Wallet::cut + Held.blocks/whole; collect across cuts (whole-history dedup, absence proofs,
              sender-history presence proofs). 6 tests; 5 mutants (1 equivalent block merge removed).
          [x] W4: Wallet::genesis, is_value (witness acceptance); token-live `wallet` mode LIVE fx/fy end to end.
              7 tests; mutants killed.
      [x] 27 WALLET CORE — DONE 10-10 (W1–W4). Next: 28 SHOP over it.
      [x] 27b SOLD ON DEMAND — DONE 10-10 (owner: "allow mint without owner for on demand"): `Definition.sale`, paid
          mints (no signature; the payer's chain read by the record), records of 7, `CappedMint.record`, `FalseMint`
          for an unpaid copy, `Wallet::buy` (W5); LIVE `token-live ondemand`. Spill (10-10): a payment for a record a race filled buys a copy in the next
          (`spill_ok`, `wallet::remint`); live ACCEPTED/REFUSED as expected.
      [ ] 29 $CRAFT KEEPER ISSUANCE + REPUTATION (owner 10-10: "lets build it. and method to monitor the
          distribution"; design TOKEN §7d). Phases:
          [x] K1 DESIGN — TOKEN §7d + §6 `keepers` row: reputation = gate, proven keeping = amount (flat per account per
              epoch, halving), KeeperMint in the keeper's own chain citing its standing contract, locked as stake,
              witnesses check the gate, the epoch issuance bag + monitor page.
          ORDER (owner 10-10, after the design review — TOKEN §7d "REVISED", "Gaps closed"):
          [ ] V1 VOUCH REGISTRY + EPOCH STATEMENT: vouches registered per epoch; reputation from the epoch-start
              snapshot (ramp, decay); the statement (reputation, keeper shares, WREN allowance) — one number set.
              [x] V1a `reputation_at(roots, vouches, height)`: the SNAPSHOT (later vouches unseen) and the RAMP (¼ a
                  standing epoch, full after 4) — measured 31.9 / 63.8 / 95.6 / 127.5 (×10⁻³) by epoch.
              [x] V1b-1 the SIGNED VOUCH: `reputation::item` (from, to, at, on; signed by a device of `from` under
                  identity's VOUCH_DOMAIN — `TokenSign` now signs SIGN_DOMAINS = token entries + vouches), verified
                  against the voucher's key log; `standing(items, now)` folds items to the vouches standing (latest
                  per pair; a re-vouch keeps its first height). 12 tests; 9 mutants (1 explained: the account check
                  is redundant with the key-log check).
              [x] V1b-2 THE REGISTRY, LIVE fx 10-10: the page signs a vouch (TokenSign, vouch domain), stores the signed
                  item as its public vouch row, drops it into the epoch's registry bag (`index.bag`, the existing bag
                  contract); reputation crawls signed items, RELAYS what a registry lacks (anyone relays), computes
                  `reputation_from_items` as of the beacon height (each item verified by its key log). Measured on fx:
                  A's panel "Vouched ✓"; `token-ops registry` epoch 0 → "45caba3e → 1ead752b at 407 vouch, 1 item".
                  The old unsigned `reputation_compute` removed; `account_device` shared by editions and vouches.
              [x] V1b-3a THE STATEMENT, computed: `reputation::statement` — `shares` (THE one pot split: by weight, capped,
                  rounded down, the rest never paid — keeper issuance, witness slice, WREN allowance), `compute` (keepers
                  past the gate who kept; the allowance to everyone past the gate; never the roots), its TREE through
                  `engine::repair::build_tree` (moved there from the token's cut: one way to build a whole tree), a line
                  provable from the root. prolly now implements `Blocks` for a plain block map (the token's private
                  `Store` wrapper removed). 15 reputation tests; 6 mutants killed. Measured: one account's weight split
                  over 100 fakes pays 1,000 against 1,000.
              [x] V1b-3b PUBLISHED, LIVE fx 10-10: `packages/statement.js` (started by the footer: every open page
                  publishes the current epoch's statement if nobody has) reads every vouch registry up to the epoch,
                  `core.statement_build` (reputation as of the epoch's start; `reputation::GATE` the one gate), puts the
                  tree's blocks through the BLOCKS door (`Core::public_block_put`, sharing `block_put` with tables) and
                  posts the root (`statement::Posted`) in the epoch's statement bag. Measured: `token-ops statement`
                  epoch 0 → root c108264c…, 8 parity, 1 posted — by an open page, unprompted.
              [x] V1b-3c(i) WITNESSES SIGN THE STATEMENT ROOT — LIVE fx 10-10 (v544): `reputation::statement::build`
                  is the ONE statement computation (it was inline in the page core's binding: a witness tool would
                  have needed a second copy); `ROOTS` are bytes (the DID text in a comment; the core turns them to text
                  for the page). Each epoch's statement is witnessed at `witness_place(epoch)` (seq 0, the root as
                  the entry), drawn at `lot_height` (two steps past due); `tokens::statement_accepted` (the core's
                  acceptance, generalised: `accepted_at` a place); the allowance's VERDICT is built, and a PAYOUT
                  collected, only under an ACCEPTED root. `token-net` `witness_statement` recomputes the statement
                  (registries + key logs) and signs the posted root only if it is the one it computes;
                  `token-ops witness` does it each tick. Tests: no verdict before the root is signed, 3 of 6 short, 4
                  builds it; an unfunded epoch none. Measured: Wren's witness signed epoch 1's statement (root
                  c108264c…).
              [ ] V1b-3c(ii) decay (activity). Was: decay, the statement — THE STATEMENT IS A PUBLIC PROLLY TREE in Block contracts (as a public
                  table's tree: did → reputation, share), witnesses sign its ROOT, a claim cites its entry with a prolly
                  proof — no new contract, no new tree (owner 10-10: "we doesn't use our craftworks prolly tree and
                  block contract?").
          [x] DEDUP — DONE 10-10: prolly CAN prove "the k-th key" (a presence proof + a count proof of what lies below:
              node counts are committed) — but MEASURED 20–40× larger: per pick ~9 KB (100 ids), ~11 KB (1,000), ~20 KB
              (25,000) against 224–480 B for a merkle path; an answer (16 picks) 141–305 KB vs ~4–8 KB, several an epoch
              per node. Two jobs, not one: a prolly tree is for KEYED MAPS that change; a FIXED LIST sampled by POSITION
              wants a path. So the positional proof MOVED into the SDK beside the tree — `freenet_prolly::list` (root,
              path, verify; hash domains unchanged, so every holdings root is the same), the standing crate re-exports
              it; one home, its test moved with it. standing/token/pool/allowance wasm rebuilt (their pins updated:
              they link freenet-prolly — every token id moved again, free before release).
          [x] W  WALLET APP — LIVE fx/fy 10-10 (site v530): its own app `#/wallet` (👛, System; `wallet-app`), the
              account's table `tokens`. Core `wallet.rs` (view, send — the creator's genesis first —, receive: worked
              credits, `pending` = sends inside their margin) over `core/src/tokens.rs` (the steps every token action
              shares, moved out of editions: Step/Ctx/Device/Put, `holding` + `begin`); `Definition::contracts` the ONE
              list of a token's three contracts (page, token-ops, token-live). Page side: `packages/tokens.js` (run /
              putAll / now / device, moved out of editions.js). Receive: an inbox note (`index.send`, kind "token")
              names the sender; retried each time the beacon advances while a send is pending (`beacon.onHeight`).
              NO TOKEN MAKING in the app (owner 10-10: "user will flood with new token") — issuers use token-ops.
              ONE CLOCK: `tokens` takes its beacon from the `beacon` service (`tokens-config` = `{ pay }` only — it
              named a second, throwaway beacon that stopped: a send never became final); a chain's params use the
              TOKEN's beacon (`Definition.beacon`); a page writes only tokens on its own clock ("another clock"
              refused, received per token so one such stops no other). token-live `BEACON_SEED` serves on the pages'
              beacon; `seed_file` moved into node-io. Measured on fx/fy: A added P (5980cdbc…), sent 25 to B → A 975;
              B's wallet (fy) listed P from the note and took 25.
          [x] P  PRIVATE HOLDINGS — STEALTH CHAINS (owner 10-10; TOKEN §7f; before A: the allowance pays to wallets).
              No contract change (a 1-of-1 multisig owner). Phases:
              [x] P1 STEALTH KEYS — BUILT 10-10 (`wallet::stealth`): seed → scan/spend keys; `pay` (R, P, owner =
                  set_id({1,[P]})); `find` (R → P and b + h); `OneTime::sign` (ed25519 by P, hazmat raw_sign); prime-
                  order points only. End to end against the token's rules: Alice's stealth chain (the token's creator)
                  begins, takes its genesis, sends to Bob's one-time owner; Bob's chain receives (worked) and spends —
                  no DID anywhere; another key's set, or a forged signature, refused. 3 mutants killed (no small-order
                  guard; spend without h; h without the scan secret — this one first SURVIVED: the test never checked
                  that finding needs s, i.e. that a holder of the card alone cannot link; now it does).
                  Recovery (owner asked): the `wallet` table is an account table — its keys escrowed to the recovery
                  words (ARCHITECTURE `keys`), so words alone restore the seed; payments re-found by scanning, and the
                  found list kept in the same table.
              [x] P2 PAYMENTS BAG — BUILT 10-10 (`wallet::stealth`): `Announce` (token, from, nonce, to, R — nothing
                  naming either account; "CWPA"), `payments_address(epoch, k)`, `Keys::scan` (compute and compare: one
                  point multiplication an announcement). Bags CHAIN (a bag holds ≤ 1,024 items — one per epoch would cap
                  the network at 1,024 payments an epoch): a sender drops into the first bag with room (`ROOM` = the
                  bag crate's ITEMS_MAX − 64) and reads back that it was kept; a scanner reads k = 0.. until one is not
                  there. (The bag I/O is the page's: P3/P4.)
              [x] P3a WALLET CORE SIGNS STEALTH CHAINS — BUILT 10-10: `Device::set()` (default None); a one-time key is a
                  device whose set is {P}: handover = `Members`, signer = the set, auth = Multi. Tested: genesis, send,
                  collect, spend through `Wallet` on stealth chains, all taken by the token; DID devices unchanged (11
                  wallet tests); 2 mutants killed (always Sig; Writer for a set). No change outputs: a chain holds a
                  balance, the rest stays in the source chain.
              [x] P3b CORE — BUILT 10-10 (`core::wallet`): `private` (a token's found chains and their sum),
                  `send_private` (from as few stealth chains as pay it, largest first, each debit to a fresh one-time
                  owner, signed by the one-time keys — no delegate), `receive_private` (worked credit from the
                  announced chain; `pending` inside the margin); a DID chain's `send` now pays a one-time owner too.
                  Bindings: `stealth_public`, `stealth_payments_address`, `stealth_scan`, `wallet_private`,
                  `wallet_send` (card keys → announce), `wallet_send_private`, `wallet_receive_private`. Tested: A's DID
                  chain pays B's card; only B's scan finds it; pending, then received; B pays A's card back from the
                  stealth chain (taken by the token); A finds and receives it; neither stealth owner is a DID.
              [x] P4 PAGE + LIVE fx/fy 10-10 (site v531): `wallet-app` — the seed(s) in the account table `wallet`
                  (`seed/<fp>`; racing devices keep both, every seed scanned, the card names the first), the card's PAY
                  KEYS (`directory.setPay`, `card(did).pay`), send = pay the card (private chains of one seed when they
                  hold enough, else the DID chain), announce into the epoch's payments bags (first with room, read
                  back), receive = scan from the last epoch scanned (found rows `found/<to>`), retried on beacon
                  advance while pending. Totals shown as "N private (k chains) · M on your id". Inbox-note receiving
                  and its binding removed. `Core.standing_epoch()` now the one epoch length (vouch, statement, wallet —
                  two hard-coded 1024s gone). Measured on fx/fy: B opened Wallet (seed written, card seq 77 with pay
                  keys); A (fx) sent 10 P privately → A 965; B (fy) scanned, found it, received: "10 private (1 chain)
                  · 25 on your id"; B sent 4 back from the stealth chain (its first spend by the one-time key, taken)
                  → B "6 private · 25 on your id"; A found it pending, received it on the next height → "4 private ·
                  965 on your id". 969 + 31 = 1,000.
          [ ] Q  A DID CHAIN HOLDS ONLY STAKE (owner 10-10; TOKEN §7f) — no contract change (every payee is an owner id):
              [x] Q1 EDITIONS PRIVATELY — LIVE fx/fy 10-10 (v535): `new_edition(payee)` (genesis, sale payee and
                  royalty = the author's fresh one-time owner, `purse.payee`, kept with its edition); `of_item` checks
                  the item only (authorship = the item's own `meta.editions`); `buy` from one of the buyer's stealth
                  chains, the copy minted to that one-time owner (`Bought.to`); `collect_copy` worked, no key log
                  needed; `sales(edition)` = each paid mint as an announcement — the purse takes them into the payee
                  chain. The private wallet moved to ONE place: `tokens.purse` (seeds, found, card keys, announce,
                  scan + receive + sales, pay, shield, payee) — Wallet and Editions both use it. Measured: A made
                  "Bronze" (5, 10 P, 10%); B bought #1 → "4 of 5 left · you hold #1"; A's Wallet took the sale → 979
                  private (3 chains); B 21; 1,000 in all. Found live: a random drawn per signing round (never matched
                  what was signed: "too many rounds") — one random per run; sale bytes passed as hex; a "null" child.
                  Older test editions (made before `keepers` joined the definition) no longer decode — dev data.
              [x] Q2 WALLET — LIVE fx/fy 10-10: send pays from private chains only (every seed's); the DID chain's
                  spendable shown "on your id" with "Move N to private" (`purse.shield`: a payment to the own card).
                  Measured: A moved 965 → "969 private (2 chains)"; B moved 25 → "31 private (2 chains)".
              [~] Q3 token-ops make-token: genesis to the creator's one-time owner — NOT POSSIBLE from a tool: a genesis
                  is signed by its owner, a one-time key derives only from the account's wallet seed (its private
                  table), and the page makes no tokens (owner 10-10). So a genesis stays on the issuer's DID and the
                  issuer moves it to private (Q2 "Move to private" — measured). WREN's treasury stays public on purpose.
              [ ] Q4 witness fees' spendable part to a one-time owner (with F).
          [ ] A  WREN DAILY ALLOWANCE (decision 6 — escrow + program; payouts to each DID's chain, the wallet moves them
              to private: the statement already makes what a DID is owed public, and a claim naming a private key would
              link the same way while needing every claimant's key log — more than a contract's 10 related reads):
              [x] A1 THE PROGRAM — BUILT 10-10 (`contracts-src/allowance`, allowance.wasm 122,183 B, 7cdd3961…): one
                  contract per standing epoch (`Params { token, epoch }`); state = the escrow SECTION + the statement
                  ROOT + each payout's PROOF; verdict = each line's allowance paid to its DID (proven under the root,
                  `pay_key(epoch, did)`), every escrow's rest refunded; empty or a verdict, never replaced. Root
                  correctness and escrow reality: witnesses' (as every program). POT 59.26 WREN an epoch (1,000 a day
                  at B's 5 s step), CAP 2%. 2 tests; 2 mutants killed (no proof check; rest left locked); a test that
                  first could not reach the line check (an overpay broke the escrow total first) rewritten.
                  DECIMALS (owner 10-10, micro payments): `token::DECIMALS = 9` (Solana's), `UNIT` — not 18 (owner
                  asked): amounts are u128 and a split multiplies by a weight ≤ 10¹² — at 18 the full supply (10²⁷)
                  × 10¹² = 10³⁹ passes u128 (3.4×10³⁸) and `saturating_mul` pays wrong; at 9 the headroom is 10⁸; token-ops
                  make-token makes 9-decimal tokens (supply in whole tokens); the Usage preview 9 too. token.wasm
                  unchanged (4079c5a3…). WREN (0 decimals) is re-minted with 9 at A3 (nothing given out yet).
              [x] A2 PAGE — BUILT 10-10: `core::allowance` (`verdict`: the treasury's escrows to the epoch's program
                  found in its chain + the statement's owed lines → the program state + the treasury's release, all
                  worked; `payout`: an account's into its DID chain); `statement_build` sets the pot and returns each
                  owed line's proof; `statement.js` computes the statement once an epoch, posts it if missing, then
                  builds the allowance verdict if funded and undecided; the purse collects each recent epoch's payout
                  and MOVES IT TO PRIVATE (never the token creator's: the treasury stays public). `tokens-config`
                  `{ pay, allowance }`; `allowance-wasm` published. `run` now ends on a `done` that is null (it took
                  `done: null` for no answer). Core test: escrow → verdict (2 lines) → release (the rest back) → a
                  payout, once; nobody else paid; a decided program not rebuilt; an unfunded epoch no verdict — every
                  chain taken by the token with the program read.
                  DECIMALS CAPPED AT 9 in the token (owner 10-10: "18 doesn't have enough buffer, so what is the
                  point?") — 63c2b4a; token/pool/swap wasm rebuilt, every token id moved (fx test tokens remade).
              [ ] A3 B: WREN re-minted (9 decimals); Wren funds upcoming epochs (escrow each POT, `until` past the
                  epoch + margin), as a B service; reclaims what no verdict took.
              [~] A4 LIVE fx (v536) — the REFUND path: `token-live serve` with ALLOWANCE=1 made a 9-decimal test token
                  whose treasury (test account 1) escrowed epochs 0–2's pots (ACCEPTED); an open fx page (no service)
                  logged "epoch 0: the allowance's verdict and release put (0 owed)" — both puts accepted by fx (the
                  release checked by the token against the verdict). fx/fy have no reputation (the only root, Wren, is
                  the main network's), so no line is owed there: the PAYOUT path is proven by the core test only, and
                  goes live on the main network once Wren has vouched for someone.
          [ ] WT WITNESSES LIVE (before anything is value; owner 10-10 "proceed"): stake is $CRAFT, none exists yet — the
              bootstrap is the decided BASELINE (Wren) until 6 live stakers.
              [x] WT1+WT2 SEATS — BUILT 10-10 (`witness::seats`, `accepted`): drawn seats min(16, live), Wren's BASELINE
                  key (`witness::BASELINE` = its tool device's public key eb514408…, `token-ops key` prints a seed's
                  public half) a stakeless seat below 6 live, ⌈⅔⌉ of the seats needed (0 live: Wren alone; 3: 3 of 4;
                  6+: no baseline, 4 of 6; 16: 11). 116 token tests (a small-network one, with Wren's real key where
                  the seed is held); 3 mutants killed. No contract bytes changed (acceptance is the wallet's and
                  witnesses').
              [x] WT3 A WITNESS RUNNER — LIVE fx 10-10 (v537): entries reach witnesses by the WITNESS QUEUE
                  (`wallet::queue`: `Queued { token, owner, seq }` in the epoch's chained bags — `wallet::chained`, the
                  one chained-bag rule, payments' too); every token-chain put a page makes carries its new entries'
                  queue items (`chain_put_obj`: seqs past the state the core read) and `putAll` drops them; one
                  chained drop in `tokens.js` (payments + queue). `tools/token-net`: the tools' ONE network view of
                  the token (chains, definitions, key logs, claims, witness bags, beacon outputs — with the OUTPUTS LOG
                  for heights the beacon no longer keeps —, stake, programs, freeze lists, the queue) and `witness()`
                  (lot, seat, may_witness, sign, drop). `token-ops witness …` runs it. Measured on fx: Wren's key
                  (baseline, no stakers) signed A's DID chain seq 0–2 (writer, genesis, move-to-private) in order by
                  induction, then A's new private chain's receive (seq 0). Found live: a chain's first entry is drawn
                  at its epoch's first step, which the beacon's state (newest 600 steps) no longer held — the lot read
                  "not drawn" forever; now read from the outputs log.
              [x] WT4 WALLETS COUNT ONLY ACCEPTED — LIVE fx/fy 10-10 (v540): `tokens::accepted` (the page core's
                  acceptance step: the lot's output from the beacon or its outputs log, the epoch's claims, the entry's
                  witness bag, the signers' stake — each a "want" round — then `witness::accepted`); `receive_private`
                  takes a payment only once its debit is ACCEPTED, else `unwitnessed` with WHY (logged by the purse);
                  the DID-to-DID `receive` (an unchecked credit, unused by the page) REMOVED. Test lot (`tokens::
                  testlot`): six real stakers (signed pool chains staking before epoch 1, signed claims — the baseline
                  unseated: no private key needed), a beacon and outputs log, `witness()` signing as the drawn
                  committee; tests: 0/3 signatures refused, 4 accepted, 6 counted; a private receive pending →
                  unwitnessed (with why) → taken; a sale taken only once its payment is witnessed. Mutant (no acceptance
                  check) killed. The Wallet RECEIVES ON EVERY BEACON ADVANCE while open (it scanned only on opening:
                  found live — a payment made with the page open never arrived). Measured: A paid B 7, 5, 3 privately;
                  Wren's witness (fx) signed each debit; B's open Wallet (fy) took the 3 in 25 s without a reload —
                  "15 private (3 chains)".
          [x] WALLET HISTORY + STATUS (owner 10-10: "why wallet UI is so basic, no transaction history, transaction
              status") — LIVE fy (v541): a token list opening each token's page (`#/wallet/t/<id>`: balance and where
              it is, send, move to private, HISTORY newest first — sent (to whom), moved to private, received
              (private), sales of your editions — each with its STATUS chip: settling (inside the margin), waiting for
              witnesses (why in its title), accepted ✓ N). Core `wallet::status` (one stage reader from the
              announcement: the debit's amount and stamp, then `tokens::accepted`), tested at every stage. The purse
              records sends (`sent/<announce>`: kind, to, at) and stamps found payments. Measured: B's page showed its
              three receipts +7 +5 +3, each "accepted ✓ 1" (Wren's baseline signature).
          [x] A CHAIN PAYS ONCE (owner 10-10: "make that default — balance to another private account") — LIVE fx
              (v543): `send_private` moves each paying chain's REST to a fresh address of the wallet's own in the same
              write (the most that leaves its fee room; dust under min_send stays) — whoever a chain paid learns an
              address that pays nobody else. The purse keeps the rest found at once and announces it (the account's
              other devices); history hides it; the balance counts payments found and not yet taken as "arriving"
              (it read 0 while the rest awaited its witnesses). Tested: B pays A 30 of 100 — B's chain left 0, the 70
              at an address only B's scan finds, not the one A was paid at. Measured: A sent 2 then 1 — history
              "Sent to #34kdzZ" settling → accepted ✓ 1; balance 983 → 982, never 0.
          [x] KA KEEP THE ASSIGNMENT 10-10: `standing::assign` — each registry file's keepers = top COPIES=3 of the
              epoch-before's listed keepers by BLAKE3(keeper key ‖ address key) (no beacon in the rank: the assignment
              holds across epochs, a leaving keeper moves only its own files); a keeper takes its files in rank order
              within its Usage limit. keeper.js computes it each epoch → `keep.assign`; keep re-reads assigned files a
              day apart (`files.keepAt`: read only, by address key; record `a/<key>` with missing indexes); holdings
              add their found pieces (from the next epoch: holdings fix at the epoch's first tick). Live fy (B) v554:
              epoch 3 "29 assigned"; "assigned file 7f16d7f1…: 111/111 piece(s) answered". OPEN: give-to-get cannot be
              enforced (registry entries name no uploader — by design, privacy); assigned and watched each use the full
              limit (no shared budget yet); the ring ARC part dropped (needs the blocked node query).
          [ ] F  WITNESS FEES on every token: a set fee per debit, split among its signers, claimed in their chains.
          KEEPER ISSUANCE ORDER (10-10, after reading the state: the keeper loop is NOT built — nothing produces
          standing records yet): K0a the KEEPER LOOP in the page (commit holdings, answer challenges into its standing
          contract; the delegate's half with a release) → K0b a KEEPERS REGISTRY per epoch (who kept is findable) → K0c
          the statement's `kept` from it (KEEP_MIN answers, bound to the account) → K2 rework (amount = the statement
          line, proven under the root; appended by anyone) → K3 witness gate → K4–K6. $CRAFT decimals 9 (doc said 6).
          [x] K0a KEEPER LOOP 10-10: the node refuses NodeQueries from web apps (freenet-core "Blocked NodeQueries
              from contract web app") → a page's keeper is named by its MEMBER KEY (hex; standing PEER_MAX 32→64,
              standing.wasm 47f3a4ca) and proves only what CRAFTWORKS keeps: holdings = pieces (`files.pieces`) of the
              files `keep` keeps whole (`keep.holdings(epoch)`, fixed per epoch in table keep `h/<epoch>`). keeper.js:
              after login, open once an epoch, answer every sampled height since the last (catch-up; the beacon read
              into the core first — beacon.js takes its reads out). Live fx v549: epoch 1 holdings 15 files / 776
              pieces committed; height 1753 answered (16 pieces sealed in the worker, signed). Owner 10-10: domain is
              ours only; next a FILES REGISTRY (root, size, epoch; per-bag size sums; renewed by keep; give-to-get base
              copies) → K0b keepers registry → KA assignment by BLAKE3(beacon ‖ keeper key ‖ root) (arc dropped: needs
              the blocked query); target = base copies (issuance) + paid copies.
          [x] FILES REGISTRY 10-10: `wallet::registry` Listed{address key, size} ("CWFL") in chained bags per standing
              epoch; live = this epoch + last (holders re-list each epoch). `files::address_key` (every piece address,
              opens nothing) + `fragment_at`; core `registry_pieces(ak, size)` = k+EXTRA fragments per generation (a
              keeper's piece list without the file key). One chained door: `index.bag.chained/chain` (tokens.js moved
              onto it; `stealth_room` → `chained_room`). keeper.js lists own whole files each epoch; `keeper.network(e)`
              → Usage › Kept "Network: N files · bytes". Live fx v550: "1 file(s) listed · 105 KB".
          [x] K0b KEEPERS REGISTRY 10-10: `reputation::keepers` Keeper{did, cred, key_seq} ("CWKP") in chained bags per
              standing epoch; `listed(items, log_of)` → (did, record key) verified by identity's `credential_node`.
              The chained rule moved to its bag: `craftworks_bag_contract::chained` (bag.wasm f5c35985 and allowance.wasm
              7cdd3961 unchanged; wallet re-exports). keeper.js lists itself each epoch; Usage › Kept shows keepers.
              Live fx v551: "1 file(s) listed · 105 KB · 1 keeper(s) this epoch".
          [x] K0c STATEMENT KEPT 10-10: `standing::record::{kept, KEEP_MIN=4, kept_accounts}`; statement e counts
              keepers listed in e-1 whose record has holdings + ≥4 answered heights. Page (`statement_build`) and
              witnesses (token-net `kept`, token-ops reads standing.wasm beside token.wasm) — one rule. Record name
              owned by `standing::keeper_peer` (core keeper fns and beacon-step's sealed_hash derive it; JS no longer
              names it); record id `standing_contract::Params::keeper(..).id(code)` (standing.wasm 47f3a4ca unchanged).
              Live fx v552: "epoch 2: … 1 account(s) kept the epoch before". Fixed: allowance verdict with nothing
              escrowed resolved `{}` → `v.puts.length` threw (v553).
          [x] KA follow-ups 10-10: every logged-in account with limit > 0 registers as a keeper (holdings or not); every
              holder re-lists the files it keeps whole (own, watched, assigned) — a file stays listed while anyone keeps
              it. Live fy epoch 4: "57 file(s) (29 assigned), 3375 piece(s); 57 file(s) listed".
          [x] BOOTSTRAP (owner 10-10): keeper pot ungated while < KEEPERS_GATED_FROM (1000) accounts kept; weight =
              rep + GATE floor; lines for zero-rep keepers; allowance stays gated. TOKEN.md gate paragraph reconciled.
          [!] TOKEN RE-KEY FOUND 10-10: token.wasm changed be2d1f2d → 3431346f (552e20f, prolly list move) → d4dd92e2
              (c7464fb, identity STANDING_DOMAIN) — the token contract links identity + prolly, so every chain moved
              address; fx/fy test chains intact under be2d1f2d (`token-ops chains` with the old wasm: all found).
              Wallet status now says why ("its chain (…) is not on the network").
          [x] FROZEN CONTRACTS 10-10 (owner: "proceed"): contracts/FROZEN pins token.wasm d4dd92e2 + signer.wasm
              f9fc78b0; contracts-src/build.sh refuses other bytes unless CONTRACT_VERSION_BUMP=<name> (owner's say);
              publish refuses a mismatch. Controls: token rebuild from today's source REFUSED (874e8b2e — the standing
              crate's later changes would have re-keyed a third time); bag rebuild copies; a tampered token.wasm
              refused at publish. OWNER 10-10: "nothing deploy yet. no need to bump. we'll do migration on first production
              deployment" → token.wasm UNPINNED until then (FROZEN keeps the signer; a note holds the token's place),
              rebuilt from source 874e8b2e. OWNER 10-11: NO MIGRATION ("Nothing really being used by user") — the
              release re-keys the token contracts; $CRAFT and WREN start fresh on the main network (no "moved" entry).
          [x] K2 KEEPER MINT = STATEMENT LINE 10-10: `Body::KeeperMint { epoch, root, proof, amount }` — amount exactly the
              chain owner's keeper line under `root` (`reputation::statement::line_proven`, now the allowance's check
              too); appended by anyone with work; once per epoch; stamped once the epoch began. The contract no longer
              reads standing records (its second KEEP_MIN=6 gone; token no longer links the standing CONTRACT crate).
              `Keepers { rate, half_life, start }` = the epoch POT. token.wasm b338db70, allowance.wasm 55e03278.
          [x] K3 WITNESS GATE 10-10: `wallet::statement_accept` in `may_witness` — a KeeperMint is witnessed only under
              the root ACCEPTED for its epoch. Test: unwitnessed → refused; another root accepted → refused; its root →
              signed.
          [x] K4 ISSUANCE 10-10: `wallet::craft` — $CRAFT's ONE definition (MAX 1e9, 9 decimals, POT 61,035/epoch,
              HALF_LIFE 8192; genesis 0, no authority: anyone puts it) and `keeper_pot(e)` = 90% of the pot, cap 2%;
              both statement builders (page, token-net) use it. `keeper::mint` (core): a keeper line of an ACCEPTED
              statement → a worked KeeperMint on the DID's $CRAFT chain (+ $CRAFT's contracts if absent) → witness
              queue. statement.js mints EVERY keeper line of the last 3 epochs (anyone's: a keeper offline is paid).
              Fixed on the way: (1) the witness read the queue by SUPPLY epoch (4096), pages write it by STANDING
              epoch (1024) — the witness now reads standing epochs; (2) page and witness computed the statement 2
              heights in and disagreed on `kept` (records still settling) — `statement::SETTLE` = 32 heights for both,
              lot at SETTLE+2, and a page posts its own root beside another's. Live fx/fy v559: "epoch 5: 2 keeper
              line(s) minted as $CRAFT stake"; Wren SIGNED $CRAFT 08e08556…/45caba3e (A) seq 0 and …/1ead752b (B) seq 0.
          [x] K5 $CRAFT IN THE WALLET 10-11: `wallet::View` + staked + its keeper mints (epoch, amount, stamp); the wallet
              lists $CRAFT first (`craft_id`), its total counts stake ("staked (locked: earned by keeping)"), its history
              "Earned by keeping" per epoch; Send hidden when nothing is spendable. Live fx v561 (A): CRAFT 2,441.40625
              staked = epochs 5 + 7 × 1,220.703125 (each the 2% cap: two keepers; the rest never minted).
          [x] K6 ISSUANCE MONITOR 10-11: statement_build's `issuance` {keepers, total, top, pot}; `statement.issuance(e)`;
              Usage › Kept "$CRAFT issuance" — the last 5 epochs: keepers paid, minted, % of the pot, largest share.
              Live fx v562: epochs 5–8 "2 · 2,441.406 · 4.44% · 50%", epoch 4 "0". OPEN: the split by LINEAGE (needs the
              reputation tree's lineage per line), below-gate claims (none possible now: the statement pays only kept
              lines; a witness signing a mint under another root is refused by K3).
          WITNESS COMPLETENESS (10-11, owner order: after keeper issuance): S1 domains + guarded signing → S2 staking
              (writer entry on the $CRAFT chain, StakeClaim, claims registry; pool = $CRAFT) → S3 the page's witness
              loop → fees F → liveness lapse.
          [x] S1 10-11: identity CLAIM_DOMAIN (in SIGN_DOMAINS) + WITNESS_DOMAIN (NOT: only `WitnessSign`), one owner
              of both message forms (`claim_message`, `witness_message`; token's StakeClaim/WitnessSig call them).
              `Request::WitnessSign { chain, seq, entry }`: home site only, the place saved before the signature,
              another entry there refused (`Why::Equivocation`) — the TOKEN §7d rule. Page door `identity.witnessSign`.
              Test: same entry → same sig; another → Equivocation; TokenSign refuses a witness message. token.wasm
              ad470883.
          [x] S2 10-11 (unit-tested; live with S3 — once stakers claim, a committee needs 2 signatures and Wren's
              baseline alone no longer suffices): the lot's POOL is $CRAFT (page: `craft_id`; witness tool: pool
              `craft`). `Wallet::adopt` (this device made the chain's writer); `keeper::claim` — staked $CRAFT claimed for
              the next open claims epoch (supply epochs; frozen LEAD before), key = member key, signed via TokenSign
              (CLAIM_DOMAIN); `keeper_claim` binding; keeper.js claims once a standing epoch. Test: claim holds, the
              lot's stake check finds the key (stake_at keys), claimed once.
          [~] S3 10-11 (unit-tested; live check pending: claims take effect at claims epoch 3 = height 12,288 on fx):
              core `witness::entry` — THE WITNESS RULE (`may_witness`) over what the page read, wants collected and
              asked first; `Verdict { NotSeated | Refused(why) | Sign { chain, seq, entry } }`; `out_at` shared.
              packages/witness.js: every 4 heights, this and last standing epoch's queue → seated + rule holds →
              `identity.witnessSign` → the witness bag. Test: B's receive — all six drawn stakers Sign its exact hash;
              an unclaimed key NotSeated. Live so far: the new $CRAFT (82e7a9ab: token.wasm ad470883 re-keyed it)
              minted again for epoch 8, Wren signed 82e7a9ab…/45caba3e seq 0; `keeper::claim` quiet while $CRAFT absent.
          [x] F WITNESS FEES 10-11 (unit-tested; published to fx after the S3 live check — token.wasm 84325c4d re-keys
              $CRAFT): `wallet::fees::count` — the ONE count (page `witness::signed`, tool `Reader::signed`): each
              signature on an epoch's queued entries by a key SEATED for its place, once per seat, to its claimant.
              Statement Line + `witness` (64 bytes), Pots.witness = `craft::witness_pot` (10%, cap 2%), roots never.
              KeeperMint + `fee` = the line's witness, credited SPENDABLE (keeper amount still locked); witness-only lines
              mint. Wallet: "Earned by witnessing" rows. Tests: statement 3:1 split + cap + root never; count 4 seated,
              stranger and other-entry sigs nothing, a place once; mint 40 locked + 30 spendable, wrong fee refused.
              OPEN: statement-place signatures not yet counted (only queued entries).
          [x] EXPLORER 10-11 (owner: "more transparent … less prose. like a block explorer"): `#/explorer` (System) —
              tables only: Clock (height, standing/claims epoch, next claims epoch, entrants close), Statements (roots
              posted, accepted n / waiting why, keepers paid, $CRAFT minted), Witness lot (claims epoch c and c+1:
              account, stake, key, this node), Witness queue (token, owner, seq, entry kind, stamp, signatures,
              accepted / waiting / missing), Registries, This page (keeper, witness). Bindings `clock`, `queued_read`,
              `claims_registry_id`, `claims_read`, `entry_status`, `statement_status`. Refreshed every 8 heights. Live
              fx v565 at 9,688: showed the re-keyed claims (no entrants) and old chains "not on the network". Fixed: an
              unreadable chain was "seated, refused" — now not seated (v566).
          [x] APPS RESTRUCTURED 10-11 (owner: "wallet, usage and explore … not structured"): one question each —
              WALLET = money (Tokens · Contribute · Earnings: the contribution level/split and ledger moved in from Usage);
              NODES (replaces Usage; `/usage` opens it) = the account's machines and their work (Nodes: account row from
              its statement lines + the witness seat; per node from its OWN standing record — pieces, bytes, answered
              this/last epoch, last answer, seat · Keeper: this node's limit and kept files · Data used: the month);
              EXPLORER = the public network only ("This page" removed). Duplicates removed (network line and issuance
              lived in Usage too). Bindings `keeper_record_id`, `standing_read`; statement `paid` lines. Live fx v567.
          [x] PER-NODE LIMIT 10-11: `limit/<member key>` in the account's `kept` table (fallback: the old account-wide
              row, then the default); `keep.limits(keys)`; Nodes shows each node's. Live fx v568: "512.0 MB" carried over.
          [x] SEAT NODE 10-11: only the account's node that answered the most challenges last epoch (ties: lowest key)
              claims — every node computes it from the public records (before: every node claimed, the registry's merge
              kept a hash-random one). Multi-node not tested live (fx/fy accounts have one node each).
          [x] Q4 FEES PRIVATE 10-11: the wallet's receive pass shields any spendable $CRAFT on the DID chain (witness fees)
              to a private address, like the allowance; the keeper share stays there locked (the lot's weight).
          [x] STATEMENT FEES 10-11: `fees::count(queued, statements, reads)` — a statement root's seated signers count
              too, only where the root reached the committee's need (the baseline's seat counts toward it, earns
              nothing). Page passes the epoch before's posted roots; the tool reads them. Test: 4 signers of the
              accepted root counted once each; a root 2 signed pays nothing. Live fx v570.
          [x] DECAY 10-11 (code + tests; contracts rebuilt and published after the live S3 check — Line grew, token.wasm
              re-keys $CRAFT): `reputation::decay` (full ≤ DECAY_AFTER=30 absent epochs, then halving per DECAY_HALF=30),
              `reputation_active` (each vouch × its voucher's decay). Statement Line + `active` (72 bytes) carried from the
              previous ACCEPTED statement (`statement::lines` reads its public blocks; `fees::statement_accepted` picks
              it, page and tool alike) and refreshed by the epoch before's activity (kept, witnessed, vouched:
              `active_of`). Tests: decay steps; an absent voucher's vouchee gets less, its own reputation does not;
              active carried/refreshed; 300 lines read back from blocks, a withheld root asked for.
          [~] BACKGROUND KEEPER + WITNESS (#3, owner 10-11 "proceed 1-5"): the identity delegate runs them with no page open.
              [x] 3a 10-11: the rules (tokens, wallet, editions, allowance, keeper, witness) moved to crate
                  `craftworks-rules` (pure: no node library) — core re-exports them; the delegate links them (it cannot
                  link core: freenet-stdlib 0.10's unconditional `__frnt_set_id` export collides with its 0.12). Core's
                  page bindings behind a default `page` feature. identity.wasm 9f74563 (1.84 MB).
              [x] 3b 10-11 (unit-tested; publish + live check next — identity changes: everyone enters their PIN):
                  the delegate's WITNESS round (`delegate/src/witness_round.rs`): the home page hands a WitnessMandate
                  over (token, idlog, bag, beacon code, beacon, pool; lists the member for upkeep, ticks); each wake-up
                  the next member in turn whose page is away reads beacon + queue (standing e-1, e), runs
                  `witness::entry` (the page's rule) per item, signs through the SAME guard as the page
                  (`upkeep_witness_sign`) and PUTs the signature into the entry's witness bag. The queuing page makes
                  each entry's empty witness bag (core `bags` on a chain put): a delegate's new contracts do not
                  propagate (freenet #5542). Test: a drawn staker, page open → no round; away → one signed put,
                  verified; next wake-up → nothing (settled).
              [x] 3c 10-11 LIVE (fy's page closed: its record gained an answer at 12572, made by its delegate): the KEEPER MANDATE (`Request::KeeperMandate`: standing +
                  beacon codes, beacon, the epoch the page opened, its hosted pieces) and the delegate's keeper round
                  (`delegate/src/keeper_round.rs`, on the shared `reads.rs` round): with the page away it opens a new
                  epoch on the same holdings, finds each sampled height since the last, SEALS ONE PIECE A WAKE-UP
                  (seconds each, by design; kept needs KEEP_MIN answers an epoch, no per-height deadline) and puts the
                  answer as an UPDATE whose delta is the merged record (no code in the delegate). Signs only
                  STANDING_DOMAIN (`upkeep_standing_sign`). Test: page open → nothing; away → one seal a wake-up, the
                  answer holds and checks against the real seal; answered once.
              [x] 3d 10-11 LIVE (fx and fy pages both signed epoch 12's root 149381c0) STATEMENTS WITNESSED BY PAGES (found: only Wren's tool signed statement roots — "no one must
                  stay online" broken): `witness::seated` (one seat check, entries and statements), `witness::statement`
                  (the root THIS page computed, signed at `witness_place(e)` when seated); witness.js signs it through
                  `witnessSign` (equivocation guard: one root per epoch per member).
          [x] DEDUP 10-10 (owner: "ensure there is no duplicated method/foundation/primitive"): identity's
              `credential_node` is the ONE credential rule (token writer + keeper mint + signed vouch); `node.read` the
              page's ONE GET-into-core (identity, editions, beacon); `tools/node-io` the tools' ONE put/get/container and
              the beacon's standard params (token-live, token-ops, beacon-live, health-watch).
          [ ] K3 WITNESS GATE: may_witness checks the reputation gate for a KeeperMint.
          [ ] K4 ISSUANCE BAG: claims listed per token per epoch.
          [ ] K5 PAGE: claim each finished epoch (needs the page's standing answers running).
          [ ] K6 MONITOR: the issuance page (per epoch: paid, total, concentration, by lineage, below-gate).
      [ ] 28 EDITIONS ON THE ITEM PAGE (was "shop"; owner 10-10: no separate app — buying and selling are embedded in
          the item kinds app) — sell an edition on demand over the wallet core. Phases (10-10, all local):
          [x] S1 CORE — DONE 10-10: `core::editions` (edition read over its records, `buy`, `collect_copy`,
              `copies_of`) as steps want/sign/done; `wallet::Asking` (a device that signs by asking the delegate, one
              round per entry); bindings `edition_read/buy/collect/chain_id`, `beacon_height`, `not_there`.
          [x] S2 DELEGATE — DONE 10-10: token entry messages carry `ENTRY_DOMAIN` (owned by identity);
              `Request::TokenSign { message }` signs one with the member key, home site only (`NotAnEntry`, `NotHome`,
              `NoSession` refused). Credential + key seq come from the account's devices contract. Full-release item.
          [x] S3 FINDING — DONE 10-10: the item lists its editions in its own `meta.editions` (hex ids; only its author
              writes the row; edited by the existing item edit, merging meta). `editions::item_id(ref)` is the ONE
              derivation of an item's 32-byte token id; `of_item` shows an edition only if it names this item and its
              creator is the item's author. Bindings `edition_item_id`, `edition_of_item`.
          [x] S4 ITEM PAGE — BUILT 10-10 (not yet run in a browser: S5): `packages/editions.js`, placed by
              item-page before the comments. Reads `meta.editions`, shows each edition the item shows (name, numbered
              or copies, left of cap, royalty, the viewer's copies), Buy (pay + paid mint, then collect), and for the
              author a form (name, price, copies, numbered, royalty %) → `edition_create` puts + `items.editItem`
              with `meta.editions` appended. Set up by `editions-config` (`{ beacon, pay }`; default `{}`: no section).
              A buyer that created the pay token begins its chain with the genesis in the same put.
          [x] S5 LIVE fx/fy — DONE 10-10 (site EckhKsQW… v520, editions-config from `token-live serve`: a ticking
              beacon + pay token P whose creator is B): A (fx, did 5hSRce…) on its post "Count test" made editions
              Gold (10) and Silver (5) through the form; B (fy, did 34kdzZDX… = 1ead752b…) pressed Buy on Silver:
              its P chain began (genesis) and paid, the paid mint went into record 0, copy #1 collected — every entry
              signed by fy's TokenSign; B's page then read "4 of 5 left · you hold #1", and A's node on fx "4 of 5".
              Found live: a stray "null" child (fixed); a "no money" refusal that named nothing — it now names the
              buyer, the token, its chain and the token's creator (it showed the test had the wrong DID).
          NEXT (not built): the item's author sees sales/earnings; B's P balance shown; resale through the swap.
      [x] 18 APPS OUT OF THE TOKEN — DONE 10-09. NEXT: NFT = Craftworks item (mint names the item id, optional version
          pin, the token the owner of record; a standard "Craftworks items" token, only the item's owner mints, once).
      [x] 17 BATCH-AUCTION POOLS (TOKEN §5) — in parts:
          [x] 17a CLEARING (`clear.rs`): limit orders (an LP's curve = a ladder of them, built by its client), ONE
              uniform price maximising matched A, ties → smaller imbalance → lower price; short side whole, long side
              pro rata (rounded down, the rest to the earliest); buyers pay ceil, sellers get floor (B dust stays in the
              batch). 6 tests; 10 mutants killed (2 after tie cases, one found by search).
          [x] 17b the POOL (kind 8, state = params) + ORDER entry (a debit held for (pool, epoch), stamped before close)
              + SETTLE (refund, once, from settle) + BOUGHT (credit in the other token's chain, once, cites the order)
              + the ORDER REGISTRY (kind 9) frozen at settle; no cut over an open order. 108 tests; 13 mutants killed.
          [x] 17c THE WITNESS RULE for settlements: `wallet::batch` (the registry's pointers that are an ACCEPTED Order
              of this pool and epoch, in the chain of the token its side gives — anything else skipped alike by
              everyone; a failed read → that witness does not sign) cleared; `Settle`/`Bought` only as that clearing.
              110 tests; 7 mutants killed (2 after a membership test).
          [x] 17d LIVE fx/fy (`token-live pool`): pool + two orders + registry ACCEPTED; an order at the close and a
              settlement before settle REFUSED; a pointer after settle ignored (2 held); cleared at 2 (250 A for 500 B);
              all four settlements ACCEPTED; a second purchase REFUSED; balances 1: 750 A / 500 B, 2: 250 A / 500 B —
              both tokens conserved. (Witness recomputation itself: unit-tested; live witnessing needs the staking
              setup of the witness mode.)
      [x] 17 BATCH-AUCTION POOLS — DONE 10-09.
      [x] 16 REPRODUCIBLE CONTRACT BUILDS — FIXED 10-09: token and standing were NOT reproducible — the same commit built
          in two checkouts gave two hashes (standing d4a58a72 / 87552c5a vs 09cfdbc8 committed; token ff19c4ca vs
          caed367c). Measured cause: location, not content — no path is in the bytes (strings diff shows code layout
          only), the path remaps had no effect, but two checkouts built from ONE fixed path gave identical bytes (Cargo's
          metadata for path dependencies at the repo root carries their absolute path). Fix in contracts-src/build.sh:
          rsync the sources to /tmp/craftworks-contract-build (a lock keeps builds apart) and build there. Verified: a
          second checkout gives token bcc27cbf / standing 43fe3f2f, identical; beacon (no root deps) unchanged
          c7b91ff4. Token re-keyed (dev mode). devices.wasm will change on its next rebuild — tied to the full
          release; signer.wasm frozen, never rebuilt. vest / misdeed / operators re-run live on the new bytes: same.
      [x] 15 STANDING CONTRACT — BUILT + LIVE 10-09: `contracts-src/standing` (own workspace, standing.wasm 09cfdbc8), one
          per peer (params: the beacon's code + id, the peer); its record checked FROM SCRATCH against the beacon and,
          for heights it no longer keeps, its OUTPUTS LOG — all read in one round; copies merge record by record. 1 test
          on a REAL beacon (an answer older than its keep: from the log); 4 mutants killed (1 after a recent-only case).
          LIVE fx/fy (`BEACON_START=2040 token-live standing`): beacon to 2752 (keeps 2153..), a record of 12 answers
          (1 past the keep) ACCEPTED on Y; one answer moved and re-signed REFUSED. Payout amounts wait on economics.
      [x] 14 THE WATCH (equivocation detection) — BUILT + LIVE 10-09: `wallet::equivocations(bag sigs, registry claims)`
          → (staker, the Slash to append). 99 tests; 3 mutants all EQUIVALENT (the key filter is an optimisation —
          `equivocation` checks the signatures itself; ed25519 is deterministic, so one key signing one entry twice gives
          the same bytes). LIVE fx/fy (witness mode): the watcher read 18 signatures at a place (16 honest + 2 by staker
          20 on other entries), found staker 20, its burn ACCEPTED, stake 50 → 0. Running it continuously = rollout.
      [x] 13 BACK-DATED ENTRIES — BUILT 10-09: a witness signs no entry stamped more than an epoch (FRESH) behind the
          beacon (an older epoch's committee is known: back-dating would choose it); accepted entries stay accepted.
          98 tests; 3 mutants killed. Witness-side only: token.wasm unchanged (caed367c).
      [x] 12 LOT HARDENING — BUILT + LIVE 10-09: CLAIMS REGISTRY per (pool, epoch) (kind 7) frozen LEAD (512)
          steps before the epoch: no selective claiming after an output is known; the lot fixed by the PLACE —
          predecessor's stamp + margin + 2, floored at the entry's epoch's first step — so two entries at one place
          share one committee (a fork gets no friendly committee); one try an epoch remains. accept takes the
          predecessor; cited() returns each cited entry's predecessor. 97 tests; 8 mutants + 1 re-run, all killed.
          LIVE fx/fy (`BEACON_START=3544 token-live witness`: a test beacon started just before the freeze, so the run
          waits 110 s, not 10-40 min; the first try from height 1 ran out of its 40 min at 3470 on a loaded machine):
          16 claims registered at 3550 (freeze 3584); a claim after the freeze changed nothing (16 held); every honest
          entry drawn 16 / signed 16; the buried fake and its payment refused by all; 3's wallet Ok(16) in 30 ms; the
          equivocating staker burned. token.wasm caed367c.
      [x] 11 BEACON OUTPUTS LOG — BUILT + LIVE 10-09: one contract per 4096 heights (beacon wasm, kind
          LOG 0x4c), filled from the beacon's FINAL steps, a held height never changed; token-live's ticker records it;
          readers fall back to it past the beacon's keep. Found: without it an account idle past the beacon's keep could
          never be witnessed again (the predecessor's lot unverifiable). Beacon 14 tests; 6 mutants killed (1 after a
          duplicate-height case was added).
          LIVE fx/fy (`token-live outputs`): beacon fast-forwarded to 700 (keeps 102..=701); heights 1, 2, 50, 99 read
          from the log and 300, 650 from the beacon — 6 of 6 exactly as computed; a forged output for 50 changed nothing;
          one for 4000 (never held final) REFUSED. beacon.wasm c7b91ff4.
      [x] 10 WITNESS MISDEED BURN — BUILT + LIVE 10-09: `Body::Misdeed` (key, its signature, the signed credit): the node
          reads the cited chain and, where it can say (unfrozen, held back to the debit's stamp), finds no debit → stake
          and unbonding burned. One home for "the debit a credit cites": `debit_for` (contract, wallet, misdeed). 96
          tests; 9 mutants + 3 re-runs after precise cases, all killed. LIVE fx/fy (`token-live misdeed`): a misdeed over a
          REAL receive REFUSED; over the buried FAKE ACCEPTED, stake 600 → 0. token.wasm 014d49a9.
      [x] 8 OPERATORS / setApprovalForAll (TOKEN §1b) — BUILT + LIVE 10-09: operators list per (token, owner), spans
          kept, append only extends the past; spends by an operator on at their stamp; read only when needed. 94 tests,
          20 mutants killed. token.wasm 6f0d1dca. NEXT (contract/mechanism, buildable now): beacon entropy mix → beacon
          output history → refuse far back-dated entries → equivocation detection → standing contract → batch-auction
          pools → pooled lending.
      [x] 7 VESTING (TOKEN §4a) — BUILT + LIVE 10-09: Vest (debit of the schedule now) / Vested (anyone appends a step
          once the beacon reaches it, once, exactly as scheduled; never taken back). Found live: a first copy skipped the
          time rule (an early claim ACCEPTED on a fresh chain) → time rule from scratch + the witness applies all timed
          rules (`timed`, shared with append_check) at the entry's own stamp and refuses stamps ahead of the beacon.
          91 tests; 20 mutants killed. token.wasm 026fd3d2. Not built: setApprovalForAll (one approval for all items).
      [x] 6 WITNESSES BY LOT, weighted by EARNED STAKE (TOKEN §7a) — owner 10-09: "foundation first", the token's
          issuance designed later; built generic (any locked stake), keeper rewards plug in as its source later:
          [x] 6a STAKE entry (6219feb) (part of a chain's balance locked as witness stake; released after a delay) + burn on a
              fraud proof (one key, two witness signatures at one place)
          [x] 6b the DRAW — public lot (no global total for a VRF): stake claims, exponential-race tickets in
              fixed point, the k lowest; measured 100/200/700 → 9.7/20.1/70.1%, seven sybils of 100 → 69.3%
          [x] 6c WITNESS SIGNATURES (5b6f469) (an entry counts once enough drawn witnesses signed; each signer's stake
              read from its chain)
          [x] 6d wallets verify by witness signatures; checkpoints + vouches removed — `wallet::accept` (one entry's
              committee signatures; lot = claims of its stamp's epoch, beacon output at stamp+1), `wallet::may_witness`
              (the INDUCTION rule: the chain UP TO the entry passes the contract check — so its citations are checked in
              full, not only the last RECENT — not forked; predecessor accepted; every cited entry accepted),
              `wallet::cited`, bag codecs for stake claims + witness sigs. Found in self-review and fixed: checking the
              whole chain let a buried receive of a REAL accepted send to someone else through (test added). Committee
              16 / need 11 = placeholders. 83 tests; 7 mutants, 6 killed; 1 survivor explained (the receive-pick
              predicate in `cited` — the prefix's contract check refuses a mismatch first). token-live: `wallet` mode
              removed (6e adds `witness`); `cut` step 11 now reads A's chain whole across its cut. token.wasm 8008e8f9.
              Open for 6e: should a witness refuse an entry stamped far behind the beacon (old-stamp first copies
              have a committee known in advance)? A cut's witnesses must audit its history (`cut::whole`+`audit`) in
              their `holds`.
          [x] 6d2 stake bound to its keys and its epoch (found preparing 6e): a claim's key must be one its chain's stake
              burns for (writer, or `Carried.retired` — writer keys retired while staked, carried by cuts, cap 16);
              stake read as held before the claims' epoch (`wallet::stake_at`), so acceptance never changes later;
              UNBOND = 2 epochs. 86 tests; 12 mutants + 3 re-runs, all killed (retire-always showed a redundant
              condition: removed). token.wasm 5a32b9ea.
          [x] 6e LIVE fx/fy (`token-live witness`, 10-09): 16 stakers received and staked 50 each in a pool token (16/16
              ACCEPTED); beacon fast-forwarded to epoch 1 (height 4098) in 1347 s (~3 steps/s); 16 stake claims in the
              stakers bag; in token A every entry of 2's chain (7) and 3's receive drawn 16 / signed 16 into its witness
              bag; 4's buried fake receive REFUSED by every member ("a receive whose send is not in its sender's chain"),
              5's receive of its 3000 refused too ("not witnessed"); 3's wallet accepted its 300 from the bags alone —
              Ok(16), 16 stake chains read, 30 ms; 5's wallet refused; staker 20's double signature burned on Y (stake now
              0, still 50 as counted for epoch 1). Run on token.wasm 5a32b9ea; the commit's 9f0b0919 adds only the bag
              address helpers (`wallet::stakers_bag` / `witness_bag`), no contract rule.
              Notes: the epoch wait (4096 steps) dominates any live test — start a test beacon near a boundary if it
              matters; claims bag cap 1024 items per epoch (enough now); beacon keeps 600 steps, so an OLD entry's lot
              output needs a record of past outputs before wallets verify old entries.
      (was: backed by PROVED STORAGE — superseded by earned stake, see TOKEN §7a)
          first; closes prevention of double spends, receiver-only history checks, partition fill/refund
          — and REMOVES checkpoints + vouches outright (owner 10-09: no token is live, no old history to keep)
  rule. Later, separate programs: distribution, staking, pools, keeper issuance.
  Found while building 1: a sender's cut broke the receiver's recent receives (re-validated from scratch, no evidence
  in them) → new citations strict at update, old ones lenient (TOKEN §2). Open: a fresh PUT can carry an unshown
  receive hidden by a cut — the wallet refuses it (TOKEN §8 item 5).
  Owner 10-09: "prolly trees are traversable; if not available, never make a network request" (a 60 s wait is what
  we built to avoid) → history + received set built WITH PARITY, the cut carries root parity; the wallet never fetches
  (`Fail::Need`). Next with the page wallet: the core's tree-by-root session for `blocks.fetch` (TOKEN §8 item 4).
- [ ] Proofs: DEFERRED until rewards pay (owner 10-07); low-cost plan in REWARDS §7 item 5. Next on the track: treasury
      pass + statements + shadow ledger (REWARDS §7 item 4).
- Decisions recorded in REWARDS: carriers = holders weighted by demand; users are the keepers; never paid to own
  account/nodes; a pass buys the private (public stays free); §4a what is signed vs provable.
- Nodes: all on freenet 0.2.142; 7509 + B `--hosting-mem-share 0.5`; B also `--max-hosting-storage 8589934592` (owner 10-07 22:40: the default 512 MiB was full at ~3,070 contracts, a publish evicted 440/h and accepted pieces went unserved; drop-in zz-hosting-storage.conf).
- Upstream: #5750 updated (head c5f584be) + reply; #5828 opened for #5785; both await maintainer CI approval (owner:
  do not ask). Unanswered-GET report uploaded from B: code 6YRSRD (9 unanswered + 9 NotFound of 100 small, listed in
  the report message).

## Level groups — real encryption for PRO / VIP (owner 10-07; REWARDS §5a "Who hands over a key")
Reuse groups (circles) + ask/admit + delegate upkeep; any member admits; honor system; lapsed removed by any member.
- Identity changed for this (owner 10-07: "do both"): 5dea06e6 → 3c186609 (44f170bf went to fx/fy only, before the
  review's fix: `lapsed` counted in ANY space against anyone but the owner — now level groups only, never an admin;
  an unknown level reaches nobody). On fx/fy a PIN login handed the member over (earlier builds' upkeep retired, keys
  gathered). Published to the 3 sites + B: everyone logs in once with their PIN.
- DELEGATE path measured: a new account at PRO asked with fy's page closed → fy's delegate admitted it (code `level`,
  epoch 2) and it read the sealed post in 41 s. Earlier, fx's ask failed at the delegate's key-log read ("no key log")
  while fy's page read fx's card — cause unknown; the delegate now says which (none answered / N bytes unread).
- [x] L1 not needed: admitters read the asker's card (contributions are on it).
- [x] L2 Per-creator PRO / VIP groups (`groups.of("level:pro")`), policy "members may invite"; made when a creator first
      gates an item.
- [x] L3 A gated item SEALED into its creator's level group (audience `level:pro` / `level:vip`, as role:/list:/circles).
- [x] L4 A reader at the level ASKS to join the creator's group (existing request bag) on opening a gated item.
- [x] L5 Admission checks the asker's card level — page upkeep (JS) and the identity delegate (Rust `delegate/`, no page).
- [x] L6 Lapsed members removed by any member (page + delegate), the group moving to a new key.
- [x] The picker's note says sealed now. Tested on fx/fy: fy's VIP post → teaser + sealed in "VIP of #34kdzZ"; fx (VIP)
      opened it → asked → fy's page let fx in → fx read the sealed body. Lapse: gov tests only (not run live).

## MLS + DAG, composed (owner 10-07: "evaluate the design and make it work") — ARCHITECTURE §3 `ordering`, §5 Ordering
Evaluation (from the code): acts were sorted by the writer's claimed time (gov replay), so a member whose clock is
behind had an act replayed before the grant it rested on (gov test `a_clock_behind_drops_an_act_a_causal_time_keeps_it`
shows the drop); a missing act was invisible; history was one `prev` per epoch (Ivvor's space: one row cut all of it);
and the add path committed BEFORE recording its act (against "acts decide, MLS carries out"). gov and the delegate are
untouched (identity stays 5dea06e6).
MEASURED on fx/fy (DAG test 06:43): re-adding a removed member churned the group (epochs 12 → 17 in two minutes): asks
to "welcome again" about welcomes from before the removal waited while the member was out, then each was answered once
they were back (a remove + an add each, making more dead welcomes and asks). Fixed: the inviter keeps the tag of its
newest welcome per person and space; an ask about an older one is marked answered and not acted on. After it: one dead
welcome (a key package on fx's card offered by its other device muxrky2j, not held by this one — fx renewed its card),
then joined at 35 with 35 earlier epochs kept; epoch 33's row "prev+8 back".
- [x] `ordering` type `dag` (replaces the planned `log`): deps = heads seen (≤8), at = max(now, latest counted + 1),
      a clock >1 h ahead not followed; only COUNTED acts move the clock (review: a refused row could push it).
- [x] roles: every act / published member act stamped; `r.missing()`; trace "acts N (M naming what they saw)".
- [x] keys: open rows carry `back` (e-2…e-9, from the epoch before's own row: on its branch); history walks every link,
      nearest wins, crosses broken rows, reports the highest gap; mend adds `back` to rows that lack it.
- [x] conversation.invite: the `added` act first, then the welcome; stale welcome-again asks dropped (newest tag kept).
- [x] (Corrected: `muxrky2j` is a key-package BATCH id, not a device.) The dead welcomes on re-add, measured: after a
      removal fx retried welcomes it had already joined (spent packages) and two inbox passes (chat + upkeep) could take
      one welcome at once. Fixed: accept() one at a time; a welcome tried once per page (retried only if not dead);
      only each space's newest welcome tried; a joined welcome marked done. Measured after: remove → 36, re-add → 37,
      joined with 37 earlier epochs (one epoch each way; before: 12 → 17).
- [x] Reconcile (owner 10-07: any node advances any request; the delegate commits too): `conversation.reconcile` —
      in by the acts, not in the group → added, members who may invite taking turns (`ordering.turn`, moved from
      file-keys: one ranking), 10 min back-off; `letIn` act-first. Dry run first: fx/fy all 0 to add; Wren's spaces:
      Craftworks 5/5, Ivvor's New Space acts 1 / group 5, Ivvor's Space 1 / 7 — 0 to add (Wren reads few of Ivvor's
      acts: hypothesis, the history gap). Control: invite faulted after its act → reconcile "2 in, holds 1, 1 to add"
      → added at epoch 39, then 0 to add.
- [x] Commits as pure reconciliation (10-08, 8ba49e6): redo removed; a lost change is not replayed: removals come
      back by `moderation.enforce` from the acts, adds by `conversation.reconcile`; a refresh owed is a `${at}~refresh`
      mark, tried once per page until its commit lands. Snapshots kept for heal, no intent. fy WA test: Remove #2dk5VD →
      "1 member(s) removed: epoch 4", group 1 member at epoch 4, every table writing at epoch 4. B published (971 served).
- Next (not built): witnessed ordering when a claimed time must not be trusted.

## Owner's spaces lost after the identity change (10-07) — measured cause and fixes
- Cause (owner node log): a spacekeys tree block (3tE9PKK…) PUT 10-06 08:03 reached no peer ("could not take the
  stream … reporting success"), evicted 19:30; NotFound since. Our code then: (1) core tail_group errored when every
  group member was already held → page took "no group" → declared lost, never rebuilt; (2) a rebuilt block was never
  put back (tail_keep_put looked the contract id up as a cid); (3) one failed read marked the table "never made"
  forever; (4) tables kept only weekly. All four fixed (076680d, 222da0c); owner's old table reads whole again (187
  rows; 2ad6db93 + cd856115 rebuilt and put back). Owner moved to spacekeys-2 (mark, e59e66b) before the real cause was
  found; Craftworks via a re-welcome, RedFox by copying its row from the old table.
- Upstream: service report VT2SJW from the owner's node (PUT reported success with no downstream copy, then evicted;
  counts 68 / 1091 / 1067 per day 10-05..07). Share the code with the Freenet team on Matrix.

## 10-08 01:41 UTC — daily statement turned final (measured on fy)
- 2026-10-07: final once fx and fy pages republished after midnight (creator 3.5, keeper 1.75, ledger 7.83333333 — the
  same numbers as while running); 2026-10-08 running. A day is final in the ledger only when EVERY contributor's
  statement for it is (each page republishes a minute after load, then hourly).

## Devices: a stolen device cut off for real (owner 10-09: "first" — removal asks the words)
Measured 10-09: (1) a credential under the OLD key still counts after a word change (account::members: true);
(2) the card's device list is the data key's — any device rewrites it (fx: devices 1 → 0 by one put).
Design (no prompt for existing users): devices count only on the key log's CURRENT key; removing a device asks the
words and ROTATES the key (same words' next key), re-issuing credentials for the devices that stay; changing words
re-issues too; the device list lives in a DEVICES contract per DID (grow-only set of credentials, each signed by a key
of the DID's log; merge = union; nobody can delete one) — readers count those on the current key.
- [x] A devices contract (contracts-src/devices) + 4 tests, 5 mutants killed; devices.wasm 133,863 B
- [x] B account: `rotate` (same words), `reissue`, members on the current key only (the measured gap now refused)
- [x] C core: devices_put / account_devices / remove_device; change_words re-issues
- [x] D pages + delegate: readers (directory.devices, the delegate's re-key rounds) on the devices contract, the card
      only until one exists; each device adds its credentials; remove-device asks the words (rotate + re-issue: the
      LOG first, then the devices — put the other way the new-key credentials do not validate); change-words re-issues.
      Identity f73ea7d3 → 94190d1e (everyone enters their PIN once).
- [x] E LIVE fx/fy 10-09: one account on two devices (both see 2); fx removed fy with the words (341 ms) → devices:
      fx only; fy then put its old credential into the devices contract and rewrote the card → still fx only.
      (On the way: a two-device account works on fx/fy only when a test logs in as the UI does — logged out, join,
      accept, staying on the page; tests that skipped that made two groups or a locked first device. A catalog-sealing
      change made then was measured unneeded with the right flow, and dropped.)
- [x] F token writers: a `Writer` entry already takes the account's device credential (the same format, checked
      against the key log) — nothing new in the contract; the page wallet names its device's credential from the
      devices contract when it moves into the app. The token's own instance-id copy replaced by contract_keys::instance.
- LIVE 10-09, change words on fx (two-device account): 288 ms; the new words open it, the old do not; both devices
  still count, from fx and from fy (re-issued under the new key).

- EPOCH CLIMB (10-09, owner: "Craftworks#0cce26 doesn't have changes but epoch keeps on changing"): #HLt27K behind at
  46 of 56; its page asked up to 12 members to welcome it again on every load and EVERY member answered with a re-add
  (one commit each); it never caught up. Fix f9fbc48: welcomed again ONCE per epoch it asks from (writers record with
  the epoch), the repair ask answered by one member in turn, the delegate skips a stuck ask already answered past that
  epoch. HOTFIX LIVE (site v369): the page half only (conversation.js, upkeep.js) on the live base 354c60f — identity
  unchanged (94190d1e), no PIN; the delegate half ships with the next full release. OPEN: why #HLt27K cannot apply a
  welcome. Also found: SHA256SUMS was stale for token/devices since 5f32f7c/d8daf2f (would refuse the next publish) —
  updated.

- TELEMETRY HOTFIX LIVE 10-11 (site v373 on B8Y3…, owner: "more telemetry so you can investigate and monitor"; dfeda6a
  ported): hourly `seen` heartbeats (account prefix, version, each space's e/oe/on/unread), `member-keys` from
  observers with the cause (devices read or why not, announcements, writers size), "devices not read" no longer shown
  as "keys not announced", failed health writes logged, real account ids (were all "did:craf"), chained day bags.
  THE LIVE BASE (measured from its manifest, all 102 packages): wasm = a 354c60f build (core ce935ce9 was NOT
  reproducible — fetched from its pieces with `publish/examples/join_package`; mls 537d5ef6, identity 94190d1e,
  devices 30140424 = 354c60f's), page JS = 0a4f258 + this hotfix (conversation, index, directory, members-list,
  upkeep; `ROOM = 960` in index.js: the live core has no `chained_room`). Published with PUBLISHED_MANIFEST = the live
  manifest: 97 "already published", 5 sent. Not run on a private node first (the base identity differs from fx/fy's);
  the same code ran on fx/fy from main (v572/573).

- 10-11 STATEMENT ROOTS DIFFERED (3 roots for epoch 12): each page left the OTHER node's keeper out. Fixes: the
  statement build reads every keeper record fresh (`statement_build(..., fresh)`), a record that does not answer is
  asked again and the build refuses rather than post (no "read, not there" from silence), each build logs
  `computed root …, K kept of L listed (read …; not there …)`. ALSO FOUND: v571–574 were published to fx WITHOUT
  BEACON_CONFIG/TOKENS_CONFIG — the pages extended a new beacon from 0 (statements of epoch 0). Use
  `$CLAUDE_JOB_DIR/tmp/pub-fx.sh`. After v575: fx posted 149381c0 listing BOTH keepers (72-byte lines: the core
  carries decay; the published token.wasm is pre-decay → mints from it fail until decay is published).
- Wren's token-ops witness stopped at its 2 h limit 10-11 ~09:30 (not restarted: statements now witnessed by pages).

## AUDIT 10-11 (4 reviewers: token contracts, identity+delegate, design gaps, page/core) — fix phases
- [x] A 10-11 (tested; each fix's negative control run except the epoch window — its unfixed form hangs; contracts
      NOT published yet: re-keys $CRAFT, held until the claims-epoch-4 live check)  (small, clear): KeeperMint in the cut's received set (replay after cut); a forked chain's stake counts 0 (burn
      escape); chained readers continue past a byte-full bag; statement build refuses on a SILENT bag read; payment
      announcements kept for retry; delegate witness round keeps "not yet" entries (only settles NotSeated when drawn);
      PIN tries counted per target not reset by another member's unlock; epoch_log/Forget/HandoverKeys bounded.
- [x] B 10-11 (B2 8fe24a8, B3 7e84a53, B4 8922125; tested, controls run): allowance verdicts only under the epoch's
      accepted root (statement terms on the escrow); a member's witnessed places carried to its next build + retire
      clears its mandates; node-wide upkeep settings are the upkeep site's; saturating clock.
      MOVED TO C: stake counts only WITNESSED entries (acceptance recurses through other stakes: needs a design — where
      the recursion is cut, caching); TokenSign place guard (a guard alone wedges a wallet whose signed entry was never
      put: the wallet must re-send what it signed at a place).
- [x] C1 VALID STAKE 10-11 (built: `wallet::valid_stakers` one rule for pages, delegate and Wren's tool; tested — an
      unwitnessed staker drops out, its negative control run; the test lot's stakers are baseline-signed under a
      test-only `testlot` baseline key, absent from every real build — checked in the wasm) (TOKEN §7a "VALID STAKE", designed 10-11): `V(E)` = claims whose staker's pool chain at E's
      start stakes ≥ the claim with its key, is not forked, and has its last pre-E entry ACCEPTED under V(E′) (E′ its
      lot's epoch < E); cached per epoch. Every lot (seated, accepted_at, fees, statement_accept, wallet stake_of) draws
      over V(E). Fixes: fake stake (self-made mint / deep forged receive) and seat stuffing / baseline removal.
- [x] C2a 10-11 CLAIM REPLACEMENT closed: a registry holds one claim per (staker, key); `valid_stakers` takes the
      first per staker its stake answers for (test with a forger key sorting first; control run).
- [ ] C2b REGISTRY FILL (open, design): 4096 forged owners fill a registry so real claims cannot enter; one shared
      contract cannot rank claims it cannot check. Direction: a claim lives in the staker's OWN pool chain (a writer-
      signed entry, witnessed like any other), and the lot's index is a chained pointer bag of staker DIDs (a flood
      costs readers only, each pointer filtered by `valid_stakers`).
- [x] C3 10-11 LISTING + CLAIMS WITH NO PAGE OPEN (owner rule "nobody must stay online"): the keeper mandate carries
      the account (DID, device credential, key seq, codes, seat); the delegate's keeper round lists the account in each
      epoch's keepers registry (tested) and, on the seat node, claims its $CRAFT stake with `keeper::claim` (the rules'
      tested step; the delegate path is NOT unit-tested — it needs minted stake in a delegate fixture). The page makes
      the next 8 epochs' keepers bags while open (a delegate's new contracts do not propagate).
- [x] C4 10-11 RECORD CROWDING closed: a keeper's record (named by its key) binds that key alone
      (`standing::keeper_key`); a stranger's binding refused (test; control run). Re-keys standing.wasm at publish.
- [x] C5 10-11 VOUCH REGISTRY CHAINED (one bag an epoch: anyone with more work evicted every real vouch): chained bags
      (`bag::chained`, new domain — fx/fy's earlier vouches are left behind), page relay/read, token-net, token-ops.
- [x] C6 10-11 TOKENSIGN PLACE WINDOW (was B5): a token entry's place (seq + prev hash) kept with its wake-up; another
      entry there refused for 2 wake-ups (`PlaceBusy`: the racing tab reads again and builds the next) — after it, an
      entry never put does not wedge the wallet. Page TokenSign and the delegate's claim signing share it (test; control).
- [x] C7 10-11 DECAY NOT RESET BY ONE MISSED STATEMENT: activity carried from the newest ACCEPTED statement within
      CARRY_BACK = 8 epochs (page core + Wren's tool, one constant). NOT unit-tested (wasm-only binding): the live roots
      agreeing on fx/fy is the check.
- [x] C8 10-11 GATE SCHEDULE: the reputation gate halves with every issuance halving (`Pots::halvings` from
      `wallet::craft::halvings`; page core and Wren's tool). The halvings count is tested; the gate's effect on a
      statement is not (a test needs a reputation just under the gate).
- [x] C9 10-11 FORGET stops the member's upkeep (members list, all mandates) — tested.
- [x] C10 10-11 WITNESS SIGNATURES NOT EVICTABLE (a place's generic bag: 16 junk items of ~16 KiB with more work than a
      signature's evicted all 11 genuine ones — measured on the old bag): their own container, a token-contract kind
      (`witness::Params` = code, pool, beacon, the lot's CLAIMS EPOCH, chain, seq). It takes a signature only if it
      verifies over the place under its key AND that key is the baseline's or one of the epoch's claims REGISTRY (read
      as a related contract by the update; not needed when only the baseline's is new — the bootstrap); each key keeps
      its 2 lowest entries. Bounded by the registry, so nothing taken is ever evicted. Readers all go through
      `Params::sigs` (`rules::tokens::witness_sigs`); `Source/StakeReads/fees::Reads.sigs` take the claims epoch; the
      chain writer makes the empty container (core `witnesses`), pages PUT, the delegate UPDATEs. Test: 8,256 sybil
      signatures + forged/replayed ones + an equivocator's extras offered — every genuine signature stays, the proof
      too (control: registry rule off → 8,268 taken, unbounded). A held signature is never dropped by a merge (a node whose registry copy lags a claim would have). Re-keys token.wasm at publish. Follows C2b: if claims
      leave the registry, this gate follows them.
- [x] C11 10-11 DELEGATE ROUND STATE CHEAP: a round's secret is its index only, each state read its own secret written
      once (was: every answer re-serialized every state read); the witness round's settled list holds 32-byte item
      hashes, looked up through a set, written only when one is added (test: an answer never rewrites earlier states;
      an ended round's are blanked).
- [x] C12 10-11 STORAGE ANSWERS AUDITED BY LOT (agent): `standing::record::kept_audited` — 2 answers per kept keeper
      drawn at the next statement's lot, one pick each re-sealed from its piece; junk hashes → not kept (test; control).
      Known: Wren's token-net fails a keeper on a piece read that does not answer (liveness only: Wren then refuses the
      page's root that epoch); partial holders pass ~f² of epochs.
- [x] 10-11 v579 on fx: token/allowance/standing rebuilt (re-key), pay 2e1a823f / allowance 2f327949 (serve), Wren's
      witness restarted on the new binary (pool craft). Live recheck pending.
- [x] 10-11 JOIN STUCK "Requested" WHILE A MEMBER (owner in Ivvor's New Space, measured as Wren: listed a member,
      "keys not announced"; Wren behind 256 of 460, welcome for a package "offered by muvu9mfo, not held").
      (1) An OPEN space's `admitted`/`added`/`member` acts are PUBLIC (were sealed: the asker, reading only public acts,
      never learned it was let in, so a failed welcome was never asked for again); the owner's page publishes an open
      space's roster once a page. (2) "Let in ✓" on the join control. (3) Telemetry: `join-waiting`,
      `join-asked-again`, `admit-failed`, `reconcile` (pending), `reconcile-failed`, `welcome-again-failed`.
      (4) Welcome-again loop bounded (agent's read-only root cause: L1 re-added a group member on every failed welcome,
      no turn, no per-epoch gate — 460 epochs): asks carry the asker's epoch, a member is re-added once per epoch
      (`welcomedAgain`/`markWelcomed`); a welcome for a package this account JOINED by already (duplicate) is never
      asked about (`joined-kp:`); asks sent after the card is renewed; a join saves the group before dropping its
      package from the batch. fx/fy v581: fy outside an open space saw its admission in the PUBLIC acts
      (`join-asked-again` ced654 08:33), then joined. NOT DONE: welcomers sharing used-package tags (agent fix 3),
      resend-after-commit outbox (fix 6), member nodes honouring others' re-adds (delegate, needs wasm).
      LIVE HOTFIX v374 (10-11, B8Y3…): the same 5 packages (keys, conversation, roles, upkeep, join-button) on the live
      base tree (0a4f258 + v373), checked against the live manifest (only those 5 differed of 83 JS); 97 already
      published, 5 sent, identity unchanged. Ivvor's New Space unblocks once its owner's page runs v374 (publishes
      the roster) — then askers' pages see they were let in and ask again, once per epoch.
      v375/v376 (10-11): (5) a request in a listed space's door bag missing from the account's `asks` copy is put back
      and handled (Wren in Craftworks: "Requested" from the bag, never asked again); (6) `admit` answers a LET-IN
      member still in the group (an entry from a lost key package) whose request is newer than its admission: one
      re-welcome per request, inviters in turn, skipped when its device announced keys after the request (fx v583
      re-added fy who was in; fixed in v584, fy recovered). Live: Wren's page "request found … kept again", "let in …
      asked again" (09:04); the owner's page must load v376 to answer.
      v377 (10-11): shared used-package tags (writers pointers + `shared:` rows, into the node's mandate `spent`) and the
      welcome resend outbox — fx v588 ran clean; live sent conversation + upkeep only (100 already published).
- [x] 10-11 STRANDED ENTRIES (fx v584: $CRAFT chains' first entries never witnessed): (a) token-net cached a failed
      definition read for the process's life — Wren's witness started before $CRAFT's definition existed and refused
      every $CRAFT entry ("the token's definition not read"); now only reads are kept. (b) An entry still unaccepted when
      its queue ages out (witnesses read only this epoch's and the last) is queued again by its page (`toqueue/` kept
      until `entry_status` says accepted; a 5-minute pass). fx v585: epoch 18's seq 0 and allowance seq 5–6 re-queued
      into epoch 21, Wren signing them.
- [ ] C (design first): claims registry backed by stake (seat stuffing / replacement); registries chained + per-voucher
      caps; standing answers checked against pieces; record crowding; listing + claims with no page open; Wren
      baseline dependency; gate schedule; lineage cap; misdeed reputation; rule versions.
- [x] 10-11 STATEMENTS WITNESSED WITH THE PAGE CLOSED (local, not published): the delegate's witness round
      (`delegate/src/witness_statement.rs`) builds a seated epoch's statement by `rules::statement::build` over its read
      rounds and signs it only where its root is POSTED (page's encoding: `witness::statement_at` + `Params::state`,
      shared `witness_round::sign_into`); mandate + standing/Block code hashes; posting page makes the empty container;
      audit seals spread 1 keeper/call, verdicts kept in secrets, round paused/resumed. Tests: same root as the page
      path, signed on wake 2 (paused on 1); posted root differs → none (control: comparison disabled → it signs).
- [x] 10-11 V(E) FIXED AT THE FREEZE (local, not published; wedge measured: late stake acceptance grew V(5), unaccepted an epoch-20 statement a KeeperMint cited): witness sigs sign `at` (beacon height), container reads the beacon and refuses `at` ahead of it or back across a freeze by > AT_SLACK=16, keeps the earliest `at`; `valid_stakers` counts only sigs below `claims::frozen_at(E)` (`witness::accepted_before`). Tests + controls in token (container) and rules (wedge regression, pre-freeze valid).
- [x] 10-11 WELCOME PACKAGES SHARED + WELCOME OUTBOX (local, page JS only, not published): each welcome names its key package tag in the space's writers list (`{ w, welcomed, kp }`; node admissions absorbed with their `kp`), `welcome` skips tags any member used (this space fresh + every server space mirrored as `keypacks` `shared:<tag>`, also handed to the delegate's mandate as spent; health `welcome-pkg-shared`); a send failing after the commit is kept (`outbox:<space>|<did>|<tag>`) and resent by upkeep (2 min doubling to 1 h, give up after a day; health `welcome-send-failed`). Test: `node --test tools/welcome-packages.test.mjs` (5 pass; control with the exclusion removed: 3 fail). No node run.
- [x] 10-11 MISDEED REGISTRY → REPUTATION (local, not published): chained bag per epoch of self-proving EQUIVOCATION proofs (`rules::misdeed`, credential binds key→DID); every statement builder reads registries < E through `statement::Inputs::misdeeds` (core/statement.js, delegate, token-net); cheat rep 0 + never earns, direct vouchers charged DAMP·ramp/n for 30 epochs (`reputation::misdeed`); `token-ops misdeed --post`. Statement roots change; token.wasm bytes change on rebuild (reputation crate linked). False credit/mint/verdict not standalone: open (TOKEN §8).
- [x] 10-11 SUPERSEDE a wedged tail (TOKEN §3d; local, token.wasm NOT rebuilt): `Body::Supersede{from}` appended by the owner voids an unaccepted tail once `may_supersede` (past FRESH + a freeze past AT_SLACK) and no witness signed any of it; state rolls back to before `from`; voided entries never witnessed/cited/collected; page auto-supersedes from its witness queue (`wallet_supersede`), re-takes voided credits; delegate only witnesses (shared `may_witness`). Open: partially-signed tails, observer-dependent "no signature" (§8 item 7).

## App domains (owner 10-11) — docs/ARCHITECTURE.md §7 is the design
- [ ] WIKI: own (only you edit; fork / suggest), space (roles edit; sections as rows; conflicts kept); GLOBAL = one
      OWNERLESS COMMONS (anyone edits behind a reputation gate, full history, disputes and rules by Vote) — after Vote.
- [x] 10-11 WIKI personal + space (local, page JS only, not published, no node run): `wiki` app + `wiki-pages` (pages by title, sections as per-writer versions with base/merges hashes → heads, conflicts kept, suggestions accept/decline, history/restore, copy with lineage, propose back; `edit` policy on domain `wiki`). Tests: `node --test tools/wiki-pages.test.mjs` (27 pass; control with conflict detection removed: 4 fail; with merge carry removed: 1 fail) and `tools/wiki-flow.test.mjs` (in-memory service flow; fails with carry removed). Global commons not built (seam: `wiki-pages.of` refuses it).
- [ ] SHARED ITEMS: owner + editors + viewers per item (a small group per item), for Docs/Sheets/Slides/Notes/Wiki/Drive.
- [ ] CO-EDITING: CRDT over per-writer feeds; first MEASURE Chat delivery delay between two nodes; choose Yjs/Loro/Automerge.
- [ ] BLOG: a lens over Board's posts — article layout, blog home, drafts/scheduling, subscribe = follow, publications.
- [x] FORMS (10-11, local, not published, no node run): `packages/form.js` + `form-fields.js`; a response is a MAIL (owner 10-11) — to `space:<id>` (space mail, its owner+admins) for a space form, to the maker for a personal one; structured body checked against the sender; results = that mailbox; own copy = Sent mail (per-owner `seal_to` + table `forms` removed); results/summaries/CSV. Test: `node --test tools/form-fields.test.mjs` (12 pass; controls: addressing always to the maker → 1 fail, sender check removed → 1 fail).
- [x] BLOG (10-11, local, page JS only, not published): `#/blog` lens over Board's posts — article view (cover, reading column, reading time, contents, byline, tags, comments), homes (yours, a person's, a space's, Discover; newest, by tag), drafts = private posts flagged `meta.blog`, scheduled = published by `upkeep` on any open page, publication approval by editors' comments; `node --test tools/blog-press.test.mjs` (10 pass; controls: fences not skipped → 1 fail, non-editor ✅ counted → 1 fail). No node or browser run.
- [ ] FORMS: builder, answers sealed to owners, results and export (no polls — those are Vote's).
- [ ] VOTE: polls; proposals with transitive delegation, direct override, cycles, depth limit, snapshot finalization;
      electorate = a space's members; outcomes write the space's acts; delegation graph view.
- [x] 10-11 VOTE (local, page JS only, not published, no node or browser run): `vote` app + `vote-tally` — polls and a space's proposals as items, ballots/delegations/finalizations as append-only items (highest seq, tie by SHA-256 hash), direct > delegation, depth 10, cycles nothing, finalize = result + inputs' hashes (re-checked by readers), outcomes as the space's own acts (grant/assign/policy) by whoever may; delegation screen and SVG graph. Test: `node --test tools/vote-tally.test.mjs` (13 pass; 8 mutants each killed: direct override, cycle detection, depth off-by-one, hash tie-break, seq, finalization recount, poll close cut-off, poll eligibility).
- [ ] SLIDES (Markdown decks), then DOCS and SHEETS after the co-editing decision (one editor vs a CRDT).
- [ ] RESALE of editions on the item page — after the token is released and stable.
- [x] Telemetry: one collector in the loader for every app (live v378).
- [x] 10-11 WIKI, BLOG, FORMS merged and published to fx (v592–v600). Measured on fx: a wiki page created (links,
      history, what-links-here); Blog's Discover lists real posts as articles; a public form made by fx, answered by fy
      (as MAIL to the maker), results table read by fx. Found and fixed on the way: Wiki "nullnull" (null children);
      the shared AUDIENCE/ITEMS rule dropped an explicit "anyone" (items followed the author's own policy — fx's
      `comment: author` — while the control said "Anyone"): now one pure rule `audience.writeRule/withWrite`, test
      tools/write-rules.test.mjs with control; LIVE v379 (audience + items only). Forms: answers gated by the form's
      rule (comment door) and its mailbox address, refusals logged with their measured reason.

## Live release v380 (2026-10-12) — everything but the token
- Site craftworks v380, loader v37, through B (signer 2.3 s once B's beacon service was stopped; 1252/1262 pieces).
- Tree: main 22a2830 with CRAFTWORKS_TOKEN=off, devices.wasm (3014…) and mls wasm (537d…) = live's bytes (build drift,
  no source change); SHA256SUMS aligned in the release worktree ($CLAUDE_JOB_DIR/tmp/release).
- Verified on the live manifest: devices/mls/idlog/bag/feed/tail/register/sealed unchanged; no Nodes/Wallet/Network;
  login-only pages caption, chat, contact, drive, mail, note, settings.
- B: craftworks-beacon.service stopped and disabled (owner 10-12). fx/fy moved to B (/opt/craftworks-test, fresh,
  started only by $CLAUDE_JOB_DIR/tmp/fxfy.sh up) — MOVED BACK LOCAL the same day: this link (~100 KB/s up) made pages time out; fxfy.sh now starts the local pair on demand.
- Token: not released; most epochs still post a second root with an unwitnessed entry.
- Live v381 (2026-10-12, loader v38): main f18965c, token off, from release worktree release2 with devices.wasm, mls wasm
  AND identity.wasm = v380's bytes (identity f6108b6e kept: no PIN prompt; the statement fix in the delegate is idle
  with the token off). Carries: guest views not counted, extension errors filtered, Chat opens a late channel, the
  address bar follows the app (+ tab title), Vote member parts hidden from outside. 1189/1262 pieces, every package
  rebuildable; site/manifest 200.
- [x] 10-12 C2b CLAIMS IN OWN CHAINS + LIVENESS (local, not published): `Body::Claim` in the staker's pool chain, chained stake-pointer bags (CPU prefilter), signatures container admits a claim-writer key named with its staker, claim witnessed only on a standing answer that epoch, claimed in the window before the freeze (page + delegate); claims.rs gone, token/pool/swap.wasm re-keyed (TOKEN §7a, §8.6).
- Live v382 (2026-10-12, loader v39): release3 = main 4bf650c, token off, devices/mls/identity = v380 bytes (no PIN
  prompt). Carries the recovery fix: a words login on a new node joins the account (the key channel is no longer
  ruled absent by the sealed directory — that made fy create a SECOND group) and the app starts again once, keys in
  place. Proven on fx/fy (fresh account with a handle, its own space, a chat message): joined + 1 earlier epoch
  recovered, handle, space, message — no manual reload; also on the release tree itself before publishing.
  Also carries 8c2d17e (reports name the site; members-only refusal not a fault). 1179/1263 pieces.
- Live v387 (2026-10-13, loader v42): release7 = main 6161bac, token off, every wasm = v384's but core (repair).
  v385 had reload-looped on the owner's slow node (resume guard < load time) and was reverted (v386 = v384 content).
  Carries v385's content + the visit resume fix, ACCOUNT REPAIR (a forked account: the owner's 10-12 words login on a
  slow node made a second group, overwrote the channel's e/0+info and sealed the directory over — Settings > Security
  > Repair account keys on the ORIGINAL login; guarded to logins whose directory does not read), slow-node fixes
  (identity registration 3 min, earlier builds asked patiently, reconnect reload once/10 min, single draft flush,
  re-send on reconnect, …) and a 3-reviewer audit's fixes. Proven on fx/fy on the release tree: a v384 account
  upgraded (space, message, desktop, paste, frame, secret, bundles, recovery), and a reproduced fork repaired (fork
  login refused; after the original's repair a new device's words login joined the original group: space + message).
  Live manifest = the fx-proven one (139 packages, 0 differ). Old Mac node runs on 127.0.0.1:7511 for the owner's
  repair (old PIN there → Repair; then words on Hetzner).
- Live v385 (2026-10-13, loader v40): release6 = main c2b0a8e, token off, contracts/devices + packages/build (core,
  mls, identity …) = release5's bytes — every wasm package identical to v384 (no PIN prompt). Carries FOUNDATION 1-8,
  the desktop (dock, app viewer, Space app), apps in frames (Paste, sample), bundles (public, with editions when the
  token is on), drafts without secrets, failures handled in one place (total/strict/background/optional + build
  check). Verified on fx from the release tree (v686): recovery complete, paste. Live manifest after publish = the
  fx-verified manifest (139 packages, 0 differ); site/manifest 200.

## FOUNDATION (owner 10-12) — docs/FOUNDATION.md is the design; full reset at the end (no migration)

Build order (FOUNDATION §7):
- [x] 1. Faults + health outcomes — `packages/fault.js` (kinds, POLICY, attempt/met/absentOnly/say; one report per
      failure however many places it reaches); loader reports through it (`fault` package registered in the publisher);
      origins typed: node.ask (transient / cancelled, default WAIT.ask), login gate (login-needed), where (refused-expected),
      blocks.put (refused), storage reads (sealed, not-yet). HEALTH OUTCOMES (owner: "not only an error collector"):
      `visit` per page load (each page's outcome loading/drawn/late/stuck/said/failed/login + settle ms, faults by kind,
      reconnects, member/spaces), `stuck` at 20 s with what it waits on, node WAITS (slow ≥3 s, never answered, queued
      behind the get cap) summed in `tools/health-watch` ("slow waits, most time lost first").
      Verified: tools/fault.test.mjs 7/7 (control: a retry-every-kind mutant fails); health-watch tests 3/3; fx v642 +
      loader v19: 4 visits read back (/ 493 ms, /board 689, /chat 607, /wiki 682 settled). Not yet seen live: a `stuck`
      report and a `sealed` fault (the /board sealed failure of v641 did not recur on the next load).
      Kept until 7: the log-word matcher (apps still only log failures). Not released to live yet; not committed.
- [x] 2. Read machine, R1 + R2 — every tail has `state` ({unknown|absent|present|unreadable}) and `decided()` (first
      read's end; still unknown → read again with WAIT.answer). A SILENT read no longer marks the tail absent in the core
      (`core.tail_absent` only on the node's "not found"). R1: `write` needs a decided table — unknown → `not-yet` fault,
      unreadable → `sealed`; `beforeCreate`/a create only on `absent`. R2: `placeOf` says "unknown" for a catalog not
      decided; `catalogOf` reads (not "never made") when the directory is not decided. Capped feed views pass state through.
      Verified on fx v645 (live probe `tmp/pw/r1.mjs`: node answers dropped, a write to a fresh table): REFUSED as
      not-yet after 150 s, state stayed unknown; CONTROL with phase-1 storage: went ahead to sign the write (a create over
      a table it never read) — "signing: no answer". Full recovery (make on fx, words on fy): name, space, message, no
      reload. Telemetry fixes found by these runs: stuck watched from the page's START (a 32 s mount was "drawn"), a
      login wait not counted as stuck (loader v21).
      SEEN, for phase 6: fx (the account's first device) reads `tables` as sealed after fy joined (epoch moved) — /chat,
      /board fail until a reload once the key arrives (same on v641, before phase 2). R3 + a failed app retrying fixes it.
- [x] 3. Lifecycle mechanism + account group + session — `packages/lifecycle.js` (MACHINES as data; `transition`
      checks from-states and guards: illegal → `bug`, guard no → `refused`, both thrown; one `lifecycle-<machine>` report
      per move; `on`, `snapshot` → the visit's `life`). ACCOUNT in keys.js: login → CONFIRMING_CHANNEL (channel
      `decided()`), then `create` (guard: fresh ∧ channel absent) or `walk` (guard: not fresh ∧ present ∧ info) → JOINING
      → MEMBER; `fail` → JOIN_FAILED; `loaded`; `removed`. `keep()` writes e/0 + info ONLY in CREATING. SESSION (shadow,
      nothing gated yet): open (auth), keyless (keys.ready with no group held), lock, removed; KEYLESS → OPEN when the
      account turns MEMBER. Verified: tools/lifecycle.test.mjs 4/4 (control: the create guard removed fails 2);
      fx v646 + loader v22: registration CONFIRMING→CREATING(channel absent)→MEMBER, words CONFIRMING→WALKING(present,
      info)→JOINING→MEMBER epoch 1 / 2 nodes; recovery complete (name, space, message). NOT built (later): COMMITTING
      for device removal, MEMBER_PARTIAL_HISTORY; space machine (phase 4).
- [x] 4. Gates and jobs — `keys.writable()`/`mustWrite(what)`: the account's writes (card `publish`/`renew`, requests
      made again, stuck asks, asks to be welcomed again) only from an OPEN session whose account group is MEMBER (else
      `refused`, or `not-yet` while joining). `lifecycle.job(name, {every, needs, run, delay})`: runs only while needs
      hold, PAUSED with why otherwise (pause/resume/stop reported, each run quiet); upkeep (needs writable), activity,
      keep, video leases, usage flush + statements moved onto jobs. SPACE machine (recorded): HELD/BEHIND/OFF_BRANCH/
      NOT_HELD/UNREADABLE/REMOVED from upkeep's selfState each round (`become`: only on change), WELCOME_FAILED{reason:
      account-keys-unreadable | not-past | package-not-held | branch}. Verified: lifecycle tests 6/6; fx v647: recovery
      complete; gate probe on fy — card written while keys held, then session KEYLESS: publish and renew REFUSED,
      upkeep job PAUSED. Left as they are: UI tickers (header, theme, video player — tied to their elements).
- [x] 5. Page/app/save/delete — `packages/drafts.js`: every text field in the body saved as typed (2 s pause, at once
      when hidden) to the account's `drafts` table and put back into an empty field when opened again; dropped when sent
      (Enter, submit, a button of its component) or emptied by the app; never a password/PIN/words field, anything in a
      dialog, header or search. An update is a resume (drafts flushed, then reload; only an open dialog holds it). Each
      body component gets `ctx.signal` (aborted when replaced) and an `app` machine (opening/open/closed quiet, failed
      reported). One SAVE line in the loader for writes the person asked for (saving… / not saved: the fault's words).
      DELETE machine + `life.deleting()`: items (hidden), spaces left, devices removed (gone), refusals recorded.
      Verified on fy (v650/652): typed in a space chat, tab closed, opened again → text back; sent → opened again, box
      empty. CONTROL (send/sweep handling removed): the sent text came back. Full recovery complete (v652).
      Moved to phase 8: installed apps (manifests, installed table, frames). FOUND (pre-existing, for phase 6): a space's
      chat opened by a DIRECT link shows no channel — chat.js `at()` reads `space.mine()` before the spaces table is
      decided, finds nothing, never looks again (same on the phase-4 build).
- [x] 6. R3/R4/R5 — R4: a failed tail, merged table or directory open is never cached; a tail opened as never made
      (`known:false`) is READ when asked for without that assumption. R3: on `craftworks:keys` / the account turning MEMBER,
      unreadable tails are read again IN PLACE (what reads them redraws); tails opened with no key are dropped only when
      their key is now held (no reopen loop), with the merged tables and directory built on them, and the page RESUMES
      (drafts flushed, reloaded where it was — every capability cache built on them fresh), at most every 30 s. R5: merged
      tables have `state` (unreadable while a writer feed did not open) and `decided()`. `space.mine()` waits for the
      spaces table to be decided (fixes a space's chat by direct link showing no channel). A page that failed sealed /
      not-yet / transient is shown again after a pause (4 tries). The reload after a words join is gone (keys event).
      Verified on fx/fy v654: words recovery complete — name, space home, chat message — and the page was NOT reloaded
      (marker survived); fx after the join: /chat and /board load (before: sealed until a reload); direct-link chat shows
      the channel and message (v653). The first try (v653) looped re-showing the page on every keys event (keyless tables
      reopened while still keyless) — fixed before commit.
- [x] 7. Apps on faults — `ctx.failed(step, e, what)` (the trace line + a report by the fault's kind) in place of the
      127 `ctx.log(…${e.message}…)` failure lines across 37 packages; the loader no longer reads log lines for words
      ("failed", "could not"). `ctx.say(e)` for 26 messages shown to people (a kind's words — log in, still arriving, not
      answered — else the error's own; plain refusals keep theirs). The private retry loops on `fault.attempt`: the
      channel wait and the epoch-log reads (keys), an earlier identity build's silence (auth). NOT converted: the ~546
      `.catch(() => null|[]|{})` reads (each needs its own judgment; R1 keeps writes safe) and statement.js's 40 rounds
      (token work). FIXED while checking: merged tables decided only once every writer's feed was gathered (a recovered
      space read "Not listed" — its list decided after 1.5 s); the visit report never waits on the spaces list (phase 6
      made it wait up to 120 s: pages closed before reporting); one job tick at a time (a resume from RUNNING, reported
      ×11 — control: the old runner fails the race test). Verified on fx/fy v656–657: recovery complete without reload,
      visits arrive, no lifecycle bugs; tests 14/14.
- [x] 8. Desktop — a space keeps its ENABLED CAPABILITIES (`roles.enabled/enable/capabilities/base`; carried by the
      existing `app` act naming the capability — no core change); chat, mail, contact are its BASE (always on, never
      off — owner 10-13); each app's manifest names what it `needs` and `roles.apps()` derives from that. SPACES app
      (`#/spaces`: your spaces, make, join; `#/spaces/<id>`: its capabilities, the apps that show it, its settings).
      INSTALLED apps (`installed` table; every app until removed; system apps always) and the APP STORE (`#/store`);
      Home shows the installed ones. Header: the app's name only — no space dropdown, no app dropdown. WHERE an app is
      is the APP's own (`places`: one list; Chat draws a Discord rail, Board and Blog a subtle "in: …" line; Video,
      Audio, Book, Wiki … show spaces in their own terms with no switcher; Contact lists every space's people). FIXED:
      grants asked in batches of ≤32 (the identity delegate refuses a longer list WHOLE: the 33rd table broke every
      write); a merged table's writer feeds that failed before keys arrived are gathered again (recovery read the space
      as "Not listed"). Verified on fx/fy v660–661: recovery complete without reload; Store remove → Home drops it →
      install back; Spaces app (base always on; enabling wiki → Wiki shows it); chat rail, board line, no picker,
      Contact's space sections. NOT built: apps in their own frames and apps not ours (the Store's catalog is the site's).
- [x] 9. Leftovers (no reset, owner 10-13) — Discover's listed spaces shown only if their public acts read and one
      counts (`items.publicSpaces`; once a page per space); hidden ones counted (`leftover` health report). fx: 3 of 8
      hidden — exactly the 3 with 0 acts AND no public description (two independent checks agree); the 5 that open
      shown. People listings already dropped unopenable cards. Also fixed: "@layer …" text shown to people in three
      places (contact's post count, account's block line, the delete confirm) — left by an earlier stylesheet edit.

## After the foundation (owner 10-13: work through 2–5 before releasing the new structure)
- [x] 2. Leftovers — Discover lists only spaces that open (727f78f).
- [x] 3. From the telemetry (fx): R4 one layer up — the loader no longer keeps a package whose load/start failed, and
      `edge` (pins, labels, people, prefs) and `moderation` lists no longer keep a failed table open (a label-menu that
      failed as a recovery's keys arrived failed Board and Chat until a reload: `lifecycle-app OPENING>FAILED`,
      `sealed label-menu`). `space.find(id)`: "not one of yours" only once the list is COMPLETE (space Home and Chat; a
      recovered device's Home said "Not listed" while the other device's list arrived). Job pause/resume quiet (×25 guest
      noise). Verified v664: two words recoveries complete, Board and Chat on the recovered device, no fault reports.
- [x] 4. Apps in their own frames — `apphost.js`: a page published as BYTES runs in a sandboxed srcdoc frame (own
      opaque origin; CSP `connect-src 'none'` set before its code; its module a data: URL) with the SYSTEM API by
      messages, the page enforcing each call: its OWN rows (the account's `appdata` table, keys `<app>/…`), places,
      where, go(#/…), say. `sample-app` (🧪 Sample, Developer) proves it. fx v665–669: its row kept across a reload;
      another app's row in `appdata` NOT seen; from inside — fetch, WebSocket to the node, the page's DOM: blocked; an
      unknown call refused. Network control: with connect allowed in our CSP the frame is STILL blocked — Freenet's
      gateway CSP on the app page allows only the node (`connect-src http://127.0.0.1:<port>`; even the outer page cannot
      reach a CORS-open server) and the node refuses the frame's opaque origin; our CSP's own effect is not separately
      shown. NOT done: built-in apps moved into frames (they hand each other DOM and live objects — needs serializable
      capability APIs first); the App Store offering apps not ours (a catalog of published app packages).
- [x] 5. Silent reads counted — `ctx.quiet(make)`: a read's `.catch` keeps the SAME fallback it had, but a failure
      that is not absent / not yet / logged out / cancelled / an expected refusal is reported by kind (`swallowed`).
      460 sites in 75 packages converted (not: sequencing `queue.catch(() => {}).then`, optional `require`, health
      itself, code without `ctx`, the token files). fx v670: recovery complete, 11 page loads all drawn, ONE swallowed
      failure surfaced (sealed `tables` on /chat while a recovery's keys arrived). Test stubs gained failed/quiet/say.
      SUPERSEDED 10-13 (owner: "if it is one machinery, why still multiple implementations?" → "implement before
      release"): `ctx.quiet` removed. Capability reads TOTAL at their export (`ctx.total`, empty value + `.state`),
      their callers' catches dropped (152 by hand + 41 by codemod); every other fallback rewritten by an AST codemod
      to `ctx.background` (220) or `ctx.optional` (231), incl. the raw `.catch(() => v)` / `.then(f, () => v)` sites
      that were never counted; tools/failure-check.mjs (build.sh) refuses any new one (positive control: 484 sites on
      the pre-change tree). Found on fx: a TDZ in the loader's `retries` (bbd3d0c) failed /chat once per visit →
      declared before the first show. apphost's frame toolkit classes renamed (cw-app-card/-loading: style-check
      failed at HEAD). 86 tests; fx v684 / loader v38: recovery complete (no reload, message back), paste, frame
      boundary, secret not kept, bundles public list, desktop.
      Review (10-13) found decisions acting on a failed read as an empty one (re-join an already-joined space, a ban
      act with no nodes, every Drive file re-keyed, a month's claim marked done unclaimed): `ctx.total(...).strict`
      throws as before — 257 sites that relied on the throw at HEAD (aligned per callee, per file, against HEAD;
      every service call without a fallback, and handlers that used the error) + 5 decision sites that already fell
      back to empty at HEAD (file-keys pass, readd stale, departed, claimOwn). 86 tests; fx v685/loader v39: all probes.
- [x] App-owned structure (owner 10-13: "the space app only manages capability; the view is the app's") — every app
      lists its GROUPS in its own menu (`where.tabs`: yours + each space enabling what it needs; Contact opts out — all
      spaces' people in one list); a member's space Home is the Spaces app's page (capabilities, people, settings).
      SETTINGS split: a capability's RULES (who may read/post/edit/comment …) beside its switch in the Spaces app
      (`app-settings.capabilityPage`); an app's OWN settings inside the app (Board's rules, Blog's approval, Chat's
      channels — `…/settings` in its menu, `where.settings`); the Settings app keeps the space-wide section. SETS are
      BUNDLES: one item kind `bundle` (its own capability), members of any kind in order (`bundles.js`), the BUNDLE app
      keeps them per group, "Add to a bundle" on every item's page. (A side-table `collections.js` and per-domain bundle
      kinds were built and replaced the same day: tags ≠ sets; sets cross capabilities.) Spaces panel removed.
      Verified fx v671–672: space Home → Spaces app; a capability's rules shown; Wiki's menu lists the space; Board and
      Chat settings in-app; Contact without group tabs; a words recovery complete; a bundle across a note and a post —
      added from their pages, reordered, one taken out.
- [x] Apps into frames, 1: PASTE (owner 10-13: every app framed; the system — loader, header, Home, Spaces, Settings,
      Store, capabilities — stays in the page). `apphost` v2 system API: where/read/get/submit/edit/remove (only the
      app's manifest `kinds`), pageOf/linkOf, tabs (its menu), go, copy, say, own rows, lib (a pure library's data:
      `highlight`), route and change events; toolkit `api.ui` (h, loading, card, action); the theme copied in; the frame
      sized to its content; the ITEM PAGE is the system's, drawn under the frame for `…/p/<ref>`. Paste rewritten on the
      API alone, published as bytes. fx v673: framed; 15 languages via lib; a paste saved → its system page under the
      frame with Raw / Copy link; raw view; listed; making a note from Paste's frame REFUSED.
- [x] RELEASE AUDIT (10-13: 4 reviewers — storage/keys/lifecycle, loader/faults/telemetry, desktop/apps, frame
      security — plus release mechanics). FIXED: drafts would have kept RECOVERY WORDS (account fields without a secret
      name; never live) — no field of the account, settings or login screens is drafted, names/ids/placeholders/labels
      checked; a framed app could send data out by a POPUP — sandbox scripts+forms only, writes and copy need the
      person's own click/key (the page's user activation), audience allowlist, level/gate/write/editions/members never
      set by an app, remove only your own, a call budget a second, row caps, a frame that navigates away is closed;
      merged table decided only once every writer's feed ANSWERED, and UNREADABLE while one is sealed (a recovery read
      "Not listed"); upkeep.tick checks it may write (called directly by Chat and a space's Home); a registration's
      channel opens known absent (a slow node left the group unmade); a long read undecided not retried within a
      minute; swallowed transient/not-yet failures not reported one by one, ids out of report keys; a failed `index`
      load no longer silences reports; the keys-arrived resume at most once a minute (window.name); Board's post rule
      read in the text domain; older app acts count their apps' needs (and turn off with them); bundle tools for its
      author only; app settings pages refused to non-admins by address too; the dock skips token packages a site
      lacks. MECHANICS: token-off build idle (no load failures); no grant prompt for the 3 new tables (home app);
      core/MLS/identity wasm unchanged (no re-login); release must be built from a clean worktree at HEAD (the tree holds
      uncommitted token edits). Verified fx v683: recovery complete without reload, framed Paste and Sample, frame
      boundary, drafts never keep words (control: a plain field is kept).

## ONE OWNER PER FACT — the duplicate sweep (owner 10-16: "continue the fix until completed")
Audit 10-16 (two sweeps, file:line in the session): most facts HAVE one owner; copies grew beside it. Fix = delete
each copy onto its existing owner + a build check per owner. Only gap with no owner: writer → person, up to when.
- [x] 1 WRITERS (26b4fd4, fx v809 proven; NOT RELEASED): `departed` (space table) = the one record of a writer that stopped being a member's (member removed,
      OR device dropped from the account — new), with its heads; `storage` scope `writersOf` = the one map (current
      devices ∪ departed), used by roles (act authors) and storage (feeds); outside readers via `pub-departed`; the
      account's page records its own former devices it finds (repair: owner's Craftworks space, 57 acts by b262cd…).
      + build check: no writer map built outside it. Rust delegate reader (table.rs) on the same record.
- [x] 2 LIVE BUGS (734a463; NOT RELEASED): delegate write.rs bag_item → table::seal_writers_item (OPEN); tag.js merge order; loader.js:547
      raw follow → the door.
- [ ] 3 ACCESS COPIES onto Governance/roles: act-permission table copies (roles.js:380, vote-tally.js:268),
      hand role checks, vote ownerAt/adminAt, creator-as-owner checks, who-reads (file by app vs post by domain),
      bans/lapsed replays, credential byte parsing → identity::read_credential.
- [ ] 4 MECHANICS onto their owners: node reads (blocks/files/loader) → node door; retries → fault.attempt;
      timeouts → WAIT; Map caches → ctx.cache; months → index.monthOf.
- [ ] 5 REFS/COUNTS/KEYS/NAMES: refs → items pure + one builder; table names → space.tableOf/table_name;
      counts → items.tally; ranking → RANK; sealing key choice (write.rs vs JS); names → directory.shown.

## MEMBERS AND KEYS (FOUNDATION "Members and keys", d85f8dc) — supersedes task #37 and every join/catch-up patch
- [ ] G1 the key-follower in the core (Rust), tests over broken states (behind, never announced, forked, dropped
      devices, strays): one commit each.
- [ ] G2 page and delegate on it; "my key" the one member message; the old ask/re-welcome/catch-up paths deleted;
      group-check: key add/readd/remove only in the follower.
- [ ] G3 every kind's member list on it (space, PRO/VIP, chats, audience, circle, account).
- [ ] G4 Ivvor's New Space repaired; done when health shows all 7 members at its epoch.
UNRELEASED on main (live = v398 b9f4951): batch 2 734a463, batch 1 26b4fd4 (fixes the owner's Craftworks space
missing from Discover once released and opened — its dropped device recorded by the `departed devices` job).
