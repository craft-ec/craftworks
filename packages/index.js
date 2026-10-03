// INDEX, a capability: POINTERS to things, kept apart from the things. Its first kind is the INBOX: an unordered set of
// items (a `bag` contract) at an address every sender computes from the recipient's DID. Anyone may drop an item in —
// nobody writes into another's store: the item is a pointer (a welcome to a conversation, a mail), SEALED to the
// recipient's inbox key (on their card), so only their nodes open it — and it costs a little WORK, so a flood costs a
// lot. (Directories and comment lists — public indexes — come here too.)
//
//   const index = await ctx.require("index");
//   await index.send(did, { kind: "welcome", … })   // sealed to their inbox key, dropped in their inbox
//   await index.inbox()                             // this account's items, opened: [{ … }]
//   await index.request(code, { … })                // an item (plain) in the bag an INVITE CODE names
//   await index.requests(code)                      // the items in it: only who holds the code finds the bag
//   await index.openPointers(ref)                    // a THING's public bag, made (by its author, when it is made)
//   await index.point(ref, { from })                 // a pointer (plain) to where something about `ref` is: its bag
//   await index.pointers(ref)                        // the pointers in it (claims: the reader resolves each)
//   await index.listSpace(desc)   await index.spaces()   // DISCOVER's list of public spaces: a space's description
//                                                    // ({ id, name, governance }) in one public bag; each a claim
//                                                    // (the id proves its owner: `space.owner`)
export async function start(ctx) {
  const auth = await ctx.require("auth");
  const space = await ctx.require("space");
  const { core, glue, ask, WAIT } = await ctx.require("node");
  const bagCode = await ctx.require("bag-wasm");
  const Core = glue.CraftworksCore;
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  const didBytes = did => (typeof did === "string" ? glue.did_bytes(did) : did);

  // Drop an item (bytes) into the bag at `address`: a PUT the hosts merge.
  async function drop(address, payload, what) {
    const [idHex, frames] = core.bag_add(bagCode, address, payload);
    const name = Core.id_name(bytes(idHex));
    const said = await ask(frames, x => (x.kind === "put" && x.key === name) || x.kind === "refused", what, 60000);
    if (said.kind === "refused") throw new Error(`${what}: the node said ${said.said}`);
  }

  // This account's own inbox, made (empty) — before anyone is told where it is.
  async function makeInbox() {
    const sp = await space.account();
    await drop(Core.inbox_address(sp.idBytes), new Uint8Array(0), "making the inbox");
  }

  // SEND: `item` (JSON) sealed to the recipient's inbox key (from their card), dropped in their inbox.
  async function send(did, item) {
    const card = await (await ctx.require("directory")).card(did);
    if (!card?.inbox) throw new Error("that person has no card with an inbox yet");
    const sealed = Core.seal_to(bytes(card.inbox), enc.encode(JSON.stringify(item)), crypto.getRandomValues(new Uint8Array(32)));
    await drop(Core.inbox_address(didBytes(did)), sealed, "sending to an inbox");
  }

  // THE ONE BAG READ: a bag's items as stored (bytes). It waits for the node's ANSWER; a bag the node answers is not there
  // (nobody made it yet: a month nobody posted in, a space from before its bag) is MADE, empty, by this first reader —
  // so it is searched for once, ever, and every reader after finds it. Silence makes nothing (a guess is never kept).
  const madeBags = new Set();
  async function read(address, what) {
    const id = Core.bag_id(bagCode, address);
    const [, frames] = core.frames_get(bytes(id));
    const said = await ask(frames, x => (x.kind === "got" || x.kind === "get-failed") && x.id === id, what, WAIT.answer).catch(() => ({ kind: "silent" }));
    if (said.kind === "got") return Array.from(core.bag_payloads(address, id) ?? []);
    if (said.kind === "get-failed" && !madeBags.has(id)) madeBags.add(id), drop(address, new Uint8Array(0), `making ${what}`).catch(() => madeBags.delete(id));
    return [];
  }

  async function inbox() {
    const sp = await space.account();
    const payloads = await read(Core.inbox_address(sp.idBytes), "the inbox");
    if (!payloads.length) return [];
    const r = await auth.identity.inboxOpen(payloads);
    return (r.opened ?? [])
      .filter(Boolean)
      .map(h => {
        try {
          return JSON.parse(dec.decode(bytes(h)));
        } catch {
          return null;
        }
      })
      .filter(Boolean);
  }

  // AN INVITE CODE's bag: at an address only the code gives (the inbox's derivation over the code's hash — no DID is
  // 32 bytes of a hash of text). Its items are plain: whoever holds the code reads them (they name who asks, nothing
  // secret).
  const codeAddress = async code => Core.invite_address(String(code));
  const openRequests = async code => drop(await codeAddress(code), new Uint8Array(0), "making an invite's bag");
  const request = async (code, item) => drop(await codeAddress(code), enc.encode(JSON.stringify(item)), "asking to join");
  const parse = payloads =>
    payloads
      .map(p => {
        try {
          return JSON.parse(dec.decode(p));
        } catch {
          return null;
        }
      })
      .filter(Boolean);
  const requests = async code => parse(await read(await codeAddress(code), "an invite's requests"));

  // A THING's POINTERS (comments and votes on a post: `posts`): a public bag at an address its ref gives, so anyone who
  // knows the thing finds it. A pointer is plain and unsigned: it says where to look, the reader checks what is there.
  const refAddress = async ref => Core.inbox_address(new Uint8Array(await crypto.subtle.digest("SHA-256", enc.encode(`craftworks pointers ${ref}`))));
  const openPointers = async ref => drop(await refAddress(ref), new Uint8Array(0), "making a pointer bag");
  const point = async (ref, item) => drop(await refAddress(ref), enc.encode(JSON.stringify(item)), "pointing");
  const pointers = async ref => parse(await read(await refAddress(ref), `the bag of ${ref}`));

  // A SPACE's SEALED bag (`name`): at an address the space's id gives, each item sealed with the space's newest EPOCH
  // key (`epoch ‖ nonce ‖ AES-GCM`) — only members read it; a member removed reads nothing added after. Its size shows
  // to whoever knows the space's id, never what is in it.
  const sealedAddress = async (sp, name) => refAddress(`space ${glue.space_table_key(sp.idBytes, `bag-${name}`)}`);
  const aes = async keyHex => crypto.subtle.importKey("raw", bytes(keyHex), "AES-GCM", false, ["encrypt", "decrypt"]);
  async function spacePoint(sp, name, item) {
    const access = await ctx.require("access");
    const k = await access.keyAt(`bag-${name}`, -1, { space: sp.idBytes });
    if (!k.key) throw new Error(`no key of the space here (${k.why})`);
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const ct = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv }, await aes(k.key), enc.encode(JSON.stringify(item))));
    const out = new Uint8Array(4 + 12 + ct.length);
    new DataView(out.buffer).setUint32(0, k.epoch);
    out.set(iv, 4);
    out.set(ct, 16);
    await drop(await sealedAddress(sp, name), out, `the space's ${name}`);
  }
  async function spacePointers(sp, name) {
    const access = await ctx.require("access");
    const keys = new Map();
    const out = [];
    for (const p of await read(await sealedAddress(sp, name), `the space's ${name}`)) {
      if (p.length < 17) continue;
      const epoch = new DataView(p.buffer, p.byteOffset).getUint32(0);
      if (!keys.has(epoch)) keys.set(epoch, await access.keyAt(`bag-${name}`, epoch, { space: sp.idBytes }).then(k => (k.key ? aes(k.key) : null), () => null));
      const key = await keys.get(epoch);
      if (!key) continue;
      try {
        out.push(JSON.parse(dec.decode(await crypto.subtle.decrypt({ name: "AES-GCM", iv: p.subarray(4, 16) }, key, p.subarray(16)))));
      } catch {}
    }
    return out;
  }

  // DISCOVER's ITEMS (ARCHITECTURE §1: the public index — bags that public posts list themselves in): a public bag per
  // DOMAIN (text, video, audio) and MONTH (UTC `YYYY-MM`), each pointer naming exactly where its item is (`ref`, `kind`,
  // `at`, and its writer and table — `w`, `t`, `sp` — on a space's public board, or `did` on a profile). A reader reads
  // only the months it shows and only the tails named; each pointer is a claim it checks against what is there.
  const monthOf = at => new Date(at).toISOString().slice(0, 7);
  const discoverBag = (domain, month) => `discover:${domain}:${month}`;
  const discoverPoint = (domain, at, ptr) => point(discoverBag(domain, monthOf(at)), ptr);
  const discoverPointers = async (domain, months) => (await Promise.all(months.map(m => pointers(discoverBag(domain, m))))).flat();
  // The months a window covers (newest first): `since` ms to now.
  const monthsSince = since => {
    const out = [];
    const d = new Date();
    d.setUTCDate(1);
    for (let i = 0; i < 24; i++) {
      out.push(d.toISOString().slice(0, 7));
      if (d.getTime() <= since) break;
      d.setUTCMonth(d.getUTCMonth() - 1);
    }
    return out;
  };

  const SPACES = "discover:spaces";
  const listSpace = desc => point(SPACES, { id: desc.id, name: desc.name, kind: desc.kind, governance: { owner: desc.governance.owner, nonce: desc.governance.nonce } });
  const spaces = () => pointers(SPACES);

  return { send, inbox, makeInbox, request, requests, openRequests, openPointers, point, pointers, listSpace, spaces, spacePoint, spacePointers, discoverPoint, discoverPointers, monthOf, monthsSince };
}
