import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { prepareDictionary } from "./prepare-dictionary.mjs";

const WEB_BUILD_INPUTS = [
  "package.json",
  "pnpm-workspace.yaml",
  "tsconfig.base.json",
  "pnpm-lock.yaml",
  "scripts/check-workbench-boundaries.mjs",
  "scripts/workbench-boundaries.mjs",
  "docs/architecture/workbench-boundaries.policy.json",
  "docs/architecture/workbench-boundaries.baseline.json",
  "docs/architecture/runtime-note-sdk-surface.json",
  "scripts/prepare-dictionary.mjs",
  "scripts/dictionary-suggestions.mjs",
  "scripts/extract-dictionary-archive.mjs",
  "apps/web/index.html",
  "apps/web/package.json",
  "apps/web/public",
  "apps/web/src",
  "apps/web/tsconfig.app.json",
  "apps/web/tsconfig.json",
  "apps/web/tsconfig.node.json",
  "apps/web/vite.config.ts",
  "apps/web/workbenchCompositionPlugin.ts",
  "apps/web/runtimeSdkPlugin.ts",
  ".dependencies/sdk/sdk-artifacts.json",
];

const SCRIPT_DIRECTORY = dirname(fileURLToPath(import.meta.url));
const REPOSITORY_ROOT = resolve(SCRIPT_DIRECTORY, "..");

function collectFiles(path, label) {
  if (!existsSync(path)) throw new Error(`Web build ${label} is missing: ${path}`);
  const entries = readdirSync(path, { withFileTypes: true });
  return entries.flatMap((entry) => {
    const child = resolve(path, entry.name);
    return entry.isDirectory() ? collectFiles(child, label) : [child];
  });
}

function resolveInputFiles(root) {
  return WEB_BUILD_INPUTS.flatMap((input) => {
    const path = resolve(root, input);
    if (!existsSync(path)) throw new Error(`Web build input is missing: ${path}`);
    return statSync(path).isDirectory() ? collectFiles(path, "input") : [path];
  }).sort((left, right) => left.localeCompare(right));
}

function filesAtPath(path, label) {
  try {
    return collectFiles(path, label).sort((left, right) =>
      left.localeCompare(right),
    );
  } catch (error) {
    if (error?.code === "ENOTDIR") return [path];
    throw error;
  }
}

function fingerprintFiles(root, files, environment = {}) {
  const hash = createHash("sha256");
  for (const [key, value] of Object.entries(environment).sort(([left], [right]) =>
    left.localeCompare(right),
  )) {
    hash.update(`${key}=${value ?? ""}\0`);
  }
  for (const file of files) {
    hash.update(`${relative(root, file)}\0`);
    hash.update(readFileSync(file));
    hash.update("\0");
  }
  return hash.digest("hex");
}

function buildEnvironmentFingerprint(environment) {
  return Object.fromEntries(
    Object.entries(environment).filter(
      ([key]) => key === "NODE_ENV" || key.startsWith("VITE_")
        || key === "RESEARCH_ASSISTANT_BUILTIN_PLUGINS" || key === "RESEARCH_ASSISTANT_RUNTIME_PLUGINS",
    ),
  );
}

export function webBuildFingerprint({ root, environment = process.env }) {
  return fingerprintFiles(
    root,
    resolveInputFiles(root),
    buildEnvironmentFingerprint(environment),
  );
}

export function webBuildPaths(root) {
  return {
    dist: resolve(root, "apps/web/dist"),
    index: resolve(root, "apps/web/dist/index.html"),
    marker: resolve(root, ".cache/web-build/state.json"),
  };
}

function outputFingerprint(root, dist) {
  return fingerprintFiles(root, filesAtPath(dist, "output"));
}

function runWebBuild(root, environment) {
  const result = spawnSync(
    "pnpm",
    ["--filter", "@research-assistant/web", "build"],
    { cwd: root, env: environment, stdio: "inherit" },
  );
  if (result.status !== 0) throw new Error("Web frontend build failed");
}

export function prepareWebBuild({
  root = REPOSITORY_ROOT,
  environment = process.env,
  build = runWebBuild,
} = {}) {
  const paths = webBuildPaths(root);
  const inputFingerprint = webBuildFingerprint({ root, environment });
  if (existsSync(paths.index) && existsSync(paths.marker)) {
    try {
      const state = JSON.parse(readFileSync(paths.marker, "utf8"));
      if (
        state.inputFingerprint === inputFingerprint &&
        state.outputFingerprint === outputFingerprint(root, paths.dist)
      ) {
        console.log("Reusing unchanged web frontend build");
        return { ...paths, inputFingerprint, rebuilt: false };
      }
    } catch {
      // A partial build or marker is stale and is replaced after a successful build.
    }
  }

  console.log("Building web frontend because its inputs or outputs changed");
  build(root, environment);
  if (!existsSync(paths.index)) {
    throw new Error(`Web frontend build did not produce ${paths.index}`);
  }

  mkdirSync(dirname(paths.marker), { recursive: true });
  const state = {
    inputFingerprint,
    outputFingerprint: outputFingerprint(root, paths.dist),
  };
  const temporaryMarker = `${paths.marker}.building`;
  writeFileSync(temporaryMarker, `${JSON.stringify(state, null, 2)}\n`);
  renameSync(temporaryMarker, paths.marker);
  return { ...paths, inputFingerprint, rebuilt: true };
}

const invokedPath = process.argv[1];
if (
  invokedPath !== undefined &&
  import.meta.url === pathToFileURL(invokedPath).href
) {
  try {
    await prepareDictionary();
    prepareWebBuild();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
