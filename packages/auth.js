// AUTH, a capability (no UI: the dialog is the `login` component): who is logged in, and the ways in. What an account
// IS (its key log, which account words hold, changing them) is the `identity` capability's; who its nodes are is the
// `membership` capability's.
// - `check()`: who is logged in, or null. Never a dialog.
// - `unlock(pin)`, `join(entropy, pin, { fresh })`: the two ways in (a PIN on this node; recovery words, which make this
//   node a member); `accept(answer)` turns an answer that opened a member into the session.
// - `logout()`: ends this app's session and starts the app again at home.
// Every change is announced as a `craftworks:auth` event (detail: the person, or null), so a page can switch views.
// - A MEMBER is one NODE's key in the account, kept by the identity delegate on that node and opened by a PIN. Every
//   browser on the node shares its members and sessions. One node can hold several members, of one person or of
//   several. Registering makes the recovery words and shows them ONCE; no node keeps them.
export async function start(ctx) {
  const id = await ctx.require("identity");
  const { glue } = await ctx.require("node");
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
      // Its keys too (the account's group, the epochs it could read), before anything reads a table.
      if (p.unlocked) {
        const k = await id.moveKeysFrom(prior, pin).catch(e => ({ moved: 0, said: e.message }));
        ctx.log("member moved", { what: `its keys: ${k.group ? "the account's group and " : ""}${k.moved} epoch(s), ${k.spaces ?? 0} space(s)${k.said ? ` (${k.said})` : ""}` });
      }
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
  async function join(entropy, pin, { fresh = false } = {}) {
    if (!joining || joining.entropyHex !== hex(entropy)) {
      const member = crypto.getRandomValues(new Uint8Array(32));
      const a = await id.accountOf(entropy, { fresh });
      joining = { entropyHex: hex(entropy), entropy, did: a.didBytes, data: a.data, member };
    }
    const r = await id.provision(joining.member, joining.did, pin, joining.data);
    if (r.unlocked) {
      // Only a REGISTRATION makes an account that has nothing on the network yet; a words login opens one that does.
      if (fresh) made = r.unlocked.did;
      // While the words are still here: whoever needs them to act for the account (the `keys` capability: this node
      // joins the account's MLS group). A failure there is reported, never a failed login.
      const node = id.publicOf(joining.member);
      for (const f of joined) await f({ entropy: joining.entropy, did: joining.did, node, fresh }).catch(e => ctx.log("after login", { what: e?.message ?? String(e) }));
      joining.member.fill(0);
      joining.entropy.fill(0);
      joining.data.fill(0);
      joining = null;
    }
    return r;
  }

  return {
    check, accept: opened, unlock, join, logout, current: () => current, identity: id,
    // `fn({ entropy, did, node, fresh })`, called after a words login or a registration, before the words are wiped.
    onJoined: f => joined.push(f),
  };
}


function hexBytes(h) {
  return new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
}

function hex(b) {
  return [...b].map(x => x.toString(16).padStart(2, "0")).join("");
}
