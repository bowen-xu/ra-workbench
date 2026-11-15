import type {PluginActivationContext, PluginViewContribution, PluginViewProps, PluginViewRegistration, PluginReadingContributions, PluginReadingActionsProps, PluginReadingTemplateProps, PluginWritingContributions, PluginWritingRuntimeProps} from "@research-assistant/workbench-sdk/activation";
import type {AgentDomainSessionProps} from "@research-assistant/workbench-sdk/agent-presentation";
import {useSyncExternalStore, type ReactNode} from "react";
import type {PluginConfiguration} from "../../../core/contracts/configuration";
import type {AgentExtensionFacets} from "../../../core/contracts/agentExtensions";
import {PluginConfigurationProvider} from "../../../core/configuration/ConfigurationProvider";
import {AgentExtensionProvider} from "../../../core/extensions/AgentExtensionProvider";
import type {LoadedRuntimePlugin, RuntimePluginLoadResult} from "../../../core/runtime/plugins/loader";
import {createPluginViewRegistry, type PluginViewRegistry, type PluginViewRegistryEffects} from "../../../core/runtime/plugins/viewRegistry";
import {bindBuiltinAgentExtensions} from "../../builtinAgentExtensions";
import {bindRuntimePluginConfigurations} from "./runtimePluginConfiguration";
import {awaitRuntimePluginWork, invokeRuntimePluginWork, type RuntimePluginWork} from "./runtimePluginLifecycle";
import {bindRuntimeWritingWorkbench} from "../writing/runtimeWriting";
import {createRuntimeLiteraturePort} from "../literature/runtimeLiterature";
import {bindRuntimeLiteratureInteroperability} from "../literature/runtimeLiteratureInteroperability";

export type RuntimePluginState = "discovered" | "activating" | "ready" | "failed" | "deactivating" | "disabled";
export interface RuntimePluginStatus {
  readonly pluginId: string;
  readonly entryUrl: string;
  readonly version: string;
  readonly state: RuntimePluginState;
  readonly error?: RuntimePluginWorkbenchError;
}
export interface RuntimePluginWorkbench {
  readonly views: PluginViewRegistry;
  readonly attachNavigation: (navigation: PluginViewRegistryEffects) => () => void;
  readonly activate: (result: RuntimePluginLoadResult) => Promise<void>;
  readonly deactivate: (pluginId: string) => Promise<void>;
  readonly dispose: () => Promise<void>;
  readonly getSnapshot: () => readonly RuntimePluginStatus[];
  readonly subscribe: (listener: () => void) => () => void;
  /** App startup also completes this when discovery fails before activation. */
  readonly completeInitialLoad: () => void;
  readonly isInitialLoadComplete: () => boolean;
  readonly subscribeInitialLoad: (listener: () => void) => () => void;
  /** Unresolved work keeps its exact instance and CSS, including after timeout. */
  readonly retainPluginStyles: (entryUrl: string) => boolean;
}

export type RuntimePluginWorkbenchErrorCode =
  | "plugin_restart_required" | "plugin_navigation_unavailable"
  | "plugin_view_route_conflict" | "plugin_view_reserved" | "plugin_view_invalid"
  | "plugin_activation_context_closed" | "plugin_activation_failed"
  | "plugin_activation_cleanup_failed" | "plugin_deactivation_failed"
  | "plugin_lifecycle_timeout" | "plugin_workbench_disposed" | "plugin_unknown_instance"
  | "plugin_deactivation_dependency_cycle";

export class RuntimePluginWorkbenchError extends Error {
  constructor(readonly code: RuntimePluginWorkbenchErrorCode, readonly pluginId: string,
    readonly entryUrl: string, options?: ErrorOptions, readonly viewId?: string,
    readonly phase?: string, readonly timeoutMs?: number) {
    super(code + ": " + pluginId + " / " + entryUrl + (viewId === undefined ? "" : " / " + viewId)
      + (phase === undefined ? "" : " / " + phase) + (timeoutMs === undefined ? "" : " / " + timeoutMs + "ms"), options);
    this.name = "RuntimePluginWorkbenchError";
  }
}

interface OwnedCleanup {
  readonly run: () => void | Promise<void>;
  work?: RuntimePluginWork;
}
interface RuntimePluginInstance {
  readonly plugin: LoadedRuntimePlugin;
  readonly dependencies: ReadonlySet<string>;
  readonly cleanups: OwnedCleanup[];
  state: RuntimePluginState;
  error?: RuntimePluginWorkbenchError;
  acceptsContext: boolean;
  retained: boolean;
  cleaning: boolean;
  activation?: RuntimePluginWork;
  deactivation?: RuntimePluginWork;
}

/** Invoke contribution hooks inside the original Core-owned providers. */
function RuntimePluginView({render, viewProps, subscribe, readPlugins}: {readonly render: PluginViewContribution["render"]; readonly viewProps: PluginViewProps;
  readonly subscribe: (listener: () => void) => () => void; readonly readPlugins: () => readonly RuntimePluginStatus[]}) {
  useSyncExternalStore(subscribe, readPlugins);
  return render(viewProps);
}

function scopedContribution(view: PluginViewContribution, facets: AgentExtensionFacets,
  configuration: PluginConfiguration | undefined, current: () => void,
  subscribe: (listener: () => void) => () => void, isDisabled: () => boolean,
  readPlugins: () => readonly RuntimePluginStatus[]): PluginViewContribution {
  const render = view.render;
  const scoped = (content: ReactNode) => <AgentExtensionProvider value={facets}>{configuration
      ? <PluginConfigurationProvider value={configuration}>{content}</PluginConfigurationProvider> : content}</AgentExtensionProvider>;
  const Actions = view.reading?.ReadingActions, TemplateDialog = view.reading?.ReadingTemplateDialog;
  const requestPortraitDetail = view.reading?.requestPortraitDetail;
  const restore = view.restore, readActiveTabId = restore?.readActiveTabId;
  const Runtime = view.writing?.Runtime, Session = view.writing?.Session;
  const writing: PluginWritingContributions | undefined = Runtime && Session ? {
    Runtime: (props: PluginWritingRuntimeProps) => scoped(<Runtime {...props} isDisabled={isDisabled} subscribe={subscribe}/>),
    Session: (props: AgentDomainSessionProps) => scoped(<Session {...props}/>),
  } : undefined;
  const reading: PluginReadingContributions | undefined = view.reading && {
    ...(Actions ? {ReadingActions: (props: PluginReadingActionsProps) => {
      const disabled = useSyncExternalStore(subscribe, isDisabled);
      return scoped(<Actions {...props} disabled={disabled || props.disabled === true}/>);
    }} : {}),
    ...(TemplateDialog ? {ReadingTemplateDialog: (props: PluginReadingTemplateProps) => {
      const disabled = useSyncExternalStore(subscribe, isDisabled);
      return scoped(<TemplateDialog {...props} disabled={disabled || props.disabled === true}/>);
    }} : {}),
    ...(requestPortraitDetail ? {requestPortraitDetail: (id: string) => {current(); requestPortraitDetail(id);}} : {}),
  };
  return {id: view.id, title: view.title,
    ...(view.priority === undefined ? {} : {priority: view.priority}),
    ...(restore && readActiveTabId ? {restore: {homeTabId: restore.homeTabId,
      readActiveTabId: () => {current(); return readActiveTabId.call(restore);}}} : {}),
    ...(reading ? {reading} : {}),
    ...(writing ? {writing} : {}),
    render: props => scoped(<RuntimePluginView render={render} viewProps={props} subscribe={subscribe} readPlugins={readPlugins}/>)};
}

/** Internal App assembly, not an SDK service. Core save owners and shared
 * runtimes outlive each package. No timeout authorizes forced disposal. */
export function createRuntimePluginWorkbench({storage, lifecycleTimeoutMs = 30_000}: {
  readonly storage: Storage; readonly lifecycleTimeoutMs?: number;
}): RuntimePluginWorkbench {
  if (!Number.isSafeInteger(lifecycleTimeoutMs) || lifecycleTimeoutMs <= 0 || lifecycleTimeoutMs > 2_147_483_647)
    throw new Error("plugin_lifecycle_timeout_invalid");
  const instances = new Map<string, RuntimePluginInstance>();
  const listeners = new Set<() => void>();
  const initialLoadListeners = new Set<() => void>();
  const stopping = new Map<string, Promise<void>>();
  let snapshot: readonly RuntimePluginStatus[] = Object.freeze([]);
  let initialLoadComplete = false;
  let navigation: {readonly effects: PluginViewRegistryEffects} | undefined;
  let tail: Promise<void> = Promise.resolve();
  let disposed = false;
  let disposing: Promise<void> | undefined;
  const publish = (): void => {
    snapshot = Object.freeze([...instances.values()].map(instance => Object.freeze({
      pluginId: instance.plugin.manifest.id, entryUrl: instance.plugin.entryUrl,
      version: instance.plugin.manifest.version, state: instance.state,
      ...(instance.error === undefined ? {} : {error: instance.error}),
    })));
    for (const listener of [...listeners]) listener();
  };
  const subscribe = (listener: () => void) => {
    const subscribed = (): void => listener(); listeners.add(subscribed);
    return () => { listeners.delete(subscribed); };
  };
  const completeInitialLoad = (): void => {
    if (initialLoadComplete) return;
    initialLoadComplete = true;
    for (const listener of [...initialLoadListeners]) listener();
  };
  const error = (instance: RuntimePluginInstance, code: RuntimePluginWorkbenchErrorCode,
    options?: ErrorOptions, phase?: string, viewId?: string): RuntimePluginWorkbenchError =>
    new RuntimePluginWorkbenchError(code, instance.plugin.manifest.id, instance.plugin.entryUrl,
      options, viewId, phase, code === "plugin_lifecycle_timeout" ? lifecycleTimeoutMs : undefined);
  const wait = (instance: RuntimePluginInstance, work: RuntimePluginWork, deadline: number, phase: string): Promise<void> =>
    awaitRuntimePluginWork(work, deadline, () => error(instance, "plugin_lifecycle_timeout", undefined, phase));
  const transition = (instance: RuntimePluginInstance, state: RuntimePluginState, failure?: RuntimePluginWorkbenchError): void => {
    instance.state = state;
    if (failure === undefined) delete instance.error; else instance.error = failure;
    publish();
  };
  const requireContext = (instance: RuntimePluginInstance): void => {
    if (disposed || !instance.acceptsContext || instances.get(instance.plugin.manifest.id) !== instance)
      throw error(instance, "plugin_activation_context_closed");
  };
  const requireNavigation = (pluginId: string, viewId: string): PluginViewRegistryEffects => {
    if (!navigation) throw new RuntimePluginWorkbenchError("plugin_navigation_unavailable", pluginId, "<navigation>", undefined, viewId);
    return navigation.effects;
  };
  const views = createPluginViewRegistry({
    open: (view, signal) => requireNavigation(view.pluginId, view.id).open(view, signal),
    prepareClose: (view, signal) => requireNavigation(view.pluginId, view.id).prepareClose(view, signal),
  }, {closeTimeoutMs: lifecycleTimeoutMs});
  const enqueue = (run: () => Promise<void>): Promise<void> => {
    const pending = tail.then(run);
    tail = pending.catch(() => undefined);
    return pending;
  };
  const runCleanups = async (instance: RuntimePluginInstance, deadline: number, rollback: boolean): Promise<void> => {
    instance.cleaning = true;
    const failures: unknown[] = [];
    try {
      for (const cleanup of [...instance.cleanups].reverse()) {
        if (Date.now() >= deadline) throw error(instance, "plugin_lifecycle_timeout", undefined, "cleanup");
        cleanup.work ??= invokeRuntimePluginWork(cleanup.run);
        try { await wait(instance, cleanup.work, deadline, "cleanup"); }
        catch (cause) {
          if (!rollback) throw cause;
          failures.push(cause);
          if (cleanup.work.state === "pending") break;
        }
      }
    } finally { instance.cleaning = false; }
    if (failures.length) throw new AggregateError(failures, "Plugin owned cleanup failed");
  };
  const activatePlugin = async (instance: RuntimePluginInstance, facets: AgentExtensionFacets, configuration?: PluginConfiguration): Promise<void> => {
    const {plugin} = instance, pluginId = plugin.manifest.id;
    instance.acceptsContext = true; instance.retained = true;
    transition(instance, "activating");
    const releaseWriting = bindRuntimeWritingWorkbench(plugin, () => {
      if (!instance.retained || instances.get(pluginId) !== instance)
        throw error(instance, "plugin_activation_context_closed");
    });
    if (releaseWriting) instance.cleanups.push({run: releaseWriting});
    const literatureRuntime = pluginId === "literature_library" && plugin.packageKey
      ? createRuntimeLiteraturePort(plugin.packageKey, plugin.manifest.provides, () => requireContext(instance)) : undefined;
    if (literatureRuntime && plugin.packageKey) {
      const release = bindRuntimeLiteratureInteroperability(literatureRuntime, () => !disposed && instance.state === "ready"
        && instance.acceptsContext && instances.get(pluginId) === instance, plugin.packageKey);
      if (release) instance.cleanups.push({run: release});
    }
    const context: PluginActivationContext = Object.freeze({pluginId, apiVersion: 1,
      ...(literatureRuntime ? {literatureRuntime} : {}),
      registerView(view: PluginViewContribution): PluginViewRegistration {
        requireContext(instance);
        if (!view || typeof view !== "object" || Array.isArray(view) || typeof view.id !== "string"
          || view.id.length > 128 || !/^[a-z][a-zA-Z0-9_-]*$/u.test(view.id)
          || typeof view.title !== "string" || !view.title.trim() || view.title.length > 256
          || /[\u0000-\u001f\u007f]/u.test(view.title) || typeof view.render !== "function"
          || (view.priority !== undefined && !Number.isFinite(view.priority)))
          throw error(instance, "plugin_view_invalid");
        if (view.id === "settings") throw error(instance, "plugin_view_reserved", undefined, undefined, view.id);
        if (views.getSnapshot().some(existing => existing.id === view.id && existing.pluginId !== pluginId))
          throw error(instance, "plugin_view_route_conflict", undefined, undefined, view.id);
        if (view.restore !== undefined && (!view.restore || typeof view.restore !== "object" || Array.isArray(view.restore)
          || Object.keys(view.restore).some(key => key !== "homeTabId" && key !== "readActiveTabId")
          || typeof view.restore.homeTabId !== "string" || !view.restore.homeTabId
          || typeof view.restore.readActiveTabId !== "function")) throw error(instance, "plugin_view_invalid");
        if (view.reading !== undefined && (!view.reading || typeof view.reading !== "object" || Array.isArray(view.reading)
          || Object.keys(view.reading).some(key => !["ReadingActions", "ReadingTemplateDialog", "requestPortraitDetail"].includes(key))
          || pluginId !== "literature_library" || view.id !== "library"
          || [view.reading.ReadingActions, view.reading.ReadingTemplateDialog, view.reading.requestPortraitDetail]
            .some(value => value !== undefined && typeof value !== "function")))
          throw error(instance, "plugin_view_invalid");
        if (view.writing !== undefined && (!view.writing || typeof view.writing !== "object" || Array.isArray(view.writing)
          || Reflect.ownKeys(view.writing).length !== 2
          || Reflect.ownKeys(view.writing).some(key => key !== "Runtime" && key !== "Session")
          || pluginId !== "writing_projects" || view.id !== "writing"
          || typeof view.writing.Runtime !== "function" || typeof view.writing.Session !== "function"))
          throw error(instance, "plugin_view_invalid");
        const registration = views.register(pluginId, scopedContribution(view, facets, configuration, () => requireContext(instance), subscribe,
          () => disposed || instance.state !== "ready" || !instance.acceptsContext || instances.get(pluginId) !== instance, () => snapshot));
        let cleanupDisposal: RuntimePluginWork | undefined;
        const owned: PluginViewRegistration = {
          open() {
            try { requireContext(instance); } catch (cause) { return Promise.reject(cause); }
            return wait(instance, invokeRuntimePluginWork(() => registration.open()), Date.now() + lifecycleTimeoutMs, "view.open");
          },
          dispose() {
            if (instance.cleaning) {
              cleanupDisposal ??= invokeRuntimePluginWork(() => registration.dispose());
              return cleanupDisposal.promise;
            }
            if (instance.state === "disabled") return Promise.resolve();
            try { requireContext(instance); } catch (cause) { return Promise.reject(cause); }
            return wait(instance, invokeRuntimePluginWork(() => registration.dispose()), Date.now() + lifecycleTimeoutMs, "view.dispose");
          },
        };
        instance.cleanups.push({run: () => owned.dispose()});
        return owned;
      },
      onDispose(dispose: () => void | Promise<void>): void {
        requireContext(instance);
        instance.cleanups.push({run: dispose});
      },
    });
    instance.activation = invokeRuntimePluginWork(() => plugin.module.activate(context));
    try {
      await wait(instance, instance.activation, Date.now() + lifecycleTimeoutMs, "activate");
      transition(instance, "ready");
    } catch (cause) {
      instance.acceptsContext = false;
      // An unresolved activate may still own accepted work. Do not run cleanup
      // concurrently, or let its eventual settlement claim a ready instance.
      if (instance.activation.state === "pending") {
        const failure = error(instance, "plugin_lifecycle_timeout", {cause}, "activate");
        transition(instance, "failed", failure); throw failure;
      }
      try { await runCleanups(instance, Date.now() + lifecycleTimeoutMs, true); }
      catch (cleanupCause) {
        const failures = cleanupCause instanceof AggregateError ? cleanupCause.errors : [cleanupCause];
        const failure = error(instance, "plugin_activation_cleanup_failed", {
          cause: new AggregateError([cause, ...failures], "Plugin activation and owned cleanup failed"),
        });
        transition(instance, "failed", failure); throw failure;
      }
      instance.retained = false;
      const failure = error(instance, "plugin_activation_failed", {cause});
      transition(instance, "failed", failure); throw failure;
    }
  };
  const apply = async (result: RuntimePluginLoadResult): Promise<void> => {
    if (disposed) throw new RuntimePluginWorkbenchError("plugin_workbench_disposed", "<workbench>", "<workbench>");
    for (const [pluginId, instance] of instances) {
      if (instance.state === "failed" && instance.retained) throw instance.error;
      const next = result.plugins.find(plugin => plugin.manifest.id === pluginId);
      if ((instance.retained && !next) || (next && (next.entryUrl !== instance.plugin.entryUrl
        || next.manifest.version !== instance.plugin.manifest.version)))
        throw error(instance, "plugin_restart_required");
      if (!next && !instance.retained) instances.delete(pluginId);
    }
    const facets = bindBuiltinAgentExtensions(result.plugins.map(plugin => plugin.manifest), result.bindings);
    const configurations = bindRuntimePluginConfigurations(storage, result.plugins);
    for (const plugin of result.plugins) {
      const existing = instances.get(plugin.manifest.id);
      if (existing?.state === "ready") continue;
      const dependencies = new Set([...plugin.manifest.dependencies.required, ...plugin.manifest.dependencies.optional]
        .filter(dependency => result.plugins.some(candidate => candidate.manifest.id === dependency.id)
          && !result.unavailableOptional.some(diagnostic => diagnostic.pluginId === plugin.manifest.id
            && diagnostic.dependencyId === dependency.id)).map(dependency => dependency.id));
      for (const binding of result.bindings)
        if (binding.pluginId === plugin.manifest.id && binding.provider !== "core" && binding.provider !== plugin.manifest.id)
          dependencies.add(binding.provider);
      instances.set(plugin.manifest.id, {plugin, dependencies, state: "discovered", cleanups: [],
        acceptsContext: false, retained: false, cleaning: false});
    }
    publish();
    for (const plugin of result.plugins) {
      if (disposed) throw new RuntimePluginWorkbenchError("plugin_workbench_disposed", "<workbench>", "<workbench>");
      const instance = instances.get(plugin.manifest.id);
      if (!instance || instance.state === "ready") continue;
      const extensions = facets.get(plugin.manifest.id);
      if (!extensions) throw new Error("agent_extension_consumer_unavailable: " + plugin.manifest.id);
      await activatePlugin(instance, extensions, configurations.get(plugin.manifest.id));
    }
  };
  const stopInstance = async (instance: RuntimePluginInstance): Promise<void> => {
    if (!instance.retained) { transition(instance, "disabled"); return; }
    // Repeated requests cannot accumulate waits on an unresolved old operation.
    // Once it settles, an explicit request may join that receipt without replay.
    if (instance.state === "failed" && instance.error && (instance.activation?.state === "pending"
      || instance.deactivation?.state === "pending" || instance.cleanups.some(cleanup => cleanup.work?.state === "pending")))
      throw instance.error;
    instance.acceptsContext = false;
    transition(instance, "deactivating");
    const deadline = Date.now() + lifecycleTimeoutMs;
    let preparation: ReturnType<PluginViewRegistry["beginOwnerClose"]> | undefined;
    try {
      const close = views.beginOwnerClose(instance.plugin.manifest.id);
      preparation = close;
      // Join the original activation; a pending/late result never creates a
      // replacement instance or automatically replays any of its effects.
      if (instance.activation?.state === "pending") await wait(instance, instance.activation, deadline, "activate.settlement");
      if (Date.now() >= deadline) throw error(instance, "plugin_lifecycle_timeout", undefined, "prepareClose");
      await wait(instance, invokeRuntimePluginWork(() => close.prepare()), deadline, "prepareClose");
      // The existing module may release its own handles as part of deactivate.
      // Only this prepared attempt permits that release; timeout closes the
      // window before cancelling it, so a late continuation cannot withdraw UI.
      instance.cleaning = true;
      try {
        const deactivate = instance.plugin.module.deactivate;
        if (instance.activation?.state === "fulfilled" && deactivate) {
          if (Date.now() >= deadline) throw error(instance, "plugin_lifecycle_timeout", undefined, "deactivate");
          instance.deactivation ??= invokeRuntimePluginWork(() => deactivate.call(instance.plugin.module));
          await wait(instance, instance.deactivation, deadline, "deactivate");
        }
        await runCleanups(instance, deadline, false);
        close.commit();
      } finally {
        instance.cleaning = false;
      }
      instance.retained = false;
      transition(instance, "disabled");
    } catch (cause) {
      preparation?.cancel();
      const failure = cause instanceof RuntimePluginWorkbenchError && cause.code === "plugin_lifecycle_timeout"
        ? cause : error(instance, "plugin_deactivation_failed", {cause});
      transition(instance, "failed", failure); throw failure;
    }
  };
  const stopWithDependents = (pluginId: string, visiting = new Set<string>(),
    attempts = new Map<string, Promise<void>>()): Promise<void> => {
    const instance = instances.get(pluginId);
    if (!instance) return Promise.reject(new RuntimePluginWorkbenchError("plugin_unknown_instance", pluginId, "<unknown>"));
    if (visiting.has(pluginId)) return Promise.reject(error(instance, "plugin_deactivation_dependency_cycle"));
    const previous = attempts.get(pluginId); if (previous) return previous;
    const ancestors = new Set(visiting).add(pluginId);
    const pending = (async () => {
      for (const dependent of [...instances.values()].reverse())
        if (dependent.retained && dependent.dependencies.has(pluginId))
          await stopWithDependents(dependent.plugin.manifest.id, ancestors, attempts);
      await stopInstance(instance);
    })();
    attempts.set(pluginId, pending);
    return pending;
  };
  return {
    views,
    getSnapshot: () => snapshot,
    subscribe,
    completeInitialLoad,
    isInitialLoadComplete: () => initialLoadComplete,
    subscribeInitialLoad(listener) {
      initialLoadListeners.add(listener);
      return () => { initialLoadListeners.delete(listener); };
    },
    retainPluginStyles: entryUrl => [...instances.values()].some(instance => instance.retained && instance.plugin.entryUrl === entryUrl),
    attachNavigation(effects) {
      const attached = {effects}; navigation = attached;
      return () => { if (navigation === attached) navigation = undefined; };
    },
    activate: result => enqueue(() => apply(result)).finally(completeInitialLoad),
    deactivate(pluginId) {
      if (disposed) return Promise.reject(new RuntimePluginWorkbenchError("plugin_workbench_disposed", pluginId, "<workbench>"));
      const current = stopping.get(pluginId); if (current) return current;
      const pending = enqueue(() => stopWithDependents(pluginId));
      stopping.set(pluginId, pending);
      void pending.then(() => { stopping.delete(pluginId); }, () => { stopping.delete(pluginId); });
      return pending;
    },
    dispose() {
      if (disposing) return disposing;
      disposed = true;
      disposing = enqueue(async () => {
        const failures: unknown[] = [];
        const attempts = new Map<string, Promise<void>>();
        for (const instance of [...instances.values()].reverse()) {
          if (!instance.retained) continue;
          try { await stopWithDependents(instance.plugin.manifest.id, new Set(), attempts); } catch (cause) { failures.push(cause); }
        }
        if (failures.length) throw new AggregateError(failures, "Plugin workbench disposal incomplete");
      });
      return disposing;
    },
  };
}
