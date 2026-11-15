import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import {createRequire} from "node:module";
import {fileURLToPath} from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(path.join(root, "apps/web/package.json"));
const ts = require("typescript");
const [core, plugin] = process.argv.slice(2);
if (!core || !plugin || process.argv.length !== 4) throw new Error("Usage: node scripts/check-runtime-sdk-build.mjs <core-output> <plugin-package>");
const graph = JSON.parse(await fs.readFile(path.join(core, "runtime-sdk.json"), "utf8"));
const composition = JSON.parse(await fs.readFile(path.join(core, "workbench-composition.json"), "utf8"));
assert.deepEqual(composition.plugins, []);
assert.ok(!graph.modules.some(module => /(?:^|\/)src\/plugins\/|(?:^|\/)plugins\/[^/]+\/src\//u.test(module.replaceAll("\\", "/"))),
  "Core's actual module graph must exclude application and root plugin implementations");
assert.equal(graph.facadeBuild, "same-core-multi-entry");
assert.ok(graph.owners.some(owner => owner.module.includes("NotesClient")), "Core's real note resource owner must be present");
assert.ok(graph.owners.some(owner => owner.module.includes("node_modules/react/")), "Core's real React module must be present");
assert.ok(graph.owners.every(owner => owner.chunks.length === 1));
const required = [];
for (const file of await fs.readdir(plugin)) {
  if (!file.endsWith(".js")) continue;
  const source = ts.createSourceFile(file, await fs.readFile(path.join(plugin, file), "utf8"), ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  for (const node of source.statements) {
    if (!ts.isImportDeclaration(node) || !ts.isStringLiteral(node.moduleSpecifier)) continue;
    const url = node.moduleSpecifier.text;
    if (!url.startsWith("/sdk/v1/")) continue;
    const facade = graph.facades.find(value => "/" + value.fileName === url);
    assert.ok(facade, `Core does not provide ${url}`);
    await fs.access(path.join(core, facade.fileName));
    const names = [];
    if (node.importClause?.name) names.push("default");
    if (node.importClause?.namedBindings && ts.isNamedImports(node.importClause.namedBindings)) {
      names.push(...node.importClause.namedBindings.elements.map(value => (value.propertyName ?? value.name).text));
    }
    for (const name of names) assert.ok(facade.exports.includes(name), `${url} is missing actual plugin import ${name}`);
    required.push({url, names});
  }
}
assert.ok(required.length > 0, "Read the actual constructed plugin entry, not an empty fixture");
const result = {status: "passed", core: path.resolve(core), plugin: path.resolve(plugin),
  modules: graph.modules.length, facadeCount: graph.facades.length, importedModules: required.length,
  actualNamedImports: required.reduce((sum, value) => sum + value.names.length, 0), owners: graph.owners, required};
await fs.writeFile(path.join(core, "runtime-sdk-consumer-check.json"), JSON.stringify(result, null, 2) + "\n");
console.log(JSON.stringify({status: result.status, facadeCount: result.facadeCount, importedModules: result.importedModules,
  actualNamedImports: result.actualNamedImports, receipt: path.resolve(core, "runtime-sdk-consumer-check.json")}));
