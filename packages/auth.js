// AUTH, a capability (no UI: the dialog is the `login` component): who is logged in, and the ways in.
// - `check()`: who is logged in, or null. Never a dialog.
// - `unlock(pin)`, `join(entropy, pin, { fresh })`: the two ways in (a PIN on this node; recovery words, which make this
//   node a member); `accept(answer)` turns an answer that opened a member into the session.
// - `logout()`: ends this app's session and goes home.
// - `nodes()`: the account's members as the network holds them. `changeWords(old, new)`: new recovery words.
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

  async function logout() {
    await id.lock();
    current = null;
    ctx.log("logged out", {});
    announce();
    // The app starts again, at home: every capability holds per-session things (grants asked, tables and their keys,
    // the account's group), and none of them may reach the next person to log in on this page.
    location.hash = "#/";
    location.reload();
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
    // Whoever holds something sealed for the old words re-seals it for the new ones, before the change is published
    // (the `keys` capability: every epoch's escrow).
    for (const f of wordsChanged) await f({ old, fresh });
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
  const joined = [];
  const wordsChanged = [];
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
      // Only a REGISTRATION makes an account that has nothing on the network yet; a words login opens one that does.
      if (fresh) made = r.unlocked.did;
      // While the words are still here: whoever needs them to act for the account (the `keys` capability: this node
      // joins the account's MLS group). A failure there is reported, never a failed login.
      for (const f of joined) await f({ entropy: joining.entropy, did: joining.did, fresh }).catch(e => ctx.log("after login", { what: e?.message ?? String(e) }));
      joining.member.fill(0);
      joining.entropy.fill(0);
      joining.data.fill(0);
      joining = null;
    }
    return r;
  }

  // Read the account's key log (so the core holds it): true if the network has it.
  const readKeyLog = async didBytes => get(glue.CraftworksCore.idlog_id((await codes())[1], didBytes), "reading the account's key log");

  return {
    check, accept: opened, unlock, join, logout, nodes, changeWords, readKeyLog, current: () => current, identity: id,
    // `fn({ entropy, did, fresh })`, called after a words login or a registration, before the words are wiped.
    onJoined: f => joined.push(f),
    // `fn({ old, fresh })`, called while the words change, both in hand; an error stops the change.
    onWordsChanged: f => wordsChanged.push(f),
  };
}


function hexBytes(h) {
  return new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
}

function hex(b) {
  return [...b].map(x => x.toString(16).padStart(2, "0")).join("");
}
