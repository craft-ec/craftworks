// APP CHECK (the build's): AN APP RUNS IN ITS OWN FRAME (FOUNDATION.md §3, "Apps are installed"; ARCHITECTURE §1). A
// page module that mounts with the page's `ctx` (`mount(ctx, el)`) is the OS's — or an app NOT YET moved into its frame.
// The OS list is fixed; the NOT-YET list only shrinks: an app moved (`mount(api, el)`, published as bytes) is taken off
// it in the same change. A new page module on neither list fails the build.
//
//   node tools/app-check.mjs [repo dir]
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const root = process.argv.slice(2).find(a => !a.startsWith("--")) ?? new URL("..", import.meta.url).pathname;
const pkgs = join(root, "packages");
const bad = [];
const bad0 = s => bad.push(s);
// THE OS: what every app stands on (FOUNDATION §3) — stays in the page.
// (The page's LAYOUT is the OS's: its header, side and footer — the manifest's `layout`.)
const OS = new Set(["header", "places-side", "footer", "home", "settings", "account", "spaces-app", "space-home", "store", "tag", "contact", "chat", "mail", "wallet-app", "nodes-app", "explorer-app"]);
// APPS NOT YET IN THEIR FRAMES (10-15): only ever shorter.
const NOT_YET = new Set([]);
// FRAMED BY THE FRAME RUNTIME (FOUNDATION §3): the manifest's `framed` (the publisher's) — the app keeps its
// `mount(ctx, el)`, run in its frame with its grants.
const pub = readFileSync(join(root, "publish/src/main.rs"), "utf8");
const framedSrc = pub.match(/\\"framed\\": \{\{(.*?)\}\}/)?.[1] ?? "";
const FRAMED = new Set([...framedSrc.matchAll(/\\"([a-z0-9-]+)\\": \[/g)].map(m => m[1]));
if (!FRAMED.size) bad0("publish/src/main.rs: no `framed` map found (the check must see one)");
for (const f of readdirSync(pkgs).filter(f => f.endsWith(".js"))) {
  const name = f.slice(0, -3);
  const src = readFileSync(join(pkgs, f), "utf8");
  if (!/export\s+async\s+function\s+mount\s*\(\s*ctx\b/.test(src)) continue;
  if (OS.has(name) || NOT_YET.has(name) || FRAMED.has(name)) continue;
  bad.push(`packages/${f}: a page mounted with the page's ctx — an app runs in its own frame (\`mount(api, el)\`), or it is the OS's`);
}
// A name on the NOT-YET list that is framed now (or gone): take it off.
for (const name of NOT_YET) {
  let src = "";
  try {
    src = readFileSync(join(pkgs, `${name}.js`), "utf8");
  } catch {}
  if (!/export\s+async\s+function\s+mount\s*\(\s*ctx\b/.test(src)) bad.push(`${name}: framed (or gone) — take it off app-check's NOT-YET list`);
}
// EACH FRAMED APP's GRANTS ARE WHAT ITS CODE REACHES (10-16: Blog's hand list missed `blog-press` once it became the
// host's): the host capabilities required by the app and every package it loads into its frame (the trust core —
// `apphost.js`'s HOST, its one owner — is where the walk stops). Not one fewer (the app fails), not one more (least
// privilege). `--fix` writes the publisher's map from this.
const hostSrc = readFileSync(join(pkgs, "apphost.js"), "utf8");
const HOST = new Set([...(hostSrc.match(/const HOST = new Set\(\[([^\]]*)\]\)/)?.[1] ?? "").matchAll(/"([a-z0-9-]+)"/g)].map(m => m[1]));
if (HOST.size < 10) bad.push("packages/apphost.js: its HOST set not found (the check must see it)");
const srcOf = name => {
  try {
    return readFileSync(join(pkgs, `${name}.js`), "utf8");
  } catch {
    return "";
  }
};
const reqs = src => [...src.matchAll(/require\(\s*"([a-z0-9-]+)"\s*\)/g)].map(m => m[1]).concat([...src.matchAll(/\[([^\[\]]*)\]\.map\(\w+ => (?:ctx|app)\.require\(\w+\)\)/g)].flatMap(m => [...m[1].matchAll(/"([a-z0-9-]+)"/g)].map(x => x[1])));
// WHICH VARIABLE holds which host capability in a package: `const [a, b] = await Promise.all(["x", "y"].map(n =>
// ctx.require(n)))` or `([ctx.require("x"), ctx.require("y")])` (over several lines too), `x = await ctx.require("x")`,
// `ctx.require("x").then(x => …)`.
// A require by a KIND's field (`ctx.require(spec.maker)`, `…view`): every name `kinds.js` — the one catalog of makers
// and views — gives that field.
const CATALOG = {};
for (const m of srcOf("kinds").matchAll(/\b(maker|view): "([a-z0-9-]+)"/g)) (CATALOG[m[1]] ??= new Set()).add(m[2]);
const bindingsOf = src => {
  const b = new Map(); // var → cap
  for (const m of src.matchAll(/\[([^\]]+)\]\s*=\s*await Promise\.all\(/g)) {
    let i = m.index + m[0].length;
    for (let d = 1; i < src.length && d > 0; i++) d += src[i] === "(" ? 1 : src[i] === ")" ? -1 : 0;
    const expr = src.slice(m.index + m[0].length, i);
    const lit = /^\s*\[([^\]]*)\]\s*\.map\(/.exec(expr);
    // (A listed array: its elements by position — `ctx.require("x")` or anything else, split at its top level.)
    const elements = () => {
      const body = expr.slice(expr.indexOf("[") + 1, expr.lastIndexOf("]"));
      const out = [];
      let d = 0, at = 0;
      for (let k = 0; k <= body.length; k++) {
        const c = body[k];
        if (c === "(" || c === "[" || c === "{") d++;
        else if (c === ")" || c === "]" || c === "}") d--;
        else if ((c === "," && d === 0) || k === body.length) out.push(body.slice(at, k)), (at = k + 1);
      }
      return out.map(e => /^\s*(?:await\s+)?\(?\s*(?:ctx|app)\.require\(\s*"([a-z0-9-]+)"\s*\)\s*\)?\s*$/.exec(e)?.[1] ?? null);
    };
    const names = lit ? [...lit[1].matchAll(/"([a-z0-9-]+)"/g)].map(x => x[1]) : elements();
    m[1].split(",").map(v => v.trim()).forEach((v, k) => /^\w+$/.test(v) && names[k] && b.set(v, names[k]));
  }
  for (const m of src.matchAll(/(\w+)\s*=\s*await\s+(?:ctx|app)\.require\("([a-z0-9-]+)"\)/g)) b.set(m[1], m[2]);
  for (const m of src.matchAll(/(?:ctx|app)\.require\("([a-z0-9-]+)"\)\.then\(\s*(?:async\s*)?\(?(\w+)/g)) b.set(m[2], m[1]);
  // (By a kind's field: the variable is each name the catalog gives — `var#name`, one binding per name.)
  for (const m of src.matchAll(/(\w+)\s*=\s*await\s+(?:ctx|app)\.require\(\s*[\w.]+\.(maker|view)\s*\)/g)) for (const n of CATALOG[m[2]] ?? []) b.set(`${m[1]}#${n}`, n);
  return b;
};
// AN APP's GRANTS, call by call (10-16): every `capability.method` its code — the app's and each package it loads into
// its frame — calls on the trust core (`HOST`, where the walk stops). A capability required where no call on it can be
// SEEN fails the check (it must be visible to be granted). A handle a granted call returns is the host's to hand out.
const grantsOf = app => {
  const seen = new Set();
  const grants = new Set();
  const walk = [app];
  while (walk.length) {
    const n = walk.pop();
    if (seen.has(n)) continue;
    seen.add(n);
    const src = srcOf(n);
    // IN THE FRAME, A HOST'S ANSWER IS A PROMISE (FOUNDATION §3): a space's roles read as a value there — `r.apps().
    // includes(…)` — throws (10-19 `where`; 10-20 the video page: "(intermediate value)?.apps(...).includes is not a
    // function", from captions). Awaited (`(await r.apps()).includes`) it is the same in the page and in a frame.
    for (const m of src.matchAll(/\b(apps|roster|members|acts|roles|held|lists|invites)\(\)\.(includes|some|filter|map|find|length|every|forEach)\b/g))
      syncReads.add(`packages/${n}.js:${src.slice(0, m.index).split("\n").length}: ${m[1]}() read as a value in an app's frame (${app}) — await it: (await r.${m[1]}()).${m[2]}`);
    const b = bindingsOf(src);
    for (const m of src.matchAll(/(?:ctx|app)\.require\(\s*(?!")([^)]*)\)/g)) {
      const arg = m[1].trim();
      const field = /^[\w.]+\.(maker|view)$/.exec(arg)?.[1];
      if (field) {
        for (const nm of CATALOG[field] ?? []) {
          if (!HOST.has(nm)) {
            walk.push(nm);
            continue;
          }
          // `(await ctx.require(x.maker)).call(` — the call on it.
          const direct = new RegExp(`\\(await (?:ctx|app)\\.require\\(\\s*${arg.replace(/\./g, "\\.")}\\s*\\)\\)\\.(\\w+)`, "g");
          for (const c of src.matchAll(direct)) grants.add(`${nm}.${c[1]}`);
        }
      } else if (!/^(n|name)$/.test(arg)) bad.push(`packages/${n}.js:${src.slice(0, m.index).split("\n").length}: requires by a name the check cannot see (${arg}) — a literal name, or a kind's maker / view`);
    }
    for (const r of new Set(reqs(src))) {
      if (!HOST.has(r)) {
        walk.push(r);
        continue;
      }
      let found = 0;
      for (const m of src.matchAll(new RegExp(`\\(await (?:ctx|app)\\.require\\("${r}"\\)\\)\\.(\\w+)`, "g"))) grants.add(`${r}.${m[1]}`), found++;
      for (const [vn, cap] of b) {
        if (cap !== r) continue;
        const v = vn.split("#")[0];
        for (const m of src.matchAll(new RegExp(`(?<![\\w.$])${v}\\??\\.(\\w+)`, "g"))) grants.add(`${r}.${m[1]}`), found++;
      }
      if (!found) bad.push(`packages/${n}.js: requires "${r}" but no call on it can be seen (bind it as const x = await ctx.require("${r}"), or in a Promise.all list)`);
    }
  }
  // A host module's own FRAME code (its `frame(cap, …)` export, run in the app's frame): its calls on `cap` are the
  // app's too (directory's name element reads `cap.name`, `cap.levelOf`).
  for (const r of new Set([...grants].map(g => g.split(".")[0]))) {
    const src = srcOf(r);
    const m = /export (?:async )?function frame\(\s*(\w+)/.exec(src);
    if (!m) continue;
    let i = src.indexOf("{", m.index);
    const from = i;
    for (let d = 0; i < src.length; i++) if ((d += src[i] === "{" ? 1 : src[i] === "}" ? -1 : 0) === 0) break;
    for (const c of src.slice(from, i).matchAll(new RegExp(`(?<![\\w.$])${m[1]}\\.(\\w+)`, "g"))) grants.add(`${r}.${c[1]}`);
  }
  return [...grants].sort();
};
const syncReads = new Set();
// THE FRAME RULES (10-30, after a person's Drive, upload and Form broke in their frames): every host call an app's frame
// makes is (1) ASYNC by its definition in the host package — `async`, or `ctx.total(…)` — or one of that package's frame
// helpers / constants (a plain function or an object through the proxy is a Promise or nothing there: `space.shared`,
// `conversation.mail`); and (2) CLASSIFIED in `apphost.js`: a READ (no gesture) or a WRITE (the person's gesture) — never
// neither (an unclassified read was refused at a person's click-less page: `share.groups`, `editions.section`).
const frameRules = (grants) => {
  const listOf = name => new Set([...(hostSrc.match(new RegExp(`const ${name} = new Set\\(\\[([\\s\\S]*?)\\]\\);`))?.[1] ?? "").matchAll(/"([A-Za-z0-9_$]+)"/g)].map(m => m[1]));
  const READS = listOf("READS"), WRITES_ = listOf("WRITES_DECLARED");
  if (READS.size < 50) bad.push("packages/apphost.js: its READS set not found (the frame rules must see it)");
  const helpersOf = cap => {
    const s = srcOf(cap);
    const m = /export (?:async )?function frame\([^)]*\)\s*\{/.exec(s);
    if (!m) return new Set();
    let i = s.indexOf("{", m.index);
    const from = i;
    for (let d = 0; i < s.length; i++) if ((d += s[i] === "{" ? 1 : s[i] === "}" ? -1 : 0) === 0) break;
    const body = s.slice(from, i);
    const ret = body.slice(body.lastIndexOf("return {") + 8);
    const keys = new Set([...ret.matchAll(/(?:^|[,{])\s*(?:\.\.\.)?([A-Za-z_$][\w$]*)\s*(?=[:,}\n(])/g)].map(x => x[1]));
    // (`...pure(…)`: the names pure returns.)
    for (const sp of ret.matchAll(/\.\.\.(\w+)\(/g)) { const f = new RegExp(`function ${sp[1]}\\([^)]*\\)\\s*\\{[\\s\\S]*?return \\{([^}]*)\\}`).exec(s); for (const k of (f?.[1] ?? "").split(",")) keys.add(k.trim().split(":")[0].trim()); }
    return keys;
  };
  const isAsync = (cap, meth) => {
    const s = srcOf(cap);
    const e = meth.replace(/\$/g, "\\$");
    return new RegExp(`async function ${e}\\(`).test(s) || new RegExp(`(?:const|let) ${e} = (?:async\\b|ctx\\.total\\()`).test(s) || new RegExp(`\\b${e}: (?:async\\b|ctx\\.total\\()`).test(s) || new RegExp(`\\basync ${e}\\(`).test(s);
  };
  // A SYNC host function reached through the proxy is a Promise in the frame: each frame call site must AWAIT it, return
  // it, or leave its value unused (a statement of its own) — a value USED there (compared, tested, read, assigned) is the
  // bug (`sh.parentId(g) === …`, `const id = node.newHex(6)`, `filter(s => space.madeBy(s, me))`).
  const syncs = new Set([...grants].filter(g => { const [cap, meth] = g.split("."); return !helpersOf(cap).has(meth) && !/^[A-Z_]+$/.test(meth) && !isAsync(cap, meth); }));
  const framePkgs = new Set();
  for (const a of FRAMED) { const walk = [a]; while (walk.length) { const n = walk.pop(); if (framePkgs.has(n) || HOST.has(n)) continue; framePkgs.add(n); for (const r of reqs(srcOf(n))) walk.push(r); } }
  for (const n of framePkgs) {
    const src = srcOf(n);
    const sites = [];
    for (const [vn, cap] of bindingsOf(src)) { const v = vn.split("#")[0]; for (const m of src.matchAll(new RegExp(`(?<![\\w.$])${v}\\??\\.(\\w+)\\(`, "g"))) sites.push([cap, m[1], m.index]); }
    for (const m of src.matchAll(/\(await (?:ctx|app)\.require\("([a-z0-9-]+)"\)\)\.(\w+)\(/g)) sites.push([m[1], m[2], m.index + m[0].indexOf(")).") + 3]);
    for (const [cap, meth, at] of sites) {
      if (!syncs.has(`${cap}.${meth}`)) continue;
      const before = src.slice(Math.max(0, at - 80), at).replace(/\.$/, "").replace(/\(await (?:ctx|app)\.require\("[a-z0-9-]+"\)\)$/, "").replace(/[\w$.?]*$/, "");
      // (Awaited, returned, voided, handed to `background`; or a STATEMENT — after `;` `{` `}`, a line's start, an `if (…)`'s
      // condition, or in a comma sequence of statements — its value unused.)
      if (/(?:\bawait|\breturn|=>|\bvoid|background\([^()]*,)\s*\(?\s*$/.test(before) || /(?:^|[;{}]|\n)\s*$/.test(before) || /\bif\s*\([^()]*(?:\([^()]*\))*[^()]*\)\s*$/.test(before) || /[(,]\s*\w+\.\w+\([^()]*(?:\{[^{}]*\})?[^()]*\),\s*$/.test(before)) continue;
      bad.push(`packages/${n}.js:${src.slice(0, at).split("\n").length}: ${cap}.${meth}() is sync in the host — a Promise in an app's frame, its value used here: await it, or make it a frame helper of ${cap}`);
    }
  }
  for (const g of grants) {
    const [cap, meth] = g.split(".");
    if (helpersOf(cap).has(meth) || /^[A-Z_]+$/.test(meth)) continue;
    if (!READS.has(meth) && !WRITES_.has(meth)) bad.push(`${g}: neither a READ nor a WRITE in packages/apphost.js — classify it`);
  }
};
const framedMap = Object.fromEntries([...framedSrc.matchAll(/\\"([a-z0-9-]+)\\": \[([^\]]*)\]/g)].map(m => [m[1], [...m[2].matchAll(/\\"([a-z0-9.A-Z_-]+)\\"/g)].map(x => x[1]).sort()]));
const want = Object.fromEntries(Object.keys(framedMap).map(a => [a, grantsOf(a)]));
for (const x of syncReads) bad.push(x);
frameRules(new Set(Object.values(want).flat()));
if (process.argv.includes("--fix")) {
  const inner = Object.entries(want).map(([a, g]) => `\\"${a}\\": [${g.map(x => `\\"${x}\\"`).join(", ")}]`).join(", ");
  writeFileSync(join(root, "publish/src/main.rs"), pub.replace(/\\"framed\\": \{\{.*?\}\}/, `\\"framed\\": {{ ${inner} }}`));
  console.log("app-check: the publisher's framed grants written");
  process.exit(0);
}
for (const [a, g] of Object.entries(want)) {
  const has = framedMap[a];
  const missing = g.filter(x => !has.includes(x));
  const extra = has.filter(x => !g.includes(x));
  if (missing.length || extra.length) bad.push(`${a}: its grants ≠ what its code reaches${missing.length ? ` — missing ${missing.join(", ")}` : ""}${extra.length ? ` — not needed ${extra.join(", ")}` : ""} (node tools/app-check.mjs --fix)`);
}
if (bad.length) {
  console.error(`app-check: ${bad.length} site(s)\n${bad.join("\n")}`);
  process.exit(1);
}
console.log(`app-check: ok (${FRAMED.size} framed: ${[...FRAMED].join(", ")}; ${NOT_YET.size} not yet)`);
