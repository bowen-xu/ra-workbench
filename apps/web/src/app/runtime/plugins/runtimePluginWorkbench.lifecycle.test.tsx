// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import {decodePluginManifest, WORKBENCH_UI_RUNTIME} from "@research-assistant/workbench-sdk";
import {resolveBuiltinPlugins} from "@research-assistant/workbench-sdk/composition";
import type {PluginActivationContext, PluginViewRegistration, RuntimePluginModule} from "@research-assistant/workbench-sdk/activation";
import type {LoadedRuntimePlugin, RuntimePluginLoadResult} from "../../../core/runtime/plugins/loader";
import type {RegisteredPluginView} from "../../../core/runtime/plugins/viewRegistry";
import {createRuntimePluginWorkbench, type RuntimePluginState} from "./runtimePluginWorkbench";

afterEach(() => { vi.useRealTimers(); localStorage.clear(); });
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((accept, refuse) => { resolve = accept; reject = refuse; });
  return {promise, resolve, reject};
}
function plugin(id: string, module: RuntimePluginModule, index = 1): LoadedRuntimePlugin {
  return {manifest: decodePluginManifest({formatVersion: 1, id, version: "1.0.0", apiVersion: 1,
    entry: "assets/index.js", styles: ["assets/index.css"], uiRuntime: WORKBENCH_UI_RUNTIME,
    dependencies: {required: [], optional: []}, provides: [], consumes: {required: [], optional: []}}),
  module, entryUrl: "/plugins/" + index.toString(16).padStart(64, "0") + "/assets/index.js", styles: []};
}
function catalog(...plugins: readonly LoadedRuntimePlugin[]): RuntimePluginLoadResult {
  return {plugins, bindings: [], unavailableOptional: []};
}
function setup(lifecycleTimeoutMs = 100) {
  const workbench = createRuntimePluginWorkbench({storage: localStorage, lifecycleTimeoutMs});
  const open = vi.fn(async (_view: RegisteredPluginView, _signal?: AbortSignal) => undefined);
  const prepareClose = vi.fn(async (_view: RegisteredPluginView, _signal?: AbortSignal): Promise<boolean> => true);
  workbench.attachNavigation({open, prepareClose});
  return {workbench, open, prepareClose};
}
const contribution = (id = "notes") => ({id, title: "Fixture " + id, render: () => id});

it("publishes the finite lifecycle, joins concurrent stop, and reactivates a fresh context over the same resources", async () => {
  const {workbench, prepareClose} = setup(), gate = deferred<boolean>(), cleanup = vi.fn();
  const contexts: PluginActivationContext[] = [], registrations: PluginViewRegistration[] = [];
  const states: RuntimePluginState[] = [];
  const unsubscribe = workbench.subscribe(() => {
    const status = workbench.getSnapshot()[0]; if (status) states.push(status.state);
  });
  const deactivate = vi.fn();
  const loaded = plugin("note_manager", {activate(context) {
    contexts.push(context); registrations.push(context.registerView(contribution()));
    context.onDispose(() => registrations.at(-1)!.dispose()); context.onDispose(cleanup);
  }, deactivate});
  await workbench.activate(catalog(loaded));
  const originalView = workbench.views.getSnapshot()[0], originalContext = contexts[0]!;
  prepareClose.mockReturnValueOnce(gate.promise);
  const stop = workbench.deactivate("note_manager"), duplicate = workbench.deactivate("note_manager");
  expect(duplicate).toBe(stop);
  await vi.waitFor(() => expect(prepareClose).toHaveBeenCalledOnce());
  expect(workbench.getSnapshot()[0]?.state).toBe("deactivating");
  expect(workbench.views.getSnapshot()[0]).toBe(originalView);
  expect(workbench.retainPluginStyles(loaded.entryUrl)).toBe(true);
  expect(deactivate).not.toHaveBeenCalled(); expect(cleanup).not.toHaveBeenCalled();
  expect(() => originalContext.registerView(contribution("late"))).toThrow("plugin_activation_context_closed");
  expect(() => originalContext.onDispose(vi.fn())).toThrow("plugin_activation_context_closed");
  await expect(registrations[0]!.open()).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  gate.resolve(true); await stop;
  expect(workbench.getSnapshot()[0]?.state).toBe("disabled");
  expect(workbench.views.getSnapshot()).toEqual([]);
  expect(workbench.retainPluginStyles(loaded.entryUrl)).toBe(false);
  expect(prepareClose).toHaveBeenCalledOnce();
  expect(deactivate).toHaveBeenCalledOnce(); expect(cleanup).toHaveBeenCalledOnce();
  await workbench.activate(catalog(loaded));
  expect(contexts).toHaveLength(2);
  expect(workbench.getSnapshot()[0]?.state).toBe("ready");
  expect(workbench.views.getSnapshot()[0]).not.toBe(originalView);
  expect(() => originalContext.registerView(contribution("stale"))).toThrow("plugin_activation_context_closed");
  expect(states).toEqual(["discovered", "activating", "ready", "deactivating", "disabled", "discovered", "activating", "ready"]);
  const snapshot = workbench.getSnapshot();
  expect(Object.isFrozen(snapshot)).toBe(true); expect(Object.isFrozen(snapshot[0])).toBe(true);
  unsubscribe();
});

it("prepares every view before any module hook or withdrawal, retaining all views on save refusal", async () => {
  const {workbench, prepareClose} = setup(), deactivate = vi.fn(), cleanup = vi.fn();
  const loaded = plugin("note_manager", {activate(context) {
    context.registerView(contribution("one")); context.registerView(contribution("two")); context.onDispose(cleanup);
  }, deactivate});
  await workbench.activate(catalog(loaded));
  const snapshot = workbench.views.getSnapshot();
  prepareClose.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
  await expect(workbench.deactivate("note_manager")).rejects.toMatchObject({
    code: "plugin_deactivation_failed", cause: expect.objectContaining({code: "plugin_view_close_blocked", viewId: "two"}),
  });
  expect(workbench.views.getSnapshot()).toBe(snapshot);
  expect(workbench.getSnapshot()[0]).toMatchObject({state: "failed", error: {code: "plugin_deactivation_failed"}});
  expect(workbench.retainPluginStyles(loaded.entryUrl)).toBe(true);
  expect(deactivate).not.toHaveBeenCalled(); expect(cleanup).not.toHaveBeenCalled();
  await workbench.deactivate("note_manager");
  expect(deactivate).toHaveBeenCalledOnce(); expect(cleanup).toHaveBeenCalledOnce();
  expect(workbench.getSnapshot()[0]?.state).toBe("disabled");
});

it("bounds save preparation and ignores its late completion without withdrawing the retained view", async () => {
  vi.useFakeTimers();
  const {workbench, prepareClose} = setup(20), gate = deferred<boolean>(), deactivate = vi.fn();
  const loaded = plugin("note_manager", {activate: context => { context.registerView(contribution()); }, deactivate});
  await workbench.activate(catalog(loaded));
  const snapshot = workbench.views.getSnapshot();
  prepareClose.mockReturnValue(gate.promise);
  const stop = workbench.deactivate("note_manager");
  const rejected = expect(stop).rejects.toMatchObject({code: "plugin_lifecycle_timeout", phase: "prepareClose", timeoutMs: 20});
  await vi.advanceTimersByTimeAsync(21); await rejected;
  const status = workbench.getSnapshot();
  expect(prepareClose.mock.calls[0]?.[1]?.aborted).toBe(true);
  gate.resolve(true); await vi.advanceTimersByTimeAsync(0);
  expect(workbench.views.getSnapshot()).toBe(snapshot);
  expect(workbench.getSnapshot()).toBe(status);
  expect(workbench.retainPluginStyles(loaded.entryUrl)).toBe(true);
  expect(deactivate).not.toHaveBeenCalled();
  await expect(workbench.activate(catalog(loaded))).rejects.toBe(status[0]?.error);
});

it.each(["module", "cleanup"] as const)("retains real views and CSS after a %s failure without replaying its hook", async phase => {
  const {workbench} = setup(), cause = new Error("DEACTIVATION_FIXTURE_FAILURE");
  const fail = async (): Promise<void> => { throw cause; };
  const deactivate = vi.fn(phase === "module" ? fail : async () => undefined);
  const cleanup = vi.fn(phase === "cleanup" ? fail : async () => undefined);
  const loaded = plugin("note_manager", {activate(context) { context.registerView(contribution()); context.onDispose(cleanup); }, deactivate});
  await workbench.activate(catalog(loaded));
  const snapshot = workbench.views.getSnapshot();
  await expect(workbench.deactivate("note_manager")).rejects.toMatchObject({code: "plugin_deactivation_failed", cause});
  expect(workbench.views.getSnapshot()).toBe(snapshot);
  expect(workbench.retainPluginStyles(loaded.entryUrl)).toBe(true);
  await expect(workbench.deactivate("note_manager")).rejects.toMatchObject({cause});
  expect(deactivate).toHaveBeenCalledOnce();
  expect(cleanup).toHaveBeenCalledTimes(phase === "cleanup" ? 1 : 0);
});

it("joins a timed-out module hook on explicit retry and does not let late settlement mark the instance disabled", async () => {
  vi.useFakeTimers();
  const {workbench} = setup(20), gate = deferred<void>(), cleanup = vi.fn();
  const deactivate = vi.fn(() => gate.promise);
  const loaded = plugin("note_manager", {activate(context) { context.registerView(contribution()); context.onDispose(cleanup); }, deactivate});
  await workbench.activate(catalog(loaded));
  const originalView = workbench.views.getSnapshot()[0];
  const stop = workbench.deactivate("note_manager");
  const rejected = expect(stop).rejects.toMatchObject({code: "plugin_lifecycle_timeout", phase: "deactivate"});
  await vi.advanceTimersByTimeAsync(21); await rejected;
  const status = workbench.getSnapshot();
  await expect(workbench.deactivate("note_manager")).rejects.toBe(status[0]?.error);
  expect(workbench.getSnapshot()).toBe(status);
  expect(deactivate).toHaveBeenCalledOnce();
  gate.resolve(); await vi.advanceTimersByTimeAsync(0);
  expect(workbench.getSnapshot()).toBe(status);
  expect(workbench.views.getSnapshot()[0]).toBe(originalView);
  expect(cleanup).not.toHaveBeenCalled();
  await workbench.deactivate("note_manager");
  expect(deactivate).toHaveBeenCalledOnce(); expect(cleanup).toHaveBeenCalledOnce();
  expect(workbench.getSnapshot()[0]?.state).toBe("disabled");
});

it("does not replay an accepted timed-out disposer and preserves the package until an explicit retry completes it", async () => {
  vi.useFakeTimers();
  const {workbench} = setup(20), gate = deferred<void>(), order: string[] = [];
  const cleanup = vi.fn(() => { order.push("async"); return gate.promise; });
  const loaded = plugin("note_manager", {activate(context) {
    const registration = context.registerView(contribution());
    context.onDispose(() => registration.dispose());
    context.onDispose(() => { order.push("first"); });
    context.onDispose(cleanup);
  }, deactivate: () => { order.push("module"); }});
  await workbench.activate(catalog(loaded));
  const snapshot = workbench.views.getSnapshot();
  const stop = workbench.deactivate("note_manager");
  const rejected = expect(stop).rejects.toMatchObject({code: "plugin_lifecycle_timeout", phase: "cleanup"});
  await vi.advanceTimersByTimeAsync(21); await rejected;
  expect(order).toEqual(["module", "async"]);
  await expect(workbench.deactivate("note_manager")).rejects.toBe(workbench.getSnapshot()[0]?.error);
  gate.resolve(); await vi.advanceTimersByTimeAsync(0);
  expect(workbench.views.getSnapshot()).toBe(snapshot);
  expect(workbench.getSnapshot()[0]?.state).toBe("failed");
  await workbench.deactivate("note_manager");
  expect(order).toEqual(["module", "async", "first"]);
  expect(cleanup).toHaveBeenCalledOnce();
  expect(workbench.views.getSnapshot()).toEqual([]);
});

it("bounds activation without running concurrent cleanup or accepting its late context", async () => {
  vi.useFakeTimers();
  const {workbench} = setup(20), gate = deferred<void>(), cleanup = vi.fn(), deactivate = vi.fn();
  let context!: PluginActivationContext;
  const activate = vi.fn((value: PluginActivationContext) => {
    context = value; context.registerView(contribution()); context.onDispose(cleanup); return gate.promise;
  });
  const loaded = plugin("note_manager", {activate, deactivate});
  const attempt = workbench.activate(catalog(loaded));
  const rejected = expect(attempt).rejects.toMatchObject({code: "plugin_lifecycle_timeout", phase: "activate"});
  await vi.advanceTimersByTimeAsync(21); await rejected;
  const status = workbench.getSnapshot(), snapshot = workbench.views.getSnapshot();
  await expect(workbench.deactivate("note_manager")).rejects.toBe(status[0]?.error);
  expect(cleanup).not.toHaveBeenCalled(); expect(deactivate).not.toHaveBeenCalled();
  expect(() => context.registerView(contribution("late"))).toThrow("plugin_activation_context_closed");
  gate.resolve(); await vi.advanceTimersByTimeAsync(0);
  expect(workbench.getSnapshot()).toBe(status); expect(workbench.views.getSnapshot()).toBe(snapshot);
  await expect(workbench.activate(catalog(loaded))).rejects.toBe(status[0]?.error);
  expect(activate).toHaveBeenCalledOnce();
  await workbench.deactivate("note_manager");
  expect(deactivate).toHaveBeenCalledOnce(); expect(cleanup).toHaveBeenCalledOnce();
  await workbench.activate(catalog(loaded));
  expect(activate).toHaveBeenCalledTimes(2);
});

it.each(["manifest", "capability"] as const)("stops the actual %s consumer before its provider", async kind => {
  const {workbench} = setup(), order: string[] = [];
  const provider = plugin("provider", {activate: context => { context.registerView(contribution("provider")); },
    deactivate: () => { order.push("provider"); }});
  const consumer = plugin("consumer", {activate: context => { context.registerView(contribution("consumer")); },
    deactivate: () => { order.push("consumer"); }}, 2);
  const consuming = kind === "manifest" ? {...consumer, manifest: {...consumer.manifest,
    dependencies: {required: [{id: "provider", apiVersion: 1}], optional: []}}} : consumer;
  // Deliberately place the capability consumer first: teardown uses the
  // accepted binding, not an assumed reverse catalog topological order.
  const result = kind === "manifest" ? catalog(provider, consuming) : {...catalog(consuming, provider),
    bindings: [{pluginId: "consumer", capabilityId: "fixture.resource", provider: "provider", version: 1}]};
  await workbench.activate(result);
  await workbench.deactivate("provider");
  expect(order).toEqual(["consumer", "provider"]);
  expect(workbench.getSnapshot().every(status => status.state === "disabled")).toBe(true);
});

it("leaves the provider available when its actual consumer cannot safely stop", async () => {
  const {workbench, prepareClose} = setup(), deactivate = vi.fn();
  const provider = plugin("provider", {activate: context => { context.registerView(contribution("provider")); }, deactivate});
  const consumer = plugin("consumer", {activate: context => { context.registerView(contribution("consumer")); }}, 2);
  await workbench.activate({...catalog(provider, consumer),
    bindings: [{pluginId: "consumer", capabilityId: "fixture.resource", provider: "provider", version: 1}]});
  prepareClose.mockResolvedValue(false);
  await expect(workbench.deactivate("provider")).rejects.toMatchObject({pluginId: "consumer"});
  expect(deactivate).not.toHaveBeenCalled();
  expect(workbench.getSnapshot().find(status => status.pluginId === "provider")?.state).toBe("ready");
  expect(workbench.retainPluginStyles(provider.entryUrl)).toBe(true);
});

it("disposes other independent packages even when one cleanup fails, without replay on repeated disposal", async () => {
  const {workbench} = setup(), cleanup = vi.fn(), cause = new Error("OWNED_CLEANUP_FIXTURE_FAILURE");
  const safe = plugin("safe", {activate: context => { context.registerView(contribution("safe")); }, deactivate: cleanup});
  const failed = plugin("failed", {activate: context => { context.registerView(contribution("failed")); },
    deactivate: vi.fn(async () => { throw cause; })}, 2);
  await workbench.activate(catalog(safe, failed));
  const disposal = workbench.dispose();
  expect(workbench.dispose()).toBe(disposal);
  await expect(disposal).rejects.toBeInstanceOf(AggregateError);
  expect(workbench.getSnapshot().find(status => status.pluginId === "safe")?.state).toBe("disabled");
  expect(workbench.getSnapshot().find(status => status.pluginId === "failed")?.state).toBe("failed");
  expect(workbench.retainPluginStyles(safe.entryUrl)).toBe(false);
  expect(workbench.retainPluginStyles(failed.entryUrl)).toBe(true);
  expect(cleanup).toHaveBeenCalledOnce(); expect(failed.module.deactivate).toHaveBeenCalledOnce();
  await expect(workbench.activate(catalog(safe))).rejects.toMatchObject({code: "plugin_workbench_disposed"});
});

it("does not implicitly retry a blocked consumer while disposing its provider in the same attempt", async () => {
  const {workbench, prepareClose} = setup(), providerStop = vi.fn();
  const provider = plugin("provider", {activate: context => { context.registerView(contribution("provider")); }, deactivate: providerStop});
  const consumer = plugin("consumer", {activate: context => { context.registerView(contribution("consumer")); }}, 2);
  await workbench.activate({...catalog(provider, consumer),
    bindings: [{pluginId: "consumer", capabilityId: "fixture.resource", provider: "provider", version: 1}]});
  prepareClose.mockResolvedValue(false);
  await expect(workbench.dispose()).rejects.toBeInstanceOf(AggregateError);
  expect(prepareClose).toHaveBeenCalledOnce();
  expect(prepareClose.mock.calls[0]?.[0].pluginId).toBe("consumer");
  expect(providerStop).not.toHaveBeenCalled();
  expect(workbench.retainPluginStyles(provider.entryUrl)).toBe(true);
  expect(workbench.retainPluginStyles(consumer.entryUrl)).toBe(true);
});

it("treats an accepted self-provided capability as the same owner rather than a teardown dependency cycle", async () => {
  const {workbench} = setup(), deactivate = vi.fn();
  const loaded = plugin("fixture", {activate: context => { context.registerView(contribution("fixture")); }, deactivate});
  await workbench.activate({...catalog(loaded),
    bindings: [{pluginId: "fixture", capabilityId: "fixture.resource", provider: "fixture", version: 1}]});
  await workbench.deactivate("fixture");
  expect(deactivate).toHaveBeenCalledOnce();
  expect(workbench.getSnapshot()[0]?.state).toBe("disabled");
});

it("uses one bounded stop deadline across save preparation and module cleanup", async () => {
  vi.useFakeTimers();
  const {workbench, prepareClose} = setup(20), saving = deferred<boolean>(), cleanup = deferred<void>();
  const deactivate = vi.fn(() => cleanup.promise);
  const loaded = plugin("note_manager", {activate: context => { context.registerView(contribution()); }, deactivate});
  await workbench.activate(catalog(loaded));
  prepareClose.mockReturnValue(saving.promise);
  const stop = workbench.deactivate("note_manager");
  const rejected = expect(stop).rejects.toMatchObject({code: "plugin_lifecycle_timeout", phase: "deactivate"});
  await vi.advanceTimersByTimeAsync(15);
  saving.resolve(true); await vi.advanceTimersByTimeAsync(0);
  expect(deactivate).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(6); await rejected;
  expect(workbench.retainPluginStyles(loaded.entryUrl)).toBe(true);
  cleanup.resolve(); await vi.advanceTimersByTimeAsync(0);
  expect(workbench.getSnapshot()[0]?.state).toBe("failed");
});

it("does not start another queued module after application disposal was requested", async () => {
  const {workbench} = setup(), pending = deferred<void>();
  const first = plugin("first", {activate: vi.fn(() => pending.promise), deactivate: vi.fn()});
  const second = plugin("second", {activate: vi.fn()}, 2);
  const activating = workbench.activate(catalog(first, second));
  const rejected = expect(activating).rejects.toMatchObject({code: "plugin_workbench_disposed"});
  await vi.waitFor(() => expect(first.module.activate).toHaveBeenCalledOnce());
  const disposing = workbench.dispose();
  pending.resolve(); await rejected; await disposing;
  expect(second.module.activate).not.toHaveBeenCalled();
  expect(first.module.deactivate).toHaveBeenCalledOnce();
  expect(workbench.getSnapshot().find(status => status.pluginId === "first")?.state).toBe("disabled");
});

it("allows module.deactivate to await its own prepared view disposal while keeping other context operations closed", async () => {
  const {workbench, prepareClose} = setup();
  let registration!: PluginViewRegistration, context!: PluginActivationContext;
  const deactivate = vi.fn(async () => {
    expect(() => context.registerView(contribution("late"))).toThrow("plugin_activation_context_closed");
    expect(() => context.onDispose(vi.fn())).toThrow("plugin_activation_context_closed");
    await expect(registration.open()).rejects.toMatchObject({code: "plugin_activation_context_closed"});
    await registration.dispose();
  });
  const loaded = plugin("note_manager", {activate(value) {
    context = value; registration = context.registerView(contribution());
    context.onDispose(() => registration.dispose());
  }, deactivate});
  await workbench.activate(catalog(loaded));
  await workbench.deactivate("note_manager");
  expect(deactivate).toHaveBeenCalledOnce();
  expect(prepareClose).toHaveBeenCalledOnce();
  expect(workbench.views.getSnapshot()).toEqual([]);
  expect(workbench.getSnapshot()[0]?.state).toBe("disabled");
});

it("rejects a module's late handle disposal after its prepared teardown attempt timed out", async () => {
  vi.useFakeTimers();
  const {workbench} = setup(20), gate = deferred<void>();
  let registration!: PluginViewRegistration;
  const deactivate = vi.fn(async () => { await gate.promise; await registration.dispose(); });
  const loaded = plugin("note_manager", {activate: context => { registration = context.registerView(contribution()); }, deactivate});
  await workbench.activate(catalog(loaded));
  const snapshot = workbench.views.getSnapshot();
  const stop = workbench.deactivate("note_manager");
  const rejected = expect(stop).rejects.toMatchObject({code: "plugin_lifecycle_timeout", phase: "deactivate"});
  await vi.advanceTimersByTimeAsync(21); await rejected;
  const status = workbench.getSnapshot();
  gate.resolve(); await vi.advanceTimersByTimeAsync(0);
  await expect(deactivate.mock.results[0]?.value).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  expect(workbench.views.getSnapshot()).toBe(snapshot);
  expect(workbench.getSnapshot()).toBe(status);
  expect(workbench.retainPluginStyles(loaded.entryUrl)).toBe(true);
});

it("keeps an incompatible optional consumer running according to the existing SDK resolver decision", async () => {
  const {workbench} = setup(), consumerStop = vi.fn();
  const provider = plugin("provider", {activate: context => { context.registerView(contribution("provider")); }});
  const consumer = plugin("consumer", {activate: context => { context.registerView(contribution("consumer")); }, deactivate: consumerStop}, 2);
  const optionalConsumer = {...consumer, manifest: {...consumer.manifest,
    dependencies: {required: [], optional: [{id: "provider", apiVersion: 2}]}}};
  const packages = [provider, optionalConsumer];
  const resolved = resolveBuiltinPlugins(packages.map(candidate => candidate.manifest), ["provider", "consumer"], []);
  expect(resolved.unavailableOptional).toEqual([expect.objectContaining({
    code: "plugin_dependency_incompatible", pluginId: "consumer", dependencyId: "provider", expectedVersion: 2, actualVersion: 1,
  })]);
  await workbench.activate({plugins: resolved.activationOrder.map(manifest => packages.find(candidate => candidate.manifest.id === manifest.id)!),
    bindings: resolved.bindings, unavailableOptional: resolved.unavailableOptional});
  await workbench.deactivate("provider");
  expect(consumerStop).not.toHaveBeenCalled();
  expect(workbench.getSnapshot().find(status => status.pluginId === "consumer")?.state).toBe("ready");
  expect(workbench.views.getSnapshot().map(view => view.id)).toEqual(["consumer"]);
});
