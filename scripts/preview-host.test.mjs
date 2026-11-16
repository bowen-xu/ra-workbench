import assert from "node:assert/strict";
import fs from "node:fs/promises";
import path from "node:path";
import {tmpdir} from "node:os";
import {afterEach, test} from "node:test";
import {fileURLToPath} from "node:url";
import {emitPreviewModules} from "../preview-host/plugin-preview.mjs";

const project = fileURLToPath(new URL("../", import.meta.url));
const temporary = [];
afterEach(async () => {
  for (const directory of temporary.splice(0)) await fs.rm(directory, {recursive: true, force: true});
});
async function fixture(text) {
  const root = await fs.mkdtemp(path.join(tmpdir(), "workbench-preview-"));
  temporary.push(root);
  for (const directory of ["src", "node_modules", "work", "logs"]) await fs.mkdir(path.join(root, directory));
  await fs.symlink(path.join(project, "node_modules/typescript"), path.join(root, "node_modules/typescript"));
  await fs.writeFile(path.join(root, "package.json"), '{"type":"module"}');
  await fs.writeFile(path.join(root, "src/index.ts"), text);
  const config = path.join(root, "tsconfig.json");
  await fs.writeFile(config, JSON.stringify({compilerOptions: {module: "ESNext", moduleResolution: "Bundler", target: "ES2022", types: []}, files: ["src/index.ts"]}));
  return {root, config, sourceRoots: ["src"], output: path.join(root, "modules"),
    workDirectory: path.join(root, "work"), logDirectory: path.join(root, "logs")};
}

test("refuses publication when a moved stylesheet is absent", async () => {
  const options = await fixture('import "./view.css"; export const value = 1;');
  await assert.rejects(emitPreviewModules(options), /preview_resource_missing: src\/index.js -> .\/view.css/u);
});

test("retains styles and generated Wasm loaders in the compiled public closure", async () => {
  const options = await fixture('import "./view.css"; export const loader = new URL("./.generated/kernel.js", import.meta.url);');
  await fs.writeFile(path.join(options.root, "src/view.css"), ".view {display: grid;}\n");
  await fs.mkdir(path.join(options.root, "src/.generated"));
  await fs.writeFile(path.join(options.root, "src/.generated/kernel.js"), "export const value = 42;\n");
  await fs.writeFile(path.join(options.root, "src/.generated/kernel_bg.wasm"), new Uint8Array([0, 97, 115, 109]));
  await emitPreviewModules(options);
  assert.match(await fs.readFile(path.join(options.output, "src/index.js"), "utf8"), /new URL/u);
  assert.equal(await fs.readFile(path.join(options.output, "src/view.css"), "utf8"), ".view {display: grid;}\n");
  assert.equal(await fs.readFile(path.join(options.output, "src/.generated/kernel.js"), "utf8"), "export const value = 42;\n");
  assert.deepEqual(await fs.readFile(path.join(options.output, "src/.generated/kernel_bg.wasm")), Buffer.from([0, 97, 115, 109]));
  await assert.rejects(fs.access(path.join(options.output, "src/index.ts")));
  await fs.access(path.join(options.output, "src/index.d.ts"));
});
