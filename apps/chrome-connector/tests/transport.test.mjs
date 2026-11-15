// SPDX-License-Identifier: AGPL-3.0-or-later
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import { createTransport, descriptor, fixture, response, STORAGE_KEY } from "./connection-fixture.mjs";

test("fresh installation probes only its fixed production address with no private payload or token", async () => {
  const calls = [];
  const values = {};
  const request = createTransport({ get: async () => structuredClone(values), set: async data => Object.assign(values, data) }, async (url, options) => {
    calls.push({ url, options }); return response(descriptor({ environment: "production" }));
  }, "0.1.0");
  const status = await request("ping", { secret: "must never be sent" });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "http://127.0.0.1:23120/connector/ping");
  assert.equal(calls[0].options.method, "GET");
  assert.equal(calls[0].options.body, undefined);
  assert.equal(calls[0].options.headers["X-Research-Assistant-Token"], undefined);
  assert.equal(calls[0].options.redirect, "error");
  assert.equal(calls[0].options.credentials, "omit");
  assert.equal(status.application, "research-assistant");
  assert.equal(status.connectionToken, undefined);
  assert.equal(JSON.stringify(values).includes("connectionToken"), false);
});

test("a verified receiver precedes every metadata and PDF upload and caller headers cannot override it", async () => {
  const calls = [];
  const { request } = fixture(async (url, options) => {
    calls.push({ url, options });
    return response(url.endsWith("/ping") ? descriptor() : { items: [] });
  });
  await request("saveItems", { sessionID: "one", items: [{ id: "paper", title: "Example", detailedCookies: [{ value: "private-cookie" }] }], cookie: "private-cookie" });
  await request({ method: "saveAttachment", queryString: "sessionID=one", headers: {
    "X-Research-Assistant-Token": "caller-token", "X-Metadata": JSON.stringify({ id: "pdf", title: "PDF", headers: { Authorization: "private-cookie" } }),
  } }, new ArrayBuffer(5));
  assert.deepEqual(calls.map(call => new URL(call.url).pathname), ["/connector/ping", "/connector/saveItems", "/connector/ping", "/connector/saveAttachment"]);
  assert.equal(calls[1].options.headers["X-Research-Assistant-Token"], "c".repeat(64));
  assert.equal(calls[3].options.headers["X-Research-Assistant-Instance-ID"], "b".repeat(32));
  assert.equal(calls[3].options.headers["X-Research-Assistant-Token"], "c".repeat(64));
  assert.equal(JSON.stringify(calls).includes("private-cookie"), false);
});

test("unrelated, incomplete, malformed, oversized, incompatible or wrong-environment discovery sends no capture data", async () => {
  const cases = [
    () => new Response("<html>Other software</html>", { headers: { "Content-Type": "text/html" } }),
    () => new Response("{}", { headers: { "Content-Type": "application/json" } }),
    () => response(descriptor(), { headers: { "X-Research-Assistant-Application": "another-application" } }),
    () => response({}),
    () => response(descriptor({ connectionToken: "invalid" })),
    () => response(descriptor({ version: "other-version" })),
    () => response(descriptor({ environment: "production" })),
    () => response(descriptor(), { headers: { "X-Research-Assistant-Connector-API-Version": "1" } }),
    () => response({ ...descriptor(), huge: "a".repeat(70 * 1024) }),
    () => { const valid = response({}); return new Response("not-json", { headers: valid.headers }); },
  ];
  for (const makeResponse of cases) {
    const calls = [];
    const { request, values } = fixture(async (url, options) => { calls.push({ url, options }); return makeResponse(); });
    await assert.rejects(request("saveItems", { sessionID: "kept", items: [{ title: "private-paper" }] }));
    await assert.rejects(request({ method: "saveAttachment", queryString: "sessionID=kept", headers: { "X-Metadata": '{"title":"private-paper"}' } }, new ArrayBuffer(2)));
    assert.ok(calls.every(call => call.url.endsWith("/ping") && !call.options.body && !call.options.headers["X-Metadata"]));
    assert.equal(JSON.stringify(calls).includes("private-paper"), false);
    assert.equal(JSON.stringify(values).includes("private-paper"), false);
  }
});

test("retrying a lost response from the same capture preserves request identities without storing literature", async () => {
  let fail = true;
  const bodies = [];
  const { request, values } = fixture(async (url, options) => {
    if (url.endsWith("/ping")) return response(descriptor());
    bodies.push(JSON.parse(options.body));
    if (fail) throw new Error("response lost");
    return response({ items: [] });
  });
  const payload = { sessionID: "one", items: [{ id: "original", cookie: "private" }] };
  await assert.rejects(request("saveItems", payload));
  assert.equal(bodies[0].sessionID, "one");
  assert.equal(bodies[0].items[0].id, "original");
  assert.equal(JSON.stringify(bodies).includes("private"), false);
  assert.equal(JSON.stringify(values).includes("original"), false);
  assert.equal(values[STORAGE_KEY].pending, undefined);
  fail = false;
  await request("saveItems", payload);
  assert.deepEqual(bodies[0], bodies[1]);
  assert.equal(values[STORAGE_KEY].pending, undefined);
});

test("repeated offline captures have no abandoned queue limit or settings recovery requirement", async () => {
  const { request, values } = fixture(async () => { throw new Error("offline"); });
  const results = await Promise.allSettled(Array.from({length: 101}, (_, index) => request("saveItems", { sessionID: `offline-${index}`, items: [] })));
  assert.ok(results.every(result => result.status === "rejected" && result.reason.status === 503 && !result.reason.message.includes("插件设置")));
  assert.equal(values[STORAGE_KEY].pending, undefined);
});

test("a cached successful ping cannot cause a later PDF upload to an unrelated service", async () => {
  let changed = false;
  const writes = [];
  const { request } = fixture(async (url, options) => {
    if (!url.endsWith("/ping")) writes.push(options.body);
    return changed ? new Response("Other service") : response(descriptor());
  });
  await request("ping", {});
  changed = true;
  await assert.rejects(request({ method: "saveAttachment", queryString: "sessionID=one" }, new ArrayBuffer(16)));
  assert.equal(writes.length, 0);
});

test("capture sessions cannot move into another receiver at the same endpoint", async () => {
  const calls = [];
  const initial = { schemaVersion: 2, environment: "development", receiverId: "d".repeat(64) };
  const { request, values } = fixture(async url => { calls.push(url); return response(descriptor()); }, initial);
  await assert.rejects(request("saveItems", { sessionID: "one", items: [] }), error => error.status === 409 && !error.message.includes("重置"));
  assert.ok(calls.every(url => url.endsWith("/ping")));
  assert.equal(values[STORAGE_KEY].receiverId, "d".repeat(64));
});

test("a changed instance in the write response cannot acknowledge the capture", async () => {
  const { request } = fixture(async url => response(url.endsWith("/ping") ? descriptor() : {}, {
    headers: url.endsWith("/ping") ? {} : { "X-Research-Assistant-Instance-ID": "d".repeat(32) },
  }));
  await assert.rejects(request("saveItems", { sessionID: "one", items: [] }), error => error.status === 409);
});

test("an acknowledged library binding also prevents a PDF from moving to another receiver", async () => {
  const calls = [];
  const { request } = fixture(async url => { calls.push(url); return response(descriptor()); }, {
    schemaVersion: 2, environment: "development", receiverId: "d".repeat(64),
  });
  await assert.rejects(request({ method: "saveAttachment", queryString: "sessionID=already-saved" }, new ArrayBuffer(5)), error => error.status === 409);
  assert.ok(calls.every(url => url.endsWith("/ping")));
});

test("legacy settings preserve receiver identity without retaining the removed queue or credential", async () => {
  const legacyToken = "a".repeat(64);
  const receiverId = createHash("sha256").update(legacyToken).digest("hex");
  const { request, values } = fixture(async () => response(descriptor({ receiverId })), {
    environment: "development", token: legacyToken, pending: [{ sessionID: "legacy", payload: { sessionID: "legacy", items: [] } }],
  });
  await request("ping", {});
  assert.equal(values[STORAGE_KEY].receiverId, receiverId);
  assert.equal(values[STORAGE_KEY].pending, undefined);
  assert.equal(values[STORAGE_KEY].token, undefined);
});

test("unsupported methods, arbitrary queries, malformed metadata and oversized PDFs fail before probing", async () => {
  let calls = 0;
  const { request } = fixture(async () => { calls++; return response(descriptor()); });
  await assert.rejects(request("https://api.zotero.org/items", {}), error => error.status === 501);
  await assert.rejects(request({ method: "saveAttachment", queryString: "url=https://example.org" }, new ArrayBuffer(1)), error => error.status === 400);
  await assert.rejects(request({ method: "saveAttachment", headers: { "X-Metadata": "null" } }, new ArrayBuffer(1)), error => error.status === 400);
  await assert.rejects(request("saveAttachment", new ArrayBuffer(64 * 1024 * 1024 + 1)), error => error.status === 413);
  assert.equal(calls, 0);
});
