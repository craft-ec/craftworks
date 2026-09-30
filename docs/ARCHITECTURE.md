# Craftworks architecture

Settled with the owner, 2026-09-28. This is the shape everything is built to; code that does not match it is a bug.
Where the code is today and what is next: [Implementation plan](#implementation-plan) (and `.claude/feature-progress.md`).

## 1. The one division: capabilities and apps

Every package is exactly one of two kinds.

- **A capability** has no UI. It defines a **shape** (what a kind of thing is) or a **rule** (what may be done to things),
  once, for every object it applies to. Capabilities never know which app uses them.
- **An app, page or component** is UI only. It composes capabilities and owns no data model of its own. An app is a
  LENS over the one global data set: two apps over the same data are two views, never two copies (Board and Blog are
  the same posts, as a feed and as articles; Chat and Board on one space share its members, roles and bans). A new app
  adds a view, never a store or a mechanism.

**Spaces and the rail.** Every account has a PERSONAL space (the whole canvas: Messages, Mail, Contacts, Notes, Board as
your own posts — your profile —, Account as its settings). A SHARED space is made and joined by invite: one governance log
(members, roles, invites, bans, which apps it uses) that every app on it reads. A permanent rail lists Personal (the
default) and the shared spaces; each space has a Home listing its apps; an app shows only the space selected — a board is its space's posts only (no feed across spaces). Messages
is personal only: conversations between equals, no roles — who needs governance makes a space.

**An app's three views.** An app declares (in the manifest) up to three views, each optional — PERSONAL (the account's
space), SHARED (a given shared space) and PUBLIC (DISCOVER: the ownerless public network, second on the rail) — and the
rail composes itself: Personal's Home lists the apps with a personal view, a shared space's Home those with a shared view
that the space added, Discover's Home those with a public view. Capabilities never know the view: they take a SCOPE (the
account, a shared space, or the public index — bags that public spaces and posts list themselves in), so one lens points
at three scopes and no view gets a store of its own. A SHARED view never crosses spaces; the PERSONAL view may gather
across every space the person is in (it is them: Board's Feed, unread across spaces). Discover has no owner and no log: what shows there is filtered by
the reader's own blocks and the signed moderation lists they follow.

A capability that applies to things applies to **every** kind of thing: moderation moderates a post, a comment, a vote or
a label alike; governance decides who controls one note or a whole company.

**Items: one structure, composed per app (owner 09-30).** A SPACE is the place (personal: your profile, public or
only you; shared: its room, public while it reads in public) — shown in each app's own words (Board: a community,
Videos: a channel). An ITEM stands on its own; its KIND says what it is (`kinds`, after handcraft's content
architecture: domain derived ← kind with its fields ← context = tags ← bundles later) — `post`, `video`, `movie` …
are SIBLINGS, never parent and child. ATTACHING kinds are about another item (`in`): a `subtitle` on a video, and
comments on anything. VOTES, public copies and the feed are the same for every kind (`items`). EDGES are a person's
links to things (follow a person — their personal space — or a shared space; pin = save; block). An APP is a lens on
a domain: Board shows posts, Videos the video kinds, Subtitles the subtitles.

## 2. Primitives (on the network, or on the node)

| Primitive | Is | Used for |
|---|---|---|
| **Block** | immutable, content-addressed | tree nodes, parity (erasure), attachments |
| **Tail** | one writer's signed sequence | a table's head and newest rows |
| **Log** | an ordered, self-certifying chain of signed events | an account's key log (the DID); an object's or space's log of acts |
| **Register** | one signed value | whoami (words → DID), site versions |
| **Set** | small owner-signed items | not used (the account's member list moved to its MLS group's roster) |
| **Bag** | unordered pointers ranked by proof of work, no signer | every index: inbox, directory, comments |
| **Site** | a web container of erasure-coded pieces | app delivery |
| **Delegate** | node-local secrets | identity; MLS state |

## 3. Capabilities (18)

### Object shapes

| Capability | Shape |
|---|---|
| `content` | an authored item with its own identity: note, post, comment, message, document |
| `edge` | *me → object*, a small value: pin, label/tag, like, vote, follow, save, block, mute |
| `space` | an object with members: a chat, a group, a community, a company, **your account** |
| `index` | pointers to objects: inbox, directory, comments on a post |

### Over any object

| Capability | Rule |
|---|---|
| `governance` | who holds authority over an object; inherited from its container (realm → space → object), overridable per object |
| `roles` | named permission sets through which authority is held: built-in, published templates, or defined in one space |
| `access` | who may read (holds the key) and write (a role that permits it); a site acts only with the person's grant |
| `membership` | who belongs: joining, leaving, inviting, removing. Your account's members are its nodes = its MLS group's roster (one list; each credential names its node key) |
| `administration` | an object's settings and policies |
| `moderation` | acts on objects (hide, remove, label) and on actors (mute, ban) |
| `ordering` | one agreed order of entries on an object: positions 0, 1, 2 …; one entry per position; a lost tie is told, re-reads, retries. A position is an MLS epoch, a snapshot number, a membership version. Several TYPES by who writes and how a tie is decided: `tail` (writers sharing one key — your account's nodes; built), `log` (a space's members, each with their own key; lowest hash wins), `witnessed` (k of a known witness set co-sign) |

### Underneath

| Capability | Owns |
|---|---|
| `node` | the connection to this machine's freenet node |
| `identity` | what an account IS: its DID and key log, which account words hold, changing the words; and the identity delegate on this node — members (node keys), PINs, sessions, signing, the kept MLS state and epoch secrets; and UPKEEP with no page open (node ≥ 0.2.139 wakes it every minute, with the person's Background grant): it admits askers into spaces by the MANDATE the page hands it (crate `delegate`, built as identity.wasm) |
| `auth` | sessions, PIN login, recovery words, changing words (no UI: the dialog is `login`) |
| `keys` | table keys per epoch (MLS), handed to granted sites; escrow of epoch secrets to the recovery words. The MLS protocol runs in the page's core — and in the identity delegate for upkeep (mls-rs vendored: no browser randomness or clock forced; the delegate's come from the page: a pool it stirs, a clock it sets). The identity delegate is the vault — it keeps the member state and the epoch secrets, and derives table keys for granted sites |
| `storage` | tables: tail + tree, one sequence per writer merged by readers, sealed nodes, catalog, range and change-only reads |
| `blocks` | the one door for tree blocks: fetch raced against parity, rebuild, put |
| `keep` | lifecycle: re-publishing signed states, retention, flushing when quiet, health |

## 4. Apps, pages, components (13)

| Kind | Items |
|---|---|
| App shell | `wrapper`, `loader`, `trace` |
| Components | `theme`, `header`, `footer`, `login`, `pin-button`, `label-menu` |
| Pages | `home`, `account`, `notes`, `chat` |

**`theme`** is the one place the look is decided: design tokens as CSS variables (colours, surfaces, lines, accent,
spacing, radius, type, shadows), light and dark (following the system), and base styles for text, buttons and inputs.
The app's manifest names its theme and the loader applies it before anything mounts. Every other component and page
uses the tokens, never its own colours or sizes — so a new look, or a second theme, is one package.

## 5. Decisions the shape rests on

- **Identity.** The DID is the id of the account's first key event (`did:craftec:<base58>`); keys rotate under it by a
  self-certifying key log (each event reveals the key the one before committed to). Recovery words can be changed; the
  DID and the data stay. People are shown as **handle + id**: handles are not unique.
- **Your account is a space** whose members are your nodes. Its log is the key log.
- **A DID is the member; a device is a way to sign in** (owner, 2026-09-28). Every other space knows PEOPLE: its group
  has one member per DID, whose keys are the ACCOUNT's — kept sealed in the account's storage, so any device logged in
  to the account uses them (as it uses the data key). Adding a device is logging in; removing one refreshes the
  account's member in each space (from a device that remains), so the removed one reads nothing newer. Two devices
  changing a space at once: the epoch log's order decides, the other reloads. WRITING stays per device (every writer
  its own feed): a DID's devices write their own feeds in a space on its behalf; the DID's card lists its devices'
  credentials (signed by an owner key of its key log), so readers gather every member's devices and attribute what
  they wrote to the DID. (Measured 2026-09-29: one DID-wide feed written by two devices lost writes both ways.)
- **Ordering.** Shared state changes by acts in an ordered log; an act counts only if its signer's role allowed it at
  that point. First: one log per object/space with a deterministic tie-break. Later, when needed: witnessed snapshots
  (a known witness set, quorums that intersect).
- **Roles are per space** (holding a role never crosses spaces); definitions are built-in, published templates, or local.
- **Confidentiality.** Every private table is sealed; its key is **MLS export(epoch secret, table)**, the epoch being its
  generation. MLS is the one key manager: a space's members (one per DID), or your account's nodes. Epoch secrets are also escrowed
  to the recovery words, so the words alone recover history. Newcomers' access to history is a per-space policy.
- **Sealing covers whole tree nodes**, so keys stay ordered inside sealed blocks (range reads work) and counts and shape
  are hidden. Each block (node, value, parity) is sealed whole and lives in a **Sealed** contract at a keyed hash of
  its id, so only the account can name it; the tree library is untouched (it works on the blocks in the clear). The
  tail's rows are sealed one by one; a flush opens them into the tree. Rows and blocks carry which key sealed them
  (the table's own key for the `mls` channel, else an epoch), and are only ever sealed over to a newer key.
- **Federated data.** Nobody writes into another's store: a message, comment or reaction is content in the **author's**
  table plus a **pointer** in the target's index (sealed for private use). A group's data is one table per writer,
  merged by readers.
- **Writers: every writer its own feed** (Scuttlebutt's lesson). A writer — a node now, a member of a space later —
  writes only its own tails, one per table, under its own key; nobody writes another's. A reader merges the feeds of
  the writers that count.
- **Versions are causal** (Matrix's lesson). Every row carries its id (writer, the feed's sequence) and the id of the
  version it replaces, so readers order versions without a clock; a delete is a version too. Concurrent versions are
  settled the same way everywhere: the longer chain, then the higher id.
- **Membership is self-certifying and gossiped** (Nostr's lesson). A node's credential is signed by the account's owner
  key and checked against the key log; a removal is signed by the member that made it. Each node lists every node and
  removal it knows in its own feed; a reader starts from its own node and takes the union, so no single writer can hide
  a node or a removal. A removal first moves the removed node's winning rows into the remover's feed, then the
  removed node's feed stops counting. In a SPACE, a person who departs (removed, banned, or left) keeps what they
  wrote: before the commit that takes their nodes out, the member making it records where each of their feeds stands
  (the space's table `departed`: node → its feeds' last sequence), and every reader counts those feeds up to there —
  nothing after, since a departed node still holds older epochs' keys. Ordering is the one exception to "every
  writer its own feed": MLS epochs admit no forks, so each epoch's commits keep one sequence — a tail signed with a key
  derived from that epoch's secret, so only the nodes in the group at that epoch can write it, and a removed node can
  write nowhere that counts.
- **A person is shown as `handle#id`** everywhere (owner): the handle from their card and the start of their DID —
  handles are not unique, the id is. One definition (`directory.shown`); no page formats a person itself.
- **Indexes are one shape** with a visibility — public (directory, tag index, comments) or sealed (inbox) — as edges
  are (public tags, private labels).
- **Durability.** Immutable blocks are erasure-coded (every node's children and the root's group of one) and read by a
  race against their group. Changing contracts (tails, logs) are kept by re-publishing their signed states.
- **Grants.** A site gets a table's key only with the person's grant (once per site, for all the kinds it uses); the
  home site needs none.

## 6. Files and media

**Bytes.** A file is cut into CHUNKS — 16 KiB to 256 KiB, the file's size ÷ 16 within those bounds, so every file
codes about 16 of them — each ENCRYPTED (XChaCha20-Poly1305, its nonce from the key and the chunk's place), then CODED:
RLNC (random linear network coding) over GF(2⁸), in GENERATIONS of 16 chunks. A generation's first 16 fragments are
its chunks as they are (systematic: a healthy file reads with no decoding, and ONE chunk reads alone — a seek fetches
just the chunk it needs); further fragments are random combinations, and any 16 independent ones rebuild it. Rateless:
more are minted whenever one is slow or refused. Each fragment carries its coefficients, so fragments a keeper mints
later, from the ciphertext it holds (no key), need no format change. Files up to 64 KiB are not coded at all: they
ride INLINE in the item (tables already keep values that size with parity).

**Addresses and the index.** Every fragment lives in a `sealed` contract at an ADDRESS only the key gives (a keyed hash
of the key, the generation and the fragment's number): nobody without the key can name a file's pieces or count them.
The INDEX — codec, size, chunk size, and per generation the fragments stored and their hashes — is a TREE of sealed
pieces (a root listing index pieces), so a file has NO size limit; the upload streams a generation at a time, and the
index goes up LAST, listing exactly the fragments stored — a file is readable only once complete, and an upload
RESUMES (same key, same fragments, same addresses: only the missing are sent). The REFERENCE is `{ key, root hash,
size, name, type, preview }`: every index piece and fragment is checked against it the moment it arrives.

**Reading.** Raced both ways: a generation's fragments are asked at once and decoded on the first 16 valid,
innovative ones; uploads send all at once and replace a slow one by a new fragment. Downloads stream a generation at a
time (to disk for large files) and resume by generation.

**Deduplication.** The key is derived from the content: a PUBLIC file's from its content alone (the whole network
dedups it); any other file's from its content and its SPACE's dedup salt (a secret in the space's sealed table; the
account is the personal space) — members' uploads dedup, and nobody outside can test whether a file is in it.

**Text and media inline.** What a person writes at length (a post, a comment) is MARKDOWN (`markdown`, Grid's
renderer: the source escaped first, so the only markup is the rules'). A file is written where it shows —
`![name](file:KEY)`, KEY naming one of the ITEM's own files (its id; a small inline file: a hash of its bytes) — so a
file shows only to who reads the item. An image shows its preview, then loads once in view; a video or an audio shows a
COVER (its poster or album cover, ▶, its length) and loads only when played. Video and audio written inline go through
the media pipeline (`video-studio`: streamed, a cover, a video id), so their subtitles, lyrics and transcripts are found
by that id (`subtitle-store.forFile`) as in Videos and Audio. The editor (`md-editor`, Reddit's): RICH TEXT by default —
formatted as it reads, media shown as itself — with Markdown as the alternative (the choice kept); what is kept is always
Markdown. 🖼 media inline (this device or Drive), 📎 files; posts, comments and messages (Chat, Messages: compact,
Enter sends, "@" mentions) are written and edited in it.

**Access.** A file belongs to a SPACE (the account: the personal space), and its current key has ONE owner: the row
`k/<id>` of the space's table `files` — `{ key, root, b (its burn hash), x (its burn secret), h (the content's
hash), pub, app, n (the salt it is keyed under; -1: public) }`, sealed like every row with the space's newest epoch. A reference names the file by `id` (its
first root) and `in` (its space); the key it carries is only what a reader outside the space uses (a public copy, a
mail). So who reads a file is who reads its space's rows — the space's policies and epoch keys — and changing the key
changes every reference at once. A file brought from another space (attached or saved from another Drive) is ADOPTED:
listed at once with the key it has, then COPIED under this space's salt.

RE-KEYING, as traditional access control expects, with no job list to keep: a row is due when its key is not the
one its space calls for, and the table says which —
- a member REMOVED, banned or LEAVING (a `leave` act; whoever may remove then takes the leaver's nodes out of the
  group, as a leaver cannot commit their own removal): the next member to see it — once the removal is in the group
  and its tables write with the newest epoch — writes a new salt (`salt`: `{ s, n + 1, removals }`); every salted row
  under an older `n` is due;
- a read policy NARROWED (a board no longer read by anyone), or a post made private: its public rows are due;
- an adopted row (keyed by another space) is due until copied; a row with no `h` (from before) is hashed first.

Any member's page works the due rows, triggered by the table, the roles and a new epoch (members' nodes on their
wake-up, with no page open, are the next step): each generation read under the old key and coded under the new — a
key DERIVED from `h` and the salt, so two members doing the same row make the same fragments at the same addresses (a
step resumes and needs no claim) — the row changed last. Rows only move forward (`n` up, public to salted), so a
member on an older view never undoes one. The OLD pieces are then BURNED: every file piece is a `piece` contract
whose first write names the sha-256 of a burn SECRET (from the space's salt and the key; kept in the file's row;
public files name zero and are never burned), and the re-keyer puts `BURNED ‖ hash ‖ secret` to each old piece —
root, index, fragments — once no row still names that key; the contract takes it only for its own hash, for good, and
the burn reaches nodes that already held the piece. Files from before burning (the `sealed` contract) only fade. A member out of a space keeps the
files they uploaded (their page adopts them into their own space and copies them); what they saved before stays — as
with any access control.

**Video — the basic first (the VIDEOS app, owner 09-30).** A video is a FILE as uploaded, with a poster frame and its
duration taken in the browser at upload. Like Board, in both places: YOUR videos (your channel — each public, on
your profile, or only you) and a SPACE's (any space that adds the Videos app: its members', and public exactly while
its `videos` app reads in public). A video is a row of the place's table `videos` (`{ ref, title, about, poster,
duration, at }`, the file's reference as `files` makes it). Playing: an MP4 by BYTE RANGE — MediaSource fed by mp4box.js, which fragments
it on the fly — the range read as the file's chunks (a seek reads only the chunks it lands in); any other container
(or a browser without MediaSource) loads whole. What follows is where it goes once the basic works.

**Subtitles — data of their own, like Drive's files (built; owner 09-30).** A subtitle track is `content` of the
attaching kind "subtitle": its language and label, its cues a WebVTT file (never in the row), and what it is FOR — the
VIDEO's ID, fixed at upload from the original's key (public: the content alone, so the same video anywhere has one
id; private: salted by its space, so only its readers can name it). KEPT where its author chooses — with the video,
their own, or a team's space — and FOUND by the video's id in the places the viewer can READ (the video's own, theirs,
the spaces they are in, the people and spaces they follow): access control and privacy decide what loads, and nothing
else is asked. `subtitle-store` owns it; players compose it; the SUBTITLES app is its lens (personal, and in a space:
a group working on its videos' tracks); WebVTT or SRT in and out.

**Media apps: one page, a domain each (built).** VIDEOS (`#/videos`: video, movie, TV episode, music video, short) and
AUDIO (`#/audio`: music, podcast, audiobook) are ONE page (`media`) whose route names the domain; its sub-types filter
the list. Audio is made like video (one AAC rendition, fragmented, streamed and seekable; its cover and tags — title,
artist, album, year, genre, embedded lyrics — taken from the file). The timed text of an item is ONE capability named
for what it is on: Subtitles on a video, LYRICS on a song or a music video, a TRANSCRIPT on a podcast or an audiobook
(`kinds.attachLabel`); for audio it is shown in time beside the player (the line playing lit; a click plays from it).

**Settings are the CONTENT's, not the app's.** In a space, what may be read or written is set per DOMAIN (text,
video, subtitle …: `kinds.policyDomain`), so any app — one per domain, or one showing every domain (a Handcraft-like
app) — shows under the same rules; a setting from before under an app's name (board, videos, subtitles) stands for its
domain until the domain has its own. What attaches to an item in the same place follows that item.

**Video — the pipeline (built).** At upload, in the uploader's browser (no server encodes here): Mediabunny (WebCodecs)
reads any common file and writes each RENDITION as a FRAGMENTED MP4 (CMAF: ~4 s fragments, a key frame every 2 s),
one file each, its fragments indexed by byte range and time; a MANIFEST names them with a poster, a scrub strip
(a sprite of frames). A LEAN ladder — every byte is the uploader's upload and the
network's keeping: ONE EFFICIENT family at full quality up to the source's height (4K included), the one the device
encodes in HARDWARE, measured (AV1 + Opus where it can; else HEVC + AAC — Macs, iPhones, most GPUs), and H.264 + AAC
only at 720p and 360p as the safety net; a device with no efficient hardware encoder makes an H.264 ladder alone.
FAST, THEN IN THE BACKGROUND: the upload keeps the original, makes one H.264 rendition and posts (seconds); the rest
is PENDING in the manifest — due work, no job list — made by any open page of the uploader's devices, one at a time
(a lease in the account's table `encodes`, with its progress shown on the video; a rendition a device cannot encode
waits for one that can), each done rendition a new manifest and the item's file replaced. The original is released
once the ladder is made, unless the uploader keeps it (a newer codec later). PLAYING: MediaSource; the family the
browser plays SMOOTHLY (MediaCapabilities: AV1, HEVC, else H.264) whose ladder reaches the screen; the rendition
switched at fragment boundaries by measured throughput; a seek reads the fragment holding that time.

Checking fragments a keeper mints later (not listed in the index) needs homomorphic hashes over a prime field —
Pedersen commitments — and comes with keepers as a new codec version the index names. Files are outside every table's
tree (the tree holds the reference only); keeping them alive follows the references (phase 4, Lifecycle).

## Implementation plan

Each phase: build → the private node (all three sites) → published through B → committed. Status as of 2026-09-28.

| # | Phase | Capabilities | What it delivers | Status |
|---|---|---|---|---|
| 0 | **Restructure to the shape** | all | packages renamed/split to §3–4: `data` → `storage`, `auth` → `auth` + `login`, grants → `access`, `pins` + `tags` → `edge` + components `pin-button`, `label-menu`, `timeline` → `trace`; no behaviour change | done |
| 0b | **Theme** | `theme` | the tokens and base styles; every component and page moved onto them; dark mode everywhere | done |
| 1 | **Keys and sealing** | `keys`, `identity`, `storage` | MLS in the core; your account as an MLS group of your nodes; table key = export(epoch, table); escrow to the words; sealing of whole tree nodes (Sealed contracts at keyed addresses); tables sealed today by row are sealed over; a removed node forgets its member | done |
| S | **Space, the shape** | `space` | what a space IS, once: id, governance root, this node's writer key, its shared key, its own tables' names (named by the identity); your account is the first; `storage`, `membership`, `keys`, `ordering` take the space instead of hard-coding the account | done |
| 2a | **Feeds and merge** | `storage`, `identity` | every node writes its own feed per table (its node key signs); rows carry id + the version they replace; readers merge causally; the shared-key tail read as the oldest writer | done |
| 2b | **Membership and removal** | `membership`, `keys` | owner-signed node credentials and member-signed removals, gossiped in each node's feed; removal moves the removed node's winning rows, then drops its feed | done |
| 2c | **Ordering per epoch** | `ordering`, `keys`, `identity` | the account's commits keep ONE sequence (MLS has no forks), but each epoch's sequence is its own tail under a key derived from that epoch's secret — only the nodes in the group at that epoch sign it, so a removed node writes no later epoch's; a words-joiner walks from epoch 0 (escrowed to the words at registration) through each epoch's next-epoch escrow; the data key signs only tables from before feeds and that first pointer | | done |
| C1 | **Groups per space** | `identity`, `keys`, `storage`, `space`, `content` | the identity keeps each space's group state and epochs (the account's where they always were); `mls` admits a space's members by their accounts' credentials; one set of log rules for every group; storage over a SCOPE (the account, or a space: its group's members write, each space table sealed with its epoch keys); a CHANNEL is a sub-space inheriting its server; `content` is the shape of an authored item; the Chat page (Discord layout) makes a server and posts in it | done |
| M | **Private messaging** (owner: the foundation of messages) | `directory`, `index`, `conversation`, `space`, `content` | a person's public card (handle + id, their nodes' key packages, the inbox key); the inbox (a `bag`: sealed items, admitted by work, no signer); `conversation` kinds direct / group / mail (only where the mechanism differs) — direct: a two-person space, the welcome in the other's inbox, each person's messages in their own feed; mail: the mail in the sender's public tail `mail` (only their account writes it: the proof of who sent it), sealed to each recipient's inbox key, a pointer in each inbox, kept by the recipient in their `mailbox` | done (direct, mail) |
| C4 | **Roles and moderation** | `roles`, `moderation`, `space`, `content`, `keys` | a space's id proves its owner (sha-256 of owner + nonce); built-in roles owner / admin / member; authority changes by ACTS in the space's `acts` table, replayed in one order by every reader, each counted only if its signer could (the signer = the feed's writer node → its account, from the group's credentials); moderation hides any object (a message, a channel) and removes a person (their nodes out of the group: a new epoch); `content`'s author is its row's writer, hidden items left out | done |
| C3 | **Invites, governance UI** | `roles`, `conversation`, `index`, `space`, `server-settings` | invite CODES as acts (expiry, uses; revoke); a joiner drops a request in the bag the code names; any member who may invite admits askers when online (`admitted` act) — and, with no page open, their node's identity delegate does (the mandate: codes in force, bans, members, the group; its admissions written as acts by the next page); ownership TRANSFER act; leave = out of the person's own list; Server settings: overview/ownership/leave, members & roles, channels, invites, moderation log | done |
| F | **Files and media** (§6) | `files`, `storage`, `content` | a file as encrypted 256 KiB chunks, RLNC-coded in generations of 16 (24 fragments), raced reads, an index checked against its reference; attachments in Messages, Chat, Mail and Board; streaming reads; Drive | in progress |
| 3 | **Reads** | `storage`, `blocks` | range (latest N, older pages) and change-only reads: item ids sort by time; a feed's page walks only its path (`data::Open::page`); a LAZY table reads no tree at open and pages across its feeds; Chat rooms and unread counts read paged; changes fetch only changed blocks (content-addressed, cached); lists are TIME WINDOWS (as Grid's: New by date a month at a time; Hot, Best, Rising, Top over today / the week / the month, counts over the same window) | done |
| 4 | **Lifecycle** | `keep` | re-publishing, retention, flush when quiet, health; blinded table names — FLUSH WHEN QUIET: a tail's rows go into its tree (erasure-coded) once writing has stopped a minute or two, not only at 32 rows; PUBLIC tails (a card, a board's public copies, a profile) flush too, into a tree IN THE CLEAR (Block contracts named by their ids, no key), so a public table grows past what one tail holds (256 KiB); KEEP (`keep`): a table's ASSET is every block its current tree reaches, in its coded groups; keeping asks each block (HEALTH per group: whole / degraded / damaged), puts each one again (re-published where there, made again where not — a member from what the page holds, parity coded again from its group) and puts the tail's signed state again; due when the account's table `keep` has no record newer than a week, one table at a time while a page is open; Account → Storage shows each table's last keep and health, and keeps all at once on a click; FILES kept the same way (every piece asked and put again as stored, health per generation) | in progress |
| 5 | **Rules over objects** | `ordering`, `governance`, `roles`, `access`, `membership`, `administration`, `moderation`, `space` | the Log for objects and spaces; acts and their rules; spaces | — |
| 6 | **Shapes** | `content`, `edge` | notes as content; pins, labels, likes, votes as edges | — |
| 7 | **Index** | `index` | the Bag: inbox (sealed pointers), directory (public, handle + id), comments | — |
| 8 | **Chat** | `chat` page | a space + its MLS group + content (messages) + index (inbox) | — |
| 9 | **Privacy and observability** | all | padding, sealed senders, no time-sortable ids in private spaces; health in `trace`/`account`; ANALYTICS: view counts (one entry per viewer per item, as Grid's tally — deduped, windowed like the feeds, public item counted publicly, private item only among its readers) and the feed bar's Popular | — |
| 10 | **When needed** | `ordering`, `roles`, `keys` | witnessed snapshots; enterprise role templates and escrow | — |

Built already (in the shape since phase 0): identity with rotatable keys (key log contract `idlog`), PIN
logins moved across delegate builds, per-site grants, tables (tail + tree, flush, catalog), erasure with raced reads and
rebuild (`blocks`), row sealing (replaced in phase 1), pins, tags, Notes.
