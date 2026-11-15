import {expect, it, vi} from "vitest";
import {WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import {BuiltinPluginCompositionError, resolveBuiltinPlugins} from "@research-assistant/workbench-sdk/composition";
import {createRuntimePluginLoader} from "../../../core/runtime/plugins/loader";
import {noteManagerManifest} from "@research-assistant/note-manager-plugin/manifest";
import {coreCapabilities} from "../../coreCapabilities";

function setup(capabilities = coreCapabilities) {
  // Preserve the real plugin's full consumption contract; only package paths
  // and build metadata are fixture data.
  const manifest = {...noteManagerManifest, formatVersion: 1, version: "1.0.0",
    entry: "assets/entry.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME};
  const activate = vi.fn(), importModule = vi.fn(async () => ({activate}));
  const gateway = {discover: vi.fn(async () => ({packages: [{packageKey: "a".repeat(64), manifest}]}))};
  return {activate, importModule, loader: createRuntimePluginLoader({gateway, importModule, coreCapabilities: capabilities})};
}

it("binds the real note package to the current Core workspace host without changing its original API versions", async () => {
  const resolved = resolveBuiltinPlugins([noteManagerManifest], [noteManagerManifest.id], coreCapabilities);
  const workspaceHost = {pluginId: "note_manager", capabilityId: "core.notes.workspace-host", version: 1, provider: "core"};
  expect(resolved.bindings).toContainEqual(workspaceHost);
  expect(resolved.bindings).toContainEqual({pluginId: "note_manager", capabilityId: "core.platform.confirmation", version: 1, provider: "core"});
  expect(resolved.bindings).toContainEqual({pluginId: "note_manager", capabilityId: "core.notes", version: 1, provider: "core"});
  expect(noteManagerManifest.apiVersion).toBe(1);
  const {loader, importModule, activate} = setup();
  expect((await loader.load()).bindings).toContainEqual(workspaceHost);
  expect(importModule).toHaveBeenCalledExactlyOnceWith(`/plugins/${"a".repeat(64)}/assets/entry.js`);
  expect(activate).not.toHaveBeenCalled();
});

it.each(["core.notes.workspace-host", "core.platform.confirmation"])("rejects the new note package without %s before evaluating its ESM entry", async capabilityId => {
  const olderCore = coreCapabilities.filter(capability => capability.id !== capabilityId);
  const {loader, importModule, activate} = setup(olderCore);
  const pending = loader.load();
  await expect(pending).rejects.toBeInstanceOf(BuiltinPluginCompositionError);
  await expect(pending).rejects.toMatchObject({code: "plugin_capability_missing", diagnostic: {
    pluginId: "note_manager", capabilityId, provider: "core",
  }});
  expect(importModule).not.toHaveBeenCalled();
  expect(activate).not.toHaveBeenCalled();
});

it("requires the exact confirmation contract version before any plugin import", async () => {
  const capabilities = coreCapabilities.map(capability => capability.id === "core.platform.confirmation" ? {...capability, version: 2} : capability);
  const {loader, importModule, activate} = setup(capabilities);
  await expect(loader.load()).rejects.toMatchObject({code: "plugin_capability_incompatible", diagnostic: {
    pluginId: "note_manager", capabilityId: "core.platform.confirmation", provider: "core", expectedVersion: 1, actualVersion: 2,
  }});
  expect(importModule).not.toHaveBeenCalled(); expect(activate).not.toHaveBeenCalled();
});
