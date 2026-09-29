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
export async function start(ctx) {
  const auth = await ctx.require("auth");
  const space = await ctx.require("space");
  const { core, glue, ask } = await ctx.require("node");
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

  // THIS account's inbox: every item, opened by the identity (only the account's nodes hold the key).
  // A bag's items as stored (bytes), or none (a bag nobody made yet).
  async function payloadsAt(address, what) {
    const id = Core.bag_id(bagCode, address);
    const [, frames] = core.frames_get(bytes(id));
    const said = await ask(frames, x => (x.kind === "got" || x.kind === "get-failed") && x.id === id, what, 30000).catch(() => ({ kind: "get-failed" }));
    if (said.kind !== "got") return [];
    return Array.from(core.bag_payloads(address, id) ?? []);
  }

  async function inbox() {
    const sp = await space.account();
    const payloads = await payloadsAt(Core.inbox_address(sp.idBytes), "reading the inbox");
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
  const codeAddress = async code =>
    Core.inbox_address(new Uint8Array(await crypto.subtle.digest("SHA-256", enc.encode(`craftworks invite ${String(code).trim().toLowerCase()}`))));
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
  const requests = async code => parse(await payloadsAt(await codeAddress(code), "reading an invite's requests"));

  // A THING's POINTERS (comments and votes on a post: `posts`): a public bag at an address its ref gives, so anyone who
  // knows the thing finds it. A pointer is plain and unsigned: it says where to look, the reader checks what is there.
  const refAddress = async ref => Core.inbox_address(new Uint8Array(await crypto.subtle.digest("SHA-256", enc.encode(`craftworks pointers ${ref}`))));
  const openPointers = async ref => drop(await refAddress(ref), new Uint8Array(0), "making a pointer bag");
  const point = async (ref, item) => drop(await refAddress(ref), enc.encode(JSON.stringify(item)), "pointing");
  const pointers = async ref => parse(await payloadsAt(await refAddress(ref), "reading pointers"));

  return { send, inbox, makeInbox, request, requests, openRequests, openPointers, point, pointers };
}
