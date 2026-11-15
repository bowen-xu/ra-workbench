/** Development always builds the Core and loads independent plugin packages. */
export function readDevelopmentWorkbenchSelection(_root, environment = process.env) {
  if (environment.RESEARCH_ASSISTANT_BUILTIN_PLUGINS !== undefined
    && environment.RESEARCH_ASSISTANT_BUILTIN_PLUGINS !== "none")
    throw new Error("plugin_static_selection_removed: select independent packages through RESEARCH_ASSISTANT_RUNTIME_PLUGIN_DIRECTORY");
  if (environment.RESEARCH_ASSISTANT_RUNTIME_PLUGINS !== undefined
    && environment.RESEARCH_ASSISTANT_RUNTIME_PLUGINS !== "1")
    throw new Error("plugin_static_selection_removed: runtime plugins are required");
  return Object.freeze({plugins: Object.freeze([]), cargoFeatures: Object.freeze([])});
}

export function developmentWorkbenchArguments(arguments_, _selection, extraArguments = [], {cargo = false} = {}) {
  if (extraArguments.some(argument => argument === "--features" || argument === "-f"
    || argument.startsWith("--features=") || argument === "--no-default-features"))
    throw new Error("plugin_build_selection_conflict: select independent plugin packages through the runtime directory");
  if (cargo) return [...arguments_, "--no-default-features", ...extraArguments];
  const delimiter = extraArguments.indexOf("--");
  const frontend = delimiter < 0 ? extraArguments : extraArguments.slice(0, delimiter);
  const runner = delimiter < 0 ? [] : extraArguments.slice(delimiter + 1);
  return [...arguments_, ...frontend, "--", "--no-default-features", ...runner];
}
