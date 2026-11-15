import {
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

import { afterEach, describe, expect, it, vi } from "vitest";

import { prepareWebBuild, webBuildFingerprint } from "./prepare-web-build.mjs";

const roots = [];
const files = [
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
  "apps/web/public/favicon.svg",
  "apps/web/src/main.tsx",
  "apps/web/tsconfig.app.json",
  "apps/web/tsconfig.json",
  "apps/web/tsconfig.node.json",
  "apps/web/vite.config.ts",
  "apps/web/workbenchCompositionPlugin.ts",
  "apps/web/runtimeSdkPlugin.ts",
  ".dependencies/sdk/sdk-artifacts.json",
];

function fixtureRoot() {
  const root = mkdtempSync(resolve(tmpdir(), "web-build-fingerprint-"));
  roots.push(root);
  for (const file of files) {
    const path = resolve(root, file);
    mkdirSync(resolve(path, ".."), { recursive: true });
    writeFileSync(path, `${file}\n`);
  }
  return root;
}

function writeOutput(root, content = "built") {
  const output = resolve(root, "apps/web/dist/index.html");
  mkdirSync(resolve(output, ".."), { recursive: true });
  writeFileSync(output, content);
}

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe("web build preparation", () => {
  it("invalidates the cached output when the build environment changes", () => {
    const root = fixtureRoot();
    const build = vi.fn(buildRoot => writeOutput(buildRoot));
    const prepare = nodeEnvironment => prepareWebBuild({root, build, environment: {NODE_ENV: nodeEnvironment}});
    expect(prepare("production").rebuilt).toBe(true);
    expect(prepare("production").rebuilt).toBe(false);
    expect(prepare("development").rebuilt).toBe(true);
    expect(prepare("development").rebuilt).toBe(false);
    expect(prepare("production").rebuilt).toBe(true);
    expect(build).toHaveBeenCalledTimes(3);
  });
  it.each(["apps/web/workbenchCompositionPlugin.ts", "apps/web/runtimeSdkPlugin.ts",
    "pnpm-workspace.yaml", "tsconfig.base.json", ".dependencies/sdk/sdk-artifacts.json",
    "docs/architecture/runtime-note-sdk-surface.json",
  ])("invalidates the cached build when %s changes", file => {
    const root = fixtureRoot();
    const before = webBuildFingerprint({root, environment: {}});
    writeFileSync(resolve(root, file), "changed composition input\n");
    expect(webBuildFingerprint({root, environment: {}})).not.toBe(before);
  });
  it("does not fingerprint generated plugin output or installed dependencies", () => {
    const root = fixtureRoot(), before = webBuildFingerprint({root, environment: {}});
    for (const file of ["plugins/note_manager/lib/entry.js", "plugins/note_manager/node_modules/foreign/index.js",
      "plugins/literature_library/build/entry.js", "plugins/literature_library/node_modules/foreign/index.js"]) {
      mkdirSync(resolve(root, file, ".."), {recursive: true});
      writeFileSync(resolve(root, file), "ignored generated dependency\n");
    }
    expect(webBuildFingerprint({root, environment: {}})).toBe(before);
  });
  it("invalidates the cached build after import-boundary policy changes", () => {
    const root = fixtureRoot();
    const before = webBuildFingerprint({root, environment: {}});
    writeFileSync(resolve(root, "docs/architecture/workbench-boundaries.policy.json"), "new public surface\n");
    expect(webBuildFingerprint({root, environment: {}})).not.toBe(before);
  });
  it("fingerprints frontend and public SDK inputs", () => {
    const root = fixtureRoot();
    const first = webBuildFingerprint({ root, environment: {} });
    writeFileSync(resolve(root, ".dependencies/sdk/sdk-artifacts.json"), "changed\n");

    expect(webBuildFingerprint({ root, environment: {} })).not.toBe(first);
  });

  it("reuses an unchanged verified output and rebuilds changed inputs", () => {
    const root = fixtureRoot();
    const build = vi.fn((buildRoot) => writeOutput(buildRoot));

    expect(prepareWebBuild({ root, environment: {}, build }).rebuilt).toBe(true);
    expect(prepareWebBuild({ root, environment: {}, build }).rebuilt).toBe(false);
    writeFileSync(resolve(root, "apps/web/src/main.tsx"), "changed\n");
    expect(prepareWebBuild({ root, environment: {}, build }).rebuilt).toBe(true);
    expect(build).toHaveBeenCalledTimes(2);
  });

  it("rebuilds when a generated output was modified", () => {
    const root = fixtureRoot();
    const build = vi.fn((buildRoot) => writeOutput(buildRoot));
    prepareWebBuild({ root, environment: {}, build });
    writeOutput(root, "modified");

    expect(prepareWebBuild({ root, environment: {}, build }).rebuilt).toBe(true);
    expect(build).toHaveBeenCalledTimes(2);
  });
});
