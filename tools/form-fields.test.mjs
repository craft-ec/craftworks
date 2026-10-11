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

// A RESPONSE IS A MAIL. A stand-in for `conversation.mail` as Forms uses it: a mail is filed in each address's "in"
// (a person's DID, or `space:<id>` — space mail, its owner and admins' to read) and in its sender's "sent"; `from` is
// who sent it (the real mail proves it by whose tail it was in).
function standInMail() {
  const boxes = new Map();
  const file = (addr, which, m) => {
    if (!boxes.has(addr)) boxes.set(addr, { in: [], sent: [] });
    boxes.get(addr)[which].push(m);
  };
  let n = 0;
  return {
    send(from, to, subject, body) {
      const m = { id: `m${n++}`, from, to, subject, body, at: 1000 + n };
      for (const d of to) file(d, "in", m);
      file(from, "sent", m);
      return m;
    },
    list: (addr, which) => [...(boxes.get(addr)?.[which] ?? [])],
  };
}
const maker = "did:craftec:maker";
const spaceDesc = { id: "S1", name: "Club" };
const spaceForm = { ref: "space:S1/f1", by: maker, title: "Sign-up" };
const myForm = { ref: `${maker}/f2`, by: maker, title: "Survey" };
const circleForm = { ref: "space:C1/f3", by: maker, title: "Friends only" };

test("a response is addressed to the SPACE for a space form, to the MAKER for a personal (or circle) form", () => {
  assert.deepEqual(F.responseMail(spaceForm, spaceDesc, "did:craftec:ann", {}).to, ["space:S1"]);
  assert.deepEqual(F.responseMail(myForm, null, "did:craftec:ann", {}).to, [maker]);
  assert.deepEqual(F.responseMail(circleForm, { id: "C1", circle: "friends" }, "did:craftec:ann", {}).to, [maker]);
  assert.deepEqual(F.responseMail(circleForm, { id: "C1", group: "audience:S1/admins" }, "did:craftec:ann", {}).to, [maker]);
  // Through the stand-in mail: the space's responses are in its mailbox, not the maker's own; results read them back.
  const mail = standInMail();
  for (const [form, place, by] of [[spaceForm, spaceDesc, "did:craftec:ann"], [spaceForm, spaceDesc, "did:craftec:bo"], [myForm, null, "did:craftec:ann"]]) {
    const x = F.responseMail(form, place, by, { q: by });
    mail.send(by, x.to, x.subject, x.body);
  }
  const of = (addr, form) => mail.list(addr, "in").map(m => F.responseOf(m, form.ref)).filter(Boolean);
  assert.deepEqual(of("space:S1", spaceForm).map(r => r.by), ["did:craftec:ann", "did:craftec:bo"]);
  assert.deepEqual(of(maker, spaceForm), []);
  assert.deepEqual(of(maker, myForm).map(r => r.answers), [{ q: "did:craftec:ann" }]);
  assert.deepEqual(of("space:S1", myForm), []);
  // The respondent's own copy is their Sent mail.
  assert.deepEqual(mail.list("did:craftec:ann", "sent").map(m => F.responseOf(m)?.form), [spaceForm.ref, myForm.ref]);
});

test("a response mail is CHECKED: the form it names, and its respondent being who sent it", () => {
  const x = F.responseMail(spaceForm, spaceDesc, "did:craftec:ann", { q: 1 });
  const m = { id: "m1", from: "did:craftec:ann", body: x.body, at: 5 };
  assert.deepEqual(F.responseOf(m, spaceForm.ref), { ref: "m1", form: spaceForm.ref, by: "did:craftec:ann", at: 5, answers: { q: 1 }, cred: null });
  // Ann's body forwarded by Eve: it names Ann, Eve sent it — nothing.
  assert.equal(F.responseOf({ ...m, from: "did:craftec:eve" }, spaceForm.ref), null);
  assert.equal(F.responseOf(m, myForm.ref), null, "another form's");
  assert.equal(F.responseOf({ ...m, body: JSON.stringify({ kind: "note", form: spaceForm.ref, by: "did:craftec:ann" }) }, spaceForm.ref), null, "not a response");
  assert.equal(F.responseOf({ ...m, body: "plain mail" }, spaceForm.ref), null);
});

test("a response counts only where it was ADDRESSED: the form's own mailbox", () => {
  const x = F.responseMail(spaceForm, spaceDesc, "did:craftec:ann", { q: 1 });
  const to = F.responseTo(spaceForm, spaceDesc);
  const m = { id: "m2", from: "did:craftec:ann", to: x.to, body: x.body, at: 6 };
  assert.ok(F.responseOf(m, spaceForm.ref, to), "addressed to the space's mailbox: counted");
  // The same body mailed to someone else (and pointed at this mailbox): not a response to this form.
  assert.equal(F.responseOf({ ...m, to: ["did:craftec:eve"] }, spaceForm.ref, to), null);
  assert.equal(F.responseOf({ ...m, to: undefined }, spaceForm.ref, to), null);
  // A circle's credential travels with the answer (what the form's rule checks it by).
  const y = F.responseMail(myForm, null, "did:craftec:ann", { q: 2 }, 7, "ab".repeat(16));
  assert.equal(F.responseOf({ id: "m3", from: "did:craftec:ann", to: y.to, body: y.body, at: 7 }, myForm.ref).cred, "ab".repeat(16));
});
