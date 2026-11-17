import {spawn} from "node:child_process";
import {resolve} from "node:path";
import {pathToFileURL} from "node:url";
import {prepareBuiltinPluginResources} from "./prepare-builtin-plugin-packages.mjs";
import {prepareDesktopRuntime} from "./prepare-desktop-runtime.mjs";
import {developmentWorkbenchArguments, readDevelopmentWorkbenchSelection} from "./workbench-development-arguments.mjs";

export function createRuntimeDesktopDevArguments(root, extraArguments = [], selection) {
  return developmentWorkbenchArguments([
    "--filter", "@research-assistant/desktop", "tauri", "dev", "--features", "development-data",
    "--config", "src-tauri/tauri.dev.conf.json", "--no-dev-server", "--config", JSON.stringify({build: {
      beforeDevCommand: {script: "node ../../scripts/prepare-desktop-development.mjs", wait: true},
      devUrl: null, frontendDist: resolve(root, "apps/web/dist"),
    }}),
  ], selection, extraArguments);
}

export async function runDesktopDevelopment(extraArguments = []) {
  const root = process.cwd();
  const selection = readDevelopmentWorkbenchSelection(root);
  const arguments_ = createRuntimeDesktopDevArguments(root, extraArguments, selection);
  const explicitCoreOnly = process.env.RESEARCH_ASSISTANT_BUILTIN_PLUGINS === "none"
    && process.env.RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY === undefined;
  let environment = {...process.env, RESEARCH_ASSISTANT_BUILTIN_PLUGINS: "none", RESEARCH_ASSISTANT_RUNTIME_PLUGINS: "1"};
  const packages = prepareBuiltinPluginResources({root, environment, ...(explicitCoreOnly ? {pluginIds: []} : {})});
  if (environment.RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY === undefined)
    environment = {...environment, RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY: packages.directory};
  const started = performance.now();
  console.log("[desktop] Preparing Core runtime resources");
  prepareDesktopRuntime({root, environment});
  console.log(`[desktop] Core runtime resources ready in ${((performance.now() - started) / 1000).toFixed(2)}s`);
  console.log("Starting Core Workbench desktop development");
  const child = spawn("pnpm", arguments_, {stdio: "inherit", env: environment});
  return new Promise((resolveExit, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => resolveExit(signal === null ? code ?? 1 : 1));
  });
}

const invokedPath = process.argv[1];
if (
  invokedPath !== undefined &&
  import.meta.url === pathToFileURL(invokedPath).href
) {
  try {
    process.exitCode = await runDesktopDevelopment(process.argv.slice(2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
