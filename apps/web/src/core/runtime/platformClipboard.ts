import {readText} from "@tauri-apps/plugin-clipboard-manager";
import {isDesktopRuntime} from "./platformCore";

/** Explicit paste gestures only. Never poll, log, persist or write clipboard
 * contents. Preview uses its browser clipboard, never the native plugin. */
export async function readClipboardText(): Promise<string> {
  if (isDesktopRuntime()) return readText();
  if (!navigator.clipboard?.readText) throw new Error("CLIPBOARD_TEXT_UNAVAILABLE：当前浏览器不支持读取剪贴板文字。");
  return navigator.clipboard.readText();
}
