// The KEYS SPENT that `admit` hands `rules::admission` (`packages/conversation.js`: `sharedTags`) and a kept welcome's
// wait (`resendWait`) — pure, run with no node and no page:   node --test tools/welcome-packages.test.mjs
// (Which key a welcome is made with is no longer chosen here: it is the request's own — `rules::admission`'s tests.)
// `CONVERSATION_JS`: another copy of the file to test.
import test from "node:test";
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";

const src = process.env.CONVERSATION_JS ?? new URL("../packages/conversation.js", import.meta.url).href;
const { sharedTags, resendWait } = await import(src.startsWith("file:") ? src : pathToFileURL(src).href);

const t = n => n.toString(16).padStart(32, "0");

test("a node's repair pointer carrying kp counts too; entries without a tag do not", () => {
  const pointers = [{ w: "a", repair: "did:craftec:x", epoch: 4, kp: t(3) }, { w: "b", repair: "did:craftec:y", epoch: 5 }, { w: "c", epoch: 7, branch: "ff" }, { w: "d", kp: "not-a-tag" }, null];
  assert.deepEqual([...sharedTags(pointers).keys()], [t(3)]);
});

test("a welcome's tag in the writers list is spent for every member", () => {
  const pointers = [{ w: "deviceA", welcomed: "did:craftec:x", kp: t(1), at: 1 }, { w: "deviceB", welcomed: "did:craftec:y", kp: t(2), at: 2 }];
  assert.deepEqual([...sharedTags(pointers).keys()], [t(1), t(2)]);
});

test("resend wait doubles from 2 minutes to at most an hour", () => {
  assert.deepEqual([1, 2, 3, 4, 5, 6, 7, 30].map(resendWait), [120e3, 240e3, 480e3, 960e3, 1920e3, 3600e3, 3600e3, 3600e3]);
});
