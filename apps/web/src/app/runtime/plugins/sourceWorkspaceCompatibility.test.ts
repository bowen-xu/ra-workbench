import {expect, it, vi} from "vitest";
import {WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import {createRuntimePluginLoader} from "../../../core/runtime/plugins/loader";
import {literatureLibraryManifest} from "@research-assistant/literature-library-plugin/manifest";
import {coreCapabilities} from "../../coreCapabilities";

const manifest = {...literatureLibraryManifest, formatVersion: 1, version: "1.0.0",
  entry: "assets/index.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME};
const gateway = {discover: async () => ({packages: [{packageKey: "a".repeat(64), manifest}]})};

it.each(["core.workbench.source-workspace-host", "core.sdk.literature-shared", "core.structured-resources.write-diagnostics"])(
  "rejects the actual literature declaration before import when %s is missing or incompatible", async capabilityId => {
    for (const version of [undefined, 2]) {
      const importModule = vi.fn(async () => ({activate: vi.fn()}));
      const capabilities = coreCapabilities.filter(value => value.id !== capabilityId);
      if (version !== undefined) capabilities.push({id: capabilityId, version});
      const loader = createRuntimePluginLoader({gateway, importModule, coreCapabilities: capabilities});
      await expect(loader.load()).rejects.toMatchObject({
        code: version === undefined ? "plugin_capability_missing" : "plugin_capability_incompatible",
        diagnostic: expect.objectContaining({pluginId: "literature_library", capabilityId, provider: "core"}),
      });
      expect(importModule).not.toHaveBeenCalled();
    }
  });

it("admits the actual frontend declaration against the implemented Core surfaces without activating it", async () => {
  const activate = vi.fn(), importModule = vi.fn(async () => ({activate}));
  const loaded = await createRuntimePluginLoader({gateway, importModule, coreCapabilities}).load();
  expect(loaded.plugins.map(value => value.manifest.id)).toEqual(["literature_library"]);
  expect(importModule).toHaveBeenCalledOnce(); expect(activate).not.toHaveBeenCalled();
});

it.each([false, true])("keeps an older literature declaration compatible whether the Core diagnostic feature is present (%s)", async featureAvailable => {
  const id = "core.structured-resources.write-diagnostics", activate = vi.fn(), importModule = vi.fn(async () => ({activate}));
  const previous = {...manifest, consumes: {...manifest.consumes, required: manifest.consumes.required.filter(value => value.id !== id)}};
  const oldGateway = {discover: async () => ({packages: [{packageKey: "a".repeat(64), manifest: previous}]})};
  const loaded = await createRuntimePluginLoader({gateway: oldGateway, importModule,
    coreCapabilities: featureAvailable ? coreCapabilities : coreCapabilities.filter(value => value.id !== id)}).load();
  expect(loaded.plugins.map(value => value.manifest.id)).toEqual(["literature_library"]);
  expect(importModule).toHaveBeenCalledOnce(); expect(activate).not.toHaveBeenCalled();
});
