// THE SIGNER IS FROZEN (signer/frozen): the committed signer.wasm is the pinned one, build.sh ships THAT file after
// the check, and a changed byte -- a rebuild after any change to a crate the signer links -- is refused by name.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { pinOf, refusal } from "../../tools/signer-frozen.mjs";

const root = new URL("../../", import.meta.url);
const bytes = readFileSync(new URL("signer/frozen/signer.wasm", root));
const pin = pinOf(readFileSync(new URL("signer/frozen/signer.sha256", root), "utf8"));
assert.equal(pin, "f9fc78b06c25a159a959ca540bcfb708f3bce6371a8a2083effa49167dfa65c4", "the pin is not the owner's identity's signer");
assert.equal(refusal(bytes, pin), null, "the committed signer does not match its pin");
const planted = Buffer.from(bytes);
planted[planted.length - 1] ^= 1;
assert.match(refusal(planted, pin) ?? "", /FROZEN.*owner's say/, "a changed signer was not refused by name");
const build = readFileSync(new URL("build.sh", root), "utf8");
assert.ok(build.includes("node tools/signer-frozen.mjs signer/frozen/signer.wasm signer/frozen/signer.sha256 || exit 1") && build.includes("cp signer/frozen/signer.wasm pkg/web/signer.wasm"), "build.sh does not ship the checked frozen signer");
assert.ok(!/^\.\/signer\/build\.sh/m.test(build), "build.sh still rebuilds the signer it ships");
console.log("all passing");
