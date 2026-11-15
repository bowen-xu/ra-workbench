// @vitest-environment happy-dom
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {decodePluginManifest, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import type {PluginActivationContext, PluginViewContribution, PluginViewRegistration, RuntimePluginModule} from "@research-assistant/workbench-sdk/activation";
import type {PluginConfigurationDeclaration} from "@research-assistant/workbench-sdk/configuration";
import type {PluginConfiguration} from "../../../core/contracts/configuration";
import {usePluginConfiguration} from "../../../core/configuration/ConfigurationProvider";
import {coreAgentExtensionFacets, useAgentExtensions} from "../../../core/extensions/AgentExtensionProvider";
import type {LoadedRuntimePlugin, RuntimePluginLoadResult} from "../../../core/runtime/plugins/loader";
import type {RegisteredPluginView} from "../../../core/runtime/plugins/viewRegistry";
import {noteConfiguration} from "@research-assistant/note-manager-plugin/configuration";
import {literatureConfiguration} from "@research-assistant/literature-library-plugin/configuration";
import {createRuntimePluginWorkbench, RuntimePluginWorkbenchError} from "./runtimePluginWorkbench";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
const roots: Root[] = [];
beforeEach(() => { localStorage.clear(); });
afterEach(async () => {
  for (const root of roots.splice(0)) await act(async () => root.unmount());
  document.body.replaceChildren();
});

function view(id = "notes"): PluginViewContribution {
  return {id, title: id === "notes" ? "笔记" : "Fixture", render: () => id};
}
function plugin(id: string, activate: RuntimePluginModule["activate"], index = 1): LoadedRuntimePlugin {
  const manifest = decodePluginManifest({formatVersion: 1, id, version: "1.0.0", apiVersion: 1,
    entry: "assets/entry.js", styles: [], uiRuntime: WORKBENCH_UI_RUNTIME,
    dependencies: {required: [], optional: []}, provides: [], consumes: {required: [], optional: []}});
  return {manifest, module: {activate}, entryUrl: `/plugins/${index.toString(16).padStart(64, "0")}/${manifest.entry}`, styles: []};
}
function catalog(...plugins: readonly LoadedRuntimePlugin[]): RuntimePluginLoadResult {
  return {plugins, bindings: [], unavailableOptional: []};
}
function deferred<T>() {
  let resolve: (value: T) => void = () => { throw new Error("Promise executor did not initialize"); };
  let reject: (reason: unknown) => void = () => { throw new Error("Promise executor did not initialize"); };
  const promise = new Promise<T>((accept, refuse) => { resolve = accept; reject = refuse; });
  return {promise, resolve, reject};
}
function setup() {
  const workbench = createRuntimePluginWorkbench({storage: localStorage});
  const open = vi.fn(async (_view: RegisteredPluginView): Promise<void> => undefined);
  const prepareClose = vi.fn(async (_view: RegisteredPluginView): Promise<boolean> => true);
  workbench.attachNavigation({open, prepareClose});
  return {workbench, open, prepareClose};
}
async function render(view: RegisteredPluginView) {
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container); roots.push(root);
  await act(async () => root.render(view.render({active: true})));
  return container;
}
function firstView(views: readonly RegisteredPluginView[]): RegisteredPluginView {
  const first = views[0]; if (!first) throw new Error("Fixture view missing"); return first;
}

it("binds frozen identity and registers both package contributions through one registry", async () => {
  const {workbench, open} = setup();
  let registration: PluginViewRegistration | undefined;
  const activate = vi.fn((context: PluginActivationContext) => {
    expect(context.pluginId).toBe("note_manager");
    expect(context.apiVersion).toBe(1);
    expect(Object.isFrozen(context)).toBe(true);
    expect(Object.keys(context).sort()).toEqual(["apiVersion", "onDispose", "pluginId", "registerView"]);
    registration = context.registerView(view());
    context.registerView(view("notes-fixture"));
  });
  const loaded = catalog(plugin("note_manager", activate));
  await workbench.activate(loaded);
  const snapshot = workbench.views.getSnapshot();
  await workbench.activate(loaded);
  expect(activate).toHaveBeenCalledTimes(1);
  expect(workbench.views.getSnapshot()).toBe(snapshot);
  expect(workbench.views.getSnapshot().map(({key}) => key)).toEqual([
    "plugin:note_manager/notes", "plugin:note_manager/notes-fixture",
  ]);
  await registration?.open();
  expect(open).toHaveBeenCalledExactlyOnceWith(workbench.views.getSnapshot()[0], expect.any(AbortSignal));
});

it("coalesces concurrent activations without interleaving dependency order", async () => {
  const {workbench} = setup(), pending = deferred<void>(), order: string[] = [];
  const first = plugin("one", vi.fn(async () => { order.push("one"); await pending.promise; }));
  const second = plugin("two", vi.fn(() => { order.push("two"); }), 2);
  const loaded = catalog(first, second);
  const initial = workbench.activate(loaded), concurrent = workbench.activate(loaded);
  await Promise.resolve();
  expect(order).toEqual(["one"]);
  pending.resolve();
  await Promise.all([initial, concurrent]);
  expect(order).toEqual(["one", "two"]);
  expect(first.module.activate).toHaveBeenCalledTimes(1);
  expect(second.module.activate).toHaveBeenCalledTimes(1);
});

it("checks a queued catalog after the current catalog settles", async () => {
  const {workbench} = setup(), pending = deferred<void>();
  const first = plugin("one", () => pending.promise);
  const second = plugin("two", vi.fn(), 2), third = plugin("three", vi.fn(), 3);
  const initial = workbench.activate(catalog(first, second));
  const changed = workbench.activate(catalog(first, third));
  const rejected = expect(changed).rejects.toMatchObject({code: "plugin_restart_required", pluginId: "two"});
  pending.resolve();
  await initial; await rejected;
  expect(third.module.activate).not.toHaveBeenCalled();
});

it.each(["remove", "entry", "version"] as const)("requires restart before an active package %s change", async change => {
  const {workbench} = setup();
  const original = plugin("note_manager", vi.fn(context => { context.registerView(view()); }));
  await workbench.activate(catalog(original));
  const snapshot = workbench.views.getSnapshot();
  const replacement = {...original, entryUrl: change === "entry" ? original.entryUrl + ".next" : original.entryUrl,
    manifest: {...original.manifest, version: change === "version" ? "2.0.0" : original.manifest.version}, module: {activate: vi.fn()}};
  await expect(workbench.activate(change === "remove" ? catalog() : catalog(replacement))).rejects.toMatchObject({
    code: "plugin_restart_required", pluginId: "note_manager", entryUrl: original.entryUrl,
  });
  expect(workbench.views.getSnapshot()).toBe(snapshot);
  expect(replacement.module.activate).not.toHaveBeenCalled();
});

it("rejects public route conflicts and settings without removing another owner", async () => {
  const {workbench} = setup();
  const notes = plugin("note_manager", context => { context.registerView(view()); });
  await workbench.activate(catalog(notes));
  const snapshot = workbench.views.getSnapshot();
  const collision = plugin("other", context => { context.registerView(view()); }, 2);
  await expect(workbench.activate(catalog(notes, collision))).rejects.toMatchObject({
    code: "plugin_activation_failed", cause: expect.objectContaining({code: "plugin_view_route_conflict", viewId: "notes"}),
  });
  const reserved = plugin("other", context => { context.registerView(view("settings")); }, 2);
  await expect(workbench.activate(catalog(notes, reserved))).rejects.toMatchObject({
    cause: expect.objectContaining({code: "plugin_view_reserved", viewId: "settings"}),
  });
  expect(workbench.views.getSnapshot()).toBe(snapshot);
});

it("rejects a duplicate owner-local view and rolls back the first registration", async () => {
  const {workbench, prepareClose} = setup();
  const duplicate = plugin("note_manager", context => { context.registerView(view()); context.registerView(view()); });
  await expect(workbench.activate(catalog(duplicate))).rejects.toMatchObject({
    cause: expect.objectContaining({code: "plugin_view_duplicate"}),
  });
  expect(workbench.views.getSnapshot()).toEqual([]);
  expect(prepareClose).toHaveBeenCalledTimes(1);
});

it.each([
  {name: "null", input: null},
  {name: "array", input: []},
  {name: "missing fields", input: {}},
  {name: "route expression", input: {...view(), id: "../notes"}},
  {name: "empty ID", input: {...view(), id: ""}},
  {name: "unbounded ID", input: {...view(), id: "a".repeat(129)}},
  {name: "empty title", input: {...view(), title: "  "}},
  {name: "unbounded title", input: {...view(), title: "a".repeat(257)}},
  {name: "control title", input: {...view(), title: "Notes\nSettings"}},
  {name: "non-function render", input: {...view(), render: "notes"}},
  {name: "NaN priority", input: {...view(), priority: NaN}},
  {name: "positive infinite priority", input: {...view(), priority: Infinity}},
  {name: "negative infinite priority", input: {...view(), priority: -Infinity}},
  {name: "string priority", input: {...view(), priority: "100"}},
])("rejects a malformed dynamic contribution: $name", async ({input}) => {
  const {workbench, open, prepareClose} = setup();
  const malformed = plugin("note_manager", context => {
    // The real module is JavaScript; exercise the boundary independently of
    // the declaration's compile-time shape.
    context.registerView(input as unknown as PluginViewContribution);
  });
  await expect(workbench.activate(catalog(malformed))).rejects.toMatchObject({
    code: "plugin_activation_failed", cause: expect.objectContaining({code: "plugin_view_invalid"}),
  });
  expect(workbench.views.getSnapshot()).toEqual([]);
  expect(open).not.toHaveBeenCalled(); expect(prepareClose).not.toHaveBeenCalled();
});

it("navigation cleanup is idempotent and cannot detach a newer App binding", async () => {
  const workbench = createRuntimePluginWorkbench({storage: localStorage});
  let registration: PluginViewRegistration | undefined;
  await workbench.activate(catalog(plugin("note_manager", context => { registration = context.registerView(view()); })));
  await expect(registration?.open()).rejects.toMatchObject({code: "plugin_navigation_unavailable"});
  const first = {open: vi.fn(async () => undefined), prepareClose: vi.fn(async () => true)};
  const second = {open: vi.fn(async () => undefined), prepareClose: vi.fn(async () => true)};
  const detachFirst = workbench.attachNavigation(first), detachSecond = workbench.attachNavigation(second);
  detachFirst(); detachFirst();
  await registration?.open();
  expect(first.open).not.toHaveBeenCalled();
  expect(second.open).toHaveBeenCalledTimes(1);
  detachSecond(); detachSecond();
  await expect(registration?.dispose()).rejects.toMatchObject({code: "plugin_navigation_unavailable"});
  expect(workbench.views.getSnapshot()).toHaveLength(1);
  workbench.attachNavigation(first);
  await registration?.dispose();
  expect(workbench.views.getSnapshot()).toEqual([]);
});

it("wraps render hooks with the real configuration and shared Agent owners using all four legacy keys", async () => {
  const {workbench} = setup(), prefix = "research-assistant:workspace.v1:";
  const settings = {mode: "dependent", scope: "archive", explorerOpen: true, fullIndex: true};
  for (const [key, value] of Object.entries({"notes-mode": settings.mode, "notes-scope": settings.scope,
    "notes-explorer-open": true, "notes-full-index": true})) localStorage.setItem(prefix + key, JSON.stringify(value));
  let configuration: PluginConfiguration | undefined;
  const notes = {...plugin("note_manager", vi.fn()), module: {configuration: noteConfiguration, activate: (context: PluginActivationContext) => {
    context.registerView({...view(), render: props => {
      configuration = usePluginConfiguration();
      const facets = useAgentExtensions();
      expect(facets.owner).toBe("note_manager");
      expect(facets.skills).toBe(coreAgentExtensionFacets.skills);
      expect(facets.mcp).toBe(coreAgentExtensionFacets.mcp);
      return <output>{JSON.stringify({active: props.active, ...Object.fromEntries(Object.keys(settings).map(key => [key, configuration?.get(key)]))})}</output>;
    }});
  }}};
  const bindings = ["core.agent.skills", "core.agent.tools", "core.agent.mcp"].map(capabilityId => ({pluginId: "note_manager", capabilityId, provider: "core", version: 1}));
  await workbench.activate({...catalog(notes), bindings});
  const container = await render(firstView(workbench.views.getSnapshot()));
  expect(container.textContent).toBe(JSON.stringify({active: true, ...settings}));
  expect(configuration?.owner).toBe("note_manager");
  expect(Object.keys(configuration ?? {}).sort()).toEqual(["get", "inspect", "owner", "subscribe", "update"]);
  configuration?.update("mode", "independent");
  configuration?.update("scope", "favorites");
  configuration?.update("explorerOpen", false);
  configuration?.update("fullIndex", false);
  expect(["notes-mode", "notes-scope", "notes-explorer-open", "notes-full-index"].map(key => localStorage.getItem(prefix + key)))
    .toEqual(['"independent"', '"favorites"', "false", "false"]);
  expect(Object.keys(localStorage).some(key => key.startsWith("research-assistant:runtime-plugin.v1:"))).toBe(false);
});

it("App derives configuration storage keys and rejects a mismatched declaration owner before activation", async () => {
  const {workbench} = setup();
  const declaration = {owner: "fixture", settings: [{key: "visible", schema: {type: "boolean"},
    defaultValue: false, scopes: ["application"], effect: "immediate"}],
    storage: {visible: {application: "another-owner-key"}}} satisfies PluginConfigurationDeclaration & {readonly storage: unknown};
  const fixture = plugin("fixture", context => { context.registerView({id: "fixture", title: "Fixture", render: () => {
    const configuration = usePluginConfiguration(); configuration.update("visible", true); return "Fixture";
  }}); });
  await workbench.activate(catalog({...fixture, module: {...fixture.module, configuration: declaration}}));
  await render(firstView(workbench.views.getSnapshot()));
  expect(localStorage.getItem("another-owner-key")).toBeNull();
  expect(localStorage.getItem("research-assistant:runtime-plugin.v1:fixture:visible")).toBe("true");
  const invalid = {...plugin("invalid", vi.fn(), 2), module: {activate: vi.fn(), configuration: declaration}};
  await expect(workbench.activate(catalog(fixture, invalid))).rejects.toMatchObject({code: "configuration_owner_mismatch", owner: "invalid"});
  expect(invalid.module.activate).not.toHaveBeenCalled();
});

it.each([null, {}, {owner: "fixture", settings: [null]}, {owner: "fixture", settings: [{key: "visible"}]}])(
  "rejects a malformed dynamic configuration before activation: %j", async declaration => {
    const {workbench} = setup(), activate = vi.fn();
    const malformed = {...plugin("fixture", activate), module: {activate,
      configuration: declaration as unknown as PluginConfigurationDeclaration}};
    await expect(workbench.activate(catalog(malformed))).rejects.toMatchObject({code: "configuration_declaration_invalid", owner: "fixture"});
    expect(activate).not.toHaveBeenCalled();
  },
);

it("does not interpret an inherited property as an App legacy storage mapping", async () => {
  const {workbench} = setup();
  let configuration: PluginConfiguration | undefined;
  const notes = plugin("note_manager", context => { context.registerView({...view(), render: () => {
    configuration = usePluginConfiguration(); return "Fixture";
  }}); });
  const declaration: PluginConfigurationDeclaration = {owner: "note_manager", settings: [{key: "constructor",
    schema: {type: "boolean"}, defaultValue: false, scopes: ["application"], effect: "immediate"}]};
  await workbench.activate(catalog({...notes, module: {...notes.module, configuration: declaration}}));
  await render(firstView(workbench.views.getSnapshot()));
  configuration?.update("constructor", true);
  expect(localStorage.getItem("research-assistant:runtime-plugin.v1:note_manager:constructor")).toBe("true");
});

it("awaits only the failing instance rollback and rejects its late registrations", async () => {
  const {workbench, open} = setup(), pending = deferred<void>(), reason = new Error("Activation failed");
  const cleanup = vi.fn(() => pending.promise);
  let existingRegistration: PluginViewRegistration | undefined;
  const existing = plugin("existing", context => { existingRegistration = context.registerView(view("existing")); });
  await workbench.activate(catalog(existing));
  let failedContext: PluginActivationContext | undefined;
  const failed = plugin("note_manager", context => {
    failedContext = context;
    const registration = context.registerView(view());
    context.onDispose(() => registration.dispose());
    context.onDispose(cleanup);
    throw reason;
  }, 2);
  let settled = false;
  const attempt = workbench.activate(catalog(existing, failed));
  const rejected = expect(attempt).rejects.toMatchObject({code: "plugin_activation_failed", cause: reason});
  const observed = attempt.finally(() => { settled = true; }).catch(() => undefined);
  await vi.waitFor(() => expect(cleanup).toHaveBeenCalledTimes(1));
  expect(settled).toBe(false);
  expect(() => failedContext?.registerView(view("late"))).toThrow("plugin_activation_context_closed");
  expect(() => failedContext?.onDispose(vi.fn())).toThrow("plugin_activation_context_closed");
  pending.resolve();
  await rejected; await observed;
  expect(workbench.views.getSnapshot().map(({id}) => id)).toEqual(["existing"]);
  expect(workbench.retainPluginStyles(existing.entryUrl)).toBe(true);
  expect(workbench.retainPluginStyles(failed.entryUrl)).toBe(false);
  await existingRegistration?.open();
  expect(open).toHaveBeenCalledExactlyOnceWith(workbench.views.getSnapshot()[0], expect.any(AbortSignal));
});

it("keeps a package activated earlier in the same failing catalog active and routable", async () => {
  const {workbench, open} = setup();
  let registration: PluginViewRegistration | undefined;
  const first = plugin("one", vi.fn(context => { registration = context.registerView(view("one")); }));
  const second = plugin("two", context => { context.registerView(view("two")); throw new Error("Second activation failed"); }, 2);
  await expect(workbench.activate(catalog(first, second))).rejects.toMatchObject({code: "plugin_activation_failed", pluginId: "two"});
  expect(workbench.retainPluginStyles(first.entryUrl)).toBe(true);
  expect(workbench.retainPluginStyles(second.entryUrl)).toBe(false);
  expect(workbench.views.getSnapshot().map(({id}) => id)).toEqual(["one"]);
  await registration?.open();
  expect(open).toHaveBeenCalledExactlyOnceWith(workbench.views.getSnapshot()[0], expect.any(AbortSignal));
  await workbench.activate(catalog(first));
  expect(first.module.activate).toHaveBeenCalledTimes(1);
});

it("keeps async cleanup failures diagnosable instead of activating a second instance", async () => {
  const {workbench} = setup(), original = new Error("Activation cause"), cleanupCause = new Error("Cleanup cause");
  const cleanup = vi.fn(async () => { throw cleanupCause; });
  const activate = vi.fn((context: PluginActivationContext) => { context.registerView(view()); context.onDispose(cleanup); throw original; });
  const failed = plugin("note_manager", activate), loaded = catalog(failed);
  const failure = await workbench.activate(loaded).catch((error: unknown) => error);
  expect(failure).toBeInstanceOf(RuntimePluginWorkbenchError);
  expect(failure).toMatchObject({code: "plugin_activation_cleanup_failed", pluginId: "note_manager", cause: expect.any(AggregateError)});
  if (!(failure instanceof RuntimePluginWorkbenchError) || !(failure.cause instanceof AggregateError)) throw new Error("Expected owned cleanup failure");
  expect(failure.cause.errors).toEqual([original, cleanupCause]);
  expect(workbench.views.getSnapshot()).toEqual([]);
  expect(workbench.retainPluginStyles(failed.entryUrl)).toBe(true);
  await expect(workbench.activate(loaded)).rejects.toBe(failure);
  expect(activate).toHaveBeenCalledTimes(1);
  expect(cleanup).toHaveBeenCalledTimes(1);
});

it("does not retry the same blocked close through both plugin and host rollback hooks", async () => {
  const {workbench, prepareClose} = setup();
  prepareClose.mockResolvedValue(false);
  const failed = plugin("note_manager", context => {
    const registration = context.registerView(view());
    context.onDispose(() => registration.dispose());
    throw new Error("Activation failed");
  });
  await expect(workbench.activate(catalog(failed))).rejects.toMatchObject({code: "plugin_activation_cleanup_failed"});
  expect(prepareClose).toHaveBeenCalledTimes(1);
  expect(workbench.views.getSnapshot()).toHaveLength(1);
  expect(workbench.retainPluginStyles(failed.entryUrl)).toBe(true);
});

it("permits retry after a fully rolled back activation failure", async () => {
  const {workbench} = setup();
  const activate = vi.fn((context: PluginActivationContext) => {
    context.registerView(view());
    if (activate.mock.calls.length === 1) throw new Error("First activation failed");
  });
  const loaded = catalog(plugin("note_manager", activate));
  await expect(workbench.activate(loaded)).rejects.toMatchObject({code: "plugin_activation_failed"});
  expect(workbench.views.getSnapshot()).toEqual([]);
  await workbench.activate(loaded);
  expect(workbench.views.getSnapshot()).toHaveLength(1);
  expect(activate).toHaveBeenCalledTimes(2);
});

it("scopes the library reading slots to the same Core providers and preserves its original sort key", async () => {
  const {workbench} = setup(), portrait = vi.fn(), restore = vi.fn(() => "pdf:original");
  const key = "research-assistant:workspace.v1:library-sort";
  localStorage.setItem(key, JSON.stringify({key: "title", direction: "ascending"}));
  let configuration: PluginConfiguration | undefined;
  const Template = ({onClose}: {onClose: () => void}) => {
    configuration = usePluginConfiguration();
    const facets = useAgentExtensions();
    expect(facets.owner).toBe("literature_library");
    expect(facets.skills).toBe(coreAgentExtensionFacets.skills); expect(facets.mcp).toBe(coreAgentExtensionFacets.mcp);
    return <button onClick={onClose}>Original dialog</button>;
  };
  const loaded = plugin("literature_library", context => {context.registerView({...view("library"),
    restore: {homeTabId: "home", readActiveTabId: restore},
    reading: {ReadingTemplateDialog: Template, requestPortraitDetail: portrait}});});
  await workbench.activate({...catalog({...loaded, module: {...loaded.module, configuration: literatureConfiguration}}),
    bindings: ["core.agent.skills", "core.agent.mcp"].map(capabilityId => ({pluginId: "literature_library", capabilityId, version: 1, provider: "core"}))});
  const registered = firstView(workbench.views.getSnapshot());
  expect(Object.isFrozen(registered.restore)).toBe(true); expect(Object.isFrozen(registered.reading)).toBe(true);
  expect(registered.restore?.readActiveTabId()).toBe("pdf:original");
  registered.reading?.requestPortraitDetail?.("paper");
  expect(portrait).toHaveBeenCalledExactlyOnceWith("paper");
  const Dialog = registered.reading?.ReadingTemplateDialog;
  if (!Dialog) throw new Error("Fixture dialog missing");
  const container = document.createElement("div"), root = createRoot(container); roots.push(root);
  const onClose = vi.fn();
  await act(async () => root.render(<Dialog onClose={onClose}/>));
  expect(configuration?.get("sort")).toEqual({key: "title", direction: "ascending"});
  await act(async () => container.querySelector("button")!.click());
  expect(onClose).toHaveBeenCalledOnce();
  configuration?.update("sort", {key: "year", direction: "descending"});
  expect(JSON.parse(localStorage.getItem(key)!)).toEqual({key: "year", direction: "descending"});
  expect(localStorage.getItem("research-assistant:runtime-plugin.v1:literature_library:sort")).toBeNull();
});

it("keeps all reading projections through blocked owner close and withdraws them only on successful cleanup", async () => {
  const {workbench, prepareClose} = setup(), portrait = vi.fn();
  await workbench.activate(catalog(plugin("literature_library", context => {context.registerView({...view("library"),
    restore: {homeTabId: "home", readActiveTabId: () => "home"}, reading: {requestPortraitDetail: portrait}});})));
  const snapshot = workbench.views.getSnapshot(), registered = firstView(snapshot);
  prepareClose.mockResolvedValueOnce(false);
  await expect(workbench.deactivate("literature_library")).rejects.toMatchObject({code: "plugin_deactivation_failed"});
  expect(workbench.views.getSnapshot()).toBe(snapshot);
  expect(registered.reading?.requestPortraitDetail).toBeDefined();
  expect(() => registered.reading?.requestPortraitDetail?.("late")).toThrow("plugin_activation_context_closed");
  expect(portrait).not.toHaveBeenCalled();
  await workbench.deactivate("literature_library");
  expect(workbench.views.getSnapshot()).toEqual([]);
  expect(() => registered.restore?.readActiveTabId()).toThrow("plugin_activation_context_closed");
});

it.each(["foreign-owner", "wrong-view", "extra-slot"])("rejects an invalid library projection: %s", async kind => {
  const {workbench} = setup();
  const declaration = {...view(kind === "wrong-view" ? "other" : "library"), reading: {
    requestPortraitDetail: vi.fn(), ...(kind === "extra-slot" ? {invoke: vi.fn()} : {}),
  }};
  const loaded = plugin(kind === "foreign-owner" ? "foreign" : "literature_library", context => {context.registerView(declaration);});
  await expect(workbench.activate(catalog(loaded))).rejects.toMatchObject({
    code: "plugin_activation_failed", cause: expect.objectContaining({code: "plugin_view_invalid"}),
  });
  expect(workbench.views.getSnapshot()).toEqual([]);
});
