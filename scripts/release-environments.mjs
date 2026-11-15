export const PRODUCTION_IDENTIFIER = "com.bowenxu.research-assistant";
export const DEVELOPMENT_IDENTIFIER = "com.bowenxu.research-assistant.dev";
export const NATIVE_DEVELOPMENT_FEATURES =
  "development-data";
export const TEMPORARY_APPLICATION_BUILD_ARGUMENTS = [
  "--debug",
  "--features",
  "development-data",
  "--config",
  "src-tauri/tauri.dev.conf.json",
  "--bundles",
  "app",
];
export const LOCAL_RELEASE_BUILD_ARGUMENTS = [
  "--config",
  "src-tauri/tauri.local-update.conf.json",
];

/** Publication prepares the full independent package set against Core.
 * Legacy development-only resource selection cannot narrow a release. */
export function requireFullReleaseWorkbench(environment = process.env) {
  if (environment.RESEARCH_ASSISTANT_BUILTIN_PLUGINS !== undefined)
    throw new Error("plugin_release_selection_unsupported: unset RESEARCH_ASSISTANT_BUILTIN_PLUGINS before publishing; select development packages through the runtime directory");
}

function requireObject(value, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be a JSON object`);
  }

  return value;
}

export function validateReleaseConfigurations({ base, development, local }) {
  requireObject(base, "Base Tauri configuration");
  requireObject(development, "Development Tauri configuration");
  requireObject(local, "Local release Tauri configuration");

  if (base.identifier !== PRODUCTION_IDENTIFIER) {
    throw new Error(
      `Base Tauri identifier must be ${PRODUCTION_IDENTIFIER}`,
    );
  }

  if (development.identifier !== DEVELOPMENT_IDENTIFIER) {
    throw new Error(
      `Development Tauri identifier must be ${DEVELOPMENT_IDENTIFIER}`,
    );
  }

  if (Object.hasOwn(local, "identifier")) {
    throw new Error(
      "Local release configuration must inherit the production identifier",
    );
  }

  if (local.bundle?.createUpdaterArtifacts !== true) {
    throw new Error(
      "Local release configuration must create signed updater artifacts",
    );
  }

  // Ad-hoc signatures have no Apple Team ID. The certificate-only base
  // entitlements would reject our bundled semantic engine during dlopen.
  if (
    local.bundle?.macOS?.signingIdentity !== "-" ||
    local.bundle?.macOS?.entitlements !== "LocalReleaseEntitlements.plist"
  ) {
    throw new Error("Local releases must use local ad-hoc signing and library entitlements");
  }
}

export function validateReleaseBuildArguments({ temporary, local }) {
  const temporaryArguments = temporary.join(" ");
  const localArguments = local.join(" ");

  if (
    !temporaryArguments.includes("--debug") ||
    !temporaryArguments.includes("--features development-data") ||
    !temporaryArguments.includes("tauri.dev.conf.json") ||
    !temporaryArguments.includes("--bundles app")
  ) {
    throw new Error(
      "Temporary application build must be a debug, app-only development-data build",
    );
  }

  if (
    localArguments.includes("development-data") ||
    localArguments.includes("tauri.dev.conf.json") ||
    !localArguments.includes("tauri.local-update.conf.json")
  ) {
    throw new Error(
      "Local release build must use the production data environment",
    );
  }
}
