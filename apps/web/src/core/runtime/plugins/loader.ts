import {
  assertPluginCompatibility, decodePluginManifest, hasPluginFrontend,
} from "@research-assistant/workbench-sdk";
import type {
  BuiltinCapabilityContract, RuntimeFrontendPluginManifest, RuntimePluginManifest,
} from "@research-assistant/workbench-sdk";
import {resolveBuiltinPlugins} from "@research-assistant/workbench-sdk/composition";
import type {
  BuiltinCapabilityBinding, BuiltinCompositionDiagnostic,
} from "@research-assistant/workbench-sdk/composition";
import type {RuntimePluginModule} from "@research-assistant/workbench-sdk/activation";

export interface RuntimePluginDiscoveryGateway {
  discover(): Promise<unknown>;
}

export interface LoadedRuntimePlugin {
  /** Production discovery supplies the verified content key directly. */
  readonly packageKey?: string;
  readonly manifest: RuntimeFrontendPluginManifest;
  readonly module: RuntimePluginModule;
  readonly entryUrl: string;
  readonly styles: readonly string[];
}

export interface RuntimePluginLoadResult {
  /** Dependency order, independent from the discovery/catalog order. */
  readonly plugins: readonly LoadedRuntimePlugin[];
  readonly bindings: readonly BuiltinCapabilityBinding[];
  readonly unavailableOptional: readonly BuiltinCompositionDiagnostic[];
}

export type RuntimePluginLoadErrorCode =
  | "plugin_discovery_failed" | "plugin_discovery_invalid"
  | "plugin_import_failed" | "plugin_module_invalid";

export interface RuntimePluginLoadDiagnostic {
  readonly code: RuntimePluginLoadErrorCode;
  readonly pluginId: string;
  readonly packageKey?: string;
  readonly field?: string;
  readonly expected?: string;
  readonly actual?: string;
}

export class RuntimePluginLoadError extends Error {
  readonly code: RuntimePluginLoadErrorCode;
  constructor(readonly diagnostic: RuntimePluginLoadDiagnostic, options?: ErrorOptions) {
    super(`${diagnostic.code}: ${diagnostic.pluginId}${diagnostic.field ? ` / ${diagnostic.field}` : ""}`, options);
    this.name = "RuntimePluginLoadError";
    this.code = diagnostic.code;
  }
}

export interface ResolvedRuntimePluginPackage {
  readonly packageKey: string;
  readonly manifest: RuntimePluginManifest;
}

export interface RuntimePluginDiscoveryResolution {
  /** Includes native-only packages, in the existing resolver's dependency order. */
  readonly packages: readonly ResolvedRuntimePluginPackage[];
  readonly bindings: readonly BuiltinCapabilityBinding[];
  readonly unavailableOptional: readonly BuiltinCompositionDiagnostic[];
}

export interface RuntimePluginLoaderOptions {
  readonly gateway: RuntimePluginDiscoveryGateway;
  readonly coreCapabilities: readonly BuiltinCapabilityContract[];
  /** Tests may substitute this effect; production always imports the validated
   * same-origin package URL with the browser's standard ESM loader. */
  readonly importModule?: (entryUrl: string) => Promise<unknown>;
}

export interface RuntimePluginLoader {
  /** Rescans on each settled call. Concurrent calls share one scan/load.
   * Imports may evaluate top-level code; this method never calls activate,
   * mounts CSS, grants capabilities or rolls back earlier module evaluation. */
  load(): Promise<RuntimePluginLoadResult>;
}

function valueType(value: unknown): string {
  return value === null ? "null" : Array.isArray(value) ? "array" : typeof value;
}

function discoveryInvalid(field: string, expected: string, actual: string): never {
  throw new RuntimePluginLoadError({code: "plugin_discovery_invalid", pluginId: "<discovery>", field, expected, actual});
}

function record(value: unknown, field: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) discoveryInvalid(field, "object", valueType(value));
  return value as Record<string, unknown>;
}

function decodeDiscovery(value: unknown): readonly ResolvedRuntimePluginPackage[] {
  const envelope = record(value, "discovery");
  if (!Array.isArray(envelope.packages)) discoveryInvalid("packages", "array", valueType(envelope.packages));
  return envelope.packages.map((value: unknown, index: number) => {
    const field = `packages[${index}]`;
    const item = record(value, field);
    if (typeof item.packageKey !== "string" || !/^[a-f0-9]{64}$/u.test(item.packageKey)) {
      discoveryInvalid(`${field}.packageKey`, "64 lowercase hexadecimal characters", valueType(item.packageKey));
    }
    return {packageKey: item.packageKey, manifest: decodePluginManifest(item.manifest)};
  });
}

/** Pure discovery preflight shared by ESM loading and the native preview App
 * adapter. Resolving metadata neither imports code nor grants native execution. */
export function resolveRuntimePluginDiscovery(
  discovered: unknown, coreCapabilities: readonly BuiltinCapabilityContract[],
): RuntimePluginDiscoveryResolution {
  const packages = decodeDiscovery(discovered);
  for (const pkg of packages) assertPluginCompatibility(pkg.manifest);
  const composition = resolveBuiltinPlugins(
    packages.map(pkg => pkg.manifest), packages.map(pkg => pkg.manifest.id), coreCapabilities,
  );
  // Let the shared resolver retain duplicate ID diagnostics for identical
  // copied packages before rejecting a duplicate content key across other IDs.
  const keys = new Set<string>();
  for (const [index, pkg] of packages.entries()) {
    if (keys.has(pkg.packageKey)) discoveryInvalid(`packages[${index}].packageKey`, "unique package key", "duplicate");
    keys.add(pkg.packageKey);
  }
  const byManifest = new Map(packages.map(pkg => [pkg.manifest, pkg]));
  return {
    packages: composition.activationOrder.map(manifest => {
      const pkg = byManifest.get(manifest);
      if (!pkg) throw new Error("Resolved manifest is absent from its decoded package catalog");
      return pkg;
    }),
    bindings: composition.bindings, unavailableOptional: composition.unavailableOptional,
  };
}

function resourceUrl(pkg: ResolvedRuntimePluginPackage, path: string): string {
  // Both segments have already been decoded. Never accept a caller-provided URL
  // or normalize a traversal into the controlled same-origin protocol prefix.
  return `/plugins/${pkg.packageKey}/${path}`;
}

function decodeModule(value: unknown, pkg: ResolvedRuntimePluginPackage): RuntimePluginModule {
  const diagnostic = {code: "plugin_module_invalid", pluginId: pkg.manifest.id, packageKey: pkg.packageKey} as const;
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new RuntimePluginLoadError({...diagnostic, field: "module", expected: "module namespace", actual: valueType(value)});
  }
  const module = value as Record<string, unknown>;
  for (const name of ["activate", "deactivate"] as const) {
    if (name === "deactivate" && module[name] === undefined) continue;
    if (typeof module[name] !== "function") {
      throw new RuntimePluginLoadError({...diagnostic, field: name, expected: "function", actual: valueType(module[name])});
    }
  }
  return module as unknown as RuntimePluginModule;
}

async function importRuntimeModule(entryUrl: string): Promise<unknown> {
  return import(/* @vite-ignore */ entryUrl);
}

/** Instance-owned cache is bounded to the latest valid catalog's entry URLs.
 * Content-addressed package keys identify immutable resources; absent entries
 * are evicted on the next valid scan, failures are evicted immediately, and
 * dropping the loader drops its records (browser ESM evaluation is not undone). */
export function createRuntimePluginLoader(options: RuntimePluginLoaderOptions): RuntimePluginLoader {
  const importModule = options.importModule ?? importRuntimeModule;
  const modules = new Map<string, Promise<RuntimePluginModule>>();
  let pending: Promise<RuntimePluginLoadResult> | undefined;

  const scan = async (): Promise<RuntimePluginLoadResult> => {
    let discovered: unknown;
    try { discovered = await options.gateway.discover(); }
    catch (cause) {
      throw new RuntimePluginLoadError({code: "plugin_discovery_failed", pluginId: "<discovery>"}, {cause});
    }
    const resolution = resolveRuntimePluginDiscovery(discovered, options.coreCapabilities);
    // The complete catalog must pass before the first executable import.
    const activeUrls = new Set(resolution.packages.flatMap(pkg =>
      hasPluginFrontend(pkg.manifest) ? [resourceUrl(pkg, pkg.manifest.entry)] : []));
    for (const url of modules.keys()) if (!activeUrls.has(url)) modules.delete(url);
    const plugins: LoadedRuntimePlugin[] = [];
    for (const pkg of resolution.packages) {
      const {manifest} = pkg;
      if (!hasPluginFrontend(manifest)) continue;
      const entryUrl = resourceUrl(pkg, manifest.entry);
      let loaded = modules.get(entryUrl);
      if (!loaded) {
        loaded = (async () => {
          let value: unknown;
          try { value = await importModule(entryUrl); }
          catch (cause) {
            throw new RuntimePluginLoadError({code: "plugin_import_failed", pluginId: manifest.id, packageKey: pkg.packageKey}, {cause});
          }
          return decodeModule(value, pkg);
        })();
        modules.set(entryUrl, loaded);
      }
      let module: RuntimePluginModule;
      try { module = await loaded; }
      catch (error) { modules.delete(entryUrl); throw error; }
      plugins.push({packageKey: pkg.packageKey, manifest, module, entryUrl, styles: manifest.styles.map(path => resourceUrl(pkg, path))});
    }
    return {plugins, bindings: resolution.bindings, unavailableOptional: resolution.unavailableOptional};
  };
  return {
    load() {
      pending ??= scan().finally(() => { pending = undefined; });
      return pending;
    },
  };
}
