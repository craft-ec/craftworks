// INDEX, a capability: POINTERS to things, kept apart from the things. Its first kind is the INBOX: an unordered set of
// items (a `bag` contract) at an address every sender computes from the recipient's DID. Anyone may drop an item in —
// nobody writes into another's store: the item is a pointer (a welcome to a conversation, a mail), SEALED to the
// recipient's inbox key (on their card), so only their nodes open it — and it costs a little WORK, so a flood costs a
// lot. (Directories and comment lists — public indexes — come here too.)
//
//   const index = await ctx.require("index");
//   await index.send(did, { kind: "welcome", … })   // sealed to their inbox key, dropped in their inbox
//   await index.inbox()                             // this account's items, opened: [{ … }]
//   await index.sendTo(address, boxPublic, item)    // ANY inbox (a space's: `Core.inbox_address(space id)`), sealed to a
//                                                   // key given (the space's mail key)
//   await index.inboxWith(address, seed)            // ANY inbox, opened with a key this page holds (a space's mail key)
//   await index.request(code, { … })                // an item (plain) in the bag an INVITE CODE names
//   await index.requests(code)                      // the items in it: only who holds the code finds the bag
//   await index.openPointers(ref)                    // a THING's public bag, made (by its author, when it is made)
//   await index.point(ref, { from })                 // a pointer (plain) to where something about `ref` is: its bag
//   await index.pointers(ref)                        // the pointers in it (claims: the reader resolves each)
//   await index.bag.chained(addressOf, epoch, item, what) // an item into an epoch's CHAINED list (`wallet::chained`)
//   await index.bag.chain(addressOf, epoch, what)         // every item of an epoch's chained list
//   await index.watch(ref, fn)                       // fn() whenever its bag changes (the node pushes it)
//   await index.listSpace(desc)   await index.spaces()   // DISCOVER's list of public spaces: a space's description
//                                                    // ({ id, name, governance }) in one public bag; each a claim
//                                                    // (the id proves its owner: `space.owner`)
export async function start(ctx) {
  const auth = await ctx.require("auth");
  const space = await ctx.require("space");
  const { core, glue, ask, WAIT, within, listen, contract, onContract, changed: wrote } = await ctx.require("node");
  const bagCode = await ctx.require("bag-wasm");
  const Core = glue.CraftworksCore;
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  const bytes = (await ctx.require("node")).bytes;
  const didBytes = (await ctx.require("node")).didBytes;

  // Drop an item (bytes) into the bag at `address`: a PUT the hosts merge.
  async function drop(address, payload, what) {
    const [idHex, frames] = core.bag_add(bagCode, address, payload);
    const name = Core.id_name(bytes(idHex));
    const said = await ask(frames, x => (x.kind === "put" && x.key === name) || x.kind === "refused", what, 60000);
    if (said.kind === "refused") throw new Error(`${what}: the node said ${said.said}`);
    wrote(idHex); // the door's copy is the old one: read again on the next ask
  }

  // This account's own inbox, made (empty) — before anyone is told where it is.
  async function makeInbox() {
    const sp = await space.account.strict();
    await drop(Core.inbox_address(sp.idBytes), new Uint8Array(0), "making the inbox");
  }

  // SEND: `item` (JSON) sealed to the recipient's inbox key (from their card), dropped in their inbox.
  async function send(did, item) {
    const card = await (await ctx.require("directory")).card(did);
    if (!card?.inbox) throw new Error("that person has no card with an inbox yet");
    const sealed = Core.seal_to(bytes(card.inbox), enc.encode(JSON.stringify(item)), crypto.getRandomValues(new Uint8Array(32)));
    await drop(Core.inbox_address(didBytes(did)), sealed, "sending to an inbox");
  }

  // ANY INBOX: an item sealed to a key given (a space's mail key, from its owner's record); and one read, opened with a
  // key held here. A person's own inbox is `send` / `inbox` (their key is their identity's).
  async function sendTo(address, boxPublic, item) {
    const sealed = Core.seal_to(boxPublic, enc.encode(JSON.stringify(item)), crypto.getRandomValues(new Uint8Array(32)));
    await drop(address, sealed, "sending to an inbox");
  }
  async function inboxWith(address, seed, { show = false } = {}) {
    const payloads = await read(address, "an inbox", { show });
    return payloads
      .map(p => {
        try {
          return JSON.parse(dec.decode(Core.open_with(seed, p)));
        } catch {
          return null;
        }
      })
      .filter(Boolean);
  }

  // THE ONE BAG READ: a bag's items as stored (bytes). It waits for the node's ANSWER; a bag the node answers is not there
  // (nobody made it yet: a month nobody posted in, a space from before its bag) is MADE, empty, by this first reader —
  // so it is searched for once, ever, and every reader after finds it. Silence makes nothing (a guess is never kept).
  // `show`: THE ONE RULE OF EVERY READ (a table's too, `storage`): what is held is returned after at most WAIT.hint,
  // and `onChange` fires when the node's answer brings more — for what is only SHOWN (a list on a page). Without it
  // (what DECIDES: whose catalogs to read, a pointer dropped once): the answer, waited for.
  const madeBags = new Set();
  const changed = [];
  const held = (address, id) => {
    try {
      return Array.from(core.bag_payloads(address, id) ?? []);
    } catch {
      return [];
    }
  };
  // `follow`: the GET subscribes this node to the bag — its copy kept current by the network (a node answers a GET from
  // its own copy when it holds one, and an unsubscribed copy goes stale: a space's writers list read from it missed
  // what members announced since).
  // `strict`: NO ANSWER throws ("no answer: …") instead of reading as empty — for what must not be decided over a bag
  // that did not answer (a statement, an assignment, a scan cursor): silence is not "nothing there".
  // Through the DOOR (`node.contract`, FOUNDATION §2 R6): read once and followed — every bag, so its copy is never
  // stale; a held bag answers from the copy with no GET (`follow` kept for its callers: every read follows now).
  async function read(address, what, { show = false, follow = false, strict = false } = {}) {
    const id = Core.bag_id(bagCode, address);
    const full = (async () => {
      const said = await ctx.optional("index: ask", contract(id, what, { frames: () => core.frames_follow(bytes(id))[1], match: x => (x.kind === "got" || x.kind === "get-failed") && x.id === id, ms: WAIT.answer }), () => ({ kind: "silent" }));
      if (said.held || said.kind === "got") return held(address, id);
      if (said.kind === "get-failed" && !madeBags.has(id)) madeBags.add(id), ctx.optional("index: drop", drop(address, new Uint8Array(0), `making ${what}`), () => madeBags.delete(id));
      if (said.kind === "silent" && strict) throw new Error(`no answer: ${what}`);
      return [];
    })();
    if (!show) return full;
    const early = await within(full.then(x => ({ x })), WAIT.hint, null);
    if (early) return early.x;
    ctx.background("index: full.then", full.then(x => x.length && changed.forEach(f => f())));
    return held(address, id);
  }

  async function inbox({ show = false } = {}) {
    const sp = await space.account.strict();
    const payloads = await read(Core.inbox_address(sp.idBytes), "the inbox", { show });
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
  const pointers = async (ref, { show = false } = {}) => parse(await read(await refAddress(ref), `the bag of ${ref}`, { show }));
  // A THING's BAG WATCHED: followed (this node subscribed: its copy kept current), `fn` run each time the node says it
  // changed (its `changed`: what a followed contract that is not a tail pushes) — never read again on a timer.
  const watchers = new Map(); // the bag's address as the node names it → [fn]
  listen(said => said.kind === "changed" && watchers.get(said.key)?.forEach(f => f()));
  async function watch(ref, fn) {
    const address = await refAddress(ref);
    const key = (await ctx.require("blocks")).address(Core.bag_id(bagCode, address));
    if (!watchers.has(key)) watchers.set(key, []);
    watchers.get(key).push(fn);
    await ctx.background("index: read", read(address, `the bag of ${ref}`, { follow: true }));
  }

  // A SPACE's SEALED bag (`name`): at an address the space's id gives, each item sealed with the space's newest EPOCH
  // key (`epoch ‖ nonce ‖ AES-GCM`) — only members read it; a member removed reads nothing added after. Its size shows
  // to whoever knows the space's id, never what is in it.
  const sealedAddress = async (sp, name) => refAddress(`space ${glue.space_table_key(sp.idBytes, `bag-${name}`)}`);
  // A space's sealed bag FOLLOWED: `fn` on each change the node pushes (the door's copy current already).
  const watchSpace = async (sp, name, fn) => onContract(Core.bag_id(bagCode, await sealedAddress(sp, name)), fn);
  // Sealed with the space's newest epoch — or, for its WRITERS list, with the SPACE's OWN key (from its id: `OPEN`):
  // every member reads it whatever epochs they hold — behind, ahead, on another branch of a fork, or missing the
  // early epochs (10-07: sealed with epoch 0, a member whose history stopped above it read 0 writers, so no acts and
  // an empty space; one without epoch 0 sealed with its newest and listed itself again on every load). It names
  // devices and branches only. Entries sealed with an epoch (before) are still read by whoever holds that epoch.
  // OPEN BAGS (the space's own key): its writers list, and each epoch's DOOR (`door-<epoch>`: the group info, in parts,
  // and the entries of a device entering by itself — FOUNDATION "Members and keys" 4).
  // (The format, the open bags and their key: `identity::bag_*`, the node's too.)
  const OPEN = glue.bag_open_epoch();
  const isOpen = name => glue.bag_is_open(name);
  async function spacePoint(sp, name, item, { epoch = -1 } = {}) {
    const access = await ctx.require("access");
    const k = isOpen(name) ? { epoch: OPEN, key: glue.bag_open_key(sp.idBytes) } : await access.keyAt(`bag-${name}`, epoch, { space: sp.idBytes });
    if (!k.key) throw new Error(`no key of the space here (${k.why})`);
    const out = glue.bag_seal(k.key, k.epoch, enc.encode(JSON.stringify(item)), crypto.getRandomValues(new Uint8Array(12)));
    await drop(await sealedAddress(sp, name), out, `the space's ${name}`);
  }
  async function spacePointers(sp, name) {
    const access = await ctx.require("access");
    const keys = new Map();
    const out = [];
    // What could not be opened, by the epoch it was sealed with (said: a bag read empty because no key opens it looks
    // the same as an empty bag).
    const unread = new Map();
    let raw = 0;
    for (const p of await read(await sealedAddress(sp, name), `the space's ${name}`, { follow: isOpen(name) })) {
      const epoch = glue.bag_item_epoch(p);
      if (epoch === undefined) continue;
      raw += 1;
      // (An open bag's `OPEN` items open with the space's own key; anything else, with that epoch's.)
      const key = await ctx.cache(keys, epoch, () => (epoch === OPEN && isOpen(name) ? Promise.resolve(glue.bag_open_key(sp.idBytes)) : access.keyAt(`bag-${name}`, epoch, { space: sp.idBytes }).then(k => k.key || null)), null);
      const plain = key ? glue.bag_open(key, p) : undefined;
      try {
        if (!plain) throw new Error("no key opens it");
        // `sealedAt`: the epoch it was sealed with (an entry only this branch reads, or one every member does: `OPEN`).
        const v = JSON.parse(dec.decode(plain));
        out.push(v && typeof v === "object" ? { ...v, sealedAt: epoch } : v);
      } catch {
        unread.set(epoch, (unread.get(epoch) ?? 0) + 1);
      }
    }
    if (unread.size) ctx.log("index", { what: `${sp.name ?? sp.id.slice(0, 8)}: ${name} bag — ${raw} item(s), ${out.length} read; no key opens ${[...unread].map(([e, n]) => `${n} sealed at epoch ${e}`).join(", ")}` });
    // How many no key here opened: what it lists is not known to be all.
    out.unread = [...unread.values()].reduce((a, n) => a + n, 0);
    return out;
  }

  // DISCOVER's ITEMS (ARCHITECTURE §1: the public index — bags that public posts list themselves in): a public bag per
  // DOMAIN (text, video, audio) and MONTH (UTC `YYYY-MM`), each pointer naming exactly where its item is (`ref`, `kind`,
  // `at`, and its writer and table — `w`, `t`, `sp` — on a space's public board, or `did` on a profile). A reader reads
  // only the months it shows and only the tails named; each pointer is a claim it checks against what is there.
  // A MONTH's key (UTC `YYYY-MM`) — the one: the Discover bags', usage's, keepers', statements' (`at`: now) — and the
  // month `n` before one.
  const monthOf = (at = Date.now()) => new Date(at).toISOString().slice(0, 7);
  const monthBefore = (m = monthOf(), n = 1) => {
    const [y, mo] = m.split("-").map(Number);
    return new Date(Date.UTC(y, mo - 1 - n, 1)).toISOString().slice(0, 7);
  };
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
  // HEALTH (10-09, owner: "auto report and you keep monitor"): a problem a page meets, written into the day's health
  // bag (`health YYYY-MM-DD`, UTC) — never content: its kind, a short space id, epochs, the reason, the reporting
  // package's version. Once a kind per space per hour from a page. The bag is PUBLIC (anyone writes a report) but each
  // report is SEALED, whole, to SUPPORT's public key (`support-config`: `{ support }`) — its prefixes (reporter, space,
  // member) would otherwise tell anyone who shares which space with whom; only support opens one (tools/health-watch).
  // No support key: nothing is reported (never in the clear). A day's reports CHAINED: `health <day>`, then
  // `health <day> #1`, … (a bag holds ROOM items: a growing network's heartbeats fill one).
  const healthRef = (day, k) => (k ? `health ${day} #${k}` : `health ${day}`);
  let supportKey;
  const support = () =>
    (supportKey ??= ctx.optional("index: support-config", ctx
      .require("support-config")
      .then(b => JSON.parse(dec.decode(b)).support)
      .then(h => (/^[0-9a-f]{64}$/.test(h ?? "") ? bytes(h) : null)), () => null));
  // Once a kind per space (per member it is about, per reason) per hour from a page — a NEW reason is a new report.
  const reported = new Map();
  async function health(item) {
    const key = `${item.k}|${item.s ?? ""}|${item.to ?? ""}|${item.why ?? ""}`;
    if (Date.now() - (reported.get(key) ?? 0) < 3600e3) return false;
    reported.set(key, Date.now());
    const day = new Date().toISOString().slice(0, 10);
    // The site's manifest hash, and `~previous` when the version before it ran (its pieces not fetchable yet): kept —
    // cut to 12 characters it was dropped, and a page on the previous version reported as the new one (10-18: the
    // owner's login ran v403's code while every report named v404).
    const [h, ran] = String(ctx.siteV ?? ctx.shaOf?.("conversation") ?? "").split("~");
    const v = `${h.slice(0, 12)}${ran ? `~${ran}` : ""}`;
    // WHO reported (the account's id, its first 8 hex: reports from one account told apart, and its other reports
    // read together) — never more of it.
    const r = String((await space.account())?.id ?? "").replace(/^did:craftec:/, "").slice(0, 8) || null;
    const to = await support();
    if (!to) {
      ctx.log("health", { what: `${item.k}: not reported (this page has no support key: support-config)` });
      return false;
    }
    try {
      const payload = Core.seal_to(to, enc.encode(JSON.stringify({ ...item, r, v, at: Date.now() })), crypto.getRandomValues(new Uint8Array(32)));
      await chained((d, k) => refAddress(healthRef(d, k)), day, payload, "reporting health");
      return true;
    } catch (e) {
      // NOT SILENT: a report that could not be written is logged here (and asked again next hour).
      reported.delete(key);
      ctx.log("health", { what: `${item.k}: not reported (${e?.message ?? e})` }); // (never a fault: reporting it would report again)
      return false;
    }
  }

  // ANY OPEN BAG by its address (a registry: the epoch's vouches): add an item (with its work), read its items.
  // A CHAINED list (`wallet::chained`: payments, the witness queue, the files registry) — an epoch's items in bags
  // k = 0, 1, …: DROP into the first with room, read back (over a cap a bag keeps its most-worked items: not kept →
  // the next); READ k = 0.. up to the first with room. `addressOf(epoch, k)` names the list (an address, or its promise).
  // FULL by count or by bytes (`Core.chained_full`: the contract's one rule — a few big items fill a bag by bytes).
  const full = items => Core.chained_full(items);
  async function chained(addressOf, epoch, item, what) {
    const want = item.join(",");
    for (let k = 0; k < 64; k++) {
      const address = await addressOf(epoch, k);
      const items = await read(address, what);
      if (items.some(i => i.join(",") === want)) return;
      if (full(items)) continue;
      await drop(address, item, what);
      if ((await read(address, what)).some(i => i.join(",") === want)) return;
    }
    throw new Error(`${what}: its bags are full`);
  }
  async function chain(addressOf, epoch, what, { strict = false } = {}) {
    const out = [];
    for (let k = 0; k < 64; k++) {
      const items = await read(await addressOf(epoch, k), what, { strict });
      out.push(...items);
      if (!full(items)) break;
    }
    return out;
  }
  const bag = { add: (address, payload, what = "adding to a bag") => drop(address, payload, what), read: (address, what = "reading a bag", opts = {}) => read(address, what, opts), chained, chain };
  return { bag, onChange: f => changed.push(f), send, inbox: ctx.total("index.inbox", () => [], inbox), sendTo, inboxWith, makeInbox, request, requests: ctx.total("index.requests", () => [], requests), openRequests, openPointers, point, pointers: ctx.total("index.pointers", () => [], pointers), watch, listSpace, spaces: ctx.total("index.spaces", () => [], spaces), health, spacePoint, watchSpace, spacePointers: ctx.total("index.spacePointers", () => [], spacePointers), discoverPoint, discoverPointers: ctx.total("index.discoverPointers", () => [], discoverPointers), monthOf, monthBefore, monthsSince, OPEN };
}
