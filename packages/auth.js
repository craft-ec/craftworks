// AUTH, a service: asked for only when something needs a logged-in person (`(await ctx.require("auth")).session()`).
// Like any website, it shows its dialog then, and not before: LOGIN | REGISTER.
// - `check()`: who is logged in, or null, never a dialog (a public page choosing its view).
// - `session({ tab })`: the logged-in person, asking with the dialog if nobody is; null if they close it.
// - `logout()`: ends this app's session and goes home.
// - `nodes()`: the account's members as the network holds them (the DID is the owner key → its member Set).
// Every change is announced as a `craftworks:auth` event (detail: the person, or null), so a page can switch views.
// - The DID is the ACCOUNT. It is named by the account's RECOVERY WORDS (BIP39, like a wallet's): the owner key is
//   derived from them, and the DID IS that key (never a contract address, which moves with every code release).
//   Registering makes the words and shows them ONCE; no node keeps them
//   (a node that did would lose the account to anyone who copied its disk). The account's data key is derived from
//   them too, and every member node keeps that.
// - A MEMBER is one NODE's key admitted to the account, kept by the identity delegate on that node and opened by a PIN.
//   Every browser on the node shares its members and sessions. One node can hold several members, of one person or of
//   several.
// - Log in with this node: the PIN. Log in with recovery words: the words name the account; this node becomes a
//   member of it, with a new PIN.
export async function start(ctx) {
  const id = await ctx.require("identity");
  const { core, glue, ask } = await ctx.require("node");
  let current = null;
  let made = null; // the DID (hex) of an account this page registered

  const announce = () => dispatchEvent(new CustomEvent("craftworks:auth", { detail: current }));
  const opened = a => {
    current = {
      member: a.unlocked.member,
      did: glue.did_of(hexBytes(a.unlocked.did)),
      didBytes: hexBytes(a.unlocked.did),
      data: a.unlocked.data, // the account's data key (public, hex), or null
      // Made by this page just now: nothing of it is on the network yet, so there is nothing to read, only to create.
      fresh: made === a.unlocked.did,
    };
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

  // Put one of the account's contracts; resolves when the node accepts it.
  async function put(p, what) {
    const said = await ask(p.frames, s => (s.kind === "put" && s.key === p.id) || s.kind === "refused", what, 60000);
    if (said.kind !== "put") throw new Error(`the node refused ${what} (${said.said})`);
  }

  // Read one contract by its 32-byte id; resolves true when its state is in the core, false if the network has none.
  async function get(id, what) {
    const [hexId, frames] = core.frames_get(id);
    const said = await ask(frames, s => (s.kind === "got" || s.kind === "get-failed") && s.id === hexId, what, 30000);
    return said.kind === "got";
  }

  // The account's members, from the network: `[{ key, name, since }]`.
  async function nodes() {
    const s = await check();
    if (!s) return [];
    // The DID is the owner's key, so the member list's address is derived from it: nothing to look up first.
    const setCode = await ctx.require("set-wasm");
    if (!(await get(core.members_id(setCode, s.didBytes), "reading the account's members"))) return [];
    return JSON.parse(core.members(setCode, s.didBytes));
  }

  // This node as a member of the account the words name. Kept across a retry with another PIN, so one attempt
  // mints one member key.
  let joining = null;
  async function join(entropy, pin) {
    if (!joining || joining.entropyHex !== hex(entropy)) {
      const member = crypto.getRandomValues(new Uint8Array(32));
      const a = core.account(await ctx.require("set-wasm"), entropy, id.publicOf(member), Date.now());
      await put(a.members, "this node's place in the account");
      ctx.log("account", { what: `${a.did}, this node admitted` });
      joining = { entropyHex: hex(entropy), entropy, did: a.didBytes, member };
    }
    const data = glue.CraftworksCore.data_seed(joining.entropy);
    const r = await id.provision(joining.member, joining.did, pin, data);
    data.fill(0);
    if (r.unlocked) {
      made = r.unlocked.did;
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
        resolve(opened(a));
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
        if (words.pin.value !== words.again.value) return { error: "The two PINs differ." };
        let entropy;
        try {
          entropy = glue.CraftworksCore.entropy_of(words.words.value);
        } catch (e) {
          return { error: String(e) };
        }
        return join(entropy, words.pin.value);
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
        const r = await join(fresh, register.pin.value);
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

  return { check, session, logout, nodes, current: () => current, identity: id };
}


function hexBytes(h) {
  return new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
}

function hex(b) {
  return [...b].map(x => x.toString(16).padStart(2, "0")).join("");
}
