import {
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

import { afterEach, describe, expect, it } from "vitest";

import {
  semanticEngineFingerprint,
  semanticEngineInputFiles,
  semanticEnginePaths,
} from "./prepare-semantic-engine.mjs";

const roots = [];

function fixtureRoot() {
  const root = mkdtempSync(resolve(tmpdir(), "semantic-engine-fingerprint-"));
  roots.push(root);
  const files = {
    ".cargo/config.toml": "[env]\nMACOSX_DEPLOYMENT_TARGET='12.0'\n",
    "apps/desktop/semantic-engine/Cargo.toml": "[package]\nname='engine'\n",
    "apps/desktop/semantic-engine/Cargo.lock": "version = 4\n",
    "apps/desktop/semantic-engine/src/lib.rs": "pub fn engine() {}\n",
    ".dependencies/sdk/native/semantic-engine-api/Cargo.toml": "[package]\nname='api'\n",
    ".dependencies/sdk/native/semantic-engine-api/src/lib.rs": "pub const ABI: u32 = 1;\n",
  };
  for (const [file, content] of Object.entries(files)) {
    const path = resolve(root, file);
    mkdirSync(resolve(path, ".."), { recursive: true });
    writeFileSync(path, content);
  }
  return root;
}

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { force: true, recursive: true });
});

describe("semantic engine preparation", () => {
  it("fingerprints source, lockfile, toolchain, target and profile", () => {
    const root = fixtureRoot();
    const base = {
      root,
      profile: "debug",
      platform: "darwin",
      architecture: "arm64",
      rustcVersion: "rustc fixture",
    };
    const first = semanticEngineFingerprint(base);
    writeFileSync(
      resolve(root, "apps/desktop/semantic-engine/src/lib.rs"),
      "pub fn changed() {}\n",
    );

    expect(semanticEngineFingerprint(base)).not.toBe(first);
    expect(semanticEngineFingerprint({ ...base, profile: "release" })).not.toBe(
      first,
    );
    expect(semanticEngineInputFiles(root)).toHaveLength(6);
  });

  it("keeps the heavy target separate from the Tauri target", () => {
    const paths = semanticEnginePaths({
      root: "/workspace",
      profile: "debug",
      platform: "darwin",
    });

    expect(paths.artifact).toContain(
      "/apps/desktop/semantic-engine/target/debug/",
    );
    expect(paths.generated).toContain("/src-tauri/.generated/semantic-engine/");
    expect(paths.artifact).not.toContain("/src-tauri/target/");
  });
});
