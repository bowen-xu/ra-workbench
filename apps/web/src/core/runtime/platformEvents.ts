import { listen as nativeListen, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";
import { getAppRuntime } from "./appRuntime";

export interface PlatformEvents {
  listen<T>(name: string, handler: EventCallback<T>): Promise<UnlistenFn>;
}
export type { UnlistenFn };

/** Production subscribers share one event port with the isolated native preview. */
export function listen<T>(name: string, handler: EventCallback<T>): Promise<UnlistenFn> {
  const runtime = getAppRuntime();
  if (runtime.events) return runtime.events.listen(name, handler);
  if (runtime.mode === "desktop") return nativeListen(name, handler);
  if (runtime.mode === "preview") return Promise.reject(new Error(`预览未覆盖平台事件：${name}`));
  return Promise.resolve(() => {});
}
