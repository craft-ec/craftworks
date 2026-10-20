// THE DECK's SLIDES (packages/slide-deck.js): ordered by place, the gone left out; a place between two.
import { test } from "node:test";
import assert from "node:assert/strict";
import { slidesOf, between } from "../packages/slide-deck.js";

test("slides by place; a gone one left out; fields gathered", () => {
  const cells = { "s:a:pos": "2", "s:a:title": "Second", "s:b:pos": "1", "s:b:title": "First", "s:c:pos": "1.5", "s:c:gone": "1", "deck:theme": "dark" };
  assert.deepEqual(slidesOf(cells).map(s => [s.id, s.title]), [["b", "First"], ["a", "Second"]]);
});

test("a place between neighbours, and at either end", () => {
  assert.equal(between(1, 2), 1.5);
  assert.equal(between(null, 1), 0);
  assert.equal(between(3, null), 4);
  assert.equal(between(null, null), 1);
});
