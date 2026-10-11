// WHAT IS SHOWN IS WHAT IS STORED: the audience control's choices become the item's write rule, "anyone" included
// (10-11, measured on fx: "anyone" was dropped as "the default", so a public form followed its maker's own policy —
// `author` — and refused every answer while the control said "Anyone who sees it").
import test from "node:test";
import assert from "node:assert/strict";
const A = await import(process.env.AUDIENCE_JS ?? new URL("../packages/audience.js", import.meta.url));
const ACTS = ["comment", "vote", "react", "save", "share", "tag"];

test("every option of the one choice is stored for every action — \"anyone\" too", () => {
  for (const one of ["anyone", "followers", "friends", "author", "members", "admins", "owner"]) {
    const w = A.writeRule(ACTS, one);
    assert.deepEqual(w, Object.fromEntries(ACTS.map(a => [a, one])), `one choice ${one}`);
    assert.deepEqual(A.withWrite({ fields: 1 }, w), { fields: 1, write: w }, `${one} reaches the item's meta`);
  }
});
test("a per-action choice overrides the one choice; \"as above\" takes the one", () => {
  const w = A.writeRule(ACTS, "anyone", { comment: "followers", vote: "" });
  assert.equal(w.comment, "followers");
  assert.equal(w.vote, "anyone");
  assert.equal(w.tag, "anyone");
});
test("no choice at all stores nothing (a space's \"who the space allows\" is the space's own rule)", () => {
  assert.equal(A.writeRule(ACTS, ""), null);
  assert.deepEqual(A.withWrite({ fields: 1 }, null), { fields: 1 });
});
