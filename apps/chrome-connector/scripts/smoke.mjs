// SPDX-License-Identifier: AGPL-3.0-or-later
// Isolated browser acceptance fixture. Run the whole command outside the sandbox;
// the HTTP servers and every local browser client share this process/network.
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";

const root = path.resolve(import.meta.dirname, "../../..");
const require = createRequire(path.join(root, "package.json"));
const { chromium } = require("@playwright/test");
const profile = await fs.mkdtemp(path.join(os.tmpdir(), "ra-connector-smoke-"));
const extension = path.join(root, ".tmp/chrome-connector");
const items = [], attachments = [], errors = [];
const native = process.env.RA_CONNECTOR_NATIVE_ENDPOINT;
const retryPDF = process.env.RA_CONNECTOR_RETRY_SMOKE === "1" && !native;
let pdfRequests = 0;
if (native && native !== "http://127.0.0.1:23121/") throw new Error("Native smoke accepts only the isolated development endpoint");
if (native) {
  const preflight = await fetch(`${native}connector/ping`, {
    method: "GET",
    headers: {
      "Content-Type": "application/json",
      "X-Zotero-Connector-API-Version": "3",
      "X-Research-Assistant-Connector-API-Version": "2",
    },
    signal: AbortSignal.timeout(10_000),
  });
  if (!preflight.ok || !preflight.headers.get("X-Research-Assistant-Version")) {
    throw new Error(`Native preflight failed (HTTP ${preflight.status}); no credential or response body was printed`);
  }
  const payload = await preflight.json();
  if (payload.application !== "research-assistant" || payload.connectorApiVersion !== 2 || payload.environment !== "development") throw new Error("Native preflight returned an incompatible protocol response");
}
let context;
const listen = (server, port) => new Promise((resolve, reject) => {
  server.once("error", reject); server.listen(port, "127.0.0.1", () => resolve(server.address().port));
});
const close = server => new Promise(resolve => server.close(resolve));
const json = (response, value) => {
  response.writeHead(200, { "Content-Type": "application/json", "X-Research-Assistant-Version": "0.2.0", "X-Zotero-Version": "9.0.0",
    "X-Research-Assistant-Application": "research-assistant", "X-Research-Assistant-Connector-API-Version": "2", "X-Zotero-Connector-API-Version": "3", "X-Research-Assistant-Instance-ID": "b".repeat(32) });
  response.end(JSON.stringify(value));
};
const bridge = http.createServer(async (request, response) => {
  let body = Buffer.alloc(0);
  for await (const chunk of request) body = Buffer.concat([body, chunk]);
  const method = new URL(request.url, "http://127.0.0.1").pathname.split("/").at(-1);
  if (method !== "ping" && (request.headers["x-research-assistant-token"] !== "a".repeat(64) || request.headers["x-research-assistant-instance-id"] !== "b".repeat(32))) { response.writeHead(401); response.end(); return; }
  if (method === "ping") json(response, { application: "research-assistant", version: "0.2.0", connectorApiVersion: 2, zoteroConnectorApiVersion: 3,
    environment: "production", receiverId: "d".repeat(64), instanceId: "b".repeat(32), connectionToken: "a".repeat(64),
    prefs: { automaticSnapshots: false, downloadAssociatedFiles: true, reportActiveURL: false } });
  else if (method === "getSelectedCollection") json(response, { id: "L1", name: "Research Assistant", libraryEditable: true, filesEditable: true, targets: [{ id: "L1", name: "Research Assistant", level: 0, filesEditable: true }] });
  else if (method === "saveItems") { items.push(JSON.parse(body)); json(response, { items: [] }); }
  else if (method === "saveAttachment" || method === "saveStandaloneAttachment") {
    attachments.push({ bytes: body.length, sessionID: new URL(request.url, "http://127.0.0.1").searchParams.get("sessionID"), metadata: JSON.parse(request.headers["x-metadata"]) });
    json(response, { canRecognize: false });
  } else if (method === "hasAttachmentResolvers" || method === "getRecognizedItem") json(response, false);
  else json(response, {});
});
const fixture = http.createServer((request, response) => {
  if (request.url === "/paper.pdf") {
    pdfRequests++;
    if (retryPDF && pdfRequests === 1) {
      response.writeHead(403, { "Content-Type": "text/html" });
      response.end("<h1>Synthetic first download denied</h1>");
      return;
    }
    response.writeHead(200, { "Content-Type": "application/pdf" });
    response.end("%PDF-1.4\n1 0 obj<</Type/Catalog>>endobj\n%%EOF");
    return;
  }
  const url = `http://127.0.0.1:${fixture.address().port}`;
  response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
  if (request.url === "/multiple") {
    const coins = title => new URLSearchParams({ ctx_ver: "Z39.88-2004", rft_val_fmt: "info:ofi/fmt:kev:mtx:journal", "rft.genre": "article", "rft.atitle": title, "rft.jtitle": "Synthetic Acceptance Journal", "rft.au": "Example, Alice", "rft.date": "2026" }).toString().replaceAll("&", "&amp;");
    response.end(`<!doctype html><title>Multiple acceptance papers</title><span class="Z3988" title="${coins("First selectable paper")}"></span><span class="Z3988" title="${coins("Second selectable paper")}"></span>`);
    return;
  }
  response.end(`<!doctype html><title>Connector acceptance paper</title>
    <meta name="citation_title" content="Connector acceptance paper">
    <meta name="citation_author" content="Example, Alice">
    <meta name="citation_date" content="2026/10/16">
    <meta name="citation_journal_title" content="Synthetic Acceptance Journal">
    <meta name="citation_pdf_url" content="${url}/paper.pdf">
    <h1>Connector acceptance paper</h1><p>Synthetic public test data.</p>`);
});
try {
  if (!native) await listen(bridge, 23120);
  const port = await listen(fixture, 0);
  context = await chromium.launchPersistentContext(profile, {
    channel: "chromium", headless: true,
    args: [`--disable-extensions-except=${extension}`, `--load-extension=${extension}`],
  });
  context.on("weberror", error => errors.push(error.error().message));
  const worker = context.serviceWorkers()[0] || await context.waitForEvent("serviceworker", { timeout: 15_000 });
  worker.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await worker.evaluate(async ({ native }) => {
    await chrome.storage.local.setAccessLevel({ accessLevel: "TRUSTED_CONTEXTS" });
    await chrome.storage.local.set({ researchAssistantPrivate: { schemaVersion: 2, environment: native ? "development" : "production" } });
  }, { native: !!native });
  const page = await context.newPage();
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  page.on("pageerror", error => errors.push(error.message));
  const url = `http://127.0.0.1:${port}/article`;
  await page.goto(url);
  const waitUntil = async (check, label) => {
    for (let attempt = 0; attempt < 60; attempt++) {
      if (await check()) return;
      await new Promise(resolve => setTimeout(resolve, 500));
    }
    throw new Error(`Timeout: ${label}. Browser errors: ${errors.join(" | ")}`);
  };
  const saveWithDeadline = async url => {
    let timeout;
    try {
      return await Promise.race([
        worker.evaluate(async url => {
          const tab = (await chrome.tabs.query({})).find(tab => tab.url === url);
          const translators = Zotero.Connector_Browser._tabInfo[tab.id].translators;
          await Zotero.Connector_Browser.saveWithTranslator(tab, 0, {});
          return translators.map(translator => translator.label);
        }, url),
        new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error("Capture did not finish within 30s; check desktop availability or an open connection dialog")), 30_000); }),
      ]);
    } finally { clearTimeout(timeout); }
  };
  await waitUntil(() => worker.evaluate(() => globalThis.Zotero?.initialized), "background initialization");
  if (native) {
    // Observe only synthetic item payloads and successful attachment metadata; never headers/tokens.
    await worker.evaluate(() => {
      globalThis.smokeReceipts = { items: [], attachments: [] };
      const original = Zotero.Connector.callMethod;
      Zotero.Connector.callMethod = async function(options, data) {
        const result = await original(options, data);
        const method = typeof options === "string" ? options : options.method;
        if (method === "saveItems") globalThis.smokeReceipts.items.push(data);
        if (method === "saveAttachment" || method === "saveStandaloneAttachment") globalThis.smokeReceipts.attachments.push({ bytes: data.byteLength, metadata: JSON.parse(options.headers["X-Metadata"]) });
        return result;
      };
    });
  }
  await waitUntil(() => worker.evaluate(async url => {
    const tab = (await chrome.tabs.query({})).find(tab => tab.url === url);
    return !!Zotero.Connector_Browser._tabInfo[tab?.id]?.translators?.length;
  }, url), "upstream scholarly detection");
  const detected = await saveWithDeadline(url);
  if (native) {
    const receipts = await worker.evaluate(() => globalThis.smokeReceipts);
    items.push(...receipts.items);
    attachments.push(...receipts.attachments);
  }
  if (retryPDF) {
    assert.equal(items.length, 1, "metadata survives the denied first PDF download");
    assert.equal(attachments.length, 0);
    const progressFrame = page.frames().find(frame => frame.url().includes("/progressWindow/progressWindow.html"));
    assert.ok(progressFrame, "upstream closed-shadow progress iframe exists");
    const retry = progressFrame.getByRole("button", { name: "重试 PDF" });
    try { await retry.waitFor({ state: "visible", timeout: 5000 }); }
    catch (error) {
      const frames = [];
      for (const frame of page.frames()) frames.push({ url: frame.url(), text: await frame.locator("body").innerText().catch(() => "unavailable") });
      console.log(JSON.stringify({ retryDiagnostic: frames, errors }, null, 2));
      throw error;
    }
    await retry.click();
    await waitUntil(() => attachments.length === 1, "retry button uploads the PDF");
    await retry.waitFor({ state: "hidden", timeout: 10_000 });
    assert.equal(items.length, 1, "retry does not create another metadata item");
    assert.equal(attachments[0].sessionID, items[0].sessionID, "retry uses the original save session");
    assert.equal(pdfRequests, 2, "browser retried only the missing PDF");
  }
  await waitUntil(() => items.length > 0 && attachments.length > 0, "item and browser PDF transfer");
  assert.equal(items[0].items[0].title, "Connector acceptance paper");
  assert.equal(items[0].translatorID, "951c027d-74ac-47d4-a107-9c3069ab7b48");
  assert.ok(items[0].translatorVersion);
  assert.equal(attachments[0].metadata.parentItemID, items[0].items[0].id);
  assert.ok(attachments[0].bytes > 20);
  const prefs = await worker.evaluate(() => Zotero.Prefs.getAll());
  assert.equal("researchAssistantPrivate" in prefs, false);
  assert.equal(JSON.stringify(prefs).includes("connectionToken"), false);
  await fs.mkdir(path.join(root, "output/playwright"), { recursive: true });
  await page.screenshot({ path: path.join(root, "output/playwright/chrome-connector-capture.png") });
  let multiple = false;
  if (!native && process.env.RA_CONNECTOR_EXTENDED_SMOKE === "1") {
    const multiplePage = await context.newPage();
    multiplePage.on("pageerror", error => errors.push(error.message));
    const multipleURL = `http://127.0.0.1:${port}/multiple`;
    await multiplePage.goto(multipleURL);
    await waitUntil(() => worker.evaluate(async url => {
      const tab = (await chrome.tabs.query({})).find(tab => tab.url === url);
      return Zotero.Connector_Browser._tabInfo[tab?.id]?.translators?.[0]?.itemType === "multiple";
    }, multipleURL), "multiple-result detection");
    const selectorOpened = context.waitForEvent("page", { timeout: 15_000 });
    const action = saveWithDeadline(multipleURL);
    const selector = await selectorOpened;
    await selector.waitForURL("**/itemSelector/itemSelector.html**");
    await selector.locator('input[type="checkbox"]').nth(0).check();
    await selector.locator('input[type="checkbox"]').nth(1).uncheck();
    await selector.locator("#ok").click();
    await action;
    await waitUntil(() => items.length === 2, "selected item capture");
    assert.equal(items[1].items.length, 1);
    assert.equal(items[1].items[0].title, "First selectable paper");
    multiple = true;
  }
  const publicSites = [];
  if (!native && process.env.RA_CONNECTOR_REAL_SITE_SMOKE === "1") {
    const sites = [
      "https://arxiv.org/abs/2303.08774",
      "https://pubmed.ncbi.nlm.nih.gov/34265844/",
      "https://doi.org/10.1038/s41586-021-03819-2",
    ].filter(site => !process.env.RA_CONNECTOR_REAL_SITE_FILTER || site.includes(process.env.RA_CONNECTOR_REAL_SITE_FILTER));
    for (const site of sites) {
      const publicPage = await context.newPage();
      const result = { url: site, detected: [], metadataSaved: false, pdfSaved: false };
      const previousItems = items.length, previousAttachments = attachments.length;
      try {
        const navigation = await publicPage.goto(site, { waitUntil: "domcontentloaded", timeout: 20_000 });
        result.httpStatus = navigation?.status();
        if (result.httpStatus >= 400) throw new Error(`Public site returned HTTP ${result.httpStatus}`);
        const pageURL = publicPage.url();
        await waitUntil(() => worker.evaluate(async url => {
          const tab = (await chrome.tabs.query({})).find(tab => tab.url === url);
          return !!Zotero.Connector_Browser._tabInfo[tab?.id]?.translators?.length;
        }, pageURL), "public scholarly detection");
        result.detected = await saveWithDeadline(pageURL);
      } catch (error) {
        result.error = String(error.message).slice(0, 350);
        result.pageTitle = await publicPage.title().catch(() => "unavailable");
        result.finalURL = publicPage.url();
        result.headings = await publicPage.locator("h1").allTextContents().catch(() => []);
      }
      result.metadataSaved = items.length > previousItems;
      result.pdfSaved = attachments.length > previousAttachments;
      publicSites.push(result);
      await publicPage.close();
    }
  }
  console.log(JSON.stringify({ bridge: native ? "native-development" : "synthetic", initialized: true, detected, multiple, retryPDF, publicSites, items: items.length, attachments: attachments.length, errors: errors.slice(0, 15) }, null, 2));
} finally {
  await context?.close();
  await Promise.all([close(fixture), ...(native ? [] : [close(bridge)])]);
  await fs.rm(profile, { recursive: true, force: true });
}
