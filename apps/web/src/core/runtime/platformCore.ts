import { invoke as nativeInvoke, isTauri } from "@tauri-apps/api/core";
import { getAppRuntime, isAppPreview } from "./appRuntime";
import {nativeCommandDiagnostics} from "./NativeCommandDiagnostics";
export { isTauri };
/** Native UI subscriptions are disabled when the preview adapter is selected. */
export function isDesktopRuntime(): boolean { return getAppRuntime().mode === "desktop"; }

/** IPC is fail-closed in preview, even if a caller misses its gateway override. */
export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isAppPreview()) {
    if (nativeCommandDiagnostics) return nativeCommandDiagnostics.observe(command,
      () => args === undefined ? nativeInvoke<T>(command) : nativeInvoke<T>(command, args));
    return args === undefined ? nativeInvoke<T>(command) : nativeInvoke<T>(command, args);
  }
  const runtime = getAppRuntime();
  if (!runtime.invoke) throw new Error(`预览未覆盖平台调用：${command}`);
  return await runtime.invoke(command, args) as T;
}
