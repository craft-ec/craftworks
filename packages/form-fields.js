// FORM FIELDS, a capability with no UI and no node: what a FORM's fields ARE and what an answer to them must be —
// pure, so one implementation serves the builder, the respondent's page, the owners' results, and the tests
// (`node --test tools/form-fields.test.mjs`).
//
//   const F = await ctx.require("form-fields");
//   F.TYPES                         // the field types: [{ type, label, choices }]
//   F.normalize(fields)             // a builder's fields cleaned (ids, labels, choices); throws, by name, on one that is not
//   F.validate(fields, answers)     // { ok, errors: { <field id>: why }, clean: { <field id>: value } } — required,
//                                   // types, choices, lengths; `clean` holds only what the fields name
//   F.latestPerAccount(responses)   // one response per `by`: its newest (`edited` ?? `at`)
//   F.summarize(fields, responses)  // per field: { id, type, label, answered, counts?, yes?, no?, min?, max?, mean? }
//   F.toCSV(fields, responses)      // RFC 4180 text (CRLF), one row per response; formula-looking cells made inert
//   F.csvCell(value)                // one cell, escaped
//   F.toSpace(place)                // is it a space's form (responses = its space mail)? else the maker's
//   F.responseTo(form, place)       // whom a response is mailed to: `space:<id>` (a space's form), else the maker
//   F.responseMail(form, place, by, answers)   // the response as a mail: { to, subject, body }
//   F.responseOf(mail, formRef)     // a mail read back as a response, checked (kind, form, sender): or null
// A response: { by, at, edited?, answers: { <field id>: value } } — value: text (short, long, email, date "YYYY-MM-DD"),
// a number (number), a choice (single), choices (multiple), a boolean (checkbox), file references (file).
export const TYPES = Object.freeze([
  { type: "short", label: "Short text" },
  { type: "long", label: "Long text" },
  { type: "number", label: "Number" },
  { type: "email", label: "Email" },
  { type: "date", label: "Date" },
  { type: "single", label: "Single choice", choices: true },
  { type: "multiple", label: "Multiple choice", choices: true },
  { type: "checkbox", label: "Checkbox" },
  { type: "file", label: "File upload" },
]);
const BY_TYPE = new Map(TYPES.map(t => [t.type, t]));
export const LIMITS = Object.freeze({ fields: 100, label: 300, help: 1000, choices: 50, choice: 200, short: 300, long: 20000, files: 10 });

const newId = () => `f${Math.random().toString(36).slice(2, 10)}`;

// THE BUILDER's fields, cleaned: each a known type with a label, ids unique (kept where given: answers name them),
// choices trimmed and without repeats (a choice field needs two).
export function normalize(fields) {
  if (!Array.isArray(fields)) throw new Error("a form's fields are a list");
  if (fields.length > LIMITS.fields) throw new Error(`a form has at most ${LIMITS.fields} fields`);
  const ids = new Set();
  return fields.map((f, i) => {
    const type = String(f?.type ?? "");
    const t = BY_TYPE.get(type);
    if (!t) throw new Error(`field ${i + 1}: no such type “${type}”`);
    const label = String(f?.label ?? "").trim();
    if (!label) throw new Error(`field ${i + 1}: it needs a question`);
    if (label.length > LIMITS.label) throw new Error(`field ${i + 1}: a question of at most ${LIMITS.label} characters`);
    let id = typeof f.id === "string" && /^[A-Za-z0-9_-]{1,40}$/.test(f.id) ? f.id : newId();
    while (ids.has(id)) id = newId();
    ids.add(id);
    const out = { id, type, label, required: !!f.required };
    const help = String(f?.help ?? "").trim();
    if (help) out.help = help.slice(0, LIMITS.help);
    if (t.choices) {
      const choices = [...new Set((Array.isArray(f.choices) ? f.choices : []).map(c => String(c ?? "").trim()).filter(Boolean))];
      if (choices.length < 2) throw new Error(`field ${i + 1} (“${label}”): a choice needs at least two options`);
      if (choices.length > LIMITS.choices) throw new Error(`field ${i + 1} (“${label}”): at most ${LIMITS.choices} options`);
      if (choices.some(c => c.length > LIMITS.choice)) throw new Error(`field ${i + 1} (“${label}”): an option of at most ${LIMITS.choice} characters`);
      out.choices = choices;
    }
    return out;
  });
}

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
const isDate = s => {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s);
  if (!m) return false;
  const d = new Date(Date.UTC(+m[1], +m[2] - 1, +m[3]));
  return d.getUTCFullYear() === +m[1] && d.getUTCMonth() === +m[2] - 1 && d.getUTCDate() === +m[3];
};
// Empty, as an answer: nothing typed, nothing chosen, a box not ticked, no file.
const empty = (type, v) => v == null || (typeof v === "string" && !v.trim()) || (Array.isArray(v) && !v.length) || (type === "checkbox" && v !== true);

// AN ANSWER CHECKED against the fields: every required one answered, each answer of its type. What the fields do not
// name is dropped (never kept, never shown).
export function validate(fields, answers = {}) {
  const errors = {};
  const clean = {};
  for (const f of fields) {
    const v = answers?.[f.id];
    if (empty(f.type, v)) {
      if (f.required) errors[f.id] = f.type === "checkbox" ? "this must be ticked" : "this is required";
      continue;
    }
    switch (f.type) {
      case "short":
      case "long": {
        const s = String(v).trim();
        const max = LIMITS[f.type];
        if (s.length > max) errors[f.id] = `at most ${max} characters`;
        else clean[f.id] = s;
        break;
      }
      case "email": {
        const s = String(v).trim();
        if (!EMAIL.test(s) || s.length > LIMITS.short) errors[f.id] = "not an email address";
        else clean[f.id] = s;
        break;
      }
      case "date": {
        const s = String(v).trim();
        if (!isDate(s)) errors[f.id] = "not a date (YYYY-MM-DD)";
        else clean[f.id] = s;
        break;
      }
      case "number": {
        const s = typeof v === "number" ? v : String(v).trim();
        const n = typeof s === "number" ? s : s === "" ? NaN : Number(s);
        if (!Number.isFinite(n)) errors[f.id] = "not a number";
        else clean[f.id] = n;
        break;
      }
      case "single": {
        const s = String(v);
        if (!f.choices.includes(s)) errors[f.id] = "not one of the options";
        else clean[f.id] = s;
        break;
      }
      case "multiple": {
        const xs = Array.isArray(v) ? [...new Set(v.map(String))] : [String(v)];
        if (xs.some(x => !f.choices.includes(x))) errors[f.id] = "not among the options";
        // In the form's own order, whatever order they were ticked.
        else clean[f.id] = f.choices.filter(c => xs.includes(c));
        break;
      }
      case "checkbox":
        clean[f.id] = true;
        break;
      case "file": {
        const xs = Array.isArray(v) ? v : [v];
        if (xs.length > LIMITS.files) errors[f.id] = `at most ${LIMITS.files} files`;
        else if (xs.some(r => !r || typeof r !== "object" || !r.name)) errors[f.id] = "not a file";
        else clean[f.id] = xs;
        break;
      }
    }
  }
  return { ok: !Object.keys(errors).length, errors, clean };
}

const timeOf = r => Number(r?.edited) || Number(r?.at) || 0;
// ONE PER ACCOUNT: each person's newest response (an edit is newer than what it replaced).
export function latestPerAccount(responses) {
  const by = new Map();
  for (const r of responses) {
    const had = by.get(r.by);
    if (!had || timeOf(r) > timeOf(had)) by.set(r.by, r);
  }
  return [...by.values()];
}

// PER FIELD, over the responses: how many answered it; a choice's count per option (every option listed, zeros too);
// a checkbox's ticked and not; a number's least, most and mean.
export function summarize(fields, responses) {
  return fields.map(f => {
    const vals = responses.map(r => r?.answers?.[f.id]).filter(v => !empty(f.type, v));
    const out = { id: f.id, type: f.type, label: f.label, answered: vals.length, of: responses.length };
    if (f.type === "single" || f.type === "multiple") {
      const counts = Object.fromEntries(f.choices.map(c => [c, 0]));
      for (const v of vals) for (const c of f.type === "single" ? [v] : v) if (c in counts) counts[c] += 1;
      out.counts = counts;
    } else if (f.type === "checkbox") {
      out.yes = vals.length;
      out.no = responses.length - vals.length;
    } else if (f.type === "number") {
      const ns = vals.map(Number).filter(Number.isFinite);
      if (ns.length) {
        out.min = Math.min(...ns);
        out.max = Math.max(...ns);
        out.mean = ns.reduce((a, b) => a + b, 0) / ns.length;
      }
    }
    return out;
  });
}

// ONE CSV CELL (RFC 4180): quoted where it holds a comma, a quote, a line break or edge spaces, its quotes doubled. A
// text a spreadsheet would run as a FORMULA (= + - @, a tab or CR first) is made inert with a leading apostrophe —
// what a respondent typed never runs in the owner's spreadsheet. Numbers stay numbers.
export function csvCell(v) {
  if (v == null) return "";
  if (typeof v === "number") return Number.isFinite(v) ? String(v) : "";
  let s = String(v);
  if (/^[=+\-@\t\r]/.test(s)) s = `'${s}`;
  return /[",\r\n]|^\s|\s$/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}
// An answer as one cell's text: choices joined by "; ", a checkbox "yes", files by name.
export function cellOf(f, v) {
  if (empty(f.type, v)) return f.type === "checkbox" ? "no" : "";
  if (f.type === "multiple") return v.join("; ");
  if (f.type === "checkbox") return "yes";
  if (f.type === "file") return v.map(x => x.name).join("; ");
  if (f.type === "number") return Number(v);
  return v;
}
// THE RESPONSES as CSV: Submitted (ISO time), Respondent (`name(by)`, else the id), then one column per field.
export function toCSV(fields, responses, { name = by => by } = {}) {
  const head = ["Submitted", "Respondent", ...fields.map(f => f.label)];
  const rows = responses.map(r => [new Date(timeOf(r)).toISOString(), name(r.by), ...fields.map(f => cellOf(f, r?.answers?.[f.id]))]);
  return [head, ...rows].map(row => row.map(csvCell).join(",")).join("\r\n") + "\r\n";
}

// A RESPONSE IS A MAIL (`conversation.mail`). TO: the space's own address (`space:<id>`: space mail — its owner and
// admins read it) where the form is in a space, else the form's maker (a personal form; a circle's — friends,
// followers — or another group's is personal too). `place`: the form's space description, or null.
export const RESPONSE = "form-response";
export const toSpace = place => !!place && !place.circle && !place.group;
export function responseTo(form, place) {
  return toSpace(place) ? `space:${place.id}` : form.by;
}
// The mail a response is: { to, subject, body } — the body structured: { kind, form, by, at, answers }.
export function responseMail(form, place, by, answers, at = Date.now()) {
  return { to: [responseTo(form, place)], subject: `🗒 ${form.title || "Untitled form"}`, body: JSON.stringify({ kind: RESPONSE, form: form.ref, by, at, answers }) };
}
// A MAIL read back as a response to the form `ref` (null: to any form), CHECKED: its kind, the form it names, and the
// respondent it names being who SENT it (`mail.from`: proven by whose tail it was in) — a body copied into another's
// mail, or naming another form, is nothing. { ref: the mail's id, form, by, at, answers }, or null.
export function responseOf(mail, ref = null) {
  try {
    const p = JSON.parse(mail?.body);
    if (p?.kind !== RESPONSE || typeof p.form !== "string" || (ref != null && p.form !== ref) || !p.by || p.by !== mail.from) return null;
    return { ref: mail.id, form: p.form, by: p.by, at: mail.at, answers: p.answers ?? {} };
  } catch {
    return null;
  }
}

export async function start() {
  return { TYPES, LIMITS, normalize, validate, latestPerAccount, summarize, csvCell, cellOf, toCSV, RESPONSE, toSpace, responseTo, responseMail, responseOf };
}
