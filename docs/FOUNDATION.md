# Foundation: lifecycles and faults

Status: DESIGN for review (2026-10-12). Nothing here is built yet; the shipped patches it replaces are listed in §6.

Every capability and app stands on two things that today are decided ad hoc, call site by call site:
**every process has a lifecycle** (§1 the mechanism, §2 the read machine, §3 the others), and **every failure is a
fault** — the failure edge of any machine (§4). The 10-12 recovery failure on the owner's node was both at once: a
channel read that had not ended was taken as absent (§2), the account's group was created by an inline `if` with no
guard (§3), and the outcome existed only as a trace line (§4). This document makes each one a single mechanism with one
owner, enforced at the lowest shared layer.

Audit sources (read-only, 10-12): storage door, session/key lifecycle, error handling. Line references are to the tree at
41c9b43.

## 1. Lifecycles: one mechanism for every process

Every process the system runs — the page, a session, the account's key group, a space membership, an open app, a read,
a save, a delete, a background job — has a lifecycle: it starts, runs, ends, and can fail on the way. Each one gets a
state table held as DATA in one module, and one function moves it from state to state. No module keeps its own flags.

### What is wrong today

No process has a written lifecycle. Each one is decided inline and learned through ad-hoc events
(`craftworks:auth`, `craftworks:keys`) or a page reload:

| Process | Today (41c9b43) |
|---|---|
| **Page** boot / update / shutdown | Boot is the loader's sequence; an update reloads when "nothing would be lost", judged by a scan for typed text (loader.js:474-478); shutdown has no handler except usage's flush (usage.js:146-147) |
| **Session** login / unlock / logout | Inline in auth.js (:55, :98-113, :139); a words login whose keys failed still opens a session |
| **Account key group** | Inline in keys.js (:102, :346, :366, :424, :468, :480); `keep()` writes `e/0`/`info` whenever the channel as read lacks them |
| **Space membership** | Inline in conversation.js (:858, :876) and upkeep.js (:31, :53) |
| **App** open / close | Close is `slots[slot].replaceChildren()` (loader.js:225): no unmount, so an app's subscriptions, listeners and timers outlive it |
| **Save** | An in-memory queue per tail (storage.js:488-527); a tab closed mid-save loses it with nothing said; the quiet-flush timer dies with the tab |
| **Delete** | Four separate removals (items.js:910, space.js:166, keys.js:461, identity.js:112), each with its own steps; none reports |
| **Background jobs** | 14 `setInterval`s in 9 packages, 4 of which ever clear one; upkeep runs for a session whose keys did not come |

The 10-12 damage chain, from the code path: a words login whose join fails still opens a session; with no account keys
its upkeep re-puts the card's key packages (directory.js:128, upkeep.js:31-34) — the card then flips between this node's
and the original devices' — and it asks members to welcome it again (conversation.js:858-877); a member's `readd`
replaces the person's entry in a space's group (conversation.js:120-123), which can drop the original devices from that
space (upkeep.js:53-54). `keep()` writes `e/0` and `info` whenever the channel *as read here* lacks them (keys.js:102):
that is how the stray group got into the owner's channel.

### The mechanism

`packages/lifecycle.js`:

```js
define(machine, { states, events: { [event]: { from: [...], to, guard? } }, persist?: true })
transition(machine, key, event, detail)  // check from + guard → (persist) → emit → one health report; illegal → `bug` fault (§4)
lifecycle.state(machine, key)            // what every module asks instead of keeping its own flags
lifecycle.on(machine, fn)                // replaces craftworks:auth / craftworks:keys and the reload
scope(owner)                             // timers, listeners, subscriptions, reads tied to a process; all released on its end
```

- Every transition emits exactly one report (`lifecycle-<machine>`) with from/to and the event's detail (§4 POLICY).
- A process's work runs in its `scope`: when the process ends (app closed, session locked, page hidden) its timers stop,
  its subscriptions drop and its in-flight reads are abandoned. No bare `setInterval` or `addEventListener` outside one.
- A process may start only when the processes it depends on are in the right state (the gate column below); the gate
  is checked by `transition`, not by the caller.

## 2. The read machine

A read is a process like any other: it is asked, it waits, it ends absent, present or unreadable, and an unreadable read
starts again when its keys arrive. `ReadState` below is that machine's state, and R1–R5 are its guards. It comes first
among the machines because every other machine's guards ask it ("only when the channel is `absent`").

### What is wrong today

`storage` already knows "the node answered" apart from "absent" (`answered()`, `index.read`'s `got/get-failed/silent`),
but every public surface merges them before a caller sees them:

- a tail is shown `absent` after `WAIT.hint` before any answer (storage.js:429-431); a node that never answers ends as
  `absent` too (:455-460, :482); `answer()` swallows a failed or sealed read (:381);
- a sealed tree, rows sealed for an epoch not held, a missing table key all read as an ordinary (smaller or empty) table;
- merged tables have no answer at all: `settled` resolves after `WAIT.hint` regardless (:1346);
- `placeOf` says "unlisted" for a catalog that is merely unread (:740), and `known:false` follows.

So creates, overwrites and defaults run on data that has not arrived: `write()` creates on "absent" (:488-495);
"not listed ⇒ absent" chains through the directory (:253, :276, :786, :1287, :1292, :1297); `directory.publish()` re-puts
the card's key packages over an unread card (directory.js:120, :128); the epoch log writes its "open" row on silence
(keys.js:160); `ordering.tail.from()` returns "no newer commit" on rows not read (ordering.js:52); space mail makes a new
seed (conversation.js:1528); roles default an unread card policy to `"anyone"` (roles.js:413, :420) — permissive.

Failures are cached for the page's life: `tails` (:212/:443, and a `known:false` open frozen for every later open, :306),
`tables` (:1166), `directoryOpen` (:765), `opened` feeds (:1003), roles' card (roles.js:413), `keysName` (keys.js:514).
When keys arrive later, only tails in `unkeyed` are re-read (:191-210); rejected promises, frozen opens and decisions
already taken are not.

### The machine and its guards

One type on every tail and merged table:

```ts
type ReadState =
  | { s: "unknown" }                                   // not asked, silent, or shown before its answer
  | { s: "absent" }                                    // the node ANSWERED not-found (or held it with NO state — wire
                                                       // `GetFail::Empty`, 10-22), or a readable record says never made
  | { s: "present" }
  | { s: "unreadable"; need: { epochs?: number[]; tableKey?: true; grant?: true }; why: string };

t.state: ReadState           // now
t.decided(): Promise<ReadState>   // waits up to WAIT.answer; "unknown" is never reported as anything else
t.onState(fn)
```

- **AN ANSWER NAMES ITS CONTRACT** (10-22): the node holding a contract with an EMPTY state is an answer — nothing
  written there — never an unusable message naming nothing (the owner's node answered so for its own catalog in two
  conversations it had never written in: each read waited for ever, every write there refused "not answered yet", a
  message lost and a member never put back on a list).
- **R1 — No create or overwrite on unknown.** `write()` awaits `decided()`; `unknown`/`unreadable` throw a retryable
  `not-yet`/`sealed` fault (§4). `beforeCreate` and a PUT run only on `absent`. Enforced in `tailAt`, once.
- **R2 — "Not listed" is only an answer when the list was read.** `placeOf` returns `unknown` unless its catalog is
  `present`/`absent`; `known:false` comes only from `absent` or a readable "none". One change covers every directory and
  card lookup.
- **R3 — Unreadable is not terminal.** `unkeyed` becomes `waiting: Map<id, need>` (epochs, table key, grant). Key
  arrival (§1 events) re-reads exactly those; their state moves and subscribers redraw. No page reload.
- **R4 — Caches hold objects, never failures.** Every per-page cache of a read goes through `ctx.cache(map, key, make,
  fallback?)`: a failure leaves the map at once (asked again, it is made again) and answers the fallback or its fault —
  one implementation (`tools/read-check.mjs` fails a cache kept by hand); a later `known:null` open upgrades a
  `known:false` one. A view that cannot yet tell "none" from "not read" (roles' public acts, R5) says `unknown`, and is
  never remembered as "none".
- **R5 — A merged table is decided when its own feed and every listed writer's feed are**; a sealed or unread writer
  list makes it `unreadable`, never an empty `present`. A writer named since it opened, its feed still opening, keeps
  it `unknown` until that feed answers. A list SHORT of whole never makes what was read unknown: what is listed and
  read is shown (10-25, live: a rule that held a space's every table "still arriving" while its list was short kept
  Craftworks' Board and Home empty, the list short for good). A read TRAVERSES WHAT IS LISTED — a catalog, the writers bag, the index — so
  every feed it waits on is there and answers; nothing is decided by a clock (owner 10-25: "it either is there or not
  and should be traversable. that is our design" — a read that needs a deadline is looking up something unlisted).
  NO READ "IN CASE", page or node: a writers-bag entry that does not open here names no one (skipped and said, never
  "every member's device"), and an account's devices contract is read only where its card's `devices` mark lists it
  (`account::table_writers`, `account::devices_step`); another writer's public table is read by name only where its
  catalog or the open list names it. A LISTING NAMES WHAT IS THERE: a device's writers-bag entry and a public table's
  open-list entry are written AFTER the contract is made (its own catalog still notes it before, for itself), and a
  device whose made catalog is not listed lists it on its next page. A listed contract the node answered NOT THERE is
  DECIDED ABSENT: the node's upkeep keeps it (`Reading::absent`, with the state of what listed it) and asks again only
  when that listing changes. Each GET of a contract nobody made is a 60 s not-found on the
  node; the identity delegate PARKS on its own GETs, and every request to it — a loading page's first — queues
  behind them (10-25, owner's node: registering waited 99.5 s behind upkeep's reads of absent feeds and an absent
  devices contract, then ran in 3 ms; its minute wake-up skipped 23–25 times an hour).
- **ONE READ STATE, one mark (10-24).** What a read gives carries this state — `ctx.stated(value, state)` — and a read
  made of parts takes the LEAST decided of theirs (`ctx.least`: any `unknown` → `unknown`). Rooms, items, Discover's
  lists and an app's frame (across the host) pass it on; a failed part keeps its state (`ctx.total`), never a plain empty
  list. The view draws "Still arriving…" for `unknown` and reads again. No second mark beside it (an `arriving` flag, an
  `answered()` check): computed again at each layer and dropped by one, it drew "no posts" for a list still arriving
  (`tools/read-check.mjs` fails one).
- **Defaults deny.** A policy, card or role not read is `unknown` and treated as the narrowest choice, never "anyone".
- **R6 — Read once, then followed (owner 10-15: "we should not be making unnecessary calls … not using the same data
  already available").** Every contract that can change (a tail, a bag, a key log, a register) is read through ONE door,
  `node.contract(id)`, which keeps its read state per contract: `UNKNOWN → ASKED → HELD`, `ABSENT`, `STALE`. The first
  read SUBSCRIBES; a push keeps the core's copy current (`HeadChanged` stores the state for every contract, not only
  tails) and fires `node.onContract(id)`. A read of a `HELD` contract answers from that copy — no GET, whatever the
  caller calls it (`fresh`, `strict`, a re-read on an event). `ABSENT` is asked again only by the door, on its one
  backoff (5 s doubling to 5 min) while someone follows it. A reconnect marks every `HELD` contract `STALE`: read again
  once, subscribed again. Work done once per device (the key-history walk) is never re-derived on load. Content-addressed
  reads (tree blocks, file pieces, the site) never change: read once per page, held. `tools/read-check.mjs` fails the
  build on a GET, a consuming `take_got`, a `backoff` or a fresh flag anywhere else.
  THE FOLLOW BUDGET (10-24): the node lets one connection follow at most 500 contracts and has no unsubscribe — past it
  every subscription is refused, the page's own included (fx, measured: a fresh account could not register; a Discover
  over hundreds of public spaces had spent them). FOLLOWED BY INTENT: what a member reads (its account's tables, its
  spaces') is followed, up to 400 a connection; what is read from OUTSIDE (Discover, a public space one is not in —
  `storage.readOnly`) is `READ` — fetched without following, kept, read again on the next ask after 60 s — and never
  spends the budget (fx: Discover had spent it, and a newcomer's join was never pushed to the space's owner). A tail
  opened from outside and then by a member is followed from the member's open on. A new connection starts a new
  budget, what was followed followed again first.

Callers: `const st = await t.decided(); if (st.s === "absent") create…; else if (st.s !== "present") wait`.
`ordering.tail.from()` returns `undefined` on unknown; `commitFrom` passes it through.

## 3. The other machines

| Machine | States | Gate / rule |
|---|---|---|
| **page** | `BOOTING → RESUMING → RUNNING ⇄ HIDDEN`, `UPDATING` | No shutdown state (see below). `RESUMING` restores what the last page left; `UPDATING` waits for **save** to have nothing owed; `HIDDEN` sends everything owed |
| **session** | `GUEST → UNLOCKING \| HANDING_OVER \| RECOVERING \| REGISTERING → OPEN \| OPEN_KEYLESS → LOCKED \| REMOVED` | `OPEN_KEYLESS` is derived on every load (logged in, the account's keys not held here) and blocks every write to the card, key packages, welcomes and asks; the page says "keys not recovered" and offers recovery again |
| **group** (per MLS group, on this device) | `UNKNOWN → HELD ⇄ BEHIND`, `OFF_BRANCH`, `NOT_HELD`, `UNREADABLE`, `STUCK`, `REMOVED`; its steps (`commit`, `join`, `join-failed`, `adopt-delegate`, `log-*`, `history-*`) | ONE machine for EVERY group (below): `CREATING` only by its maker on a channel answered absent (§2); `BEHIND` applies commits in order; `OFF_BRANCH` repairs onto the branch the owner announced; `HISTORY_GAP` asks the group's earlier keys of THIS ACCOUNT's own devices first, then its admins; every STEP its keys take (a commit and why, a join, a log written or mended, history handed) a step of this machine — kept in the account's JOURNAL and reported (the `keyevents` table it replaces) |
| **member** (per REQUEST, on each decider: a page, or the node — `by`) | `SKIPPED{why} → ADMITTING → WELCOMED`, `KEPT{why}` (sent later), `FAILED{why}` | admitted ONLY with the key the person's REQUEST carries (and only if that key's credential is theirs, by their key log); only the member whose turn it is; never again for the same request — no member re-adds anyone on its own (the loop that drained cards and ran a space to epoch 478). The node's decisions are moves of this machine too (`identity::Decided`, moved by the page's `absorb`) |
| **join** (per REQUEST, on the asking device) | `ASKED → WAITING{why} → MEMBER`, `FAILED{reason} → ASKED (a new request)` | every request carries ITS OWN key package, made by this device for it (its secret kept in the account: any device of it opens the welcome) — the right key to the right invite by construction; one TOLD it is in, BEHIND, OFF the owner's branch or whose welcome did not open asks in again itself with a new key. No key packages on the card. `join` and `member` share the REQUEST's key (`q:<its key's tag>`): every move one health report, so one person's request is read whole from anywhere |
| **app** (per open app) | `OPENING → OPEN → CLOSED`, `FAILED{fault}` | `OPENING` needs page `RUNNING` and, for login-only apps, session `OPEN`; `CLOSED` ends its scope at once — its changes were already saved as made |
| **save** (per change) | `EDITING → OWED → PREPARED → SENT → HELD`, `REFUSED{fault}`, `RETRYING` | Every change is saved as it is made (below); `OWED` only on a decided tail (§2 R1); `RETRYING` by §4 POLICY; the page shows "saving…/saved/not saved" from this state, one place |
| **delete** (per object) | `REQUESTED → HIDDEN → GONE`, `REFUSED{fault}` | one path for items, spaces, devices and files; who may delete is the object's rule (roles), checked in `REQUESTED`; `GONE` only where the network can forget (a tail can hide, not erase — said to the person) |
| **job** (per background task) | `IDLE → RUNNING → IDLE`, `PAUSED`, `STOPPED` | upkeep, renew, sync, usage flush, heartbeat: each declares the states it needs (e.g. upkeep needs session `OPEN` + account group `MEMBER`) and is `PAUSED` otherwise; `HIDDEN` pauses jobs that only serve the screen |

A VIEW draws a read that answered before its screen was put in place once it is there (10-23: held data answers in 0 ms, the answer was dropped as if the screen had closed — a space's Board came back after 11–41 s; now ~120 ms). A VIEW never takes away what it shows while a re-read is still arriving (10-23, owner: "if it show the first time, why it hide and show again?"): an arriving read with fewer entries, or a "not yet", leaves the drawn screen until the read answers.

A person's SEND is a save they wait on: sent through the ONE action (`theme.action` — a comment, a chat message alike):
its text kept and "Sending…" said until it is HELD, cleared only then; REFUSED, the text stays with why (10-22: Chat
cleared the text at once and its refusal came up to 2 minutes later — the person gone, a message "lost"). The JOURNAL
keeps each change ONCE across pages: a step (nothing moved) the same as the last one recorded for its key is not written
again, by any page or load (10-22: one refused key tree recorded dozens of times an hour). A WRITE checks the newest key
of the table it writes only — the walk over every open table is the keys' event's (10-22, measured live: a chat send
waited ~20 s while each table open on the page asked the identity for its key, one after another).


### The node's rounds: one table each (10-20)

The node (the identity delegate, no page open) runs ROUNDS: a space's FOLLOW round (`upkeep`: its acts and key
operations read, the keys kept to the list — BeeKEM, or MLS for a space not moved yet), a space's FILES round
(`rekey_round`: a removal's new salt, each due file re-keyed), and the account's KEEPER and WITNESS rounds (`reads`).
Each is a state machine driven by answers that come back one at a time; prior art: Erlang's `gen_statem` (a
function per state over every event, an unexpected message handled explicitly, never dropped by a default) and a
correlated request/response (an answer moves only the step that asked for it).

**Invariants** (every cell below keeps them):
- **R1 one at a time**: one round of each kind on a node; a page asking (`FollowNow`) sets aside another member's follow
  round, never its own.
- **R2 an answer moves only the step that asked for it** (`waits_for` / `wants`): any other answer — late, duplicate,
  another round's, the wrong kind — moves nothing and refreshes nothing, its own cell in every state (no `_ =>`).
- **R3 every state leaves**: by its answer, or STUCK (`upkeep::stuck`: `STUCK` wake-ups or `STUCK_MS` of the node's
  time since it last MOVED, R2's answers only) and dropped at the next wake-up, the step recorded (`STALLED`).
- **R4 nothing changes from a short read** (`guards::whole`): applied what the check takes, nobody out, no key change.
- **R5 nothing is kept that is not published**: a generation is kept only when its operations can be written from
  here (`upkeep::creates`: a contract made on the node reaches nobody today — the change is a page's).
- **R6 one record**: every outcome a named step (`rules::steps`), the page's lifecycle the same.

**Follow round** (`upkeep::Step`; events: the step's ANSWER — `Got` / `Put ok` / `Put refused` — a STRAY answer, a
WAKE-UP, a page ASKING):

| State | Got (its own) | Put / Updated (its own) | stray answer | wake-up | page asks |
|---|---|---|---|---|---|
| (none) | — | — | nothing | next member's round begins (each in turn) | its round begins |
| `Acts` (read) | read on; whole → BeeKEM: cards needed → `Acts` (by list); none → reconcile; MLS → `CatchUp` | impossible (asks only GETs) | nothing (R2) | stuck → dropped (R3) | same member: left; other: set aside, its own begins |
| `CatchUp{epoch}` | log + door both in → an entry applied → `CatchUp{epoch+1}`; none → follow out (`Log`) or done | impossible | nothing | stuck → dropped | as above |
| `Log` | position taken meanwhile → done (`MOVED_MEANWHILE`); else the commit written → `Commit` | impossible | nothing | stuck → dropped | as above |
| `Commit` | impossible | ok → held, next epoch's log → `Next`; refused → done (`COMMIT_REFUSED`) | nothing | stuck → dropped | as above |
| `Next` | impossible | → door announced, done (`COMMIT`) | nothing | stuck → dropped | as above |
| `KeyFileRead` | a part kept; whole → checked, kept by its key → next file, or reconcile | impossible | nothing | stuck → dropped | as above |
| `KeyFileWrite` | impossible | next part / the index / the root; stored → its row and listing → next file, or `Keys` | nothing | stuck → dropped | as above |
| `Keys{table}` | impossible | all answered → more rows → `Keys`; none → `ListFiles` (files written) or done; refused → done (`KEYS_REFUSED`) | nothing | stuck → dropped | as above |
| `ListFiles` | read on; whole → `Keys{files}` | impossible | nothing | stuck → dropped | as above |

**Files round** (`rekey_round::Step`):

| State | Got (its own) | Put / Updated (its own) | stray answer | wake-up |
|---|---|---|---|---|
| (none) | — | — | nothing | due (`EVERY`, or the soonest row's turn) and a member whose page is away → `Reading` |
| `Reading` | read on; whole → a new salt (`Writing{Replan}`), a file due → `Recoding`, else done | impossible | nothing | stuck → dropped |
| `Recoding` | its read (`file_io::FileRead`) on | its write (`file_io::FileWrite`) on; stored → `Writing{Row}`; failed → next file | nothing | stuck → dropped |
| `Writing{after}` | impossible | all answered → `Replan`: read again; `Row`: old pieces burned, next file; refused → done | nothing | stuck → dropped |

**Keeper and witness rounds** (`reads::Round`): `(none) → waiting{asked} → (none)`; an answer it asked for is taken, any
other nothing (R2); paused (its step left work for later) kept; stuck → dropped.

A **model test** drives these over the real code with seeded random sequences — the real answers, duplicates, strays,
answers that never come, refusals — and checks R2 and R3 after every event; it fails on a tree where a stray answer
moves or refreshes a round.

### Members and keys: one member list, keys that follow it (owner 10-16: "fix it")

Every encrypted thing — a space, a PRO/VIP group, a direct or group chat, an audience (a role, chosen people), a circle
(friends, followers), the account's own devices — is the SAME two parts: a MEMBER LIST, and the KEYS (an MLS group)
that let exactly those members read. Nothing else.

**What went wrong (measured, 10-16).** About 50 commits (10-03 → 10-16, releases v374, v377, v387, v398 …) patched
joining and catching up; none converged.

- **Two member lists, never forced to agree.** The space's RECORD (its acts: admitted / added / removed / banned —
  `gov` replays them into a roster) and the KEYS (who holds a leaf in the MLS group) were each changed on their own
  paths. Ivvor's New Space (`f5697a`), live 10-15/16: the keys at epoch 485; of its 7 members besides Ivvor, four hold
  keys 147–256 epochs old and three never announced any — none reads the space — and the record says the owner was
  never admitted while the keys hold them. Every patch tried to make the two agree after the fact.
- **Many mechanisms for one job.** Ask on every load; ask hourly; catch-up asks (turned off 10-06, back on the same
  day); repair welcomes; re-welcomes; a resend outbox; the stuck bag; the node's re-adds; the page's re-adds. They
  interact; their interaction ran Ivvor's space to epoch 485.
- **The same operation in several places.** Putting someone into the keys: 3 (`conversation.welcome`, the delegate's
  `upkeep.rs:666`, `keys.js:934`). Taking someone out: 2 (`moderation`, `membership`). Who belongs, per kind: an inline
  condition (`conversation.js:987`), PRO/VIP checked again on the node (`upkeep.rs:290`, `:651`).
- **Keys only by asking.** A member who cannot read must send the right KIND of request (join vs. again) — chosen by
  its own reading of the record; when the record and the keys disagree it sends the wrong one and waits for ever
  (`rules::admission` skipped it: "in the group already").

**What went wrong after (10-16, eight review rounds).** The first fix kept the ask/answer shape — a device that could not
read POSTED A KEY and a member's node ANSWERED by re-adding it — and wrapped it in rules (once, answered, windows,
turns, refusals, waits). Every round found new loops and dead ends in that layer; an audit against the invariants found
six of eight broken. The layer was invented: MLS already lets a member enter BY ITSELF (an EXTERNAL COMMIT from the
group's published info) — and this codebase already does so for the account's own group (`mls::Account::join`); the
spaces never did. Prior art agrees (MLS RFC 9420 external commits; Matrix's room-state DAG; Keybase's team sigchains;
DCGKA, Weidner–Kleppmann 2021): keys never wait on another member answering.

**The design (10-17).**

1. **ONE GROUP IMPLEMENTATION, every kind.** A space, a PRO/VIP group, a direct or group chat, an audience, a circle and
   the account's own devices are each an MLS group with the SAME operations, implemented once (Rust core; the page and
   the identity delegate through it):
   | Operation | MLS | Who does it |
   |---|---|---|
   | create | a new group | its maker |
   | ENTER / RE-ENTER | an EXTERNAL COMMIT from the group's published info — removing its own old entry in the same commit (`with_removal`) | the LISTED account's own device, by itself, whenever it cannot read (never held, another branch, stuck) — nobody else online, nobody answering |
   | follow the list OUT | a Remove of every entry whose account is not listed | any member, by turn (`admission::turn`) |
   | bring current | apply the commits from the epoch logs | each device |
   A kind supplies only: WHO IS LISTED (below) and WHERE its group info is published. Nothing else differs.

2. **ONE MEMBER LIST per kind**, the only thing decided:
   | Kind | Its member list | How someone gets on it |
   |---|---|---|
   | a space | its acts' roster | self-service: an invite CODE (their entry carrying the code's proof bound to their DID — `gov::join_proof`; invite acts are never made public), the OPEN door (their own join act under the "anyone" policy); or MANUAL approval (a request, below) |
   | a PRO/VIP group | members whose card shows the level | self-service: entered with the level as its proof (the honor system); a member whose page reads their card short writes `lapsed` (in turn), and the follower takes them out |
   | a direct / group chat | its started / added acts | its maker, a member adding |
   | an audience | the role's / list's assignments | the space's admins |
   | a circle | the owner's friend / follower edges | the owner |
   | the account | its devices contract | the words (`membership`) |
   The REPLAY decides every entry (`gov`: the same on every node). Getting the keys then needs nobody: the listed device
   enters by itself (1).

3. **A REQUEST (manual approval only) never hangs** (owner 10-17): its own lifecycle, every one ending decided —
   | From | Event | To | By | Recorded |
   |---|---|---|---|---|
   | — | ask | REQUESTED | the asker | their signed request item |
   | REQUESTED | cancel | CANCELLED | the asker | their signed cancel item |
   | REQUESTED | let in | LISTED | one who may invite | a `let-in` act naming the request |
   | REQUESTED | reject | REJECTED | one who may invite | a `reject` act naming it (an optional reason the asker sees) |
   | LISTED | enter | MEMBER | the asker's own device | its external commit |
   One end per request (the replay orders a cancel and a decision that race); a new request after CANCELLED / REJECTED
   is new. Self-service entries have no REQUESTED step.

4. **WHERE AN ENTRY GOES, and the group info** (checked against the code 10-17: an epoch's log is written only with a
   key from that epoch's secret — a device that lost its keys cannot write it). Members' commits stay in their epoch
   logs. An external commit goes into the epoch's DOOR — an open bag per group and epoch (`door-<epoch>`, sealed with
   the group's open key from its id, as the writers list is). Position `e`'s entries = the epoch log's ∪ the door's, in
   ONE order (`rules::ordering::order`). The GROUP INFO of an epoch is put in its door in parts, announced in the writers
   list (`{w, epoch, branch, ih}`): the OWNER's always (its owner's node too, once an epoch, with every page closed),
   another member's only where no whole info is there yet (a door holds 256 KiB). A device enters only from AUTHENTIC
   info — signed by the space's OWNER (proven by the space's id) or, its list read whole, by a listed member; never its
   own account's (its own fork). HISTORY is unchanged: each epoch log's `open` row (prev / back secrets), written by any
   member reaching the epoch — the entered device then reads back.

5. **EVERY ENTRY CHECKED AGAINST THE LIST, above MLS** (one rule, `mls::entry_ok`, the page and the node alike), by
   answers that NEVER CHANGE as more acts are read (acts only grow — I5: every reader alike, whenever it reads; review
   10-17): the committer EVER listed and not banned, or entering by itself with a proof the acts admit (`Gov::admits`:
   uses, expiry, standing — one entering is listed by its own `join` act soon after, so a reader behind judges it as
   listed); everyone added the same; an entry takes out only its committer's own; a member's commit takes
   out only accounts the acts EVER put out or never listed (`Gov::may_be_taken_out` — no member removes a listed
   account on its own). A reader takes the LOWEST VALID entry at each position; an invalid one is skipped by everyone
   (an outsider's never holds a position: no jam). Nothing is skipped from a list not read whole (deferred: I6). A
   self-entrant's PROOF also rides its own leaf (a leaf extension): the follower keeps that entry while the acts admit
   it NOW (`Gov::admits`: never the banned, never one REMOVED — back only by an invite) and the acts have not put it
   out, until its own `join` act lists it.
   THE LIST CARRIED OVER (10-22): a space made before 10-17 has no act listing its members — its list is its owner
   alone, every other member a stray (Ivvor taken out of the owner's direct conversation). One who may list puts on it
   (an `added` act) whoever is in its group and a direct conversation's other, never listed and never put out, once
   the list reads whole; one not in the group is told again and enters by itself (`conversation.relist`, upkeep).
   BEEKEM ADMITS AT ONCE (K4, 10-22): a space's content is sealed with its BeeKEM generations (`BeeGroup.current`: a
   key change made when none is current). A newcomer is in within seconds, nothing called: entering a space (every way
   in, `conversation.enterSpace`) reconciles it — this account's member written on its card, the add of it merged as
   soon as it is written; and each space is reconciled again soon after its list, its key operations or a listed
   account's card changes (`members` follow). Measured on fx: placed 16–17 s after the invite, a generation-sealed
   message read 15 s after it was written (before: never in 15 min — the card's member waited for the 10-minute pass).
   WHO IS IN IS THE LIST (K8.1, retiring MLS): a space's members and writers are its acts' list — each listed account's
   devices (`storage.setRoster` from each replay; the node keeps the list it last read whole, `upkeep::roster_kept`,
   reading only carded accounts), widened as the acts name more — never the MLS group. While migrating (until K8.6)
   whoever the MLS group still holds and no act lists yet is read too, so a space made before the list acts loses
   no one's items.
   REMOVED IS THE LIST's WORD, THE KEYS FOLLOW IT (K8.2): this node is out of a space when the list's out-acts name its
   account and it is not listed again (`roles.left`; upkeep forgets the space) — never the MLS group's; a person put out
   is out of the keys once the BeeKEM tree no longer holds them (`members.holds`): the file re-key waits for that and
   for its table to seal with the newest generation; where their feeds stood is recorded while the tree (or, migrating,
   MLS) still holds them.
   NOT MIGRATED, NOT SHOWN (owner 10-24: "drop support … any lost messages during this time is benign"; "hidden
   totally … won't even appear as open"): no MLS-era history is carried over. A space whose list, read whole, never
   moved this account in or out (not listed, not its owner, no counted act letting it in or putting it out —
   `gov::entry_move`; an MLS-era member nobody listed) is
   taken out of this person's list as a removal is (`roles.unlisted`, `upkeep`), and so is a space with no BeeKEM tree
   at all (`members.hasTree`: its key operations, read to a decision, hold none — an MLS-era space whose sealed acts a
   member without MLS keys cannot even read whole; live, Wren in Ivvor's Space); a public space whose key operations
   hold no tree is not listed in Discover and does not open from outside (`items.opens`). THE NODE FOLLOWS NO MLS
   (K8.6's first step, 10-26): its upkeep keeps a space to its list by the BeeKEM tree alone — no MLS catch-up or
   follow-out; a space with no tree is said `not-migrated` and not followed. The catch-up read the NEXT MLS epoch's log
   every round, a contract nobody made unless someone committed: on the owner's node a 60–75 s not-found that parked
   the delegate and failed the page's requests behind it. A newcomer is named by its own
   join or its invite, so never hidden. From OUTSIDE, a public space is read from what its index LISTS: its public acts
   (and departed record, and key operations — its creator, who writes the tree's root, writes its acts) from the
   writers its acts bag names (`acts <space>`; its writers bag is a member's to read) — never from every device of
   every member, most of which never wrote one (10-24/25, live: those
   lookups were silent, and Discover showed no public space, then stayed "still arriving"). Not decided while a listed
   writer's feed is still opening (`storage.readOnly`, as a merged table's).
   SPACES WITHOUT MLS (K8.3): a space made or joined makes no MLS group and no MLS entry; a write waits on no MLS
   read; the node upkeeps it too (its mandate names every space — one with no MLS group as an empty group, epoch 0,
   never loaded: its list and BeeKEM keys followed, no MLS catch-up). A SELF-JOINER (code, open door) has no key yet: its `join` act is public (a membership move), its device names
   its account in the writers bag, and the tables judged row by row (acts, public acts, key operations) read every bagged
   device its account's devices confirm — the replay then counts the join by its proof, the list names it, the tree
   places it. (fx: listed 21 s after a join by code, placed 66 s, the space's post read 81 s.)
   THE ACCOUNT's GROUP ON BEEKEM (K8.5, design 10-26 — the last thing on MLS). The same tree as a space's (`groups`),
   its id the account's (`tree_id` of the DID), its MEMBERS the account's DEVICES, not its accounts:
   - WHO IS IN IS THE DEVICES LIST: the account's current devices (`account::devices` — credentials in its devices
     contract signed under its key log's current key, the card's `nodes`). One owner of the fact; the tree follows it
     as a space's tree follows its acts. An operation is taken when its author is a current device, and it adds only
     current devices and removes only devices no longer current (the account's `Check`, as `ListCheck` is a space's).
   - A DEVICE's MEMBER is the key its credential already names (the field MLS's signing key fills): the device's
     BeeKEM member is derived from that key's seed (`groups::Account::of`), so every existing credential, signed by the
     owner key, already binds member ⇄ device — nothing new to sign. Its LEAF (what an add encrypts to) is published
     as a row signed by that member, so no other holder of the account's data key can put a leaf of its own in its place.
   - EVERYTHING IN THE ACCOUNT's CHANNEL (`log`, the one shared tail under the account's DATA key — readable with the
     words before any group key, which is why MLS's escrows live there): each operation `o/<hash>`, each device's
     signed leaf `d/<member>`, each generation's escrow `g/<n>`. Not a per-device feed listed in the account's directory:
     the directory is sealed with the very keys a words login comes to recover. Merged by every device as a space's
     operations are; republished from a saved tree (`unpublished`). The ROOT's maker (`AccountCheck::writers`): the
     authors of the root operations found there, the lowest current one taken.
   - SEALING: the account's tables are sealed with the tree's current GENERATION (numbered from `FIRST_GENERATION`,
     never an MLS epoch's number); a device is WRITABLE (the account `MEMBER`, the session `OPEN`) when it is placed.
   - RECOVERY WITH THE WORDS: whoever makes a generation ESCROWS its secret to the account's encryption key (its key
     log's head: `account::escrow_seal`) in the channel (`g/<n>`), beside the operation. A words login opens every
     escrow (`escrow_open`) — every generation held at once, every table read — then its credential is written (login,
     as now) and the tree places it: any placed device adds it (reconcile), or it enters by itself, its credential the
     proof. Never a second tree: a device makes the root only when no device feed holds an operation, read whole.
   - A DEVICE REMOVED (the delete machine: the key log rotated, the kept devices credentialed again): no longer current,
     the next reconcile takes it out with a new generation — it reads nothing written after. Its OPERATIONS STILL COUNT
     (`AccountCheck::ever`: an author is any device credentialed under any key the log has had — judged by the current
     list alone, the root it made went unauthorized and the removal never landed); an ADD is of a current device only,
     so a removed one never adds itself back. The reconcile is given EVERY device with a signed leaf (it takes out only
     a placed member it has a card for), and an account write moves onto the newest generation BEFORE it is sealed
     (`storage` write: the account's tables too — a write right after a removal went out under the generation before,
     which the removed device read; fx/fy 10-27: now it reads nothing written after).
   - CARRIED OVER (10-24 rule: MLS-era history is not carried — EXCEPT the account's own data, which is the person's):
     a device holding the account's MLS epochs makes the tree (or is placed), then reseals every table it writes under the
     current generation (`storage.resealOwn`) — each device its own. The MLS epochs it keeps still open what other
     devices have not resealed yet; K8.6 drops them.
   - ONE DEVICE RESOLVES CONCURRENT HEADS: its members are separate devices, so two making the key change that resolves
     concurrent heads at once each made one the other then saw as concurrent (fx 10-26: a generation every ~6 s). Only
     the LOWEST placed device makes it; the others write with the newest generation they hold, which every device reads.
   - SAVED PER DEVICE (`g2/account/<member>` in the account's table): its members are devices — a tree saved in one
     shared row handed a second device the first's leaf secrets (fx/fy 10-26; a space's tree is one member's, shared).
   - UNTIL K8.6, ALSO SEALED FOR THE ACCOUNT's MLS EPOCH (`m/<n>/<epoch>`: each generation under a key from the
     account's CURRENT MLS epoch — `keys.mlsNow` — written by whoever reconciles, for the epoch it is at): every current
     device is in that MLS group, so one that comes back unplaced and without the words (offline while another made the
     tree) opens the generations and reads everything before anyone places it (fx/fy 10-26, the upgrade: PASS).
   - A WORDS LOGIN opens every escrow in the channel BEFORE its walk (`keys.joinOrMake`): every table sealed with a
     generation reads at once (fx 10-26: without it, a device joined through MLS held epoch 0 only, and the account's
     directory, resealed with the generation, did not read).
   - THE NODE's UPKEEP follows the account's tree as a space's (`keys_round::reconcile` with the devices list): a device
     added or removed while no page is open. Its member is THIS device's: the device seed the delegate KEEPS (`identity_device/`,
     set once — the signer seed of the account's MLS state if it has one, else a fresh seed the page stirs in:
     `Request::DeviceMember`; the page's `keys.deviceSeed` and the round read the same one); it reads the account's channel (the data key's tail, under its table
     key) and the devices list (key log, devices contract), merges, reconciles, escrows each new generation (`g/`, `m/`),
     keeps the generations as the account's epochs, and writes its rows to the channel signed with the data key behind
     the same fork guard as every upkeep step (`identity::upkeep_sign_channel`). Its channel is the identity's
     (`identity::CHANNEL`); every member whose page handed the node its inbox watch is served (an account in no space
     too), each its OWN round, every wake-up, its page open or not (as a space's follower: concurrent reconciles merge);
     how each member's round last went is in the upkeep status (`accountTree`). A RETIRED member (an empty mandate: the
     page moved to a newer build) is never served again, and a build every member left reads nothing at a wake-up: the
     node wakes EVERY build of the app it ever registered, every minute, all charged to one node-wide duty budget
     (freenet 0.2.146: 100 ms/s across delegates, 10 ms/s each; a wake-up that finds either spent is skipped). fx 10-27
     with 46 builds woken: the newest build's wake-ups were skipped for ~20 min after registering (its own budget full,
     so the node-wide one). fx/fy 10-27: a device back with every page of the account closed was placed by the node in 320 s. REQUIRED BEFORE K8.6 (MLS removed): a device that comes back after another
     made the tree reads through `m/` meanwhile, but is placed only once a placed device reconciles.
   MLS REMOVED (K8.6, design 10-27), in three phases, each through the gates:
   (6a) THE ACCOUNT ON ITS TREE ALONE — WHO ITS DEVICES ARE is the devices LIST (`keys.ready`: credentials on the key
   log's current key, `account_devices_now` — no MLS group loaded; `epoch` the key log's `seq`); this device is the
   account's (`MEMBER`, the session `OPEN`) when its node holds one, and WRITES for it once the tree PLACES it
   (`keys.writable`); the list is read again whenever the account's card moves (a device added or removed elsewhere). A
   DEVICE FROM BEFORE THE TREE whose node is not in the list carries the credentials its MLS group holds into the devices
   contract once (fx/fy 10-27: a v446 device had none there — never current, never placed). Its member SEED is kept by the delegate (`identity_device/`: a device with an MLS state keeps the
   seed its credential already names). A REGISTRATION or a WORDS LOGIN opens the generations' escrows (a words login),
   then writes this device's CREDENTIAL (`account::device_credential`: its node, its member, signed by the current owner
   key the words give) — no MLS group made or joined, no `info`/`e/` rows. The tree places it: the root's maker, a placed
   device's reconcile, or it ENTERS BY ITSELF — a current device the tree does not place adds its own leaf and changes
   the key (a words login with every placed device gone; added meanwhile by another too, both adds converge: `groups`
   tests). A device REMOVED: the key log rotated with the words and the kept devices credentialed again (as before),
   then the tree takes it out on its next reconcile — no MLS commit; a device whose node held a credential once and holds
   none on the current key forgets everything. The node's round takes the seed from the delegate; `m/` copies go on
   under the account's LAST MLS epoch (frozen: `keys.mlsNow` loads a device's MLS state read-only and catches it up,
   never commits). The 10-13 repair (`repairAccount`: MLS group info re-pointed) is gone with it.
   (6b) SPACES OFF MLS — a space's tables seal and read with its tree's generations alone (nothing waits on an MLS group
   before sealing; writes move onto a newer generation as it arrives); there is no MLS space group (`keys.group`,
   `setList`, the `spacekeys` state, the self-entry and its retries, the node's moved groups adopted, the owner's
   branch/epoch announcements all gone; the build bans an MLS space entry on a page). A member's keys are the TREE's
   word (`keyStatus`, the `group` machine: held / not-held / unreadable). The bridge (`keys-history`) carries no MLS epoch
   any more: what it holds from before is still read, a dropped tree generation kept as the epoch it is, a forked MLS
   epoch's other branches as storage's own ALTERNATES (`storage.keepAlternates`). The NODE: its file re-key reads a
   space's members from the list it kept (`rekey_round::space_of` — before, from the mandate's MLS group, so every space
   since K8.3 was UNLOADABLE and never re-keyed with the pages closed); its dead MLS follower is gone; the mandate's
   `epoch`/`state` stay in the wire, always empty. An MLS-era space is not shown (K8.4's rule).
   A REMOVAL TAKES BACK EVERYTHING (K8.7, design 10-27 — before 6c and before any new feature). THE CAUSE: one 32-byte
   DATA SEED (`account::data_seed`, from the words; every device holds it) carries three powers, and a removal (the key
   log rotated, the devices that stay credentialed again) revoked none of them — (1) WRITE authority over everything
   addressed by the data key: the card, the account's directory, its channel (`mls`: the tree's operations, leaves,
   escrows), its public tails (posts, mail, modlist, statements, write-credential tables, public items); (2) the
   account's MEMBER in every space's tree (`space_member_seed(data)`; every later leaf `leaf_after(seed, heads)`), so a
   removed device could derive the account's current leaf in every space and read every space's new content; (3) the
   inbox's seal key (`inbox_seed(data)`). Operations are now also verified (above), but a power held is not a power
   checked. THE FIX — a removal ROTATES the data seed:
   - TWO SEEDS. The NAMES seed — the inception's data seed, never changing — keeps what only names and finds: every
     table's blinded label and base key (`table_key` — the account's channel's too: what is secret in it is sealed row
     by row, and a rotation must still open the channel it is moving), `ACCOUNT_CHECK`. The DATA seed rotates. The key log's
     VAULT seals both (`names ‖ data`, 64 bytes; a 32-byte vault is both the same seed), so words entered after any
     number of rotations reach both.
   - THE ROTATION is the removal's own key-log event (`account::rotate_data`, the remover holds the words): its `data` a
     fresh seed's public key, its vault both seeds for the current words. ONE COMMIT: the new seed is kept as PENDING
     (`DataPending`) before the key log is put; its key signs the moved tables before it is kept; `DataKeep` — last —
     commits and clears it. A page stopped midway finds its pending seed is the head's and does it all again (each step
     the same twice) — never sealing while the tree still places a device the list no longer holds. The NEW SEED to the
     devices that stay: a row of the NEW channel (`k/<seq>/<n>`, the rotation event's seq) sealed with the account
     tree's generation made by the removal's key change — the new channel is at the head's key's address (anyone of
     the account reads it: its table key is the names seed's) and only the new key writes it, so the removed device can
     neither read the seed (it is out of that tree) nor overwrite it. A device that stays reads that channel, merges its
     operations (behind by several rotations, it reaches the newest at once), opens the seed, checks it against the
     head (the page and the node — the identity's `DataKeep` takes what they give it) and keeps it; the replaced seeds
     stay kept (`DATA_OLD`) to open what was sealed to them.
   - WHAT MOVES. Everything addressed by the data key is found through the key log's head (`directory.keysOf`, the
     node's `log.head().data`): the remover copies the card, the directory, the public tails and the CHANNEL to their
     new addresses, and the old ones are read by nobody. The channel goes CLEANED (the old one a removed device can
     still write): ONLY what the remover can vouch for — every operation (each merge judges it), a leaf only signed by
     its member, each generation's escrow and `m/` copy MADE AGAIN from what its tree reaches (under the account's last
     MLS epoch, where it holds it), the pre-tree words escrows (`e/`, `info`, `start`) as they stood; anything else
     dropped. A removed device can still READ the new channel (the names seed and the head are its): every secret there
     is sealed row by row, so it sees the tree's shape and timing only — a row kind sealed with the table key alone
     must never be added to it. The account's SPACE MEMBER is the new seed's: its card names the new member (credential signed by the new
     data key), and every space's tree takes the account's OLD member out and the new one in (one member per account: a
     placed member whose account's card names another is out). The inbox seals to the new key; the old seeds are kept
     (`DATA_OLD`) to open what was sealed before.
   - A TREE SAVED BY THE OLD MEMBER loads for the new one UNPLACED (`Group::load`: the operations kept, the owner the
     new member with its own leaf — the old one's keys never used again), and it enters by itself (fx 10-28: the old
     owner loaded with the new signer failed every change, "Identifier not found").
   - PROVEN by probes: after a removal the removed device cannot write the card or the channel where anyone reads, does
     not read anything written after in the account or in any space; the devices that stay go on; a device offline
     during the removal catches up; a words login after it reaches everything.
   (6c) THE MLS CODE GOES — the delegate's MLS requests and dead catch-up, the `mls` crate, the wasm package. KEPT, named
   for what they are — the account's PRE-TREE HISTORY, which migration needs and which needs no MLS code: the signer seed
   read from a saved state (`signer_seed_of`), the epoch secrets the identity keeps, and the words walk over the
   channel's escrow chain (`e/<epoch>`, each commit's escrowed next secret). An account none of whose devices ran K8.5
   still migrates: its first device to load makes the tree from that seed, reads its tables with the kept epochs, and
   reseals them.
   Phases (each through the gates): (1) the account's `Check`, signed leaves, the devices' members, in `groups`/
   `account`, tested alone; (2) the page makes or joins the tree beside MLS, seals new writes with the generation, reseals its own;
   (3) words login recovers from the escrows and is placed; (4) device removal and the writable gate by the tree; the
   node's upkeep; (5) K8.6 — MLS removed.
   A SHORT LIST STILL MERGES what the check takes (10-23): an operation taken on a short read is taken on the whole one
   ("ever listed" only grows — the entry rule above); one it does not take is DEFERRED, never refused. Merging nothing
   deadlocked a member one generation behind: the acts are sealed with the generation it lacked, so its list never read
   whole (live, Wren in Craftworks). The node merges a short list the same way (`keys_round::reconcile`): its
   generations kept, nothing reconciled until the list reads whole.
   AN OPERATION NO FEED HOLDS IS WRITTEN AGAIN (10-24): a device whose saved tree holds an operation the network's rows
   lack, and our check admits, writes it to its own key-operation feed, its author's signature as it was
   (`Group::unpublished`; from a whole list), from EVERY version of the account's saved tree (each device saves its
   own; the saved tree is read only once the account table has settled — a row not arrived yet was taken for no tree,
   and an empty one saved over another device's). A refused branch is never written again. (Live: the Ivvor DM's root was on
   no feed — the owner's own operations waited on it everywhere and Ivvor was never placed — while the owner's saved
   tree held it. fx: a root row emptied is written again in 15 s; B then placed and reading.)
   A PUBLIC TABLE IS READ BY ITS NAME (10-23): a space's `pub-…` tables (key operations, public acts, the public board)
   are in the clear and judged by what they hold — every writer's read by its name, never only where its sealed catalog
   or the open writers list names it. Found through a catalog sealed with its writer's newest generation, the operation
   giving that generation could never be read (Craftworks: three of the owner's own operations waited for ever on a
   predecessor in its other device's feed, which no catalog the page opened and no open-list entry named). Read by
   name FIRST, never after the writer's catalog answers (10-24: Wren's page waited on Ivvor's catalog, sealed with a key
   it lacked, and never opened Ivvor's key operations in Ivvor's New Space — a feed on the network).
   THE TREE IS WHAT THE CHECK MAKES OF THE OPERATIONS (10-23): from a whole list every page merges every operation
   afresh — one tree on every device. A saved tree that reaches generations the fresh merge does not (a branch on an
   operation the check now refuses — a member's stray root taken before the creator rule) is dropped; the bridge carries
   what was sealed with its generations as lost alternates, so every member reads it (Craftworks: the owner's other
   device's catalog sealed with such a branch's generation, 104120349).
   AN OPERATION IS TAKEN ONLY SIGNED BY THE MEMBER IT NAMES (10-27): the check judges the issuer an operation NAMES, so
   its signature is verified first (`Group::merge`, page and node alike) — before, nothing verified it, and whoever could
   write where an operation is read (a listed member's device; for the account's tree any holder of its data key, a
   removed device too) could write a key change in another member's name. One already held is taken without a second
   judgement: each operation is verified once per device.
   WHO RECEIVES IS DECIDED BY THE ONE WHO SEALS, NEVER BY WHAT AN OPERATION CLAIMS (10-28, after three reviews of
   history-judged admission: every rule that judged an operation by its predecessors, or by what was offered with it,
   read evidence its SENDER chooses — a kicked account built an add on its pre-kick view, a device never placed offered
   a removal of itself that never merged — and every rule that judged by who is listed NOW refused history and forked
   fresh merges; the walks cost O(N²) and stopped a 400-op tree converging). As MLS and Matrix: the sender of content
   decides who it is for. TWO RULES:
   (a) ADMISSION BY FIXED FACTS ONLY (`groups::admitted`, page and node): the signature verifies; the signer was EVER
   let in (`Check::ever_in` — an act, the founder's first placement; the account's tree: a device ever credentialed);
   an add names an act letting its target in (`Check::allows` — the account's tree: a device ever credentialed) and
   carries the leaf its member published (unknown: it waits); a removal names an act putting it out (a member its
   account replaced: none); one root, by the creator. With no current device (the key log moved, the devices that stay
   not credentialed on its new key yet) the account's check judges nothing: all wait (`Check::whole`). Nothing judged
   by who is listed now, nothing by an operation's own history: no fork, no stall.
   (b) A GENERATION IS WRITTEN WITH ONLY FROM A CLEAN TREE (`Group::writable`): the tree places nobody the list puts
   out NOW (`Check::in_now` — a space: its account on the roster, its member not one its account replaced; the
   account's tree: a current device) — its current generation, then, is the one storage SEALS with (`EpochWrite`);
   every other generation reached is kept for READING only (`EpochHold`; an MLS state saved, a build's handed keys:
   held too — the mark moves ONLY by `EpochWrite`). A tree placing one out CLEARS the mark (`EpochWrite` none): nothing
   is written — not even with the last clean generation, which a member just kicked held (10-28 review) — until the
   tree is clean again: a WRITE first makes it so (`members.readyToWrite`: the reconcile takes them out — its removal,
   then a key change; a removed member reads nothing after: BeeKEM, asserted — and the current generation marked, any
   device resolving concurrent heads), within WAIT.answer, else it is REFUSED. A member placed by an operation it
   should not have made (re-added by its own pre-removal view, a device taken off the list) reads nothing sealed:
   nobody honest seals for it. Whose a placed member is stays known when its card stops naming it (`Group::known`,
   recorded at every add offered, held ones too, and saved BESIDE the tree — the tree's own save stays what every build
   reads; the node: the members a card named before, kept): it is still judged, and taken out by its account's act.
   MARKED ONLY FROM A WHOLE MERGE (nothing waiting) and a list read whole: a tree stopped short, or a list read short,
   leaves the mark as it is (page and node alike). A write from inside a group's own queue never waits on that queue
   (`readyToWrite` answers at once: refused for now). The node's new SALT is sealed only once its tree was found clean at
   the generation written with, placing no account off the list now (`clean_at`) — the page's gate too. Left by design:
   a sealer acts on a kick once it has READ it — between the act reaching it and its reconcile, it seals as before.
   THE KEY STORE (`identity::KEYSTORE`, `beekem`: the saved trees, their leaf secrets) is keyed by the DATA seed — not a
   group's generation (a group's own state is saved there: waiting on it, a fresh registration's save waited on the
   generation the save was making — 10-28 review), and not the names seed (a removed device holds that for ever). A
   rotation moves its key: what was saved before reads no more, and the trees are merged afresh.
   (c) A TABLE SEALED BY A GROUP IS NEVER WRITTEN WITH ITS TABLE KEY (`data::Open::group_sealed`, page and node): that
   key — a space's from its id, the account's from the names seed — opens for anyone who names the space or ever held
   the account: it READS what came before any generation, nothing more. The NODE writes with the identity's mark too
   (`Reading::writing`), never the newest epoch it holds.
   (d) THE MLS BRIDGE (`m/`: a generation sealed with the account's last MLS epoch secret, for a device from before the
   tree) is written only while no device the account ever credentialed is out (`account::mls_bridge_open`, page and
   node): every device that was in the MLS group holds that secret, a removed one too — through `m/` it opened the
   generation after its removal, and the new data seed sealed with it (10-28 review). Closed, a device from before the
   tree enters it by itself.
   OPEN: a FRESH device's merge waits on a kicked member's history once that member's card stops naming it (a saved tree
   and the node's learned members keep whose it was; a fresh one has nothing to judge it by) — the binding of a member
   to its account must come from something the member cannot erase.
   AN ADD CARRIES THE LEAF ITS MEMBER PUBLISHED (10-28): an add names the added member's leaf key, and it is taken only
   if that is the leaf the member PUBLISHED (a space's: its card's, under the data key; the account's: its signed `d/`
   leaf) — unknown, it waits (`Check::leaf_of`, page and node). Before, an add carried any leaf: a listed member could
   plant a leaf it held in another's place and read on after its own removal.

6. **THROUGH THE LIFECYCLE**: `group` (HELD, BEHIND, OFF_BRANCH, NOT_HELD, STUCK; the `enter` step), `request` (the table
   in 3), the follower's `commit` step. No ask, no answer, no window, no timer.

**THE INVARIANTS — every rule derives from one; a change starts from a row, never from a symptom (owner 10-16:
"coordinated and synergized rules, not whack-a-mole"); a review checks the code against this table:**
| # | Always true | The one rule that keeps it (where) |
|---|---|---|
| I1 | Keys follow the list: an account holds an entry ⇔ it is listed | IN: the listed device enters itself (1); OUT: the follower's Remove (1); every commit checked (5) |
| I2 | A listed account without working keys gets them, needing nobody | its own external commit from the published info (1, 4) |
| I3 | Nobody unlisted gets keys | the check above MLS (5): an unlisted entry is skipped by every reader |
| I4 | Churn is bounded: a device enters only while it holds no usable state (never held, another branch, stuck) — entering gives it one, so it cannot repeat; BEHIND reads the logs; history gaps are asked of members, never re-entered | the `group` states (6) |
| I5 | Every node decides alike from shared data (the acts, the logs) | the replay (2), the lowest valid entry (5) |
| I6 | Nothing permanent from a short read | a removal only on a whole read of the list (`guards::whole`); an entry is the device's own, from its own read of its own listing |
| I7 | One actor at a time where one is needed (removals) | `admission::turn` + takeover |
| I8 | Requests never hang | the request lifecycle (3) |

**THE FLOWS (every one must hold before code):**
| # | Scenario | Flow |
|---|---|---|
| 1 | a new person joins by a code / the open door / a level | their own join act (proof) → the replay lists them → their device enters (external commit) |
| 2 | manual approval | request → let in (an act) → their device enters; or reject / cancel: ended |
| 3 | every other page closed | 1 and 2 need nobody else; a removal waits for any member (its node by turn) |
| 4 | a device that lost its keys / on another branch / stuck | it enters again by itself, its old entry out in the same commit |
| 5 | a member that missed commits | BEHIND → reads the logs |
| 6 | removed or banned | the act → off the list → a member's Remove; their own later external commit is skipped by every reader |
| 7 | a device dropped from an account | the account's list → out of the account's group |
| 8 | a PRO/VIP card falls below its level | off the list → Remove |
| 9 | Ivvor's space | the 7 written into the acts; each of their devices enters by itself on the new build's first load |
| 10 | an outsider posting commits | skipped by every reader (5) |

**Deleted** (each with its tests): the space-only join path and every ask/answer piece built 10-16 — "my key" and the
member bag, answers in the writers list, `guards::readd` / `post_key` / `once` / `fresh` / `answered`, the follower's
adds, `rules::admission`'s request decisions (the replay decides list entries), the per-kind admit paths in the page.
ENFORCED: `tools/group-check.mjs` — a second group implementation, an Add of another account's key, or a key request
anywhere fails the build.

**Order of work** (each: built, tested with a negative control, proven on fx traced, audited against the invariants):
- D1 the core (Rust): ONE `enter` (the account's words join and a space's listed entry), `entry_ok`, the ordered
  candidates of a position; tests with negative controls.
- D2 the page: the door, `info` in announcements, catch-up over both with `entry_ok`, enter on NOT_HELD / OFF_BRANCH /
  STUCK; the ask/answer layer deleted (page).
- D3 the node: its follower removes only, catches up by the same rule; its adds and answers deleted; group-check.
- D4 self-service list entries (code proof, open door, level) and the request lifecycle.
- D5 each kind's list on it (chats, audiences, circles, PRO/VIP).
- D6 release (a PIN once; old builds give back their background slot), then Ivvor's space — done when health shows all 7.

### Apps are installed, not built in

Today the 20 apps are one list written into the publisher (publish/src/main.rs:700), served as one site in one page
with one combined `uses` and full trust in each other. The permission boundary already exists one level up: the identity
delegate grants data per **(site, table)** (identity/src/lib.rs:989-997, `access.js`), and a site outside the home asks.

The app machine makes each app what a separately published site already is, and the home site the OS that runs it:

- **An app is a package with its own manifest**: name, icon, routes, `uses` (the tables it asks for), version = its
  hash. Built-in apps are written the same way: one path, nobody special.
- **Installed apps are a table** in the account (on the network, so they follow the person's devices). Install =
  add its manifest hash; uninstall = remove it and revoke its grants; its data stays the person's until they delete it
  (the delete machine). Updates: name → newest hash, the person told what changed in `uses`.
- **Each app runs in its own sandboxed frame** and reaches the system only through one message API — the capabilities
  (storage, items, spaces, files, people…) as the OS's calls. It cannot read another app's tables, the session or keys.
- **Grants are per app**, through the delegate grant that already exists (the app's hash in place of the site's): one
  prompt at install for its `uses`, the rest asked when first used.
- **Anyone can publish an app**: it is a contract like any page; a catalog of apps is a catalog (already a capability).

- **The OS and the apps (10-15).** The OS is what every app stands on, and stays in the page: Home, Settings, the
  account, Spaces and a space's home, the App Store, Tag (search across every kind), Contacts, Chat and Mail (the
  MLS groups and their keys), and the token pages (Wallet, Nodes, Network). Everything else is an APP and runs framed:
  Paste (framed already), then Wiki, Vote, Forms, Notes, Board, Blog, Videos/Audio/Images/Books,
  Captions, Drive, Bundles. `tools/app-check.mjs` holds the line: an app module not framed fails the build unless it
  is on its NOT-YET list, which only shrinks (an app moved is taken off it in the same change).
- **THE FRAME RUNTIME — one implementation, two hosts (10-15; as iOS/Android: the UI frameworks are shared libraries
  loaded into every app's process, privileged work goes through system services checked against the app's permissions,
  pickers run outside the app).** A frame runs the page's own loader and `ctx`: UI and PURE packages (theme, kinds,
  cards, the editor and renderer, an item's pure helpers) are loaded into the frame by their hash — the same bytes as
  the page's, never a frame version; STATEFUL capabilities (items, storage, space, files, identity, roles) stay in the
  host and `ctx.require` in the frame returns a PROXY whose calls cross the bridge, each checked against the app's
  grants (the manifest's `framed`: module → grants; `packages/apphost.js mountCtx`, `packages/frame-runtime.js`). THE
  BRIDGE carries what a phone's IPC parcel does: data by value (structured clone — a Map stays a Map, a total read's
  `.state` kept) and a function by reference, both ways (a handle whose calls come back to the host — a place's steps;
  a callback the frame passes, called back there; a Promise; an element as a place on the page). ONE CALL PATH: host
  data is an awaited call — a caller never uses a host answer as a value; PURE code (a capability's helpers, its
  constants) runs in the frame from the module's own bytes (`frame(cap, ctx)`) — never a second implementation. A
  SHARED RULE (vote-tally, wiki-pages, blog-press, editions, moderation) and BACKGROUND WORK (keep, video-studio) are
  the host's. A WRITE only on the person's gesture, FAIL-CLOSED: an exact set of read names passes. SYSTEM SURFACES the
  host draws: an app's settings in a space (in its place), an item's editions (a sheet: money never in a frame), the
  person menu; `theme.ask` is the one way to ask (no browser dialogs — a frame has none). GRANTS are what the code
  reaches (`tools/app-check.mjs`, `--fix`). Done 10-16: all ten apps framed. The audience and publish pickers stay in
  the app (10-16): they show only what its grants already give it (a space's roles and members) — a host-drawn copy
  would protect nothing. GRANTS ARE CALL BY CALL (10-16): `capability.method`, every call its code makes (app-check;
  the bridge refuses any other, said in the trace). WHAT IT MAY LIST is its own kinds: `items.list` / `items.inPlaces`
  of its apps' `needs` domains (by policy domain) and what the manifest declares it `reads` (Drive "*", Captions video and
  audio, Video and Audio captions) — an item by its REFERENCE is any kind's (holding it is the permission). Choosing among
  the person's own (the editor's "Find among yours", "Add to a bundle") is a SYSTEM PICKER (`apphost.pickSheet`): the
  app is given the one chosen, never the list. Phase 8 complete.
- **System controls are SYSTEM PICKERS.** A frame cannot draw the host's elements, and a copy of a control inside the
  frame would be a second implementation. So an app ASKS (`api.pick("audience", { space, kind })`, `api.pick("files",
  …)`, `api.pick("sort")`): the system shows its own control above the frame, the person chooses, the value comes
  back. The item page (votes, comments, actions) is already drawn this way, under the frame.

Cost: one frame per open app and messages instead of calls; packages shared by hash so a capability is loaded once per
frame at most. Order: after the lifecycle mechanism (phase 3), as part of the app machine (phase 5) — the boundary is
drawn once, where `OPENING`/`CLOSED` already are.

### Close at any time

People close tabs; they do not shut down. So no machine has a shutdown step and nothing is kept only for one: the page
may vanish between any two instructions, and what was there comes back when it is opened again — as a Mac restores its
windows after sleep.

- **Save on change.** A change is saved as it is made: settled actions (a post, a vote, a setting) are written at once,
  as today; text being typed is written to the account's private **drafts** table after a short pause (~2 s) and at
  once when the page goes `HIDDEN` — the last moment a browser reliably gives (`pagehide`/`unload` are not run on
  phones or when a tab is killed). A draft is replaced by the real write when it is sent, and deleted with it.
- **Drafts live on the network**, encrypted for the account (a sandboxed page has no local store, and a draft on the
  network follows the person to their other devices — open the same thing there and the text is there).
- **Resume, not restart.** `RESUMING` reads where the person was (the route is already in the address), the drafts for
  what is open, and owed saves the last page could not send (they are drafts until `HELD`), and puts them back.
- **An update is a resume.** `UPDATING` no longer guesses whether typed text would be lost: everything is a draft
  already, so the page reloads and resumes in the same place.
## 4. Faults: the failure edge of every machine

A fault is not a process; it is how any transition fails. Every machine's failure state (`FAILED{fault}`,
`REFUSED{fault}`, `unreadable{why}`) carries one of these kinds, and one table decides what happens next.

### What is wrong today

Errors are plain `Error(text)`; the cause lives in the text. ~558 swallow points (`.catch(() => null|{}|[])` 497, empty
`catch` 29, constants ~32) turn failures into "absent" — including the health reports themselves (`index.health(...)
.catch(() => {})` everywhere, loader.js:38). The loader classifies by a regex over log words (loader.js:51) and keys a
report by a hash of its message (:31): rewording a message changes its identity. ~194 sites show `e.message` to people.
Retries are a dozen private loops (statement 40 rounds, files 3, keep 3, keys 6×10 s, auth 3, node reconnect every 2 s
forever) and two different "ask" timeouts (15 s default vs `WAIT.ask` 30 s); none looks at what kind of failure it is.

### The structure

`packages/fault.js`, loaded by the loader:

```js
const KINDS = ["refused-expected", "login-needed", "not-yet", "absent", "sealed", "transient", "refused", "cancelled", "extension", "bug"];
class Fault extends Error { constructor(kind, what, { cause, d = {}, retry } = {}) { … } }
const classify = e => (e instanceof Fault ? e : new Fault("bug", e?.message ?? String(e), { cause: e }));

const POLICY = {                       // ONE table: retries and reporting by kind
  transient:  { tries: 4, wait: n => Math.min(5e3 * 2 ** n, 3e5), report: "final" },
  "not-yet":  { tries: 6, wait: () => 1e4, report: "final" },
  sealed:     { tries: 1, report: "always" },   // d: { epochs, held }
  refused:    { tries: 1, report: "always" },
  bug:        { tries: 1, report: "always" },
  "refused-expected": { report: "never" }, "login-needed": { report: "never" },
  extension:  { report: "never" }, absent: { report: "never" }, cancelled: { report: "never" },
};

attempt(source, what, fn, { policy })  // run → classify → retry by POLICY[kind] → report ONCE with diagnosis → throw
present(el, fault)                     // one wording per kind: "Join to open", "Log in to do that", "Still arriving…"
absentOnly                             // .catch(absentOnly): null for absent/not-yet, rethrows everything else
```

- Origins throw kinds, not text: `node.ask` → `transient {ms, reconnecting}`; a node refusal → `refused`; blocks lost →
  `absent`; access "no key for epoch N" → `sealed {epochs}`; `where` → `refused-expected`; the session gate →
  `login-needed`; extension stacks are classified, not regex-skipped.
- The collector reports through `classify` → `POLICY[kind].report`, keyed `kind|source|what` (`what` a stable label,
  the changing details in `d`): `fail()`, the error/rejection listeners, a package that does not load, and `attempt`.
  The match on log words ("failed", "could not" …) stays only until the apps throw faults themselves (build order 7),
  then goes. Mechanisms keep their own report names
  (`join-failed`, `account-recovery`) by throwing a `Fault` with `d`.
- A report that cannot be written is retried as `transient` and said in the trace — never swallowed.
- One `ask` timeout (`WAIT.ask`); private loops become per-call `policy` overrides.
- A failure that is HANDLED (not thrown on) is handled in one place, never by a call site's own fallback (owner
  10-13: "if it is one machinery, why still multiple implementations?"). Three forms, all in the loader, all reporting
  by kind (absent / not yet / slow / logged out / cancelled / an expected refusal are not reported one by one):
  - a capability's READ is TOTAL — `ctx.total(what, empty, read)` at its export: it never throws, gives its natural
    empty value (null, [], false) carrying its read state where it can (`.state: { s: unknown | unreadable | failed,
    why }`), so callers use the value and never catch it (items, space, roles, directory, index, files, keys, auth,
    bundles, vouch, person, recovery, membership, usage, captions, drive, conversation, keep, storage, `where`'s read);
    a DECISION that must not act on "unknown" (a join, a removal, a claim, a copy) reads `.strict` — the same read,
    throwing its fault, so the step stops (services read strict; views read total);
  - a best-effort WRITE (a mark, a notice, a record) — `ctx.background(what, promise)`: its value when it works,
    reported when it fails, never stopping the flow;
  - an OPTIONAL step (a browser codec, a package a site may lack, an internal read with a fallback) —
    `ctx.optional(what, promise, fallback)`.
  The build refuses anything else (`tools/failure-check.mjs`, run by build.sh): no `.catch(() => v)`, no
  `.then(f, () => v)`, no `ctx.quiet` in any package but the loader and framed apps (whose boundary is the frame API).

### Health: outcomes, not only errors (owner 2026-10-12)

An error collector hears only what throws. A page that never finishes, a login whose keys did not come, a space that
reads empty say nothing by themselves — and a person rarely knows what to report. So until the owner declares the
ecosystem stable, every page reports how it WENT, success included (sealed to support, never content; read with
`tools/health-watch`):

- **`visit`** — one per page load: sent when its first page settles (`#1`) and again when it is hidden with something
  new (`#2`…, at most every 5 min). Guest or member; for a member, how many spaces read; time to the first page; each
  page opened and its OUTCOME — `drawn` (settled: no work left, no placeholder, and in how long), `late` (settled
  after the stuck mark), `stuck`, `said` (a refusal shown) or `failed`; the faults met, by kind; reconnects to the node.
- **`stuck`** — at once, when a page is still working after 20 s: which components, and the oldest questions to the
  node still waiting.
- **Waits** — every question to the node is timed by what it was (ids taken out): how many, how many SLOW (≥ 3 s),
  never answered, the longest, the total; and the time spent QUEUED behind the get cap. Each visit carries its worst;
  health-watch sums them across visits, most time lost first — what stalls, what blocks, what to optimize.
- **Lifecycles** (§1) add their states as they land: a session that ended `OPEN_KEYLESS`, an account group
  `JOIN_FAILED`, a space `WELCOME_FAILED` are outcomes in the visit even when nothing threw.
- The hourly heartbeat (`seen`, members' spaces) stays.

## 5. No migration, no reset (owner 2026-10-12, 10-13)

Craftworks is in beta: the foundation lands with NO migration code — no converters for older accounts, spaces or acts.
And NO reset (10-13): Freenet deletes nothing, and a change that moves every address (a new code epoch) only makes old
data unreachable to the new app, not gone — while a new identity build alone (10-04) moves no address at all and leaves
old data readable by nothing that understands it. So the network stays as it is: the new code reads existing data (an
older space's apps turned on by name still count), and what cannot be opened — a listing pointing at a space or post
under keys nobody holds — is a READ STATE (§2): left out of what is shown, and counted in health. The owner's damaged
account stays usable from its other devices, or a new one is made; its guards (§3 account group) keep a stray group from
happening again.

## 6. Shipped patches this replaces

| Patch | Becomes |
|---|---|
| storage `channel()` reads without the directory (4bf650c) | R2 |
| keys: page restart after a join (4bf650c) | R3 |
| keys: wait for the channel, 6 × 10 s; logs read fresh, 2 × 5 s (41c9b43) | R1 `decided()` |
| keys: a words login never makes a group (41c9b43) | account-group guard `CREATING ⇐ REGISTERING ∧ absent` |
| keys/auth `account-recovery` reports (48ceed4) | account-group transition reports |
| loader: extension stacks skipped, `expected` refusals (f9d7c64, 8c2d17e) | §4 `extension`, `refused-expected` |
| statement page 40 rounds (8ca546a) | §4 per-call `policy` |

## 7. Build order — lowest layer first, each phase its own gate

1. **Faults**: `fault.js`, the loader collector, `node.ask`/`WAIT` (one timeout), `blocks`/`index` throwing kinds.
2. **Reads**: `state`/`decided()` in `tailAt` + R1 (closes every create-on-unknown at once), then R2.
3. **Lifecycle mechanism** + **account group** (taking over keys.js:290-376 and :102); **session** in shadow first.
4. **Gates and jobs**: `directory.publish`, asking in (`askIn`) and upkeep become **jobs** gated on account
   group `MEMBER`; **group** with failure reasons; every `setInterval` moved into a scope.
5. **Page, app, save, delete**: save on change with the drafts table, resume on open (an update is a resume), unmount
   on close, save states shown from one place, one delete path.
6. **R3/R4/R5/R6**: caches, re-open on key arrival, merged tables; one door that reads each contract once and then
   follows it; remove the reload and the polling loops.
7. **Apps**: `absentOnly` for reads, `present()` for messages; retire the private retry loops.
8. **Desktop** (ARCHITECTURE §1): apps installed (each its own manifest, an installed-apps table, per-app grants;
   their own frames and the system API before any app not ours is offered); Home screen, App Store, Spaces and Settings apps; a space's enabled capabilities
   (`enable`/`disable` acts in place of `app`); apps' `needs`/`can use` in manifests; each app's own space
   sidebar and settings schema; the header's space and app dropdowns, the rail and `spaces-panel` retired.
9. **Leftovers** (§5): listings that point at what cannot be opened left out of Discover and the directory, counted in health.

Each phase: a test with a negative control (the old behaviour fails it), the full-recovery check on fx/fy (a fresh
account with a handle, its own space, a message: recovered with no reload), then a release.
