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
  const { core, glue, ask, WAIT, listen } = await ctx.require("node");
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
  async function read(address, what, { show = false, follow = false, strict = false } = {}) {
    const id = Core.bag_id(bagCode, address);
    const full = (async () => {
      const [, frames] = follow ? core.frames_follow(bytes(id)) : core.frames_get(bytes(id));
      const said = await ask(frames, x => (x.kind === "got" || x.kind === "get-failed") && x.id === id, what, WAIT.answer).catch(() => ({ kind: "silent" }));
      if (said.kind === "got") return held(address, id);
      if (said.kind === "get-failed" && !madeBags.has(id)) madeBags.add(id), drop(address, new Uint8Array(0), `making ${what}`).catch(() => madeBags.delete(id));
      if (said.kind === "silent" && strict) throw new Error(`no answer: ${what}`);
      return [];
    })();
    if (!show) return full;
    const early = await Promise.race([full.then(x => ({ x })), new Promise(r => setTimeout(r, WAIT.hint, null))]);
    if (early) return early.x;
    full.then(x => x.length && changed.forEach(f => f()), () => {});
    return held(address, id);
  }

  async function inbox({ show = false } = {}) {
    const sp = await space.account();
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
    await read(address, `the bag of ${ref}`, { follow: true }).catch(() => {});
  }

  // A SPACE's SEALED bag (`name`): at an address the space's id gives, each item sealed with the space's newest EPOCH
  // key (`epoch ‖ nonce ‖ AES-GCM`) — only members read it; a member removed reads nothing added after. Its size shows
  // to whoever knows the space's id, never what is in it.
  const sealedAddress = async (sp, name) => refAddress(`space ${glue.space_table_key(sp.idBytes, `bag-${name}`)}`);
  const aes = async keyHex => crypto.subtle.importKey("raw", bytes(keyHex), "AES-GCM", false, ["encrypt", "decrypt"]);
  // Sealed with the space's newest epoch — or, for its WRITERS list, with the SPACE's OWN key (from its id: `OPEN`):
  // every member reads it whatever epochs they hold — behind, ahead, on another branch of a fork, or missing the
  // early epochs (10-07: sealed with epoch 0, a member whose history stopped above it read 0 writers, so no acts and
  // an empty space; one without epoch 0 sealed with its newest and listed itself again on every load). It names
  // devices and branches only. Entries sealed with an epoch (before) are still read by whoever holds that epoch.
  const OPEN = 0xffffffff;
  const openKey = sp => aes(glue.space_table_key(sp.idBytes, "bag-writers-open"));
  async function spacePoint(sp, name, item, { epoch = -1 } = {}) {
    const access = await ctx.require("access");
    const k = name === "writers" ? { epoch: OPEN, key: glue.space_table_key(sp.idBytes, "bag-writers-open") } : await access.keyAt(`bag-${name}`, epoch, { space: sp.idBytes });
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
    // What could not be opened, by the epoch it was sealed with (said: a bag read empty because no key opens it looks
    // the same as an empty bag).
    const unread = new Map();
    let raw = 0;
    for (const p of await read(await sealedAddress(sp, name), `the space's ${name}`, { follow: name === "writers" })) {
      if (p.length < 17) continue;
      raw += 1;
      const epoch = new DataView(p.buffer, p.byteOffset).getUint32(0);
      if (!keys.has(epoch)) keys.set(epoch, epoch === OPEN ? await openKey(sp) : await access.keyAt(`bag-${name}`, epoch, { space: sp.idBytes }).then(k => (k.key ? aes(k.key) : null), () => null));
      const key = await keys.get(epoch);
      if (!key) {
        unread.set(epoch, (unread.get(epoch) ?? 0) + 1);
        continue;
      }
      try {
        // `sealedAt`: the epoch it was sealed with (an entry only this branch reads, or one every member does: `OPEN`).
        const v = JSON.parse(dec.decode(await crypto.subtle.decrypt({ name: "AES-GCM", iv: p.subarray(4, 16) }, key, p.subarray(16))));
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
    (supportKey ??= ctx
      .require("support-config")
      .then(b => JSON.parse(dec.decode(b)).support)
      .then(h => (/^[0-9a-f]{64}$/.test(h ?? "") ? bytes(h) : null))
      .catch(() => null));
  // Once a kind per space (per member it is about) per hour from a page.
  const reported = new Map();
  async function health(item) {
    const key = `${item.k}|${item.s ?? ""}|${item.to ?? ""}`;
    if (Date.now() - (reported.get(key) ?? 0) < 3600e3) return false;
    reported.set(key, Date.now());
    const day = new Date().toISOString().slice(0, 10);
    const v = String(ctx.siteV ?? ctx.shaOf?.("conversation") ?? "").slice(0, 12); // the site's manifest hash
    // WHO reported (the account's id, its first 8 hex: reports from one account told apart, and its other reports
    // read together) — never more of it.
    const r = String((await space.account().catch(() => null))?.id ?? "").replace(/^did:craftec:/, "").slice(0, 8) || null;
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
  return { bag, onChange: f => changed.push(f), send, inbox, sendTo, inboxWith, makeInbox, request, requests, openRequests, openPointers, point, pointers, watch, listSpace, spaces, health, spacePoint, spacePointers, discoverPoint, discoverPointers, monthOf, monthsSince, OPEN };
}
