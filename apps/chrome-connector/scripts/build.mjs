// SPDX-License-Identifier: AGPL-3.0-or-later
import { execFileSync } from "node:child_process";
import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";
import { createRequire } from "node:module";
import { createHash } from "node:crypto";
import { transformSync } from "@babel/core";
import ts from "typescript";
import { patchUpstream } from "./patch-upstream.mjs";
import { makeIcons } from "./icons.mjs";
import { translatorLicense } from "./translator-licenses.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const app = path.resolve(here, "..");
const root = path.resolve(app, "../..");
const output = path.join(root, ".tmp/chrome-connector");
const sources = path.join(root, ".tmp/chrome-connector-source");
const lock = JSON.parse(await fs.readFile(path.join(app, "upstream-lock.json"), "utf8"));
const packageJSON = JSON.parse(await fs.readFile(path.join(app, "package.json"), "utf8"));
const require = createRequire(import.meta.url);
const run = (command, args, cwd) => execFileSync(command, args, { cwd, encoding: "utf8", maxBuffer: 192 * 1024 * 1024 });

await fs.mkdir(sources, { recursive: true });
for (const [name, entry] of Object.entries(lock)) {
  let cache = path.join(root, entry.cache);
  try { run("git", ["cat-file", "-e", `${entry.revision}^{commit}`], cache); }
  catch {
    if (!process.argv.includes("--fetch")) throw new Error(`Missing pinned ${name}; rerun with --fetch to download exact sources.`);
    cache = path.join(root, ".tmp/chrome-connector-upstream", name);
    await fs.mkdir(cache, { recursive: true });
    run("git", ["init"], cache);
    run("git", ["fetch", "--depth=1", entry.repository, entry.revision], cache);
  }
  const destination = path.join(sources, name);
  await fs.rm(destination, { recursive: true, force: true });
  await fs.mkdir(destination, { recursive: true });
  const archive = path.join(sources, `${name}.tar`);
  run("git", ["archive", `--output=${archive}`, entry.revision], cache);
  run("tar", ["-xf", archive, "-C", destination], root);
  await fs.rm(archive);
}

await fs.rm(output, { recursive: true, force: true });
await fs.mkdir(output, { recursive: true });
const connector = path.join(sources, "connector");
await fs.cp(path.join(connector, "src/common"), output, { recursive: true });
await fs.cp(path.join(connector, "src/browserExt"), output, { recursive: true });
await fs.cp(path.join(sources, "translate/src"), path.join(output, "translate"), { recursive: true });
await fs.cp(path.join(sources, "utilities"), path.join(output, "utilities"), { recursive: true });
await fs.copyFile(path.join(connector, "COPYING"), path.join(output, "COPYING"));
await fs.copyFile(path.join(connector, "COPYING"), path.join(app, "COPYING"));

// Use the actual pinned Gulp script lists, keeping MV3 sandbox/content/background ordering.
const gulp = await fs.readFile(path.join(connector, "gulpfile.js"), "utf8");
const context = { require: name => {
  if (name === "./src/zotero-schema/schema.json") return {};
  if (name === "yargs") return new Proxy({ argv: { p: true } }, { get: (target, key) => key === "argv" ? target.argv : () => new Proxy(target, { get: (t, k) => k === "argv" ? t.argv : () => context.require("yargs") }) });
  return {};
}, process: { env: {} } };
vm.createContext(context);
vm.runInContext(gulp.slice(0, gulp.indexOf("var reloadChromiumTimeout;")), context);
const injected = [...context.injectIncludeManifestV3, "research-assistant/inject.js"];
const background = context.backgroundIncludeBrowserExt.filter(name => ![
  "zotero-google-docs-integration/api.js", "oauthsimple.js", "updaterFix.js",
].includes(name));
background.push("research-assistant/connection.js", "research-assistant/transport.js", "research-assistant/background.js");
const schema = JSON.parse(await fs.readFile(path.join(sources, "schema/schema.json"), "utf8"));

async function* files(directory) {
  for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
    const target = path.join(directory, entry.name);
    if (entry.isDirectory()) yield* files(target);
    else if (entry.isFile()) yield target;
  }
}
for await (const file of files(output)) {
  const relative = path.relative(output, file);
  if (![".js", ".jsx", ".html"].includes(path.extname(file))) continue;
  let text = patchUpstream(relative, await fs.readFile(file, "utf8"), schema);
  text = text.replace("/*BACKGROUND SCRIPTS*/", background.map(name => JSON.stringify(name)).join(",\n"));
  text = text.replace("/*INJECT SCRIPTS*/", injected.map(name => JSON.stringify(name)).join(",\n"));
  text = text.replace("<!--SCRIPTS-->", injected.map(name => `<script src="../../${name}"></script>`).join("\n"));
  if (relative === "zotero_config.js") {
    text = `// Research Assistant transport configuration; upstream service accounts are intentionally absent.\nconst ZOTERO_CONFIG = ${JSON.stringify({
      CLIENT_NAME: "Research Assistant", DOMAIN_NAME: "research-assistant.local",
      SETTINGS_URL: "", REPOSITORY_URL: "", REPOSITORY_CHECK_INTERVAL: 86400,
      REPOSITORY_RETRY_INTERVAL: 3600, REPOSITORY_CHANNEL: "pinned",
      ALWAYS_FETCH_FROM_REPOSITORY: false, BASE_URI: "https://research-assistant.local/",
      WWW_BASE_URL: "https://research-assistant.local/", CLIENT_DOWNLOAD_URL: "",
      API_URL: "", OAUTH: { ZOTERO: { CALLBACK_URL: "about:blank#disabled-zotero-auth" }, GOOGLE_DOCS: {} },
    })};`;
  }
  if (path.extname(file) === ".jsx") {
    text = transformSync(text, { configFile: false, babelrc: false, plugins: [require.resolve("@babel/plugin-transform-react-jsx")] }).code;
    await fs.writeFile(file.slice(0, -1), text);
    await fs.rm(file);
  } else await fs.writeFile(file, text);
}

await fs.mkdir(path.join(output, "research-assistant"), { recursive: true });
for (const name of ["connection", "transport", "background", "inject", "api", "options"]) {
  const source = await fs.readFile(path.join(app, "src", `${name}.ts`), "utf8");
  const destination = name === "api" ? path.join(output, "api.js") : path.join(output, "research-assistant", `${name}.js`);
  await fs.writeFile(destination, ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText);
}
await fs.cp(path.join(app, "options"), path.join(output, "research-assistant"), { recursive: true });
const typography = (await fs.readFile(path.join(root, "apps/web/src/core/styles.css"), "utf8"))
  .match(/^\s*--ui-font-(?:caption|meta|label|body|heading):[^;]+;/gm)?.join("\n");
if (!typography) throw new Error("Shared UI typography tokens were not found");
for (const stylesheet of ["options.css", "progress.css"]) {
  const target = path.join(output, "research-assistant", stylesheet);
  await fs.writeFile(target, (await fs.readFile(target, "utf8")).replace("/*SHARED_TYPOGRAPHY*/", typography));
}
await fs.mkdir(path.join(output, "lib"), { recursive: true });
for (const [dependency, from, to] of [
  ["react", "umd/react.production.min.js", "react.js"],
  ["react-dom", "umd/react-dom.production.min.js", "react-dom.js"],
  ["react-dom-factories", "index.js", "react-dom-factories.js"],
  ["prop-types", "prop-types.min.js", "prop-types.js"],
  ["dompurify", "dist/purify.min.js", "dompurify.js"],
]) {
  let base = path.dirname(require.resolve(dependency));
  while (JSON.parse(await fs.readFile(path.join(base, "package.json"), "utf8").catch(() => "{}" )).name !== dependency) {
    const parent = path.dirname(base);
    if (parent === base) throw new Error(`Cannot locate source package ${dependency}`);
    base = parent;
  }
  await fs.copyFile(path.join(base, from), path.join(output, "lib", to));
  await fs.cp(base, path.join(sources, "dependencies", dependency), { recursive: true, filter: file => !file.includes(`${path.sep}node_modules${path.sep}node_modules`) });
  for (const license of (await fs.readdir(base)).filter(name => /^(LICENSE|COPYING|NOTICE)/i.test(name))) {
    const destination = path.join(output, "licenses", dependency);
    await fs.mkdir(destination, { recursive: true });
    await fs.copyFile(path.join(base, license), path.join(destination, license));
  }
}

// Preserve individual licenses; project default applies only when there is no
// overriding notice. GPLv3/AGPLv3 permit combining under their section 13 rules.
const metadata = [];
const excluded = [];
const inventory = [];
await fs.mkdir(path.join(output, "translators"), { recursive: true });
for (const name of (await fs.readdir(path.join(sources, "translators"))).sort()) {
  if (!name.endsWith(".js")) continue;
  const text = await fs.readFile(path.join(sources, "translators", name), "utf8");
  const match = text.match(/^\s*\{[\s\S]*?\}\s*?[\r\n]/);
  if (!match) continue;
  const license = translatorLicense(text);
  if (!license.compatible) {
    excluded.push(name);
    continue;
  }
  const info = JSON.parse(match[0]);
  metadata.push(info);
  inventory.push({ file: name, translatorID: info.translatorID, sha256: createHash("sha256").update(text).digest("hex"), ...license });
  await fs.writeFile(path.join(output, "translators", `${info.translatorID}.js`), text);
}
await fs.writeFile(path.join(output, "translators/metadata.json"), JSON.stringify(metadata));
await fs.writeFile(path.join(output, "translators/excluded.json"), JSON.stringify(excluded, null, 2));
await fs.writeFile(path.join(output, "translators/licenses.json"), JSON.stringify(inventory, null, 2));
await fs.mkdir(path.join(output, "licenses"), { recursive: true });
await fs.copyFile(path.join(app, "GPL-3.0.txt"), path.join(output, "licenses/GPL-3.0.txt"));
const messages = JSON.parse(await fs.readFile(path.join(connector, "src/messages.json"), "utf8"));
for (const entry of Object.values(messages)) entry.message = entry.message.replaceAll("Zotero", "Research Assistant");
messages.progressWindow_savingToOnlineLibrary.message = "桌面未连接";
await fs.mkdir(path.join(output, "_locales/en"), { recursive: true });
await fs.writeFile(path.join(output, "_locales/en/messages.json"), JSON.stringify(messages));
const manifest = JSON.parse((await fs.readFile(path.join(output, "manifest-v3.json"), "utf8"))
  .replace("/*INJECT SCRIPTS*/", injected.map(name => JSON.stringify(name)).join(",\n")));
manifest.name = "Research Assistant Connector";
manifest.version = packageJSON.version;
manifest.description = "Save papers and accessible PDFs to your Research Assistant desktop library.";
delete manifest.homepage_url;
manifest.minimum_chrome_version = "116";
manifest.host_permissions = ["http://*/*", "https://*/*"];
manifest.content_scripts = manifest.content_scripts.slice(0, 1);
manifest.content_scripts[0].js = injected;
manifest.permissions = manifest.permissions.filter(name => name !== "cookies");
// Upstream browser PDF retrieval uses cookies without exporting them to the desktop.
manifest.permissions.push("cookies");
delete manifest.declarative_net_request;
manifest.options_ui.page = "research-assistant/options.html";
manifest.web_accessible_resources[0].resources = manifest.web_accessible_resources[0].resources.filter(name => !name.includes("SingleFile") && !name.startsWith("test/"));
await fs.writeFile(path.join(output, "manifest.json"), JSON.stringify(manifest, null, 2));
await fs.rm(path.join(output, "manifest-v3.json"));
for (const name of ["test", "tools", "preferences", "singlefile.js", "singlefile-config.js", "oauthsimple.js", "updaterFix.js"]) {
  await fs.rm(path.join(output, name), { recursive: true, force: true });
}
// Keep upstream load slots, but snapshots are not part of this release.
await fs.writeFile(path.join(output, "singlefile.js"), "Zotero.SingleFile = { retrievePageData: async () => { throw new Error('HTML snapshots are not supported'); } };\n");
await fs.mkdir(path.join(output, "preferences"), { recursive: true });
await fs.writeFile(path.join(output, "preferences/preferences.html"), '<!doctype html><meta charset="utf-8"><meta http-equiv="refresh" content="0; url=../research-assistant/options.html">');
await makeIcons(output, schema);
await fs.copyFile(path.join(app, "NOTICE.md"), path.join(output, "NOTICE.md"));
await fs.copyFile(path.join(app, "upstream-lock.json"), path.join(output, "upstream-lock.json"));
await fs.cp(app, path.join(sources, "research-assistant-adapter"), { recursive: true, filter: file => !file.includes("node_modules") });
console.log(`Built ${output}\n${metadata.length} pinned translators with per-file license inventory; ${excluded.length} incompatible/unresolved files excluded.\nCorresponding source: ${sources}`);
