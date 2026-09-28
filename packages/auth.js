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

  const codes = () => Promise.all(["set-wasm", "idlog-wasm", "register-wasm"].map(n => ctx.require(n)));

  // The account's members, from the network: `[{ key, name, since }]`. The DID names the account's key log; the log
  // names the CURRENT owner key; the member list is that key's. `null`: the account has no key log (a member made
  // before the log existed: logging in once with the recovery words brings it over).
  async function nodes() {
    const s = await check();
    if (!s) return [];
    const [setCode, idlogCode] = await codes();
    if (!(await get(glue.CraftworksCore.idlog_id(idlogCode, s.didBytes), "reading the account's key log"))) return null;
    const owner = core.idlog_owner(idlogCode, s.didBytes);
    if (!(await get(core.members_id(setCode, owner), "reading the account's members"))) return [];
    return JSON.parse(core.members(setCode, owner));
  }

  // CHANGE THE RECOVERY WORDS: `old` (the current words, typed again) hands the account to `fresh` (new words, shown
  // once). The log gets its two rotations, the new words their whoami, and every node of the account its place under
  // the new owner key. Afterwards the old words open nothing.
  async function changeWords(old, fresh) {
    const s = await check();
    if (!s) throw new Error("nobody is logged in");
    const [setCode, idlogCode, registerCode] = await codes();
    if (!(await get(glue.CraftworksCore.idlog_id(idlogCode, s.didBytes), "reading the account's key log")))
      throw new Error("this account has no key log yet: log in once with its recovery words first");
    const list = (await nodes()) ?? [];
    const keys = new Uint8Array(list.length * 32);
    list.forEach((m, i) => keys.set(hexBytes(m.key), i * 32));
    const c = core.change_words(idlogCode, registerCode, setCode, s.didBytes, old, fresh, keys, Date.now());
    await put(c.log, "the account's key log");
    await put(c.whoami, "the new words' account");
    for (const m of c.members) await put(m, "a node's place in the account");
    ctx.log("recovery words changed", { what: `${s.did.slice(0, 24)}…: ${list.length} node(s) moved to the new owner key` });
  }

  // Log in with the PIN. A member made by an EARLIER build of the identity delegate is not in this one (a delegate's
  // secrets stay with its build): on a PIN this build does not know, the earlier builds are asked, newest first, and
  // the member the PIN opens there is moved here, so an update never costs anyone their login.
  async function unlock(pin) {
    const r = await id.unlock(pin);
    if (!r.wrongPin) return r;
    for (const prior of ctx.identityPrior) {
      let h;
      try {
        h = await id.handoverFrom(prior, pin);
      } catch (e) {
        ctx.log("earlier build", { what: `${prior.slice(0, 12)}…: ${e.message}` });
        continue;
      }
      if (!h.handed) continue;
      const bytes = x => (x ? hexBytes(x) : new Uint8Array(0));
      const p = await id.provision(bytes(h.handed.seed), bytes(h.handed.did), pin, bytes(h.handed.data));
      ctx.log("member moved", { what: `from the earlier build ${prior.slice(0, 12)}…: ${p.unlocked ? "logged in" : JSON.stringify(p)}` });
      return p;
    }
    return r;
  }

  // This node as a member of the account the words hold. Kept across a retry with another PIN, so one attempt
  // mints one member key.
  //
  // WHICH account: new words (`fresh`, just made) hold their own inception's. Typed words hold either their own
  // inception's (an account's original words) or the one their whoami names (words rotated in): both are asked AT
  // ONCE and the first that answers wins, so the one that does not exist never holds the login up. Neither: an
  // account from before the key log, whose inception is put now (its tables and nodes stay where they were).
  let joining = null;
  async function join(entropy, pin, { fresh = false } = {}) {
    if (!joining || joining.entropyHex !== hex(entropy)) {
      const [setCode, idlogCode, registerCode] = await codes();
      const member = crypto.getRandomValues(new Uint8Array(32));
      const plan = core.words_plan(registerCode, entropy);
      const logOf = d => glue.CraftworksCore.idlog_id(idlogCode, d);
      let did = plan.inceptionDid;
      if (!fresh) {
        const own = get(logOf(plan.inceptionDid), "reading the account's key log").then(ok => (ok ? plan.inceptionDid : Promise.reject()));
        const named = get(plan.whoamiId, "asking which account these words hold").then(async ok => {
          const d = ok && core.whoami_did(registerCode, entropy);
          if (!d || !(await get(logOf(d), "reading the account's key log"))) throw new Error("none");
          return d;
        });
        did = await Promise.any([own, named]).catch(() => plan.inceptionDid);
      }
      const a = core.join_account(idlogCode, setCode, entropy, did, id.publicOf(member), Date.now());
      await put(a.log, "the account's key log");
      await put(a.members, "this node's place in the account");
      ctx.log("account", { what: `${a.did}, this node admitted` });
      joining = { entropyHex: hex(entropy), entropy, did: a.didBytes, data: a.data, member };
    }
    const r = await id.provision(joining.member, joining.did, pin, joining.data);
    if (r.unlocked) {
      made = r.unlocked.did;
      joining.member.fill(0);
      joining.entropy.fill(0);
      joining.data.fill(0);
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
      on(device, () => unlock(device.pin.value));

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
        const r = await join(fresh, register.pin.value, { fresh: true });
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

  return { check, session, logout, nodes, changeWords, current: () => current, identity: id };
}


function hexBytes(h) {
  return new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
}

function hex(b) {
  return [...b].map(x => x.toString(16).padStart(2, "0")).join("");
}
