import { spawnSync } from "node:child_process";
import {
  constants,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { basename, dirname, relative, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const ENGINE_MANIFEST = "apps/desktop/semantic-engine/Cargo.toml";
const ENGINE_INPUTS = [
  ".cargo/config.toml",
  "apps/desktop/semantic-engine/Cargo.toml",
  "apps/desktop/semantic-engine/Cargo.lock",
  "apps/desktop/semantic-engine/src",
  ".dependencies/sdk/native/semantic-engine-api/Cargo.toml",
  ".dependencies/sdk/native/semantic-engine-api/src",
];

function libraryFilename(platform) {
  if (platform === "darwin") return "libresearch_semantic_engine.dylib";
  if (platform === "win32") return "research_semantic_engine.dll";
  return "libresearch_semantic_engine.so";
}

function collectFiles(path) {
  if (!existsSync(path)) throw new Error(`Semantic engine input is missing: ${path}`);
  const entries = readdirSync(path, { withFileTypes: true });
  return entries.flatMap((entry) => {
    const child = resolve(path, entry.name);
    return entry.isDirectory() ? collectFiles(child) : [child];
  });
}

export function semanticEngineInputFiles(root) {
  return ENGINE_INPUTS.flatMap((input) => {
    const path = resolve(root, input);
    return input.endsWith("/src") ? collectFiles(path) : [path];
  }).sort((left, right) => left.localeCompare(right));
}

export function semanticEngineFingerprint({
  root,
  profile,
  platform = process.platform,
  architecture = process.arch,
  rustcVersion,
}) {
  const hash = createHash("sha256");
  hash.update(`profile=${profile}\nplatform=${platform}\narch=${architecture}\n`);
  hash.update(`rustc=${rustcVersion}\n`);
  for (const file of semanticEngineInputFiles(root)) {
    hash.update(`${relative(root, file)}\0`);
    hash.update(readFileSync(file));
    hash.update("\0");
  }
  return hash.digest("hex");
}

function runChecked(command, arguments_, options, failureMessage) {
  const result = spawnSync(command, arguments_, {
    cwd: options.root,
    env: options.environment,
    encoding: "utf8",
    stdio: options.inherit ? "inherit" : ["ignore", "pipe", "pipe"],
  });
  if (result.status !== 0) {
    const details = [result.stdout, result.stderr]
      .filter((value) => value?.trim())
      .join("\n");
    throw new Error(details ? `${failureMessage}\n${details}` : failureMessage);
  }
  return result.stdout?.trim() ?? "";
}

export function semanticEnginePaths({
  root,
  profile,
  platform = process.platform,
}) {
  const filename = libraryFilename(platform);
  const targetRoot = resolve(root, "apps/desktop/semantic-engine/target");
  const generatedRoot = resolve(
    root,
    "apps/desktop/src-tauri/.generated/semantic-engine",
  );
  return {
    artifact: resolve(targetRoot, profile, filename),
    generated: resolve(generatedRoot, filename),
    marker: resolve(generatedRoot, `${filename}.json`),
    targetRoot,
  };
}

export function prepareSemanticEngine({
  root = process.cwd(),
  profile = "debug",
  environment = process.env,
  platform = process.platform,
  architecture = process.arch,
} = {}) {
  if (!new Set(["debug", "release"]).has(profile)) {
    throw new Error(`Unsupported semantic engine profile: ${profile}`);
  }
  const rustcVersion = runChecked(
    "rustc",
    ["-vV"],
    { root, environment },
    "Could not identify the Rust toolchain",
  );
  const fingerprint = semanticEngineFingerprint({
    root,
    profile,
    platform,
    architecture,
    rustcVersion,
  });
  const paths = semanticEnginePaths({ root, profile, platform });
  if (existsSync(paths.generated) && existsSync(paths.marker)) {
    try {
      const marker = JSON.parse(readFileSync(paths.marker, "utf8"));
      if (marker.fingerprint === fingerprint) {
        console.log(`Reusing semantic engine ${basename(paths.generated)}`);
        return { ...paths, fingerprint, rebuilt: false };
      }
    } catch {
      // A partial or manually edited marker is stale and is replaced after a successful build.
    }
  }

  const arguments_ = [
    "build",
    "--locked",
    "--manifest-path",
    ENGINE_MANIFEST,
  ];
  if (profile === "release") arguments_.push("--release");
  console.log(`Building required semantic engine (${profile})`);
  runChecked(
    "cargo",
    arguments_,
    { root, environment, inherit: true },
    "Required semantic engine build failed",
  );
  if (!existsSync(paths.artifact)) {
    throw new Error(`Semantic engine artifact was not produced: ${paths.artifact}`);
  }

  mkdirSync(dirname(paths.generated), { recursive: true });
  const temporary = `${paths.generated}.building`;
  rmSync(temporary, { force: true });
  copyFileSync(paths.artifact, temporary, constants.COPYFILE_FICLONE);
  renameSync(temporary, paths.generated);
  writeFileSync(
    paths.marker,
    `${JSON.stringify({ fingerprint, profile }, null, 2)}\n`,
  );
  return { ...paths, fingerprint, rebuilt: true };
}

const invokedPath = process.argv[1];
if (
  invokedPath !== undefined &&
  import.meta.url === pathToFileURL(invokedPath).href
) {
  try {
    prepareSemanticEngine({
      profile: process.argv.includes("--release") ? "release" : "debug",
    });
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
