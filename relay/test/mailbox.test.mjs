import { test } from "node:test";
import assert from "node:assert/strict";
import { handle, bearer, tokenHash, MAX_BODY } from "../src/mailbox.js";

const NONCE = "AAECAwQFBgcICQoL";
const CT = "UcqqKN+j0S2YVtLI2StUerwpqF/KNKjI1Hzcac50dzPtEw5tYv5gpse67A==";
const body = (baseVersion, ciphertext = CT) => JSON.stringify({ baseVersion, nonce: NONCE, ciphertext });

/** Runs requests against one in-memory mailbox, like the Durable Object does. */
function mailbox() {
  let state = null;
  return {
    get state() {
      return state;
    },
    send(method, hash, b = "") {
      const r = handle({ method, hash, body: b }, state);
      if (r.state !== undefined) state = r.state;
      return r;
    },
  };
}

test("token hash matches the protocol test vector", async () => {
  assert.equal(
    await tokenHash("AJz6SLfpq2jeFQJFKtPqDMJjLGr4HmRTVsv4YRMp_Cw"),
    "81d430e62ca3afa96592b7a7c50f6213ccbde0fd21a6cebfa6b0d5038facb16c",
  );
});

test("bearer parsing", () => {
  assert.equal(bearer("Bearer AJz6SLfpq2jeFQJFKtPqDMJjLGr4HmRTVsv4YRMp_Cw"), "AJz6SLfpq2jeFQJFKtPqDMJjLGr4HmRTVsv4YRMp_Cw");
  assert.equal(bearer("Basic abc"), "");
  assert.equal(bearer(""), "");
  assert.equal(bearer("Bearer short"), "");
});

test("empty mailbox: 404, then the first write creates it", () => {
  const m = mailbox();
  assert.equal(m.send("GET", "h1").status, 404);
  const put = m.send("PUT", "h1", body(0));
  assert.equal(put.status, 200);
  assert.deepEqual(put.json, { version: 1 });
  const get = m.send("GET", "h1");
  assert.equal(get.status, 200);
  assert.deepEqual(get.json, { version: 1, nonce: NONCE, ciphertext: CT });
});

test("compare-and-swap: a stale base gets 409 with the current value", () => {
  const m = mailbox();
  m.send("PUT", "h1", body(0));
  assert.equal(m.send("PUT", "h1", body(1)).status, 200);
  const stale = m.send("PUT", "h1", body(1));
  assert.equal(stale.status, 409);
  assert.equal(stale.json.version, 2);
  assert.equal(stale.json.ciphertext, CT);
  assert.equal(m.state.version, 2, "nothing was overwritten");
});

test("a different token can't read or write", () => {
  const m = mailbox();
  m.send("PUT", "h1", body(0));
  assert.equal(m.send("GET", "other").status, 401);
  assert.equal(m.send("PUT", "other", body(1)).status, 401);
  assert.equal(m.send("DELETE", "other").status, 401);
  assert.equal(m.send("GET", "").status, 401, "no token");
});

test("closing leaves a 410 marker for the other devices", () => {
  const m = mailbox();
  m.send("PUT", "h1", body(0));
  assert.equal(m.send("DELETE", "h1").status, 204);
  assert.equal(m.send("GET", "h1").status, 410);
  assert.equal(m.send("PUT", "h1", body(1)).status, 410);
  assert.equal(m.state.ciphertext, undefined, "the dictionary is gone");
});

test("bad requests", () => {
  const m = mailbox();
  assert.equal(m.send("POST", "h1").status, 405);
  assert.equal(m.send("PUT", "h1", "not json").status, 400);
  assert.equal(m.send("PUT", "h1", JSON.stringify({ baseVersion: -1, nonce: NONCE, ciphertext: CT })).status, 400);
  assert.equal(m.send("PUT", "h1", JSON.stringify({ baseVersion: 0, nonce: "short", ciphertext: CT })).status, 400);
  assert.equal(m.send("PUT", "h1", "x".repeat(MAX_BODY + 1)).status, 413);
  assert.equal(m.state, null, "nothing stored");
});

test("expired or missing mailbox accepts a write with any base", () => {
  const m = mailbox();
  assert.equal(m.send("PUT", "h1", body(7)).status, 200);
  assert.equal(m.state.version, 1);
});
