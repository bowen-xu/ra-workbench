// @vitest-environment happy-dom
import {act, createElement} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {decodePluginManifest, hasPluginFrontend, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import type {RuntimePluginModule} from "@research-assistant/workbench-sdk/activation";
import {getOptionalLiteratureGateway, type LiteratureInteroperability} from "../../../core/public/domainCapabilities";
import {selectAppRuntime, type AppRuntime} from "../../../core/runtime/appRuntime";
import {getLiteratureGateway as getSourceLiteratureGateway} from "../../../core/services/sourceDomainGateways";
import type {LoadedRuntimePlugin} from "../../../core/runtime/plugins/loader";
import {createRuntimePluginWorkbench} from "../plugins/runtimePluginWorkbench";
import {bindRuntimeLiteratureInteroperability} from "./runtimeLiteratureInteroperability";

const native = vi.hoisted(() => ({invoke: vi.fn()}));
vi.mock("../../../core/runtime/platformCore", () => ({invoke: native.invoke}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
const key = "a".repeat(64);
const roots: Root[] = [];
const workbenches: ReturnType<typeof createRuntimePluginWorkbench>[] = [];
const releases: (() => void)[] = [];
afterEach(async () => {
  for (const root of roots.splice(0)) await act(async () => root.unmount());
  for (const workbench of workbenches.splice(0)) await workbench.dispose();
  for (const release of releases.splice(0)) release();
  native.invoke.mockReset();
  selectAppRuntime({mode: "browser", storage: localStorage, gateways: {}, dispose() {}});
});
function desktop(): AppRuntime {
  const runtime: AppRuntime = {mode: "desktop", storage: localStorage, gateways: {}, dispose() {}};
  selectAppRuntime(runtime);
  return runtime;
}
function methods() {
  return {get: vi.fn(), list: vi.fn(), save: vi.fn(), updateMetadataField: vi.fn()};
}

it("retains the existing four methods, ready gate and captured-instance rejection", async () => {
  const runtime = desktop(), literature = methods();
  let ready = false;
  const release = bindRuntimeLiteratureInteroperability(
    {literature} as unknown as RuntimeLiteraturePort, () => ready, key,
  )!;
  releases.push(release);
  expect(getOptionalLiteratureGateway()).toBeUndefined();
  ready = true;
  const selected = getOptionalLiteratureGateway()!;
  expect(Object.keys(selected).sort()).toEqual(["get", "list", "save", "updateMetadataField"]);
  await selected.get("item");
  expect(literature.get).toHaveBeenCalledExactlyOnceWith("item");
  ready = false;
  expect(getOptionalLiteratureGateway()).toBeUndefined();
  await expect(selected.list()).rejects.toMatchObject({
    code: "literature_citations_unavailable", packageKey: key, pluginId: "literature_library",
  });
  expect(literature.list).not.toHaveBeenCalled();
  ready = true;
  expect(getOptionalLiteratureGateway()).toBe(selected);
  selectAppRuntime({...runtime, gateways: {}});
  await expect(selected.get("later")).rejects.toMatchObject({code: "literature_citations_unavailable"});
});

it("preserves explicitly selected static and preview owners", () => {
  for (const mode of ["desktop", "preview"] as const) {
    const existing = methods();
    const runtime: AppRuntime = {
      mode, storage: localStorage, gateways: {getLiteratureGateway: existing as never}, dispose() {},
    };
    selectAppRuntime(runtime);
    expect(bindRuntimeLiteratureInteroperability(
      {literature: methods()} as unknown as RuntimeLiteraturePort, () => true, key,
    )).toBeUndefined();
    expect(getOptionalLiteratureGateway()).toBe(existing);
  }
});

it("uses the same finite port for shared source reads and keeps absent-source behavior", async () => {
  desktop();
  const literature = methods();
  literature.get.mockResolvedValue({id: "item", title: "Synthetic"});
  literature.list.mockResolvedValue([]);
  const release = bindRuntimeLiteratureInteroperability(
    {literature} as unknown as RuntimeLiteraturePort, () => true, key,
  )!;
  releases.push(release);
  const source = getSourceLiteratureGateway();
  await expect(source.get("item")).resolves.toEqual({id: "item", title: "Synthetic"});
  await expect(source.list()).resolves.toEqual([]);
  expect(literature.get).toHaveBeenCalledExactlyOnceWith("item");
  release();
  await expect(source.get("item")).rejects.toMatchObject({code: "source_domain_unavailable"});
  await expect(source.list()).resolves.toEqual([]);
});

function plugin(id: string, activate: RuntimePluginModule["activate"]): LoadedRuntimePlugin {
  const manifest = decodePluginManifest({
    formatVersion: 2, id, version: "1.0.0", apiVersion: 1, entry: "entry.js", styles: [],
    uiRuntime: WORKBENCH_UI_RUNTIME,
    dependencies: {required: [], optional: []}, consumes: {required: [], optional: []},
    provides: id === "literature_library" ? [
      {id: "literature.catalog-read", version: 1}, {id: "literature.catalog-save", version: 1},
      {id: "literature.metadata", version: 1},
    ] : [],
  });
  if (!hasPluginFrontend(manifest)) throw new Error("fixture frontend required");
  return {
    packageKey: id === "literature_library" ? key : "b".repeat(64),
    entryUrl: `/plugins/${id}/entry.js`, manifest, module: {activate}, styles: [],
  };
}

it("rerenders an already mounted consumer when the later provider becomes ready or stops", async () => {
  desktop();
  const workbench = createRuntimePluginWorkbench({storage: localStorage});
  workbenches.push(workbench);
  let captured: LiteratureInteroperability | undefined;
  const consumer = plugin("writing_projects", context => {
    context.registerView({id: "writing", title: "Writing", render() {
      captured = getOptionalLiteratureGateway();
      return createElement("span", null, captured ? "available" : "absent");
    }});
  });
  await workbench.activate({plugins: [consumer], bindings: [], unavailableOptional: []});
  const view = workbench.views.getSnapshot()[0]!;
  const host = document.createElement("div"), root = createRoot(host);
  roots.push(root);
  await act(async () => root.render(createElement(() => view.render({active: true}))));
  expect(host.textContent).toBe("absent");
  const provider = plugin("literature_library", context => {
    context.registerView({id: "library", title: "Literature", render: () => null});
  });
  await act(async () => {
    await workbench.activate({plugins: [consumer, provider], bindings: [], unavailableOptional: []});
  });
  expect(host.textContent).toBe("available");
  const old = captured!;
  native.invoke.mockResolvedValueOnce([]);
  await expect(old.list()).resolves.toEqual([]);
  expect(native.invoke).toHaveBeenCalledExactlyOnceWith("list_runtime_literature", {packageKey: key, query: undefined});
  workbench.attachNavigation({open: async () => {}, prepareClose: async () => true});
  await act(async () => workbench.deactivate("literature_library"));
  expect(host.textContent).toBe("absent");
  await expect(old.list()).rejects.toMatchObject({code: "literature_citations_unavailable", packageKey: key});
});
