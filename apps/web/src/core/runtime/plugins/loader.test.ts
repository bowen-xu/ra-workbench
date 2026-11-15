import {describe, expect, it, vi} from "vitest";
import {PluginPackageError, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import {BuiltinPluginCompositionError} from "@research-assistant/workbench-sdk/composition";
import {createRuntimePluginLoader, resolveRuntimePluginDiscovery, RuntimePluginLoadError} from "./loader";

function manifest(id = "note_manager", overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    formatVersion: 1, id, version: "1.0.0", apiVersion: 1,
    entry: "assets/entry.js", styles: ["assets/entry.css"], uiRuntime: WORKBENCH_UI_RUNTIME,
    dependencies: {required: [], optional: []}, provides: [], consumes: {required: [], optional: []},
    ...overrides,
  };
}

function key(index: number): string { return index.toString(16).padStart(64, "0"); }
function catalog(...manifests: readonly unknown[]): unknown {
  return {packages: manifests.map((value, index) => ({packageKey: key(index + 1), manifest: value}))};
}
function setup(input: unknown) {
  const activate = vi.fn();
  const module = {activate};
  const gateway = {discover: vi.fn(async (): Promise<unknown> => input)};
  const importModule = vi.fn(async (_url: string): Promise<unknown> => module);
  return {activate, module, gateway, importModule,
    loader: createRuntimePluginLoader({gateway, importModule, coreCapabilities: []})};
}

it("accepts an empty catalog without importing or activating code", async () => {
  const state = setup(catalog());
  expect(await state.loader.load()).toEqual({plugins: [], bindings: [], unavailableOptional: []});
  expect(state.importModule).not.toHaveBeenCalled();
  expect(state.activate).not.toHaveBeenCalled();
});

it("imports validated same-origin resources in dependency order without activating or mounting CSS", async () => {
  const consumer = manifest("consumer", {dependencies: {required: [{id: "provider", apiVersion: 1}], optional: []}});
  const state = setup(catalog(consumer, manifest("provider")));
  const result = await state.loader.load();
  expect(result.plugins.map(plugin => plugin.manifest.id)).toEqual(["provider", "consumer"]);
  expect(result.plugins.map(plugin => plugin.packageKey)).toEqual([key(2), key(1)]);
  expect(state.importModule.mock.calls.map(([url]) => url)).toEqual([
    `/plugins/${key(2)}/assets/entry.js`, `/plugins/${key(1)}/assets/entry.js`,
  ]);
  expect(result.plugins.map(plugin => plugin.styles)).toEqual([
    [`/plugins/${key(2)}/assets/entry.css`], [`/plugins/${key(1)}/assets/entry.css`],
  ]);
  expect(result.plugins[0]?.module).toBe(state.module);
  expect(state.activate).not.toHaveBeenCalled();
});

describe("validates the whole catalog before evaluating any entry", () => {
  const cases: readonly [string, readonly unknown[], Record<string, unknown>,
    typeof PluginPackageError | typeof BuiltinPluginCompositionError][] = [
    ["manifest version", [manifest("valid"), manifest("bad", {formatVersion: 3})],
      {code: "plugin_manifest_invalid", pluginId: "bad", field: "formatVersion"}, PluginPackageError],
    ["duplicate identity", [manifest("duplicate"), manifest("duplicate")],
      {code: "plugin_duplicate_id", pluginId: "duplicate"}, BuiltinPluginCompositionError],
    ["API version", [manifest("valid"), manifest("bad", {apiVersion: 2})],
      {code: "plugin_api_incompatible", pluginId: "bad", expectedVersion: 1, actualVersion: 2}, PluginPackageError],
    ["shared UI runtime", [manifest("valid"), manifest("bad", {uiRuntime: "react@other"})],
      {code: "plugin_ui_runtime_incompatible", pluginId: "bad", expectedVersion: WORKBENCH_UI_RUNTIME, actualVersion: "react@other"}, PluginPackageError],
    ["missing required dependency", [manifest("consumer", {
      dependencies: {required: [{id: "absent", apiVersion: 1}], optional: []},
    })], {code: "plugin_dependency_missing", pluginId: "consumer", dependencyId: "absent"}, BuiltinPluginCompositionError],
    ["incompatible required dependency", [manifest("consumer", {
      dependencies: {required: [{id: "provider", apiVersion: 2}], optional: []},
    }), manifest("provider")], {code: "plugin_dependency_incompatible", pluginId: "consumer", dependencyId: "provider",
      expectedVersion: 2, actualVersion: 1}, BuiltinPluginCompositionError],
    ["dependency cycle", [manifest("consumer", {
      dependencies: {required: [{id: "provider", apiVersion: 1}], optional: []},
    }), manifest("provider", {dependencies: {required: [{id: "consumer", apiVersion: 1}], optional: []}})],
    {code: "plugin_dependency_cycle", pluginId: "consumer", cycle: ["consumer", "provider", "consumer"]}, BuiltinPluginCompositionError],
    ["missing required capability", [manifest("consumer", {consumes: {required: [{id: "catalog", version: 1}], optional: []}})],
      {code: "plugin_capability_missing", pluginId: "consumer", capabilityId: "catalog"}, BuiltinPluginCompositionError],
    ["incompatible required capability", [manifest("consumer", {consumes: {
      required: [{id: "catalog", version: 2, provider: "provider"}], optional: [],
    }}), manifest("provider", {provides: [{id: "catalog", version: 1}]})],
    {code: "plugin_capability_incompatible", pluginId: "consumer", capabilityId: "catalog", provider: "provider",
      expectedVersion: 2, actualVersion: 1}, BuiltinPluginCompositionError],
    ["ambiguous providers", [manifest("consumer", {consumes: {required: [{id: "catalog", version: 1}], optional: []}}),
      manifest("first", {provides: [{id: "catalog", version: 1}]}), manifest("second", {provides: [{id: "catalog", version: 1}]})],
    {code: "plugin_capability_ambiguous", pluginId: "consumer", capabilityId: "catalog"}, BuiltinPluginCompositionError],
  ];
  it.each(cases)("rejects %s with the shared SDK diagnostic", async (_label, manifests, diagnostic, errorClass) => {
    const state = setup(catalog(...manifests));
    const result = state.loader.load();
    await expect(result).rejects.toBeInstanceOf(errorClass);
    await expect(result).rejects.toMatchObject({diagnostic});
    expect(state.importModule).not.toHaveBeenCalled();
  });
});

it("preserves optional absence and never substitutes an unrelated named provider", async () => {
  const state = setup(catalog(manifest("consumer", {
    dependencies: {required: [], optional: [{id: "absent", apiVersion: 1}]},
    consumes: {required: [], optional: [{id: "catalog", version: 1, provider: "absent"}]},
  }), manifest("unrelated", {provides: [{id: "catalog", version: 1}]})));
  const result = await state.loader.load();
  expect(result.bindings).toEqual([]);
  expect(result.unavailableOptional).toEqual([
    {code: "plugin_dependency_missing", pluginId: "consumer", dependencyId: "absent"},
    {code: "plugin_capability_missing", pluginId: "consumer", capabilityId: "catalog", provider: "absent"},
  ]);
  expect(state.importModule).toHaveBeenCalledTimes(2);
  expect(state.activate).not.toHaveBeenCalled();
});

it("uses the original explicit capability provider selection and Core capabilities", async () => {
  const state = setup(catalog(manifest("consumer", {consumes: {required: [
    {id: "catalog", version: 1, provider: "second"}, {id: "notes", version: 1, provider: "core"},
  ], optional: []}}), manifest("first", {provides: [{id: "catalog", version: 1}]}),
  manifest("second", {provides: [{id: "catalog", version: 1}]})));
  const loader = createRuntimePluginLoader({...state, coreCapabilities: [{id: "notes", version: 1}]});
  expect((await loader.load()).bindings).toEqual([
    {pluginId: "consumer", capabilityId: "catalog", version: 1, provider: "second"},
    {pluginId: "consumer", capabilityId: "notes", version: 1, provider: "core"},
  ]);
});

it("still rejects ambiguous optional capabilities before import", async () => {
  const state = setup(catalog(manifest("consumer", {consumes: {required: [], optional: [{id: "catalog", version: 1}]}}),
    manifest("first", {provides: [{id: "catalog", version: 1}]}), manifest("second", {provides: [{id: "catalog", version: 1}]})));
  await expect(state.loader.load()).rejects.toMatchObject({code: "plugin_capability_ambiguous"});
  expect(state.importModule).not.toHaveBeenCalled();
});

const unsafePaths = ["/entry.js", "../entry.js", "assets/../entry.js", "./entry.js", "assets//entry.js",
  "https://host/entry.js", "//host/entry.js", "C:\\entry.js", "assets/%2e%2e/entry.js", "entry.js?x", "entry.js#x", "entry\0.js"];
it.each(unsafePaths)("rejects package resource path %j through the SDK decoder", async path => {
  for (const overrides of [{entry: path}, {styles: [path]}]) {
    const state = setup(catalog(manifest("valid"), manifest("bad", overrides)));
    await expect(state.loader.load()).rejects.toMatchObject({code: "plugin_manifest_invalid", diagnostic: {pluginId: "bad"}});
    expect(state.importModule).not.toHaveBeenCalled();
  }
});

const invalidDiscoveries: readonly [string, unknown, string][] = [
  ["null", null, "discovery"], ["array", [], "discovery"], ["missing packages", {}, "packages"],
  ["non-array packages", {packages: {}}, "packages"], ["invalid record", {packages: [null]}, "packages[0]"],
  ["URL key", {packages: [{packageKey: "https://host", manifest: manifest()}]}, "packages[0].packageKey"],
  ["short key", {packages: [{packageKey: "f".repeat(63), manifest: manifest()}]}, "packages[0].packageKey"],
  ["uppercase key", {packages: [{packageKey: "A".repeat(64), manifest: manifest()}]}, "packages[0].packageKey"],
  ["duplicate key", {packages: [{packageKey: key(1), manifest: manifest("first")},
    {packageKey: key(1), manifest: manifest("second")}]}, "packages[1].packageKey"],
];
it.each(invalidDiscoveries)("rejects %s at the discovery boundary", async (_label, input, field) => {
  const state = setup(input);
  await expect(state.loader.load()).rejects.toMatchObject({code: "plugin_discovery_invalid", diagnostic: {
    pluginId: "<discovery>", field, expected: expect.any(String), actual: expect.any(String),
  }});
  expect(state.importModule).not.toHaveBeenCalled();
});

it("retains the original duplicate ID diagnostic for identical copied package hashes", async () => {
  const state = setup({packages: [
    {packageKey: key(1), manifest: manifest()}, {packageKey: key(1), manifest: manifest()},
  ]});
  const attempt = state.loader.load();
  await expect(attempt).rejects.toBeInstanceOf(BuiltinPluginCompositionError);
  await expect(attempt).rejects.toMatchObject({code: "plugin_duplicate_id", diagnostic: {pluginId: "note_manager"}});
  expect(state.importModule).not.toHaveBeenCalled();
});

it.each([
  [null, "module", "null"], [[], "module", "array"], [{}, "activate", "undefined"],
  [{activate: "not callable"}, "activate", "string"], [{activate() {}, deactivate: true}, "deactivate", "boolean"],
] as const)("rejects an invalid module namespace %j without activation", async (module, field, actual) => {
  const state = setup(catalog(manifest()));
  state.importModule.mockResolvedValue(module);
  await expect(state.loader.load()).rejects.toMatchObject({code: "plugin_module_invalid", diagnostic: {
    pluginId: "note_manager", packageKey: key(1), field, actual,
  }});
  expect(state.activate).not.toHaveBeenCalled();
});

it("retains discovery failure as the cause without exposing its raw message", async () => {
  const state = setup(catalog());
  const cause = new Error("transport detail");
  state.gateway.discover.mockRejectedValueOnce(cause);
  const result = state.loader.load();
  await expect(result).rejects.toBeInstanceOf(RuntimePluginLoadError);
  await expect(result).rejects.toMatchObject({code: "plugin_discovery_failed", cause});
  await expect(result).rejects.not.toThrow("transport detail");
  expect(state.importModule).not.toHaveBeenCalled();
});

it("retains import failure identity/cause and permits an explicit rescan retry", async () => {
  const state = setup(catalog(manifest()));
  const cause = new Error("module network detail");
  state.importModule.mockRejectedValueOnce(cause);
  await expect(state.loader.load()).rejects.toMatchObject({code: "plugin_import_failed", cause, diagnostic: {
    pluginId: "note_manager", packageKey: key(1),
  }});
  expect((await state.loader.load()).plugins).toHaveLength(1);
  expect(state.importModule).toHaveBeenCalledTimes(2);
  expect(state.activate).not.toHaveBeenCalled();
});

it("coalesces concurrent scans, then rescans while reusing the same content-addressed module", async () => {
  const state = setup(catalog(manifest()));
  let release: ((value: unknown) => void) | undefined;
  state.gateway.discover.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
  const first = state.loader.load(), concurrent = state.loader.load();
  expect(first).toBe(concurrent);
  expect(state.gateway.discover).toHaveBeenCalledTimes(1);
  expect(state.importModule).not.toHaveBeenCalled();
  if (!release) throw new Error("Discovery did not start");
  release(catalog(manifest()));
  const result = await first;
  const rescanned = await state.loader.load();
  expect(state.gateway.discover).toHaveBeenCalledTimes(2);
  expect(state.importModule).toHaveBeenCalledTimes(1);
  expect(rescanned.plugins[0]?.module).toBe(result.plugins[0]?.module);
});

it("revalidates rescans before cached modules can mask a newly incompatible catalog", async () => {
  const state = setup(catalog(manifest()));
  await state.loader.load();
  state.gateway.discover.mockResolvedValueOnce(catalog(manifest(), manifest("bad", {apiVersion: 2})));
  await expect(state.loader.load()).rejects.toMatchObject({code: "plugin_api_incompatible"});
  expect(state.importModule).toHaveBeenCalledTimes(1);
});

it("drops records for absent packages and treats a changed package hash as a new module", async () => {
  const state = setup(catalog(manifest()));
  await state.loader.load();
  state.gateway.discover.mockResolvedValueOnce(catalog());
  await state.loader.load();
  await state.loader.load();
  state.gateway.discover.mockResolvedValueOnce({packages: [{packageKey: key(3), manifest: manifest()}]});
  await state.loader.load();
  expect(state.importModule.mock.calls.map(([url]) => url)).toEqual([
    `/plugins/${key(1)}/assets/entry.js`, `/plugins/${key(1)}/assets/entry.js`, `/plugins/${key(3)}/assets/entry.js`,
  ]);
});

function nativeManifest(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  const {entry: _entry, styles: _styles, uiRuntime: _uiRuntime, ...metadata} = manifest("writing_projects");
  return {...metadata, formatVersion: 2,
    native: {protocolVersion: 1, os: "macos", arch: "aarch64", entry: "native/writing-host"},
    provides: [{id: "writing.import-preview", version: 1}], ...overrides};
}

it("preflights native packages through the same dependency and capability resolver without importing code", async () => {
  const consumer = manifest("consumer", {
    dependencies: {required: [{id: "writing_projects", apiVersion: 1}], optional: []},
    consumes: {required: [{id: "writing.import-preview", version: 1, provider: "writing_projects"}], optional: []},
  });
  const input = catalog(consumer, nativeManifest());
  const resolved = resolveRuntimePluginDiscovery(input, []);
  expect(resolved.packages.map(pkg => [pkg.packageKey, pkg.manifest.id])).toEqual([
    [key(2), "writing_projects"], [key(1), "consumer"],
  ]);
  expect(resolved.packages[0]?.manifest.native?.entry).toBe("native/writing-host");
  expect(resolved.bindings).toEqual([
    {pluginId: "consumer", capabilityId: "writing.import-preview", version: 1, provider: "writing_projects"},
  ]);
  const state = setup(input);
  const loaded = await state.loader.load();
  expect(loaded.plugins.map(plugin => plugin.manifest.id)).toEqual(["consumer"]);
  expect(loaded.bindings).toEqual(resolved.bindings);
  expect(state.importModule.mock.calls.map(([url]) => url)).toEqual([`/plugins/${key(1)}/assets/entry.js`]);
  expect(state.activate).not.toHaveBeenCalled();
});

it("does not invent an ESM import or activation for a native-only catalog", async () => {
  const state = setup(catalog(nativeManifest()));
  expect(await state.loader.load()).toEqual({plugins: [], bindings: [], unavailableOptional: []});
  expect(state.importModule).not.toHaveBeenCalled();
  expect(state.activate).not.toHaveBeenCalled();
});

it("imports a combined package's genuine frontend once while preserving its native declaration", async () => {
  const state = setup(catalog(nativeManifest({entry: "writing.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME})));
  const result = await state.loader.load();
  expect(result.plugins[0]?.manifest.native?.entry).toBe("native/writing-host");
  expect(result.plugins[0]?.entryUrl).toBe(`/plugins/${key(1)}/writing.js`);
  expect(state.importModule).toHaveBeenCalledTimes(1);
  expect(state.activate).not.toHaveBeenCalled();
});

it.each([
  [nativeManifest({native: {protocolVersion: 3, os: "macos", arch: "aarch64", entry: "native/host"}}),
    "plugin_native_protocol_incompatible"],
  [nativeManifest({native: {protocolVersion: 1, os: "macos", arch: "aarch64", entry: "../host"}}),
    "plugin_manifest_invalid"],
  [nativeManifest({dependencies: {required: [{id: "missing", apiVersion: 1}], optional: []}}),
    "plugin_dependency_missing"],
] as const)("rejects an invalid native package before evaluating any frontend", async (native, code) => {
  const state = setup(catalog(manifest("valid"), native));
  await expect(state.loader.load()).rejects.toMatchObject({code, diagnostic: {pluginId: "writing_projects"}});
  expect(state.importModule).not.toHaveBeenCalled();
});
