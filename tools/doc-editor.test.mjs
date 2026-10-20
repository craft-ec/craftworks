// THE DOC EDITOR's MODEL (packages/doc-editor.js): lines, a line's style, inline runs, and its own copy moved by an
// edit as `codoc`'s marks move (expanding after).
import { test } from "node:test";
import assert from "node:assert/strict";
import { linesOf, blockOf, runsOf, shifted } from "../packages/doc-editor.js";

test("lines, their styles (over the newline too), an empty line's", () => {
  const text = "Title\nbody\n\nend";
  const marks = [{ start: 0, end: 6, name: "block", value: "h1" }, { start: 11, end: 12, name: "block", value: "quote" }];
  const ls = linesOf(text);
  assert.deepEqual(ls, [{ start: 0, end: 5 }, { start: 6, end: 10 }, { start: 11, end: 11 }, { start: 12, end: 15 }]);
  assert.deepEqual(ls.map(l => blockOf(marks, l, text)), ["h1", "", "quote", ""]);
});

test("inline runs split where marks start and end", () => {
  const text = "Hello world";
  const marks = [{ start: 0, end: 5, name: "bold", value: null }, { start: 3, end: 8, name: "italic", value: null }];
  const runs = runsOf(text, marks, { start: 0, end: 11 }).map(r => [r.text, Object.keys(r.marks).sort().join("+")]);
  assert.deepEqual(runs, [["Hel", "bold"], ["lo", "bold+italic"], [" wo", "italic"], ["rld", ""]]);
});

test("an edit moves the marks: after shift, inside grows, at the end extends, at the start does not, deleted gone", () => {
  const m = [{ start: 2, end: 5, name: "bold" }];
  assert.deepEqual(shifted(m, 0, 0, 3), [{ start: 5, end: 8, name: "bold" }], "before: shifted");
  assert.deepEqual(shifted(m, 3, 0, 2), [{ start: 2, end: 7, name: "bold" }], "inside: grows");
  assert.deepEqual(shifted(m, 5, 0, 1), [{ start: 2, end: 6, name: "bold" }], "at its end: extends");
  assert.deepEqual(shifted(m, 2, 0, 1), [{ start: 3, end: 6, name: "bold" }], "at its start: shifted, not grown");
  assert.deepEqual(shifted(m, 1, 6, 0), [], "deleted whole: gone");
  assert.deepEqual(shifted(m, 3, 1, 0), [{ start: 2, end: 4, name: "bold" }], "a part deleted: shrinks");
});
