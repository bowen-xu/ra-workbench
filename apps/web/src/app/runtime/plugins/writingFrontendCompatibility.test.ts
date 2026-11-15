import {expect, it, vi} from "vitest";
import {WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import {createRuntimePluginLoader} from "../../../core/runtime/plugins/loader";
import {writingProjectsManifest} from "@research-assistant/writing-projects-plugin/manifest";
import {coreCapabilities} from "../../coreCapabilities";

const manifest = {...writingProjectsManifest, formatVersion: 1, version: "1.0.0",
  entry: "assets/index.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME};
const gateway = {discover: async () => ({packages: [{packageKey: "a".repeat(64), manifest}]})};

it.each([
  ["core.workbench.writing-frontend", undefined], ["core.workbench.writing-frontend", 2],
  ["core.workbench.writing-runtime", undefined], ["core.workbench.writing-runtime", 2],
  ["core.agent.writing-dispatch-admission", undefined], ["core.agent.writing-dispatch-admission", 2],
  ["core.agent.writing-compaction-retirement", undefined], ["core.agent.writing-compaction-retirement", 2],
])("rejects the writing frontend before import with %s version %s", async (capabilityId, version) => {
  const capabilities = coreCapabilities.filter(value => value.id !== capabilityId);
  if (version !== undefined) capabilities.push({id: capabilityId, version});
  const importModule = vi.fn(async () => ({activate: vi.fn()}));
  const loader = createRuntimePluginLoader({gateway, importModule, coreCapabilities: capabilities});
  await expect(loader.load()).rejects.toMatchObject({
    code: version === undefined ? "plugin_capability_missing" : "plugin_capability_incompatible",
    diagnostic: expect.objectContaining({pluginId: "writing_projects", capabilityId, provider: "core"}),
  });
  expect(importModule).not.toHaveBeenCalled();
});

it("admits the actual writing declaration without activating or claiming runtime roles", async () => {
  const activate = vi.fn(), importModule = vi.fn(async () => ({activate}));
  const loaded = await createRuntimePluginLoader({gateway, importModule, coreCapabilities}).load();
  expect(loaded.plugins.map(value => value.manifest.id)).toEqual(["writing_projects"]);
  expect(importModule).toHaveBeenCalledOnce();
  expect(activate).not.toHaveBeenCalled();
});

it("continues to admit the previous writing declaration with the new Core", async () => {
  const previous = {...manifest, consumes: {...manifest.consumes,
    required: manifest.consumes.required.filter(value => !["core.agent.writing-dispatch-admission", "core.agent.writing-compaction-retirement"].includes(value.id))}};
  const importModule = vi.fn(async () => ({activate: vi.fn()}));
  const loaded = await createRuntimePluginLoader({
    gateway: {discover: async () => ({packages: [{packageKey: "a".repeat(64), manifest: previous}]})},
    importModule, coreCapabilities,
  }).load();
  expect(loaded.plugins.map(value => value.manifest.id)).toEqual(["writing_projects"]);
  expect(importModule).toHaveBeenCalledOnce();
});

it("admits the preceding dispatch-aware writing package without the new retirement requirement", async () => {
  const previous = {...manifest, consumes: {...manifest.consumes,
    required: manifest.consumes.required.filter(value => value.id !== "core.agent.writing-compaction-retirement")}};
  const importModule = vi.fn(async () => ({activate: vi.fn()}));
  const loaded = await createRuntimePluginLoader({gateway: {discover: async () => ({packages: [{packageKey: "a".repeat(64), manifest: previous}]})},
    importModule, coreCapabilities}).load();
  expect(loaded.plugins.map(value => value.manifest.id)).toEqual(["writing_projects"]);
  expect(importModule).toHaveBeenCalledOnce();
});
