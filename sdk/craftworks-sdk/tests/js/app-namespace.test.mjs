// THE APP NAMESPACE (the forest ruling), on the REAL wasm Session: a person's
// one tree is divided by app; every name an app uses is RELATIVE to it, so it
// has no name for another app's data and cannot write it. Reading another
// app's data (public by default) is the explicit `@app/name` form `other()`
// uses. The mapping itself is tested natively (tests/app_namespace.rs); an
// app's own reads and writes need a node, so the two-apps-on-one-register run
// is live: examples/notes/acceptance.mjs.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { engineDb } from "../../js/engine-db.js";
import { open } from "../../js/session.js";
import { wrap } from "../../js/wrap.js";

const { Session } = createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js");
let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack ?? JSON.stringify(e)}\n`); }
};
const schema = JSON.stringify({ type: "Note", fields: [{ name: "text", kind: "text", required: true }] });
const refused = (fn, re) => {
  let err; try { fn(); } catch (e) { err = e; }
  assert.ok(err, "not refused");
  assert.equal(err.code, "REFUSED", JSON.stringify(err));
  assert.match(err.message, re);
};
const app = name => { const s = new Session(7999); s.set_app(name); return s; };

await t("**a session with NO app writes nothing — refused by name**", async () => {
  const s = new Session(7999);
  refused(() => s.define("notes", schema), /no app/);
  refused(() => s.put("notes", JSON.stringify({ text: "x" })), /no app/);
});

await t("**another app's data is never WRITTEN: `@app/name` is refused before the store**", async () => {
  const a = app("alpha");
  for (const write of [() => a.put("@beta/notes", "{}"), () => a.define("@beta/notes", schema), () => a.delete("@beta/notes", "00".repeat(24)), () => a.update("@beta/notes", "00".repeat(24), "{}")]) {
    refused(write, /another app's: an app writes only its own/);
  }
});

await t("an app id is set once, and must be one", async () => {
  const s = app("alpha");
  refused(() => s.set_app("beta"), /cannot become `beta`/);
  s.set_app("alpha");
  for (const bad of ["", "has.dot", "UPPER", "a".repeat(33), "@x"]) refused(() => new Session(7999).set_app(bad), /must be 1–32/);
});

await t("**open() with NO app is refused by name, before it connects**", async () => {
  let connected = 0;
  await assert.rejects(() => open(Session, { port: 7999, connect: () => { connected += 1; return { pump() {}, close() {} }; } }),
    /open\(\) needs \{ app \}/);
  await assert.rejects(() => open(Session, { port: 7999, app: "", connect: () => { connected += 1; return { pump() {}, close() {} }; } }),
    /open\(\) needs \{ app \}/);
  assert.equal(connected, 0, "open() connected before refusing a missing app");
});

await t("**db.other(app) READS through `@app/name` and never WRITES — the node-backed db**", async () => {
  const calls = [];
  const session = {
    scan: d => { calls.push(["scan", d]); return "[]"; },
    count: d => { calls.push(["count", d]); return 0; },
    get: (d, id) => { calls.push(["get", d, id]); return "null"; },
    schema: d => { calls.push(["schema", d]); return "null"; },
    children: (d, p) => { calls.push(["children", d, p]); return "[]"; },
    put: d => { calls.push(["put", d]); return "x"; }, create_at: d => { calls.push(["create_at", d]); return "x"; },
    update: d => { calls.push(["update", d]); return "x"; }, delete: d => { calls.push(["delete", d]); return true; },
    define: d => { calls.push(["define", d]); },
    take_loads: () => "[]", pump() {},
  };
  const other = engineDb(session).other("app-x");
  await other.scan("notes");
  await other.count("notes");
  assert.deepEqual(calls.map(c => c.slice(0, 2)), [["scan", "@app-x/notes"], ["count", "@app-x/notes"]], "reads did not take the absolute form");
  for (const w of ["define", "put", "createAt", "update", "delete"]) {
    await assert.rejects(() => other[w]("notes", {}), e => e.code === "REFUSED" && /another app's data/.test(e.message), `${w} was not refused by name`);
  }
  assert.equal(calls.length, 2, `a refused write reached the session: ${JSON.stringify(calls.slice(2))}`);
  // A bad app id is refused where every name is: the session's `app::read`
  // (tests/app_namespace.rs: "@BAD/notes" is refused). The db writes no second
  // copy of the rule; it hands the name over as given.
  calls.length = 0;
  await engineDb(session).other("Has.Dot").scan("notes");
  assert.deepEqual(calls.map(c => c.slice(0, 2)), [["scan", "@Has.Dot/notes"]], "the db decided the app id itself instead of the session");
});

await t("the IN-MEMORY db has the same other(), and says it holds only this app's data — never an empty answer", async () => {
  const raw = createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js");
  const other = new (wrap(raw).Db)().other("app-x");
  for (const m of ["scan", "count", "put"]) {
    await assert.rejects(() => other[m]("notes", {}), e => e.code === "REFUSED" && /holds only this app's data/.test(e.message), `in-memory ${m}`);
  }
});

await t("**the in-tab db holds the SAME name rule as a Session: 32 is a name, 33 is refused naming it** (sdk#276)", async () => {
  const raw = createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js");
  const db = new (wrap(raw).Db)();
  const schema = { type: "Row", fields: [{ name: "n", kind: "int" }] };
  const ok = "n".repeat(32), long = "n".repeat(33);
  await db.define(ok, schema);
  await db.put(ok, { n: 1 });
  assert.equal((await db.scan(ok)).length, 1, "THE CONTROL: a 32-character name writes and scans");
  await assert.rejects(() => db.define(long, schema), e => e.message.includes(`domain \`${long}\` must be 1–32`), "a 33-character name was not refused in the node's words");
  // And a Session with an app at the LONGEST id: its longest name passes the
  // name rule and goes on to the next question -- a session on no node yet is
  // NOT_DECIDED (#342: whether it may write is not known, and it waits), one
  // holding nothing loaded answers NOT_LOADED. Either answer is the proof: a
  // name refusal comes first.
  const s = new Session(7999);
  s.set_app("a".repeat(32));
  const got = (() => { try { s.define(ok, JSON.stringify(schema)); return null; } catch (e) { return e; } })();
  assert.ok(!got || got.code === "NOT_LOADED" || got.code === "NOT_DECIDED", `a 32-character name at a 32-character app id was refused: ${JSON.stringify(got)}`);
  const past = (() => { try { s.define(long, JSON.stringify(schema)); return null; } catch (e) { return e; } })();
  assert.ok(past && /domain `n{33}` must be 1–32/.test(past.message), `a 33-character name was not refused in the node's words: ${JSON.stringify(past)}`);
  assert.ok(!past.message.includes("aaaa."), `the refusal names the PREFIXED name: ${past.message}`);
});

if (failures) { process.stdout.write(`${failures} failed\n`); process.exit(1); }
