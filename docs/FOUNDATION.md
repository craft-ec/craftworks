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
  list makes it `unreadable`, never an empty `present`.
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

A person's SEND is a save they wait on: sent through the ONE action (`theme.action` — a comment, a chat message alike):
its text kept and "Sending…" said until it is HELD, cleared only then; REFUSED, the text stays with why (10-22: Chat
cleared the text at once and its refusal came up to 2 minutes later — the person gone, a message "lost"). The JOURNAL
keeps each change ONCE across pages: a step (nothing moved) the same as the last one recorded for its key is not written
again, by any page or load (10-22: one refused key tree recorded dozens of times an hour).


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
