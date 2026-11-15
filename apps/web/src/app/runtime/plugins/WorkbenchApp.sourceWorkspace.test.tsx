// @vitest-environment happy-dom
import {readFileSync} from "node:fs";
import {act, useLayoutEffect, type ReactNode} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {decodePluginManifest, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import type {PluginWorkspaceHostProps, RuntimePluginModule} from "@research-assistant/workbench-sdk/activation";
import type {ReadingNoteSource} from "@research-assistant/core/notes";
import {getReaderCloseCoordinator} from "../../../core/workspace/DocumentCloseCoordinator";
import {useReadingNoteResearchActions} from "../../../core/features/notes/reading-notes/ReadingNotePresentation";
import {WorkbenchApp} from "../../WorkbenchApp";
import {assembleWorkbench} from "../../workbenchComposition";
import {createRuntimePluginWorkbench} from "./runtimePluginWorkbench";

const effects = vi.hoisted(() => ({floatingActions: undefined as unknown, noteOwnerRenders: 0,
  deactivate: vi.fn(), forgetOwner: vi.fn(), restoreOwner: vi.fn(() => undefined), settingsCapture: vi.fn()}));
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
  activeId: undefined, surface: null, deactivate: effects.deactivate, forgetOwner: effects.forgetOwner, restoreOwner: effects.restoreOwner,
})}));
vi.mock("../../../core/features/notes/workspace/useNoteDocumentTabHost", () => ({useNoteDocumentTabHost: () => undefined}));
vi.mock("../../../core/public/noteDocuments", () => ({CoreNoteWorkspace: () => null,
  CoreNoteWorkspaceHost: ({children}: {children: ReactNode}) => {effects.noteOwnerRenders++; return children;}}));
vi.mock("../../../core/features/agent/AgentCanvasShell", () => ({AgentCanvasShell: ({children}: {children: ReactNode}) => children}));
vi.mock("../../notes/ReadingNoteFloatingPanel", () => ({ReadingNoteFloatingPanel: () => {
  effects.floatingActions = useReadingNoteResearchActions(); return null;
}}));
vi.mock("../../../core/public/workbench", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../core/public/workbench")>();
  return {...actual, AppWindowToolbar: ({onTabHostChange}: {onTabHostChange: (host: HTMLDivElement | null) => void}) => {
    useLayoutEffect(() => {const host = document.createElement("div"); onTabHostChange(host); return () => onTabHostChange(null);}, [onTabHostChange]);
    return null;
  }};
});
vi.mock("../../../core/features/settings/SettingsWorkspace", () => ({SettingsWorkspace: ({browserCaptureAvailable}: {browserCaptureAvailable: boolean}) => {
  effects.settingsCapture(browserCaptureAvailable); return null;
}}));
vi.mock("../../../core/components/AppToaster", () => ({AppToaster: () => null}));
vi.mock("../../../core/components/AppUpdateControl", () => ({AppUpdateControl: () => null}));
vi.mock("../../../core/components/PreviewResetControl", () => ({PreviewResetControl: () => null}));
vi.mock("../../../core/components/NotesPreviewControl", () => ({NotesPreviewControl: () => null}));
vi.mock("../../../core/components/NoteRecoveryPreviewControl", () => ({NoteRecoveryPreviewControl: () => null}));

const roots: Root[] = [], unregister: (() => void)[] = [];
beforeEach(() => {vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); localStorage.clear(); effects.floatingActions = undefined; effects.noteOwnerRenders = 0;});
afterEach(async () => {
  for (const root of roots.splice(0)) await act(async () => root.unmount());
  unregister.splice(0).forEach(dispose => dispose()); document.body.replaceChildren(); vi.clearAllMocks(); vi.unstubAllGlobals();
});

async function setup(priority?: number) {
  const workbench = createRuntimePluginWorkbench({storage: localStorage});
  const manifest = decodePluginManifest({formatVersion: 1, id: "literature_library", version: "1.0.0", apiVersion: 1,
    entry: "assets/index.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME,
    dependencies: {required: [], optional: []}, provides: [], consumes: {required: [], optional: []}});
  let host: PluginWorkspaceHostProps | undefined;
  const ReadingActions = () => null;
  const readActiveTabId = vi.fn(() => "pdf:restored");
  const activate: RuntimePluginModule["activate"] = context => {
    context.registerView({id: "library", title: "Literature", ...(priority === undefined ? {} : {priority}), restore: {homeTabId: "library", readActiveTabId},
      reading: {ReadingActions}, render: props => {host = props.workspace; return null;}});
  };
  const node = document.createElement("div"); document.body.append(node);
  const root = createRoot(node); roots.push(root);
  await act(async () => root.render(<WorkbenchApp composition={assembleWorkbench([])} runtimePlugins={workbench}/>));
  const start = () => workbench.activate({plugins: [{manifest, module: {activate}, entryUrl: "/fixture/index.js", styles: []}], bindings: [], unavailableOptional: []});
  await act(start);
  const read = () => {if (!host) throw new Error("Actual WorkbenchApp did not supply its workspace host"); return host;};
  let mount = 0;
  const remount = () => act(async () => root.render(<WorkbenchApp key={++mount} composition={assembleWorkbench([])} runtimePlugins={workbench}/>));
  return {workbench, node, read, start, remount, readActiveTabId};
}
const source: ReadingNoteSource = {paperId: "synthetic-paper", paperTitle: "Synthetic paper", attachmentId: "attachment",
  contentSha256: "a".repeat(64), pageIndex: 0, rects: [], quoteExact: "", quotePrefix: "", quoteSuffix: "", state: "page_only"};
const inspect = (detail: {source: ReadingNoteSource; resolve: () => void; reject: (reason: Error) => void}) =>
  window.dispatchEvent(new CustomEvent("research-assistant:inspect-reading-source", {detail}));

it("shows Core capture status for the ready runtime library and follows its withdrawal and reactivation", async () => {
  const {workbench, node, start} = await setup();
  await act(async () => node.querySelector<HTMLButtonElement>('button[aria-label="设置"]')!.click());
  expect(effects.settingsCapture).toHaveBeenLastCalledWith(true);
  await act(async () => workbench.deactivate("literature_library"));
  expect(effects.settingsCapture).toHaveBeenLastCalledWith(false);
  await act(start);
  expect(effects.settingsCapture).toHaveBeenLastCalledWith(true);
});

it("renders navigation by configured priority while registration order remains intact", async () => {
  const {workbench, node} = await setup(300);
  await act(async () => {
    workbench.views.register("writing_projects", {id: "writing", title: "写作", priority: 100, render: () => null});
    workbench.views.register("note_manager", {id: "notes", title: "笔记", priority: 200, render: () => null});
  });
  expect(workbench.views.getSnapshot().map(view => view.id)).toEqual(["library", "writing", "notes"]);
  expect([...node.querySelectorAll("button.nav-item")].map(button => button.getAttribute("aria-label")))
    .toEqual(["Literature", "笔记", "写作", "设置"]);
});

it("keeps the dynamic note view in a vertical flex host and hides inactive registered views", async () => {
  const styles = document.createElement("style");
  styles.textContent = readFileSync("apps/web/src/core/styles.css", "utf8");
  document.head.append(styles);
  try {
    const {workbench, node} = await setup();
    await act(async () => {
      workbench.views.register("note_manager", {id: "notes", title: "笔记",
        render: () => <span data-note-view="fixture"/>});
    });
    const content = node.querySelector('[data-note-view="fixture"]')!;
    const host = content.closest<HTMLDivElement>(".registered-workspace-host")!;
    const libraryHost = [...node.querySelectorAll<HTMLDivElement>(".registered-workspace-host")]
      .find(registered => registered !== host)!;
    expect(host.parentElement).toBe(node.querySelector("main"));
    expect(host.hidden).toBe(true);
    expect(getComputedStyle(host).display).toBe("none");
    await act(async () => node.querySelector<HTMLButtonElement>('button[aria-label="笔记"]')!.click());
    expect(host.hidden).toBe(false);
    expect(getComputedStyle(host).display).toBe("flex");
    expect(getComputedStyle(host).flexDirection).toBe("column");
    expect(getComputedStyle(host).minHeight).toBe("0");
    expect(libraryHost.hidden).toBe(true);
    expect(getComputedStyle(libraryHost).display).toBe("none");
    await act(async () => node.querySelector<HTMLButtonElement>('button[aria-label="设置"]')!.click());
    expect(getComputedStyle(host).display).toBe("none");
    await act(async () => node.querySelector<HTMLButtonElement>('button[aria-label="笔记"]')!.click());
    expect(node.querySelector('[data-note-view="fixture"]')).toBe(content);
    expect(getComputedStyle(host).display).toBe("flex");
  } finally {styles.remove();}
});

it("uses stable navigation IDs for equal default priorities independently of labels and re-registration", async () => {
  const {workbench, node} = await setup();
  let alpha!: ReturnType<typeof workbench.views.register>;
  await act(async () => {
    workbench.views.register("fixture", {id: "beta", title: "A label", render: () => null});
    alpha = workbench.views.register("fixture", {id: "alpha", title: "Z label", priority: 0, render: () => null});
  });
  const labels = () => [...node.querySelectorAll("button.nav-item")].map(button => button.getAttribute("aria-label"));
  expect(labels()).toEqual(["Z label", "A label", "Literature", "设置"]);
  await act(async () => {await alpha.dispose();});
  await act(async () => {
    alpha = workbench.views.register("fixture", {id: "alpha", title: "Z label", render: () => null});
  });
  expect(labels()).toEqual(["Z label", "A label", "Literature", "设置"]);
  expect(workbench.views.getSnapshot().map(view => view.id)).toEqual(["library", "beta", "alpha"]);
});

it("passes the actual App tab, source and item callbacks through the registered view and keeps Core note presentation", async () => {
  const {workbench, read} = await setup();
  expect(read().navigationTabId).toBe("pdf:restored"); expect(read().tabHost).toBeInstanceOf(HTMLDivElement);
  expect(read().project).toBeUndefined();
  expect(effects.floatingActions).toBe(workbench.views.getSnapshot()[0]?.reading?.ReadingActions);
  expect(effects.noteOwnerRenders).toBeGreaterThan(0);
  await act(async () => read().onNavigateTab("pdf:next"));
  expect(read().navigationTabId).toBe("pdf:next");
  await act(async () => read().onSessionsChanged([], "library"));
  expect(read().navigationTabId).toBe("library");
  await act(async () => window.dispatchEvent(new CustomEvent("research-assistant:open-research-target", {
    detail: {kind: "paper", id: "synthetic-paper", labelSnapshot: "Synthetic paper"},
  })));
  expect(read().openItemId).toBe("synthetic-paper");
  await act(async () => read().onItemOpened()); expect(read().openItemId).toBeUndefined();
  const resolve = vi.fn(), reject = vi.fn();
  await act(async () => inspect({source, resolve, reject}));
  expect(read().openSource).toBe(source);
  await act(async () => read().onSourceOpened(true));
  expect(resolve).toHaveBeenCalledOnce(); expect(reject).not.toHaveBeenCalled(); expect(read().openSource).toBeUndefined();
});

it("awaits the original Core reader coordinator even for a hidden library and preserves its save owner after plugin withdrawal", async () => {
  const {workbench, node} = await setup(), coordinator = getReaderCloseCoordinator();
  const prepare = vi.fn(async () => true), commit = vi.fn();
  unregister.push(coordinator.register("synthetic-core-reading-note", {isActive: () => false, prepare, commit}));
  await act(async () => node.querySelector<HTMLButtonElement>('button[aria-label="设置"]')!.click());
  prepare.mockClear();
  let accept!: (saved: boolean) => void;
  prepare.mockImplementationOnce(() => new Promise(resolve => {accept = resolve;}));
  let closed!: Promise<void>;
  await act(async () => {closed = workbench.deactivate("literature_library");});
  expect(prepare).toHaveBeenCalledOnce(); expect(workbench.views.getSnapshot()).toHaveLength(1);
  expect(effects.floatingActions).toBe(workbench.views.getSnapshot()[0]?.reading?.ReadingActions);
  await act(async () => {accept(true); await closed;});
  expect(workbench.views.getSnapshot()).toEqual([]); expect(effects.floatingActions).toBeUndefined();
  expect(await coordinator.prepareAll()).toBe(true); expect(prepare).toHaveBeenCalledTimes(2);
  expect(commit).not.toHaveBeenCalled(); expect(effects.noteOwnerRenders).toBeGreaterThan(0);
});

it("does not let an older reader acknowledgment settle a later AI source inspection", async () => {
  const {read} = await setup(), first = {source, resolve: vi.fn(), reject: vi.fn()};
  await act(async () => inspect(first));
  const oldAck = read().onSourceOpened;
  const second = {source: {...source, pageIndex: 1}, resolve: vi.fn(), reject: vi.fn()};
  await act(async () => inspect(second));
  expect(first.reject).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({message: "已由新的来源定位请求取代"}));
  await act(async () => oldAck(true));
  expect(second.resolve).not.toHaveBeenCalled(); expect(second.reject).not.toHaveBeenCalled();
  expect(read().openSource).toBe(second.source);
  await act(async () => read().onSourceOpened(true));
  expect(second.resolve).toHaveBeenCalledOnce(); expect(read().openSource).toBeUndefined();
});

it("rejects an outstanding AI source inspection when its actual owner stops", async () => {
  const {workbench, read, start} = await setup(), request = {source, resolve: vi.fn(), reject: vi.fn()};
  await act(async () => inspect(request));
  const oldAck = read().onSourceOpened;
  await act(async () => workbench.deactivate("literature_library"));
  expect(request.reject).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({message: expect.stringContaining("plugin_unavailable")}));
  await act(async () => oldAck(true));
  expect(request.resolve).not.toHaveBeenCalled(); expect(request.reject).toHaveBeenCalledOnce();
  const unavailable = {source, resolve: vi.fn(), reject: vi.fn()};
  await act(async () => inspect(unavailable));
  expect(unavailable.reject).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({message: expect.stringContaining("plugin_unavailable")}));
  await act(start);
  expect(read().openSource).toBeUndefined();
});

it.each(["deactivating", "failed"] as const)("mounts a fresh Core workbench over the same %s retained library without invoking its closed restore context", async state => {
  const {workbench, node, remount, readActiveTabId} = await setup(), coordinator = getReaderCloseCoordinator();
  let finish!: (saved: boolean) => void;
  const pending = new Promise<boolean>(resolve => {finish = resolve;});
  unregister.push(coordinator.register("synthetic-retained-core-note", {isActive: () => false,
    prepare: () => pending, commit: vi.fn()}));
  const registered = workbench.views.getSnapshot()[0];
  const locationKey = "research-assistant:workspace.v1:location:composition:core";
  const saved = localStorage.getItem(locationKey);
  let stopping!: Promise<void>;
  await act(async () => {stopping = workbench.deactivate("literature_library");});
  const rejected = expect(stopping).rejects.toMatchObject({code: "plugin_deactivation_failed"});
  if (state === "failed") await act(async () => {finish(false); await rejected;});
  expect(workbench.getSnapshot()[0]?.state).toBe(state);
  readActiveTabId.mockClear();
  try {
    await remount();
    expect(readActiveTabId).not.toHaveBeenCalled();
    expect(workbench.views.getSnapshot()[0]).toBe(registered);
    expect(node.querySelector('button[aria-label="设置"]')).not.toBeNull();
    expect(node.textContent).toContain(state === "failed" ? "此管理视图未能安全停用" : "正在停用管理视图");
    expect(effects.noteOwnerRenders).toBeGreaterThan(0);
    expect(localStorage.getItem(locationKey)).toBe(saved);
  } finally {
    if (state === "deactivating") await act(async () => {finish(false); await rejected;});
  }
});
