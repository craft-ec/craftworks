// RECOVERY, a capability: opening the account on a NEW device without typing the recovery words. The words are sealed
// under a PASSPHRASE (Argon2id-stretched: `account::passphrase_seal`) and kept on the account's CARD (row `recovery`: only
// the account writes its card; anyone may fetch the sealed copy, so the passphrase must be long).
// A new device gives the account's id and the passphrase, gets the words' entropy back, and logs in as with the words.
// When the shell brokers passkeys (freenet-core #5764), a passkey's secret seals a second copy the same way.
// New recovery words make the copy useless (it holds the old ones): it is dropped then.
//
//   const recovery = await ctx.require("recovery");
//   await recovery.set(words, passphrase)   // this account's words (checked to be its own), sealed
//   await recovery.has()                    // this account has a passphrase copy
//   await recovery.clear()
//   await recovery.open(id, passphrase)     // the words' entropy (then `auth.join(entropy, pin)`)
export async function start(ctx) {
  const [auth, space, storage, directory] = await Promise.all(["auth", "space", "storage", "directory"].map(n => ctx.require(n)));
  const { glue } = await ctx.require("node");
  const Core = glue.CraftworksCore;
  const TAIL = "card";
  const ROW = "recovery";
  const MIN = 12;
  const hexOf = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  const bytes = h => new Uint8Array(h.match(/../g).map(x => parseInt(x, 16)));

  const mine = async () => {
    const me = await space.account();
    if (!me?.shared) throw new Error("log in with your recovery words once on this device first");
    return { me, t: await storage.publicTail(TAIL, me.shared) };
  };

  async function set(words, passphrase) {
    if (String(passphrase ?? "").length < MIN) throw new Error(`a passphrase of at least ${MIN} characters (anyone can try guesses offline)`);
    if (new Set(passphrase).size < 5) throw new Error("that passphrase is too easy to guess");
    const { me, t } = await mine();
    const entropy = Core.entropy_of(words);
    const a = await auth.identity.accountOf(entropy);
    if (a.did !== me.id) throw new Error("those are not this account's recovery words");
    const sealed = Core.recovery_seal(passphrase, me.idBytes, entropy, crypto.getRandomValues(new Uint8Array(16)), crypto.getRandomValues(new Uint8Array(24)));
    await t.put(ROW, hexOf(sealed));
    ctx.log("recovery", { what: "a passphrase copy of the words kept" });
  }

  const has = async () => {
    const { t } = await mine();
    return t.rows().some(r => r.key === ROW && r.value);
  };
  async function clear() {
    const { t } = await mine();
    if (t.rows().some(r => r.key === ROW)) await t.remove(ROW);
  }

  // A NEW DEVICE: the account by its full id (nothing else is known here), its recovery copy opened.
  async function open(id, passphrase) {
    const did = String(id ?? "").trim();
    if (!did.startsWith("did:craftec:")) throw new Error("your account's full id: did:craftec:… (on your Card page)");
    const t = await directory.publicOf(did, TAIL);
    await t?.reread?.().catch(() => {});
    const sealed = t?.rows().find(r => r.key === ROW)?.value;
    if (!sealed) throw new Error("this account has no recovery passphrase set");
    return Core.recovery_open(passphrase, glue.did_bytes(did), bytes(sealed));
  }

  // New words: the copy holds the old ones — dropped.
  auth.identity.onWordsChanged(() => clear().catch(() => {}));

  return { set, has, clear, open };
}
