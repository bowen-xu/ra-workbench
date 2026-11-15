// @vitest-environment happy-dom
import {act, type ReactNode} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {decodePluginManifest, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import type {
  PluginViewContribution, PluginWritingRuntimeBinding, PluginWritingRuntimeProps, RuntimePluginModule,
} from "@research-assistant/workbench-sdk/activation";
import type {AgentDomainSessionProps} from "@research-assistant/workbench-sdk/agent-presentation";
import type {LoadedRuntimePlugin, RuntimePluginLoadResult} from "../../../core/runtime/plugins/loader";
import {usePluginConfiguration} from "../../../core/configuration/ConfigurationProvider";
import {coreAgentExtensionFacets, useAgentExtensions} from "../../../core/extensions/AgentExtensionProvider";
import {createRuntimePluginWorkbench} from "./runtimePluginWorkbench";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
const roots: Root[] = [];
beforeEach(() => {localStorage.clear();});
afterEach(async () => {
  for (const root of roots.splice(0)) await act(async () => root.unmount());
  document.body.replaceChildren();
});

function contribution(): PluginViewContribution {
  return {id: "writing", title: "Writing", render: () => "Workspace",
    writing: {Runtime: () => null, Session: () => null}};
}
function plugin(id: string, activate: RuntimePluginModule["activate"]): LoadedRuntimePlugin {
  return {manifest: decodePluginManifest({formatVersion: 1, id, version: "1.0.0", apiVersion: 1,
    entry: "assets/entry.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME,
    dependencies: {required: [], optional: []}, provides: [], consumes: {required: [], optional: []}}),
  module: {activate}, entryUrl: "/plugins/" + "a".repeat(64) + "/assets/entry.js", styles: []};
}
function catalog(loaded: LoadedRuntimePlugin): RuntimePluginLoadResult {
  return {plugins: [loaded], bindings: ["core.agent.skills", "core.agent.tools", "core.agent.mcp"]
    .map(capabilityId => ({pluginId: loaded.manifest.id, capabilityId, provider: "core", version: 1})), unavailableOptional: []};
}
function setup() {
  const workbench = createRuntimePluginWorkbench({storage: localStorage});
  const prepareClose = vi.fn(async () => true);
  workbench.attachNavigation({open: async () => undefined, prepareClose});
  return {workbench, prepareClose};
}
async function render(content: ReactNode) {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); roots.push(root);
  await act(async () => root.render(content));
  return {host, root};
}

it.each([
  {name: "null", value: null},
  {name: "array", value: []},
  {name: "empty", value: {}},
  {name: "missing Runtime", value: {Session: (): null => null}},
  {name: "missing Session", value: {Runtime: (): null => null}},
  {name: "non-function Runtime", value: {Runtime: "runtime", Session: () => null}},
  {name: "non-function Session", value: {Runtime: () => null, Session: {}}},
  {name: "unknown field", value: {Runtime: (): null => null, Session: (): null => null, extra: true}},
  {name: "hidden field", value: Object.defineProperty({Runtime: () => null, Session: () => null}, "extra", {value: true})},
  {name: "symbol field", value: {Runtime: () => null, Session: () => null, [Symbol("extra")]: true}},
])("rejects malformed writing roles before publishing: $name", async ({value}) => {
  const {workbench, prepareClose} = setup();
  const loaded = plugin("writing_projects", context => {
    context.registerView({...contribution(), writing: value} as unknown as PluginViewContribution);
  });
  await expect(workbench.activate(catalog(loaded))).rejects.toMatchObject({code: "plugin_activation_failed",
    cause: expect.objectContaining({code: "plugin_view_invalid"})});
  expect(workbench.views.getSnapshot()).toEqual([]);
  expect(prepareClose).not.toHaveBeenCalled();
});

it.each([
  {owner: "note_manager", view: "writing"},
  {owner: "writing_projects", view: "other"},
])("rejects writing roles for $owner/$view", async ({owner, view}) => {
  const {workbench} = setup();
  const loaded = plugin(owner, context => {context.registerView({...contribution(), id: view});});
  await expect(workbench.activate(catalog(loaded))).rejects.toMatchObject({
    cause: expect.objectContaining({code: "plugin_view_invalid"})});
  expect(workbench.views.getSnapshot()).toEqual([]);
});

it("scopes all three roles to the same configuration and facets and retains the Runtime callbacks through stopping", async () => {
  const {workbench, prepareClose} = setup();
  const observed: PluginWritingRuntimeProps[] = [];
  const seen: {role: string; owner: string; configuration: unknown; value: unknown}[] = [];
  const configuration = {owner: "writing_projects", settings: [{key: "visible", defaultValue: true,
    schema: {type: "boolean" as const}, scopes: ["application" as const], effect: "immediate" as const}]};
  function useRole(role: string) {
    const facets = useAgentExtensions(), settings = usePluginConfiguration();
    expect(facets.skills).toBe(coreAgentExtensionFacets.skills);
    expect(facets.mcp).toBe(coreAgentExtensionFacets.mcp);
    seen.push({role, owner: facets.owner, configuration: settings, value: settings.get("visible")});
    return role;
  }
  function Runtime(props: PluginWritingRuntimeProps) {observed.push(props); return <output>{useRole("Runtime")}</output>;}
  function Session() {return <output>{useRole("Session")}</output>;}
  const selected = plugin("writing_projects", context => {context.registerView({...contribution(),
    render: props => <output>{useRole("Workspace")}{String(props.writingWorkspace?.openManuscriptId)}</output>,
    writing: {Runtime, Session}});});
  const loaded = {...selected, module: {...selected.module, configuration}};
  await workbench.activate(catalog(loaded));
  const registered = workbench.views.getSnapshot()[0]!;
  const roles = registered.writing!;
  const onReady = vi.fn((_binding: PluginWritingRuntimeBinding) => vi.fn());
  const callerDisabled = vi.fn(() => false), callerSubscribe = vi.fn(() => () => undefined);
  const session: AgentDomainSessionProps = {id: "conversation", directory: {manuscriptId: "paper", title: "Paper", path: "paper"},
    title: "Conversation", archived: false, open: true, onOpenChange: () => undefined, headerControls: null, onRenamed: () => undefined};
  await render(<><roles.Runtime onReady={onReady} isDisabled={callerDisabled} subscribe={callerSubscribe}/>
    <roles.Session {...session}/>{registered.render({active: true, writingWorkspace: {openManuscriptId: "paper", onManuscriptOpened: () => undefined}})}</>);
  expect(seen.map(item => item.role)).toEqual(["Runtime", "Session", "Workspace"]);
  expect(seen.every(item => item.owner === "writing_projects" && item.value === true)).toBe(true);
  expect(seen.every(item => item.configuration === seen[0]!.configuration)).toBe(true);
  const runtime = observed[0]!;
  expect(runtime.onReady).toBe(onReady);
  expect(runtime.isDisabled).not.toBe(callerDisabled);
  expect(runtime.subscribe).not.toBe(callerSubscribe);
  expect(runtime.isDisabled()).toBe(false);
  const changed = vi.fn(); const unsubscribe = runtime.subscribe(changed);
  let finishClose: (value: boolean) => void = () => {throw new Error("Close preparation was not initialized");};
  prepareClose.mockImplementationOnce(() => new Promise<boolean>(resolve => {finishClose = resolve;}));
  const stopping = workbench.deactivate("writing_projects");
  const rejected = expect(stopping).rejects.toMatchObject({code: "plugin_deactivation_failed"});
  await vi.waitFor(() => expect(workbench.getSnapshot()[0]?.state).toBe("deactivating"));
  const binding: PluginWritingRuntimeBinding = {Scope: ({children}) => children, prepareClose: async () => true};
  const release = runtime.onReady(binding);
  expect(onReady).toHaveBeenCalledWith(binding);
  expect(runtime.isDisabled()).toBe(true);
  finishClose(false);
  await rejected;
  expect(runtime.isDisabled()).toBe(true);
  expect(changed).toHaveBeenCalled();
  expect(workbench.views.getSnapshot()[0]).toBe(registered);
  expect(registered.writing).toBe(roles);
  expect(runtime.onReady(binding)).toBeDefined();
  release();
  unsubscribe();
  await workbench.deactivate("writing_projects");
  await workbench.activate(catalog(loaded));
  const replacement = workbench.views.getSnapshot()[0]!;
  expect(replacement).not.toBe(registered);
  expect(replacement.writing?.Runtime).not.toBe(roles.Runtime);
  expect(replacement.writing?.Session).not.toBe(roles.Session);
  expect(runtime.isDisabled()).toBe(true);
});
