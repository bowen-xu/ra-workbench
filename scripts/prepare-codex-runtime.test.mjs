import assert from "node:assert/strict";
import { chmodSync, existsSync, mkdirSync, readFileSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { mkdtempSync } from "node:fs";

import { prepareCodexRuntime } from "./prepare-codex-runtime.mjs";

function isolatedFixture({ includePlatform = true } = {}) {
  const root = mkdtempSync(resolve(tmpdir(), "research-assistant-codex-"));
  const desktopManifest = resolve(root, "apps/desktop/package.json");
  mkdirSync(dirname(desktopManifest), { recursive: true });
  writeFileSync(desktopManifest, "{}");
  const codexRoot = resolve(root, "node_modules/@openai/codex");
  mkdirSync(codexRoot, { recursive: true });
  writeFileSync(resolve(codexRoot, "package.json"), JSON.stringify({ name: "@openai/codex", version: "0.146.0" }));
  if (includePlatform) {
    const actual = resolve(root, "node_modules/.isolated/codex-platform");
    const binary = resolve(actual, "vendor/aarch64-apple-darwin/bin/codex");
    mkdirSync(dirname(binary), { recursive: true });
    writeFileSync(resolve(actual, "package.json"), JSON.stringify({ name: "@openai/codex-darwin-arm64", version: "0.146.0-darwin-arm64" }));
    writeFileSync(binary, "pinned-codex-binary");
    chmodSync(binary, 0o755);
    const alias = resolve(codexRoot, "node_modules/@openai/codex-darwin-arm64");
    mkdirSync(dirname(alias), { recursive: true });
    symlinkSync(actual, alias, "dir");
  }
  return root;
}

test("copies the platform package resolved from the pinned Codex package context", () => {
  const root = isolatedFixture();
  const destination = prepareCodexRuntime(root, { platform: "darwin", arch: "arm64" });
  assert.equal(readFileSync(destination, "utf8"), "pinned-codex-binary");
});

test("reuses an unchanged prepared runtime without rewriting it", () => {
  const root = isolatedFixture();
  const destination = prepareCodexRuntime(root, { platform: "darwin", arch: "arm64" });
  const first = statSync(destination).mtimeMs;
  prepareCodexRuntime(root, { platform: "darwin", arch: "arm64" });
  assert.equal(statSync(destination).mtimeMs, first);
});

test("atomically replaces the runtime when the package source changes", () => {
  const root = isolatedFixture();
  const destination = prepareCodexRuntime(root, { platform: "darwin", arch: "arm64" });
  const source = resolve(root, "node_modules/.isolated/codex-platform/vendor/aarch64-apple-darwin/bin/codex");
  writeFileSync(source, "updated-pinned-codex-binary");
  prepareCodexRuntime(root, { platform: "darwin", arch: "arm64" });
  assert.equal(readFileSync(destination, "utf8"), "updated-pinned-codex-binary");
  assert.equal(existsSync(`${destination}.preparing`), false);
});

test("reports a missing platform package without falling back to a global Codex", () => {
  const root = isolatedFixture({ includePlatform: false });
  assert.throws(
    () => prepareCodexRuntime(root, { platform: "darwin", arch: "arm64" }),
    /Missing @openai\/codex-darwin-arm64 optional package/,
  );
});

test("reports an unsupported platform clearly", () => {
  const root = isolatedFixture();
  assert.throws(
    () => prepareCodexRuntime(root, { platform: "aix", arch: "ppc64" }),
    /Unsupported Codex runtime platform: aix-ppc64/,
  );
});
