import { confirm as nativeConfirm, open as nativeOpen, save as nativeSave } from "@tauri-apps/plugin-dialog";
import { diagnosticError } from "../services/diagnosticError";
import { getAppRuntime, isAppPreview } from "./appRuntime";
export const open: typeof nativeOpen = ((options?: Parameters<typeof nativeOpen>[0]) => {
  if (!isAppPreview()) return nativeOpen(options);
  const path = options?.directory ? "/Users/preview/Documents" : "mock://selected-sample.pdf";
  return Promise.resolve(options?.multiple ? [path] : path);
}) as typeof nativeOpen;
export const save: typeof nativeSave = options => isAppPreview() ? Promise.resolve("mock://export-result") : nativeSave(options);

/** Await the real confirmation before admitting work. The desktop plugin's
 * injected window.confirm uses an obsolete command, so use its supported API. */
export async function confirm(message: string): Promise<boolean> {
  const desktop = getAppRuntime().mode === "desktop";
  try {
    return desktop ? await nativeConfirm(message) : await window.confirm(message);
  } catch (cause) {
    const diagnostic = diagnosticError(cause);
    if (desktop && typeof cause === "string" && /^dialog\.message not allowed(?:\.|$)/u.test(cause))
      diagnostic.cause = "permission_denied";
    const details = Object.entries(diagnostic)
      .filter(([key, value]) => ["code", "cause", "requestId", "turnId", "operationId", "taskId", "httpStatus"].includes(key)
        && (typeof value === "string" || typeof value === "number"))
      .map(([key, value]) => `${key}=${value}`);
    throw Object.assign(new Error(`PLATFORM_CONFIRMATION_FAILED：确认对话框未能完成（adapter=${desktop ? "plugin:dialog|message" : "browser.confirm"}；${details.join("；")}）`, {cause}),
      {code: "PLATFORM_CONFIRMATION_FAILED", diagnostic});
  }
}
