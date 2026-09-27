# Craftworks — rules

1. Search before writing: any code that talks to the node, fetches, retries, queues or reconnects uses an existing
   door (SDK wire / page-io, or this repo). Never a second path.
2. One change at a time, verified on a private node, then published, then committed.
3. Test only what changed. No full suites.
4. The wrapper is loader-only; the loader is its own site; everything else is a package named by its hash.
5. Publishing goes through B under the shared lock; the owner's node (7509) is read-only for tools.
6. The frozen signer is identity: `contracts/signer.wasm` changes only with the owner's say.
