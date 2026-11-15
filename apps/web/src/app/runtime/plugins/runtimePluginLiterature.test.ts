// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import {decodePluginManifest, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import type {PluginActivationContext} from "@research-assistant/workbench-sdk/activation";
import type {LoadedRuntimePlugin} from "../../../core/runtime/plugins/loader";
import {selectAppRuntime} from "../../../core/runtime/appRuntime";
import {createRuntimePluginWorkbench} from "./runtimePluginWorkbench";

const native = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("../../../core/runtime/platformCore", () => ({invoke: native.invoke}));
afterEach(() => {native.invoke.mockReset(); selectAppRuntime({mode: "browser", storage: window.localStorage, gateways: {}, dispose() {}});});

it("binds the discovered key rather than an entry URL and withdraws the original context on deactivation", async () => {
  selectAppRuntime({mode: "desktop", storage: window.localStorage, gateways: {}, dispose() {}});
  const key = "b".repeat(64), urlKey = "a".repeat(64);
  let context: PluginActivationContext | undefined;
  const manifest = decodePluginManifest({formatVersion: 1, id: "literature_library", apiVersion: 1, version: "1.0.0",
    entry: "entry.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME, dependencies: {required: [], optional: []},
    consumes: {required: [], optional: []}, provides: [{id: "literature.doi-preview", version: 1}]});
  const plugin: LoadedRuntimePlugin = {packageKey: key, manifest, entryUrl: `/plugins/${urlKey}/entry.js`, styles: [],
    module: {activate(value) {context = value;}}};
  const workbench = createRuntimePluginWorkbench({storage: window.localStorage});
  await workbench.activate({plugins: [plugin], bindings: [], unavailableOptional: []});
  expect(context?.literatureRuntime).toBeDefined();
  native.invoke.mockResolvedValueOnce({id: "candidate"});
  await context!.literatureRuntime!.literature.previewDoi("doi");
  expect(native.invoke).toHaveBeenCalledExactlyOnceWith("preview_runtime_doi_metadata", {packageKey: key, doi: "doi", itemId: undefined});
  await workbench.deactivate("literature_library");
  await expect(context!.literatureRuntime!.literature.previewDoi("later")).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  expect(native.invoke).toHaveBeenCalledTimes(1);
});

it("does not guess a missing key or expose a literature port to another plugin", async () => {
  for (const id of ["literature_library", "note_manager"]) {
    let context: PluginActivationContext | undefined;
    const manifest = decodePluginManifest({formatVersion: 1, id, apiVersion: 1, version: "1.0.0", entry: "entry.js", styles: [],
      uiRuntime: WORKBENCH_UI_RUNTIME, dependencies: {required: [], optional: []}, consumes: {required: [], optional: []}, provides: []});
    const plugin: LoadedRuntimePlugin = {manifest, entryUrl: `/plugins/${"a".repeat(64)}/entry.js`, styles: [], module: {activate(value) {context = value;}}};
    const workbench = createRuntimePluginWorkbench({storage: window.localStorage});
    await workbench.activate({plugins: [plugin], bindings: [], unavailableOptional: []});
    expect(context?.literatureRuntime).toBeUndefined();
    await workbench.dispose();
  }
  expect(native.invoke).not.toHaveBeenCalled();
});
