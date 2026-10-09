// publishDefinition({ site }) (app-as-data P5): the definition's one write and the app's SITE in ONE call -- the
// engine db hands the session's one door exactly the site it was given; without a site it is the definition alone;
// the in-tab db refuses a site by name (it has no node).
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { engineDb } from "../../js/engine-db.js";
import { wrap } from "../../js/wrap.js";

const calls = [];
const session = {
  publish_definition: () => { calls.push(["publish_definition"]); return 2; },
  publish_definition_site: (...a) => { calls.push(["publish_definition_site", ...a]); return JSON.stringify({ changed: 2, link: "http://node/v1/contract/web/abc/" }); },
};
const db = engineDb({ session });
assert.equal(await db.publishDefinition(), 2, "without a site: the definition alone");
const starter = { files: "the AppContainer" };
const set = { name: "core", k: 3, m: 2, pieces: [] };
const r = await db.publishDefinition({ site: { set, webappCode: new Uint8Array([1]), pieceStates: [new Uint8Array([2])], siteCode: new Uint8Array([3]), starter } });
assert.deepEqual(r, { changed: 2, link: "http://node/v1/contract/web/abc/" });
assert.deepEqual(calls, [["publish_definition"], ["publish_definition_site", JSON.stringify(set), new Uint8Array([1]), [new Uint8Array([2])], new Uint8Array([3]), starter]], "the session was not handed the site as given");
const tab = new (wrap(createRequire(import.meta.url)("../../pkg/node/craftworks_sdk.js")).Db)();
await assert.rejects(() => tab.publishDefinition({ site: { set } }), /has no node/);
assert.equal(await tab.publishDefinition(), 0, "the in-tab db's definition publish without a site");
console.log("all passing");
