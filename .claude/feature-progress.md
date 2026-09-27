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
  retire acts, scopes) + profile row + vault (owner key wrapped, on the network).
- **Members, by device + PIN:** the DID is the ACCOUNT; a MEMBER is one device key admitted to it, opened on its
  device by a PIN. "Log in with this device" mints a member key (and, for a new person, the DID), the person sets a
  PIN; the identity delegate stores key + DID + PIN hash + home app as ONE record. One device holds many members
  (a shared computer): each PIN opens its own member, of the same DID or of different people's. No two members on a
  device share a PIN (a refused taken PIN counts as a guess). 5 wrong PINs in a row lock PIN unlock on the device;
  a member's key file sets its PIN anew. Each app unlocks its own session (random token in page memory). BUILT in
  identity/ (13 tests).
- **Auth** = ways to get a member into a DID: this device + PIN (new DID, or another member on this device), device id
  + PIN pairing (a member on another device), import a key file, keycraft (pinned surface).
- **Device id + PIN (pairing, a second device):** device id = an admitted device's public key. On it, "add a device" shows a one-time
  PIN (its identity delegate stores it with a try counter). The new device encrypts {its own key, PIN} to that public
  key and writes it to the admitted device's inbox. Only that device's delegate can open it: it checks the PIN (a few
  tries, then the PIN is void) and admits the new key to the member Set and answers with the DID; the new device stores it with its own key and sets its
  own PIN. The PIN is never checkable offline, so a
  6-digit PIN is enough. No server, no email; the admitted device must be online.
- **DID + passphrase (any computer; also recovery):** the vault holds the owner key encrypted under a slow
  passphrase hash (argon2-class), on the network at an address derived from the DID. Anyone can copy it and guess
  offline, so it takes a long passphrase (several random words), never a PIN. Unlocking it gives the owner key, which
  admits this computer as a session member (forgotten at logout) or, after losing every device, admits a new device
  and retires the lost members. Forgotten passphrase: guardians (Shamir shares of the owner key).
- **Session members:** a public computer's member is admitted as class "session"; logout retires it in the member Set
  and deletes its key from that computer's identity delegate (needs a Forget request: TODO).
- Passkeys: impossible in freenet's app frame (measured 2026-09-27: opaque origin).

## Phases (each: build → private node → publish through B → commit)
- [x] A. identity delegate (members by PIN, sessions, sign with seq guard + origin rule) + page `identity` + `auth`
  dialog (asked only by pages that need it) + first login makes the DID (owner seat + passphrase vault). Verified on a
  private node (create, wrong PIN counted, same PIN reopens, a 2nd person on the same device gets another DID);
  published through B as app site v5. Pages must connect to the host they were served from (localhost ≠ 127.0.0.1).
- [ ] B. data: device tail per app (identity signs), subscribe, write rows, flush into the tree, read tail→tree
- [ ] C. DID records: owner seat + member Set + vault; create identity; admit this device
- [ ] D. auth package: on-demand sign-in dialog; import/export key file
- [ ] E. auth: keycraft
- [ ] F. auth: device id + PIN (pairing through the admitted device's inbox; needs C)
- [ ] G. republish Craftworks under the new identity; retire the old signer
