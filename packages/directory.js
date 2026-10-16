// DIRECTORY, a capability: every person's public CARD — found from their DID alone: DID → their key log (its address
// comes from the DID) → their account's data key → their card, a public tail under it. Nobody else can write it; anyone
// reads it. A card holds a HANDLE (a name to show — not unique: a person is their handle AND their id), the DID's KEY
// PACKAGES (MLS: with one, anyone adds this person to a conversation while they are away — the person, never a device),
// the account's INBOX key (what is sealed to it only the account's nodes open: `index`'s inbox), and its DEVICES'
// credentials (each signed by an owner key of the DID's key log: where a device writes on the DID's behalf in a space,
// readers learn it is that DID's).
//
//   const directory = await ctx.require("directory");
//   await directory.card(did)                 // { did, handle, inbox, … }, or null (no key packages: a request carries its own)
//   await directory.publish({ handle })       // this person's handle, and their key packages
//   await directory.setContributions(list)    // this person's contributions, PUBLIC on their card (rewards §5a)
//   await directory.setPay({ scan, spend })   // this person's wallet pay keys on their card (TOKEN §7f); card(did).pay
//   await directory.levelOf(did)              // their level now, from their card: { id, name } ("free"|"pro"|"vip")
//   await directory.handle(did)               // their handle, or null (each card read once per page)
//   directory.shown(did, handle)              // how a person is SHOWN everywhere: `pat#8r4orC`
//   await directory.name(did)                 // the same, their handle looked up
//   await directory.publicOf(did, name)       // any public tail of theirs (`card`, `mail`: only their account writes it)
//   await directory.dataKey(did)              // their account's data key (hex) as their key log names it, or null
//   await directory.devices(did, fresh?)      // their devices' keys (hex), each credential checked against their key log
//   directory.devicesWhy(did)                 // why their devices read empty here (null: they did not)
//   directory.onDevices(did, fn)              // their devices changed (their card moved)
//   await directory.listMe(on)   directory.isListed()   // shown in DISCOVER (Contacts' public view), or not
//   await directory.listed()                  // the people shown in Discover: [did] (each one's card says so)
// How a person is SHOWN, everywhere: their handle and the start of their id — `pat#8r4orC` (handles are not unique;
// the id is). Without a handle: `#8r4orC`.
const shown = (did, handle) => `${handle ?? ""}#${String(did).replace(/^did:craftec:/, "").slice(0, 6)}`;
// A person's NAME ON A PAGE — the one way one is drawn: an element showing what is known now (never waited on),
// its text replaced when their card is read.
// With their BADGE (rewards §5a): ⭐ PRO, 💎 VIP — their level now, from their public contributions.
const BADGE = { pro: " ⭐ PRO", vip: " 💎 VIP" };
const nameElOf =
  (ctx, { name, levelOf, known = () => null }) =>
  (did, tag = "span", props = {}) => {
    const e = Object.assign(document.createElement(tag), props, { textContent: shown(did, known(did)) });
    ctx.background("directory: Promise.all", Promise.all([name(did), ctx.optional("directory: levelOf", levelOf(did), () => null)]).then(([t, lv]) => (e.textContent = t + (BADGE[lv?.id] ?? ""))));
    return e;
  };
// IN AN APP's FRAME (FOUNDATION §3, "THE FRAME RUNTIME"): the same helpers over the host's proxy (`cap`) — a name
// shown at once by its id, then as read.
export function frame(cap, ctx) {
  return { shown, nameEl: nameElOf(ctx, { name: cap.name, levelOf: cap.levelOf }) };
}

export async function start(ctx) {
  const auth = await ctx.require("auth");
  const storage = await ctx.require("storage");
  const space = await ctx.require("space");
  const { core, glue } = await ctx.require("node");
  const idlogCode = await ctx.require("idlog-wasm");
  const CARD = "card";

  const didBytes = did => (typeof did === "string" ? glue.did_bytes(did) : did);
  // An account's public keys, from its key log (read first).
  async function keysOf(did) {
    const b = didBytes(did);
    if (!(await auth.identity.readKeyLog(b))) return null;
    return core.idlog_keys(idlogCode, b);
  }

  // A REQUEST'S KEY IS THEIRS (`account::space_member`, the delegate's check too): the key package's credential names
  // `did` under the data key their key log holds now. A request is an unsigned drop in a bag: without this, one naming
  // someone carrying another's key would let that other in under their name. Why not, or null.
  async function keyNotTheirs(did, kpHex) {
    if (!(await keysOf(did))) return "their key log not read";
    const cred = await (await ctx.require("keys")).kpCredential(kpHex);
    if (!cred) return "the request's key does not read as a key package";
    return core.space_member_of(idlogCode, didBytes(did), cred) ? null : "the request's key is not theirs (another account's, or a key their log moved past)";
  }

  const read = t => {
    const rows = t.rows();
    return {
      handle: rows.find(r => r.key === "handle")?.value ?? null,
      inbox: rows.find(r => r.key === "inbox")?.value ?? null,
      // Their wallet's PAY KEYS (TOKEN §7f: what a private payment to them is derived from) — { scan, spend } hex, or null.
      pay: (() => {
        try {
          const p = JSON.parse(rows.find(r => r.key === "pay")?.value ?? "null");
          return p && /^[0-9a-f]{64}$/.test(p.scan) && /^[0-9a-f]{64}$/.test(p.spend) ? { scan: p.scan, spend: p.spend } : null;
        } catch {
          return null;
        }
      })(),
      // Their CONTRIBUTIONS (rewards §5a, PUBLIC — owner 10-07): [{ at, amount, days }], what their level is made of.
      contrib: (() => {
        try {
          const list = JSON.parse(rows.find(r => r.key === "contrib")?.value ?? "[]");
          return Array.isArray(list) ? list.filter(c => c && Number(c.amount) > 0 && Number(c.days) > 0 && Number(c.at) > 0) : [];
        } catch {
          return [];
        }
      })(),
      // The account's devices' credentials (hex), as its key group has them.
      nodes: (() => {
        try {
          const list = JSON.parse(rows.find(r => r.key === "nodes")?.value ?? "[]");
          return Array.isArray(list) ? list.filter(x => typeof x === "string") : [];
        } catch {
          return [];
        }
      })(),
      // Whether the account HAS a devices contract (its own word, written once it put one): readers look it up only then
      // — an absent contract is never searched for.
      hasDevices: rows.find(r => r.key === "devices")?.value === "1",
    };
  };

  // A person's public tail `name`, under their account's data key: whatever is in it, their account wrote.
  async function publicOf(did, name, opts = {}) {
    // Nobody (a guest's "me"): an empty table — nothing there.
    if (did == null) return storage.empty(name);
    const me = await space.account.strict();
    const id = typeof did === "string" ? did : glue.did_of(did);
    if (me?.id === id) return storage.publicTail(name, me.shared, opts);
    const k = await keysOf(did);
    return k ? storage.publicTail(name, k.data, opts) : null;
  }

  // A WRITE CREDENTIAL's public table (`circles` writes it, `roles` checks it): named by its token alone — 32 hex,
  // a table's whole name (a name is at most 32 bytes: `cred-<token>` never fit).
  const credTable = (owner, token) => publicOf(owner, String(token), { unlisted: true });

  async function card(did, { fresh = false } = {}) {
    const t = await publicOf(did, CARD);
    await t?.answer?.();
    if (fresh) await ctx.background("directory: t?.reread", t?.reread?.());
    if (!t || t.absent) return null;
    return { did: typeof did === "string" ? did : glue.did_of(did), ...read(t) };
  }

  // THIS person's card: its handle, and this node's key package (made by `keys`, its secrets kept with this node).
  async function publish({ handle } = {}) {
    await (await ctx.require("keys")).mustWrite("the card");
    const sp = await space.account.strict();
    if (!sp?.shared) throw new Error("this node does not hold the account's data key");
    const t = await storage.publicTail(CARD, sp.shared);
    if (handle != null && read(t).handle !== handle) await t.put("handle", handle);
    // Shown at once, everywhere on this page.
    if (handle != null) handles.set(sp.id, Promise.resolve(handle));
    // The inbox key, and the (empty) inbox made now: nobody ever waits on an inbox that does not exist yet.
    if (!read(t).inbox) {
      const k = await auth.identity.inboxKey();
      if (!k.inboxKey) throw new Error(`no inbox key here: ${k.refused ?? JSON.stringify(k)}`);
      await (await ctx.require("index")).makeInbox();
      await t.put("inbox", k.inboxKey);
    }
    await putNodes(t);
    return { did: sp.id, ...read(t) };
  }

  // THIS person's CONTRIBUTIONS on their card (PUBLIC: anyone computes their level from them — `usage.levelFrom`).
  // This person's wallet PAY KEYS on their card (TOKEN §7f: the only wallet fact tied to the DID).
  async function setPay(keys) {
    const sp = await space.account.strict();
    if (!sp?.shared) throw new Error("this node does not hold the account's data key");
    const t = await storage.publicTail(CARD, sp.shared);
    await t.put("pay", JSON.stringify({ scan: keys.scan, spend: keys.spend }));
  }
  async function setContributions(list) {
    const sp = await space.account.strict();
    if (!sp?.shared) throw new Error("this node does not hold the account's data key");
    const t = await storage.publicTail(CARD, sp.shared);
    await t.put("contrib", JSON.stringify(list.map(c => ({ at: c.at, amount: c.amount, days: c.days }))));
    levels.delete(sp.id);
  }
  // A person's LEVEL now, from their card's contributions (once per page per person).
  const levels = new Map();
  function levelOf(did) {
    return ctx.cache(levels, did, () => Promise.all([card(did), ctx.require("usage")]).then(([c, u]) => u.levelFrom(c?.contrib ?? [])), () => ({ id: "free", name: "Free" }));
  }

  // The account's DEVICES on the card: the credentials its key group holds now (rewritten when they change: a device
  // joined or was removed).
  async function putNodes(t) {
    const st = await ctx.optional("directory: ready", (await ctx.require("keys")).ready(), () => null);
    const creds = (st?.members ?? []).map(m => m.cred).filter(Boolean).sort();
    // And into the account's DEVICES contract (a grow-only set: the union is kept) — where readers count them.
    const me = await auth.check();
    const put = me && creds.length ? await auth.identity.addDevices(me.didBytes, creds.map(h => new Uint8Array(h.match(/../g).map(x => parseInt(x, 16))))).then(() => true, e => (ctx.failed("card", e, `the account's devices: ${e.message}`), false)) : false;
    // Said on the card once the contract is put: what readers check before looking it up.
    if (put && !read(t).hasDevices) await t.put("devices", "1");
    if (!creds.length || JSON.stringify(creds) === JSON.stringify(read(t).nodes.slice().sort())) return;
    await t.put("nodes", JSON.stringify(creds));
  }

  // A person's HANDLE, read once per page (a name to show; the id is what makes them them).
  // Kept while known: NOT when there is none yet (a card read before its name was set — a new person's own page — would
  // otherwise show no name until a reload), and dropped when their card changes (the node pushes it).
  const handles = new Map();
  const knownHandles = new Map(); // did → the handle last read (shown at once on the next draw)
  const watchedCards = new Set();
  function handle(did) {
    if (!handles.has(did)) {
      const p = ctx.optional("directory: card", card(did).then(c => c?.handle ?? null), () => null);
      p.then(h => h != null && knownHandles.set(did, h));
      handles.set(did, p);
      p.then(h => h == null && handles.get(did) === p && handles.delete(did));
      if (!watchedCards.has(did)) {
        watchedCards.add(did);
        ctx.background("directory: publicOf", publicOf(did, CARD).then(t => t?.onChange(() => handles.delete(did))));
      }
    }
    return handles.get(did);
  }

  const name = async did => shown(did, await handle(did));
  const nameEl = nameElOf(ctx, { name, levelOf, known: did => knownHandles.get(did) });

  // A person's DATA key, from their key log (once per page): what signs their member's credential in a space.
  const dataKeys = new Map();
  function dataKey(did) {
    return ctx.cache(dataKeys, did, () => keysOf(did).then(k => k?.data ?? null), null);
  }

  // A person's DEVICES: the keys their card's credentials name, each checked against their key log (a credential signed
  // by a key the log never named counts for nothing). Once per page, or again when asked (`fresh`: a device may have
  // joined since).
  const deviceSets = new Map();
  // A person's devices CHANGED (their card moved: a device added or removed): `fn(did)`. Their card is followed once,
  // and the devices read again on its next change.
  const deviceWatch = new Map(); // did → [fn]
  function onDevices(did, fn) {
    if (!deviceWatch.has(did)) {
      deviceWatch.set(did, []);
      ctx.background("directory: publicOf", publicOf(did, CARD)
        .then(t =>
          t?.onChange(() => {
            deviceSets.delete(did);
            for (const f of deviceWatch.get(did)) f(did);
          }),
        ));
    }
    deviceWatch.get(did).push(fn);
  }
  // WHY a person's devices read EMPTY (`devicesWhy`): an empty list is not "no devices" — their key log, devices contract
  // or card may not have been read here — and what shows it (a member's key state) must say which.
  const deviceWhy = new Map();
  function devices(did, fresh = false) {
    if (fresh || !deviceSets.has(did))
      deviceSets.set(
        did,
        (async () => {
          const none = why => (deviceWhy.set(did, why), []);
          if (!(await keysOf(did))) return none("their key log not read");
          // Their DEVICES contract (owner 10-09): the devices on the key log's CURRENT key — looked up only when their
          // card says they have one (an absent contract costs a whole network search on every page). Their card's
          // devices otherwise (the card is the data key's: any of its devices could rewrite it).
          const c = await card(did, { fresh });
          const creds = (c?.nodes ?? []).map(h => new Uint8Array(h.match(/../g).map(x => parseInt(x, 16))));
          // WHICH COUNT: the one resolution (`craftworks_account::devices` in the core — the node's too): their devices
          // contract when read, else their card's. (Here only what to READ, and when.)
          const devicesCode = await ctx.require("devices-wasm");
          const resolve = () => Array.from(core.account_devices(devicesCode, idlogCode, didBytes(did), creds));
          if (c?.hasDevices) await auth.identity.readDevices(didBytes(did));
          let out = resolve();
          // Nothing counts and their card does not say: an account that put its contract before cards said so.
          if (!out.length && !c?.hasDevices && (await auth.identity.readDevices(didBytes(did)))) out = resolve();
          if (out.length) return (deviceWhy.delete(did), out);
          if (!c) return none("their card not read");
          return none(c.nodes.length ? `none of their card's ${creds.length} device(s) on their key log` : "their card lists no device");
        })().catch(e => (deviceWhy.set(did, `reading their devices: ${String(e?.message ?? e).slice(0, 80)}`), [])),
      );
    return deviceSets.get(did);
  }
  const devicesWhy = did => deviceWhy.get(did) ?? null;
  // EVERY DEVICE they have had (their devices contract: a grow-only set — `craftworks_account::devices_ever`), current
  // or dropped: what a dropped one wrote before is still theirs (`storage.writersOf`, up to where it stood).
  async function devicesEver(did) {
    if (!(await keysOf(did))) return [];
    const devicesCode = await ctx.require("devices-wasm");
    if (!(await auth.identity.readDevices(didBytes(did)))) return [];
    return Array.from(core.account_devices_ever(devicesCode, idlogCode, didBytes(did)));
  }

  // DISCOVER's people: whoever chose to be shown. A pointer in one public bag says where to look; their own CARD says
  // whether they still want it (row `listed`): a bag keeps its pointers, so leaving is the card's word.
  const PEOPLE = "discover:people";
  async function listMe(on) {
    const sp = await space.account.strict();
    const t = await storage.publicTail(CARD, sp.shared);
    await t.put("listed", on ? "1" : "");
    if (on) await (await ctx.require("index")).point(PEOPLE, { did: sp.id });
  }
  const isListed = async () => {
    const sp = await space.account.strict();
    return (await storage.publicTail(CARD, sp.shared)).rows().some(r => r.key === "listed" && r.value === "1");
  };
  async function listed() {
    const dids = [...new Set((await ctx.optional("directory: pointers", (await ctx.require("index")).pointers(PEOPLE, { show: true }), () => [])).map(p => p.did).filter(d => typeof d === "string" && d.startsWith("did:craftec:")))];
    const ok = await Promise.all(dids.map(d => ctx.optional("directory: publicOf", publicOf(d, CARD).then(t => !!t?.rows().some(r => r.key === "listed" && r.value === "1")), () => false)));
    return dids.filter((_, i) => ok[i]);
  }

  return { credTable, keyNotTheirs, card: ctx.total("directory.card", () => null, card), publish, setContributions, setPay, levelOf, devicesEver: ctx.total("directory.devicesEver", () => [], devicesEver), handle: ctx.total("directory.handle", () => null, handle), shown, name, nameEl, publicOf: ctx.total("directory.publicOf", () => null, publicOf), dataKey, devices: ctx.total("directory.devices", () => [], devices), devicesWhy, onDevices, listMe, isListed: ctx.total("directory.isListed", () => false, isListed), listed };
}
