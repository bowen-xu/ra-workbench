// SPDX-License-Identifier: AGPL-3.0-or-later
// Run outside the sandbox with the browser/client. No real Chrome profile needed.
import fs from "node:fs/promises";
import http from "node:http";
import { fileURLToPath } from "node:url";

const build = new URL("../../../.tmp/chrome-connector/", import.meta.url);
const fixture = new URL("../tests/csp/", import.meta.url);
const original = new URL("../../../.tmp/chrome-connector-source/connector/src/browserExt/offscreen/offscreenSandbox.html", import.meta.url);
const policyOf = html => html.match(/http-equiv="Content-Security-Policy" content="([^"]+)"/)[1];
const policies = {
  fixed: policyOf(await fs.readFile(new URL("offscreen/offscreenSandbox.html", build), "utf8")),
  baseline: policyOf(await fs.readFile(original, "utf8")),
};
const assets = new Map(await Promise.all([
  ["/bootstrap.js", new URL("bootstrap.js", fixture)], ["/run.js", new URL("run.js", fixture)],
  ["/capture.js", new URL("offscreen/offscreenTranslate.js", build)],
  ["/overrides.js", new URL("offscreen/offscreenFunctionOverrides.js", build)],
].map(async ([route, file]) => [route, await fs.readFile(file)])));
const received = [];
export function cspFixtureHandler(request, response) {
  response.setHeader("Cache-Control", "no-store");
  if (!/^127\.0\.0\.1:\d+$/.test(request.headers.host ?? "")) { response.writeHead(400).end(); return; }
  if (!["GET", "HEAD"].includes(request.method)) { response.writeHead(405).end(); return; }
  const url = new URL(request.url, `http://${request.headers.host}`);
  if (url.pathname.startsWith("/blocked/")) { received.push(url.pathname); response.writeHead(204).end(); return; }
  if (url.pathname === "/network-count") { response.setHeader("Content-Type", "application/json"); response.end(JSON.stringify({ received })); return; }
  if (assets.has(url.pathname)) { response.setHeader("Content-Type", "text/javascript"); response.end(assets.get(url.pathname)); return; }
  if (url.pathname !== "/") { response.writeHead(404).end(); return; }
  const mode = url.searchParams.has("baseline") ? "baseline" : "fixed";
  response.setHeader("Content-Type", "text/html; charset=utf-8");
  // Match the packaged page's delivery mechanism. An HTTP header would hide
  // warnings about directives that are unsupported in a meta policy.
  response.end(`<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${policies[mode]}"><title>Connector CSP regression</title></head><body>
    <h1>Connector CSP regression — ${mode}</h1><p id="status" role="status">Running…</p><ol id="results"></ol>
    <p><a href="/?baseline=1">Original policy</a> · <a href="/">Fixed policy</a></p>
    <script src="/bootstrap.js"></script><script src="/overrides.js"></script><script src="/capture.js"></script><script src="/run.js"></script>
    </body></html>`);
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const server = http.createServer(cspFixtureHandler);
  server.listen(0, "127.0.0.1", () => console.log(`CSP fixture: http://127.0.0.1:${server.address().port}/`));
  for (const signal of ["SIGINT", "SIGTERM"]) process.once(signal, () => { server.close(); server.closeAllConnections(); });
}
