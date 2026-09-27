// AUTH, a service: asked for only when something needs a logged-in person (`(await ctx.require("auth")).session()`).
// Like any website, it shows its dialog then, and not before. What a session is:
// - the DID is the ACCOUNT; a MEMBER is one device key admitted to it, kept by the identity delegate on this node;
// - on this device a PIN opens a member. Several people (or several members of one person) can share a device, each
//   with their own PIN;
// - "Log in with this device" makes a new account: a member key, and a DID (owner seat + vault on the network). The
//   vault keeps the owner key under a recovery PASSPHRASE: the way in from any computer, and back after losing every
//   device.
export async function start(ctx) {
  const id = await ctx.require("identity");
  const { core, glue, ask } = await ctx.require("node");
  let current = null;

  const opened = a => {
    current = { member: a.unlocked.member, did: glue.did_of(hexBytes(a.unlocked.did)) };
    ctx.log("logged in", { what: `${current.did.slice(0, 24)}… member ${current.member.slice(0, 12)}…` });
    return current;
  };

  async function session() {
    if (current) return current;
    const w = await id.who();
    if (w.unlocked) return opened(w);
    return dialog();
  }

  async function logout() {
    await id.lock();
    current = null;
    ctx.log("logged out", {});
  }

  // Put a Register the first login made; resolves when the node accepts it.
  async function put(p, what) {
    const said = await ask(p.frames, s => (s.kind === "put" && s.key === p.id) || s.kind === "refused", what, 60000);
    if (said.kind !== "put") throw new Error(`${what}: the node refused (${said.said})`);
  }

  // A new account. Its keys live only inside this call (the member seed goes to the delegate; the owner seed into the
  // vault). Kept across a retry with another PIN, so one attempt makes one DID.
  let made = null;
  async function create(pin, passphrase) {
    if (!made) {
      const member = crypto.getRandomValues(new Uint8Array(32));
      const owner = crypto.getRandomValues(new Uint8Array(32));
      const a = core.new_account(
        await ctx.require("register-wasm"), owner, id.publicOf(member), passphrase,
        crypto.getRandomValues(new Uint8Array(16)), crypto.getRandomValues(new Uint8Array(12)));
      owner.fill(0);
      await put(a.seat, "putting the owner seat (the DID)");
      await put(a.vault, "putting the vault");
      ctx.log("account made", { what: a.did });
      made = { member, did: a.didBytes };
    }
    const r = await id.provision(made.member, made.did, pin);
    if (r.unlocked) {
      made.member.fill(0);
      made = null;
    }
    return r;
  }

  function dialog() {
    return new Promise(resolve => {
      const box = document.createElement("div");
      box.className = "auth-dialog";
      box.innerHTML = `
        <style>
          .auth-dialog { position: fixed; inset: 0; background: #0008; display: grid; place-items: center; z-index: 20; }
          .auth-dialog form { background: var(--bg, #fff); color: inherit; padding: 1.2em 1.4em; border-radius: 8px;
            width: min(24em, calc(100vw - 32px)); display: grid; gap: .6em; }
          .auth-dialog form[hidden] { display: none; }
          .auth-dialog input { font: inherit; padding: .4em; }
          .auth-dialog .said { min-height: 1.2em; font-size: .9em; }
          .auth-dialog .alt { font-size: .9em; }
        </style>
        <form class="pin">
          <strong>Log in</strong>
          <label>PIN <input name="pin" type="password" inputmode="numeric" autocomplete="off" required></label>
          <button>Unlock</button>
          <p class="said"></p>
          <button type="button" class="alt">New here? Log in with this device</button>
        </form>
        <form class="new" hidden>
          <strong>Log in with this device</strong>
          <label>Choose a PIN (6 or more) <input name="pin" type="password" minlength="6" autocomplete="off" required></label>
          <label>Recovery passphrase (4 or more words, write it down)
            <input name="pass" type="password" minlength="20" autocomplete="off" required></label>
          <button>Create my account</button>
          <p class="said"></p>
          <button type="button" class="alt">I have a PIN on this device</button>
        </form>`;
      document.body.append(box);
      const [pinForm, newForm] = box.querySelectorAll("form");
      const say = (f, t) => (f.querySelector(".said").textContent = t);
      box.querySelectorAll("button.alt").forEach(a =>
        a.addEventListener("click", e => {
          e.preventDefault();
          pinForm.hidden = !pinForm.hidden;
          newForm.hidden = !newForm.hidden;
        }));
      const done = a => {
        box.remove();
        resolve(opened(a));
      };
      pinForm.addEventListener("submit", async e => {
        e.preventDefault();
        say(pinForm, "Checking…");
        const r = await id.unlock(pinForm.pin.value).catch(err => ({ error: err.message }));
        if (r.unlocked) return done(r);
        say(pinForm, r.wrongPin ? `Wrong PIN. ${r.wrongPin.triesLeft} tries left on this device.`
          : r.locked ? "Too many wrong PINs: this device is locked. Use your key file to set a new PIN."
          : r.error ?? `Refused: ${r.refused}`);
      });
      newForm.addEventListener("submit", async e => {
        e.preventDefault();
        say(newForm, "Making your account…");
        const r = await create(newForm.pin.value, newForm.pass.value).catch(err => ({ error: err.message }));
        if (r.unlocked) return done(r);
        say(newForm, r.refused === "PinTaken" ? "That PIN is taken on this device: choose another."
          : r.locked ? "This device is locked after too many wrong PINs."
          : r.error ?? `Refused: ${r.refused}`);
      });
      (pinForm.hidden ? newForm : pinForm).querySelector("input").focus();
    });
  }

  return { session, logout, current: () => current };
}

function hexBytes(h) {
  return new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
}
