// THE DECODER, BUILT (sdk#347): the frozen load-piece vectors (pieces/examples/vectors.rs, checked against the
// native code by the `pieces` crate's test) decoded by the SHIPPED decoder wasm (pkg/web/decoder.wasm, from
// build.sh) through js/pieces.js. So the one RS implementation is proven equal on both targets: native cut, wasm
// decode.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { decoder, openPieces } from "../../js/pieces.js";

let failures = 0;
const t = async (name, fn) => {
  try { await fn(); process.stdout.write(`  ok  ${name}\n`); }
  catch (e) { failures += 1; process.stdout.write(`  FAIL ${name}\n    ${e.message}\n`); }
};
const unhex = h => Uint8Array.from(h.match(/../g) ?? [], x => parseInt(x, 16));

const v = JSON.parse(await readFile(new URL("./pieces-vectors.json", import.meta.url), "utf8"));
const wasm = await readFile(new URL("../../pkg/web/decoder.wasm", import.meta.url));
const pieces = v.pieces.map(unhex);
const want = new Map(v.files.map(f => [f.name, unhex(f.hex)]));
const shape = { k: v.k, m: v.m, payload: v.payload, bundle_len: v.bundle_len };
const n = v.k + v.m;

await t(`the shipped decoder is ${wasm.length} B (the per-app storage this design adds)`, async () => {
  assert.ok(wasm.length > 1000 && wasm.length < 48 * 1024, `${wasm.length} B`);
});

const cases = [
  ["every piece", () => true],
  ["the first m pieces missing (data lost: a real solve)", i => i >= v.m],
  ["every parity piece missing (the data alone)", i => i < v.k],
  ["every other piece missing", i => i % 2 === 0 || i >= n - (v.m - Math.floor(n / 2)) ],
  ["the last data piece and 7 parity missing", i => i !== v.k - 1 && !(i >= v.k && i < v.k + 7)],
];
for (const [name, keep] of cases) {
  await t(`the vectors open with ${name}`, async () => {
    const have = pieces.map((p, i) => (keep(i) ? p : null));
    assert.ok(have.filter(Boolean).length >= v.k, "the case keeps fewer than k: the test is wrong");
    const { files: got, bundle } = openPieces(await decoder(wasm), shape, have);
    assert.equal(bundle.length, v.bundle_len, "the rebuilt bundle is not its length");
    assert.deepEqual([...got.keys()], [...want.keys()]);
    for (const [f, bytes] of want) assert.ok(Buffer.from(got.get(f)).equals(Buffer.from(bytes)), `${f} differs`);
  });
}

await t("m + 1 pieces missing is refused in the decoder's words, not a wrong file", async () => {
  const have = pieces.map((p, i) => (i > v.m ? p : null));
  const dec = await decoder(wasm);
  assert.throws(() => openPieces(dec, shape, have), /fewer than k pieces/);
});

await t("a piece altered in flight (a caller that skipped the hash) does not open as the files", async () => {
  const dec = await decoder(wasm);
  const have = pieces.map((p, i) => (i < v.m ? null : p));
  const bad = have.slice();
  bad[v.m] = bad[v.m].slice();
  bad[v.m][10] ^= 0xff;
  let got = null;
  try { got = openPieces(dec, shape, bad).files; } catch (_) { return; }
  assert.ok(![...want].every(([f, b]) => Buffer.from(got.get(f) ?? []).equals(Buffer.from(b))), "an altered piece still gave the right files: the case is vacuous");
});

if (failures) { process.stdout.write(`decoder: ${failures} FAILED\n`); process.exit(1); }
process.stdout.write("decoder: all ok\n");
