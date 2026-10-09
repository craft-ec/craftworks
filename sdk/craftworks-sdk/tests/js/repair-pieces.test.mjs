// REPAIR AFTER LOAD (sdk#347): a loader that rebuilt the SDK PUTs back exactly the pieces it asked and did not get,
// each re-derived through the SDK wasm (`load_piece`) and checked against the manifest the NATIVE `load-pieces` tool
// wrote (a second path to the same bytes): its sha256 and its container's address. Nothing else is PUT.
import assert from "node:assert/strict";
import { webcrypto } from "node:crypto";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createRequire } from "node:module";
import { repairPieces } from "../../js/pieces.js";
import { wrap } from "../../js/wrap.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};
const subtle = webcrypto.subtle;
const hex = b => [...new Uint8Array(b)].map(x => x.toString(16).padStart(2, "0")).join("");
const unhex = h => Uint8Array.from(h.match(/../g) ?? [], x => parseInt(x, 16));
const root = new URL("../../", import.meta.url).pathname;
// The SDK as an app has it: through the entry (`wrap`), never the raw bindings.
const sdk = wrap(createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js"));
const webappCode = readFileSync(join(root, "pkg/web/webapp.wasm"));

// The manifest as the builder's build makes it: the native tool over the vectors' files.
const v = JSON.parse(readFileSync(new URL("./pieces-vectors.json", import.meta.url), "utf8"));
const dir = mkdtempSync(join(tmpdir(), "repair-pieces-"));
const paths = v.files.map(f => { const p = join(dir, f.name); writeFileSync(p, unhex(f.hex)); return p; });
const tool = join(root, "pkg/tools/load-pieces");
const out = join(dir, "out");
const shape = JSON.parse(execFileSync(tool, [join(root, "pkg/web/webapp.wasm"), String(v.payload), String(v.m), out, ...paths], { encoding: "utf8" }));
const pieceBytes = shape.pieces.map((_, i) => readFileSync(join(out, `piece-${i}.bin`)));
const spec = {
  ...shape,
  pieces: await Promise.all(shape.pieces.map(async (p, i) => ({ address: p.address, sha256: hex(await subtle.digest("SHA-256", pieceBytes[i])) }))),
};
const bundle = readFileSync(join(out, "bundle.bin"));
const n = spec.k + spec.m;

await t("the native tool's pieces ARE the frozen vectors (one implementation, two callers)", async () => {
  assert.equal(shape.k, v.k);
  for (let i = 0; i < n; i += 1) assert.equal(hex(pieceBytes[i]), v.pieces[i], `piece ${i}`);
});

function session() {
  const puts = [];
  return { puts, put_piece: (code, piece) => { puts.push({ code, piece }); return `key-${puts.length}`; } };
}

await t("**only the pieces answered NOT HELD and not got are PUT back** — never one the race cancelled at k — each byte-identical to the tool's container", async () => {
  const lost = new Set([1, 5, 12]);
  const cancelled = new Set([7, 9]); // asked, never arrived, never answered not-found: the race ended at k
  const asked = [...Array(n).keys()].filter(i => i !== 14); // piece 14 was never asked: not this load's to repair
  const raced = { asked, notHeld: [1, 5, 12], pieces: pieceBytes.map((b, i) => (lost.has(i) || cancelled.has(i) || i === 14 ? null : b)) };
  const s = session();
  const done = await repairPieces({ spec, bundle, raced, sdk, session: s, webappCode, subtle });
  assert.deepEqual(done.map(d => d.piece), [1, 5, 12], `repaired ${JSON.stringify(done)}`);
  assert.ok(done.every(d => d.put), JSON.stringify(done));
  for (const [j, i] of [1, 5, 12].entries()) {
    const put = s.puts[j];
    // put_piece takes the RAW piece and frames it itself (the SDK's one framing): framed the same way, it is the tool's.
    const state = sdk.pieces.container(put.piece);
    assert.ok(Buffer.from(state).equals(readFileSync(join(out, `piece-${i}.webapp`))), `piece ${i}: not the tool's container`);
    assert.equal(sdk.webapp.address(webappCode, state), spec.pieces[i].address);
    assert.ok(Buffer.from(put.code).equals(webappCode));
  }
});

await t("a piece whose re-derived bytes do not match the manifest is REFUSED, not PUT", async () => {
  const bad = { ...spec, pieces: spec.pieces.map((p, i) => (i === 3 ? { ...p, sha256: "00".repeat(32) } : p)) };
  const raced = { asked: [3, 4], notHeld: [3, 4], pieces: pieceBytes.map((b, i) => (i === 3 || i === 4 ? null : b)) };
  const s = session();
  const done = await repairPieces({ spec: bad, bundle, raced, sdk, session: s, webappCode, subtle });
  assert.deepEqual(done.map(d => [d.piece, !!d.put]), [[3, false], [4, true]]);
  assert.match(done[0].refused, /sha256/);
  assert.equal(s.puts.length, 1, "a refused piece was PUT");
});

await t("a piece whose container is not at the manifest's ADDRESS is refused, not PUT", async () => {
  const bad = { ...spec, pieces: spec.pieces.map((p, i) => (i === 6 ? { ...p, address: spec.pieces[7].address } : p)) };
  const s = session();
  const done = await repairPieces({ spec: bad, bundle, raced: { asked: [6], notHeld: [6], pieces: pieceBytes.map((b, i) => (i === 6 ? null : b)) }, sdk, session: s, webappCode, subtle });
  assert.equal(done.length, 1);
  assert.match(done[0].refused ?? "", /address/);
  assert.equal(s.puts.length, 0);
});

await t("nothing missing: nothing PUT", async () => {
  const s = session();
  const done = await repairPieces({ spec, bundle, raced: { asked: [...Array(n).keys()], notHeld: [], pieces: pieceBytes }, sdk, session: s, webappCode, subtle });
  assert.deepEqual(done, []);
  assert.equal(s.puts.length, 0);
});

if (failures) { process.stdout.write(`repair-pieces: ${failures} FAILED\n`); process.exit(1); }
process.stdout.write("repair-pieces: all ok\n");
