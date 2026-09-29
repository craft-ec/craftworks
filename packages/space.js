// SPACE, a capability: the SHAPE of an object with members — a group chat, a server and its channels, a community, a
// company, and YOUR ACCOUNT, the first. It defines what a space IS, once; the capabilities that apply to spaces take
// one: `membership` (who belongs), `keys` (its MLS group: epochs, escrow), `ordering` (its logs), `storage` (its
// members' feeds and tables), and later `roles`, `governance`, `administration`, `moderation`, `access`, `index`.
// A group chat, a Discord server, a Facebook group or a subreddit is a space CONFIGURED: its access, how one joins,
// its roles, its content and what indexes it.
//
// A space is:
// - `id`: what names it (your account: its DID).
// - `governance`: the root that admits members — whose signature makes a credential count (your account: the owner
//   keys of its key log, `{ kind: "key-log" }`).
// - `self`: the writer key its feeds are written under here — this device's (a DID is a space's member; each of its
//   devices writes its own feed on its behalf, and the DID's card says which devices are its: `directory.devices`);
//   `shared`: the space's shared key (your account: its data key — the tables from before feeds, and the channel).
// - `tables`: the names of its own tables — `catalog` (each writer's tables), `members` (credentials and removals,
//   gossiped), `channel` (its group's pointer and commits). Named once, by the identity (its rules name them too).
//
// A SERVER (the first space made by people, Discord's): made by one person, who owns it (`governance: { kind: "owner",
// owner: <DID>, nonce }`). Its id PROVES its owner: the id is sha-256 of the owner and a nonce drawn at making, so no
// member can claim another (`space.owner(sp)`: the owner, if the id proves it). Its group is its members' nodes; its tables are named `x<first 12 hex of its id>-<name>`. The spaces
// a person belongs to are listed in their account's table `spaces` (`<id>` → `{ kind, name, owner, at }`).
//
//   const space = await ctx.require("space");
//   const account = await space.account()   // the logged-in account as a space, or null
//   await space.mine()                      // the spaces this person belongs to
//   await space.create("server", name)      // a new server: made here, listed in the account
//   space.tableOf(sp, "channels")           // a space's own table's name
//   space.channel(server, id, name)         // a CHANNEL: a sub-space of the server
//   space.board(sp)                         // its BOARD: posts beside its messages (same members and roles)
//   await space.owner(sp)                   // its owner's DID, if its id proves it (else null)
//   await space.leave(sp)                   // out of this person's list (on every device of the account)
//   space.shown(sp)                         // how it is shown: `name#abc123` (names are not unique)
//
// A CHANNEL is a SUB-SPACE: it has a `parent` and inherits what it does not set itself. A channel that inherits the
// server's access has no group of its own — its group, scope and keys are the server's, and its messages are its own
// table in the server. (A channel that narrows access — private, a moderators' room — gets its own members and group:
// with roles.)
export async function start(ctx) {
  const auth = await ctx.require("auth");
  const { glue } = await ctx.require("node");
  const tables = Object.freeze(JSON.parse(glue.account_tables()));

  const SPACES = "spaces";
  const hex = b => [...b].map(x => x.toString(16).padStart(2, "0")).join("");
  const bytes = h => new Uint8Array(h.match(/../g).map(b => parseInt(b, 16)));
  // A space's own table: the identity's rule for names is 1–32 of a–z 0–9 - _.
  const tableOf = (sp, name) => `x${sp.id.slice(0, 12)}-${name}`;

  async function account() {
    const s = await auth.check();
    if (!s) return null;
    return Object.freeze({
      kind: "account",
      id: s.did,
      idBytes: s.didBytes,
      governance: Object.freeze({ kind: "key-log", did: s.didBytes }),
      self: s.member,
      shared: s.data,
      fresh: s.fresh,
      tables,
    });
  }

  // A space from its row in the account's `spaces`.
  function made(id, v, acc, self = acc.self) {
    const sp = { kind: v.kind, id, idBytes: bytes(id), name: v.name, with: v.with ?? null, governance: Object.freeze({ kind: "owner", owner: v.owner, nonce: v.nonce ?? null }), self };
    sp.tables = Object.freeze({ catalog: tableOf(sp, "tables"), members: tableOf(sp, "members"), channel: tableOf(sp, "log") });
    // A space that is itself a conversation (a direct one): its messages, in its own scope.
    sp.messages = tableOf(sp, "messages");
    sp.scope = sp;
    return Object.freeze(sp);
  }

  // A CHANNEL of a server: a sub-space inheriting the server's access (its group, members, keys and scope).
  function channel(server, id, name) {
    return Object.freeze({ kind: "channel", id: `${server.id}/${id}`, name, parent: server, inherits: true, messages: tableOf(server, `c${id}`), scope: server });
  }

  // A space's BOARD: its posts (Reddit's), beside its messages — a sub-space like a channel: the same group, members,
  // roles and moderation, its posts in the space's own table `board`. Every space has one (a server, a group, a
  // direct conversation).
  // Its PUBLIC board (`pub-board`: written in the clear — anyone reads; its members write), when the board reads in
  // public; `outside`: the same seen by someone not in the space (`sp`: its description, { id, name, governance }).
  function board(sp, { pub = false, outside = false } = {}) {
    const name = pub || outside ? "pub-board" : "board";
    return Object.freeze({ kind: "board", id: `${sp.id}/${name}`, name: sp.name, parent: sp, inherits: true, messages: tableOf(sp, name), scope: sp, outside });
  }

  async function mine() {
    const acc = await account();
    if (!acc) return [];
    const t = await (await ctx.require("storage")).table(SPACES);
    return t
      .rows()
      .map(r => {
        try {
          return made(r.key, JSON.parse(r.value), acc);
        } catch {
          return null;
        }
      })
      .filter(Boolean)
      .sort((a, b) => a.name.localeCompare(b.name));
  }

  // A space someone else made, JOINED here: listed in the account (its group is `keys`').
  async function record(id, v) {
    const acc = await account();
    await (await (await ctx.require("storage")).table(SPACES)).put(id, JSON.stringify({ ...v, at: v.at ?? Date.now() }));
    return made(id, v, acc);
  }

  // A space as it would be listed, before it is (to join its group first).
  const describe = async (id, v) => made(id, v, await account());

  // The id an owner's space has with a nonce.
  const idOf = async (owner, nonce) => hex(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(`craftworks space\0${owner}\0${nonce}`))));
  const owner = async sp => {
    const g = (sp.parent ?? sp).governance;
    return g?.owner && g.nonce && (await idOf(g.owner, g.nonce)) === (sp.parent ?? sp).id ? g.owner : null;
  };

  // A NEW space: its id from its owner (this person) and a nonce, its group made by this node (its first member), then
  // listed in the account.
  async function create(kind, name, extra = {}) {
    const acc = await account();
    if (!acc) throw new Error("nobody is logged in");
    const nonce = hex(crypto.getRandomValues(new Uint8Array(16)));
    const v = { kind, name, owner: acc.id, nonce, at: Date.now(), ...extra };
    const sp = made(await idOf(acc.id, nonce), v, acc);
    await (await ctx.require("keys")).group(sp).create();
    await (await (await ctx.require("storage")).table(SPACES)).put(sp.id, JSON.stringify(v));
    ctx.log("space", { what: `${kind} “${name}” made: ${sp.id.slice(0, 12)}…` });
    return sp;
  }

  // LEAVE: out of the space — a `leave` act (whoever may remove then takes this person's nodes out of the group, and
  // its files re-key: a leaver reads nothing newer, like one removed) — and out of this person's list. The owner hands
  // the space on first; theirs is only the list.
  async function leave(sp) {
    if (sp.kind === "server") {
      const r = await (await ctx.require("roles")).of(sp).catch(() => null);
      const me = (await account())?.id;
      // Naming its devices (written by one of them), so what this person wrote stays theirs once they are out.
      const nodes = await (await ctx.require("directory")).devices(me).catch(() => []);
      if (r && !r.left && r.role(me) && r.role(me) !== "owner") await r.act({ act: "leave", did: me, nodes });
    }
    await (await (await ctx.require("storage")).table(SPACES)).remove(sp.id);
    ctx.log("space", { what: `left ${sp.kind} “${sp.name}”` });
  }

  // How a space is SHOWN, everywhere: its name and the start of its id — `Makers#30fe18` (names are not unique; the id
  // is), as people are (`pat#8r4orC`).
  const shown = sp => `${String(sp?.name ?? "").trim() || "space"}#${String(sp?.id ?? "").slice(0, 6)}`;

  return { account, tables, mine, create, record, describe, tableOf, channel, board, owner, leave, shown };
}
