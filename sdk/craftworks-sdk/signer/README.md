# signer

The one delegate: it holds the user's signing key and signs their heads.

**The shipped signer is FROZEN** (`frozen/signer.wasm`, pinned by `frozen/signer.sha256`). A delegate's identity is its code hash, so different bytes would be a different delegate, and every user a new identity. `build.sh` at the repo root ships the committed bytes, never a rebuild.

So this crate's SOURCE can drift from the shipped bytes. A change here builds and tests, but it is INERT: nothing ships it until a deliberate pin bump replaces `frozen/signer.wasm`, and that bump is where the change is tested against the new bytes. A pin bump is an identity change and needs the owner's say (rule 15) until sdk#14's migration exists.
