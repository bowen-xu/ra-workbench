import { openUrl } from "@tauri-apps/plugin-opener";
import { getAppRuntime, type AppRuntime } from "./appRuntime";

/** Web destinations only. Application resource navigation has a separate owner. */
export interface BrowserAdapter {
  open(url: string): Promise<void>;
}

const systemBrowser: BrowserAdapter = { open: url => openUrl(url) };
const webBrowser: BrowserAdapter = {
  async open(url) { window.open(url, "_blank", "noopener,noreferrer"); },
};

export function isWebUrl(value: string): boolean {
  try { return ["http:", "https:"].includes(new URL(value).protocol); }
  catch { return false; }
}

/** Resolve at invocation so an embedded browser can replace the startup port. */
export async function openBrowserPage(value: string, runtime: AppRuntime = getAppRuntime()): Promise<void> {
  if (!isWebUrl(value)) throw new Error("[BROWSER_INVALID_URL] 不是有效的 HTTP／HTTPS 网页地址");
  const url = new URL(value).href;
  try {
    await (runtime.browser ?? (runtime.mode === "desktop" ? systemBrowser : webBrowser)).open(url);
  } catch (cause) {
    const detail = cause instanceof Error ? cause.message : String(cause);
    // Native opener failures can echo the URL; omit credentials and query values.
    const safeDetail = detail.replace(/https?:\/\/[^\s"'<>]+/giu, raw => {
      try { const parsed = new URL(raw); return parsed.origin + parsed.pathname; }
      catch { return "[网页地址]"; }
    });
    throw new Error(`[BROWSER_OPEN_FAILED] 无法打开网页：${safeDetail}`, { cause });
  }
}
