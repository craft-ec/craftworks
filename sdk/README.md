# sdk/ — the outside code Craftworks uses, kept in this repo

Copied 2026-10-09 from the exact revisions Craftworks pinned, keeping only the crates it builds with:

| Folder | From | Revision | Crates kept |
|---|---|---|---|
| `craftworks-sdk/` | github.com/craft-ec/craftworks-sdk | f079914 | the root crate, contract-keys, core-types, engine, page, page-io, pieces, probe, protocol, signer, signer-proto, tail, wire |
| `freenet-prolly/` | github.com/craft-ec/freenet-prolly | 17d67d8 | freenet-prolly |
| `freenet-contracts/` | github.com/craft-ec/freenet-contracts | 689ca61 | register, tail, and `contract-build.sh` (the contracts' build definition) |
| `craftworks-instrument/` | github.com/craft-ec/craftworks-instrument | 3ad6f3d | instrument |

Every dependency on them is a `path`. Each folder keeps its own workspace (excluded from the root one), as before.
Two test-only dependencies still name git (`wire`'s `craftec-webapp-contract`, `freenet-prolly`'s `instrument`): not in
Craftworks' build graph.

Moving them changed some built bytes (a crate's source — git or path — reaches its wasm): the identity delegate and the
core and MLS packages (new hashes at the next publish, as any delegate change), and the `devices` and `token` contracts
(new ids). Five contracts rebuilt byte-identical: idlog, sealed, bag, piece, beacon.
