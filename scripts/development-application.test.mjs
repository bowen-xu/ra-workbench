import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { buildDevelopmentApplication, macOsBundleDirectory } from "./development-application.mjs";
import { TEMPORARY_APPLICATION_BUILD_ARGUMENTS } from "./release-environments.mjs";

const f = vi.hoisted(() => ({spawn: vi.fn(), selection: vi.fn(), codex: vi.fn(), semantic: vi.fn(), docling: vi.fn(), packages: vi.fn()}));
vi.mock("./prepare-builtin-plugin-packages.mjs", () => ({prepareBuiltinPluginResources: f.packages}));
vi.mock("node:child_process", () => ({spawnSync: f.spawn}));
vi.mock("./prepare-codex-runtime.mjs", () => ({prepareCodexRuntime: f.codex}));
vi.mock("./prepare-semantic-engine.mjs", () => ({prepareSemanticEngine: f.semantic}));
vi.mock("./prepare-docling-runtime.mjs", () => ({prepareDoclingRuntime: f.docling}));
vi.mock("./workbench-development-arguments.mjs", async original => ({
  ...await original(), readDevelopmentWorkbenchSelection: f.selection,
}));

const roots = [];
beforeEach(() => {
  vi.clearAllMocks();
  f.selection.mockReturnValue(undefined);
  f.packages.mockImplementation(({root}) => ({directory: resolve(root, ".generated-test/plugins")}));
  // Stop before any bundle promotion, signing or application launch. The test
  // exercises the actual build entry and inspects only its Tauri invocation.
  f.spawn.mockReturnValue({status: 1});
});
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, {recursive: true, force: true}); });

function attemptedBuild(environment, selection = {plugins: [], cargoFeatures: []}) {
  const root = mkdtempSync(resolve(tmpdir(), "development-runtime-core-"));
  roots.push(root);
  f.selection.mockReturnValue(selection);
  expect(() => buildDevelopmentApplication({root, environment})).toThrow("Tauri temporary development application build failed");
  return {root, arguments: f.spawn.mock.calls[0][1]};
}

describe("temporary development application", () => {
  it("keeps debug and release bundle directories separate", () => {
    expect(macOsBundleDirectory("/workspace", "debug")).toBe(
      "/workspace/apps/desktop/src-tauri/target/debug/bundle/macos",
    );
    expect(macOsBundleDirectory("/workspace")).toBe(
      "/workspace/apps/desktop/src-tauri/target/release/bundle/macos",
    );
  });

  it("limits disabled library validation to the disposable development app", () => {
    const root = resolve(import.meta.dirname, "..");
    const developmentEntitlements = readFileSync(
      resolve(
        root,
        "apps/desktop/src-tauri/DevelopmentEntitlements.plist",
      ),
      "utf8",
    );
    const releaseEntitlements = readFileSync(
      resolve(root, "apps/desktop/src-tauri/Entitlements.plist"),
      "utf8",
    );

    expect(developmentEntitlements).toContain(
      "com.apple.security.cs.disable-library-validation",
    );
    expect(releaseEntitlements).not.toContain(
      "com.apple.security.cs.disable-library-validation",
    );
  });

  it("builds the default temporary application from one Core and independently prepared builtin packages", () => {
    const build = attemptedBuild({}, undefined);
    const override = JSON.parse(build.arguments.find(value => value.startsWith("{")));
    expect(override).toEqual({build: {
      beforeBuildCommand: "node ../../scripts/build-workbench-composition.mjs --plugins none",
      frontendDist: resolve(build.root, "apps/web/dist"),
    }, bundle: {resources: {[resolve(build.root, ".generated-test/plugins") + "/"]: "plugins/"}}});
    expect(build.arguments).toContain("--no-default-features");
    expect(f.packages).toHaveBeenCalledExactlyOnceWith({root: build.root, environment: {
      RESEARCH_ASSISTANT_BUILTIN_PLUGINS: "none", RESEARCH_ASSISTANT_RUNTIME_PLUGINS: "1",
    }});
  });

  it("embeds the checked runtime Core .tmp output while preserving native Core-only and development identity", () => {
    const environment = {RESEARCH_ASSISTANT_BUILTIN_PLUGINS: "none", RESEARCH_ASSISTANT_RUNTIME_PLUGINS: "1"};
    const selection = {plugins: [], cargoFeatures: []};
    const build = attemptedBuild(environment, selection);
    const override = build.arguments.find(value => value.startsWith("{"));
    expect(JSON.parse(override)).toEqual({build: {
      beforeBuildCommand: "node ../../scripts/build-workbench-composition.mjs --plugins none",
      frontendDist: resolve(build.root, "apps/web/dist"),
    }, bundle: {resources: {[resolve(build.root, ".generated-test/plugins") + "/"]: "plugins/"}}});
    expect(build.arguments.indexOf(override)).toBeLessThan(build.arguments.indexOf("--"));
    expect(build.arguments).toContain("src-tauri/tauri.dev.conf.json");
    expect(build.arguments.slice(build.arguments.indexOf("--features"), build.arguments.indexOf("--features") + 2)).toEqual(["--features", "development-data"]);
    expect(build.arguments.slice(build.arguments.indexOf("--bundles"), build.arguments.indexOf("--bundles") + 2)).toEqual(["--bundles", "app"]);
    expect(build.arguments.slice(-2)).toEqual(["--", "--no-default-features"]);
    expect(f.codex).toHaveBeenCalledWith(build.root);
    expect(f.semantic).toHaveBeenCalledWith({root: build.root, profile: "debug", environment});
    expect(f.docling).toHaveBeenCalledWith({root: build.root, environment});
    expect(f.packages).toHaveBeenCalledWith({root: build.root, environment, pluginIds: []});
  });

  it("stops a removed static selection before resource preparation", () => {
    const root = mkdtempSync(resolve(tmpdir(), "development-static-invalid-"));
    roots.push(root);
    f.selection.mockImplementationOnce(() => {throw new Error("plugin_static_selection_removed");});
    expect(() => buildDevelopmentApplication({root, environment: {RESEARCH_ASSISTANT_BUILTIN_PLUGINS: "all"}}))
      .toThrow("plugin_static_selection_removed");
    expect(f.codex).not.toHaveBeenCalled();
    expect(f.packages).not.toHaveBeenCalled();
    expect(f.spawn).not.toHaveBeenCalled();
  });
});
