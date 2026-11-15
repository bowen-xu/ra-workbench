// @vitest-environment happy-dom
import {act, createContext, useContext, useEffect, useLayoutEffect, useMemo, useState, type ReactNode} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {decodePluginManifest, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import type {PluginWritingRuntimeBinding, PluginWritingRuntimeProps, PluginWritingWorkspaceHostProps} from "@research-assistant/workbench-sdk/activation";
import {getReaderCloseCoordinator} from "../../../core/workspace/DocumentCloseCoordinator";
import {useWritingConversationSession} from "../../../core/features/agent/context/AgentConversationAdapters";
import {WorkbenchApp} from "../../WorkbenchApp";
import {assembleWorkbench, type WorkbenchComposition} from "../../workbenchComposition";
import {createRuntimePluginWorkbench} from "./runtimePluginWorkbench";

const probes = vi.hoisted(() => ({coreMounts: 0, coreUnmounts: 0}));
vi.mock("../../../core/runtime/appStorage", () => ({getAppStorage: () => localStorage}));
vi.mock("../../../core/runtime/appRuntime", async original => ({
  ...await original<typeof import("../../../core/runtime/appRuntime")>(), getAppRuntime: () => ({storage: localStorage}),
}));
vi.mock("../../../core/runtime/platformCore", async original => ({
  ...await original<typeof import("../../../core/runtime/platformCore")>(), isDesktopRuntime: () => false,
}));
vi.mock("../../../core/runtime/desktopZoom", () => ({installDesktopZoom: () => () => undefined}));
vi.mock("../../desktopLifecycle", () => ({installDesktopLifecycle: async () => () => undefined}));
vi.mock("../../../core/components/text-editor/useTextDocuments", () => ({useTextDocuments: () => ({
  activeId: undefined, surface: null, deactivate() {}, forgetOwner() {}, restoreOwner() {},
})}));
vi.mock("../../../core/features/notes/workspace/useNoteDocumentTabHost", () => ({useNoteDocumentTabHost: () => undefined}));
vi.mock("../../../core/public/noteDocuments", () => ({CoreNoteWorkspace: () => null,
  CoreNoteWorkspaceHost: ({children}: {children: ReactNode}) => children}));
vi.mock("../../../core/features/agent/AgentCanvasShell", () => ({AgentCanvasShell: ({children}: {children: ReactNode}) => <CanvasProbe>{children}</CanvasProbe>}));
vi.mock("../../notes/ReadingNoteFloatingPanel", () => ({ReadingNoteFloatingPanel: () => null}));
vi.mock("../../../core/public/workbench", async original => {
  const actual = await original<typeof import("../../../core/public/workbench")>();
  return {...actual, AppWindowToolbar: ({onTabHostChange}: {onTabHostChange: (host: HTMLDivElement | null) => void}) => {
    useLayoutEffect(() => {const host = document.createElement("div"); onTabHostChange(host); return () => onTabHostChange(null);}, [onTabHostChange]);
    return null;
  }};
});
vi.mock("../../../core/features/settings/SettingsWorkspace", () => ({SettingsWorkspace: () => <span>Core settings</span>}));
vi.mock("../../../core/components/AppToaster", () => ({AppToaster: () => null}));
vi.mock("../../../core/components/AppUpdateControl", () => ({AppUpdateControl: () => null}));
vi.mock("../../../core/components/PreviewResetControl", () => ({PreviewResetControl: () => null}));
vi.mock("../../../core/components/NotesPreviewControl", () => ({NotesPreviewControl: () => null}));
vi.mock("../../../core/components/NoteRecoveryPreviewControl", () => ({NoteRecoveryPreviewControl: () => null}));

const CanvasContext = createContext("outside"), FileContext = createContext<object | null>(null);
function CanvasProbe({children}: {children: ReactNode}) {
  useEffect(() => {probes.coreMounts++; return () => {probes.coreUnmounts++;};}, []);
  const Session = useWritingConversationSession();
  return <CanvasContext value="original-canvas">{children}{Session && <Session id="conversation"
    directory={{manuscriptId: "paper", title: "Paper", path: "paper"}} title="Conversation" archived={false}
    open onOpenChange={() => undefined} headerControls={null} onRenamed={() => undefined}/>}</CanvasContext>;
}
const roots: Root[] = [], unregister: (() => void)[] = [];
beforeEach(() => {vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); localStorage.clear(); probes.coreMounts = 0; probes.coreUnmounts = 0;});
afterEach(async () => {
  for (const root of roots.splice(0)) await act(async () => root.unmount());
  unregister.splice(0).forEach(dispose => dispose()); document.body.replaceChildren(); vi.clearAllMocks(); vi.unstubAllGlobals();
});
async function setup({ready = true, failRuntime = false, composition = assembleWorkbench([])}: {
  readonly ready?: boolean; readonly failRuntime?: boolean; readonly composition?: WorkbenchComposition;
} = {}) {
  const workbench = createRuntimePluginWorkbench({storage: localStorage});
  let runtimeFails = failRuntime;
  const seen = {owners: [] as object[], workspaces: [] as object[], sessions: [] as object[], canvas: [] as string[],
    props: [] as PluginWritingWorkspaceHostProps[], runtime: [] as PluginWritingRuntimeProps[],
    bindings: [] as PluginWritingRuntimeBinding[], releases: [] as (() => void)[], abandoned: [] as PluginWritingRuntimeProps[], unmounted: 0};
  const prepareClose = vi.fn(async () => true);
  function Runtime(props: PluginWritingRuntimeProps) {
    if (runtimeFails) {
      seen.abandoned.push(props);
      throw Object.assign(new Error("Synthetic initial owner failure"), {code: "WRITING_OWNER_UNAVAILABLE"});
    }
    const [owner] = useState(() => ({}));
    const binding = useMemo<PluginWritingRuntimeBinding>(() => ({Scope: ({children}) => <FileContext value={owner}>{children}</FileContext>, prepareClose}), [owner]);
    useEffect(() => {seen.owners.push(owner); return () => {seen.unmounted++;};}, [owner]);
    useEffect(() => {
      seen.runtime.push(props); seen.bindings.push(binding);
      if (!ready) return;
      const release = props.onReady(binding); seen.releases.push(release); return release;
    }, [props.onReady, binding]);
    return null;
  }
  function Session() {
    const owner = useContext(FileContext); if (!owner) throw new Error("Missing original file owner");
    seen.sessions.push(owner); seen.canvas.push(useContext(CanvasContext)); return <span>Bound writing session</span>;
  }
  const manifest = decodePluginManifest({formatVersion: 1, id: "writing_projects", version: "1.0.0", apiVersion: 1,
    entry: "assets/index.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME,
    dependencies: {required: [], optional: []}, provides: [], consumes: {required: [], optional: []}});
  const module = {activate(context: import("@research-assistant/workbench-sdk/activation").PluginActivationContext) {
    context.registerView({id: "writing", title: "Writing", writing: {Runtime, Session}, render(props) {
      const owner = useContext(FileContext); if (!owner) throw new Error("Workspace fallback must not create another owner");
      seen.workspaces.push(owner); seen.props.push(props.writingWorkspace!); return <textarea defaultValue="Retained writing draft"/>;
    }});
  }};
  const catalog = {plugins: [{manifest, module, entryUrl: "/synthetic/entry.js", styles: []}], bindings: [], unavailableOptional: []};
  const node = document.createElement("div"); document.body.append(node);
  const root = createRoot(node, {onCaughtError: vi.fn()}); roots.push(root);
  await act(async () => root.render(<WorkbenchApp composition={composition} runtimePlugins={workbench} initialLocation={{view: "settings"}}/>));
  const activate = () => act(async () => {await workbench.activate(catalog);});
  return {workbench, node, seen, prepareClose, activate, setRuntimeFailure: (value: boolean) => {runtimeFails = value;}};
}

it("mounts one Runtime after Core and shares its Scope at the original workspace and session positions", async () => {
  const f = await setup();
  expect(probes.coreMounts).toBe(1);
  await f.activate();
  expect(f.seen.owners).toHaveLength(1);
  expect(f.seen.workspaces.length).toBeGreaterThan(0);
  expect(f.seen.sessions.length).toBeGreaterThan(0);
  expect([...f.seen.workspaces, ...f.seen.sessions].every(owner => owner === f.seen.owners[0])).toBe(true);
  expect(f.seen.canvas.every(value => value === "original-canvas")).toBe(true);
  expect(probes.coreMounts).toBe(1); expect(probes.coreUnmounts).toBe(0);
  expect(f.seen.props[0]?.tabHost).toBeInstanceOf(HTMLDivElement);
  expect(f.seen.props[0]?.tabsActive).toBe(false);
  expect(f.seen.props[0]?.onOperationBusyChange).toBeTypeOf("function");
  expect(f.seen.props[0]?.onFocusModeChange).toBeTypeOf("function");
  expect(f.seen.props[0]?.onEditorVisibilityChange).toBeTypeOf("function");
});

it("keeps hidden writing Runtime and its draft after refused close and removes it only after preparation commits", async () => {
  const f = await setup(); await f.activate();
  const draft = f.node.querySelector("textarea")!, original = f.workbench.views.getSnapshot()[0];
  const corePrepare = vi.fn(async () => true);
  unregister.push(getReaderCloseCoordinator().register("retained-core-note", {isActive: () => false, prepare: corePrepare, commit() {}}));
  f.prepareClose.mockResolvedValueOnce(false);
  await act(async () => {await expect(f.workbench.deactivate("writing_projects")).rejects.toMatchObject({code: "plugin_deactivation_failed"});});
  expect(f.seen.unmounted).toBe(0); expect(f.node.querySelector("textarea")).toBe(draft);
  expect(f.workbench.views.getSnapshot()[0]).toBe(original);
  expect(corePrepare).not.toHaveBeenCalled();
  expect(f.seen.runtime[0]!.isDisabled()).toBe(true);
  await act(async () => {await f.workbench.deactivate("writing_projects");});
  expect(corePrepare).toHaveBeenCalledOnce();
  expect(f.seen.unmounted).toBe(1); expect(f.node.querySelector("textarea")).toBeNull();
  expect(probes.coreMounts).toBe(1); expect(probes.coreUnmounts).toBe(0);
});

it("blocks an unready Runtime and accepts only that retained registration's late binding", async () => {
  const f = await setup({ready: false}); await f.activate();
  expect(f.seen.workspaces).toEqual([]); expect(f.seen.sessions).toEqual([]);
  await act(async () => {await expect(f.workbench.deactivate("writing_projects")).rejects.toMatchObject({code: "plugin_deactivation_failed"});});
  expect(f.prepareClose).not.toHaveBeenCalled(); expect(f.seen.unmounted).toBe(0);
  const oldRuntime = f.seen.runtime[0]!, oldBinding = f.seen.bindings[0]!;
  let oldRelease: () => void = () => undefined;
  await act(async () => {oldRelease = oldRuntime.onReady(oldBinding);});
  expect(f.seen.sessions.length).toBeGreaterThan(0);
  expect(oldRuntime.isDisabled()).toBe(true);
  await act(async () => {await f.workbench.deactivate("writing_projects");});
  await f.activate();
  const newRuntime = f.seen.runtime.at(-1)!, newBinding = f.seen.bindings.at(-1)!;
  await act(async () => {newRuntime.onReady(newBinding);});
  const current = f.workbench.views.getSnapshot()[0];
  await act(async () => {oldRelease(); oldRuntime.onReady(oldBinding);});
  expect(f.workbench.views.getSnapshot()[0]).toBe(current);
  expect(f.seen.sessions.at(-1)).toBe(f.seen.owners.at(-1));
  expect(f.seen.owners).toHaveLength(2); expect(probes.coreMounts).toBe(1);
});

it("keeps Core mounted when the writing Runtime fails its initial mount", async () => {
  const f = await setup({failRuntime: true}); await f.activate();
  expect(f.node.textContent).toContain("WRITING_OWNER_UNAVAILABLE");
  expect(f.node.textContent).toContain("Core settings");
  expect(f.seen.workspaces).toEqual([]); expect(f.seen.sessions).toEqual([]);
  expect(probes.coreMounts).toBe(1); expect(probes.coreUnmounts).toBe(0);
});

it("rejects extra, hidden and symbol binding fields while retaining the unready owner", async () => {
  const f = await setup({ready: false}); await f.activate();
  const binding = f.seen.bindings[0]!, runtime = f.seen.runtime[0]!;
  const invalid = [{...binding, Store: {synthetic: true}}, Object.defineProperty({...binding}, "extra", {value: true}),
    {...binding, [Symbol("extra")]: true}];
  for (const value of invalid) expect(() => runtime.onReady(value)).toThrow("plugin_writing_binding_invalid");
  expect(f.seen.workspaces).toEqual([]); expect(f.seen.sessions).toEqual([]);
  expect(f.prepareClose).not.toHaveBeenCalled(); expect(f.seen.unmounted).toBe(0);
  await act(async () => {runtime.onReady(binding);});
  expect(f.seen.sessions.at(-1)).toBe(f.seen.owners[0]);
});

it("rejects a failed initial mount's late publication after retrying the same registration", async () => {
  const f = await setup({failRuntime: true}); await f.activate();
  const abandoned = f.seen.abandoned[0]!;
  f.setRuntimeFailure(false);
  await act(async () => f.node.querySelector<HTMLButtonElement>('[role="alert"] button')!.click());
  expect(f.seen.owners).toHaveLength(1);
  const owner = f.seen.owners[0], stale = {};
  await act(async () => {abandoned.onReady({Scope: ({children}) => <FileContext value={stale}>{children}</FileContext>, prepareClose: async () => true});});
  expect(f.seen.workspaces.at(-1)).toBe(owner);
  expect(f.seen.sessions.at(-1)).toBe(owner);
  expect(f.node.querySelector("textarea")).not.toBeNull();
  expect(probes.coreMounts).toBe(1); expect(probes.coreUnmounts).toBe(0);
});
