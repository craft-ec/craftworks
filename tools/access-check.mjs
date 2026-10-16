// ACCESS CHECK (the build's): WHO MAY DO WHAT HAS ONE OWNER (the duplicate sweep, batch 3; owner 10-16: "continue the fix
// until completed"). The replay (`gov`, through the core and `roles`) answers every access question; a page asks
// `roles` — never a copy of a rule beside it:
//   - the permission an act needs: `gov::act_need` (`r.needOf`, `roles.act`) — no act → permission table of its own;
//   - an admin, now or then: `r.isAdmin(did, at)` — no `["owner", "admin"].includes(…)` beside it;
//   - whether a domain reads in public: `r.readsAnyone(domain)` — no `… "read") === "anyone"` beside it;
//   - the default where nothing is set: `r.fallback(path, action)`;
//   - a policy's vocabulary: `roles.actions` / `roles.validWho` (`gov::ACTIONS`, `gov::valid_who`);
//   - who made a space: `space.madeBy(sp, did)` — no `governance?.owner === …` beside it;
//   - who may take whom out: `r.mayTakeOut(by, did)`; an item's reference: `items.refOf(by, id)` (batch 5).
// Fails the build, naming each site.
//
//   node tools/access-check.mjs [repo dir]
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const root = process.argv[2] ?? new URL("..", import.meta.url).pathname;
const pkgs = join(root, "packages");
const lineOf = (src, i) => src.slice(0, i).split("\n").length;
const code = src => src.replace(/(^|[^:"'`\\])\/\/[^\n]*/g, (m, a) => a + " ".repeat(m.length - a.length));
// [pattern, the one file it may be in (null: none), what]
const RULES = [
  [/\["owner",\s*"admin"\]\.includes\(/g, "roles.js", "an admin check of its own: `r.isAdmin(did, at)`"],
  [/"read"\)\s*===\s*"anyone"/g, null, "a public-read test of its own: `r.readsAnyone(domain)`"],
  [/governance\??\.owner\s*===/g, "space.js", "who made a space, by hand: `space.madeBy(sp, did)`"],
  [/\{\s*role:\s*"roles",\s*assign:\s*"roles"/g, null, "an act → permission table of its own: `gov::act_need` (`r.needOf`)"],
  [/\["read",\s*"post",\s*"comment",\s*"vote"/g, null, "a policy vocabulary of its own: `roles.actions` (`gov::ACTIONS`)"],
  // (Batch 5.) Who may take whom out: the replay's own rule (`r.mayTakeOut`) — no rank table beside it.
  [/\{\s*owner:\s*3,\s*admin:\s*2,\s*member:\s*1\s*\}/g, null, "a role rank table of its own: `r.mayTakeOut(by, did)` (`Gov::may_take_out`)"],
  // An item's reference: `items.refOf` — never built by hand.
  [/`\$\{[\w.]*by\}\/\$\{[\w.]*id\}`/g, "items.js", "an item's reference built by hand: `items.refOf(by, id)`"],
];
const bad = [];
for (const f of readdirSync(pkgs).filter(f => f.endsWith(".js") && !f.startsWith("craftworks_core"))) {
  const src = code(readFileSync(join(pkgs, f), "utf8"));
  for (const [re, home, what] of RULES) {
    if (f === home) continue;
    for (const m of src.matchAll(re)) bad.push(`packages/${f}:${lineOf(src, m.index)}: ${what}`);
  }
}
if (bad.length) {
  console.error(`access-check: ${bad.length} site(s) decide access outside its one owner\n${bad.join("\n")}`);
  process.exit(1);
}
console.log("access-check: ok");
