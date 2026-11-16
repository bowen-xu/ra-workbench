import {prepareCodexRuntime} from "./prepare-codex-runtime.mjs";
import {prepareDoclingRuntime} from "./prepare-docling-runtime.mjs";
import {prepareSemanticEngine} from "./prepare-semantic-engine.mjs";

/** Prepare the Workbench-owned runtime resources required by Tauri's manifest. */
export function prepareDesktopRuntime({root = process.cwd(), environment = process.env} = {}) {
  prepareCodexRuntime(root);
  prepareSemanticEngine({root, profile: "debug", environment});
  prepareDoclingRuntime({root, environment});
}
