// A RELOAD IS THE SAME PERSON (the switch-over blocker, sdk#234), on the REAL
// wasm Session in page mode: it ASKS the signer which Register it signs for
// before anything is minted — and mints only when told "none". The frames are
// decoded, and the signer's answers encoded, by the stdlib and signer-proto
// themselves (`wire/examples/node_answers.rs`).
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { childEnv } from "./common/child-env.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const { Session } = createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js");
let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack}\n`); }
};
const enc = s => new TextEncoder().encode(s);
const hex = b => Buffer.from(b).toString("hex");
function stdlib(frames = [], signerAnswer = null) {
  const r = spawnSync("cargo", ["run", "-q", "-p", "wire", "--example", "node_answers", "--", "00", "00", "00", "x", ...frames.map(hex)],
    { cwd: root, encoding: "utf8", maxBuffer: 1 << 26, env: childEnv({ SIGNER_CODE: hex(enc("signer code")), ...(signerAnswer ? { SIGNER_ANSWER: signerAnswer } : {}) }) });
  assert.equal(r.status, 0, r.stderr);
  return JSON.parse(r.stdout);
}
/** The signer requests a session sends now, by name, with their ids. */
const signerRequests = s => { const out = s.outbound(); s.sent(out.length); return stdlib(out).frames.flatMap(f => f.signer ?? []); };
/** Register params as `wire::register_params` lays them out: RG01, a version byte, the key, the name. */
const params = keyByte => new Uint8Array([...enc("RG01"), 0, ...new Array(32).fill(keyByte), ...enc("head")]);
/** The frames a session sends now, decoded, and given up. */
const sentNow = s => { const out = s.outbound(); s.sent(out.length); return stdlib(out).frames; };
/** A page provisioning, its registration ANSWERED as the node answers it
 * (an empty DelegateResponse naming the signer): only then does it ask. */
const page = () => {
  const s = new Session(7999);
  s.provision(enc("signer code"), enc("block code"), enc("register code"));
  sentNow(s);
  s.on_inbound(new Uint8Array(Buffer.from(stdlib().registered, "hex")));
  return s;
};
/** Several turns of the page's clock and pump; then what it had to call unusable. */
const settled = s => { for (let i = 0; i < 4; i += 1) { s.tick(); s.sent(s.outbound().length); } return JSON.parse(s.take_unusable()); };
const answer = (s, id, p) => { s.on_inbound(new Uint8Array(Buffer.from(stdlib([], `register:${id}:${p ? hex(p) : "none"}`).signer_answer, "hex"))); };

await t("**a page ASKS the signer which Register first — nothing is minted or provisioned yet**", async () => {
  const reqs = signerRequests(page());
  assert.deepEqual(reqs.map(r => r.req), ["Register"], `a page opened with ${JSON.stringify(reqs)}`);
});

await t("**the signer's registration goes ALONE: its first request waits for the registration's answer (#260, measured)**", async () => {
  const s = new Session(7999);
  s.provision(enc("signer code"), enc("block code"), enc("register code"));
  const first = sentNow(s);
  assert.deepEqual(first.map(f => f.op), ["delegate"], `the first flush: ${JSON.stringify(first)}`);
  assert.deepEqual(first.flatMap(f => f.signer ?? []), [], "a signer request went out with the registration, before it was answered");
  s.on_inbound(new Uint8Array(Buffer.from(stdlib().registered, "hex")));
  assert.deepEqual(signerRequests(s).map(r => r.req), ["Register"], "the registration's answer did not release the Register query");
});

await t("**the signer names a Register: the page opens IT, provisions nothing, and two pages told the same stand on the same head**", async () => {
  const heads = [];
  for (const _ of ["the reload", "a second tab"]) {
    const s = page();
    const [q] = signerRequests(s);
    answer(s, q.id, params(7));
    assert.deepEqual(signerRequests(s).filter(r => r.req === "Provision"), [], "a key was provisioned over the signer's own");
    assert.equal(s.provisioned(), true, "the named Register was not opened");
    // NOTHING UNUSABLE after several pumps: a page that kept trying to mint
    // (and was stopped a layer down) would report itself unusable for ever.
    assert.deepEqual(settled(s), [], `${_} is unusable`);
    heads.push(s.head_id());
  }
  assert.ok(heads[0].length === 64 && heads[0] === heads[1], `the reload and the second tab stand on ${heads}`);
  const other = page();
  answer(other, signerRequests(other)[0].id, params(8));
  assert.notEqual(other.head_id(), heads[0], "THE CONTROL: another Register gave the same head");
});

await t("**the signer holds NO key: only then is one minted and provisioned**", async () => {
  const s = page();
  const [q] = signerRequests(s);
  answer(s, q.id, null);
  assert.deepEqual(signerRequests(s).map(r => r.req), ["Provision"], "a first page did not provision a new key");
  assert.deepEqual(settled(s), [], "a first page that minted once is unusable");
  assert.deepEqual(signerRequests(s).filter(r => r.req === "Provision"), [], "a second key was provisioned");
});

await t("**opening REFUSED by the signer ends by name: Session.refused() says so, in the signer's words**", async () => {
  const s = page();
  const [q] = signerRequests(s);
  assert.equal(s.refused(), "", "THE CONTROL: refused before anything was answered");
  s.on_inbound(new Uint8Array(Buffer.from(stdlib([], `refused:${q.id}:`).signer_answer, "hex")));
  assert.match(s.refused(), /KeyAlreadyProvisioned/, "the signer's refusal did not reach Session.refused()");
  assert.equal(s.provisioned(), false);
});

if (failures) { process.stdout.write(`${failures} failed\n`); process.exit(1); }
