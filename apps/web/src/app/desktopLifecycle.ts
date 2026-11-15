import { invoke, isDesktopRuntime } from "../core/runtime/platformCore";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { getReaderCloseCoordinator } from "../core/public/workbench";
import {getAppStorage} from "../core/public/storage";
import {flushStorage} from "../core/public/storage";

export type DesktopCloseIntent = "reader-tab" | "window";

export interface DesktopLifecycleOptions {
  readonly onError?: (message: string) => void;
}

export function desktopCloseIntent(hasActiveReader: boolean): DesktopCloseIntent {
  return hasActiveReader ? "reader-tab" : "window";
}

export async function installDesktopLifecycle(options: DesktopLifecycleOptions = {}): Promise<() => void> {
  if (!isDesktopRuntime()) return () => undefined;
  const coordinator = getReaderCloseCoordinator();
  const appWindow = getCurrentWindow();
  let closing = false;
  const withCloseLock = async (label: string, operation: () => Promise<void>): Promise<void> => {
    if (closing) return;
    closing = true;
    try {
      await operation();
    } catch (reason: unknown) {
      options.onError?.(`${label}失败：${reason instanceof Error ? reason.message : String(reason)}`);
    } finally {
      closing = false;
    }
  };
  const requestWindowClose = async (): Promise<void> => withCloseLock("关闭窗口", async () => {
    if (!(await coordinator.prepareAll())) return;
    await flushStorage(getAppStorage());
    await appWindow.hide();
    if (await appWindow.isVisible()) throw new Error("窗口仍然可见，请重试");
  });
  const requestAppQuit = async (): Promise<void> => withCloseLock("退出应用", async () => {
    if (await coordinator.prepareAll()) { await flushStorage(getAppStorage()); await invoke("confirm_app_quit"); }
  });
  const unlistenWindow = await appWindow.onCloseRequested(async (event) => {
    event.preventDefault();
    await requestWindowClose();
  });
  const unlistenQuit = await listen("app-quit-request", requestAppQuit);
  const keydown = (event: KeyboardEvent): void => {
    if (event.defaultPrevented || !event.metaKey || event.ctrlKey || event.altKey || event.key.toLocaleLowerCase() !== "w") return;
    event.preventDefault();
    const intent = desktopCloseIntent(coordinator.hasActive());
    if (intent === "reader-tab") void coordinator.closeActive();
    else void requestWindowClose();
  };
  window.addEventListener("keydown", keydown);
  return () => {
    window.removeEventListener("keydown", keydown);
    unlistenWindow();
    unlistenQuit();
  };
}
