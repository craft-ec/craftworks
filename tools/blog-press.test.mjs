// BLOG's pure logic (`packages/blog-press.js`: reading time, the contents from headings, a schedule's due check, the
// editors' word, the cover) — run with no node and no page:   node --test tools/blog-press.test.mjs
// `BLOG_PRESS_JS`: another copy of the file to test (the negative controls: one whose contents do not skip fenced code
// fails "headings inside fenced code are not contents"; one counting a non-editor's ✅ fails "…once an editor approves").
import test from "node:test";
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";

const src = process.env.BLOG_PRESS_JS ?? new URL("../packages/blog-press.js", import.meta.url).href;
const { readingTime, headingsOf, isDue, approvalOf, coverOf, WPM } = await import(src.startsWith("file:") ? src : pathToFileURL(src).href);

test("reading time: words at WPM a minute, rounded up, at least a minute; nothing to read is 0", () => {
  assert.deepEqual(readingTime(""), { words: 0, minutes: 0 });
  assert.deepEqual(readingTime("one two three"), { words: 3, minutes: 1 });
  const long = Array.from({ length: WPM * 2 + 1 }, (_, i) => `w${i}`).join(" ");
  assert.deepEqual(readingTime(long), { words: WPM * 2 + 1, minutes: 3 });
});

test("reading time: code, images, embeds and link targets are not words; link text is; CJK counts per character", () => {
  const body = "Read [the docs](https://example.org/a/b/c) now.\n\n```\nlots of code words here\n```\n![cover](file:abc)\n![post](item:space:x/y)\nhttps://bare.example/path\n日本語";
  assert.equal(readingTime(body).words, 4 + 3);
  assert.equal(readingTime("don't stop-gap it’s").words, 3);
});

test("contents: headings in order, levels, text without markup, unique slugs", () => {
  const body = "# Intro\ntext\n## **Bold** and `code`\n### [A link](https://x.y) ##\n## Intro\n#NoSpace\n####### seven";
  assert.deepEqual(headingsOf(body), [
    { level: 1, text: "Intro", slug: "intro" },
    { level: 2, text: "Bold and code", slug: "bold-and-code" },
    { level: 3, text: "A link", slug: "a-link" },
    { level: 2, text: "Intro", slug: "intro-2" },
  ]);
});

test("headings inside fenced code are not contents", () => {
  const body = "# Real\n```sh\n# a shell comment\n## not a heading\n```\n## After";
  assert.deepEqual(headingsOf(body).map(h => h.text), ["Real", "After"]);
});

test("contents: a quote or a list item starting with # is not a heading; a heading of symbols only gets a slug", () => {
  assert.deepEqual(headingsOf("> # quoted\n- # listed\n# !!!").map(h => [h.text, h.slug]), [["!!!", "section"]]);
});

test("due: a draft with a time now or past; not before, not without a time, not one in review, not a published post", () => {
  const now = 1_000_000;
  const d = (blog, extra = {}) => ({ meta: { blog }, ...extra });
  assert.equal(isDue(d({ draft: true, due: now }), now), true);
  assert.equal(isDue(d({ draft: true, due: now - 1 }), now), true);
  assert.equal(isDue(d({ draft: true, due: now + 1 }), now), false);
  assert.equal(isDue(d({ draft: true, due: null }), now), false);
  assert.equal(isDue(d({ draft: true, due: now - 1, review: { ref: "space:x/t1", at: 1 } }), now), false);
  assert.equal(isDue(d({ from: "t1", due: now - 1 }), now), false);
  assert.equal(isDue({ meta: {} }, now), false);
});

test("the editors' word: the latest approve or decline by an editor, nested replies included; others' count for nothing", () => {
  const editors = new Set(["ed1", "ed2"]);
  const may = did => editors.has(did);
  const c = (by, at, body, replies = []) => ({ by, at, body, replies });
  assert.equal(approvalOf([c("member", 5, "✅ Approved, by me")], may), null);
  assert.deepEqual(approvalOf([c("ed1", 1, "❌ Declined — too short"), c("x", 2, "fixed", [c("ed2", 3, "✅ Approved")])], may), { word: "approved", by: "ed2", at: 3 });
  assert.deepEqual(approvalOf([c("ed1", 4, "❌ Declined"), c("ed2", 3, "✅ Approved")], may), { word: "declined", by: "ed1", at: 4 });
  assert.equal(approvalOf([c("ed1", 1, "Looks good, ✅ Approved")], may), null);
  assert.equal(approvalOf([], may), null);
});

test("cover: the chosen file, else the first image not shown in the text, else none", () => {
  const keyOf = f => f.key;
  const kindOf = f => (f.type.startsWith("image/") ? "image" : "file");
  const files = [{ key: "a", type: "application/pdf" }, { key: "b", type: "image/png" }, { key: "c", type: "image/jpeg" }];
  assert.equal(coverOf({ files, meta: { cover: "c" } }, { keyOf, kindOf }).key, "c");
  assert.equal(coverOf({ files, meta: {} }, { keyOf, kindOf }).key, "b");
  assert.equal(coverOf({ files, meta: {} }, { keyOf, kindOf, inlined: new Set(["b"]) }).key, "c");
  assert.equal(coverOf({ files: [files[0]], meta: {} }, { keyOf, kindOf }), null);
});

// THE PRESS over a stand-in `items` (what it calls, recorded): a draft for a publication whose editors approve goes to
// them as a members-only review copy; an editor's approval publishes it public and takes the copy and the draft out.
const { start } = await import(src.startsWith("file:") ? src : pathToFileURL(src).href);
async function pressWith({ approval = "editors", editors = ["did:ed"] } = {}) {
  const calls = [];
  const sp = { id: "s1", name: "Pub" };
  const store = new Map();
  let n = 0;
  const items = {
    inPlaces: async () => [...store.values()].filter(x => x.by === "did:me"),
    list: async () => [...store.values()].filter(x => x.board?.id === sp.id),
    boardOf: async id => (String(id).replace(/^space:/, "").startsWith(sp.id) ? sp : null),
    publicIn: async () => true,
    submit: async o => {
      calls.push(["submit", o.board ?? null, o.audience, o.meta?.blog]);
      const id = `t${++n}`;
      const ref = o.board ? `space:${o.board}/${id}` : `did:me/${id}`;
      store.set(ref, { ref, id, by: "did:me", kind: "post", title: o.title, body: o.body, files: o.files ?? [], meta: o.meta, private: o.audience === "private", board: o.board ? sp : null, at: Date.now() });
      return ref;
    },
    editItem: async (ref, body, o) => (calls.push(["edit", ref, o.meta?.blog]), Object.assign(store.get(ref), { body, ...o })),
    remove: async ref => (calls.push(["remove", ref]), store.delete(ref)),
    get: async ref => store.get(ref) ?? null,
    thread: async () => comments,
    comment: async () => {},
  };
  let comments = [];
  const r = { settled: Promise.resolve(), config: (app, key, d) => (app === "blog" && key === "approval" ? approval : d), can: (did, what) => what === "moderate" && editors.includes(did) };
  const ctx = { log: () => {}, require: async name => ({ items, space: { account: async () => ({ id: "did:me" }) }, roles: { of: async () => r }, files: { publicity: async () => {} } })[name] };
  const press = await start(ctx);
  return { press, calls, store, items, setComments: c => (comments = c) };
}

test("a publication's draft goes to its editors, and is published once an editor approves", async () => {
  const { press, calls, store, items, setComments } = await pressWith();
  const ref = await press.saveDraft({ title: "Hello", body: "# A\ntext", place: "s1", audience: "public" });
  const d = await items.get(ref);
  const sent = await press.publish(d);
  assert.equal(sent.review, true);
  assert.deepEqual(calls.find(c => c[0] === "submit" && c[1] === "s1").slice(2), ["members", { from: d.id, review: "pending" }]);
  // A member's ✅ is nothing; nothing published yet.
  setComments([{ by: "did:member", at: 1, body: "✅ Approved" }]);
  await press.publishDue();
  assert.equal([...store.values()].filter(x => x.board && !x.meta?.blog?.review).length, 0);
  // An editor's ✅: the public copy made, the review copy and the draft taken out.
  const { press: p2, calls: c2, store: s2, items: i2, setComments: set2 } = await pressWith();
  const r2 = await p2.saveDraft({ title: "Hello", body: "x", place: "s1", audience: "public" });
  await p2.publish(await i2.get(r2));
  set2([{ by: "did:ed", at: 2, body: "✅ Approved" }]);
  await p2.publishDue();
  const pub = [...s2.values()].filter(x => x.board && !x.meta?.blog?.review);
  assert.equal(pub.length, 1);
  assert.equal(c2.filter(c => c[0] === "submit").at(-1)[2], "public");
  assert.equal(s2.has(r2), false);
});

test("an editor's draft, or one in a publication without approval, is published at once; a due one by publishDue", async () => {
  const { press, items, store } = await pressWith({ approval: "" });
  const ref = await press.saveDraft({ title: "Now", body: "x", place: "s1", audience: "public", due: Date.now() - 1 });
  await press.publishDue();
  const pub = [...store.values()].filter(x => x.board && !x.private);
  assert.equal(pub.length, 1);
  assert.equal(pub[0].meta.blog.from, ref.slice(ref.lastIndexOf("/") + 1));
  assert.equal(await items.get(ref), null);
});
