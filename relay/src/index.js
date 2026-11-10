// Wirl sync relay: one Durable Object per mailbox. See docs/sync-protocol.md.
import { handle, bearer, tokenHash, MAX_BODY, EXPIRY_MS, MAILBOX_ID } from "./mailbox.js";

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/" || url.pathname === "/health") {
      return new Response("Wirl sync relay\n", { headers: { "content-type": "text/plain" } });
    }
    const m = /^\/v2\/m\/([^/]+)$/.exec(url.pathname);
    if (!m || !MAILBOX_ID.test(m[1])) return new Response(null, { status: 404 });
    if (Number(request.headers.get("content-length") || 0) > MAX_BODY) return new Response(null, { status: 413 });
    const stub = env.MAILBOX.get(env.MAILBOX.idFromName(m[1]));
    return stub.fetch(request);
  },
};

export class Mailbox {
  constructor(ctx) {
    this.ctx = ctx;
  }

  async fetch(request) {
    const body = request.method === "PUT" ? await request.text() : "";
    const token = bearer(request.headers.get("authorization"));
    // Hash first: everything after this is storage only, so requests to the
    // same mailbox can't interleave between the read and the write.
    const hash = token ? await tokenHash(token) : "";
    const state = (await this.ctx.storage.get("m")) ?? null;
    const r = handle({ method: request.method, hash, body }, state);
    if (r.state !== undefined) await this.ctx.storage.put("m", r.state);
    if (r.touch) await this.ctx.storage.setAlarm(Date.now() + EXPIRY_MS);
    if (r.json) return Response.json(r.json, { status: r.status });
    return new Response(null, { status: r.status });
  }

  /** Nobody touched this mailbox for 180 days. */
  async alarm() {
    await this.ctx.storage.deleteAll();
  }
}
