import path from "node:path";
import {createRequire} from "node:module";
import {fileURLToPath, pathToFileURL} from "node:url";

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** Use Vite's installed config loader, rather than Node's evolving TypeScript
 * syntax support or a second implementation of the manifest dependency rules. */
export async function readWorkbenchBuildPlan(input = process.env.RESEARCH_ASSISTANT_BUILTIN_PLUGINS, root = repository) {
  const require = createRequire(path.join(root, "apps/web/package.json"));
  const {loadConfigFromFile} = await import(pathToFileURL(require.resolve("vite")).href);
  if (input !== undefined && input !== "none") throw new Error("plugin_static_selection_removed: the application builds only Core");
  const key = "RESEARCH_ASSISTANT_BUILTIN_PLUGINS", previous = process.env[key];
  if (input === undefined) delete process.env[key]; else process.env[key] = input;
  try {
    const loaded = await loadConfigFromFile({command: "build", mode: "production"}, path.join(root, "apps/web/vite.config.ts"), path.join(root, "apps/web"), "silent");
    const plugins = loaded?.config.plugins?.flat(Infinity) ?? [];
    const plugin = plugins.find(value => value && typeof value === "object" && value.name === "workbench-runtime-core");
    const plan = plugin?.api?.selection;
    if (!plan || !Array.isArray(plan.plugins) || !Array.isArray(plan.cargoFeatures)) throw new Error("plugin_build_plan_unavailable");
    const runtime = plugins.find(value => value && typeof value === "object" && value.name === "workbench-runtime-sdk");
    let runtimeSdkTypeRoots;
    if (runtime) {
      if (!Array.isArray(runtime.api?.facades)) throw new Error("runtime_sdk_type_roots_unavailable");
      runtimeSdkTypeRoots = [...new Set(runtime.api.facades.flatMap(facade => {
        if (typeof facade.source !== "string") throw new Error("runtime_sdk_type_root_invalid");
        // External public packages keep their own declarations/checks. Every
        // application producer in the actual runtime entry plan must be Core.
        if (!facade.source.startsWith("apps/")) return [];
        const source = path.relative(root, path.resolve(root, facade.source)).replaceAll("\\", "/");
        if (!source.startsWith("apps/web/src/core/public/") || !source.endsWith(".ts"))
          throw new Error("runtime_sdk_type_root_outside_core: " + facade.source);
        return [source];
      }))];
      if (!runtimeSdkTypeRoots.length) throw new Error("runtime_sdk_type_roots_unavailable");
    }
    return {plugins: [...plan.plugins], cargoFeatures: [...plan.cargoFeatures], unavailableOptional: [...plan.unavailableOptional],
      ...(runtimeSdkTypeRoots ? {runtimeSdkTypeRoots} : {})};
  } finally {
    if (previous === undefined) delete process.env[key]; else process.env[key] = previous;
  }
}
