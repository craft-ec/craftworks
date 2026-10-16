// GROUP CHECK (the build's): LETTING SOMEONE INTO A GROUP HAS ONE IMPLEMENTATION (FOUNDATION.md §3, "Members and keys"; owner
// 10-13: "one implementation and no duplicate"). Every MLS group — a space, an audience, a circle, a conversation, a
// level group — is entered the same way: a REQUEST carrying its asker's own key (`conversation.askIn`, the one
// `index.request`), decided by `rules::admission` (the one `admission_decide`, in `conversation.admit`; the identity
// delegate runs the same rule), let in by `conversation.welcome` (the one `readd`/`add` of a member), ranked by the one
// turn (`rules::admission::turn`: no FNV of its own anywhere else). Before this, seven paths each added on their own
// rules — the loop that drained cards and ran a space to epoch 478. Fails the build, naming each site.
//
//   node tools/group-check.mjs [repo dir]
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const root = process.argv[2] ?? new URL("..", import.meta.url).pathname;
const pkgs = join(root, "packages");
const bad = [];
const lineOf = (src, i) => src.slice(0, i).split("\n").length;
// [pattern, the one file it may be in, how many times there, what it is]
const ONE = [
  [/\bindex\.request\(/g, "conversation.js", 1, "a request to be let in: only `askIn` (it carries the device's own key)"],
  [/\badmission_decide\(/g, "conversation.js", 1, "the admission rule: only `admit`"],
  [/\.readd\(/g, "conversation.js", 1, "a member let in again: only `welcome`"],
  [/\bkeyPackages\(/g, "conversation.js", 1, "a key package made: only for a request (`askIn`)"],
];
const BANNED = [
  [/2166136261/g, "a turn ranking of its own: `ordering.turn` (the core's `rules::admission::turn`)"],
  [/kind:\s*"welcome-again"/g, "`welcome-again` is retired: the asker asks in again (`askAgainIn`)"],
  [/\.put\(\s*"kp"/g, "a card's key packages are retired: a request carries its own"],
  [/`c\/\$\{/g, "an ordered log's row built by hand: `rules::ordering` (`ordering_position_prefix`) — 10-15, a bare `c/<n>` no reader took"],
];
// `keys.js` is the MLS group module: its `group(sp).readd` calls the binding and `keyPackages` is defined there — the
// operations' home, not uses of them.
const HOMES = { "keys.js": new Set(["a member let in again: only `welcome`", "a key package made: only for a request (`askIn`)"]) };
// Comments (a line's `//` on) are prose, not sites: blanked, lines kept.
const code = src => src.replace(/(^|[^:"'`\\])\/\/[^\n]*/g, (m, a) => a + " ".repeat(m.length - a.length));
for (const f of readdirSync(pkgs).filter(f => f.endsWith(".js"))) {
  const src = code(readFileSync(join(pkgs, f), "utf8"));
  for (const [re, home, n, what] of ONE) {
    if (HOMES[f]?.has(what)) continue;
    const hits = [...src.matchAll(re)];
    if (f !== home) for (const m of hits) bad.push(`packages/${f}:${lineOf(src, m.index)}: ${what}`);
    else if (hits.length !== n) bad.push(`packages/${f}: ${hits.length} sites, the one implementation is ${n} — ${what}${hits.length ? ` (lines ${hits.map(m => lineOf(src, m.index)).join(", ")})` : ""}`);
  }
  for (const [re, what] of BANNED) for (const m of src.matchAll(re)) bad.push(`packages/${f}:${lineOf(src, m.index)}: ${what}`);
}
// The Rust crates: the one turn is `rules/src/admission.rs`.
const rs = [];
const walk = d => {
  for (const e of readdirSync(d)) {
    if (e.startsWith(".") || ["target", "vendor", "node_modules", "pkg"].includes(e)) continue;
    const p = join(d, e);
    const s = statSync(p);
    if (s.isDirectory()) walk(p);
    else if (e.endsWith(".rs")) rs.push(p);
  }
};
walk(root);
for (const p of rs) {
  const rel = relative(root, p);
  if (rel === join("rules", "src", "admission.rs")) continue;
  const src = readFileSync(p, "utf8");
  for (const m of src.matchAll(/2166136261/g)) bad.push(`${rel}:${lineOf(src, m.index)}: a turn ranking of its own: \`craftworks_rules::admission::turn\``);
  if (rel !== join("rules", "src", "ordering.rs")) for (const m of src.matchAll(/"c\/\{:012\}"/g)) bad.push(`${rel}:${lineOf(src, m.index)}: an ordered log's row built by hand: \`craftworks_rules::ordering\``);
}
if (bad.length) {
  console.error(`group-check: ${bad.length} site(s) let people into a group outside the one implementation\n${bad.join("\n")}`);
  process.exit(1);
}
console.log("group-check: ok");
