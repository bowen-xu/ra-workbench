import fs from "node:fs/promises";
import path from "node:path";
import {fileURLToPath, pathToFileURL} from "node:url";
import {createRequire} from "node:module";
import {withCachedBuild} from "../.dependencies/sdk/tooling/build-artifacts.mjs";
import {prepareNoteQuery} from "./prepare-note-query.mjs";
import {prepareDictionary} from "./prepare-dictionary.mjs";
import {emitPreviewModules, packPreview} from "../preview-host/plugin-preview.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(path.join(root, "apps/web/package.json"));
const receipt = await withCachedBuild({root, kind: "preview-host", inputs: [
  "apps/web", "packages/ui", "preview-host", "apps/desktop/agent-resources", "scripts/build-preview-host.mjs", "scripts/prepare-note-query.mjs", "scripts/prepare-dictionary.mjs",
  "patches", "tsconfig.preview-host.json", "tsconfig.base.json", "package.json", "pnpm-lock.yaml",
]}, async ({workDirectory, logDirectory, output}) => {
  prepareNoteQuery();
  await prepareDictionary();
  const {loadConfigFromFile} = await import(pathToFileURL(require.resolve("vite")));
  const config = await loadConfigFromFile({command: "build", mode: "production"},
    path.join(root, "apps/web/vite.config.ts"), path.join(root, "apps/web"));
  const sdk = config?.config.plugins.flat(Infinity).find(plugin => plugin?.name === "workbench-runtime-sdk");
  if (!sdk?.api?.facades) throw new Error("preview_host_sdk_surface_missing");
  await emitPreviewModules({root, config: path.join(root, "tsconfig.preview-host.json"),
    sourceRoots: ["apps/web/src", "apps/desktop/agent-resources"],
    entryFiles: [...sdk.api.facades.filter(facade => facade.source.startsWith("apps/")).map(facade => path.join(root, facade.source)),
      ...["citationFetch.ts", "citationSyncFetch.ts"].map(file => path.join(root, "apps/web/src/shims", file))],
    output: path.join(output, "modules"), workDirectory, logDirectory});
  const surface = JSON.parse(await fs.readFile(path.join(root, "apps/web/previewHostSurface.json"), "utf8"));
  const metadata = JSON.parse(await fs.readFile(path.join(root, "package.json"), "utf8"));
  const frontend = JSON.parse(await fs.readFile(path.join(root, "apps/web/package.json"), "utf8"));
  const exports = {".": {types: "./modules/preview-host/index.d.ts", import: "./modules/preview-host/index.js"},
    "./package.json": "./package.json", "./tooling": "./tooling.mjs", "./styles": "./modules/apps/web/src/core/styles.css"};
  for (const module of surface.modules) exports["./" + module.name] = {
    types: "./modules/" + module.source.replace(/\.tsx?$/u, ".d.ts"),
    import: "./modules/" + module.source.replace(/\.tsx?$/u, ".js"),
  };
  for (const style of surface.styles) exports["./styles/" + style.name] = "./modules/" + style.source;
  await fs.cp(path.join(root, "patches"), path.join(output, "patches"), {recursive: true});
  const patches = {"@codemirror/autocomplete@6.20.3": "patches/@codemirror__autocomplete@6.20.3.patch", "pdfjs-dist@6.1.200": "patches/pdfjs-dist@6.1.200.patch"};
  await fs.cp(path.join(root, "apps/web/public"), path.join(output, "public"), {recursive: true});
  await fs.copyFile(path.join(root, "preview-host/plugin-preview.mjs"), path.join(output, "tooling.mjs"));
  await fs.writeFile(path.join(output, "preview-host.json"), JSON.stringify({formatVersion: 1, version: metadata.version,
    sdkFacades: sdk.api.facades, modules: surface.modules, patches}, null, 2) + "\n");
  await fs.writeFile(path.join(output, "package.json"), JSON.stringify({name: "@research-assistant/workbench-preview",
    version: metadata.version, private: true, type: "module", files: ["modules", "public", "preview-host.json", "tooling.mjs", "patches"], exports,
    dependencies: {...frontend.dependencies, "@vitejs/plugin-react": frontend.devDependencies["@vitejs/plugin-react"], "@types/react": frontend.devDependencies["@types/react"],
      "@types/react-dom": frontend.devDependencies["@types/react-dom"]}}, null, 2) + "\n");
  const archive = await packPreview({root, output, workDirectory, logDirectory});
  return {status: "passed", packageRoot: output, archive, version: metadata.version};
});
console.log(JSON.stringify({status: receipt.status, cacheHit: receipt.cacheHit, output: receipt.output,
  buildKey: receipt.buildKey, artifactKey: receipt.artifactKey, receipt: receipt.receipt}));
