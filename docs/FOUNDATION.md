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
  | { s: "absent" }                                    // the node ANSWERED not-found, or a readable record says never made
  | { s: "present" }
  | { s: "unreadable"; need: { epochs?: number[]; tableKey?: true; grant?: true }; why: string };

t.state: ReadState           // now
t.decided(): Promise<ReadState>   // waits up to WAIT.answer; "unknown" is never reported as anything else
t.onState(fn)
```

- **R1 — No create or overwrite on unknown.** `write()` awaits `decided()`; `unknown`/`unreadable` throw a retryable
  `not-yet`/`sealed` fault (§4). `beforeCreate` and a PUT run only on `absent`. Enforced in `tailAt`, once.
- **R2 — "Not listed" is only an answer when the list was read.** `placeOf` returns `unknown` unless its catalog is
  `present`/`absent`; `known:false` comes only from `absent` or a readable "none". One change covers every directory and
  card lookup.
- **R3 — Unreadable is not terminal.** `unkeyed` becomes `waiting: Map<id, need>` (epochs, table key, grant). Key
  arrival (§1 events) re-reads exactly those; their state moves and subscribers redraw. No page reload.
- **R4 — Caches hold objects, never failures.** `tails`, `tables`, `directoryOpen`, `opened` delete themselves on
  rejection (as `memberKeys`, `readyShared`, `bags` already do); a later `known:null` open upgrades a `known:false` one.
- **R5 — A merged table is decided when its own feed and every listed writer's feed are**; a sealed or unread writer
  list makes it `unreadable`, never an empty `present`.
- **Defaults deny.** A policy, card or role not read is `unknown` and treated as the narrowest choice, never "anyone".

Callers: `const st = await t.decided(); if (st.s === "absent") create…; else if (st.s !== "present") wait`.
`ordering.tail.from()` returns `undefined` on unknown; `commitFrom` passes it through.

## 3. The other machines

| Machine | States | Gate / rule |
|---|---|---|
| **page** | `BOOTING → RESUMING → RUNNING ⇄ HIDDEN`, `UPDATING` | No shutdown state (see below). `RESUMING` restores what the last page left; `UPDATING` waits for **save** to have nothing owed; `HIDDEN` sends everything owed |
| **session** | `GUEST → UNLOCKING \| HANDING_OVER \| RECOVERING \| REGISTERING → OPEN \| OPEN_KEYLESS → LOCKED \| REMOVED` | `OPEN_KEYLESS` is derived on every load (logged in, the account's keys not held here) and blocks every write to the card, key packages, welcomes and asks; the page says "keys not recovered" and offers recovery again |
| **account group** (per device) | `NONE, CONFIRMING_CHANNEL, WALKING, JOINING, CREATING, MEMBER, MEMBER_PARTIAL_HISTORY, COMMITTING, JOIN_FAILED, REMOVED` | `CREATING` only from session `REGISTERING` and only when the channel is `absent` (§2); `JOINING` only with group info `present`; `keep()` writes `e/0`/`info` only in `CREATING`; a device removal holds `COMMITTING` (persisted) until its commit is in the log |
| **space** (per device) | `UNKNOWN, LOADING, HELD, NOT_HELD, AWAITING_WELCOME, JOINING, BEHIND, STUCK_OWN, OFF_BRANCH, REFRESH_OWED, WELCOME_FAILED{reason}, REMOVED` | `LOADING` needs account group `MEMBER`; only `WELCOME_FAILED{package-not-held}` may renew key packages or ask for a welcome again; a new device of a member gets space keys from the account's `spacekeys` table (keys.js:699-718) |
| **app** (per open app) | `OPENING → OPEN → CLOSED`, `FAILED{fault}` | `OPENING` needs page `RUNNING` and, for login-only apps, session `OPEN`; `CLOSED` ends its scope at once — its changes were already saved as made |
| **save** (per change) | `EDITING → OWED → PREPARED → SENT → HELD`, `REFUSED{fault}`, `RETRYING` | Every change is saved as it is made (below); `OWED` only on a decided tail (§2 R1); `RETRYING` by §4 POLICY; the page shows "saving…/saved/not saved" from this state, one place |
| **delete** (per object) | `REQUESTED → HIDDEN → GONE`, `REFUSED{fault}` | one path for items, spaces, devices and files; who may delete is the object's rule (roles), checked in `REQUESTED`; `GONE` only where the network can forget (a tail can hide, not erase — said to the person) |
| **job** (per background task) | `IDLE → RUNNING → IDLE`, `PAUSED`, `STOPPED` | upkeep, renew, sync, usage flush, heartbeat: each declares the states it needs (e.g. upkeep needs session `OPEN` + account group `MEMBER`) and is `PAUSED` otherwise; `HIDDEN` pauses jobs that only serve the screen |


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
4. **Gates and jobs**: `directory.publish`/`renew`, welcome-again asks and upkeep become **jobs** gated on account
   group `MEMBER`; **space** with failure reasons; every `setInterval` moved into a scope.
5. **Page, app, save, delete**: save on change with the drafts table, resume on open (an update is a resume), unmount
   on close, save states shown from one place, one delete path.
6. **R3/R4/R5**: caches, re-open on key arrival, merged tables; remove the reload and the polling loops.
7. **Apps**: `absentOnly` for reads, `present()` for messages; retire the private retry loops.
8. **Desktop** (ARCHITECTURE §1): apps installed (each its own manifest, an installed-apps table, per-app grants;
   their own frames and the system API before any app not ours is offered); Home screen, App Store, Spaces and Settings apps; a space's enabled capabilities
   (`enable`/`disable` acts in place of `app`); apps' `needs`/`can use` in manifests; each app's own space
   sidebar and settings schema; the header's space and app dropdowns, the rail and `spaces-panel` retired.
9. **Leftovers** (§5): listings that point at what cannot be opened left out of Discover and the directory, counted in health.

Each phase: a test with a negative control (the old behaviour fails it), the full-recovery check on fx/fy (a fresh
account with a handle, its own space, a message: recovered with no reload), then a release.
