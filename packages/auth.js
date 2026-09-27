// AUTH, a service: asked for only when something needs a logged-in person (`(await ctx.require("auth")).session()`).
// Like any website, it shows its dialog then, and not before: LOGIN | REGISTER.
// - `check()`: who is logged in, or null, never a dialog (a public page choosing its view).
// - `session({ tab })`: the logged-in person, asking with the dialog if nobody is; null if they close it.
// - `logout()`: ends this app's session and goes home.
// Every change is announced as a `craftworks:auth` event (detail: the person, or null), so a page can switch views.
// - The DID is the ACCOUNT. It is named by the account's RECOVERY WORDS (BIP39, like a wallet's): the owner key is
//   derived from them, and the DID is its seat. Registering makes the words; the Account page shows them.
// - A MEMBER is one device key admitted to the account, kept by the identity delegate on this node and opened here by
//   a PIN. One device can hold several members, of one person or of several.
// - Log in with this device: the PIN. Log in with recovery words: the words name the account; this device becomes a
//   member of it, with a new PIN.
export async function start(ctx) {
  const id = await ctx.require("identity");
  const { core, glue, ask } = await ctx.require("node");
  let current = null;

  const announce = () => dispatchEvent(new CustomEvent("craftworks:auth", { detail: current }));
  const opened = a => {
    current = { member: a.unlocked.member, did: glue.did_of(hexBytes(a.unlocked.did)) };
    ctx.log("logged in", { what: `${current.did.slice(0, 24)}… member ${current.member.slice(0, 12)}…` });
    announce();
    return current;
  };

  async function check() {
    if (current) return current;
    const w = await id.who();
    return w.unlocked ? opened(w) : null;
  }

  async function session({ tab = "login" } = {}) {
    return (await check()) ?? dialog(tab);
  }

  async function logout() {
    await id.lock();
    current = null;
    ctx.log("logged out", {});
    announce();
    location.hash = "#/";
  }

  // The account's seat, put (the same PUT every time for the same words: it makes the account, or finds it).
  async function putSeat(a) {
    const said = await ask(a.seat.frames, s => (s.kind === "put" && s.key === a.seat.id) || s.kind === "refused", "putting the account's seat", 60000);
    if (said.kind !== "put") throw new Error(`the node refused the account's seat (${said.said})`);
  }

  // This device as a member of the account the words name. Kept across a retry with another PIN, so one attempt
  // mints one member key.
  let joining = null;
  async function join(entropy, pin) {
    if (!joining || joining.entropyHex !== hex(entropy)) {
      const a = core.account(await ctx.require("register-wasm"), entropy);
      await putSeat(a);
      ctx.log("account", { what: a.did });
      joining = { entropyHex: hex(entropy), entropy, did: a.didBytes, member: crypto.getRandomValues(new Uint8Array(32)) };
    }
    const r = await id.provision(joining.member, joining.did, pin, joining.entropy);
    if (r.unlocked) {
      joining.member.fill(0);
      joining.entropy.fill(0);
      joining = null;
    }
    return r;
  }

  function dialog(tab) {
    return new Promise(resolve => {
      const box = document.createElement("div");
      box.className = "auth-dialog";
      box.innerHTML = `
        <style>
          .auth-dialog { position: fixed; inset: 0; background: #0008; display: grid; place-items: center; z-index: 20; }
          .auth-dialog .card { background: var(--bg, #fff); color: inherit; padding: 1.2em 1.4em; border-radius: 8px;
            width: min(26em, calc(100vw - 32px)); display: grid; gap: .7em; }
          .auth-dialog .tabs { display: flex; gap: .5em; }
          .auth-dialog .close { margin-left: auto; }
          .auth-dialog .tabs button[aria-selected="true"] { font-weight: bold; text-decoration: underline; }
          .auth-dialog form { display: grid; gap: .5em; }
          .auth-dialog form[hidden] { display: none; }
          .auth-dialog input, .auth-dialog textarea { font: inherit; padding: .4em; }
          .auth-dialog .said { min-height: 1.2em; font-size: .9em; margin: 0; }
          .auth-dialog .note { font-size: .85em; opacity: .8; margin: 0; }
        </style>
        <div class="card">
          <div class="tabs" role="tablist">
            <button type="button" role="tab" data-tab="login" aria-selected="true">Login</button>
            <button type="button" role="tab" data-tab="register" aria-selected="false">Register</button>
            <button type="button" class="close" aria-label="Close">✕</button>
          </div>
          <div data-panel="login">
            <form class="device">
              <strong>Log in with this device</strong>
              <label>PIN <input name="pin" type="password" inputmode="numeric" autocomplete="off" required></label>
              <button>Log in</button>
              <p class="said"></p>
              <button type="button" class="swap">Use recovery words instead</button>
            </form>
            <form class="words" hidden>
              <strong>Log in with recovery words</strong>
              <p class="note">On any device: your words open your account, and this device joins it with its own PIN.</p>
              <label>Your 12 or 24 words <textarea name="words" rows="3" autocomplete="off" spellcheck="false" required></textarea></label>
              <label>A PIN for this device (6 or more) <input name="pin" type="password" minlength="6" autocomplete="off" required></label>
              <button>Log in</button>
              <p class="said"></p>
              <button type="button" class="swap">Back to PIN</button>
            </form>
          </div>
          <div data-panel="register" hidden>
            <form class="register">
              <strong>Register with this device</strong>
              <label>Choose a PIN (6 or more) <input name="pin" type="password" minlength="6" autocomplete="off" required></label>
              <label>The PIN again <input name="again" type="password" minlength="6" autocomplete="off" required></label>
              <button>Register</button>
              <p class="said"></p>
              <p class="note">Your account comes with recovery words. See them any time on your Account page.</p>
            </form>
          </div>
        </div>`;
      document.body.append(box);
      const q = s => box.querySelector(s);
      const say = (f, t) => (f.querySelector(".said").textContent = t);
      for (const t of box.querySelectorAll("[data-tab]")) {
        t.addEventListener("click", () => {
          for (const u of box.querySelectorAll("[data-tab]")) u.setAttribute("aria-selected", String(u === t));
          for (const p of box.querySelectorAll("[data-panel]")) p.hidden = p.dataset.panel !== t.dataset.tab;
          box.querySelector(`[data-panel="${t.dataset.tab}"] input`).focus();
        });
      }
      const done = a => {
        box.remove();
        resolve(opened(a));
      };
      q(".close").addEventListener("click", () => {
        box.remove();
        resolve(null);
      });
      const why = r =>
        r.wrongPin ? `Wrong PIN. ${r.wrongPin.triesLeft} tries left on this device.`
        : r.locked ? "Too many wrong PINs: this device is locked. Log in with your recovery words."
        : r.refused === "PinTaken" ? "That PIN is taken on this device: choose another."
        : r.error ?? `Refused: ${r.refused}`;
      const on = (form, run) =>
        form.addEventListener("submit", async e => {
          e.preventDefault();
          say(form, "Working…");
          const r = await run().catch(err => ({ error: err.message }));
          if (r?.unlocked) return done(r);
          if (r) say(form, why(r));
        });

      // The Login tab's two ways, one at a time.
      for (const b of box.querySelectorAll("button.swap")) {
        b.addEventListener("click", () => {
          const [d, w] = [q("form.device"), q("form.words")];
          d.hidden = !d.hidden;
          w.hidden = !w.hidden;
          (d.hidden ? w : d).querySelector("textarea, input").focus();
        });
      }
      const device = q("form.device");
      on(device, () => id.unlock(device.pin.value));

      const words = q("form.words");
      on(words, async () => {
        let entropy;
        try {
          entropy = glue.CraftworksCore.entropy_of(words.words.value);
        } catch (e) {
          return { error: String(e) };
        }
        return join(entropy, words.pin.value);
      });

      const register = q("form.register");
      let fresh = null;
      on(register, async () => {
        if (register.pin.value !== register.again.value) return { error: "The two PINs differ." };
        // New words, made once per dialog (a retry with another PIN is the same new account): 16 bytes, 12 words.
        fresh ??= crypto.getRandomValues(new Uint8Array(16));
        return join(fresh, register.pin.value);
      });
      box.querySelector(`[data-tab="${tab}"]`).click();
    });
  }

  return { check, session, logout, current: () => current, identity: id, words: e => glue.CraftworksCore.words_of(e) };
}

function hexBytes(h) {
  return new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
}

function hex(b) {
  return [...b].map(x => x.toString(16).padStart(2, "0")).join("");
}
