// SPDX-License-Identifier: AGPL-3.0-or-later
// Local HTTP service and all clients must run outside the sandbox together.
import assert from "node:assert/strict";
import http from "node:http";
import test from "node:test";
import { descriptor, fixture } from "./connection-fixture.mjs";

async function listen(handler, port = 0) {
  const server = http.createServer((request, response) => { response.setHeader("Connection", "close"); return handler(request, response); });
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(port, "127.0.0.1", resolve); });
  return server;
}
function fixtureFetch(server) {
  const port = server.address().port;
  return (url, options) => {
    const target = new URL(url);
    assert.equal(target.origin, "http://127.0.0.1:23121");
    target.port = String(port);
    return fetch(target, options);
  };
}
async function close(server) {
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
}

test("an unrelated application on a real loopback HTTP port receives only empty discovery GETs", async () => {
  const calls = [];
  const server = await listen(async (request, response) => {
    let size = 0;
    for await (const chunk of request) size += chunk.length;
    calls.push({ method: request.method, path: request.url, size, metadata: request.headers["x-metadata"], token: request.headers["x-research-assistant-token"] });
    response.writeHead(200, { "Content-Type": "text/html" }); response.end("<title>Unrelated application</title>");
  });
  try {
    const { request, values } = fixture(fixtureFetch(server));
    await assert.rejects(request("saveItems", { sessionID: "pending", items: [{ title: "private-title" }] }), error => error.status === 502);
    await assert.rejects(request({ method: "saveAttachment", queryString: "sessionID=pending", headers: { "X-Metadata": '{"title":"private-title"}' } }, new ArrayBuffer(64)), error => error.status === 502);
    assert.equal(calls.length, 2);
    assert.ok(calls.every(call => call.method === "GET" && call.path === "/connector/ping" && call.size === 0 && call.metadata === undefined && call.token === undefined));
    assert.equal(JSON.stringify(values).includes("private-title"), false);
  } finally { await close(server); }
});

test("a port replaced by another application after a successful discovery is rechecked before PDF transfer", async () => {
  const first = await listen((_request, response) => {
    response.writeHead(200, { "Content-Type": "application/json", "X-Research-Assistant-Application": "research-assistant",
      "X-Research-Assistant-Version": "0.2.0", "X-Research-Assistant-Connector-API-Version": "2", "X-Zotero-Connector-API-Version": "3", "X-Research-Assistant-Instance-ID": "b".repeat(32) });
    response.end(JSON.stringify(descriptor()));
  });
  const port = first.address().port;
  const { request } = fixture(fixtureFetch(first));
  try { await request("ping", {}); } finally { await close(first); }
  const calls = [];
  const other = await listen((incoming, response) => { calls.push({ method: incoming.method, path: incoming.url }); response.end("Another application"); }, port);
  try {
    await assert.rejects(request({ method: "saveAttachment", queryString: "sessionID=old" }, new ArrayBuffer(64)), error => error.status === 502 || error.status === 503);
    if (!calls.length) await assert.rejects(request({ method: "saveAttachment", queryString: "sessionID=old" }, new ArrayBuffer(64)), error => error.status === 502);
    assert.equal(calls.length, 1);
    assert.ok(calls.every(call => call.method === "GET" && call.path === "/connector/ping"));
  } finally { await close(other); }
});
