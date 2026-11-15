import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
} from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { ENGINE_VERSION } from "./docling-models.mjs";

/** No model resources enter the installer. Cargo's locked build owns incremental caching. */
export function prepareDoclingRuntime({
  root = process.cwd(),
  profile = "release",
  environment = process.env,
} = {}) {
  const args = [
    "build",
    "--locked",
    "--manifest-path",
    "apps/desktop/docling-daemon/Cargo.toml",
  ];
  if (profile === "release") args.push("--release");
  const result = spawnSync("cargo", args, {
    cwd: root,
    env: environment,
    stdio: "inherit",
  });
  if (result.status !== 0) throw new Error("Native Rust Docling build failed");
  const filename = `research-docling-daemon${process.platform === "win32" ? ".exe" : ""}`;
  const artifact = resolve(
    root,
    "apps/desktop/docling-daemon/target",
    profile,
    filename,
  );
  const version = spawnSync(artifact, ["--version"], {
    encoding: "utf8",
    env: environment,
    timeout: 10_000,
  });
  if (
    version.status !== 0 ||
    !/\bservice=[1-9]\d*\b/.test(version.stdout) ||
    !version.stdout.includes(
      `research-docling-daemon ${ENGINE_VERSION} fp32 protocol=2`,
    )
  )
    throw new Error("Native Docling executable identity mismatch");
  const destination = resolve(
    root,
    "apps/desktop/src-tauri/.generated/docling",
    filename,
  );
  publishDoclingRuntime(artifact, destination);
  return destination;
}

/** Atomic publication preserves inodes already mapped by running macOS children. */
export function publishDoclingRuntime(artifact, destination) {
  if (existsSync(destination) && readFileSync(artifact).equals(readFileSync(destination))) {
    return;
  }
  const directory = resolve(destination, "..");
  mkdirSync(directory, { recursive: true });
  const staging = mkdtempSync(resolve(directory, ".preparing-"));
  try {
    const temporary = resolve(staging, "daemon");
    copyFileSync(artifact, temporary);
    chmodSync(temporary, 0o755);
    renameSync(temporary, destination);
  } finally {
    rmSync(staging, { recursive: true, force: true });
  }
}
if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  try {
    prepareDoclingRuntime({
      profile: process.argv.includes("--debug") ? "debug" : "release",
    });
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
