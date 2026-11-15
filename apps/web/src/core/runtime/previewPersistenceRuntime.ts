import type { AppRuntime } from "./appRuntime";
import { flushStorage, storagePersistence } from "./storagePersistence";

/** Keep synchronous capabilities/subscriptions intact, but async API receipts wait for storage. */
export function withPreviewPersistence(runtime: AppRuntime): AppRuntime {
  const port = storagePersistence(runtime.storage);
  if (!port) return runtime;
  const proxies = new WeakMap<object, object>();
  function wrap<T extends object>(target: T): T {
    const cached = proxies.get(target);
    if (cached) return cached as T;
    const methods = new Map<string | symbol, {source: Function; wrapped: Function}>();
    const proxy = new Proxy(target, {get(object, key) {
      const member: unknown = Reflect.get(object, key);
      if (typeof member === "function") {
        const cached = methods.get(key);
        if (cached?.source === member) return cached.wrapped;
        const wrapped = (...args: unknown[]) => {
          const value: unknown = member.apply(object, args);
          if (!(value instanceof Promise)) return value;
          return value.then(async result => { await flushStorage(runtime.storage); return result; }, async reason => {
            try { await flushStorage(runtime.storage); }
            catch (storageError) {
              const code = reason && typeof reason === "object" && "code" in reason && typeof reason.code === "string" ? `${reason.code}：` : "";
              throw new Error(`${code}${reason instanceof Error ? reason.message : String(reason)}；${storageError instanceof Error ? storageError.message : String(storageError)}`, {cause: reason});
            }
            throw reason;
          });
        };
        methods.set(key, {source: member, wrapped}); return wrapped;
      }
      // Only gateway members are traversed, never returned domain records or callback handles.
      return member && typeof member === "object" ? wrap(member) : member;
    }});
    proxies.set(target, proxy); return proxy;
  }
  return {...runtime, gateways: wrap(runtime.gateways),
    ...(runtime.invoke ? {invoke: wrap({invoke: runtime.invoke}).invoke} : {}),
    dispose() { try { runtime.dispose(); } finally { port.close(); } },
  };
}
