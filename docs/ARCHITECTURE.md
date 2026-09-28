# Craftworks architecture

Settled with the owner, 2026-09-28. This is the shape everything is built to; code that does not match it is a bug.
Where the code is today and what is next: [Implementation plan](#implementation-plan) (and `.claude/feature-progress.md`).

## 1. The one division: capabilities and apps

Every package is exactly one of two kinds.

- **A capability** has no UI. It defines a **shape** (what a kind of thing is) or a **rule** (what may be done to things),
  once, for every object it applies to. Capabilities never know which app uses them.
- **An app, page or component** is UI only. It composes capabilities and owns no data model of its own.

A capability that applies to things applies to **every** kind of thing: moderation moderates a post, a comment, a vote or
a label alike; governance decides who controls one note or a whole company.

## 2. Primitives (on the network, or on the node)

| Primitive | Is | Used for |
|---|---|---|
| **Block** | immutable, content-addressed | tree nodes, parity (erasure), attachments |
| **Tail** | one writer's signed sequence | a table's head and newest rows |
| **Log** | an ordered, self-certifying chain of signed events | an account's key log (the DID); an object's or space's log of acts |
| **Register** | one signed value | whoami (words → DID), site versions |
| **Set** | small owner-signed items | the account's member list today (replaced by the account's log, §4) |
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
| `membership` | joining, leaving, inviting, removing — for objects with members |
| `administration` | an object's settings and policies |
| `moderation` | acts on objects (hide, remove, label) and on actors (mute, ban) |
| `ordering` | one agreed order of acts on an object: its own log, or its container's |

### Underneath

| Capability | Owns |
|---|---|
| `node` | the connection to this machine's freenet node |
| `identity` | the DID (its key log), the identity delegate: members (nodes), PINs, sessions, signing, MLS state |
| `auth` | sessions, PIN login, recovery words, changing words (no UI: the dialog is `login`) |
| `keys` | table keys per epoch (MLS), handed to granted sites; escrow of epoch secrets to the recovery words. The MLS protocol runs in the page's core (mls-rs needs the page's randomness and clock); the identity delegate is the vault — it keeps the member state and the epoch secrets, and derives table keys for granted sites |
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
- **Ordering.** Shared state changes by acts in an ordered log; an act counts only if its signer's role allowed it at
  that point. First: one log per object/space with a deterministic tie-break. Later, when needed: witnessed snapshots
  (a known witness set, quorums that intersect).
- **Roles are per space** (holding a role never crosses spaces); definitions are built-in, published templates, or local.
- **Confidentiality.** Every private table is sealed; its key is **MLS export(epoch secret, table)**, the epoch being its
  generation. MLS is the one key manager: a space's members, or your account's nodes. Epoch secrets are also escrowed
  to the recovery words, so the words alone recover history. Newcomers' access to history is a per-space policy.
- **Sealing covers whole tree nodes**, so keys stay ordered inside sealed blocks (range reads work) and counts and shape
  are hidden.
- **Federated data.** Nobody writes into another's store: a message, comment or reaction is content in the **author's**
  table plus a **pointer** in the target's index (sealed for private use). A group's data is one table per writer,
  merged by readers.
- **Indexes are one shape** with a visibility — public (directory, tag index, comments) or sealed (inbox) — as edges
  are (public tags, private labels).
- **Durability.** Immutable blocks are erasure-coded (every node's children and the root's group of one) and read by a
  race against their group. Changing contracts (tails, logs) are kept by re-publishing their signed states.
- **Grants.** A site gets a table's key only with the person's grant (once per site, for all the kinds it uses); the
  home site needs none.

## Implementation plan

Each phase: build → the private node (all three sites) → published through B → committed. Status as of 2026-09-28.

| # | Phase | Capabilities | What it delivers | Status |
|---|---|---|---|---|
| 0 | **Restructure to the shape** | all | packages renamed/split to §3–4: `data` → `storage`, `auth` → `auth` + `login`, grants → `access`, `pins` + `tags` → `edge` + components `pin-button`, `label-menu`, `timeline` → `trace`; no behaviour change | done |
| 0b | **Theme** | `theme` | the tokens and base styles; every component and page moved onto them; dark mode everywhere | done |
| 1 | **Keys and sealing** | `keys`, `identity`, `storage` | MLS in the core; your account as an MLS group of your nodes; table key = export(epoch, table); escrow to the words; sealing of whole tree nodes (a hook in freenet-prolly); tables sealed today by row are sealed over | — |
| 2 | **Writers and merge** | `storage` | one sequence per writer (per node now, per member later); readers merge, causally ordered | — |
| 3 | **Reads** | `storage`, `blocks` | range (latest N, older pages) and change-only reads (tree diff) | — |
| 4 | **Lifecycle** | `keep` | re-publishing, retention, flush when quiet, health; blinded table names | — |
| 5 | **Rules over objects** | `ordering`, `governance`, `roles`, `access`, `membership`, `administration`, `moderation`, `space` | the Log for objects and spaces; acts and their rules; spaces | — |
| 6 | **Shapes** | `content`, `edge` | notes as content; pins, labels, likes, votes as edges | — |
| 7 | **Index** | `index` | the Bag: inbox (sealed pointers), directory (public, handle + id), comments | — |
| 8 | **Chat** | `chat` page | a space + its MLS group + content (messages) + index (inbox) | — |
| 9 | **Privacy and observability** | all | padding, sealed senders, no time-sortable ids in private spaces; health in `trace`/`account` | — |
| 10 | **When needed** | `ordering`, `roles`, `keys` | witnessed snapshots; enterprise role templates and escrow | — |

Built already (in the shape since phase 0): identity with rotatable keys (key log contract `idlog`), PIN
logins moved across delegate builds, per-site grants, tables (tail + tree, flush, catalog), erasure with raced reads and
rebuild (`blocks`), row sealing (replaced in phase 1), pins, tags, Notes.
