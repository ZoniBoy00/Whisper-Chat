// Smoke test for whisper-relay: verifies live routing, acks, the offline
// delivery queue (SQLite-backed), fetch_since offline sync, per-IP rate
// limiting and the signed-hello spoofing protection. Envelope payloads are
// treated as opaque bytes by the relay (and they are, by construction).
//
// Usage:
//   # Start the relay with a bounded rate limit and a scratch DB, then:
//   node tests/smoke.mjs   (relay must be running on 127.0.0.1:8080)
//
// The rate-limit tests rely on WHISPER_RATE_BURST / WHISPER_RATE_REFILL
// being set low. The username/profile tests perform ~12 profile operations
// from one source IP, the envelope tests burst 20+ acks and the group tests
// (including ownership transfer) consume ~27 tokens from the per-IP group
// bucket, so the shared budget must hold at least that many: run with
// WHISPER_RATE_BURST=40 (which every bucket falls back to) or set
// WHISPER_PROFILE_RATE_BURST explicitly. The 120-envelope burst test still
// overflows a 40-token budget, so rate limiting stays meaningfully exercised.
//
// The presence tests also consume a few tokens from the per-IP buckets.

import { generateKeyPairSync, createHash, sign } from "node:crypto";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";

const URL = process.env.WHISPER_WS_URL || "ws://127.0.0.1:8080/ws";

const DEBUG = process.env.DEBUG === "1";

// Build a self-authenticating signed hello.
// - x25519 public key (raw 32 bytes) -> peer_id = sha256(pub)[:24 hex]
// - ed25519 signature over the peer_id, base64-encoded
// An optional `displayName` is attached as the public profile name
// (Signal-style) and stored by the relay on the first hello.
function makeIdentity(displayName) {
  const { privateKey: edPriv, publicKey: edPub } = generateKeyPairSync("ed25519");
  const { publicKey: xPub } = generateKeyPairSync("x25519");

  const xDer = xPub.export({ type: "spki", format: "der" });
  const curveRaw = xDer.subarray(xDer.length - 32);
  const peerId = createHash("sha256").update(curveRaw).digest("hex").slice(0, 24);

  const edDer = edPub.export({ type: "spki", format: "der" });
  const edRaw = edDer.subarray(edDer.length - 32);

  const identity = {
    peer_id: peerId,
    curve25519_key: curveRaw.toString("base64"),
    ed25519_key: edRaw.toString("base64"),
    signature: sign(null, Buffer.from(peerId, "utf8"), edPriv).toString("base64"),
  };
  if (displayName) identity.display_name = displayName;
  return { ...identity, edPriv };
}

// Build a signed pre-key bundle for an `identity` (from makeIdentity) holding
// `count` fresh one-time keys. Mirrors PreKeyBundle::signed_bytes in
// e2ee-core: the ed25519 signature covers the raw x25519 identity key
// followed by every one-time key in ascending base64 order.
//
// Key fields are emitted unpadded-base64 to match vodozemac's to_base64 wire
// format exactly (Node's toString("base64") would add trailing "=" padding).
function makeBundle(identity, count) {
  const unpadded = (buf) => buf.toString("base64").replace(/=+$/, "");
  const oneTimeRaw = [];
  for (let i = 0; i < count; i++) {
    const { publicKey } = generateKeyPairSync("x25519");
    const der = publicKey.export({ type: "spki", format: "der" });
    oneTimeRaw.push(der.subarray(der.length - 32));
  }
  oneTimeRaw.sort((a, b) => a.toString("base64").localeCompare(b.toString("base64")));
  const signature = sign(
    null,
    Buffer.concat([Buffer.from(identity.curve25519_key, "base64"), ...oneTimeRaw]),
    identity.edPriv
  );
  return {
    version: 1,
    identity_key: unpadded(Buffer.from(identity.curve25519_key, "base64")),
    signing_key: unpadded(Buffer.from(identity.ed25519_key, "base64")),
    signature: unpadded(signature),
    one_time_keys: oneTimeRaw.map(unpadded),
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

const HTTP_URL = process.env.WHISPER_HTTP_URL || "http://127.0.0.1:8080";

// A tiny valid 1x1 PNG used as the avatar upload payload.
const AVATAR_PNG = Buffer.from(
  "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c489" +
    "0000000d4944415478da63fcffff3f030005fef9810e14e0960000000049454e44ae426082",
  "hex"
);

// Sign the canonical username binding: username || 0x00 || curve25519_raw.
// Mirrors e2ee-core profile::canonical_bytes exactly.
function signUsername(identity, username) {
  return sign(
    null,
    Buffer.concat([
      Buffer.from(username, "utf8"),
      Buffer.from([0x00]),
      Buffer.from(identity.curve25519_key, "base64"),
    ]),
    identity.edPriv
  ).toString("base64");
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
// Wait until at least `count` messages match the predicate (re-registrations
// produce several replies of the same shape).
const waitForCount = async (label, ws, predicate, count, timeoutMs = 5000) => {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (ws.messages.filter(predicate).length >= count) return;
    await sleep(50);
  }
  throw new Error(`timeout waiting for ${count} x ${label}`);
};

let failures = 0;
const check = (name, ok) => {
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}`);
  if (!ok) failures++;
};

async function main() {
  const alice = makeIdentity("Test Alice");
  const bob = makeIdentity("Test Bob");
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

  // --- Test (a): pre-key publish + fetch roundtrip ---
  // Alice publishes her signed pre-key bundle; Bob fetches it and receives
  // the exact same bundle (identity key, signing key and one-time keys).
  const aliceBundle = makeBundle(alice, 5);
  aliceConn.ws.sendJson({ type: "publish_prekeys", bundle: aliceBundle });
  await waitFor("prekeys_published", () =>
    aliceConn.ws.messages.some((m) => m.type === "prekeys_published")
  );
  check("prekeys: publish acknowledged with prekeys_published", true);

  bob4.ws.sendJson({ type: "fetch_prekeys", peer_id: alice.peer_id });
  await waitFor("prekeys reply", () =>
    bob4.ws.messages.some((m) => m.type === "prekeys")
  );
  const fetchedMsg = bob4.ws.messages.filter((m) => m.type === "prekeys").pop();
  const fetchedBundle = fetchedMsg.bundle;
  check(
    "prekeys: fetch roundtrip returns the published bundle",
    fetchedBundle &&
      fetchedBundle.identity_key === aliceBundle.identity_key &&
      fetchedBundle.signing_key === aliceBundle.signing_key &&
      fetchedBundle.signature === aliceBundle.signature &&
      JSON.stringify(fetchedBundle.one_time_keys) === JSON.stringify(aliceBundle.one_time_keys)
  );
  check(
    "prekeys: response carries the peer's display name",
    fetchedMsg.display_name === "Test Alice"
  );

  // --- Test (b): identity mismatch ---
  // Alice tries to publish a bundle owned by a different identity. The bundle
  // is cryptographically valid, but its identity key fingerprints to another
  // peer, so the relay must reject it.
  const oscar = makeIdentity();
  aliceConn.ws.sendJson({ type: "publish_prekeys", bundle: makeBundle(oscar, 3) });
  await waitFor("identity_mismatch error", () =>
    aliceConn.ws.messages.some((m) => m.type === "error" && m.code === "identity_mismatch")
  );
  check("prekeys: foreign bundle rejected with identity_mismatch", true);

  // --- Test (c): no_prekeys ---
  // Dave never published a bundle, so fetching his pre-keys must fail.
  bob4.ws.sendJson({ type: "fetch_prekeys", peer_id: dave.peer_id });
  await waitFor("no_prekeys error", () =>
    bob4.ws.messages.some((m) => m.type === "error" && m.code === "no_prekeys")
  );
  check("prekeys: unknown peer returns no_prekeys", true);

  // --- Test (a): update_profile sets a new public display name ---
  aliceConn.ws.sendJson({ type: "update_profile", display_name: "Alice Prime" });
  await waitFor("profile_updated", () =>
    aliceConn.ws.messages.some((m) => m.type === "profile_updated")
  );
  check("update_profile: acknowledged with profile_updated", true);

  // --- Test (b): the new name is visible in the next pre-keys lookup ---
  bob4.ws.sendJson({ type: "fetch_prekeys", peer_id: alice.peer_id });
  await waitFor("prekeys reply (updated profile)", () =>
    bob4.ws.messages
      .filter((m) => m.type === "prekeys")
      .some((m) => m.display_name === "Alice Prime")
  );
  const refetched = bob4.ws.messages.filter((m) => m.type === "prekeys").pop();
  check(
    "update_profile: new name visible in the next prekeys fetch",
    refetched.display_name === "Alice Prime"
  );

  // --- Test (c): an over-long display name is rejected ---
  aliceConn.ws.sendJson({ type: "update_profile", display_name: "A".repeat(65) });
  await waitFor("invalid_display_name error", () =>
    aliceConn.ws.messages.some((m) => m.type === "error" && m.code === "invalid_display_name")
  );
  check("update_profile: name over 64 chars rejected with invalid_display_name", true);

  // --- Presence tests ---

  // (a) get_presence for a peer that never connected: offline, no last_seen.
  aliceConn.ws.sendJson({ type: "get_presence", peer_id: dave.peer_id });
  await waitFor("get_presence unknown reply", () =>
    aliceConn.ws.messages.some(
      (m) => m.type === "presence" && m.peer_id === dave.peer_id
    )
  );
  const unknownPresence = aliceConn.ws.messages
    .filter((m) => m.type === "presence" && m.peer_id === dave.peer_id)
    .pop();
  check(
    "presence: unknown peer reports online:false and last_seen:null",
    unknownPresence &&
      unknownPresence.online === false &&
      unknownPresence.last_seen === null
  );

  // (b) alice watches bob; bob disconnects -> alice gets an offline push.
  aliceConn.ws.sendJson({ type: "watch_presence", peer_id: bob.peer_id });
  await sleep(100); // let the watch registration reach the relay
  const bobPresenceBefore = aliceConn.ws.messages.filter(
    (m) => m.type === "presence" && m.peer_id === bob.peer_id
  ).length;
  bob4.ws.close();
  await waitFor("presence offline push", () =>
    aliceConn.ws.messages.filter(
      (m) => m.type === "presence" && m.peer_id === bob.peer_id
    ).length > bobPresenceBefore
  );
  const offlinePush = aliceConn.ws.messages
    .filter((m) => m.type === "presence" && m.peer_id === bob.peer_id)
    .slice(bobPresenceBefore)
    .pop();
  check(
    "presence: watcher receives an offline push with online:false",
    offlinePush && offlinePush.online === false
  );

  // (c) bob's last_seen is persisted on disconnect and visible via get_presence.
  const bobPresenceBefore2 = aliceConn.ws.messages.filter(
    (m) => m.type === "presence" && m.peer_id === bob.peer_id
  ).length;
  aliceConn.ws.sendJson({ type: "get_presence", peer_id: bob.peer_id });
  await waitFor("get_presence bob reply", () =>
    aliceConn.ws.messages.filter(
      (m) => m.type === "presence" && m.peer_id === bob.peer_id
    ).length > bobPresenceBefore2
  );
  const bobReply = aliceConn.ws.messages
    .filter((m) => m.type === "presence" && m.peer_id === bob.peer_id)
    .slice(bobPresenceBefore2)
    .pop();
  check(
    "presence: get_presence reports last_seen after disconnect",
    bobReply && bobReply.online === false && typeof bobReply.last_seen === "number"
  );

  // (d) bob reconnects -> alice's watch delivers an online push.
  const bob5 = connect("bob5");
  await bob5.ready;
  bob5.ws.hello(bob);
  await waitFor("presence online push", () =>
    aliceConn.ws.messages.some(
      (m) => m.type === "presence" && m.peer_id === bob.peer_id && m.online === true
    )
  );
  check("presence: watcher receives an online push on reconnect", true);
  bob5.ws.close();
  await sleep(100);

  // --- Privacy (presence visibility) ---

  // A fresh identity so the shared carol connection used by the later tests
  // stays intact.
  const frank = makeIdentity("Test Frank");
  const frankConn = connect("frank");
  await frankConn.ready;
  frankConn.ws.hello(frank);
  await sleep(100);

  // (a) frank hides his presence while he is ONLINE: get_presence must report
  //     online:false + last_seen:null.
  frankConn.ws.sendJson({ type: "set_privacy", presence_visible: false });
  await waitFor("privacy_updated (hidden)", () =>
    frankConn.ws.messages.some((m) => m.type === "privacy_updated")
  );
  check("privacy: set_privacy(false) acknowledged with privacy_updated", true);

  aliceConn.ws.sendJson({ type: "get_presence", peer_id: frank.peer_id });
  await waitFor("hidden presence reply", () =>
    aliceConn.ws.messages.some(
      (m) => m.type === "presence" && m.peer_id === frank.peer_id
    )
  );
  const hiddenReply = aliceConn.ws.messages
    .filter((m) => m.type === "presence" && m.peer_id === frank.peer_id)
    .pop();
  check(
    "privacy: hidden peer reports online:false + last_seen:null even while online",
    hiddenReply &&
      hiddenReply.online === false &&
      hiddenReply.last_seen === null
  );

  // (b) the offline push for a hidden peer must hide last_seen too.
  aliceConn.ws.sendJson({ type: "watch_presence", peer_id: frank.peer_id });
  await sleep(100);
  const frankPresenceBefore = aliceConn.ws.messages.filter(
    (m) => m.type === "presence" && m.peer_id === frank.peer_id
  ).length;
  frankConn.ws.close();
  await waitFor("hidden offline push", () =>
    aliceConn.ws.messages.filter(
      (m) => m.type === "presence" && m.peer_id === frank.peer_id
    ).length > frankPresenceBefore
  );
  const hiddenOffline = aliceConn.ws.messages
    .filter((m) => m.type === "presence" && m.peer_id === frank.peer_id)
    .slice(frankPresenceBefore)
    .pop();
  check(
    "privacy: hidden peer's offline push hides last_seen",
    hiddenOffline &&
      hiddenOffline.online === false &&
      hiddenOffline.last_seen === null
  );

  // (c) re-enabling visibility reports the peer normally again.
  const frank2 = connect("frank2");
  await frank2.ready;
  frank2.ws.hello(frank);
  await sleep(100);
  frank2.ws.sendJson({ type: "set_privacy", presence_visible: true });
  await waitFor("privacy_updated (visible)", () =>
    frank2.ws.messages.some((m) => m.type === "privacy_updated")
  );
  check("privacy: set_privacy(true) acknowledged with privacy_updated", true);

  aliceConn.ws.sendJson({ type: "get_presence", peer_id: frank.peer_id });
  await waitFor("visible presence reply", () =>
    aliceConn.ws.messages
      .filter((m) => m.type === "presence" && m.peer_id === frank.peer_id)
      .some((m) => m.online === true)
  );
  const visibleReply = aliceConn.ws.messages
    .filter((m) => m.type === "presence" && m.peer_id === frank.peer_id)
    .pop();
  check(
    "privacy: visible peer reports online:true normally",
    visibleReply &&
      visibleReply.online === true &&
      visibleReply.last_seen === null
  );
  frank2.ws.close();
  await sleep(100);

  // --- Usernames & profiles ---

  // Reconnect bob: bob4 was closed by the presence tests.
  const bob6 = connect("bob6");
  await bob6.ready;
  bob6.ws.hello(bob);

  // (a) register_profile -> search_users -> get_profile roundtrip.
  aliceConn.ws.sendJson({
    type: "register_profile",
    username: "alice_test",
    signature: signUsername(alice, "alice_test"),
    display_name: "Test Alice",
  });
  await waitFor("alice profile_registered", () =>
    aliceConn.ws.messages.some((m) => m.type === "profile_registered")
  );
  const aliceReg = aliceConn.ws.messages.filter((m) => m.type === "profile_registered").pop();
  check(
    "profile: alice registers username alice_test",
    aliceReg && aliceReg.username === "alice_test"
  );

  bob6.ws.sendJson({
    type: "register_profile",
    username: "bob_test",
    signature: signUsername(bob, "bob_test"),
    display_name: "Test Bob",
  });
  await waitFor("bob profile_registered", () =>
    bob6.ws.messages.some((m) => m.type === "profile_registered")
  );
  check("profile: bob registers username bob_test", true);

  // Search by username prefix.
  aliceConn.ws.sendJson({ type: "search_users", query: "alice", limit: 10 });
  await waitFor("search by username reply", () =>
    aliceConn.ws.messages.some((m) => m.type === "users_search")
  );
  let search = aliceConn.ws.messages.filter((m) => m.type === "users_search").pop();
  check(
    "profile: search by username prefix finds alice_test",
    search &&
      search.results.some((r) => r.username === "alice_test" && r.peer_id === alice.peer_id)
  );

  // Search by UID prefix.
  bob6.ws.sendJson({ type: "search_users", query: alice.peer_id.slice(0, 10) });
  await waitFor("search by uid reply", () =>
    bob6.ws.messages.some((m) => m.type === "users_search")
  );
  search = bob6.ws.messages.filter((m) => m.type === "users_search").pop();
  check(
    "profile: search by UID prefix finds alice_test",
    search && search.results.some((r) => r.peer_id === alice.peer_id)
  );

  // get_profile by peer id (both directions).
  aliceConn.ws.sendJson({ type: "get_profile", peer_id: bob.peer_id });
  await waitFor("get_profile(bob) reply", () =>
    aliceConn.ws.messages.some((m) => m.type === "profile")
  );
  const bobProfile = aliceConn.ws.messages.filter((m) => m.type === "profile").pop();
  check(
    "profile: get_profile(bob) returns username + display name + curve key",
    bobProfile &&
      bobProfile.username === "bob_test" &&
      bobProfile.display_name === "Test Bob" &&
      bobProfile.peer_id === bob.peer_id &&
      bobProfile.curve25519_key === bob.curve25519_key
  );

  bob6.ws.sendJson({ type: "get_profile", peer_id: alice.peer_id });
  await waitFor("get_profile(alice) reply", () =>
    bob6.ws.messages.some((m) => m.type === "profile")
  );
  const aliceProfile = bob6.ws.messages.filter((m) => m.type === "profile").pop();
  check(
    "profile: get_profile(alice) returns alice_test",
    aliceProfile &&
      aliceProfile.username === "alice_test" &&
      aliceProfile.curve25519_key === alice.curve25519_key
  );

  // (b) tampered signature -> bad_signature.
  aliceConn.ws.sendJson({
    type: "register_profile",
    username: "tamper_test",
    signature: tamper(signUsername(alice, "tamper_test")),
  });
  await waitFor("bad_signature error", () =>
    aliceConn.ws.messages.some((m) => m.type === "error" && m.code === "bad_signature")
  );
  check("profile: tampered username signature -> bad_signature", true);

  // (c) duplicate username -> username_taken.
  bob6.ws.sendJson({
    type: "register_profile",
    username: "alice_test",
    signature: signUsername(bob, "alice_test"),
  });
  await waitFor("username_taken error", () =>
    bob6.ws.messages.some((m) => m.type === "error" && m.code === "username_taken")
  );
  check("profile: duplicate username -> username_taken", true);

  // (d) reserved username -> invalid_username.
  carolConn.ws.sendJson({
    type: "register_profile",
    username: "admin",
    signature: signUsername(carol, "admin"),
  });
  await waitFor("invalid_username error", () =>
    carolConn.ws.messages.some((m) => m.type === "error" && m.code === "invalid_username")
  );
  check("profile: reserved username rejected with invalid_username", true);

  // (e) avatar upload -> /media/{hash} serves the blob.
  const avatarB64 = AVATAR_PNG.toString("base64");
  const avatarHash = createHash("sha256").update(AVATAR_PNG).digest("hex");
  aliceConn.ws.sendJson({
    type: "register_profile",
    username: "alice_test",
    signature: signUsername(alice, "alice_test"),
    avatar: avatarB64,
  });
  await waitForCount(
    "alice profile_registered (avatar)",
    aliceConn.ws,
    (m) => m.type === "profile_registered" && m.username === "alice_test",
    2
  );
  check("profile: avatar upload acknowledged with profile_registered", true);

  aliceConn.ws.sendJson({ type: "get_profile", peer_id: alice.peer_id });
  await waitFor("get_profile with avatar_url", () =>
    aliceConn.ws.messages
      .filter((m) => m.type === "profile")
      .some((m) => m.avatar_url === `/media/${avatarHash}`)
  );
  check("profile: get_profile exposes avatar_url = /media/{sha256}", true);

  const mediaRes = await fetch(`${HTTP_URL}/media/${avatarHash}`);
  const mediaBlob = Buffer.from(await mediaRes.arrayBuffer());
  check(
    "profile: GET /media/{hash} serves the uploaded avatar bytes",
    mediaRes.status === 200 &&
      mediaRes.headers.get("content-type") === "image/png" &&
      mediaBlob.equals(AVATAR_PNG)
  );

  // The blob must also exist on disk next to the SQLite database
  // (`<db dir>/media/<hash>.bin`). This guards against the regression where
  // the avatar_hash was persisted while the media directory was never created
  // (or the write silently failed), leaving /media/{hash} to 404 forever.
  const dbPath = process.env.WHISPER_DB_PATH;
  if (dbPath) {
    const blobPath = join(dirname(dbPath), "media", `${avatarHash}.bin`);
    check(
      "profile: avatar blob exists on disk under the media directory",
      existsSync(blobPath)
    );
  } else {
    console.log("SKIP  profile: avatar blob on-disk check (WHISPER_DB_PATH unset)");
  }

  // (f) unknown peer get_profile -> no_profile.
  aliceConn.ws.sendJson({ type: "get_profile", peer_id: "000000000000000000000000" });
  await waitFor("no_profile error", () =>
    aliceConn.ws.messages.some((m) => m.type === "error" && m.code === "no_profile")
  );
  check("profile: unknown peer get_profile -> no_profile", true);

  // --- Group chat tests -------------------------------------------------------
  // alice/bob/carol are all connected; bob6 is bob's current socket. Group
  // operations draw from the separate `group:<ip>` rate bucket.

  // (a) alice creates a group -> bob is added -> alice sends a group message
  //     that bob receives and carol (a non-member) does not.
  aliceConn.ws.sendJson({ type: "create_group", name: "Ghost Squad" });
  await waitFor("group_created", () =>
    aliceConn.ws.messages.some((m) => m.type === "group_created")
  );
  const groupCreated = aliceConn.ws.messages.filter((m) => m.type === "group_created").pop();
  const groupId = groupCreated && groupCreated.group_id;
  check(
    "group: create_group returns group_id, name and owner member",
    groupCreated &&
      typeof groupId === "string" &&
      groupCreated.name === "Ghost Squad" &&
      Array.isArray(groupCreated.members) &&
      groupCreated.members.length === 1 &&
      groupCreated.members[0] === alice.peer_id
  );

  // Bob is not a member yet: he cannot read the group info (also covered by
  // the relay unit test `get_group_info_requires_membership`).

  aliceConn.ws.sendJson({ type: "add_group_member", group_id: groupId, peer_id: bob.peer_id });
  await waitFor("group_member_added", () =>
    aliceConn.ws.messages.some((m) => m.type === "group_member_added")
  );
  const added = aliceConn.ws.messages.filter((m) => m.type === "group_member_added").pop();
  check(
    "group: add_group_member acknowledged with the peer id",
    added && added.group_id === groupId && added.peer_id === bob.peer_id
  );

  aliceConn.ws.sendJson({
    type: "send_group_message",
    group_id: groupId,
    envelope: {
      sender: alice.peer_id,
      recipient: "ignored-by-server",
      payload: Buffer.from("group ciphertext #1").toString("base64"),
      seq: 7001,
    },
  });
  await waitFor("group send ack", () =>
    aliceConn.ws.messages.some((m) => m.type === "ack" && m.seq === 7001)
  );
  check("group: send_group_message acked for the sender", true);

  await waitFor("group msg delivered to bob", () =>
    bob6.ws.messages.some((m) => m.type === "envelope" && m.envelope.seq === 7001)
  );
  const groupEnv = bob6.ws.messages.filter((m) => m.type === "envelope" && m.envelope.seq === 7001).pop();
  check(
    "group: bob received the group envelope (recipient rewritten per member)",
    groupEnv &&
      groupEnv.envelope.sender === alice.peer_id &&
      groupEnv.envelope.recipient === bob.peer_id
  );
  await sleep(300);
  const carolGotGroup = carolConn.ws.messages.some(
    (m) => m.type === "envelope" && m.envelope.seq === 7001
  );
  check("group: carol (non-member) did not receive the group message", !carolGotGroup);

  // (b) a non-member cannot send a group message.
  carolConn.ws.sendJson({
    type: "send_group_message",
    group_id: groupId,
    envelope: {
      sender: carol.peer_id,
      recipient: "ignored-by-server",
      payload: Buffer.from("intruder").toString("base64"),
      seq: 7002,
    },
  });
  await waitFor("not_a_member (send)", () =>
    carolConn.ws.messages.some((m) => m.type === "error" && m.code === "not_a_member")
  );
  check("group: non-member send_group_message -> not_a_member", true);

  // (c) get_group_info returns the member roster to members.
  aliceConn.ws.sendJson({ type: "get_group_info", group_id: groupId });
  await waitFor("group_info", () =>
    aliceConn.ws.messages.some((m) => m.type === "group_info")
  );
  const info = aliceConn.ws.messages.filter((m) => m.type === "group_info").pop();
  // Members are reported as { peer_id, role } objects (owner/admin/member).
  const infoMemberIds =
    info && Array.isArray(info.members) ? info.members.map((m) => m.peer_id) : [];
  check(
    "group: get_group_info returns owner + full member list with roles",
    info &&
      info.owner_peer_id === alice.peer_id &&
      info.name === "Ghost Squad" &&
      infoMemberIds.length === 2 &&
      infoMemberIds.includes(alice.peer_id) &&
      infoMemberIds.includes(bob.peer_id) &&
      info.members.some((m) => m.peer_id === alice.peer_id && m.role === "owner") &&
      info.members.some((m) => m.peer_id === bob.peer_id && m.role === "member")
  );

  // --- Group role management (promote / demote / remove) ---------------------
  // Alice (owner) promotes Bob to admin. The roster must reflect the new role.
  aliceConn.ws.sendJson({ type: "promote_member", group_id: groupId, peer_id: bob.peer_id });
  await waitFor("group_member_promoted", () =>
    aliceConn.ws.messages.some((m) => m.type === "group_member_promoted")
  );
  const promoted = aliceConn.ws.messages
    .filter((m) => m.type === "group_member_promoted")
    .pop();
  check(
    "group: owner promotes member -> group_member_promoted",
    promoted && promoted.group_id === groupId && promoted.peer_id === bob.peer_id
  );

  aliceConn.ws.sendJson({ type: "get_group_info", group_id: groupId });
  await waitFor("group_info after promote", () =>
    aliceConn.ws.messages.filter((m) => m.type === "group_info").length >= 2
  );
  const infoPromoted = aliceConn.ws.messages.filter((m) => m.type === "group_info").pop();
  check(
    "group: promoted member shows role admin in group_info",
    infoPromoted &&
      Array.isArray(infoPromoted.members) &&
      infoPromoted.members.some(
        (m) => m.peer_id === bob.peer_id && m.role === "admin"
      ) &&
      infoPromoted.members.some(
        (m) => m.peer_id === alice.peer_id && m.role === "owner"
      )
  );

  // Bob (now an admin) cannot demote or remove the owner: demote/remove are
  // owner-only, and the owner can never be touched.
  bob6.ws.sendJson({ type: "demote_member", group_id: groupId, peer_id: alice.peer_id });
  await waitFor("demote by admin -> not_owner", () =>
    bob6.ws.messages.some((m) => m.type === "error" && m.code === "not_owner")
  );
  check("group: admin demote -> not_owner (owner-only)", true);

  bob6.ws.sendJson({ type: "remove_member", group_id: groupId, peer_id: alice.peer_id });
  await waitFor("remove by admin -> not_owner", () =>
    bob6.ws.messages.some((m) => m.type === "error" && m.code === "not_owner")
  );
  check("group: admin remove -> not_owner (owner-only)", true);

  // The owner cannot demote themselves (also covered by the relay unit test
  // `owner_cannot_demote_themselves`).

  // The owner demotes Bob back to a regular member.
  aliceConn.ws.sendJson({ type: "demote_member", group_id: groupId, peer_id: bob.peer_id });
  await waitFor("group_member_demoted", () =>
    aliceConn.ws.messages.some((m) => m.type === "group_member_demoted")
  );
  const demoted = aliceConn.ws.messages
    .filter((m) => m.type === "group_member_demoted")
    .pop();
  check(
    "group: owner demotes admin -> group_member_demoted",
    demoted && demoted.group_id === groupId && demoted.peer_id === bob.peer_id
  );

  // A plain member cannot promote anyone (also covered by the relay unit test
  // `promote_member_rejects_regular_member_actor`).

  // The owner removes Bob. He is a member again after the demote, so this
  // verifies the owner-only remove path on a regular member.
  aliceConn.ws.sendJson({ type: "remove_member", group_id: groupId, peer_id: bob.peer_id });
  await waitFor("group_member_removed", () =>
    aliceConn.ws.messages.some((m) => m.type === "group_member_removed")
  );
  const removed = aliceConn.ws.messages
    .filter((m) => m.type === "group_member_removed")
    .pop();
  check(
    "group: owner removes member -> group_member_removed",
    removed && removed.group_id === groupId && removed.peer_id === bob.peer_id
  );

  // Re-add Bob so the leave_group test below still has a member to remove.
  aliceConn.ws.sendJson({ type: "add_group_member", group_id: groupId, peer_id: bob.peer_id });
  await waitFor("group_member_added (re-add)", () =>
    aliceConn.ws.messages.filter((m) => m.type === "group_member_added").length >= 2
  );
  check("group: re-added member acknowledged", true);

  // --- Group ownership transfer (transfer_ownership) -------------------------
  // bob is a plain member again: he cannot transfer ownership (owner-only).
  // The `not_owner` / `not_a_member` / `group_not_found` edge cases are also
  // covered by the relay's unit tests, so this block stays lean to fit the
  // per-IP `group:<ip>` rate budget.
  bob6.ws.sendJson({
    type: "transfer_ownership",
    group_id: groupId,
    new_owner_peer_id: carol.peer_id,
  });
  await waitFor("transfer by member -> not_owner", () =>
    bob6.ws.messages.some((m) => m.type === "error" && m.code === "not_owner")
  );
  check("group: member transfer_ownership -> not_owner (owner-only)", true);

  // The owner transfers ownership to bob; the reply names the new owner.
  aliceConn.ws.sendJson({
    type: "transfer_ownership",
    group_id: groupId,
    new_owner_peer_id: bob.peer_id,
  });
  await waitFor("ownership_transferred", () =>
    aliceConn.ws.messages.some((m) => m.type === "ownership_transferred")
  );
  const transferred = aliceConn.ws.messages
    .filter((m) => m.type === "ownership_transferred")
    .pop();
  check(
    "group: owner transfers ownership -> ownership_transferred",
    transferred &&
      transferred.group_id === groupId &&
      transferred.new_owner_peer_id === bob.peer_id
  );

  // get_group_info reflects the swap: bob owns the group, alice is an admin.
  aliceConn.ws.sendJson({ type: "get_group_info", group_id: groupId });
  await waitFor("group_info after transfer", () =>
    aliceConn.ws.messages.some(
      (m) =>
        m.type === "group_info" &&
        m.owner_peer_id === bob.peer_id &&
        Array.isArray(m.members) &&
        m.members.some((x) => x.peer_id === bob.peer_id && x.role === "owner") &&
        m.members.some((x) => x.peer_id === alice.peer_id && x.role === "admin")
    )
  );
  const infoTransferred = aliceConn.ws.messages
    .filter((m) => m.type === "group_info")
    .pop();
  check(
    "group: new owner visible in group_info (bob owner, alice admin)",
    infoTransferred &&
      infoTransferred.owner_peer_id === bob.peer_id &&
      infoTransferred.members.some(
        (m) => m.peer_id === bob.peer_id && m.role === "owner"
      ) &&
      infoTransferred.members.some(
        (m) => m.peer_id === alice.peer_id && m.role === "admin"
      )
  );

  // The new owner transfers ownership back to alice, restoring the state the
  // leave test below expects (alice owns the group again).
  bob6.ws.sendJson({
    type: "transfer_ownership",
    group_id: groupId,
    new_owner_peer_id: alice.peer_id,
  });
  await waitFor("ownership_transferred (back)", () =>
    bob6.ws.messages.filter((m) => m.type === "ownership_transferred").length >= 1
  );
  check("group: new owner transfers ownership back -> ownership_transferred", true);

  // (d) leave_group removes the member from the roster and revokes sends.
  bob6.ws.sendJson({ type: "leave_group", group_id: groupId });
  await waitFor("group_member_left", () =>
    bob6.ws.messages.some((m) => m.type === "group_member_left")
  );
  check("group: leave_group acknowledged", true);

  aliceConn.ws.sendJson({ type: "get_group_info", group_id: groupId });
  await waitFor("group_info after leave", () =>
    aliceConn.ws.messages.some(
      (m) =>
        m.type === "group_info" &&
        Array.isArray(m.members) &&
        m.members.length === 1 &&
        m.members[0].peer_id === alice.peer_id
    )
  );
  const infoAfter = aliceConn.ws.messages.filter((m) => m.type === "group_info").pop();
  const infoAfterMemberIds =
    infoAfter && Array.isArray(infoAfter.members)
      ? infoAfter.members.map((m) => m.peer_id)
      : [];
  check(
    "group: leave_group removes the member from the roster",
    infoAfter &&
      infoAfterMemberIds.length === 1 &&
      infoAfterMemberIds[0] === alice.peer_id &&
      infoAfter.members[0].role === "owner"
  );

  bob6.ws.sendJson({
    type: "send_group_message",
    group_id: groupId,
    envelope: {
      sender: bob.peer_id,
      recipient: "ignored-by-server",
      payload: Buffer.from("post-leave").toString("base64"),
      seq: 7003,
    },
  });
  await waitFor("not_a_member after leave", () =>
    bob6.ws.messages.some((m) => m.type === "error" && m.code === "not_a_member")
  );
  check("group: a left member cannot send -> not_a_member", true);

  // (e) unknown group_id -> group_not_found. An empty group name is rejected
  //     with invalid_group_name (also covered by the unit test
  //     `create_group_rejects_invalid_name`).
  aliceConn.ws.sendJson({ type: "get_group_info", group_id: "does-not-exist" });
  await waitFor("group_not_found", () =>
    aliceConn.ws.messages.some((m) => m.type === "error" && m.code === "group_not_found")
  );
  check("group: unknown group_id -> group_not_found", true);

  aliceConn.ws.close();
  bob6.ws.close();
  carolConn.ws.close();

  console.log(failures === 0 ? "\nALL TESTS PASSED" : `\n${failures} TEST(S) FAILED`);
  process.exit(failures === 0 ? 0 : 1);
}

main().catch((err) => {
  console.error("SMOKE TEST ERROR:", err.message);
  process.exit(1);
});
