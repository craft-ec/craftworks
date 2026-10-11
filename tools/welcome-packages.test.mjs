// The KEY PACKAGE CHOICE of a welcome (`packages/conversation.js`: `sharedTags`, `freePackages`, `resendWait`) — pure,
// run with no node and no page:   node --test tools/welcome-packages.test.mjs
// `CONVERSATION_JS`: another copy of the file to test (the negative control: one with the exclusion taken out fails).
import test from "node:test";
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";

const src = process.env.CONVERSATION_JS ?? new URL("../packages/conversation.js", import.meta.url).href;
const { sharedTags, freePackages, resendWait } = await import(src.startsWith("file:") ? src : pathToFileURL(src).href);

const t = n => n.toString(16).padStart(32, "0");

// 10-11, as measured: two members welcome the same person from the same card. A's account used t(1); B's account never
// saw that (its own `keypacks` only) — B must not pick t(1) once A named it in the space's writers list.
test("a package another member used is not chosen again", () => {
  const card = [t(1), t(2)];
  const pointers = [{ w: "deviceA", welcomed: "did:craftec:x", kp: t(1), at: 1, sealedAt: 0xffffffff }];
  const { free, skipped } = freePackages(card, new Set(), sharedTags(pointers));
  assert.deepEqual(free, [1]);
  assert.deepEqual(skipped, [t(1)]);
});

test("a node's repair pointer carrying kp counts too; entries without a tag do not", () => {
  const pointers = [{ w: "a", repair: "did:craftec:x", epoch: 4, kp: t(3) }, { w: "b", repair: "did:craftec:y", epoch: 5 }, { w: "c", epoch: 7, branch: "ff" }, { w: "d", kp: "not-a-tag" }, null];
  assert.deepEqual([...sharedTags(pointers).keys()], [t(3)]);
});

test("own spent packages are excluded and not reported as shared", () => {
  const card = [t(1), t(2), t(3)];
  const { free, skipped } = freePackages(card, new Set([t(1)]), sharedTags([{ w: "me", kp: t(1) }, { w: "other", kp: t(2) }]));
  assert.deepEqual(free, [2]);
  assert.deepEqual(skipped, [t(2)]);
});

test("every package used: none free (the caller asks for a renewal)", () => {
  const { free } = freePackages([t(1), t(2)], new Set([t(2)]), sharedTags([{ w: "o", kp: t(1) }]));
  assert.deepEqual(free, []);
});

test("resend wait doubles from 2 minutes to at most an hour", () => {
  assert.deepEqual([1, 2, 3, 4, 5, 6, 7, 30].map(resendWait), [120e3, 240e3, 480e3, 960e3, 1920e3, 3600e3, 3600e3, 3600e3]);
});
