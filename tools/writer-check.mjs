// WRITER CHECK (the build's): WHOSE DEVICE WROTE THIS HAS ONE ANSWER (owner 10-16: "every time, there is something
// that you missed out … no matter how many times i say no duplicate implementation"). The one map is
// `storage.writersOf` over the core's `space_writers` (`craftworks_account::writers` — the node's delegate runs the same):
// a space's members' current devices and its DEPARTED writers (a member put out, a device dropped), capped where they
// stood. An account's devices are resolved once (`core.account_devices` = `craftworks_account::devices`). Seven places
// built their own map from current devices only — a dropped device's rows had no author (the owner's space lost its
// rules and left Discover). Fails the build naming each site that looks up devices outside the places allowed.
//
//   node tools/writer-check.mjs [repo dir]
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const root = process.argv.slice(2).find(a => !a.startsWith("--")) ?? new URL("..", import.meta.url).pathname;
const pkgs = join(root, "packages");
// Where an account's devices may be looked up, and why: directory (their one resolution), storage (the one map),
// moderation (recording departed writers), space (a leave act naming its own devices), roles (from outside: whose
// departed record to read).
const ALLOWED = new Set(["directory.js", "storage.js", "moderation.js", "space.js", "roles.js"]);
const RULES = [
  [/\.devices(?:\.strict)?\(/g, "an account's devices looked up to map writers: `storage.writersOf` (the one map)"],
  [/\bcore\.account_(?:devices|members)\(|craftworks_devices_contract::current\(/g, "devices resolved by hand: `directory.devices` (`craftworks_account::devices`)"],
];
const RESOLVERS = new Set(["directory.js", "membership.js"]); // (membership: the account's own nodes, by its gossiped feeds)
const bad = [];
const code = src => src.replace(/(^|[^:"'`\\])\/\/[^\n]*/g, (m, a) => a + " ".repeat(m.length - a.length));
for (const f of readdirSync(pkgs).filter(f => f.endsWith(".js"))) {
  const src = code(readFileSync(join(pkgs, f), "utf8"));
  const line = i => src.slice(0, i).split("\n").length;
  for (const m of src.matchAll(RULES[0][0])) if (!ALLOWED.has(f)) bad.push(`packages/${f}:${line(m.index)}: ${RULES[0][1]}`);
  for (const m of src.matchAll(RULES[1][0])) if (!RESOLVERS.has(f)) bad.push(`packages/${f}:${line(m.index)}: ${RULES[1][1]}`);
}
if (bad.length) {
  console.error(`writer-check: ${bad.length} site(s) decide whose device wrote outside the one map\n${bad.join("\n")}`);
  process.exit(1);
}
console.log("writer-check: ok");
