// THE WIKI's pure parts (`packages/wiki-pages.js`): titles, [[links]], sections, versions and their merge (conflicts,
// suggestions), the edit plans — run with no node and no page:   node --test tools/wiki-pages.test.mjs
// `WIKI_PAGES_JS`: another copy of the file to test (the negative control: one whose conflict detection is taken out
// fails the conflict tests).
import test from "node:test";
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";

const src = process.env.WIKI_PAGES_JS ?? new URL("../packages/wiki-pages.js", import.meta.url).href;
const W = await import(src.startsWith("file:") ? src : pathToFileURL(src).href);
const { normTitle, cleanTitle, titleProblem, parseLinks, unescapeLinks, linkify, splitSections, joinSections, versionHash, versionOf, resolve, textOf, planWhole, planSection, planPropose, backlinks, tree } = W;

// A version as a writer stores it (an item of `items`' shape), its hash as every reader computes it.
const item = (by, at, meta, body = "") => ({ ref: `space:x/${by}-${meta.sid}`, by, at, body, meta });
const v = (by, at, meta, body) => versionOf(item(by, at, meta, body));
let n = 0;
const seq = () => `s${String(++n).padStart(4, "0")}`;

// ── titles ──
test("titles: one key for spacing, underscores and case", () => {
  assert.equal(normTitle("  Foo_bar   BAZ "), "foo bar baz");
  assert.equal(normTitle("Foo bar baz"), normTitle("foo_BAR baz"));
  assert.equal(normTitle("Ｆｏｏ"), "foo"); // full-width folded
  assert.equal(cleanTitle("  Foo_bar  "), "Foo bar");
});
test("titles: what cannot be one", () => {
  assert.equal(titleProblem("Good page"), null);
  assert.equal(titleProblem("Recipes/Bread"), null);
  assert.match(titleProblem("   "), /needs a title/);
  assert.match(titleProblem("a[b]"), /none of/);
  assert.match(titleProblem("a|b"), /none of/);
  assert.match(titleProblem("x".repeat(201)), /at most/);
  assert.match(titleProblem("/Bread"), /not empty/);
  assert.match(titleProblem("A//B"), /not empty/);
});

// ── links ──
test("links: [[Title]] and [[Title|label]], keyed by title", () => {
  const ls = parseLinks("See [[Main page]] and [[bread_recipes|the bread]]. Not [[ ]] or [[a]b]].");
  assert.deepEqual(ls.map(l => [l.title, l.label, l.key]), [["Main page", "Main page", "main page"], ["bread recipes", "the bread", "bread recipes"]]);
});
test("links: none read inside code", () => {
  const t = "a [[One]] `[[Two]]`\n```\n[[Three]]\n```\nb [[Four]]";
  assert.deepEqual(parseLinks(t).map(l => l.title), ["One", "Four"]);
});
test("links: rich text's escaped brackets made a link again", () => {
  assert.equal(unescapeLinks("x \\[\\[My\\_page\\]\\] y"), "x [[My_page]] y");
  assert.equal(unescapeLinks("x \\[\\[A\\|b\\]\\]"), "x [[A|b]]");
  assert.equal(unescapeLinks("plain [[A]]"), "plain [[A]]");
});
test("links: written as Markdown links, the address made safe", () => {
  const out = linkify("go [[Foo (bar)]] and `[[Code]]`", t => `#/wiki/t/${encodeURIComponent(t)}`);
  assert.equal(out, "go [Foo (bar)](#/wiki/t/Foo%20%28bar%29) and `[[Code]]`");
});

// ── sections ──
test("sections: cut at every heading; the lead kept when it holds something; code headings ignored", () => {
  const t = "Lead text.\n\n# One\nbody 1\n\n## Two ##\n```\n# not a heading\n```\n\n# Three";
  const s = splitSections(t);
  assert.deepEqual(s.map(x => [x.heading, x.level]), [[null, 0], ["One", 1], ["Two", 2], ["Three", 1]]);
  assert.equal(s[2].body, "## Two ##\n```\n# not a heading\n```");
  assert.deepEqual(splitSections("# Only\n").map(x => x.heading), ["Only"]);
  assert.deepEqual(splitSections("   \n\n"), []);
});
test("sections: split then joined gives the text back (normalized blank lines)", () => {
  const t = "Lead.\n\n# A\nx\n\n# B\ny";
  assert.equal(joinSections(splitSections(t)), t);
});

// ── versions and the merge ──
test("hash: the same text written over another version is a new version", () => {
  const a = { sid: "s1", base: null, merges: [], pos: 1, gone: false, body: "x" };
  assert.notEqual(versionHash(a), versionHash({ ...a, base: versionHash(a) }));
  assert.equal(versionHash({ ...a, merges: ["b", "a"] }), versionHash({ ...a, merges: ["a", "b"] }));
});
test("merge: a chain of edits by two editors has one head, the newest", () => {
  const v1 = v("A", 1, { sid: "sec1", pos: 1 }, "# S\none");
  const v2 = v("B", 2, { sid: "sec1", pos: 1, base: v1.h }, "# S\ntwo");
  const v3 = v("A", 3, { sid: "sec1", pos: 1, base: v2.h }, "# S\nthree"); // A's item now holds v3 (v1 gone)
  const { sections } = resolve([v3, v2]);
  assert.equal(sections.length, 1);
  assert.equal(sections[0].conflict, false);
  assert.equal(sections[0].body, "# S\nthree");
});
test("merge: two editors of DIFFERENT sections never collide", () => {
  const a1 = v("A", 1, { sid: "seca", pos: 1 }, "# A\n1");
  const b1 = v("A", 1, { sid: "secb", pos: 2 }, "# B\n1");
  const a2 = v("B", 5, { sid: "seca", pos: 1, base: a1.h }, "# A\n2"); // B edits section A
  const b2 = v("C", 5, { sid: "secb", pos: 2, base: b1.h }, "# B\n2"); // C edits section B, at once
  const { sections } = resolve([a1, b1, a2, b2]);
  assert.deepEqual(sections.map(s => [s.sid, s.conflict, s.body]), [["seca", false, "# A\n2"], ["secb", false, "# B\n2"]]);
});
// THE CONFLICT (what the negative control takes out): two edits of one section from the same version — both kept.
test("conflict: two edits of one section at once are BOTH kept, shown as a conflict", () => {
  const v1 = v("A", 1, { sid: "sec1", pos: 1 }, "# S\none");
  const x = v("B", 5, { sid: "sec1", pos: 1, base: v1.h }, "# S\nB's");
  const y = v("C", 6, { sid: "sec1", pos: 1, base: v1.h }, "# S\nC's");
  const { sections } = resolve([v1, x, y]);
  assert.equal(sections.length, 1);
  assert.equal(sections[0].conflict, true);
  assert.deepEqual(sections[0].heads.map(h => h.body).sort(), ["# S\nB's", "# S\nC's"]);
});
test("conflict: an edit from a stale view of the section conflicts with the newer line", () => {
  const v1 = v("A", 1, { sid: "sec1", pos: 1 }, "x1");
  const v2 = v("B", 2, { sid: "sec1", pos: 1, base: v1.h }, "x2");
  const v3 = v("A", 3, { sid: "sec1", pos: 1, base: v2.h }, "x3"); // A's item: v3
  const v4 = v("C", 4, { sid: "sec1", pos: 1, base: v1.h }, "x4"); // C saw only v1
  const { sections } = resolve([v3, v2, v4]);
  assert.equal(sections[0].conflict, true);
  assert.deepEqual(sections[0].heads.map(h => h.body), ["x3", "x4"]);
});
test("conflict: settled by an edit that merges the other head — one head again", () => {
  const v1 = v("A", 1, { sid: "sec1", pos: 1 }, "one");
  const x = v("B", 5, { sid: "sec1", pos: 1, base: v1.h }, "B's");
  const y = v("C", 6, { sid: "sec1", pos: 1, base: v1.h }, "C's");
  const before = resolve([v1, x, y]).sections;
  const ops = planSection(before, "sec1", "B's and C's", { base: x.h, merges: [y.h] });
  assert.deepEqual(ops.map(o => [o.op, o.base, o.merges]), [["edit", x.h, [y.h]]]);
  const z = v("B", 7, { sid: "sec1", ...ops[0] }, ops[0].body); // B's item now holds z
  const { sections } = resolve([v1, z, y]);
  assert.equal(sections[0].conflict, false);
  assert.equal(sections[0].body, "B's and C's");
});
// Found by the service flow (an in-memory run): a writer's item holds only their LATEST version — so the version that
// settled a conflict leaves it at their next edit, and the other head would come back unless what it settled is carried.
test("conflict: settled stays settled when the settler edits the section again (merges carried)", () => {
  const v1 = v("A", 1, { sid: "sec1", pos: 1 }, "one");
  const x = v("B", 5, { sid: "sec1", pos: 1, base: v1.h }, "B's");
  const y = v("A", 6, { sid: "sec1", pos: 1, base: v1.h }, "A's");
  const settle = { op: "edit", sid: "sec1", base: y.h, merges: [x.h], body: "both", pos: 1 };
  const z = v("A", 7, { sid: "sec1", pos: 1, base: settle.base, merges: W.carryMerges(y, settle, new Set([x.h])) }, "both");
  assert.equal(resolve([z, x]).sections[0].conflict, false);
  // A edits again: z leaves A's item; what z settled (x) is carried on.
  const again = { op: "edit", sid: "sec1", base: z.h, body: "both, edited", pos: 1 };
  const merges = W.carryMerges(z, again, new Set([x.h]));
  assert.ok(merges.includes(x.h));
  const w = v("A", 8, { sid: "sec1", pos: 1, base: z.h, merges }, "both, edited");
  const r = resolve([w, x]);
  assert.equal(r.sections[0].conflict, false);
  assert.equal(r.sections[0].body, "both, edited");
});
test("conflict: an edit against a deletion is a conflict too", () => {
  const v1 = v("A", 1, { sid: "sec1", pos: 1 }, "one");
  const del = v("B", 5, { sid: "sec1", pos: 1, base: v1.h, gone: true }, "");
  const ed = v("C", 6, { sid: "sec1", pos: 1, base: v1.h }, "kept");
  const { sections } = resolve([v1, del, ed]);
  assert.equal(sections[0].conflict, true);
  assert.equal(sections[0].gone, false);
  const only = resolve([v1, del]).sections;
  assert.equal(only[0].gone, true);
  assert.equal(textOf(only), "");
});
test("restoring an old text never hides the section (a version's hash names its base)", () => {
  const v1 = v("A", 1, { sid: "sec1", pos: 1 }, "same");
  const v2 = v("B", 2, { sid: "sec1", pos: 1, base: v1.h }, "changed");
  const v3 = v("A", 3, { sid: "sec1", pos: 1, base: v2.h }, "same"); // A restores v1's text
  const { sections } = resolve([v3, v2]);
  assert.equal(sections.length, 1);
  assert.equal(sections[0].body, "same");
  assert.equal(sections[0].conflict, false);
});

// ── suggestions ──
test("suggestions: a non-editor's version is pending, never the page's text; accepted or declined it goes", () => {
  const editors = new Set(["A"]);
  const may = x => editors.has(x.by);
  const v1 = v("A", 1, { sid: "sec1", pos: 1 }, "one");
  const s = v("Z", 2, { sid: "sec1", pos: 1, base: v1.h }, "Z's idea");
  let r = resolve([v1, s], may);
  assert.equal(r.sections[0].body, "one");
  assert.equal(r.pending.length, 1);
  assert.equal(r.pending[0].fresh, true);
  // Accepted: the editor writes it, settling it.
  const acc = v("A", 3, { sid: "sec1", pos: 1, base: v1.h, merges: [s.h] }, "Z's idea");
  r = resolve([acc, s], may);
  assert.equal(r.sections[0].body, "Z's idea");
  assert.equal(r.pending.length, 0);
  // Declined instead: the editor's review item names it.
  const rev = versionOf({ ref: "space:x/rev", by: "A", at: 3, body: "", meta: { declined: [s.h] } });
  r = resolve([v1, s, rev], may);
  assert.equal(r.pending.length, 0);
  assert.equal(r.sections[0].body, "one");
  // A non-editor's "decline" counts for nothing.
  const fake = versionOf({ ref: "space:x/rev2", by: "Z", at: 3, body: "", meta: { declined: [s.h] } });
  assert.equal(resolve([v1, s, fake], may).pending.length, 1);
});
test("suggestions: a non-editor cannot settle or supersede anything", () => {
  const may = x => x.by === "A";
  const v1 = v("A", 1, { sid: "sec1", pos: 1 }, "one");
  const evil = v("Z", 2, { sid: "sec1", pos: 1, base: v1.h, merges: [v1.h] }, "taken over");
  const r = resolve([v1, evil], may);
  assert.equal(r.sections[0].body, "one");
  assert.equal(r.pending.length, 1);
});
test("suggestions: on an older version, said so (not fresh)", () => {
  const may = x => x.by === "A";
  const v1 = v("A", 1, { sid: "sec1", pos: 1 }, "one");
  const v2 = v("A", 3, { sid: "sec1", pos: 1, base: v1.h }, "two");
  const s = v("Z", 2, { sid: "sec1", pos: 1, base: v1.h }, "Z's");
  assert.equal(resolve([v2, s], may).pending[0].fresh, false);
});

// ── plans ──
const page = text => {
  n = 0;
  const ops = planWhole([], text, { newSid: seq });
  const vs = ops.map(o => v("A", 1, { sid: o.sid, pos: o.pos, ...(o.base ? { base: o.base } : {}) }, o.body));
  return resolve(vs).sections;
};
test("plan: a new page is one new section per heading, in order", () => {
  const s = page("Lead\n\n# One\na\n\n# Two\nb");
  assert.deepEqual(s.map(x => x.heading), [null, "One", "Two"]);
  assert.ok(s[0].pos < s[1].pos && s[1].pos < s[2].pos);
});
test("plan: editing the whole page writes only the sections that changed", () => {
  const s = page("# One\na\n\n# Two\nb\n\n# Three\nc");
  const ops = planWhole(s, "# One\na\n\n# Two\nB CHANGED\n\n# Three\nc", { newSid: seq });
  assert.deepEqual(ops.map(o => [o.op, o.sid]), [["edit", s[1].sid]]);
  assert.equal(ops[0].base, s[1].h);
});
test("plan: a heading renamed keeps its section (matched by place); one added between gets a place between", () => {
  const s = page("# One\na\n\n# Two\nb\n\n# Three\nc");
  const ops = planWhole(s, "# One\na\n\n# New\nn\n\n# Two renamed\nb2\n\n# Three\nc", { newSid: () => "fresh1" });
  const add = ops.find(o => o.op === "new");
  const ren = ops.find(o => o.op === "edit");
  assert.equal(ren.sid, s[1].sid);
  assert.ok(add.pos > s[0].pos && add.pos < s[1].pos);
  assert.equal(ops.length, 2);
});
test("plan: a section removed is deleted; a section moved keeps its id with a new place", () => {
  const s = page("# One\na\n\n# Two\nb\n\n# Three\nc");
  const del = planWhole(s, "# One\na\n\n# Three\nc");
  assert.deepEqual(del.map(o => [o.op, o.sid]), [["gone", s[1].sid]]);
  const moved = planWhole(s, "# Three\nc\n\n# One\na\n\n# Two\nb");
  assert.deepEqual(moved.map(o => [o.op, o.sid]), [["edit", s[2].sid]]);
  assert.ok(moved[0].pos < s[0].pos);
});
test("plan: one section edited may grow new sections after it, before the next", () => {
  const s = page("# One\na\n\n# Two\nb");
  const ops = planSection(s, s[0].sid, "# One\na2\n\n# One and a half\nh", { newSid: () => "half" });
  assert.deepEqual(ops.map(o => [o.op, o.sid]), [["edit", s[0].sid], ["new", "half"]]);
  assert.ok(ops[1].pos > s[0].pos && ops[1].pos < s[1].pos);
  assert.deepEqual(planSection(s, s[0].sid, "# One\na"), []); // unchanged: nothing
  assert.deepEqual(planSection(s, s[1].sid, "  ").map(o => o.op), ["gone"]);
});
test("propose: a copy's changes go back over the versions copied — stale ones then conflict there", () => {
  const origin = page("# One\na\n\n# Two\nb");
  const heads = Object.fromEntries(origin.map(s => [s.sid, s.h]));
  const mine = [{ ...origin[0], body: "# One\nmine" }, origin[1], { sid: "added", pos: 9, body: "# Three\nc", gone: false }];
  const ops = planPropose(mine, origin, heads);
  assert.deepEqual(ops.map(o => [o.op, o.sid, o.base]), [["edit", origin[0].sid, origin[0].h], ["new", "added", null]]);
  // Deleted in the copy: deleted there.
  assert.deepEqual(planPropose([origin[0]], origin, heads).map(o => [o.op, o.sid]), [["gone", origin[1].sid]]);
  // The origin changed section One meanwhile: the proposal, written over the version copied, is a second head there.
  const theirs = v("B", 5, { sid: origin[0].sid, pos: origin[0].pos, base: origin[0].h }, "# One\ntheirs");
  const prop = v("Me", 6, { sid: origin[0].sid, pos: ops[0].pos, base: ops[0].base }, ops[0].body);
  const r = resolve([theirs, prop]);
  assert.equal(r.sections[0].conflict, true);
});

// ── around a page ──
test("what links here, and the index as a tree", () => {
  const pages = [
    { key: "a", title: "A", text: "see [[B]]" },
    { key: "b", title: "B", text: "self [[b]]" },
    { key: "c", title: "Recipes/Bread", text: "[[b|bee]] `[[A]]`" },
    { key: "d", title: "Recipes", text: "" },
  ];
  assert.deepEqual(backlinks(pages, "b").map(p => p.key), ["a", "c"]);
  assert.deepEqual(backlinks(pages, "a"), []);
  const t = tree(pages);
  assert.deepEqual(t.map(x => x.name), ["A", "B", "Recipes"]);
  assert.equal(t[2].page.key, "d");
  assert.deepEqual(t[2].children.map(x => [x.name, x.page.key]), [["Bread", "c"]]);
});
