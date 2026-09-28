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
  PRF supported, the same secret on every sign-in, and the passkey appears on the person's iPhone (synced).
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

## Not in scope

- Hosted gateways (a public domain) work the same way, with that domain as the rp id.
- Where the app keeps what the PRF seals is the app's (Craftworks: the account key, sealed, on the network).
