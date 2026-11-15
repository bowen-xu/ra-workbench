import type { ConfigurationRegistration } from "../core/configuration/configuration";
import type { AppRuntime } from "../core/runtime/appRuntime";
import type {BuiltinCompositionDiagnostic, BuiltinCapabilityBinding} from "./pluginComposition";

/** Core has no statically selected plugin implementations. Views and domain
 * roles arrive through the runtime plugin registry. */
export interface WorkbenchComposition {
  readonly plugins: readonly never[];
  readonly configurations: readonly ConfigurationRegistration[];
  readonly unavailableOptional: readonly BuiltinCompositionDiagnostic[];
  readonly bindings: readonly BuiltinCapabilityBinding[];
  readonly configureRuntime: (runtime: AppRuntime) => void;
}

const coreWorkbench: WorkbenchComposition = Object.freeze({
  plugins: Object.freeze([]), configurations: Object.freeze([]), unavailableOptional: Object.freeze([]),
  bindings: Object.freeze([]), configureRuntime: () => undefined,
});
export function assembleWorkbench(contributions: readonly never[] = []): WorkbenchComposition {
  if (contributions.length) throw new Error("static_plugin_composition_removed");
  return coreWorkbench;
}
