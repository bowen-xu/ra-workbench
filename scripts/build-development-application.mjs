import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { buildDevelopmentApplication } from "./development-application.mjs";
import {
  LOCAL_RELEASE_BUILD_ARGUMENTS,
  TEMPORARY_APPLICATION_BUILD_ARGUMENTS,
  validateReleaseBuildArguments,
  validateReleaseConfigurations,
} from "./release-environments.mjs";

const root = process.cwd();
const readJson = (path) => JSON.parse(readFileSync(resolve(root, path), "utf8"));

validateReleaseConfigurations({
  base: readJson("apps/desktop/src-tauri/tauri.conf.json"),
  development: readJson("apps/desktop/src-tauri/tauri.dev.conf.json"),
  local: readJson("apps/desktop/src-tauri/tauri.local-update.conf.json"),
});
validateReleaseBuildArguments({
  temporary: TEMPORARY_APPLICATION_BUILD_ARGUMENTS,
  local: LOCAL_RELEASE_BUILD_ARGUMENTS,
});

buildDevelopmentApplication({ root });
