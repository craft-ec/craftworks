// READ CHECK (the build's): EVERY CONTRACT THAT CAN CHANGE IS READ ONCE, THEN FOLLOWED (FOUNDATION.md §2 R6; owner
// 10-15: "we should not be making unnecessary calls … not using the same data already available"). The one door is
// `node.contract`: the first GET subscribes, pushes keep the core's copy, a held contract answers with no GET, an absent
// one is asked again only on the door's own backoff. A GET built anywhere else, a consuming `take_got` of a followed
// contract, or a private retry loop is a second way to read — it fails the build, naming each site.
//
//   node tools/read-check.mjs [repo dir]
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const root = process.argv[2] ?? new URL("..", import.meta.url).pathname;
const pkgs = join(root, "packages");
const bad = [];
const lineOf = (src, i) => src.slice(0, i).split("\n").length;
// [pattern, { file: how many sites there }, what it is]
const ONE = [
  [
    /\bcore\.(frames_get|frames_follow|tail_get)\(/g,
    // node: the door's own; index, storage: the frames each hands the door; blocks, files: content-addressed (a tree
    // block, a file piece — never change: read once per page). (The loader's newest version: through the door, 10-16.)
    { "node.js": 1, "index.js": 1, "storage.js": 1, "blocks.js": 2, "files.js": 1 },
    "a GET built outside the door (`node.contract`)",
  ],
  [/\bcore\.take_got\(/g, { "node.js": 1, "blocks.js": 1, "files.js": 2 }, "a held copy TAKEN (`held_state` reads it; only immutable reads are freed)"],
];
const BANNED = [
  [/\bbackoff\(/g, "a retry loop of its own: the door asks an absent contract again (`node.onContract`)"],
  // R4: a per-page cache of a READ kept by hand keeps its failure (or a fallback in its place) for the page's life —
  // `ctx.cache(map, key, make, fallback?)` forgets a failure.
  [/if \(!(\w+)\.has\([^)]*\)\)\s*\1\.set\([^\n]*(\.then\(|ctx\.optional\(|ctx\.require\()/g, "a cache of a read kept by hand: `ctx.cache` (a failure is forgotten, FOUNDATION §2 R4)"],
];
const code = src => src.replace(/(^|[^:"'`\\])\/\/[^\n]*/g, (m, a) => a + " ".repeat(m.length - a.length));
for (const f of readdirSync(pkgs).filter(f => f.endsWith(".js"))) {
  const src = code(readFileSync(join(pkgs, f), "utf8"));
  for (const [re, homes, what] of ONE) {
    const hits = [...src.matchAll(re)];
    const n = homes[f] ?? 0;
    if (hits.length > n) bad.push(`packages/${f}: ${hits.length} site(s), at most ${n} — ${what} (lines ${hits.map(m => lineOf(src, m.index)).join(", ")})`);
  }
  for (const [re, what] of BANNED) for (const m of src.matchAll(re)) bad.push(`packages/${f}:${lineOf(src, m.index)}: ${what}`);
}
if (bad.length) {
  console.error(`read-check: ${bad.length} site(s) read outside the one door\n${bad.join("\n")}`);
  process.exit(1);
}
console.log("read-check: ok");
