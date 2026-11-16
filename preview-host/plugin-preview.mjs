import fs from "node:fs/promises";
import {readFileSync} from "node:fs";
import path from "node:path";
import {createRequire} from "node:module";
import {createHash} from "node:crypto";
import {spawnSync} from "node:child_process";

const sha = bytes => createHash("sha256").update(bytes).digest("hex");
const ignored = new Set(["node_modules", ".dependencies", ".cache", "artifacts", "build", "dist"]);
const asset = /\.(?:css|json|svg|png|jpe?g|gif|webp|md|woff2?|ttf|otf|wasm)$/u;

async function files(directory) {
  const entries = await fs.readdir(directory, {withFileTypes: true});
  const nested = await Promise.all(entries.filter(entry => !ignored.has(entry.name)).map(entry => {
    const file = path.join(directory, entry.name);
    if (entry.isSymbolicLink()) throw new Error("preview_source_symlink: " + file);
    return entry.isDirectory() ? files(file) : [file];
  }));
  return nested.flat();
}

/** Emit an immutable ESM snapshot, retaining real module identities and mocks. */
export async function emitPreviewModules({root, config, sourceRoots, output, workDirectory, logDirectory, entryFiles = []}) {
  root = path.resolve(root);
  const require = createRequire(path.join(root, "package.json"));
  const sources = (await Promise.all(sourceRoots.map(source => files(path.join(root, source))))).flat();
  const configured = JSON.parse(await fs.readFile(config, "utf8"));
  const workers = sources.filter(file => /(?:\.worker|Worker)\.ts$/u.test(file) && !file.includes(".test."));
  const generated = path.join(workDirectory, "tsconfig.json");
  await fs.writeFile(generated, JSON.stringify({extends: config, compilerOptions: {
    composite: false, declaration: true, declarationMap: false, sourceMap: false,
    emitDeclarationOnly: false, noEmit: false, rewriteRelativeImportExtensions: true,
    allowImportingTsExtensions: true, resolveJsonModule: true, outDir: output, rootDir: root,
  }, files: [...(configured.files ?? []).map(file => path.resolve(path.dirname(config), file)), ...workers, ...entryFiles], include: []}, null, 2));
  console.error(`[preview] compiling modules · ${logDirectory}`);
  const result = spawnSync(process.execPath, [require.resolve("typescript/bin/tsc"), "-p", generated], {cwd: root, encoding: "utf8"});
  await fs.writeFile(path.join(logDirectory, "modules.log"), (result.stdout ?? "") + (result.stderr ?? ""));
  if (result.status !== 0) throw new Error(`preview_modules_failed: ${result.stdout ?? ""}${result.stderr ?? ""}`);
  const listed = spawnSync(process.execPath, [require.resolve("typescript/bin/tsc"), "-p", generated, "--listFilesOnly"], {cwd: root, encoding: "utf8"});
  if (listed.status !== 0) throw new Error("preview_source_inventory_failed");
  for (const file of listed.stdout.split(/\r?\n/u).filter(file => /\.tsx?$/u.test(file) && !file.endsWith(".d.ts"))) {
    if (!file.startsWith(root + path.sep) && !file.includes("/node_modules/") && !file.includes("/.dependencies/")) {
      throw new Error("preview_private_source_dependency: " + file);
    }
  }
  for (const file of sources.filter(file => (asset.test(file) || file.endsWith(".d.ts") || file.includes("/.generated/") && file.endsWith(".js"))
    && !/^(?:package|tsconfig[^/]*|public-modules)\.json$/u.test(path.basename(file)))) {
    const target = path.join(output, path.relative(root, file));
    await fs.mkdir(path.dirname(target), {recursive: true});
    await fs.copyFile(file, target);
  }
  for (const file of (await files(output)).filter(file => file.endsWith(".js"))) {
    const original = await fs.readFile(file, "utf8");
    const updates = [...original.matchAll(/new URL\((['"])([^'"\n]+\.tsx?)\1,\s*import\.meta\.url\)/gu)];
    let text = original;
    for (const match of updates) {
      const relative = match[2].replace(/\.tsx?$/u, ".js");
      await fs.access(path.resolve(path.dirname(file), relative));
      text = text.replace(match[0], `new URL(${JSON.stringify(relative)}, import.meta.url)`);
    }
    if (text !== original) await fs.writeFile(file, text);
  }
  const metadataFiles = (await files(output)).filter(file => path.basename(file) === "package.json");
  if (metadataFiles.length) {
    for (const file of (await files(output)).filter(file => /\.(?:js|ts)$/u.test(file))) {
      const original = await fs.readFile(file, "utf8");
      let text = original;
      for (const match of original.matchAll(/(["'])((?:\.\.?\/)[^"'\n]*package\.json)\1/gu)) {
        const target = path.resolve(path.dirname(file), match[2]);
        if (!metadataFiles.includes(target)) continue;
        const replacement = match[2].replace(/package\.json$/u, "package-metadata.json");
        text = text.replace(match[0], JSON.stringify(replacement));
      }
      if (text !== original) await fs.writeFile(file, text);
    }
    for (const file of metadataFiles) await fs.rename(file, path.join(path.dirname(file), "package-metadata.json"));
  }
  const ts = require("typescript");
  for (const file of (await files(output)).filter(file => file.endsWith(".js"))) {
    const text = await fs.readFile(file, "utf8");
    for (const imported of ts.preProcessFile(text).importedFiles) {
      const specifier = imported.fileName.split(/[?#]/u)[0];
      if (!specifier.startsWith(".")) continue;
      const target = path.resolve(path.dirname(file), specifier);
      const candidates = [target, target + ".js", path.join(target, "index.js")];
      if (!(await Promise.all(candidates.map(file => fs.stat(file).then(value => value.isFile(), () => false)))).some(Boolean)) {
        throw new Error("preview_resource_missing: " + path.relative(output, file) + " -> " + specifier);
      }
    }
  }
  return sources;
}

export async function packPreview({root, output, workDirectory, logDirectory}) {
  console.error("[preview] packing developer artifact");
  const result = spawnSync("pnpm", ["--dir", output, "pack", "--pack-destination", workDirectory], {cwd: root, encoding: "utf8"});
  await fs.writeFile(path.join(logDirectory, "pack.log"), (result.stdout ?? "") + (result.stderr ?? ""));
  if (result.status !== 0) throw new Error(`preview_pack_failed: ${result.stderr ?? result.stdout}`);
  const archive = (await fs.readdir(workDirectory)).find(file => file.endsWith(".tgz"));
  if (!archive) throw new Error("preview_archive_missing");
  await fs.copyFile(path.join(workDirectory, archive), path.join(output, archive));
  return archive;
}

/** Each producer publishes its explicit preview module entries. */
export async function buildPluginPreview({root, withCachedBuild}) {
  const preview = path.join(root, "preview");
  const metadata = JSON.parse(await fs.readFile(path.join(preview, "package.json"), "utf8"));
  const entries = JSON.parse(await fs.readFile(path.join(preview, "public-modules.json"), "utf8"));
  const archives = (await fs.readdir(path.join(preview, ".dependencies"))).filter(file => file.endsWith(".tgz")).sort();
  const dependencies = await Promise.all(archives.map(async file => file + ":" + sha(await fs.readFile(path.join(preview, ".dependencies", file)))));
  return withCachedBuild({root, kind: "preview", args: dependencies,
    inputs: ["src", "preview", "package.json", "pnpm-lock.yaml"]}, async ({workDirectory, output, logDirectory}) => {
    await emitPreviewModules({root, config: path.join(preview, "tsconfig.json"), sourceRoots: ["src", "preview"],
      output: path.join(output, "modules"), workDirectory, logDirectory});
    const exports = {".": {types: "./modules/preview/public.d.ts", import: "./modules/preview/public.js"}, "./package.json": "./package.json"};
    for (const entry of entries) exports["./" + entry.name] = /\.tsx?$/u.test(entry.source) ? {
      types: "./modules/" + entry.source.replace(/\.tsx?$/u, ".d.ts"),
      import: "./modules/" + entry.source.replace(/\.tsx?$/u, ".js"),
    } : "./modules/" + entry.source;
    const dependencies = {...metadata.dependencies};
    for (const [name, specifier] of Object.entries(dependencies)) if (specifier.startsWith("link:")) {
      const linked = JSON.parse(await fs.readFile(path.join(preview, specifier.slice(5), "package.json"), "utf8"));
      dependencies[name] = linked.version;
    }
    await fs.writeFile(path.join(output, "package.json"), JSON.stringify({name: metadata.name, version: metadata.version,
      private: true, type: "module", files: ["modules"], dependencies, exports}, null, 2) + "\n");
    const archive = await packPreview({root, output, workDirectory, logDirectory});
    return {status: "passed", packageRoot: output, archive, version: metadata.version};
  });
}

/** Bind SDK and developer APIs to one compiled Workbench snapshot. */
export async function createPluginPreviewConfig({root, previewDirectory = path.join(root, "preview")}) {
  const preview = previewDirectory;
  const require = createRequire(path.join(preview, "package.json"));
  const host = path.dirname(require.resolve("@research-assistant/workbench-preview/package.json"));
  const metadata = JSON.parse(await fs.readFile(path.join(host, "preview-host.json"), "utf8"));
  const own = JSON.parse(await fs.readFile(path.join(preview, "package.json"), "utf8"));
  const entries = JSON.parse(await fs.readFile(path.join(preview, "public-modules.json"), "utf8"));
  const {default: react} = await import("@vitejs/plugin-react");
  const hostRequire = createRequire(path.join(host, "package.json"));
  const exported = (packageDirectory, subpath) => {
    const value = JSON.parse(readFileSync(path.join(packageDirectory, "package.json"), "utf8")).exports[subpath];
    const target = typeof value === "string" ? value : value?.import;
    if (!target) throw new Error("preview_public_entry_missing: " + subpath);
    return path.resolve(packageDirectory, target);
  };
  async function resolvePublic(id, importer) {
    if (id === own.name) return path.join(preview, "public.ts");
    if (id.startsWith(own.name + "/")) {
      const entry = entries.find(entry => entry.name === id.slice(own.name.length + 1));
      if (entry) return path.join(root, entry.source);
    }
    if (id.startsWith("@research-assistant/workbench-sdk/")) {
      const facade = metadata.sdkFacades.find(facade => facade.name === id.slice("@research-assistant/workbench-sdk/".length));
      if (facade?.source.startsWith("apps/")) return path.join(host, "modules", facade.source.replace(/\.tsx?$/u, ".js"));
      if (facade) return this.resolve(facade.source, importer, {skipSelf: true});
    }
    if (id.startsWith("/preview-core/")) return exported(host, "./" + id.slice("/preview-core/".length).replace(/\.js$/u, ""));
    if (id.startsWith("/preview-api/")) {
      const [name, ...rest] = id.slice("/preview-api/".length).replace(/\.js$/u, "").split("/");
      const packageName = "@research-assistant/" + name + "-preview";
      const subpath = rest.length ? "./" + rest.join("/") : ".";
      if (packageName === own.name) {
        if (subpath === ".") return path.join(preview, "public.ts");
        const entry = entries.find(entry => "./" + entry.name === subpath);
        if (!entry) throw new Error("preview_public_entry_missing: " + subpath);
        return path.join(root, entry.source);
      }
      return exported(path.dirname(require.resolve(packageName + "/package.json")), subpath);
    }
  };
  return {
    root: preview, publicDir: path.join(host, "public"), clearScreen: false,
    define: {__APP_VERSION__: JSON.stringify(metadata.version), __RUNTIME_PLUGINS__: false},
    plugins: [react(), {name: "public-workbench-preview-bindings", enforce: "pre", resolveId: resolvePublic}],
    optimizeDeps: {include: ["daff"], exclude: ["@research-assistant/workbench-preview", "@research-assistant/workbench-sdk",
      ...Object.keys(own.dependencies).filter(name => name.endsWith("-preview"))]},
    ssr: {noExternal: [/^@research-assistant\//u]},
    resolve: {dedupe: ["react", "react-dom", "@research-assistant/core", "@research-assistant/data", "@research-assistant/writing", "@research-assistant/workbench-sdk",
      "@tiptap/pm", "@blocknote/core", "@blocknote/react", "@ariakit/react", "sonner", "@codemirror/state", "@codemirror/view", "@codemirror/autocomplete", "@codemirror/language",
      "@codemirror/merge", "@codemirror/commands", "@codemirror/search", "@tiptap/core", "pdfjs-dist"],
      alias: {"decode-named-character-reference": hostRequire.resolve("decode-named-character-reference"),
        "hast-util-from-html-isomorphic": hostRequire.resolve("hast-util-from-html-isomorphic"),
        "node-fetch": path.join(host, "modules/apps/web/src/shims/citationFetch.js"),
        "sync-fetch": path.join(host, "modules/apps/web/src/shims/citationSyncFetch.js")}},
    server: {port: 5173, strictPort: true, fs: {allow: [root, host, await fs.realpath(path.join(host, "public"))]}},
  };
}
