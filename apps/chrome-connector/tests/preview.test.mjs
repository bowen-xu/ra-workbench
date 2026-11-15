// SPDX-License-Identifier: AGPL-3.0-or-later
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import vm from "node:vm";
import test from "node:test";
import ts from "typescript";
import { createPreviewHandler } from "../scripts/preview-server.mjs";

const output = new URL("../../../.tmp/chrome-connector/", import.meta.url);
async function request(url, overrides = {}) {
  const result = { headers: {} };
  const response = {
    setHeader: (key, value) => { result.headers[key] = value; },
    writeHead: code => { result.code = code; return response; },
    end: body => { result.body = body?.toString() ?? ""; },
  };
  await (await createPreviewHandler())({ method: "GET", headers: { host: "127.0.0.1:5187" }, url, ...overrides }, response);
  return result;
}

test("preview uses the exact production pages and CSS, adding only platform scripts", async () => {
  for (const file of ["research-assistant/options.html", "progressWindow/progressWindow.html", "itemSelector/itemSelector.html"]) {
    const result = await request(`/extension/${file}`);
    const original = await fs.readFile(new URL(file, output), "utf8");
    assert.equal(result.code, 200);
    assert.equal(result.body.replace('<script src="/preview/fixtures.js"></script><script src="/preview/platform.js"></script>', ""), original);
  }
  for (const file of ["research-assistant/options.css", "progressWindow/progressWindow.css", "itemSelector/itemSelector.css", "ui/ProgressWindow.js"]) {
    const result = await request(`/extension/${file}`);
    assert.equal(result.body, await fs.readFile(new URL(file, output), "utf8"));
  }
});

test("preview has no network port to the desktop and cannot serve other workspace files", async () => {
  const response = await request("/");
  assert.match(response.headers["Content-Security-Policy"], /connect-src 'none'/);
  for (const url of ["/extension/%2e%2e%2fpackage.json", "/extension/background.html", "/package.json"]) {
    assert.equal((await request(url)).code, 404);
  }
  assert.equal((await request("/", { method: "POST" })).code, 405);
  assert.equal((await request("/", { headers: { host: "example.invalid:5187" } })).code, 400);
});

test("preview ports are absent from the actual loadable extension", async () => {
  const manifest = await fs.readFile(new URL("manifest.json", output), "utf8");
  assert.equal(manifest.includes("preview"), false);
  for (const file of ["research-assistant/options.html", "progressWindow/progressWindow.html", "itemSelector/itemSelector.html"]) {
    assert.equal((await fs.readFile(new URL(file, output), "utf8")).includes("/preview/"), false);
  }
});

test("actual transport discovers an in-memory receiver without delegating to a real fetch", async () => {
  let realRequests = 0;
  const context = { location: { origin: "http://127.0.0.1:5187", search: "" },
    URL, URLSearchParams, Headers, Response, AbortSignal, structuredClone, TextEncoder, TextDecoder, Uint8Array,
    fetch: () => { realRequests++; throw new Error("Unexpected real request"); },
    addEventListener() {},
  };
  context.window = context; context.parent = context;
  vm.createContext(context);
  for (const file of ["../preview/fixtures.ts", "../preview/platform.ts", "../src/connection.ts", "../src/transport.ts"]) {
    const source = await fs.readFile(new URL(file, import.meta.url), "utf8");
    vm.runInContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText, context);
  }
  const transport = vm.runInContext("ResearchAssistant.createTransport(chrome.storage.local, fetch, '0.1.0')", context);
  assert.equal((await transport("ping", {})).application, "research-assistant");
  await transport("saveItems", { sessionID: "preview-capture-1", items: [] });
  const config = (await context.chrome.storage.local.get("researchAssistantPrivate")).researchAssistantPrivate;
  assert.equal(config.pending, undefined);
  assert.equal(JSON.stringify(config).includes("preview-capture-1"), false);
  await assert.rejects(context.fetch("https://example.invalid/"), /blocked/);
  assert.equal(realRequests, 0);
});
