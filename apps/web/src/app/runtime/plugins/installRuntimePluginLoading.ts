import type {
  RuntimePluginLoader, RuntimePluginLoadResult,
} from "../../../core/runtime/plugins/loader";

export interface RuntimePluginLoadingSummary {
  readonly plugins: readonly {
    readonly id: string;
    readonly version: string;
    readonly entryUrl: string;
    readonly styles: readonly string[];
  }[];
  readonly bindings: RuntimePluginLoadResult["bindings"];
  readonly unavailableOptional: RuntimePluginLoadResult["unavailableOptional"];
}

export interface RuntimePluginLoadingOptions {
  readonly loader: RuntimePluginLoader;
  /** Internal event requests report errors here. Direct load callers own their
   * rejection, so application startup can use its existing notification path. */
  readonly onError: (reason: unknown) => void;
  readonly onLoaded?: (result: RuntimePluginLoadResult) => void | Promise<void>;
  /** App owns safe close/deactivation; this loader only releases its CSS after the receipt. */
  readonly onDeactivate?: (pluginId: string) => Promise<void>;
  /** A partially successful activation keeps only the exact active package's
   * ready styles. This is App ownership, not a plugin-callable capability. */
  readonly retainPluginStyles?: (entryUrl: string) => boolean;
  readonly target?: Window;
  readonly document?: Document;
}

export interface InstalledRuntimePluginLoading {
  load(): Promise<RuntimePluginLoadResult>;
  deactivate(pluginId: string): Promise<void>;
  dispose(): void;
}

interface LoadingDiagnostic {
  readonly code: "plugin_stylesheet_failed" | "plugin_stylesheet_timeout" | "plugin_loading_disposed"
    | "plugin_deactivation_invalid" | "plugin_deactivation_unavailable"
    | "plugin_unknown_instance" | "plugin_deactivation_incomplete";
  readonly pluginId?: string;
  readonly href?: string;
  readonly timeoutMs?: number;
}

export class RuntimePluginLoadingError extends Error {
  readonly code: LoadingDiagnostic["code"];
  constructor(readonly diagnostic: LoadingDiagnostic, options?: ErrorOptions) {
    super(`${diagnostic.code}${diagnostic.pluginId ? `: ${diagnostic.pluginId}` : ""}`, options);
    this.name = "RuntimePluginLoadingError";
    this.code = diagnostic.code;
  }
}

interface OwnedStylesheet {
  readonly ready: Promise<void>;
  isReady(): boolean;
  dispose(): void;
}

// Packages are local protocol resources. A generous 30s bound prevents a lost
// native load/error event from holding application loading open indefinitely;
// timeout fails this scan and an explicit later scan may retry.
const STYLESHEET_TIMEOUT_MS = 30_000;
const LOAD_EVENT = "research-assistant:load-runtime-plugins";
const LOADED_EVENT = "research-assistant:runtime-plugins-loaded";
const DEACTIVATE_EVENT = "research-assistant:deactivate-runtime-plugin";
const DEACTIVATED_EVENT = "research-assistant:runtime-plugin-deactivated";

function disposedError(): RuntimePluginLoadingError {
  return new RuntimePluginLoadingError({code: "plugin_loading_disposed"});
}

function installStylesheet(document: Document, target: Window, pluginId: string, href: string): OwnedStylesheet {
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.setAttribute("href", href);
  let cancel: (() => void) | undefined;
  let loadedSuccessfully = false;
  const ready = new Promise<void>((resolve, reject) => {
    let settled = false;
    const finish = (reason?: unknown): void => {
      if (settled) return;
      settled = true;
      target.clearTimeout(timeout);
      link.removeEventListener("load", loaded);
      link.removeEventListener("error", failed);
      if (reason === undefined) { loadedSuccessfully = true; resolve(); } else reject(reason);
    };
    const loaded = (): void => finish();
    const failed = (cause: Event): void => finish(new RuntimePluginLoadingError({
      code: "plugin_stylesheet_failed", pluginId, href,
    }, {cause}));
    const timeout = target.setTimeout(() => finish(new RuntimePluginLoadingError({
      code: "plugin_stylesheet_timeout", pluginId, href, timeoutMs: STYLESHEET_TIMEOUT_MS,
    })), STYLESHEET_TIMEOUT_MS);
    cancel = () => finish(disposedError());
    link.addEventListener("load", loaded);
    link.addEventListener("error", failed);
    try { document.head.append(link); }
    catch (cause) {
      finish(new RuntimePluginLoadingError({code: "plugin_stylesheet_failed", pluginId, href}, {cause}));
    }
  });
  return {ready, isReady: () => loadedSuccessfully, dispose() { cancel?.(); link.remove(); }};
}

function summarize(result: RuntimePluginLoadResult): RuntimePluginLoadingSummary {
  return {
    plugins: result.plugins.map(({manifest, entryUrl, styles}) => ({
      id: manifest.id, version: manifest.version, entryUrl, styles: [...styles],
    })),
    bindings: result.bindings.map(binding => ({...binding})),
    unavailableOptional: result.unavailableOptional.map(diagnostic => ({
      ...diagnostic, ...(diagnostic.cycle ? {cycle: [...diagnostic.cycle]} : {}),
    })),
  };
}

/** App owns these listeners and stylesheet links until dispose. Ready links
 * survive failed rescans. Safe deactivation stays with App; only its fulfilled
 * receipt and exact retained-entry check permit removing this owner's CSS.
 * Page disposal remains synchronous presentation cleanup, not a safe-stop receipt. */
export function installRuntimePluginLoading(options: RuntimePluginLoadingOptions): InstalledRuntimePluginLoading {
  const target = options.target ?? window;
  const document = options.document ?? target.document;
  const styles = new Map<string, OwnedStylesheet>();
  const owners = new Map<string, {readonly pluginId: string; readonly hrefs: readonly string[]}>();
  const stopping = new Map<string, Promise<void>>();
  const reportedStopFailures = new WeakSet<Promise<void>>();
  let disposed = false;
  let pending: Promise<RuntimePluginLoadResult> | undefined;
  let generation = 0;
  let pendingGeneration = 0;
  let mutation: Promise<unknown> | undefined;
  let reportedEventFailure: Promise<RuntimePluginLoadResult> | undefined;

  // Only this installed loader's load/stop mutations share a tail. Starting a
  // first load remains immediate; a later scan cannot race CSS release.
  const mutate = <T>(run: () => Promise<T>): Promise<T> => {
    const operation = mutation ? mutation.then(run, run) : run();
    mutation = operation;
    const clear = (): void => { if (mutation === operation) mutation = undefined; };
    void operation.then(clear, clear);
    return operation;
  };
  const performLoad = async (): Promise<RuntimePluginLoadResult> => {
    if (disposed) throw disposedError();
    const added = new Map<string, OwnedStylesheet>();
    const addedOwners = new Map<string, Set<string>>();
    let attempted: RuntimePluginLoadResult["plugins"] = [];
    try {
      const result = await options.loader.load();
      if (disposed) throw disposedError();
      attempted = result.plugins;
      const ready: Promise<void>[] = [];
      const currentHrefs = new Set<string>();
      for (const plugin of result.plugins) {
        for (const href of plugin.styles) {
          currentHrefs.add(href);
          let style = styles.get(href);
          if (!style) {
            style = installStylesheet(document, target, plugin.manifest.id, href);
            styles.set(href, style); added.set(href, style);
          }
          const owningEntries = addedOwners.get(href) ?? new Set<string>();
          owningEntries.add(plugin.entryUrl); addedOwners.set(href, owningEntries);
          ready.push(style.ready);
        }
      }
      await Promise.all(ready);
      if (disposed) throw disposedError();
      await options.onLoaded?.(result);
      if (disposed) throw disposedError();
      // Commit only after the entire replacement is ready. Failed replacements
      // leave the prior catalog intact; removed/replaced packages cannot leave
      // old CSS active or grow this owner's cache across successful scans.
      for (const [href, style] of styles) {
        if (currentHrefs.has(href)) continue;
        styles.delete(href); style.dispose();
      }
      owners.clear();
      for (const plugin of result.plugins)
        owners.set(plugin.entryUrl, {pluginId: plugin.manifest.id, hrefs: [...plugin.styles]});
      const Event = document.defaultView?.CustomEvent ?? CustomEvent;
      target.dispatchEvent(new Event(LOADED_EVENT, {detail: summarize(result)}));
      return result;
    } catch (reason) {
      // Previously successful styles stay. If one new plugin activated before
      // a later plugin failed, its live view also keeps that exact entry's ready
      // styles; an old instance cannot retain a replacement URL's new styles.
      for (const [href, style] of added) {
        if (!disposed && style.isReady()
          && [...(addedOwners.get(href) ?? [])].some(entry => options.retainPluginStyles?.(entry))) continue;
        if (styles.get(href) === style) styles.delete(href);
        style.dispose();
      }
      if (!disposed) for (const plugin of attempted) if (options.retainPluginStyles?.(plugin.entryUrl))
        owners.set(plugin.entryUrl, {pluginId: plugin.manifest.id, hrefs: [...plugin.styles]});
      throw reason;
    }
  };
  const load = (): Promise<RuntimePluginLoadResult> => {
    if (disposed) return Promise.reject(disposedError());
    if (pending && pendingGeneration === generation) return pending;
    const operation = mutate(performLoad).finally(() => { if (pending === operation) pending = undefined; });
    pending = operation; pendingGeneration = generation;
    return operation;
  };
  const deactivate = (pluginId: string): Promise<void> => {
    if (disposed) return Promise.reject(disposedError());
    if (typeof pluginId !== "string" || !/^[a-z][a-z0-9_-]*$/u.test(pluginId) || pluginId === "core")
      return Promise.reject(new RuntimePluginLoadingError({code: "plugin_deactivation_invalid"}));
    const existing = stopping.get(pluginId); if (existing) return existing;
    // A scan requested after stop must not join a pre-stop load and silently
    // miss reactivation. Same-generation scans still coalesce as before.
    generation += 1;
    const operation = mutate(async () => {
      // Reserve the operation before a callback can request the same stop again.
      await Promise.resolve();
      if (disposed) throw disposedError();
      if (![...owners.values()].some(owner => owner.pluginId === pluginId))
        throw new RuntimePluginLoadingError({code: "plugin_unknown_instance", pluginId});
      if (!options.onDeactivate || !options.retainPluginStyles)
        throw new RuntimePluginLoadingError({code: "plugin_deactivation_unavailable", pluginId});
      await options.onDeactivate(pluginId);
      if (disposed) throw disposedError();
      const retainedHrefs = new Set<string>();
      for (const [entryUrl, owner] of owners) {
        if (!options.retainPluginStyles(entryUrl)) continue;
        if (owner.pluginId === pluginId)
          throw new RuntimePluginLoadingError({code: "plugin_deactivation_incomplete", pluginId});
        for (const href of owner.hrefs) retainedHrefs.add(href);
      }
      // A shared href remains while any exact entry still owns it. This also
      // releases dependents stopped by App without touching unrelated owners.
      for (const [href, style] of styles) if (!retainedHrefs.has(href)) {
        styles.delete(href); style.dispose();
      }
      const Event = document.defaultView?.CustomEvent ?? CustomEvent;
      target.dispatchEvent(new Event(DEACTIVATED_EVENT, {detail: {pluginId}}));
    });
    stopping.set(pluginId, operation);
    const clear = (): void => { if (stopping.get(pluginId) === operation) stopping.delete(pluginId); };
    void operation.then(clear, clear);
    return operation;
  };
  const requested = (event: Event): void => {
    const detail: unknown = "detail" in event ? event.detail : undefined;
    const callbacks = detail && typeof detail === "object" && !Array.isArray(detail)
      ? detail as Record<string, unknown> : {};
    const operation = load();
    const report = (reason: unknown): void => {
      if (disposed || reportedEventFailure === operation) return;
      reportedEventFailure = operation;
      options.onError(reason);
    };
    void operation.then(result => {
      if (!disposed && typeof callbacks.resolve === "function") callbacks.resolve(summarize(result));
    }, reason => {
      report(reason);
      if (typeof callbacks.reject === "function") callbacks.reject(reason);
    }).catch(report);
  };
  const stopRequested = (event: Event): void => {
    const detail: unknown = "detail" in event ? event.detail : undefined;
    const callbacks = detail && typeof detail === "object" && !Array.isArray(detail)
      ? detail as Record<string, unknown> : {};
    const pluginId = callbacks.pluginId;
    const operation = typeof pluginId === "string" ? deactivate(pluginId)
      : Promise.reject(new RuntimePluginLoadingError({code: "plugin_deactivation_invalid"}));
    const report = (reason: unknown): void => {
      if (disposed || reportedStopFailures.has(operation)) return;
      reportedStopFailures.add(operation); options.onError(reason);
    };
    void operation.then(() => {
      if (!disposed && typeof callbacks.resolve === "function") callbacks.resolve({pluginId});
    }, reason => {
      if (disposed) return;
      report(reason);
      if (typeof callbacks.reject === "function") callbacks.reject(reason);
    }).catch(report);
  };
  target.addEventListener(LOAD_EVENT, requested);
  target.addEventListener(DEACTIVATE_EVENT, stopRequested);
  return {
    load,
    deactivate,
    dispose() {
      if (disposed) return;
      disposed = true;
      target.removeEventListener(LOAD_EVENT, requested);
      target.removeEventListener(DEACTIVATE_EVENT, stopRequested);
      for (const style of styles.values()) style.dispose();
      styles.clear();
      owners.clear();
    },
  };
}
