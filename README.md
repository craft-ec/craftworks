# Craftworks

An app on freenet that is a WRAPPER, a LOADER and PACKAGES:

- `wrapper/` — `index.html` + `boot.js`, the only fixed code in the app's site. It fetches the loader from the
  loader's own site and hands over.
- `packages/loader.js` (+ `timeline.js`, the trace) — the loader's site. Reads the app's `manifest.json`, makes the
  header / body / footer slots, loads each package the first time something asks for it, checked by sha256.
- `packages/*.js` — the app's own components (`header`, `footer`, `home`, `who`) and the `node` service.
- `core/` — the one compiled package: the node's framing (`wire`) and the signer's questions.
- `contracts/` — the released contract code and the frozen signer this app publishes with, pinned by `SHA256SUMS`.
- `publish/` — the publish tool: packages as immutable containers, the loader and app sites through a node's signer.

    ./build.sh      build the core package
    ./publish.sh    publish through B (the shared lock, one tunnel); only packages the live manifest lacks are sent

Live: app site `B8Y3M8Z3eDzXoxKDmJnU72ysWGzQ9xjpYWGkbkMaUiZr`, loader site `CnTxCjneT3RfY4TBmdR3N4RsmGFBtebW32Mf75aEbomA`,
on any node at `/v1/contract/web/<site>/`.

The SDK is used only for its proven parts (wire, page / page-io site publication, probe helpers), pinned to one
commit. Earlier work is in `~/dev/craftworks-archive`.
