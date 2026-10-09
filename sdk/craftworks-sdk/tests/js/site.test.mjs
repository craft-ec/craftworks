// A SITE (builder#117) through the REAL packaged `Session` (pkg/node), driven the way a page drives it: what
// goes to the node is decoded by the stdlib's own encoding (`wire/examples/node_answers.rs`), never by bytes
// this file built. The site's whole path -- read, sign, PUT, read back -- is `page/tests/site.rs` and
// `page-io/tests/wire_node.rs`; this pins the ENTRY an app uses: the link, the first frame, the status and the
// refusals.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { generateKeyPairSync } from "node:crypto";
import { childEnv } from "./common/child-env.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const { Session } = createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js");

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.stack}\n`); }
};

const hex = b => Buffer.from(b).toString("hex");
const enc = s => new TextEncoder().encode(s);
const SITE_CODE = enc("a site contract's code");

/** The frames `s` would send now, decoded by the stdlib -- and sent. */
const flush = s => {
  const out = s.outbound();
  s.sent(out.length);
  const r = spawnSync("cargo", ["run", "-q", "-p", "wire", "--example", "node_answers", "--",
    hex(enc("c")), hex(enc("p")), hex(enc("s")), "x", ...out.map(hex)], { cwd: root, encoding: "utf8", maxBuffer: 1 << 26 });
  assert.equal(r.status, 0, `node_answers failed: ${r.stderr}`);
  return JSON.parse(r.stdout).frames;
};

/** Everything `node_answers` says for `frames`, with the signer's answer `signerAnswer` encoded. */
function stdlib(frames = [], signerAnswer = null) {
  const r = spawnSync("cargo", ["run", "-q", "-p", "wire", "--example", "node_answers", "--", "00", "00", "00", "x", ...frames.map(hex)],
    { cwd: root, encoding: "utf8", maxBuffer: 1 << 26, env: childEnv({ SIGNER_CODE: hex(enc("signer code")), ...(signerAnswer ? { SIGNER_ANSWER: signerAnswer } : {}) }) });
  assert.equal(r.status, 0, r.stderr);
  return JSON.parse(r.stdout);
}
/** Register params as `wire::register_params` lays them out: RG01, mode 0, the key, the name. */
// A REAL ed25519 public key per `keyByte` (the Register crate's params reader refuses a key that is no
// usable point, sdk#364), the same key each time for one `keyByte`.
const keys = new Map();
const pubKey = keyByte => {
  if (!keys.has(keyByte)) {
    const { publicKey } = generateKeyPairSync("ed25519");
    keys.set(keyByte, Buffer.from(publicKey.export({ format: "jwk" }).x, "base64url"));
  }
  return keys.get(keyByte);
};
const params = keyByte => new Uint8Array([...enc("RG01"), 0, ...pubKey(keyByte), ...enc("head")]);
/** A page whose registration was answered, its Register query not yet. */
const asking = () => {
  const s = new Session(7999);
  s.provision(enc("signer code"), enc("block code"), enc("register code"));
  flush(s);
  s.on_inbound(new Uint8Array(Buffer.from(stdlib().registered, "hex")));
  return s;
};
/** A page whose signer NAMED its Register (`keyByte`'s): the authority a site is relabelled from. */
const provisioned = (keyByte = 7) => {
  const s = asking();
  const out = s.outbound();
  s.sent(out.length);
  const [q] = stdlib(out).frames.flatMap(f => f.signer ?? []);
  s.on_inbound(new Uint8Array(Buffer.from(stdlib([], `register:${q.id}:${hex(params(keyByte))}`).signer_answer, "hex")));
  flush(s);
  return s;
};

// THE SITE'S LINK (builder#117): a site is WRITTEN only through the definition's one publish
// (`publish_definition_site`, §19 P5); its link is the site contract's under this person's Register and the app id.
await t("no page, no link: before there is a path to the node the link is refused by name", async () => {
  assert.throws(() => new Session(7999).site_link("notes", SITE_CODE), /provision first/);
});

await t("before the signer names the Register there is no authority yet: refused as NOT YET, not as a bad app id", async () => {
  assert.throws(() => asking().site_link("notes", SITE_CODE), /not yet: the node's signer has not said/);
});

await t("**the link is the site contract's: one per app per person**", async () => {
  const s = provisioned();
  const link = s.site_link("notes", SITE_CODE);
  assert.equal(s.site_link("notes", SITE_CODE), link, "the same app got another link");
  assert.notEqual(s.site_link("tasks", SITE_CODE), link, "THE CONTROL: another app got the same link");
  assert.notEqual(provisioned(8).site_link("notes", SITE_CODE), link, "THE CONTROL: another person's site got the same link");
});

await t("a label that is no app id has no link, by name", async () => {
  assert.throws(() => provisioned().site_link("Not An App!", SITE_CODE), /not an app id/);
});

if (failures) { process.stdout.write(`${failures} failed\n`); process.exit(1); }
