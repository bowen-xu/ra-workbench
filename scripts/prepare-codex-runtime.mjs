import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
  constants,
} from "node:fs";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const packageNames = {
  "darwin-arm64": ["@openai/codex-darwin-arm64", "aarch64-apple-darwin"],
  "darwin-x64": ["@openai/codex-darwin-x64", "x86_64-apple-darwin"],
  "linux-arm64": ["@openai/codex-linux-arm64", "aarch64-unknown-linux-musl"],
  "linux-x64": ["@openai/codex-linux-x64", "x86_64-unknown-linux-musl"],
  "win32-arm64": ["@openai/codex-win32-arm64", "aarch64-pc-windows-msvc"],
  "win32-x64": ["@openai/codex-win32-x64", "x86_64-pc-windows-msvc"],
};

export function prepareCodexRuntime(
  root = resolve(dirname(fileURLToPath(import.meta.url)), ".."),
  { platform = process.platform, arch = process.arch } = {},
) {
  const target = packageNames[`${platform}-${arch}`];
  if (target === undefined) throw new Error(`Unsupported Codex runtime platform: ${platform}-${arch}`);
  const [packageName, triple] = target;
  const desktopManifest = resolve(root, "apps/desktop/package.json");
  let codexManifest;
  try {
    codexManifest = createRequire(desktopManifest).resolve("@openai/codex/package.json");
  } catch {
    throw new Error("Missing pinned @openai/codex build dependency; run pnpm install in the repository root");
  }
  let platformManifest;
  try {
    platformManifest = createRequire(codexManifest).resolve(`${packageName}/package.json`);
  } catch {
    throw new Error(`Missing ${packageName} optional package required by @openai/codex for ${platform}-${arch}`);
  }
  const packageRoot = dirname(platformManifest);
  const packageVersion = JSON.parse(readFileSync(platformManifest, "utf8")).version ?? "unknown";
  const executableName = platform === "win32" ? "codex.exe" : "codex";
  const source = resolve(packageRoot, "vendor", triple, "bin", executableName);
  if (!existsSync(source)) throw new Error(`The pinned @openai/codex package does not contain ${source}`);
  const destinationDirectory = resolve(root, "apps/desktop/src-tauri/.generated/codex");
  const destination = resolve(destinationDirectory, executableName);
  const temporary = `${destination}.preparing`;
  const marker = resolve(destinationDirectory, "runtime.json");
  const markerTemporary = `${marker}.preparing`;
  const sourceStat = statSync(source);
  const fingerprint = {
    packageName,
    packageVersion,
    triple,
    size: sourceStat.size,
    mtimeMs: sourceStat.mtimeMs,
  };
  mkdirSync(destinationDirectory, { recursive: true });
  if (existsSync(destination) && existsSync(marker)) {
    try {
      if (JSON.stringify(JSON.parse(readFileSync(marker, "utf8"))) === JSON.stringify(fingerprint)) {
        return destination;
      }
    } catch {
      // An incomplete marker is repaired by the atomic replacement below.
    }
  }
  rmSync(temporary, { force: true });
  rmSync(markerTemporary, { force: true });
  copyFileSync(source, temporary, constants.COPYFILE_FICLONE);
  if (platform !== "win32") chmodSync(temporary, 0o755);
  renameSync(temporary, destination);
  writeFileSync(markerTemporary, `${JSON.stringify(fingerprint, null, 2)}\n`);
  renameSync(markerTemporary, marker);
  return destination;
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(invokedPath).href) {
  try {
    console.log(`Prepared bundled Codex runtime at ${prepareCodexRuntime()}`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
