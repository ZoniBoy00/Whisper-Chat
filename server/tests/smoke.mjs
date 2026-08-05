// Smoke test for whisper-relay: verifies live routing, acks, the offline
// delivery queue (SQLite-backed), fetch_since offline sync, per-IP rate
// limiting and the signed-hello spoofing protection. Envelope payloads are
// treated as opaque bytes by the relay (and they are, by construction).
//
// Usage:
//   # Start the relay with a bounded rate limit and a scratch DB, then:
//   node tests/smoke.mjs   (relay must be running on 127.0.0.1:8080)
//
// The rate-limit test relies on WHISPER_RATE_BURST / WHISPER_RATE_REFILL
// being set low; it also passes with the defaults (60/min burst).

import { generateKeyPairSync, createHash, sign } from "node:crypto";

const URL = process.env.WHISPER_WS_URL || "ws://127.0.0.1:8080/ws";

const DEBUG = process.env.DEBUG === "1";

// Build a self-authenticating signed hello.
// - x25519 public key (raw 32 bytes) -> peer_id = sha256(pub)[:16 hex]
// - ed25519 signature over the peer_id, base64-encoded
function makeIdentity() {
  const { privateKey: edPriv, publicKey: edPub } = generateKeyPairSync("ed25519");
  const { publicKey: xPub } = generateKeyPairSync("x25519");

  const xDer = xPub.export({ type: "spki", format: "der" });
  const curveRaw = xDer.subarray(xDer.length - 32);
  const peerId = createHash("sha256").update(curveRaw).digest("hex").slice(0, 16);

  const edDer = edPub.export({ type: "spki", format: "der" });
  const edRaw = edDer.subarray(edDer.length - 32);

  return {
    peer_id: peerId,
    curve25519_key: curveRaw.toString("base64"),
    ed25519_key: edRaw.toString("base64"),
    signature: sign(null, Buffer.from(peerId, "utf8"), edPriv).toString("base64"),
  };
}

// Reuse an existing identity's peer_id + curve key but sign with a fresh
// ed25519 key: verification passes, yet the identity conflicts with the
// already-registered one.
function conflictingHello(original) {
  const { privateKey: edPriv, publicKey: edPub } = generateKeyPairSync("ed25519");
  const edDer = edPub.export({ type: "spki", format: "der" });
  const edRaw = edDer.subarray(edDer.length - 32);
  return {
    peer_id: original.peer_id,
    curve25519_key: original.curve25519_key,
    ed25519_key: edRaw.toString("base64"),
    signature: sign(null, Buffer.from(original.peer_id, "utf8"), edPriv).toString("base64"),
  };
}

// Flip one base64 character in the middle of a string (keeps it decodable).
function tamper(b64) {
  const i = Math.floor(b64.length / 2);
  return b64.slice(0, i) + (b64[i] === "A" ? "B" : "A") + b64.slice(i + 1);
}

function connect(label) {
  const ws = new WebSocket(URL);
  const ready = new Promise((res, rej) => {
    ws.onopen = () => {
      if (DEBUG) console.log(`[${label}] ws open`);
      res();
    };
    ws.onerror = () => rej(new Error("ws connection failed"));
  });
  ws.messages = [];
  ws.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    if (DEBUG) console.log(`[${label}] <- ${JSON.stringify(msg).slice(0, 120)}`);
    ws.messages.push(msg);
  };
  ws.onclose = () => DEBUG && console.log(`[${label}] ws closed`);
  ws.sendJson = (obj) => {
    if (DEBUG) console.log(`[${label}] -> ${JSON.stringify(obj).slice(0, 120)}`);
    ws.send(JSON.stringify(obj));
  };
  ws.hello = (hello) => ws.sendJson({ type: "hello", ...hello });
  return { ws, ready };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const waitFor = async (label, fn, timeoutMs = 5000) => {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (fn()) return;
    await sleep(50);
  }
  throw new Error(`timeout waiting for: ${label}`);
};

let failures = 0;
const check = (name, ok) => {
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}`);
  if (!ok) failures++;
};

async function main() {
  const alice = makeIdentity();
  const bob = makeIdentity();
  const carol = makeIdentity();
  const dave = makeIdentity();

  const aliceConn = connect("alice");
  const bobConn = connect("bob");
  await Promise.all([aliceConn.ready, bobConn.ready]);

  aliceConn.ws.hello(alice);
  bobConn.ws.hello(bob);

  // --- Test 1: live routing ---
  aliceConn.ws.sendJson({
    type: "envelope",
    envelope: {
      sender: alice.peer_id,
      recipient: bob.peer_id,
      payload: Buffer.from("opaque ciphertext #1").toString("base64"),
      seq: 1,
    },
  });

  await waitFor("test", () =>
    bobConn.ws.messages.some(
      (m) => m.type === "envelope" && m.envelope.sender === alice.peer_id
    )
  );
  const gotLive = bobConn.ws.messages.find((m) => m.type === "envelope");
  check(
    "live routing: bob received alice's envelope",
    gotLive && gotLive.envelope.sender === alice.peer_id
  );

  await waitFor("test", () =>
    aliceConn.ws.messages.some((m) => m.type === "ack" && m.seq === 1)
  );
  check("live routing: alice received ack for seq 1", true);

  // --- Test 12 (new): spoofing guard ---
  // Alice claims to be bob; the relay must reject the envelope and never
  // deliver it to bob.
  aliceConn.ws.sendJson({
    type: "envelope",
    envelope: {
      sender: bob.peer_id,
      recipient: bob.peer_id,
      payload: Buffer.from("spoofed ciphertext").toString("base64"),
      seq: 2000,
    },
  });
  await waitFor("sender_mismatch error", () =>
    aliceConn.ws.messages.some((m) => m.type === "error" && m.code === "sender_mismatch")
  );
  check("spoofing: sender_mismatch error sent to the sender", true);
  await sleep(300);
  const bobGotSpoof = bobConn.ws.messages.some(
    (m) => m.type === "envelope" && m.envelope.seq === 2000
  );
  check("spoofing: bob did not receive the spoofed envelope", !bobGotSpoof);

  // --- Test 2: offline queue + flush on reconnect ---
  bobConn.ws.close();
  await sleep(200); // let the relay notice the disconnect

  aliceConn.ws.sendJson({
    type: "envelope",
    envelope: {
      sender: alice.peer_id,
      recipient: bob.peer_id,
      payload: Buffer.from("opaque ciphertext #2 (offline)").toString("base64"),
      seq: 2,
    },
  });
  await waitFor("test", () =>
    aliceConn.ws.messages.some((m) => m.type === "ack" && m.seq === 2)
  );
  check("offline queue: alice got ack while bob was offline", true);

  const bob2 = connect("bob2");
  await bob2.ready;
  bob2.ws.hello(bob);
  await waitFor("test", () =>
    bob2.ws.messages.some(
      (m) => m.type === "envelope" && m.envelope.seq === 2
    )
  );
  check("offline queue: reconnected bob received queued blob", true);

  // --- Test 3: oversized envelope is dropped ---
  aliceConn.ws.sendJson({
    type: "envelope",
    envelope: {
      sender: alice.peer_id,
      recipient: bob.peer_id,
      payload: "x".repeat(9 * 1024 * 1024), // 9 MiB > 8 MiB cap
      seq: 3,
    },
  });
  await sleep(300);
  const gotOversize = aliceConn.ws.messages.some((m) => m.type === "ack" && m.seq === 3);
  check("DoS guard: oversized envelope is dropped (no ack)", !gotOversize);

  // --- Test (a): SQLite persistence + fetch_since offline sync ---
  // Seq 2 (from test 2) is still persisted; add two more offline blobs.
  bob2.ws.close();
  await sleep(200);

  aliceConn.ws.sendJson({
    type: "envelope",
    envelope: {
      sender: alice.peer_id,
      recipient: bob.peer_id,
      payload: Buffer.from("opaque ciphertext #4 (sqlite)").toString("base64"),
      seq: 4,
    },
  });
  aliceConn.ws.sendJson({
    type: "envelope",
    envelope: {
      sender: alice.peer_id,
      recipient: bob.peer_id,
      payload: Buffer.from("opaque ciphertext #5 (sqlite)").toString("base64"),
      seq: 5,
    },
  });
  await waitFor("test", () =>
    aliceConn.ws.messages.some((m) => m.type === "ack" && m.seq === 5)
  );
  check("sqlite: offline envelopes acked while bob offline", true);

  // Reconnecting bob is pushed the persisted blobs (proves they live in
  // SQLite, not only in memory).
  const bob3 = connect("bob3");
  await bob3.ready;
  bob3.ws.hello(bob);
  await waitFor("test", () =>
    bob3.ws.messages.some(
      (m) => m.type === "envelope" && m.envelope.seq === 4
    )
  );
  await waitFor("test", () =>
    bob3.ws.messages.some(
      (m) => m.type === "envelope" && m.envelope.seq === 5
    )
  );
  check("sqlite: reconnected bob received persisted offline envelopes", true);

  // fetch_since returns the whole offline history in one batch.
  bob3.ws.sendJson({ type: "fetch_since", since: 0 });
  await waitFor("fetch_since reply", () =>
    bob3.ws.messages.some((m) => m.type === "envelopes")
  );
  const batch = bob3.ws.messages.filter((m) => m.type === "envelopes").pop();
  check(
    "fetch_since: returns persisted envelopes to the client",
    batch &&
      Array.isArray(batch.envelopes) &&
      batch.envelopes.some((e) => e.seq === 4) &&
      batch.envelopes.some((e) => e.seq === 5)
  );

  // A successful fetch drains the store: the next connection sees nothing.
  bob3.ws.close();
  await sleep(200);
  const bob4 = connect("bob4");
  await bob4.ready;
  bob4.ws.hello(bob);
  bob4.ws.sendJson({ type: "fetch_since", since: 0 });
  await waitFor("empty fetch_since reply", () =>
    bob4.ws.messages.some((m) => m.type === "envelopes")
  );
  const batch2 = bob4.ws.messages.filter((m) => m.type === "envelopes").pop();
  check(
    "fetch_since: store drained after fetch (empty batch)",
    batch2 && Array.isArray(batch2.envelopes) && batch2.envelopes.length === 0
  );

  // --- Test (b): rate limiting per IP ---
  const carolConn = connect("carol");
  await carolConn.ready;
  carolConn.ws.hello(carol);
  await sleep(100);

  // Burst into the bucket: the relay must ack some envelopes and reject the
  // excess with a rate_limited error.
  for (let i = 0; i < 120; i++) {
    carolConn.ws.sendJson({
      type: "envelope",
      envelope: {
        sender: carol.peer_id,
        recipient: dave.peer_id,
        payload: Buffer.from(`carol burst #${i}`).toString("base64"),
        seq: 1000 + i,
      },
    });
  }
  await waitFor("rate_limited error", () =>
    carolConn.ws.messages.some((m) => m.type === "error" && m.code === "rate_limited")
  );
  check("rate limit: excess envelopes are blocked", true);

  const carolAcks = carolConn.ws.messages.filter((m) => m.type === "ack").length;
  check("rate limit: envelopes within burst still get acks", carolAcks >= 1);

  // --- Test 13 (new): identity conflict (same peer id, different ed25519) ---
  const mallory = connect("mallory");
  await mallory.ready;
  mallory.ws.hello(conflictingHello(bob));
  await waitFor("identity_conflict error", () =>
    mallory.ws.messages.some((m) => m.type === "error" && m.code === "identity_conflict")
  );
  check("identity conflict: rejected with identity_conflict", true);
  mallory.ws.close();

  // --- Test 14 (new): invalid signature hello ---
  const eve = makeIdentity();
  eve.signature = tamper(eve.signature);
  const eveConn = connect("eve");
  await eveConn.ready;
  eveConn.ws.hello(eve);
  await waitFor("invalid_hello error", () =>
    eveConn.ws.messages.some((m) => m.type === "error" && m.code === "invalid_hello")
  );
  check("invalid hello: tampered signature is rejected with invalid_hello", true);
  eveConn.ws.close();

  aliceConn.ws.close();
  bob4.ws.close();
  carolConn.ws.close();

  console.log(failures === 0 ? "\nALL TESTS PASSED" : `\n${failures} TEST(S) FAILED`);
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((err) => {
  console.error("SMOKE TEST ERROR:", err.message);
  process.exit(1);
});
