// The socket wrapper, against a fake WebSocket.
//
// The defect this file exists for: the first version took the engine's whole
// outbound queue and broke out of the send loop when the socket refused, so
// everything taken and not sent was dropped. Its own comment said the engine
// still had them — it did not, because the take had already happened. A write
// made before `onopen` is the ORDINARY start of a session, so the normal path
// lost its first write silently.
//
// Every test here runs the real `connect()` over a fake socket whose open/close
// the test controls. The engine is faked too, and deliberately: what is under
// test is the wrapper's contract with the engine — take nothing you cannot
// send, confirm exactly what went, in order — and a real engine would make the
// assertions about queue contents harder to read without making them stronger.

import assert from "node:assert/strict";
import { connect } from "../../js/connection.js";

/** A WebSocket the test opens, closes and reads. */
class FakeSocket {
  static last = null;
  constructor(url) {
    this.url = url;
    this.readyState = 0;           // CONNECTING
    this.sent = [];
    this.throwOnSend = false;
    FakeSocket.last = this;
  }
  send(bytes) {
    if (this.throwOnSend) throw new Error("socket went away mid-batch");
    this.sent.push(bytes);
  }
  close() { this.readyState = 3; this.onclose?.(); }
  /** The node accepted the connection. */
  open() { this.readyState = 1; this.onopen?.(); }
  /** It went away without warning. */
  drop() { this.readyState = 3; this.onclose?.(); }
}

/** An engine that only models the outbound queue. */
function fakeEngine(items = []) {
  let queue = [...items];
  return {
    outbound: () => [...queue],
    sent: n => { queue = queue.slice(n); },
    outbound_len: () => queue.length,
    on_inbound: () => {},
    queue: () => [...queue],
    push: x => queue.push(x),
  };
}

/**
 * Run `fn` with the fake socket installed globally.
 *
 * AWAITS it before restoring. The first version restored in a `finally`,
 * which runs the moment `fn` returns its promise — so anything the test did
 * on a timer (a reconnect, which is the only way to observe one) constructed
 * a REAL WebSocket and the assertions read the wrong object. The test failed
 * for a reason that had nothing to do with the code it was testing.
 */
const withGlobalSocket = async fn => {
  const had = globalThis.WebSocket;
  globalThis.WebSocket = FakeSocket;
  try { return await fn(); } finally { globalThis.WebSocket = had; }
};

// ---------------------------------------------------------------------------
// A closed socket loses NOTHING.
// ---------------------------------------------------------------------------
await withGlobalSocket(async () => {
  const engine = fakeEngine(["a", "b", "c"]);
  const conn = connect(engine, { url: "ws://x" });
  const sock = FakeSocket.last;

  // The socket has not opened yet — the ordinary start of every session.
  conn.pump();
  assert.deepEqual(sock.sent, [], "a closed socket sent something");
  assert.deepEqual(
    engine.queue(), ["a", "b", "c"],
    "the engine lost requests it never got to send — this is the defect, and " +
    "the ordinary path is a write made before onopen",
  );

  // Now it opens: everything goes, in order, once.
  sock.open();
  assert.deepEqual(sock.sent, ["a", "b", "c"], "order or completeness lost on open");
  assert.deepEqual(engine.queue(), [], "the engine still holds what was sent");
  conn.close();
  console.log("ok connection: a closed socket loses nothing");
});

// ---------------------------------------------------------------------------
// A close PART WAY THROUGH a batch loses nothing, and keeps the order.
// ---------------------------------------------------------------------------
await withGlobalSocket(async () => {
  const engine = fakeEngine(["a", "b", "c", "d"]);
  const conn = connect(engine, { url: "ws://x" });
  const sock = FakeSocket.last;
  sock.open();
  assert.deepEqual(sock.sent, ["a", "b", "c", "d"]);

  // More work arrives, and the socket dies on the second of it.
  engine.push("e");
  engine.push("f");
  engine.push("g");
  let n = 0;
  const realSend = sock.send.bind(sock);
  sock.send = bytes => { if (n++ === 1) throw new Error("gone"); realSend(bytes); };
  conn.pump();

  assert.deepEqual(sock.sent.slice(4), ["e"], "the first of the batch should have gone");
  assert.deepEqual(
    engine.queue(), ["f", "g"],
    "a mid-batch failure must leave exactly the unsent remainder, in order",
  );

  // The socket comes back; the remainder goes, still in order.
  sock.send = realSend;
  conn.pump();
  assert.deepEqual(engine.queue(), []);
  assert.deepEqual(sock.sent.slice(4), ["e", "f", "g"], "order was not preserved across the failure");
  conn.close();
  console.log("ok connection: a mid-batch failure loses nothing and keeps the order");
});

// ---------------------------------------------------------------------------
// THE CONTROL: the shape the first version used fails these.
//
// Not a comment claiming the old code was wrong — the old code, run.
// ---------------------------------------------------------------------------
{
  const engine = fakeEngine(["a", "b", "c"]);
  // `take_outbound` + break, which is what the first version did.
  const takeOutbound = () => { const q = engine.queue(); engine.sent(q.length); return q; };
  const sock = { readyState: 0, sent: [] };
  const send = b => { if (sock.readyState !== 1) return false; sock.sent.push(b); return true; };
  for (const bytes of takeOutbound()) { if (!send(bytes)) break; }

  assert.deepEqual(sock.sent, [], "the control sent something with the socket closed");
  assert.deepEqual(
    engine.queue(), [],
    "the control did NOT lose the queue, so the tests above are not " +
    "demonstrating what the fix prevents",
  );
  console.log("ok connection CONTROL: the take-and-break shape loses the whole queue");
}


// ---------------------------------------------------------------------------
// A REPLACED socket's frames never reach the engine (sdk#490, OPENING.md Machine 3's S2): a head move the old
// connection carried must not "prove" the new connection's subscription (the node dropped the old one's at close).
// THE CONTROL: the current socket's frame does reach it.
// ---------------------------------------------------------------------------
await withGlobalSocket(async () => {
  const engine = fakeEngine();
  const inbound = [];
  engine.on_inbound = bytes => inbound.push([...bytes]);
  const events = [];
  const conn = connect(engine, { url: "ws://x", onEvent: e => events.push(e.kind) });
  const old = FakeSocket.last;
  old.open();
  old.onmessage({ data: new Uint8Array([1]).buffer });
  assert.deepEqual(inbound, [[1]], "THE SETUP: the open socket's frame did not arrive");
  old.drop();
  await new Promise(r => setTimeout(r, 400)); // the reconnect's backoff
  const fresh = FakeSocket.last;
  assert.notEqual(fresh, old, "THE SETUP: no new socket was opened");
  fresh.open();
  // The OLD socket's frame, arriving after the replacement: dropped, counted, named.
  old.onmessage({ data: new Uint8Array([2]).buffer });
  assert.deepEqual(inbound, [[1]], "a replaced socket's frame reached the engine");
  assert.equal(conn.staleDropped, 1);
  assert.ok(events.includes("stale"), "the drop was not named");
  // THE CONTROL: the current socket's frame arrives.
  fresh.onmessage({ data: new Uint8Array([3]).buffer });
  assert.deepEqual(inbound, [[1], [3]], "the current socket's frame did not arrive");
  conn.close();
  console.log("ok connection: a replaced socket's frames never reach the engine; the current one's do");
});
