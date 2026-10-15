// JOB CHECK (the build's): A TIMER IS EITHER A JOB OR A VIEW'S CLOCK (FOUNDATION.md §7.4). Background work runs as a
// lifecycle JOB (`lifecycle.job`: its needs, its state, its reports); a clock that only moves what is shown runs through
// `ctx.every(el, ms, fn)`, which stops when its element leaves the page. Any other `setInterval` is work nobody sees —
// it fails the build, naming each site.
//
//   node tools/job-check.mjs [repo dir]
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const root = process.argv[2] ?? new URL("..", import.meta.url).pathname;
const pkgs = join(root, "packages");
// The two homes: the job's own timer, and the view clock's.
const HOMES = { "lifecycle.js": 1, "runtime.js": 1 };
const bad = [];
const code = src => src.replace(/(^|[^:"'`\\])\/\/[^\n]*/g, (m, a) => a + " ".repeat(m.length - a.length));
for (const f of readdirSync(pkgs).filter(f => f.endsWith(".js"))) {
  const src = code(readFileSync(join(pkgs, f), "utf8"));
  const hits = [...src.matchAll(/\bsetInterval\(/g)].map(m => src.slice(0, m.index).split("\n").length);
  if (hits.length > (HOMES[f] ?? 0)) bad.push(`packages/${f}: setInterval at line(s) ${hits.join(", ")} — a lifecycle job (background work) or ctx.every (a view's clock)`);
}
if (bad.length) {
  console.error(`job-check: ${bad.length} file(s) with a timer outside a job or a view's clock\n${bad.join("\n")}`);
  process.exit(1);
}
console.log("job-check: ok");
