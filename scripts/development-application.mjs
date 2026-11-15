import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  renameSync,
  rmSync,
} from "node:fs";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

import {
  DEVELOPMENT_IDENTIFIER,
  TEMPORARY_APPLICATION_BUILD_ARGUMENTS,
} from "./release-environments.mjs";
import { prepareCodexRuntime } from "./prepare-codex-runtime.mjs";
import { prepareDoclingRuntime } from "./prepare-docling-runtime.mjs";
import { prepareSemanticEngine } from "./prepare-semantic-engine.mjs";
import { developmentWorkbenchArguments, readDevelopmentWorkbenchSelection } from "./workbench-development-arguments.mjs";
import {prepareBuiltinPluginResources} from "./prepare-builtin-plugin-packages.mjs";

const TEMPORARY_APPLICATION_NAME = "Research Assistant Dev.app";
const TEMPORARY_APPLICATION_DISPLAY_NAME = "Research Assistant Dev";
const STAGING_APPLICATION_NAME = ".Research Assistant Dev.building.app";
const BACKUP_APPLICATION_NAME = ".Research Assistant Dev.previous.app";

function runChecked(command, arguments_, errorMessage, root) {
  const result = spawnSync(command, arguments_, {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });

  if (result.status !== 0) {
    const details = [result.stdout, result.stderr]
      .filter((value) => value?.trim())
      .join("\n");
    throw new Error(details ? `${errorMessage}\n${details}` : errorMessage);
  }

  return result.stdout.trim();
}

export function macOsBundleDirectory(root, profile = "release") {
  return resolve(root, `apps/desktop/src-tauri/target/${profile}/bundle/macos`);
}

export function temporaryApplicationPath(root) {
  return resolve(root, ".tmp", TEMPORARY_APPLICATION_NAME);
}

export function readApplicationIdentifier(applicationPath, context, root) {
  const infoPlist = resolve(applicationPath, "Contents/Info.plist");
  return runChecked(
    "/usr/libexec/PlistBuddy",
    ["-c", "Print :CFBundleIdentifier", infoPlist],
    `Could not read the ${context} application identifier`,
    root,
  );
}

export function requireApplicationIdentifier(
  applicationPath,
  expected,
  context,
  root,
) {
  const identifier = readApplicationIdentifier(applicationPath, context, root);
  if (identifier !== expected) {
    throw new Error(
      `${context} application must use ${expected}; found ${identifier}`,
    );
  }

  return identifier;
}

export function findBundledApplication(root, profile = "release") {
  const directory = macOsBundleDirectory(root, profile);
  if (!existsSync(directory)) {
    throw new Error("Tauri macOS application bundle was not found");
  }

  const application = readdirSync(directory, {
    withFileTypes: true,
  }).find((entry) => entry.isDirectory() && entry.name.endsWith(".app"));

  if (application === undefined) {
    throw new Error("Tauri macOS application bundle was not found");
  }

  return resolve(directory, application.name);
}

function prepareTemporaryApplication(applicationPath, root) {
  requireApplicationIdentifier(
    applicationPath,
    DEVELOPMENT_IDENTIFIER,
    "Temporary development",
    root,
  );
  const infoPlist = resolve(applicationPath, "Contents/Info.plist");

  for (const key of ["CFBundleName", "CFBundleDisplayName"]) {
    runChecked(
      "/usr/libexec/PlistBuddy",
      ["-c", `Set :${key} ${TEMPORARY_APPLICATION_DISPLAY_NAME}`, infoPlist],
      `Could not set ${key} on the temporary application`,
      root,
    );
  }

  runChecked(
    "codesign",
    [
      "--force",
      "--deep",
      "--sign",
      "-",
      "--options",
      "runtime",
      "--entitlements",
      resolve(root, "apps/desktop/src-tauri/DevelopmentEntitlements.plist"),
      applicationPath,
    ],
    "Could not re-sign the temporary application",
    root,
  );
  runChecked(
    "codesign",
    ["--verify", "--deep", "--strict", applicationPath],
    "Temporary application signature verification failed",
    root,
  );
}

function preserveTemporaryApplication(applicationPath, root) {
  const temporaryDirectory = resolve(root, ".tmp");
  const destination = temporaryApplicationPath(root);
  const staging = resolve(temporaryDirectory, STAGING_APPLICATION_NAME);
  const backup = resolve(temporaryDirectory, BACKUP_APPLICATION_NAME);
  mkdirSync(temporaryDirectory, { recursive: true });
  rmSync(staging, { recursive: true, force: true });
  rmSync(backup, { recursive: true, force: true });

  try {
    cpSync(applicationPath, staging, { recursive: true });
    prepareTemporaryApplication(staging, root);
    const hasPreviousApplication = existsSync(destination);
    if (hasPreviousApplication) renameSync(destination, backup);
    try {
      renameSync(staging, destination);
    } catch (error) {
      if (hasPreviousApplication && !existsSync(destination)) {
        renameSync(backup, destination);
      }
      throw error;
    }
    rmSync(backup, { recursive: true, force: true });
    rmSync(resolve(temporaryDirectory, "Research Assistant.app"), {
      recursive: true,
      force: true,
    });
  } catch (error) {
    rmSync(staging, { recursive: true, force: true });
    if (existsSync(backup) && !existsSync(destination)) {
      renameSync(backup, destination);
    }
    throw error;
  }

  console.log(`Preserved disposable debug application at ${destination}`);
  return destination;
}

export function removeLooseApplications(root, profile = "release") {
  const directory = macOsBundleDirectory(root, profile);
  if (!existsSync(directory)) return;

  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.isDirectory() && entry.name.endsWith(".app")) {
      rmSync(resolve(directory, entry.name), {
        recursive: true,
        force: true,
      });
    }
  }
}

export function buildDevelopmentApplication({
  root = process.cwd(),
  environment = process.env,
} = {}) {
  const explicitCoreOnly = environment.RESEARCH_ASSISTANT_BUILTIN_PLUGINS === "none"
    && environment.RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY === undefined;
  const selection = readDevelopmentWorkbenchSelection(root, environment);
  environment = {...environment, RESEARCH_ASSISTANT_BUILTIN_PLUGINS: "none", RESEARCH_ASSISTANT_RUNTIME_PLUGINS: "1"};
  const packages = prepareBuiltinPluginResources({root, environment,
    ...(explicitCoreOnly ? {pluginIds: []} : {})});
  const frontend = ["--config", JSON.stringify({build: {
    beforeBuildCommand: "node ../../scripts/build-workbench-composition.mjs --plugins none",
    frontendDist: resolve(root, ".tmp/workbench-compositions/core/web"),
  }, bundle: {resources: {[packages.directory + "/"]: "plugins/"}}})];
  const buildArguments = developmentWorkbenchArguments(TEMPORARY_APPLICATION_BUILD_ARGUMENTS, selection, frontend);
  prepareCodexRuntime(root);
  prepareSemanticEngine({ root, profile: "debug", environment });
  prepareDoclingRuntime({ root, environment });
  removeLooseApplications(root, "debug");
  try {
    const build = spawnSync(
      "pnpm",
      [
        "--filter",
        "@research-assistant/desktop",
        "tauri",
        "build",
        ...buildArguments,
      ],
      {
        cwd: root,
        env: environment,
        stdio: "inherit",
      },
    );
    if (build.status !== 0) {
      throw new Error("Tauri temporary development application build failed");
    }

    return preserveTemporaryApplication(
      findBundledApplication(root, "debug"),
      root,
    );
  } finally {
    removeLooseApplications(root, "debug");
  }
}
