// SPDX-License-Identifier: AGPL-3.0-or-later
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import vm from "node:vm";
import test from "node:test";
import { createRequire } from "node:module";
import { replaceOnce } from "../scripts/patch-upstream.mjs";
import { cspFixtureHandler } from "../scripts/csp-fixture-server.mjs";

const output = path.resolve(import.meta.dirname, "../../../.tmp/chrome-connector");
const manifest = JSON.parse(await fs.readFile(path.join(output, "manifest.json"), "utf8"));
const require = createRequire(import.meta.url);

test("MV3 output contains the production upstream inject/offscreen/download pipeline", async () => {
  assert.equal(manifest.manifest_version, 3);
  assert.equal(manifest.name, "Research Assistant Connector");
  assert.equal(manifest.minimum_chrome_version, "116");
  assert.equal(manifest.content_scripts.length, 1);
  assert.ok(manifest.content_scripts[0].js.includes("inject/virtualOffscreenTranslate.js"));
  assert.ok(manifest.content_scripts[0].js.includes("itemSaver.js"));
  assert.ok(!JSON.stringify(manifest).includes("zotero-google-docs-integration"));
  assert.ok(!manifest.declarative_net_request);
  for (const name of manifest.content_scripts[0].js) await fs.access(path.join(output, name));
  for (const name of [manifest.background.service_worker, manifest.options_ui.page, "offscreen/offscreenSandbox.html", "itemSelector/itemSelector.html", "COPYING"]) await fs.access(path.join(output, name));
});

test("the full pinned translator bundle preserves per-file licensing and scholarly/import fallbacks", async () => {
  const metadata = JSON.parse(await fs.readFile(path.join(output, "translators/metadata.json"), "utf8"));
  for (const label of ["arXiv.org", "PubMed", "Embedded Metadata", "DOI", "Crossref REST", "RIS", "BibTeX", "COinS"]) assert.ok(metadata.some(item => item.label === label), label);
  assert.equal(metadata.length, 750);
  const inventory = JSON.parse(await fs.readFile(path.join(output, "translators/licenses.json"), "utf8"));
  assert.equal(inventory.length, metadata.length);
  assert.equal(inventory.filter(item => item.license === "GPL-3.0-or-later").length, 35);
  assert.equal(inventory.filter(item => item.license === "GPL-2.0-or-later").length, 1);
  assert.equal(inventory.filter(item => item.license === "Apache-2.0").length, 1);
  assert.ok(inventory.every(item => item.compatible && item.sha256.length === 64));
  for (const item of metadata) {
    const text = await fs.readFile(path.join(output, "translators", `${item.translatorID}.js`), "utf8");
    assert.ok(text.includes(item.translatorID));
  }
});

test("cloud/account service implementation is removed, private storage never enters upstream Prefs", async () => {
  const api = await fs.readFile(path.join(output, "api.js"), "utf8");
  assert.equal(api.includes("https://api.zotero.org"), false);
  const adapter = await fs.readFile(path.join(output, "research-assistant/background.js"), "utf8");
  assert.ok(adapter.includes("TRUSTED_CONTEXTS"));
  assert.ok(adapter.includes("delete Zotero.Prefs.syncStorage[ResearchAssistant.STORAGE_KEY]"));
  const config = await fs.readFile(path.join(output, "zotero_config.js"), "utf8");
  assert.equal(config.includes("CLIENT_SECRET"), false);
});

test("upstream saving does not use cloud fallback when local capture fails", async () => {
  const source = await fs.readFile(path.join(output, "itemSaver.js"), "utf8");
  const context = { Zotero: { ItemSaver: {}, debug() {} }, document: {} };
  vm.createContext(context);
  vm.runInContext(source, context);
  const saver = new context.Zotero.ItemSaver({});
  let called = false;
  saver._saveToZotero = async () => { throw { status: 0 }; };
  saver._saveToServer = async () => { called = true; };
  await assert.rejects(saver.saveItems([]));
  assert.equal(called, false);
});

test("source drift fails closed instead of silently producing a partially adapted extension", () => {
  assert.throws(() => replaceOnce("missing", "expected", "replacement", "test"), /Upstream drift/);
  assert.throws(() => replaceOnce("expected expected", "expected", "replacement", "test"), /Upstream drift/);
});

test("the translator sandbox accepts captured inline styles while direct network and inline scripts stay blocked", async () => {
  const html = await fs.readFile(path.join(output, "offscreen/offscreenSandbox.html"), "utf8");
  const policy = html.match(/http-equiv="Content-Security-Policy" content="([^"]+)"/)[1];
  const directives = new Map(policy.split(";").map(value => value.trim().split(/\s+/)).filter(([name]) => name).map(([name, ...sources]) => [name, sources]));
  assert.deepEqual(directives.get("default-src"), ["'none'"]);
  assert.deepEqual(directives.get("style-src"), ["'unsafe-inline'"]);
  assert.deepEqual(directives.get("script-src"), ["'self'", "'unsafe-eval'"]);
  assert.deepEqual(directives.get("form-action"), ["'none'"]);
  for (const unsupported of ["frame-ancestors", "sandbox", "report-uri"]) assert.equal(directives.has(unsupported), false);
  for (const resource of ["connect-src", "img-src", "font-src", "media-src", "object-src", "frame-src", "worker-src"]) {
    assert.deepEqual(directives.get(resource) ?? directives.get("default-src"), ["'none'"]);
  }
  assert.equal(manifest.content_security_policy.extension_pages, "script-src 'self'; object-src 'self'");
  assert.deepEqual(manifest.sandbox.pages, ["offscreen/offscreenSandbox.html"]);
  assert.ok(manifest.web_accessible_resources.every(entry => entry.resources.every(resource => resource !== "*" && !resource.startsWith("offscreen/"))));
});

test("browser CSP regression uses the production meta policy instead of an HTTP-header substitute", async () => {
  const headers = {};
  let body;
  cspFixtureHandler({ method: "GET", url: "/", headers: { host: "127.0.0.1:5187" } }, {
    setHeader(name, value) { headers[name] = value; }, end(value) { body = value; },
  });
  const html = await fs.readFile(path.join(output, "offscreen/offscreenSandbox.html"), "utf8");
  const meta = html.match(/<meta http-equiv="Content-Security-Policy" content="[^"]+">/)[0];
  assert.equal(headers["Content-Security-Policy"], undefined);
  assert.ok(body.includes(meta));
  assert.ok(body.indexOf(meta) < body.indexOf('<script src="/bootstrap.js">'));
});

test("actual progress component keeps connection failures concise without appending the save destination", async () => {
  const React = require("react");
  const { renderToStaticMarkup } = require("react-dom/server");
  const Zotero = { getExtensionURL: path => path, getString: key => key };
  const context = { window: { Zotero }, Zotero, React, PropTypes: require("prop-types"), clearTimeout, setTimeout };
  vm.createContext(context);
  vm.runInContext(await fs.readFile(path.join(output, "ui/ProgressWindow.js"), "utf8"), context);
  class CaptureProgress extends Zotero.UI.ProgressWindow {
    constructor(props) {
      super(props);
      this.state = { ...this.state, headlineText: "Saving to", target: { id: "L1", name: "Research Assistant 文献库" },
        errors: props.message ? [["connectionError", props.message]] : [] };
    }
  }
  for (const message of ["桌面与插件协议不兼容，已停止传输。请更新到兼容版本后重试。", "本机端口上的服务可能被其他软件占用。已停止传输。"]) {
    const markup = renderToStaticMarkup(React.createElement(CaptureProgress, { message }));
    assert.match(markup, /class="ProgressWindow-headline">连接失败</);
    assert.ok(markup.includes(message));
    assert.equal(markup.includes("Research Assistant 文献库"), false);
  }
  const success = renderToStaticMarkup(React.createElement(CaptureProgress));
  assert.ok(success.includes("Research Assistant 文献库"));
  assert.equal(success.includes("连接失败"), false);
});
