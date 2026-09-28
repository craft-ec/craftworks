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
// - `self`: this node's writer key in it (your account: this node's member key); `shared`: the space's shared key (your
//   account: its data key — the tables from before feeds, and the channel).
// - `tables`: the names of its own tables — `catalog` (each writer's tables), `members` (credentials and removals,
//   gossiped), `channel` (its group's pointer and commits). Named once, by the identity (its rules name them too).
//
//   const space = await ctx.require("space");
//   const account = await space.account()   // the logged-in account as a space, or null
export async function start(ctx) {
  const auth = await ctx.require("auth");
  const { glue } = await ctx.require("node");
  const tables = Object.freeze(JSON.parse(glue.account_tables()));

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

  return { account, tables };
}
