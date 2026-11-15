// @vitest-environment happy-dom
import {act, type ReactNode} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {decodePluginManifest, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import type {PluginActivationContext} from "@research-assistant/workbench-sdk/activation";
import type {LoadedRuntimePlugin, RuntimePluginLoadResult} from "../../../core/runtime/plugins/loader";
import {writeWorkspaceState} from "../../../core/runtime/workspaceState";
import {WorkbenchApp} from "../../WorkbenchApp";
import {assembleWorkbench} from "../../workbenchComposition";
import {createRuntimePluginWorkbench, type RuntimePluginWorkbench} from "./runtimePluginWorkbench";

vi.mock("../../../core/runtime/appStorage", () => ({getAppStorage: () => localStorage}));
vi.mock("../../../core/runtime/appRuntime", async importOriginal => ({
  ...await importOriginal<typeof import("../../../core/runtime/appRuntime")>(), getAppRuntime: () => ({storage: localStorage}),
}));
vi.mock("../../../core/runtime/platformCore", async importOriginal => ({
  ...await importOriginal<typeof import("../../../core/runtime/platformCore")>(), isDesktopRuntime: () => false,
}));
vi.mock("../../../core/runtime/desktopZoom", () => ({installDesktopZoom: () => () => undefined}));
vi.mock("../../desktopLifecycle", () => ({installDesktopLifecycle: async () => () => undefined}));
vi.mock("../../../core/components/text-editor/useTextDocuments", () => ({useTextDocuments: () => ({
  activeId: undefined, surface: null, deactivate() {}, forgetOwner() {}, restoreOwner() {},
})}));
vi.mock("../../../core/features/notes/workspace/useNoteDocumentTabHost", () => ({useNoteDocumentTabHost: () => undefined}));
vi.mock("../../../core/public/noteDocuments", () => ({CoreNoteWorkspace: () => null,
  CoreNoteWorkspaceHost: ({children}: {children: ReactNode}) => children}));
vi.mock("../../../core/features/agent/AgentCanvasShell", () => ({AgentCanvasShell: ({children}: {children: ReactNode}) => children}));
vi.mock("../../notes/ReadingNoteFloatingPanel", () => ({ReadingNoteFloatingPanel: () => null}));
vi.mock("../../../core/features/settings/SettingsWorkspace", () => ({SettingsWorkspace: () => <span>Core settings</span>}));
vi.mock("../../../core/components/AppToaster", () => ({AppToaster: () => null}));
vi.mock("../../../core/components/AppUpdateControl", () => ({AppUpdateControl: () => null}));
vi.mock("../../../core/components/PreviewResetControl", () => ({PreviewResetControl: () => null}));
vi.mock("../../../core/components/NotesPreviewControl", () => ({NotesPreviewControl: () => null}));
vi.mock("../../../core/components/NoteRecoveryPreviewControl", () => ({NoteRecoveryPreviewControl: () => null}));

const roots: Root[] = [];
const workbenches: RuntimePluginWorkbench[] = [];
beforeEach(() => {vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); localStorage.clear();});
afterEach(async () => {
  for (const workbench of workbenches.splice(0)) await act(async () => workbench.dispose());
  for (const root of roots.splice(0)) await act(async () => root.unmount());
  document.body.replaceChildren(); vi.clearAllMocks(); vi.unstubAllGlobals();
});

function plugin(id: string, view: string, beforeRegister?: () => void | Promise<void>): LoadedRuntimePlugin {
  const manifest = decodePluginManifest({formatVersion: 1, id, version: "1.0.0", apiVersion: 1,
    entry: "assets/index.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME,
    dependencies: {required: [], optional: []}, provides: [], consumes: {required: [], optional: []}});
  return {manifest, entryUrl: `/fixtures/${id}/index.js`, styles: [], module: {
    async activate(context: PluginActivationContext) {
      await beforeRegister?.();
      context.registerView({id: view, title: view,
        ...(view === "library" ? {restore: {homeTabId: "library", readActiveTabId: () => "original-pdf"}} : {}),
        render: ({active}) => <span data-navigation-view={view} data-active={active}>{view}</span>});
    },
  }};
}
function completeProduct(beforeWriting?: () => void | Promise<void>) {
  return [plugin("literature_library", "library"), plugin("writing_projects", "writing", beforeWriting),
    plugin("note_manager", "notes")];
}
function result(plugins: readonly LoadedRuntimePlugin[]): RuntimePluginLoadResult {
  return {plugins, bindings: [], unavailableOptional: []};
}
async function mount() {
  const workbench = createRuntimePluginWorkbench({storage: localStorage}); workbenches.push(workbench);
  const node = document.createElement("div"); document.body.append(node);
  const root = createRoot(node); roots.push(root);
  await act(async () => root.render(<WorkbenchApp composition={assembleWorkbench([])} runtimePlugins={workbench}/>));
  const active = (view: string) => node.querySelector(`[data-navigation-view="${view}"]`)?.getAttribute("data-active") === "true";
  const choose = (view: string) => act(async () => node.querySelector<HTMLButtonElement>(`button[aria-label="${view}"]`)!.click());
  const close = async () => {
    await act(async () => workbench.dispose());
    await act(async () => root.unmount());
    roots.splice(roots.indexOf(root), 1);
  };
  return {workbench, node, active, choose, close};
}
const savedLocation = () => JSON.parse(localStorage.getItem("research-assistant:workspace.v1:location")!);

it("keeps the full product destination separate from the Core-only composition", async () => {
  const first = await mount();
  await act(async () => first.workbench.activate(result(completeProduct())));
  await first.choose("writing");
  expect(savedLocation()).toEqual({view: "writing"});
  await first.close();

  const core = await mount();
  await act(async () => core.workbench.activate(result([])));
  expect(core.workbench.isInitialLoadComplete()).toBe(true);
  await core.choose("设置");
  expect(savedLocation()).toEqual({view: "writing"});
  expect(JSON.parse(localStorage.getItem("research-assistant:workspace.v1:location:composition:core")!)).toEqual({view: "settings"});
  await core.close();

  const restored = await mount();
  await act(async () => restored.workbench.activate(result(completeProduct())));
  expect(restored.active("writing")).toBe(true);
});

it.each([["writing_projects", "writing"], ["note_manager", "notes"]] as const)("restores %s without a literature plugin", async (id, view) => {
  writeWorkspaceState(`location:composition:${id}`, {view});
  const app = await mount();
  await act(async () => app.workbench.activate(result([plugin(id, view)])));
  expect(app.active(view)).toBe(true);
});

it("waits for sequential activation and restores the original complete-product writing destination", async () => {
  writeWorkspaceState("location", {view: "writing"});
  let finishWriting!: () => void;
  const writingReady = new Promise<void>(resolve => {finishWriting = resolve;});
  const app = await mount();
  let loading!: Promise<void>;
  await act(async () => {loading = app.workbench.activate(result(completeProduct(() => writingReady)));});
  expect(app.workbench.getSnapshot().map(status => status.state)).toEqual(["ready", "activating", "discovered"]);
  expect(savedLocation()).toEqual({view: "writing"});
  expect(app.active("library")).toBe(false);
  await act(async () => {finishWriting(); await loading;});
  expect(app.active("writing")).toBe(true);
  expect(savedLocation()).toEqual({view: "writing"});
});

it("preserves explicit user navigation while the remaining runtime plugins activate", async () => {
  writeWorkspaceState("location", {view: "writing"});
  let finishWriting!: () => void;
  const writingReady = new Promise<void>(resolve => {finishWriting = resolve;});
  const app = await mount();
  let loading!: Promise<void>;
  await act(async () => {loading = app.workbench.activate(result(completeProduct(() => writingReady)));});
  await app.choose("设置");
  await act(async () => {finishWriting(); await loading;});
  expect(app.active("writing")).toBe(false);
  expect(savedLocation()).toEqual({view: "settings"});
});

it("settles failed activation without overwriting the unavailable full-product destination", async () => {
  writeWorkspaceState("location", {view: "writing"});
  const app = await mount();
  await act(async () => {
    await expect(app.workbench.activate(result(completeProduct(() => {throw new Error("fixture activation failure");}))))
      .rejects.toMatchObject({code: "plugin_activation_failed"});
  });
  expect(app.workbench.isInitialLoadComplete()).toBe(true);
  expect(app.active("library")).toBe(true);
  expect(savedLocation()).toEqual({view: "writing"});
});

it("finishes discovery failure before activation while leaving the full product preference intact", async () => {
  writeWorkspaceState("location", {view: "writing"});
  const app = await mount();
  await act(async () => app.workbench.completeInitialLoad());
  expect(app.workbench.isInitialLoadComplete()).toBe(true);
  expect(app.node.textContent).toContain("Core settings");
  expect(savedLocation()).toEqual({view: "writing"});
});
