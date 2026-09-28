// INDEX, a capability: POINTERS to things, kept apart from the things. Its first kind is the INBOX: an unordered set of
// items (a `bag` contract) at an address every sender computes from the recipient's DID. Anyone may drop an item in —
// nobody writes into another's store: the item is a pointer (a welcome to a conversation, a mail), SEALED to the
// recipient's inbox key (on their card), so only their nodes open it — and it costs a little WORK, so a flood costs a
// lot. (Directories and comment lists — public indexes — come here too.)
//
//   const index = await ctx.require("index");
//   await index.send(did, { kind: "welcome", … })   // sealed to their inbox key, dropped in their inbox
//   await index.inbox()                             // this account's items, opened: [{ … }]
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
  async function inbox() {
    const sp = await space.account();
    const address = Core.inbox_address(sp.idBytes);
    const id = Core.bag_id(bagCode, address);
    const [, frames] = core.frames_get(bytes(id));
    const said = await ask(frames, x => (x.kind === "got" || x.kind === "get-failed") && x.id === id, "reading the inbox", 30000).catch(() => ({ kind: "get-failed" }));
    if (said.kind !== "got") return [];
    const payloads = Array.from(core.bag_payloads(address, id) ?? []);
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

  return { send, inbox, makeInbox };
}
