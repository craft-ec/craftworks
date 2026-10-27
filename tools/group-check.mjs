// GROUP CHECK (the build's): KEYS FOLLOW THE LIST ONE WAY (FOUNDATION "Members and keys", the design of 10-17). A space's
// keys are its BeeKEM tree (`members`), which follows its list — K8.6: there is NO MLS space group, so no MLS entry and no
// entry checked against the list exist on a page any more (banned below). NOBODY ADDS ANYONE by asking: no key package,
// no welcome, no request for keys — the ask/answer layer that ran ~60 fixes (10-03 → 10-16) cannot come back. One turn
// (`rules::admission::turn`), one order of a position's entries (`rules::ordering`). Fails the build, naming each site.
//
//   node tools/group-check.mjs [repo dir]
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";

const root = process.argv[2] ?? new URL("..", import.meta.url).pathname;
const pkgs = join(root, "packages");
const bad = [];
const lineOf = (src, i) => src.slice(0, i).split("\n").length;
// [pattern, the one file it may be in, how many times there, what it is]
const ONE = [
  [/\bglue\.ordering_entries\(/g, "ordering.js", 1, "a position's entries in the one order: `ordering` (`rules::ordering::order`)"],
];
// The PAGE never adds, welcomes, asks for keys or takes anyone out of a space's group.
const BANNED = [
  [/\.enter\(sp\.idBytes,|\.apply_checked\(/g, "an MLS space entry: K8.6 — a space's keys are its BeeKEM tree (`members`), which follows its list"],
  [/2166136261/g, "a turn ranking of its own: `ordering.turn` (the core's `rules::admission::turn`)"],
  [/\.key_package\(|\bkeyPackages\(|SpaceMember\([^)]*,[^)]*,[^)]*\)/g, "a key package: nobody adds anyone — a listed device enters by itself"],
  [/\bjoin_space\(|\bwelcome_message\(|kind:\s*"welcome"/g, "a welcome: nobody adds anyone — a listed device enters by itself"],
  [/\.readd\(|\bm\.add\(|\bg\.add\(/g, "an add of another's key: nobody adds anyone — a listed device enters by itself"],
  [/kind:\s*"key"|memberBag\(|askAgainIn\(|myKey\(/g, "a request for keys (\"my key\"): retired — the device enters by itself"],
  [/admission_decide\(|guard_post_key\(|guard_join_key_of_listed\(/g, "the retired ask/answer rules: the list decides, the device enters"],
  [/`c\/\$\{/g, "an ordered log's row built by hand: `rules::ordering` (`ordering_position_prefix`) — 10-15, a bare `c/<n>` no reader took"],
  [/\.remove\(\s*nodes\.map/g, "a member taken out of a space's group by a page: the node's follower does it (the acts decide)"],
];
// Comments (a line's `//` on) are prose, not sites: blanked, lines kept.
const code = src => src.replace(/(^|[^:"'`\\])\/\/[^\n]*/g, (m, a) => a + " ".repeat(m.length - a.length));
for (const f of readdirSync(pkgs).filter(f => f.endsWith(".js"))) {
  if (f.startsWith("craftworks_core") || f === "mls-glue.js") continue;
  const src = code(readFileSync(join(pkgs, f), "utf8"));
  for (const [re, home, n, what] of ONE) {
    const hits = [...src.matchAll(re)];
    if (f !== home) for (const m of hits) bad.push(`packages/${f}:${lineOf(src, m.index)}: ${what}`);
    else if (hits.length !== n) bad.push(`packages/${f}: ${hits.length} sites, the one implementation is ${n} — ${what}${hits.length ? ` (lines ${hits.map(m => lineOf(src, m.index)).join(", ")})` : ""}`);
  }
  // (`members.js` is the BeeKEM door: a member ADDS who the list names there, by design — the MLS layer's bans are not
  // its rules.)
  if (f !== "members.js") for (const [re, what] of BANNED) for (const m of src.matchAll(re)) bad.push(`packages/${f}:${lineOf(src, m.index)}: ${what}`);
  // ONE DOOR TO BEEKEM on a page: its package loaded only by `members` (our check before it).
  if (f !== "members.js") for (const m of src.matchAll(/"groups-glue"|"groups-wasm"|\bBeeGroup\b/g)) bad.push(`packages/${f}:${lineOf(src, m.index)}: BeeKEM used outside its one door on a page: \`members\``);
}
// The Rust crates.
const rs = [];
const walk = d => {
  for (const e of readdirSync(d)) {
    if (e.startsWith(".") || ["target", "vendor", "node_modules", "pkg", "sdk"].includes(e)) continue;
    const p = join(d, e);
    const s = statSync(p);
    if (s.isDirectory()) walk(p);
    else if (e.endsWith(".rs")) rs.push(p);
  }
};
walk(root);
for (const p of rs) {
  const rel = relative(root, p);
  const src = readFileSync(p, "utf8");
  if (rel !== join("rules", "src", "admission.rs")) for (const m of src.matchAll(/2166136261/g)) bad.push(`${rel}:${lineOf(src, m.index)}: a turn ranking of its own: \`craftworks_rules::admission::turn\``);
  if (rel !== join("rules", "src", "ordering.rs")) for (const m of src.matchAll(/"c\/\{:012\}"/g)) bad.push(`${rel}:${lineOf(src, m.index)}: an ordered log's row built by hand: \`craftworks_rules::ordering\``);
  // THE MLS LAYER (until K8 removes it): NOBODY ADDS ANYONE there (an MLS Add or Welcome: the device enters by itself).
  // The BeeKEM layer (`groups/`, docs/proposals/members-and-keys-beekem.md, owner 10-17) ADDS — by design: a member
  // adds who the list names — so the rule covers every crate but that one door.
  const inGroups = rel.startsWith(`groups${sep}`);
  if (!inGroups) for (const m of src.matchAll(/\.add_member\(|\.join_group\(|welcome_messages/g)) bad.push(`${rel}:${lineOf(src, m.index)}: an MLS add or welcome: nobody adds anyone — a listed device enters by itself (\`mls::enter_with\`)`);
  // ONE DOOR TO BEEKEM: the tree, its operations and its content API only through `groups` (our check before it).
  if (!inGroups) for (const m of src.matchAll(/\bbeekem::|\bCgka\b/g)) bad.push(`${rel}:${lineOf(src, m.index)}: BeeKEM used outside its one door: \`craftworks_groups\` (our check before it)`);
  // ONE WAY IN: the external commit builder only in `mls` (`enter_with`).
  if (rel !== join("mls", "src", "lib.rs")) for (const m of src.matchAll(/external_commit_builder\(/g)) bad.push(`${rel}:${lineOf(src, m.index)}: an entry of its own: \`mls::enter_with\``);
  // ONE ENTRY CHECK: `entry_ok` defined only in `mls`.
  if (rel !== join("mls", "src", "lib.rs")) for (const m of src.matchAll(/fn entry_ok\(/g)) bad.push(`${rel}:${lineOf(src, m.index)}: a second entry check: \`mls::entry_ok\``);
}
if (bad.length) {
  console.error(`group-check: ${bad.length} site(s) outside the one way keys follow the list\n${bad.join("\n")}`);
  process.exit(1);
}
console.log("group-check: ok");
