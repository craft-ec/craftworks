# freenet-contracts

The platform's Freenet contracts. Each is its own Cargo workspace with its own
lockfile, because a contract's wasm hash is part of its network key.

| Contract | Holds | Status |
|---|---|---|
| `block` | immutable, hash-keyed bytes | built |
| `register` | one signed value per writer (key or k-of-n keyset) | built |
| `tail` | a writer's live head and write buffer: the tree's root plus rows not yet flushed, moved by signed deltas (the Register's params and signature rules, as a library) | built, not released |
| `set` | union of signed items, admitted by capability | built |
| `bag` | unsigned immutable pointers, top-M by work | built |

`./build.sh` → `build/*.wasm` (reproducible); `./hash-gate.sh` proves it is;
`./check-wasm.sh` runs and times each contract inside wasm32. Spec:
`craftworks-docs/ARCHITECTURE.md` §1.

### What a terminal Register record names

A Register can say "I moved to that one". The value it carries is the
successor's **identity** — `BLAKE3("RG01-succ" ‖ successor params)`, computed by
`register::succ::successor_value` — and never a Freenet instance id.

An instance id is `BLAKE3(code hash ‖ params)`, so it names CODE. A terminal
record is signed and immutable, so a record naming code would point under a
dead build the moment the Register contract is upgraded. An identity names the
register; a reader turns it into a current key through the epoch table, which
is the one thing that should know about code.

The exception is the HEAD's terminal, which does carry an instance id, because
nothing above the head supplies a table to resolve an identity with. That is
specified where the head is.

### What a contract's wasm hash commits to

The hash is part of the contract's network key, so what goes into it matters.
It commits to **the compiled code** — this repo's source, the pinned source of
every dependency, the toolchain and the build flags — **and to every
dependency's cargo IDENTITY, which is not its source** (see below). It does not
commit to where any of that lived on disk, nor to which LINE of a file a
statement sits on.

Both used to leak, through panic locations. Every `unwrap`, `expect`, index,
`/`, `%` and `#[track_caller]` callee records `file:line:col`, and those
strings reach the binary:

- **Line numbers.** Adding one comment line to `register/src/wire.rs` changed
  `register.wasm` from `a3be5ec4…` to `b5f09452…` — measured (#16). A
  documentation edit re-keyed every register in existence.
- **Directory names.** Cargo unpacks a git dependency into a directory named
  after the REVISION, so two revisions with identical code produced different
  wasm (#11); and the absolute path of cargo's own home was in the binary, so
  the same source built on two machines did not agree. The remaps closed the
  STRING leak. They did not close the identity one — #36 found the same symptom
  surviving them, for a different reason, below.

`contract-build.sh` — one copy of the flags for every contract, because three
contracts built three slightly different ways is the failure worth preventing
— closes both. `-Zlocation-detail=none` drops `file:line:col` from every panic
site the contract compiles, and the checkouts, the registry, both toolchain
homes and the repo root are remapped to fixed names, derived from what is on
disk (and from `CARGO_HOME`/`RUSTUP_HOME` where they are set) so that a rev
bump never needs an edit there. `--locked` pins the dependency set. Bumping a
rev that DOES change the code still changes the hash, which is the point.

**The remaps are emitted general first and specific last, because rustc applies
the LAST matching `--remap-path-prefix`.** Emitted the other way round,
`$CARGO_HOME=/cargo` swallows the registry and the git checkouts and the
specific mappings never apply at all — every dependency path comes out as
`/cargo/registry/src/index.crates.io-<index hash>/…` or
`/cargo/git/checkouts/<pkg>-<hash>/<rev>/…`, which is #11's revision name back
in the binary. With `-Zlocation-detail=none` on top, nothing shows it: the
flag removes the panic locations that would have carried those paths, so a
broken remap and a working one produce the same bytes. That is why the gate
builds each contract a third way, with the remaps and no location strip, and
reads the paths out of that. `file!()` and `module_path!()` in a dependency
are not covered by the flag, so this is a live exposure and not a cosmetic one.

`-Z` on the pinned stable toolchain needs `RUSTC_BOOTSTRAP=1`, so the bytes are
tied to that exact toolchain. A toolchain bump was always an epoch (#8), so
that is not a new exposure. The flags reach the wasm target only: `cargo test`
and native builds are untouched, and panics there keep their locations.

### A dependency's IDENTITY reaches the binary, and its source does not decide the bytes

Two revisions of `freenet-prolly` whose only source difference was inside
`#[cfg(test)]` produced two different `block.wasm` (#36). It is not a path
string: rebuilt with byte-identical `RUSTFLAGS` that remapped both dependency
directories to the same name, the two hashes still differed. Cargo derives each
dependency's `-C metadata` from its **package id** — which for a git dependency
carries the revision, for a registry dependency the version, and for a path
dependency the path, absolute unless it is inside the workspace root — and
rustc mixes `-C metadata` into the crate's stable id and so into the symbol
hashes of its items.

What that does to the output was measured twice, and the second time is the
one that matters:

- Between two `freenet-prolly` revisions: a pure reordering, a 6-cycle over
  functions 45–50, all 543 bodies identical, plus one call operand.
- Changing **only the `version` field** of a vendored dependency whose source
  is byte-identical: `block.wasm` went from 551 functions and 124,020 B to 543
  and 123,393 B (#37). Not a reordering — the code itself differs, because
  under `lto = true` and `codegen-units = 1` those symbol hashes reach LLVM's
  inlining and merging decisions.

Three consequences, all of them load-bearing:

1. **A dependency bump is an epoch, even a patch bump, even one whose source
   the contract never reaches.** There is nothing to "check whether it really
   changed": the version string alone re-keys the contract.
2. **Equal hashes for two different revisions are a COLLISION, not a
   guarantee.** The map from inputs to hash is not injective — sixteen builds
   of one source produced six distinct hashes, two of which collided. A hash
   can never stand in for a record of its inputs, which is what `released.toml`
   rows and `./release-check.sh` are for.
3. **A path dependency outside its own workspace root makes the hash depend on
   the CHECKOUT LOCATION.** Cargo hashes such an id absolutely. No contract has
   one; `wasm-check` does, which is why `check-wasm.sh` prints that harness's
   size and not its hash.

### Rebuilding a release from its row

`./release-check.sh <epoch>` takes a `released.toml` row, checks out the commit
it names **at a different absolute path**, asserts the toolchain, each
contract's `Cargo.lock` digest, its git-dependency revisions and its profile
field by field, and only then rebuilds and compares the hash and the byte
count. It also asserts that nothing touched the wasm after cargo, which is what
makes `postprocess.tools = []` a checked claim rather than an asserted one.

It does not gate ON the table: every value is used as an input to the rebuild or
as an assertion about the environment, so editing a row can only turn it red.
A missing field is a FAILURE, not a skip.

`./release-check.sh --write <n> --tag <t>` emits the row, refusing a dirty tree
or a tag that is not at HEAD — the first row is written by the gate, never by
hand. `./release-check.sh --self-test` writes a row, verifies it, then puts each
field back wrong and requires the gate to say which field: every mutant must be
caught, and all but one must be caught BEFORE the rebuild, because the cheap
assertions exist so that a wrong row costs seconds instead of a cold build of
every contract.

### The hash gate asserts properties, never a constant

`./hash-gate.sh` builds each contract about ten times and asserts what the hash
must NOT move with: two clean builds agree; a build from a different absolute
repo directory agrees; a build with a different absolute cargo home agrees; and
a build with a blank line inserted at the top of every source file of the crate
agrees. Every property is also run with the hardening switched off, through
`CONTRACT_BUILD_UNHARDENED=1`, so each control RUNS rather than being described
— and a control that fails to fail is itself a gate failure, unless the gate
can show why: for `block` a comment edit changes nothing either way, because
none of `block`'s own panic locations reach the binary at all, and the gate
says so and starts failing the day that stops being true.

It then builds each contract once more with the remaps and no location strip —
the only build in which the remaps are observable — and requires of it: no
cargo index-hash directory, no revision-named git checkout, no `$HOME`, at
least one path under `/registry/`, and at least one under `/dep/` for every
contract whose unremapped build shows a git checkout. The floors matter as
much as the absences: an empty binary satisfies every "no X" check, and
`/cargo/registry/src/…` contains the substring `/registry/` while the registry
remap is doing nothing, so the check is anchored to the start of the path.

It also asks cargo what each contract's wasm dependency CLOSURE contains, and
requires the instrumentation crate to be absent from it. That check exists
because a dependency's identity reaches the hash even when nothing calls it
(F37), so merely depending on a probe crate would re-key every contract — an
epoch bought by an unused import. It is deliberately not done by comparing
hashes with and without: the input→hash map is not one-to-one, so a coinciding
hash proves nothing about the closure, and that is exactly the case this guards.
A dev-dependency is fine and stays fine — it is in the native test build and
absent from the wasm, which is the pattern the contracts' `testing` feature
already follows.

What the gate deliberately does not do is compare against an expected hash. A
pinned constant goes red on every legitimate code change, and whoever bumps it
to go green has silently declared an epoch. Released hashes live in
`released.toml`, are written only by the #8 procedure, and are never read here.

### What stripping locations costs

Measured, not argued. `./check-wasm.sh` builds its harness with the flags a
contract ships with, and `CHECK_WASM_UNHARDENED=1` builds it without them;
three interleaved rounds of each, worst case, wasm32:

| | unhardened | ships | |
|---|---|---|---|
| `block` parse + `check_node` | 184.4 µs | 192.0 µs | **+4.1 %** |
| `block` `check()` with hash | 203.5 µs | 211.7 µs | **+4.0 %** |
| `register` `validate_state` (16-of-16) | 2,862 µs | 2,879 µs | +0.6 %, within spread |
| `register` stale replay, eager | 3,796 µs | 3,814 µs | +0.5 %, within spread |

So the contracts got smaller and `block`'s validation got slightly slower —
the opposite of the direction one would guess from "this only removes data".
The cause is `-Zlocation-detail=none` itself and not the remaps: built with
the remaps alone, `block` times 185.0/185.4 µs and 202.5/204.1 µs, i.e. the
unhardened figures. Panic sites that no longer differ by location merge, and
the code around them is laid out differently.

Four per cent of 190 µs, once per block validated, bought against a
documentation edit re-keying every block on the network. Worth knowing; worth
re-measuring at the next toolchain bump.

Residual `.rs` strings in the built wasm all come from the precompiled
standard library (`/rustc/<toolchain>/library/…`, `/rust/deps/…`). They are
fixed by `rust-toolchain.toml` and do not move with this repository's source —
which property `c` of the gate measures directly, per contract.

`./check-wasm.sh` runs each contract's validation inside wasm32, where it
actually runs: for every contract it checks that a good state is accepted and a
corrupt one is not, then times the worst case a host can be handed — for `block`
a leaf filled to the boundary rule's 12 KiB measure with the smallest entries the
format allows, for `register` a 16-of-16 keyset with a full-size value and
evidence. It needs `node` and the `wasm32-unknown-unknown` target, and fails if
either is missing rather than skipping.
