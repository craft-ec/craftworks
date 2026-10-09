// A VIEW OF SOMEBODY'S PUBLISHED HEAD (sdk#239), through the REAL `Session`.
//
// Published data is readable by default; writing is access control, which a
// reader does not have. So a view reads the named head and its blocks, and:
//   - installs NOTHING on the node it reads from: no delegate registered, no
//     signer message, no PUT, no UPDATE — only GETs (main's addition);
//   - refuses every write before it reaches the store — the safety net under
//     a runtime that renders a view with no inputs.
// The frames are decoded by the stdlib (`wire/examples/node_answers.rs`).
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const { Session } = createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js");

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack ?? JSON.stringify(e)}\n`); }
};
const hex = b => Buffer.from(b).toString("hex");
const BLOCK = new TextEncoder().encode("block code");
const HEAD = "ab".repeat(32);

/** The frames waiting to go to the node, decoded by the stdlib, and sent. */
function sent(s) {
  const out = s.outbound();
  s.sent(out.length);
  const r = spawnSync("cargo", ["run", "-q", "-p", "wire", "--example", "node_answers", "--", "00", "00", "00", "x", ...out.map(hex)],
    { cwd: root, encoding: "utf8", maxBuffer: 1 << 26 });
  assert.equal(r.status, 0, r.stderr);
  return JSON.parse(r.stdout).frames;
}
const refusedAsReadOnly = fn => {
  let err;
  try { fn(); } catch (e) { err = e; }
  assert.ok(err, "the write was not refused");
  assert.equal(err.code, "REFUSED", JSON.stringify(err));
  assert.match(err.message, /^read-only: /);
};

// A view carries its app, as `tree()` gives it one: whose data a write names
// is decided before whether this session may write (a name no app owns is
// refused by name first).
const view = () => { const s = new Session(7999); s.set_app("notes-app"); s.open_named(BLOCK, HEAD, 1, 0); return s; };

await t("**a view may write nothing -- the ONE decision says no -- and stands on the NAMED head**", async () => {
  const s = view();
  for (const head of ["", HEAD]) {
    const w = JSON.parse(s.can_write(head));
    assert.equal(w.answer, "no", `a view may write ${head || "its own tree"}: ${JSON.stringify(w)}`);
    assert.match(w.why, /^read-only: /);
  }
  assert.equal(s.head_id(), HEAD, "the view does not stand on the head it was given");
  // THE CONTROL: a session opening its own tree may write it -- the "no"
  // above is the view's, not the decision's only answer.
  const own = new Session(7999);
  own.provision(new TextEncoder().encode("signer code"), BLOCK, new TextEncoder().encode("register"));
  assert.equal(JSON.parse(own.can_write("")).answer, "yes", "THE CONTROL: an opening session may not write its own tree");
  assert.equal(JSON.parse(new Session(7999).can_write("")).answer, "unknown", "a session with no page was decided");
});

await t("**a view installs NOTHING on the node: its only frames are GETs, the head read with a subscription**", async () => {
  const s = view();
  s.provision(new TextEncoder().encode("signer code"), BLOCK, new TextEncoder().encode("register")); // refused, never sent
  const frames = sent(s);
  assert.ok(frames.length > 0, "THE CONTROL: the view sent nothing at all, so 'only GETs' would be vacuous");
  assert.deepEqual([...new Set(frames.map(f => f.op))], ["get"], `a view sent ${JSON.stringify(frames)}`);
  assert.ok(frames.some(f => f.subscribe), "the head was not read with a subscription (F55)");
  assert.match(s.take_unusable(), /read-only: .*provisioning refused/);
});

await t("THE CONTROL: an ordinary session DOES register with the node — the check above can see it", async () => {
  const s = new Session(7999);
  s.provision(new TextEncoder().encode("signer code"), BLOCK, new TextEncoder().encode("register"));
  assert.ok(sent(s).some(f => f.op === "delegate"), "provisioning sent no delegate op");
});

await t("**every write on a view is refused before it reaches the store**", async () => {
  const s = view();
  sent(s);
  refusedAsReadOnly(() => s.put("notes", JSON.stringify({ text: "x" })));
  refusedAsReadOnly(() => s.create_at("notes", "00".repeat(16), JSON.stringify({ text: "x" })));
  refusedAsReadOnly(() => s.update("notes", "00".repeat(24), JSON.stringify({ text: "y" })));
  refusedAsReadOnly(() => s.delete("notes", "00".repeat(24)));
  assert.equal(s.unsaved_writes(), 0, "a refused write was queued anyway");
  assert.deepEqual(sent(s).filter(f => f.op !== "get"), [], "a refused write reached the node");
});

await t("a published seq that is not a whole number is refused by name (sdk#349)", async () => {
  for (const seq of [-1, 1.5, NaN, 2 ** 60]) {
    assert.throws(() => new Session(7999).open_named(BLOCK, HEAD, 1, seq), /published seq is a whole number/, `seq ${seq} was taken`);
  }
  // THE CONTROL: a whole number opens, and so does 0 (no floor).
  for (const seq of [0, 3]) new Session(7999).open_named(BLOCK, HEAD, 1, seq);
});

await t("a malformed head, and a view over a session already on its own head, are refused", async () => {
  assert.throws(() => new Session(7999).open_named(BLOCK, "abc", 1, 0), /64 hex/);
  assert.throws(() => new Session(7999).open_named(BLOCK, "zz".repeat(32), 1, 0), /64 hex/);
  const s = new Session(7999);
  s.provision(new TextEncoder().encode("signer code"), BLOCK, new TextEncoder().encode("register"));
  assert.throws(() => s.open_named(BLOCK, HEAD, 1, 0), /already open/);
});

if (failures) { process.stdout.write(`${failures} failed\n`); process.exit(1); }
