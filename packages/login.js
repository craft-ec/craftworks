// LOGIN, a component: the dialog — LOGIN | REGISTER — shown when a page needs someone logged in, like any website's,
// and not before. The ways in are the `auth` capability's; this is only their UI.
//
//   const login = await ctx.require("login");
//   const s = await login.session({ tab: "login" | "register" })   // the person; the dialog if nobody is; null if closed
export async function start(ctx) {
  const auth = await ctx.require("auth");
  // Loaded before anyone logs in: after a words login it takes this node into the account's MLS group.
  await ctx.require("keys");
  const { glue } = await ctx.require("node");

  async function session({ tab = "login" } = {}) {
    return (await auth.check()) ?? dialog(tab);
  }

  function dialog(tab) {
    return new Promise(resolve => {
      const box = document.createElement("div");
      box.className = "auth-dialog";
      box.innerHTML = `
        <style>
          .auth-dialog { position: fixed; inset: 0; background: var(--cw-scrim); display: grid; place-items: center; z-index: 20; }
          .auth-dialog .card { background: var(--cw-surface); color: var(--cw-fg); padding: 1.2em 1.4em; border-radius: var(--cw-radius);
            box-shadow: var(--cw-shadow-lg); width: min(26em, calc(100vw - 32px)); display: grid; gap: .7em; }
          .auth-dialog .tabs { display: flex; gap: .5em; }
          .auth-dialog .tabs button { border: 0; background: none; cursor: pointer; padding: var(--cw-space-1) var(--cw-space-2); border-radius: var(--cw-radius-sm); }
          .auth-dialog .tabs button:hover { background: var(--cw-hover); }
          .auth-dialog .close { margin-left: auto; }
          .auth-dialog .tabs button[aria-selected="true"] { font-weight: 600; box-shadow: inset 0 -2px var(--cw-accent); }
          .auth-dialog form { display: grid; gap: .5em; }
                    .auth-dialog input:not([type="checkbox"]), .auth-dialog textarea { padding: .4em; }
          .auth-dialog form > button:not(.swap) { background: var(--cw-accent); color: var(--cw-accent-fg); border: 0; border-radius: var(--cw-radius-sm); padding: var(--cw-space-2); cursor: pointer; }
          .auth-dialog .swap { border: 0; background: none; color: var(--cw-accent); cursor: pointer; padding: 0; justify-self: start; }
          .auth-dialog .said { min-height: 1.2em; font-size: var(--cw-text-sm); margin: 0; }
          .auth-dialog .note { font-size: var(--cw-text-sm); color: var(--cw-muted); margin: 0; }
        </style>
        <div class="card">
          <div class="tabs" role="tablist">
            <button type="button" role="tab" data-tab="login" aria-selected="true">Login</button>
            <button type="button" role="tab" data-tab="register" aria-selected="false">Register</button>
            <button type="button" class="close" aria-label="Close">✕</button>
          </div>
          <div data-panel="login">
            <form class="device">
              <strong>Log in with this node</strong>
              <label>PIN <input name="pin" type="password" inputmode="numeric" autocomplete="off" required></label>
              <button>Log in</button>
              <p class="said"></p>
              <button type="button" class="swap">Use recovery words instead</button>
            </form>
            <form class="words" hidden>
              <strong>Log in with recovery words</strong>
              <p class="note">On any node: your words open your account, and this node joins it with its own PIN.</p>
              <label>Your 12 or 24 words <textarea name="words" rows="3" autocomplete="off" spellcheck="false" required></textarea></label>
              <label>Create a PIN (6 or more) <input name="pin" type="password" minlength="6" autocomplete="off" required></label>
              <label>Confirm the PIN <input name="again" type="password" minlength="6" autocomplete="off" required></label>
              <button>Log in</button>
              <p class="said"></p>
              <button type="button" class="swap">Back to PIN</button>
            </form>
          </div>
          <div data-panel="register" hidden>
            <form class="register">
              <strong>Register with this node</strong>
              <label>Create a PIN (6 or more) <input name="pin" type="password" minlength="6" autocomplete="off" required></label>
              <label>Confirm the PIN <input name="again" type="password" minlength="6" autocomplete="off" required></label>
              <button>Next</button>
              <p class="said"></p>
            </form>
            <form class="words-once" hidden>
              <strong>Your recovery words</strong>
              <p class="note">Write these down and keep them offline. They are your account: with them you log in on any
              node and get your account back. They are shown only now; no node keeps them.</p>
              <ol class="shown" style="columns:3;font-family:monospace"></ol>
              <label><input type="checkbox" name="kept" required> I have written them down</label>
              <button>Create my account</button>
              <p class="said"></p>
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
        resolve(auth.accept(a));
      };
      q(".close").addEventListener("click", () => {
        box.remove();
        resolve(null);
      });
      const why = r =>
        r.wrongPin ? `Wrong PIN. ${r.wrongPin.triesLeft} tries left on this node.`
        : r.locked ? "Too many wrong PINs: this node is locked. Log in with your recovery words."
        : r.refused === "PinTaken" ? "That PIN is taken on this node: choose another."
        : r.error ?? `Refused: ${r.refused}`;
      const on = (form, run) =>
        form.addEventListener("submit", async e => {
          e.preventDefault();
          say(form, "Working…");
          const r = await run().catch(err => ({ error: err?.message ?? String(err) }));
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
      on(device, () => auth.unlock(device.pin.value));

      const words = q("form.words");
      on(words, async () => {
        if (words.pin.value !== words.again.value) return { error: "The two PINs differ." };
        let entropy;
        try {
          entropy = glue.CraftworksCore.entropy_of(words.words.value);
        } catch (e) {
          return { error: String(e) };
        }
        return auth.join(entropy, words.pin.value);
      });

      // REGISTER: the PIN, then the new words shown once, then the account.
      const register = q("form.register");
      const once = q("form.words-once");
      let fresh = null;
      on(register, async () => {
        if (register.pin.value !== register.again.value) return { error: "The two PINs differ." };
        // New words, made once per dialog (a retry with another PIN is the same new account): 16 bytes, 12 words.
        fresh ??= crypto.getRandomValues(new Uint8Array(16));
        const list = once.querySelector(".shown");
        list.replaceChildren(...glue.CraftworksCore.words_of(fresh).split(" ").map(w => Object.assign(document.createElement("li"), { textContent: w })));
        register.hidden = true;
        once.hidden = false;
        say(register, "");
        return null;
      });
      on(once, async () => {
        const r = await auth.join(fresh, register.pin.value, { fresh: true });
        if (r.unlocked) once.querySelector(".shown").replaceChildren();
        if (r.refused === "PinTaken") {
          once.hidden = true;
          register.hidden = false;
          say(register, "That PIN is taken on this node: choose another.");
          return null;
        }
        return r;
      });
      box.querySelector(`[data-tab="${tab}"]`).click();
    });
  }

  return { session };
}
