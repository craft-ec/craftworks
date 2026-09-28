# Proposal (freenet-core): passkeys for apps, brokered by the shell

Status: DRAFT — not filed. Measured 2026-09-28 (below).

## Problem

A web app served by a node runs in the shell's iframe
(`crates/core/src/server/path_handlers/assets/shell.html`): sandboxed WITHOUT `allow-same-origin` (an opaque
origin), with `allow="clipboard-read; clipboard-write; fullscreen *"`. WebAuthn is not usable there:

- `navigator.credentials.create` in the app frame → `NotAllowedError: The 'publickey-credentials-create' feature is
  not enabled in this document` (measured, Chromium, `http://localhost:<port>`).
- Granting `publickey-credentials-*` in `allow` would not be enough: an opaque origin has no effective domain to be a
  relying party.

Apps need passkeys to protect account keys without an always-on server: a passkey's **PRF** extension gives a
per-credential secret that seals an account key stored on the network; the passkey syncs across the person's devices
(iCloud Keychain, Google Password Manager), so a new device opens the account with Face ID / a fingerprint.

## Measured (2026-09-28)

- The shell's own document at `http://localhost:<port>`: create + get with PRF work (Chromium, virtual authenticator:
  32-byte PRF output).
- A real Mac: a passkey created for rp id `localhost` (Edge → iCloud Keychain, platform authenticator, Touch ID) —
  PRF supported, the same secret on every sign-in, and the passkey appears on the person's iPhone (synced). Signing in
  from SAFARI on the same Mac with that passkey gives the same PRF secret (`c0c404621af2`, a fingerprint): the secret
  is the credential's, not the browser's.
- `http://127.0.0.1:<port>`: `SecurityError: This is an invalid domain` — an IP is not a relying party id. Nodes are
  reached at `localhost` for this to work.

## Proposal

A shell-bridge message, like `clipboard` / `download` / `notification`:

```js
// app (iframe) → shell
{ __freenet_shell__: true, type: "passkey", id, op: "create" | "get", salt /* 32 bytes, app-chosen */, name? }
// shell → app
{ __freenet_shell__: true, type: "passkey_result", id, ok, credential /* id, for later gets */, prf /* 32 bytes */ }
```

1. **The shell asks the person, in its own UI**, naming the app (its contract id / title): "Sign in with a passkey for
   *App*" with a button. WebAuthn needs user activation in the calling document, and a postMessage carries none — the
   shell's own button provides it (Safari requires it), and is the consent step.
2. **The shell performs WebAuthn** as rp `localhost` (the shell's host), with `userVerification: "required"` and the
   PRF extension.
3. **The PRF salt is bound to the app**: the shell evaluates PRF at `SHA-256("freenet shell passkey" ‖ contract id ‖
   salt)`, never at the app's salt alone. Every app on every localhost node shares the rp id `localhost`; without the
   binding, any app could ask for another app's secret by using its salt.
4. **Only the PRF output and the credential id** go back to the app — never assertions usable elsewhere.
5. Refused when the shell's host is not a valid rp id (an IP): the app falls back to its own methods.

## Security

1. **One rp id for every app** (`localhost`): the shell binds the PRF salt to the requesting app. The contract id comes
   from WHICH iframe sent the message (`event.source` against the shell's own frame), never from the message — an app
   cannot name another app.
2. **Other pages on `localhost`** (rp ids ignore ports): any web server on this machine can prompt for the same passkey
   and compute the same binding (it cannot be secret). Mitigated only by the platform's prompt, which names the site
   asking; a local process that can serve pages is already inside the person's machine (it can read the node's data
   directory). A node served at its own name (e.g. `freenet.localhost`) moves the rp id off plain `localhost` and cuts
   accidental overlap with dev servers, not a determined local attacker.
3. **The prompt is the shell's**, drawn above the iframe (the app cannot cover or restyle it); one at a time, and
   rate-limited per app.
4. **Discoverable credentials**: a get may list every `localhost` passkey the person has; choosing another only yields
   a different secret, which unlocks nothing.
5. **What the app receives** is its own secret; what it does with it is the app's (as with any app-held key).

## Not in scope

- Hosted gateways (a public domain) work the same way, with that domain as the rp id.
- Where the app keeps what the PRF seals is the app's (Craftworks: the account key, sealed, on the network).
