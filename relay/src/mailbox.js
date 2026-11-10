// The mailbox rules (docs/sync-protocol.md §5), kept free of Cloudflare APIs
// so they can be tested with plain Node (relay/test/mailbox.test.mjs).
//
// A mailbox holds one encrypted value and a version number. The relay never
// sees the key: it stores ciphertext and the SHA-256 of the mailbox token.

export const MAX_BODY = 1024 * 1024;
/** A mailbox nobody has touched for this long is deleted. */
export const EXPIRY_MS = 180 * 24 * 60 * 60 * 1000;
export const MAILBOX_ID = /^[0-9a-f]{64}$/;

const B64 = /^[A-Za-z0-9+/]+={0,2}$/;

/** SHA-256 hex of the token (Web Crypto; works in Workers and Node 20+). */
export async function tokenHash(token) {
  const data = new TextEncoder().encode(token);
  const digest = await crypto.subtle.digest("SHA-256", data);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/** The bearer token from an Authorization header, or "". */
export function bearer(header) {
  const m = /^Bearer\s+([A-Za-z0-9_-]{20,100})$/.exec(header || "");
  return m ? m[1] : "";
}

function parsePut(body) {
  let v;
  try {
    v = JSON.parse(body);
  } catch {
    return null;
  }
  if (!v || typeof v !== "object") return null;
  const { baseVersion, nonce, ciphertext } = v;
  if (!Number.isSafeInteger(baseVersion) || baseVersion < 0) return null;
  if (typeof nonce !== "string" || nonce.length !== 16 || !B64.test(nonce)) return null;
  if (typeof ciphertext !== "string" || ciphertext.length < 24 || !B64.test(ciphertext)) return null;
  return { baseVersion, nonce, ciphertext };
}

/**
 * Applies one request to a mailbox. Synchronous, so a Durable Object can run
 * it between one storage read and one storage write without interleaving.
 *
 * @param {{method: string, hash: string, body: string}} req  hash = tokenHash(bearer token), "" if missing
 * @param {null | {tokenHash: string, version: number, nonce?: string, ciphertext?: string, closed?: boolean}} state
 * @returns {{status: number, json?: object, state?: object|null, touch?: boolean}}
 *   `state` is the new state to store (omitted = unchanged); `touch` = reset the expiry timer.
 */
export function handle(req, state) {
  const { method, hash, body } = req;
  if (!["GET", "PUT", "DELETE"].includes(method)) return { status: 405 };
  if (!hash) return { status: 401 };
  if (state && state.tokenHash !== hash) return { status: 401 };
  if (state && state.closed) return { status: 410, touch: true };

  if (method === "GET") {
    if (!state) return { status: 404 };
    return { status: 200, json: { version: state.version, nonce: state.nonce, ciphertext: state.ciphertext }, touch: true };
  }

  if (method === "DELETE") {
    // Closing leaves a marker so other devices learn they were disconnected.
    return { status: 204, state: { tokenHash: hash, version: state ? state.version : 0, closed: true }, touch: true };
  }

  // PUT
  if (body.length > MAX_BODY) return { status: 413 };
  const put = parsePut(body);
  if (!put) return { status: 400 };
  const current = state ? state.version : 0;
  // A missing mailbox (new, or expired) accepts any base: the writer's data
  // is the best there is.
  if (state && put.baseVersion !== current) {
    return { status: 409, json: { version: current, nonce: state.nonce, ciphertext: state.ciphertext }, touch: true };
  }
  const version = current + 1;
  return {
    status: 200,
    json: { version },
    state: { tokenHash: hash, version, nonce: put.nonce, ciphertext: put.ciphertext },
    touch: true,
  };
}
