import type {Plugin} from "vite";
import {fileURLToPath} from "node:url";

const virtual = "virtual:workbench-composition";
const resolved = "\0" + virtual;
const selection = Object.freeze({plugins: Object.freeze([]), cargoFeatures: Object.freeze([]), unavailableOptional: Object.freeze([])});

/** All production builds contain the same Core. Plugin selection happens in
 * runtime discovery and cannot reintroduce a statically linked domain. */
export function readWorkbenchBuildSelection(input = process.env.RESEARCH_ASSISTANT_BUILTIN_PLUGINS) {
  if (input !== undefined && input !== "none") throw new Error("static_plugin_composition_removed");
  return {selected: [], resolution: {plugins: [], activationOrder: [], bindings: [], unavailableOptional: []}, plan: selection};
}

export function workbenchCompositionPlugin(input = process.env.RESEARCH_ASSISTANT_BUILTIN_PLUGINS): Plugin {
  readWorkbenchBuildSelection(input);
  let build = false;
  const root = fileURLToPath(new URL("../../", import.meta.url)).replaceAll("\\", "/");
  const source = fileURLToPath(new URL("./src/app/workbenchComposition.ts", import.meta.url));
  return {
    name: "workbench-runtime-core", api: Object.freeze({selection}),
    configResolved(config) {build = config.command === "build";},
    transformIndexHtml: {order: "pre", handler(html) {return build ? html.replace('/src/main.tsx', '/src/main.selected.tsx') : html;}},
    resolveId(id) {if (id === virtual) return resolved;},
    load(id) {if (id === resolved) return 'import {assembleWorkbench} from ' + JSON.stringify(source) + '; export const composition = assembleWorkbench();';},
    generateBundle() {
      if (!build) return;
      const modules = [...this.getModuleIds()].filter(id => id.includes("/src/")).sort();
      const forbidden = modules.filter(id => /\/plugins\/(?:ra-literature-library|ra-note-manager|ra-writing-projects|literature_library|note_manager|writing_projects)\/src\//u.test(id));
      if (forbidden.length) this.error("plugin_build_closure_violation: " + forbidden.join("\n"));
      this.emitFile({type: "asset", fileName: "workbench-composition.json", source: JSON.stringify({
        ...selection, modules: modules.map(id => id.replace(root, "")),
      }, null, 2)});
    },
  };
}
