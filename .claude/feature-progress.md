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
- [ ] B2. flush the tail into the prolly tree; tree blocks as erasure piece sets.

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
