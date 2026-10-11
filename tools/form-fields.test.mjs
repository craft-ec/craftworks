// A FORM's pure logic (`packages/form-fields.js`: fields, answers checked, summaries, CSV) — run with no node and no
// page:   node --test tools/form-fields.test.mjs
// `FORM_FIELDS_JS`: another copy of the file to test (the negative control: one with the CSV quoting or the required
// check taken out fails).
import test from "node:test";
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";

const src = process.env.FORM_FIELDS_JS ?? new URL("../packages/form-fields.js", import.meta.url).href;
const F = await import(src.startsWith("file:") ? src : pathToFileURL(src).href);

const fields = F.normalize([
  { id: "name", type: "short", label: "Name", required: true },
  { id: "mail", type: "email", label: "Email" },
  { id: "age", type: "number", label: "Age" },
  { id: "day", type: "date", label: "Day" },
  { id: "size", type: "single", label: "Size", choices: ["S", "M", "L", " M "], required: true },
  { id: "tops", type: "multiple", label: "Toppings", choices: ["cheese", "ham", "olive"] },
  { id: "ok", type: "checkbox", label: "I agree", required: true },
  { id: "cv", type: "file", label: "CV" },
  { id: "note", type: "long", label: "Notes" },
]);

test("normalize: choices trimmed and deduplicated, ids kept, bad fields refused by name", () => {
  assert.deepEqual(fields.find(f => f.id === "size").choices, ["S", "M", "L"]);
  assert.deepEqual(fields.map(f => f.id), ["name", "mail", "age", "day", "size", "tops", "ok", "cv", "note"]);
  assert.throws(() => F.normalize([{ type: "single", label: "One", choices: ["a"] }]), /at least two options/);
  assert.throws(() => F.normalize([{ type: "nope", label: "x" }]), /no such type/);
  assert.throws(() => F.normalize([{ type: "short", label: "  " }]), /needs a question/);
  const two = F.normalize([{ id: "a", type: "short", label: "A" }, { id: "a", type: "short", label: "B" }]);
  assert.notEqual(two[0].id, two[1].id);
});

const good = { name: " Ann ", mail: "ann@example.org", age: "41", day: "2026-02-28", size: "M", tops: ["olive", "cheese"], ok: true, cv: [{ name: "cv.pdf", key: "k" }], extra: "dropped" };

test("validate: a good answer is cleaned (trimmed, typed, ordered) and what no field names is dropped", () => {
  const v = F.validate(fields, good);
  assert.equal(v.ok, true, JSON.stringify(v.errors));
  assert.equal(v.clean.name, "Ann");
  assert.equal(v.clean.age, 41);
  assert.deepEqual(v.clean.tops, ["cheese", "olive"]);
  assert.equal("extra" in v.clean, false);
  assert.equal("note" in v.clean, false);
});

test("validate: required fields missing are each named (an unticked required box too)", () => {
  const v = F.validate(fields, { name: "  ", size: "", ok: false });
  assert.equal(v.ok, false);
  assert.deepEqual(Object.keys(v.errors).sort(), ["name", "ok", "size"]);
  assert.match(v.errors.ok, /ticked/);
});

test("validate: wrong types refused", () => {
  const v = F.validate(fields, { ...good, mail: "ann at example", age: "forty", day: "2026-02-30", size: "XL", tops: ["cheese", "pineapple"], cv: [{}] });
  assert.deepEqual(Object.keys(v.errors).sort(), ["age", "cv", "day", "mail", "size", "tops"]);
});

test("validate: an empty number is not zero, and an optional empty field is simply absent", () => {
  const v = F.validate(fields, { ...good, age: "" });
  assert.equal(v.ok, true);
  assert.equal("age" in v.clean, false);
});

const responses = [
  { by: "did:a", at: 1000, answers: { name: "A", size: "S", tops: ["cheese"], ok: true, age: 20 } },
  { by: "did:b", at: 2000, answers: { name: "B", size: "M", tops: ["cheese", "ham"], ok: true, age: 40 } },
  { by: "did:a", at: 1500, edited: 3000, answers: { name: "A2", size: "L", tops: [], ok: true } },
];

test("one per account: each person's newest (an edit counts as newer)", () => {
  const one = F.latestPerAccount(responses);
  assert.equal(one.length, 2);
  assert.equal(one.find(r => r.by === "did:a").answers.name, "A2");
});

test("summaries: counts per option (zeros listed), checkbox yes/no, number min/max/mean", () => {
  const s = Object.fromEntries(F.summarize(fields, responses).map(x => [x.id, x]));
  assert.deepEqual(s.size.counts, { S: 1, M: 1, L: 1 });
  assert.deepEqual(s.tops.counts, { cheese: 2, ham: 1, olive: 0 });
  assert.equal(s.tops.answered, 2);
  assert.equal(s.ok.yes, 3);
  assert.equal(s.ok.no, 0);
  assert.equal(s.age.min, 20);
  assert.equal(s.age.max, 40);
  assert.equal(s.age.mean, 30);
  assert.equal(s.mail.answered, 0);
});

test("CSV: commas, quotes, line breaks and edge spaces quoted; quotes doubled", () => {
  assert.equal(F.csvCell("plain"), "plain");
  assert.equal(F.csvCell("a,b"), '"a,b"');
  assert.equal(F.csvCell('say "hi"'), '"say ""hi"""');
  assert.equal(F.csvCell("two\nlines"), '"two\nlines"');
  assert.equal(F.csvCell("cr\rhere"), '"cr\rhere"');
  assert.equal(F.csvCell(" padded"), '" padded"');
  assert.equal(F.csvCell(null), "");
  assert.equal(F.csvCell(-3), "-3");
});

test("CSV: a formula-looking text is made inert", () => {
  assert.equal(F.csvCell("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
  assert.equal(F.csvCell("+1"), "'+1");
  assert.equal(F.csvCell("@SUM(A1)"), "'@SUM(A1)");
  assert.equal(F.csvCell("-2+3"), "'-2+3");
});

test("CSV: a whole export parses back to the same cells", () => {
  const rs = [{ by: "did:x", at: Date.UTC(2026, 0, 2), answers: { name: 'Mo, "the" one', size: "S", tops: ["cheese", "ham"], ok: true, note: "line1\r\nline2", cv: [{ name: "a.pdf" }, { name: "b,c.png" }] } }];
  const csv = F.toCSV(fields, rs, { name: d => `@${d}` });
  // A tiny RFC 4180 reader (quoted fields, doubled quotes, CRLF rows).
  const parse = text => {
    const rows = [[]];
    let cell = "";
    let q = false;
    for (let i = 0; i < text.length; i++) {
      const c = text[i];
      if (q) {
        if (c === '"' && text[i + 1] === '"') (cell += '"'), i++;
        else if (c === '"') q = false;
        else cell += c;
      } else if (c === '"') q = true;
      else if (c === ",") rows.at(-1).push(cell), (cell = "");
      else if (c === "\r" && text[i + 1] === "\n") rows.at(-1).push(cell), (cell = ""), rows.push([]), i++;
      else cell += c;
    }
    return rows.filter(r => r.length);
  };
  const [head, row] = parse(csv);
  assert.deepEqual(head, ["Submitted", "Respondent", "Name", "Email", "Age", "Day", "Size", "Toppings", "I agree", "CV", "Notes"]);
  assert.equal(row.length, head.length);
  assert.equal(row[0], "2026-01-02T00:00:00.000Z");
  assert.equal(row[1], "'@did:x");
  assert.equal(row[2], 'Mo, "the" one');
  assert.equal(row[7], "cheese; ham");
  assert.equal(row[8], "yes");
  assert.equal(row[9], "a.pdf; b,c.png");
  assert.equal(row[10], "line1\r\nline2");
  assert.ok(csv.endsWith("\r\n"));
});
