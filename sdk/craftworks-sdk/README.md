# craftworks-sdk

App-facing SDK. Every substrate capability is written once in Rust and exposed to
JavaScript the phase it lands: `collection` (1) · `identity` `query` `subscribe` (3) ·
`inbox` `edge` `stream` (6) · `cap` (8) · `file` `blob` (9) · `ledger` (10).

    ./build.sh                     # → pkg/web (ES module) and pkg/node (CommonJS)
    ./gate.sh                      # EVERYTHING this repo checks with, once

`gate.sh` is the canonical check, and it is the only place the commands are
written down. It derives what to test from the workspace itself, so a crate
added later is covered without anyone remembering; it prints what it RAN and
the per-member counts, not just a verdict; and a step it cannot run is a
failure with its reason, never a skip.

This block used to list the commands instead, and that is exactly how a run of
plain `cargo test` — which covers the ROOT PACKAGE ONLY — reported 175 tests
into two merged pull requests while eight workspace members sat out. The real
number is 358. Nothing failed and nothing warned; the only tell was a count
that did not move.

## Build a site on the SDK

A whole site on the SDK alone, with no builder: [`examples/notes/`](examples/notes/index.html). Serve the repository root after `./build.sh` and open `/examples/notes/#node=<your node's ws port>`. The engine runs in the page; the node holds only the signer.

The page's whole program, verbatim (`tests/js/readme-notes.test.mjs` fails if this block and the page ever differ):

```js
import { load } from "../../pkg/web/index.js";

const $ = id => document.getElementById(id);
const say = (text, bad = false) => { $("status").textContent = text; $("status").className = bad ? "bad" : ""; };
const params = new URLSearchParams(location.hash.slice(1));
const port = Number(params.get("node"));
let saving = 0;

try {
  // NO DEFAULT PORT: which node gets your key is a decision, not a default.
  if (!Number.isInteger(port) || port <= 0) throw new Error("say which node: add #node=<port> to the URL");
  const sdk = await load();
  // ONE CALL: connect, provision the node if it needs it, and hand back a db.
  // "saving N…" comes from the session, which counts every write not yet
  // published — closing the tab before 0 would lose them.
  // THE APP this is. A person has one tree, divided by app: `notes` here
  // is this app's own, and no other app can write it.
  const app = params.get("app") ?? "notes-example";
  const handle = await sdk.open({
    app,
    port,
    onEvent: e => { if (e.kind === "saving") { saving = e.count; render(); } },
  });
  const { db } = handle;
  await db.define("notes", { type: "Note", fields: [{ name: "text", kind: "text", required: true }] });

  // A BINDING: the list re-reads by itself when the data changes.
  const notes = db.bind("notes", { limit: 500 });
  notes.subscribe(render);
  await notes.reload();

  function render() {
    const rows = notes.getSnapshot();
    $("list").replaceChildren(...rows.map(r => {
      const li = document.createElement("li");
      const span = Object.assign(document.createElement("span"), { textContent: r.fields.text });
      const edit = Object.assign(document.createElement("button"), { textContent: "Edit" });
      edit.onclick = async () => {
        const text = prompt("Edit note", r.fields.text);
        if (text) await db.update("notes", r.id, { text });
      };
      const del = Object.assign(document.createElement("button"), { textContent: "Delete" });
      del.onclick = () => db.delete("notes", r.id);
      li.append(span, edit, del);
      return li;
    }));
    say(saving > 0 ? `saving ${saving}…` : `${rows.length} note${rows.length === 1 ? "" : "s"}`);
  }

  $("add").onsubmit = async e => {
    e.preventDefault();
    const text = $("text").value.trim();
    if (!text) return;
    $("text").value = "";
    await db.put("notes", { text });
  };
  // The acceptance seam: the tools read the db through this.
  globalThis.__notes = { db, notes, saving: () => saving, head: () => handle.headId() };
  render();
} catch (e) {
  say(`Could not open: ${e.message}`, true);
}
```

What it relies on, and what each part is for:
- **`sdk.open({ app, port })`** connects, provisions the node if it needs it, and hands back a db once it's ready. There's no default port: which node gets your key is a decision. `app` is required: a person has one tree, divided by app, and every name an app uses (`notes`) is its own. It can't write another app's data. `db.other(app)` reads another app's data, read-only.
- **`onEvent` `saving`** counts every write not yet published. Show "saving N…" until 0; closing the tab before then would lose them.
- **`db.define`** gives a collection its schema, once. **`db.bind`** gives a list that re-reads itself when the data changes.
- **`db.put` / `db.update` / `db.delete`** are the writes; each is published to the node.

`examples/notes/acceptance.mjs` runs this page live against a private local node. It checks that 50 notes put come back after a reload, a second tab sees them, and an update and a delete reach that second tab.

## Collections

```js
import { load } from "./sdk/index.js";
const sdk = await load();
const db = new sdk.Db();                         // in-memory for now
db.define("tasks", { type: "Task", fields: [
  { name: "title", kind: "text", required: true }, { name: "done", kind: "bool" }] });
const t = db.put("tasks", { title: "hello" });   // { id, created, updated, fields }
db.update("tasks", t.id, { done: true });
db.scan("tasks", { reverse: true, limit: 50 });  // newest first
```

- Field kinds: `text int float bool time bytes ref`. A write that violates the schema is refused.
- **Schemas only grow** — append optional fields; nothing else. Old records stay valid, old readers ignore new fields: no migrations.
- Record ids are 16 bytes (ms ‖ device ‖ tail), time-sortable, strictly increasing per generator.
- Everything sits on a four-operation `Store` (get · put · delete · scan). Today an in-memory map; the prolly tree, then the node's engine, plug in behind it.

`tests/vectors.txt` is checked from both languages, so a binding that mangles
bytes on the way in or out cannot pass both. Needs `wasm-bindgen-cli` 0.2.128
(pinned to the crate version in `Cargo.toml`).

## What a build says it is

```js
sdk.buildInfo();
// { version: "0.1.0", rev: "a178e3a", prollyRev: "afcada3", formatTag: "PT01" }
```

Every field comes from the build, and each from the artefact that actually
decides it:

| field | where it comes from | why that source |
|---|---|---|
| `version` | `CARGO_PKG_VERSION` | the crate's own |
| `rev` | `src/build_rev.txt` inside a `git archive`, else `git rev-parse` (with `-dirty`) | see below — it must not be passed in |
| `prollyRev` | `Cargo.lock` | the manifest says what was ASKED for; the lock says what was RESOLVED, and only that was compiled |
| `formatTag` | `freenet_prolly::node::MAGIC` | a tag this crate spelled itself would keep saying `PT01` the day the library moved on |

**Why `rev` is stamped rather than passed in.** A consumer pins a revision and
copies the built wasm into its own tree; the check worth having is "are these
bytes from the revision I asked for". If the consumer handed us its own pinned
rev to bake, both sides of that comparison would come from the same value and a
stale wasm would still match. So the rev comes from the source: `git archive`
rewrites `$Format:%H$` in `src/build_rev.txt` for files marked `export-subst`,
which is how a tree with no `.git` still knows its own commit.

That mechanism fails silently — lose `.gitattributes` and `rev` becomes
`unknown` — and the unit tests cannot see it, because they run in a checkout
where `git rev-parse` answers and the placeholder is never used. So it has its
own gate, which builds the way a consumer builds:

    ./tests/archive-provenance.sh

**Do not carry this into a contract.** Baking a rev into a wasm changes that
wasm's hash on every commit. That is free here, because this wasm is not
addressed by its hash — but a contract's wasm hash is part of its network key.

## The tree library pin

`freenet-prolly` is pinned by revision in `Cargo.toml`, and that revision must be
**the one the Block contract validates with**. A node written under one version
of the split rule and checked under another is refused by every host — and no
test inside this repo can see that, because both sides of those tests are the
same library. `tests/prolly_pin.rs` compares the two repositories' pins directly;
it is the only gate here that can.

Changing the pin moves every root hash. `tests/rootvec.rs` pins the root of a
fixed dataset so that movement is visible, and `tests/root_vector.txt` is the
value. When it fails, that is not a number to update on its own: every root
written under the old format now names something no reader will find.

| pin | fixture root |
|---|---|
| `afcada3` (before the parity rule) | `358d4094f3a2caede057d0601e98a92d2807d0a7aca88c3aa353e607d7c2a51c` |
| `e2756c8` (parity) | `e9d30de0e61ca4c54282e83cffed9b6d03bfa20df6b6f40ef0d75d8566719081` |
