// THE SHEET's FORMULAS (packages/sheet-grid.js): refs, ranges, functions, precedence, errors, cycles.
import { test } from "node:test";
import assert from "node:assert/strict";
import { evaluate, shown, parseRef } from "../packages/sheet-grid.js";

const cells = o => Object.fromEntries(Object.entries(o).map(([k, v]) => [`c:${k}`, v]));

test("values, refs, precedence, functions over ranges", () => {
  const c = cells({ A1: "2", A2: "3", A3: "5", B1: "=A1*A2+A3", B2: "=SUM(A1:A3)", B3: "=AVERAGE(A1:A3)", B4: "=(A1+A2)^2/5", B5: "=MAX(A1:A3)-MIN(A1:A3)", B6: "=COUNT(A1:B5)", C1: "hello" });
  assert.equal(evaluate(c, "B1"), 11);
  assert.equal(evaluate(c, "B2"), 10);
  assert.equal(evaluate(c, "B3"), 10 / 3);
  assert.equal(evaluate(c, "B4"), 5);
  assert.equal(evaluate(c, "B5"), 3);
  assert.equal(evaluate(c, "B6"), 8);
  assert.equal(evaluate(c, "C1"), "hello");
  assert.equal(evaluate(c, "D9"), "");
});

test("errors: a cycle, a division by zero, text in arithmetic, what does not parse", () => {
  const c = cells({ A1: "=B1", B1: "=A1", A2: "=1/0", A3: "=C1+1", C1: "word", A4: "=2+*3", A5: "=A2+1" });
  assert.equal(evaluate(c, "A1"), "#CYCLE");
  assert.equal(evaluate(c, "A2"), "#DIV/0");
  assert.equal(evaluate(c, "A3"), "#VALUE");
  assert.equal(evaluate(c, "A4"), "#ERR");
  assert.equal(evaluate(c, "A5"), "#DIV/0", "an error flows through");
});

test("number formats and refs", () => {
  assert.equal(shown(0.256, "pct"), "25.6%");
  assert.equal(shown(0.1 + 0.2), "0.3");
  assert.deepEqual(parseRef("c12"), { r: 11, c: 2 });
  assert.equal(parseRef("AA1"), null);
});
