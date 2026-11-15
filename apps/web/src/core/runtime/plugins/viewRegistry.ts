import type {
  PluginViewContribution, PluginViewRegistration,
} from "@research-assistant/workbench-sdk/activation";

/** Core-internal presentation identity; plugin view IDs remain local to their owner. */
export interface RegisteredPluginView extends PluginViewContribution {
  readonly key: string;
  readonly pluginId: string;
}

export interface PluginViewRegistryEffects {
  readonly open: (view: RegisteredPluginView, signal?: AbortSignal) => Promise<void>;
  /** Preparation keeps the original view/document mounted. False blocks removal. */
  readonly prepareClose: (view: RegisteredPluginView, signal?: AbortSignal) => Promise<boolean>;
}

/** Internal owner teardown: prepare preserves every projection until all
 * owner hooks finish. Cancellation leaves accepted Core work with Core. */
export interface PluginViewOwnerClose {
  prepare(): Promise<void>;
  commit(): void;
  cancel(): void;
}

export interface PluginViewRegistry {
  readonly register: (pluginId: string, contribution: PluginViewContribution) => PluginViewRegistration;
  readonly getSnapshot: () => readonly RegisteredPluginView[];
  readonly subscribe: (listener: () => void) => () => void;
  readonly beginOwnerClose: (pluginId: string) => PluginViewOwnerClose;
}

export type PluginViewRegistryErrorCode =
  | "plugin_view_duplicate" | "plugin_view_closing"
  | "plugin_view_disposed" | "plugin_view_close_blocked" | "plugin_view_close_timeout";

export class PluginViewRegistryError extends Error {
  constructor(readonly code: PluginViewRegistryErrorCode, readonly pluginId: string, readonly viewId: string) {
    super(`${code}: ${pluginId} / ${viewId}`);
    this.name = "PluginViewRegistryError";
  }
}

/** Owns registrations and immutable presentation snapshots only. Navigation,
 * document preparation and save/close ownership stay with the injected ports. */
export function createPluginViewRegistry(effects: PluginViewRegistryEffects,
  {closeTimeoutMs = 30_000}: {readonly closeTimeoutMs?: number} = {}): PluginViewRegistry {
  if (!Number.isFinite(closeTimeoutMs) || closeTimeoutMs <= 0) throw new RangeError("Plugin view close timeout must be positive");
  const entries = new Map<string, RegisteredPluginView>();
  const controls = new Map<RegisteredPluginView, {
    prepare(signal: AbortSignal): Promise<void>;
    markDisposed(): void;
    abortOpen(reason: unknown): void;
  }>();
  const ownerCloses = new Map<string, PluginViewOwnerClose>();
  const listeners = new Set<() => void>();
  let snapshot: readonly RegisteredPluginView[] = Object.freeze([]);

  const publish = (): void => {
    snapshot = Object.freeze([...entries.values()]);
    for (const listener of [...listeners]) listener();
  };

  function untilAborted<T>(work: Promise<T>, signal: AbortSignal): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const aborted = (): void => reject(signal.reason);
      if (signal.aborted) reject(signal.reason);
      else signal.addEventListener("abort", aborted, {once: true});
      // The underlying operation is still observed after cancellation. Its
      // accepted write remains owned by Core, while its UI continuation is stale.
      void work.then(value => {
        signal.removeEventListener("abort", aborted); resolve(value);
      }, reason => {
        signal.removeEventListener("abort", aborted); reject(reason);
      });
    });
  }

  return {
    getSnapshot: () => snapshot,
    subscribe(listener) {
      // Each subscription has its own identity, even for a reused callback.
      const subscribed = (): void => listener();
      listeners.add(subscribed);
      return () => { listeners.delete(subscribed); };
    },
    beginOwnerClose(pluginId) {
      if (ownerCloses.has(pluginId)) throw new PluginViewRegistryError("plugin_view_closing", pluginId, "<owner>");
      const captured = [...entries.values()].filter(view => view.pluginId === pluginId);
      const controller = new AbortController();
      let state: "preparing" | "prepared" | "committed" | "cancelled" = "preparing";
      let preparing: Promise<void> | undefined;
      const attempt: PluginViewOwnerClose = {
        prepare() {
          if (state === "cancelled") return Promise.reject(controller.signal.reason);
          if (state === "committed") return Promise.resolve();
          preparing ??= (async () => {
            for (const view of captured) {
              if (controller.signal.aborted) throw controller.signal.reason;
              await controls.get(view)?.prepare(controller.signal);
            }
            if (controller.signal.aborted) throw controller.signal.reason;
            state = "prepared";
          })();
          return preparing;
        },
        commit() {
          if (state === "committed") return;
          if (state !== "prepared") throw new PluginViewRegistryError("plugin_view_close_blocked", pluginId, "<owner>");
          state = "committed";
          let changed = false;
          for (const view of captured) {
            controls.get(view)?.markDisposed(); controls.delete(view);
            if (entries.get(view.key) === view) { entries.delete(view.key); changed = true; }
          }
          if (ownerCloses.get(pluginId) === attempt) ownerCloses.delete(pluginId);
          if (changed) publish();
        },
        cancel() {
          if (state === "committed" || state === "cancelled") return;
          state = "cancelled";
          const reason = new PluginViewRegistryError("plugin_view_closing", pluginId, "<owner>");
          controller.abort(reason);
          for (const view of captured) controls.get(view)?.abortOpen(reason);
          if (ownerCloses.get(pluginId) === attempt) ownerCloses.delete(pluginId);
        },
      };
      ownerCloses.set(pluginId, attempt);
      return attempt;
    },
    register(pluginId, contribution) {
      if (ownerCloses.has(pluginId)) throw new PluginViewRegistryError("plugin_view_closing", pluginId, contribution.id);
      const key = `plugin:${pluginId}/${contribution.id}`;
      if (entries.has(key)) throw new PluginViewRegistryError("plugin_view_duplicate", pluginId, contribution.id);
      const view: RegisteredPluginView = Object.freeze({key, pluginId,
        id: contribution.id, title: contribution.title, render: contribution.render,
        ...(contribution.priority === undefined ? {} : {priority: contribution.priority}),
        ...(contribution.restore ? {restore: Object.freeze({...contribution.restore})} : {}),
        ...(contribution.reading ? {reading: Object.freeze({...contribution.reading})} : {}),
        ...(contribution.writing ? {writing: Object.freeze({...contribution.writing})} : {})});
      let disposed = false;
      let opening: {readonly promise: Promise<void>; readonly controller: AbortController} | undefined;
      let closing: Promise<void> | undefined;
      const prepare = async (signal: AbortSignal): Promise<void> => {
        if (disposed) return;
        const pendingOpen = opening;
        if (pendingOpen) await untilAborted(pendingOpen.promise.catch(() => undefined), signal);
        if (signal.aborted) throw signal.reason;
        if (!await untilAborted(effects.prepareClose(view, signal), signal))
          throw new PluginViewRegistryError("plugin_view_close_blocked", pluginId, view.id);
        if (signal.aborted) throw signal.reason;
      };
      controls.set(view, {prepare: signal => closing ? untilAborted(closing, signal) : prepare(signal),
        markDisposed: () => { disposed = true; },
        abortOpen: reason => opening?.controller.abort(reason)});
      entries.set(key, view);
      publish();

      return {
        open() {
          if (disposed) return Promise.reject(new PluginViewRegistryError("plugin_view_disposed", pluginId, view.id));
          if (closing || ownerCloses.has(pluginId)) return Promise.reject(new PluginViewRegistryError("plugin_view_closing", pluginId, view.id));
          if (opening) return opening.promise;
          const controller = new AbortController();
          const pending: Promise<void> = untilAborted(Promise.resolve().then(() => {
            if (controller.signal.aborted) throw controller.signal.reason;
            return effects.open(view, controller.signal);
          }), controller.signal).finally(() => { if (opening?.promise === pending) opening = undefined; });
          opening = {promise: pending, controller};
          return pending;
        },
        dispose() {
          if (closing) return closing;
          if (disposed) return Promise.resolve();
          const ownerClose = ownerCloses.get(pluginId);
          // During owner cleanup this handle is already covered by the atomic
          // preparation. Publishing removal remains the owner's final commit.
          if (ownerClose) return ownerClose.prepare();
          const controller = new AbortController();
          const timeout = setTimeout(() => {
            const reason = new PluginViewRegistryError("plugin_view_close_timeout", pluginId, view.id);
            opening?.controller.abort(reason); controller.abort(reason);
          }, closeTimeoutMs);
          closing = Promise.resolve().then(async () => {
            // Drain an already dispatched open before preparing/removing its
            // view. Its caller retains the rejection; an open failure cannot
            // permanently prevent this registration from being released.
            await prepare(controller.signal);
            disposed = true;
            controls.delete(view);
            if (entries.get(key) === view) {
              entries.delete(key);
              publish();
            }
          }).catch(reason => {
            closing = undefined;
            throw reason;
          }).finally(() => { clearTimeout(timeout); });
          return closing;
        },
      };
    },
  };
}
